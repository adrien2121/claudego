use crate::harness::{LimitState, LimitUpdate, ParseOutcome, TranscriptParser};
use chrono::{DateTime, Local};
use serde_json::Value;

use super::reset_time;

/// Holds structured information about a detected rate limit.
#[derive(Debug)]
pub struct RateLimitInfo {
    pub event_time: DateTime<Local>,
    pub target_time: DateTime<Local>,
    pub display_str: String,
    pub raw_message: String,
}

pub(crate) fn rate_limit_from_message(
    event_time: DateTime<Local>,
    content_text: &str,
) -> Option<RateLimitInfo> {
    let (target_time, display_str) = reset_time::parse_reset_time(event_time, content_text)?;
    Some(RateLimitInfo {
        event_time,
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

    match rate_limit_from_message(log_time, content_text) {
        Some(limit) => RateLimitLine::Found(limit),
        None => RateLimitLine::NoMatch,
    }
}

/// Result of parsing a single log line.
enum RateLimitLine {
    Found(RateLimitInfo),
    /// The line is not a rate limit message.
    NoMatch,
}

pub struct TranscriptLineParser;

impl TranscriptParser for TranscriptLineParser {
    fn parse_line(&self, line: &str, _now: DateTime<Local>) -> ParseOutcome {
        match parse_rate_limit_line(line) {
            RateLimitLine::Found(limit) => ParseOutcome::Update(LimitUpdate {
                event_time: limit.event_time,
                state: LimitState::Locked {
                    target_time: limit.target_time,
                    display: limit.display_str,
                },
            }),
            RateLimitLine::NoMatch => ParseOutcome::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TranscriptLineParser;
    use crate::harness::{LimitState, ParseOutcome, TranscriptParser};
    use chrono::{Local, TimeZone};

    #[test]
    fn expired_limit_retains_event_and_target_times() {
        let row = r#"{"timestamp":"2026-07-12T23:00:00-04:00","error":"rate_limit","message":{"content":[{"type":"text","text":"Claude limit reached; resets 2am"}]}}"#;
        let ParseOutcome::Update(limit) = TranscriptLineParser.parse_line(row, Local::now()) else {
            panic!("expected parsed rate-limit event");
        };
        assert_eq!(
            limit.event_time,
            Local.with_ymd_and_hms(2026, 7, 12, 23, 0, 0).unwrap()
        );
        let LimitState::Locked { target_time, .. } = limit.state else {
            panic!("expected locked state");
        };
        assert_eq!(
            target_time,
            Local.with_ymd_and_hms(2026, 7, 13, 2, 0, 5).unwrap()
        );
    }

    #[test]
    fn newest_valid_row_wins_within_one_file_even_when_expired() {
        let content = concat!(
            "{\"timestamp\":\"2026-07-12T20:00:00-04:00\",\"error\":\"rate_limit\",\"message\":{\"content\":[{\"text\":\"Claude limit reached; resets Jul 14 at 10am\"}]}}\n",
            "{\"timestamp\":\"2026-07-12T23:00:00-04:00\",\"error\":\"rate_limit\",\"message\":{\"content\":[{\"text\":\"Claude limit reached; resets 11pm\"}]}}\n"
        );
        let limit = content
            .lines()
            .filter_map(
                |line| match TranscriptLineParser.parse_line(line, Local::now()) {
                    ParseOutcome::Update(update) => Some(update),
                    ParseOutcome::Ignored | ParseOutcome::Diagnostic(_) => None,
                },
            )
            .max_by_key(|update| update.event_time)
            .expect("expected newest event");
        assert_eq!(
            limit.event_time,
            Local.with_ymd_and_hms(2026, 7, 12, 23, 0, 0).unwrap()
        );
    }
}
