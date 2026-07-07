use crate::logging::log_to_file;
use crate::watcher::reset_time;
use chrono::{DateTime, Local};
use serde_json::Value;

/// The outcome of scanning a block of text for any rate limit message.
/// Used during the initial startup scan.
#[derive(Debug)]
pub(crate) enum InitialScanResult {
    /// An active rate limit was found.
    Active((DateTime<Local>, String)),
    /// A stale (expired) rate limit was found.
    Stale,
    /// No rate limit messages were found in the scanned content.
    NoLimitFound,
}

/// Scans string content from the end for the most recent rate limit message.
///
/// # Returns
/// `Option<(DateTime<Local>, String)>` - The rate limit target time and display string if an active limit is found.
pub(crate) fn scan_content_for_limit(content: &str) -> Option<(DateTime<Local>, String)> {
    // Iterate lines in reverse because we only care about the most recent limit.
    for line in content.lines().rev() {
        match parse_rate_limit_line(line) {
            RateLimitLine::Active(limit) => return Some(limit),
            // If we find a stale limit, we can stop. Any earlier limits are, by definition, also stale.
            RateLimitLine::Stale => return None,
            RateLimitLine::NoMatch => continue,
        }
    }

    None
}

/// Scans content from the end, returning the status of the first rate limit message found.
/// This is more comprehensive than `scan_content_for_limit` because it distinguishes
/// between finding a stale limit and finding no limit at all.
pub(crate) fn scan_content_for_any_limit(content: &str) -> InitialScanResult {
    // Iterate lines in reverse because we only care about the most recent limit.
    for line in content.lines().rev() {
        match parse_rate_limit_line(line) {
            RateLimitLine::Active(limit) => return InitialScanResult::Active(limit),
            RateLimitLine::Stale => return InitialScanResult::Stale,
            RateLimitLine::NoMatch => continue,
        }
    }

    InitialScanResult::NoLimitFound
}

/// Parses a single JSON line to check if it is a rate limit error.
fn parse_rate_limit_line(line: &str) -> RateLimitLine {
    // Optimization: Avoid expensive JSON parsing by doing a cheap string check first.
    // The vast majority of lines will not be rate limit errors.
    if !line.contains("rate_limit") {
        return RateLimitLine::NoMatch;
    }
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

/// Result of parsing a single log line.
enum RateLimitLine {
    /// The line contains an active rate limit.
    Active((DateTime<Local>, String)),
    /// The line contains a rate limit that has already expired.
    Stale,
    /// The line is not a rate limit message.
    NoMatch,
}
