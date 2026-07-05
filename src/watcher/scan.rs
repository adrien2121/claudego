use crate::logging::log_to_file;
use crate::models::AppState;
use crate::watcher::reset_time;
use chrono::{DateTime, Local};
use serde_json::Value;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const CHUNK_SIZE: u64 = 65536;

pub fn scan_session_log(
    path: &Path,
    state: &mut AppState,
    should_scan: bool,
) -> Option<(DateTime<Local>, String)> {
    let file_name = path
        .file_name()
        .map_or("Unknown", |name| name.to_str().unwrap_or("Unknown"));
    let mut file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();

    state.file_size_cache.insert(path.to_path_buf(), size);

    if size == 0 || !should_scan {
        return None;
    }

    if state.show_logs {
        log_to_file(&format!(
            "[Watcher] Deep scanning file updates for session: {} (Size: {} bytes)",
            file_name, size
        ));
    }

    scan_file_from_end(&mut file, size)
}

fn scan_file_from_end(file: &mut File, size: u64) -> Option<(DateTime<Local>, String)> {
    if size > 0 {
        let mut last_byte = [0u8; 1];
        if file.seek(SeekFrom::End(-1)).is_ok() && file.read_exact(&mut last_byte).is_ok() {
            if last_byte[0] != b'\n' {
                return None; // File is mid-write, ignore it for now
            }
        }
    }

    let mut offset = size;
    let mut leftover = Vec::new();

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

        if !leftover.is_empty() {
            buf.extend(&leftover);
        }

        let contents = String::from_utf8_lossy(&buf);
        let mut lines: Vec<&str> = contents.lines().collect();

        if offset > 0 && !lines.is_empty() {
            leftover = lines.remove(0).as_bytes().to_vec();
        } else {
            leftover.clear();
        }

        match scan_lines_newest_first(lines) {
            LineScanResult::ActiveLimit(limit) => return Some(limit),
            LineScanResult::StaleLimit => break,
            LineScanResult::NoLimit => {}
        }
    }

    None
}

fn scan_lines_newest_first(lines: Vec<&str>) -> LineScanResult {
    for line in lines.into_iter().rev() {
        match parse_rate_limit_line(line) {
            RateLimitLine::NoMatch => continue,
            RateLimitLine::Stale => return LineScanResult::StaleLimit,
            RateLimitLine::Active(limit) => return LineScanResult::ActiveLimit(limit),
        }
    }

    LineScanResult::NoLimit
}

fn parse_rate_limit_line(line: &str) -> RateLimitLine {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return RateLimitLine::NoMatch;
    };

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

    let Some((target_time, display)) = reset_time::parse_reset_time(log_time, content_text) else {
        return RateLimitLine::NoMatch;
    };

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

enum LineScanResult {
    ActiveLimit((DateTime<Local>, String)),
    StaleLimit,
    NoLimit,
}

enum RateLimitLine {
    Active((DateTime<Local>, String)),
    Stale,
    NoMatch,
}
