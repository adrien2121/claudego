use chrono::{DateTime, Local};
use serde_json::Value;

use super::reset_time;

/// Holds structured information about a detected active rate limit.
#[derive(Debug)]
pub struct ActiveRateLimitInfo {
    pub target_time: DateTime<Local>,
    pub display_str: String,
    pub raw_message: String,
}

/// The outcome of scanning a block of text for any rate limit message.
/// Used during the initial startup scan.
#[derive(Debug)]
pub(crate) enum InitialScanResult {
    /// An active rate limit was found.
    Active(ActiveRateLimitInfo),
    /// A stale (expired) rate limit was found.
    Stale,
    /// No rate limit messages were found in the scanned content.
    NoLimitFound,
}

/// Scans content from the end and returns the result of the first (most recent)
/// rate limit message found.
fn scan_content_for_most_recent_limit(content: &str) -> RateLimitLine {
    for line in content.lines().rev() {
        let result = parse_rate_limit_line(line);
        if !matches!(result, RateLimitLine::NoMatch) {
            return result;
        }
    }
    RateLimitLine::NoMatch
}

/// Scans string content from the end for the most recent rate limit message.
///
/// # Returns
/// `Option<ActiveRateLimitInfo>` - Structured info if an active limit is found.
pub(crate) fn scan_content_for_limit(content: &str) -> Option<ActiveRateLimitInfo> {
    scan_content_for_most_recent_limit(content).into()
}

/// Scans content from the end, returning the status of the first rate limit message found.
/// This is more comprehensive than `scan_content_for_limit` because it distinguishes
/// between finding a stale limit and finding no limit at all.
pub(crate) fn scan_content_for_any_limit(content: &str) -> InitialScanResult {
    scan_content_for_most_recent_limit(content).into()
}

pub(crate) fn active_rate_limit_from_message(
    log_time: DateTime<Local>,
    content_text: &str,
) -> Option<ActiveRateLimitInfo> {
    let (target_time, display_str) = reset_time::parse_reset_time(log_time, content_text)?;
    if Local::now() > target_time {
        return None;
    }

    Some(ActiveRateLimitInfo {
        target_time,
        display_str,
        raw_message: content_text.to_string(),
    })
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

    if value["error"].as_str() != Some("rate_limit") {
        return RateLimitLine::NoMatch;
    }

    // --- All checks below this point are for a potential rate_limit error ---
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

    let Some(limit) = active_rate_limit_from_message(log_time, content_text) else {
        return RateLimitLine::Stale;
    };

    RateLimitLine::Active(limit)
}

/// Result of parsing a single log line.
enum RateLimitLine {
    /// The line contains an active rate limit.
    Active(ActiveRateLimitInfo),
    /// The line contains a rate limit that has already expired.
    Stale,
    /// The line is not a rate limit message.
    NoMatch,
}

impl From<RateLimitLine> for Option<ActiveRateLimitInfo> {
    fn from(result: RateLimitLine) -> Self {
        match result {
            RateLimitLine::Active(limit) => Some(limit),
            _ => None,
        }
    }
}

impl From<RateLimitLine> for InitialScanResult {
    fn from(result: RateLimitLine) -> Self {
        match result {
            RateLimitLine::Active(limit) => InitialScanResult::Active(limit),
            RateLimitLine::Stale => InitialScanResult::Stale,
            RateLimitLine::NoMatch => InitialScanResult::NoLimitFound,
        }
    }
}
