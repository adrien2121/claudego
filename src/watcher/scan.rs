use crate::logging::log_to_file;
use crate::watcher::reset_time;
use chrono::{DateTime, Local};
use serde_json::Value;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const CHUNK_SIZE: u64 = 65536;

/// Scans a log file from the end for rate limit messages.
///
/// This function is designed to be called without holding a lock on `AppState`.
/// It performs all necessary file I/O and returns the results.
///
/// # Arguments
/// * `path` - The path to the `.jsonl` log file.
/// * `old_size` - The last known size of the file. Used to determine if the file could be opened.
///
/// # Returns
/// A tuple containing:
/// 1. `Option<(DateTime<Local>, String)>` - The rate limit target time and display string if an active limit is found.
/// 2. `u64` - The new size of the file.
pub(crate) fn scan_session_log(
    path: &Path,
    old_size: u64,
) -> (Option<(DateTime<Local>, String)>, u64) {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return (None, old_size),
    };
    let new_size = match file.metadata() {
        Ok(m) => m.len(),
        Err(_) => return (None, old_size),
    };

    if new_size == 0 {
        return (None, 0);
    }

    let limit_opt = scan_file_from_end(&mut file, new_size);

    (limit_opt, new_size)
}

/// Reads a file in chunks from the end, searching for rate limit messages.
fn scan_file_from_end(file: &mut File, size: u64) -> Option<(DateTime<Local>, String)> {
    let mut offset = size;
    let mut leftover = Vec::new(); // Stores a partial line from a chunk boundary.
    
    while offset > 0 {
        let read_size = std::cmp::min(CHUNK_SIZE, offset);
        offset -= read_size;

        if file.seek(SeekFrom::Start(offset)).is_err() {
            break;
        }

        let mut buf = vec![0u8; read_size as usize];
        if file.read_exact(&mut buf).is_err() {
            break;
        }

        // Prepend leftover from previous chunk to handle lines spanning across chunks.
        if !leftover.is_empty() {
            buf.extend(&leftover);
        }

        let contents = String::from_utf8_lossy(&buf);
        let mut lines: Vec<&str> = contents.lines().collect();

        // The first line of a chunk may be incomplete; save it for the next read.
        if offset > 0 && !lines.is_empty() {
            leftover = lines.remove(0).as_bytes().to_vec();
        } else {
            leftover.clear();
        }

        match scan_lines_newest_first(lines) { // Scan lines from newest to oldest.
            LineScanResult::ActiveLimit(limit) => return Some(limit),
            LineScanResult::StaleLimit => break, // Found a stale limit, no need to look further.
            LineScanResult::NoLimit => {}         // No limit found, continue to the next chunk.
        }
    }

    None
}

fn scan_lines_newest_first(lines: Vec<&str>) -> LineScanResult {
    // Iterate in reverse because we are scanning from the end of the file.
    for line in lines.into_iter().rev() {
        match parse_rate_limit_line(line) {
            RateLimitLine::NoMatch => continue,
            // A stale limit means older entries are also stale; stop scanning.
            RateLimitLine::Stale => return LineScanResult::StaleLimit,
            RateLimitLine::Active(limit) => return LineScanResult::ActiveLimit(limit),
        }
    }

    LineScanResult::NoLimit
}

/// Parses a single JSON line to check if it is a rate limit error.
fn parse_rate_limit_line(line: &str) -> RateLimitLine {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return RateLimitLine::NoMatch;
    };

    // We only care about "rate_limit" errors.
    if value["error"].as_str() != Some("rate_limit") {
        return RateLimitLine::NoMatch;
    }

    log_to_file("  [MATCH] Active 'rate_limit' row found! Parsing contents...");

    let Some(raw_timestamp) = value["timestamp"].as_str() else {
        return RateLimitLine::NoMatch;
    };

    let Ok(log_time) =
        DateTime::parse_from_rfc3339(raw_timestamp).map(|time| time.with_timezone(&Local))
    else {
        return RateLimitLine::NoMatch;
    };

    let Some(content_text) = value["message"]["content"][0]["text"].as_str() else {
        return RateLimitLine::NoMatch;
    };

    log_to_file(&format!(
        "  [Extracted Text] Raw Limit Message: \"{}\"",
        content_text
    ));

    // Extract the reset time from the error message text.
    let Some((target_time, display)) = reset_time::parse_reset_time(log_time, content_text) else {
        return RateLimitLine::NoMatch;
    };

    // If the reset time is in the past, the limit is stale.
    if Local::now() > target_time {
        log_to_file(
            "  [STALE] Reset time targets evaluated to the past. Ignoring historical match.",
        );
        return RateLimitLine::Stale;
    }

    log_to_file(&format!(
        "  [SUCCESS] Valid active limit confirmed! Resets: {}",
        display
    ));
    RateLimitLine::Active((target_time, display))
}

/// Result of scanning a chunk of lines.
enum LineScanResult {
    /// An active rate limit was found.
    ActiveLimit((DateTime<Local>, String)),
    /// A rate limit was found, but it's in the past (stale).
    StaleLimit,
    /// No rate limit was found in the chunk.
    NoLimit,
}

/// Result of parsing a single log line.
enum RateLimitLine {
    /// The line contains an active rate limit.
    Active((DateTime<Local>, String)),
    /// The line contains a rate limit that has already expired.
    Stale,
    /// The line is not a rate limit message.
    NoMatch,
}
