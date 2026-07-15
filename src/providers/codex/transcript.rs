use crate::harness::{LimitState, LimitUpdate, ParseDiagnostic, ParseOutcome, TranscriptParser};
use chrono::{DateTime, Local, Utc};
use serde::Deserialize;

pub struct CodexTranscriptParser;

#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    kind: Option<String>,
    payload: Option<EnvelopePayload>,
}

#[derive(Deserialize)]
struct EnvelopePayload {
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Deserialize)]
struct Record {
    timestamp: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    payload: Option<Payload>,
}

#[derive(Deserialize)]
struct Payload {
    #[serde(rename = "type")]
    kind: Option<String>,
    rate_limits: Option<RateLimits>,
}

#[derive(Deserialize)]
struct RateLimits {
    primary: Option<Window>,
    secondary: Option<Window>,
}

#[derive(Deserialize)]
struct Window {
    used_percent: Option<f64>,
    resets_at: Option<i64>,
}

impl TranscriptParser for CodexTranscriptParser {
    fn parse_line(&self, line: &str, now: DateTime<Local>) -> ParseOutcome {
        let Ok(envelope) = serde_json::from_str::<Envelope>(line) else {
            return ParseOutcome::Ignored;
        };
        if envelope.kind.as_deref() != Some("event_msg")
            || envelope
                .payload
                .as_ref()
                .and_then(|payload| payload.kind.as_deref())
                != Some("token_count")
        {
            return ParseOutcome::Ignored;
        }

        let Ok(record) = serde_json::from_str::<Record>(line) else {
            return ParseOutcome::Diagnostic(ParseDiagnostic::MalformedRecord);
        };
        let Some(payload) = record.payload else {
            return ParseOutcome::Diagnostic(ParseDiagnostic::MalformedRecord);
        };
        if record.kind.as_deref() != Some("event_msg")
            || payload.kind.as_deref() != Some("token_count")
        {
            return ParseOutcome::Ignored;
        }
        let Some(rate_limits) = payload.rate_limits else {
            return ParseOutcome::Ignored;
        };
        let Ok(event_time) =
            DateTime::parse_from_rfc3339(record.timestamp.as_deref().unwrap_or(""))
                .map(|time| time.with_timezone(&Local))
        else {
            return ParseOutcome::Diagnostic(ParseDiagnostic::MissingEventTimestamp);
        };

        let saturated = [rate_limits.primary.as_ref(), rate_limits.secondary.as_ref()]
            .into_iter()
            .flatten()
            .filter(|window| window.used_percent.is_some_and(|percent| percent >= 100.0));
        let mut target_time: Option<DateTime<Local>> = None;
        for window in saturated {
            let Some(target) = window
                .resets_at
                .and_then(|timestamp| DateTime::<Utc>::from_timestamp(timestamp, 0))
                .map(|time| time.with_timezone(&Local))
                .filter(|target| *target > now)
            else {
                return ParseOutcome::Diagnostic(ParseDiagnostic::SaturatedWithoutFutureReset);
            };
            target_time = Some(target_time.map_or(target, |current| current.max(target)));
        }

        let state = match target_time {
            Some(target_time) => LimitState::Locked {
                display: target_time.to_rfc3339(),
                target_time,
            },
            None => LimitState::Clear,
        };
        ParseOutcome::Update(LimitUpdate { event_time, state })
    }
}

#[cfg(test)]
mod tests {
    use super::CodexTranscriptParser;
    use crate::harness::{LimitState, ParseDiagnostic, ParseOutcome, TranscriptParser};
    use crate::models::AppState;
    use crate::monitor::record_limit_update;
    use chrono::{Local, TimeZone};
    use std::sync::{Arc, Mutex};

    const PRIMARY: &str = include_str!("../../../tests/fixtures/codex/primary-saturated.jsonl");
    const CLEARED: &str = include_str!("../../../tests/fixtures/codex/cleared.jsonl");

    fn now() -> chrono::DateTime<Local> {
        Local.with_ymd_and_hms(2026, 7, 15, 0, 0, 0).unwrap()
    }

    #[test]
    fn parses_codex_limit_snapshots() {
        let parser = CodexTranscriptParser;
        let cases = [
            (PRIMARY, Some(1_784_672_717)),
            (
                include_str!("../../../tests/fixtures/codex/secondary-saturated.jsonl"),
                Some(1_784_673_600),
            ),
            (
                include_str!("../../../tests/fixtures/codex/both-saturated.jsonl"),
                Some(1_784_673_600),
            ),
            (CLEARED, None),
        ];

        for (line, expected_reset) in cases {
            let ParseOutcome::Update(update) = parser.parse_line(line.trim(), now()) else {
                panic!("expected update");
            };
            match (update.state, expected_reset) {
                (LimitState::Locked { target_time, .. }, Some(timestamp)) => {
                    assert_eq!(target_time.timestamp(), timestamp);
                }
                (LimitState::Clear, None) => {}
                result => panic!("unexpected state: {result:?}"),
            }
        }
    }

    #[test]
    fn null_snapshot_does_not_clear_state() {
        assert_eq!(
            CodexTranscriptParser.parse_line(
                include_str!("../../../tests/fixtures/codex/null-rate-limits.jsonl").trim(),
                now(),
            ),
            ParseOutcome::Ignored,
        );
    }

    #[test]
    fn targeted_malformed_records_have_sanitized_diagnostics() {
        assert_eq!(
            CodexTranscriptParser.parse_line(
                include_str!("../../../tests/fixtures/codex/malformed.jsonl").trim(),
                now(),
            ),
            ParseOutcome::Diagnostic(ParseDiagnostic::MissingEventTimestamp),
        );
        assert_eq!(
            CodexTranscriptParser.parse_line(
                r#"{"timestamp":"2026-07-15T01:58:37.161Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":100.0,"resets_at":null}}}}"#,
                now(),
            ),
            ParseOutcome::Diagnostic(ParseDiagnostic::SaturatedWithoutFutureReset),
        );
        assert_eq!(
            CodexTranscriptParser.parse_line(
                r#"{"type":"event_msg","payload":{"type":"token_count","rate_limits":"PRIVATE"}}"#,
                now(),
            ),
            ParseOutcome::Diagnostic(ParseDiagnostic::MalformedRecord),
        );
    }

    #[test]
    fn malformed_unrelated_records_are_ignored() {
        assert_eq!(
            CodexTranscriptParser.parse_line(r#"{"type":"other","payload":"PRIVATE"}"#, now()),
            ParseOutcome::Ignored,
        );
        assert_eq!(
            CodexTranscriptParser.parse_line("PRIVATE not json", now()),
            ParseOutcome::Ignored,
        );
    }

    #[test]
    fn newer_clear_wins_over_stale_saturation() {
        let parser = CodexTranscriptParser;
        let ParseOutcome::Update(clear) = parser.parse_line(CLEARED.trim(), now()) else {
            panic!("expected clear update");
        };
        let clear_event_time = clear.event_time;
        let ParseOutcome::Update(stale) = parser.parse_line(
            include_str!("../../../tests/fixtures/codex/stale.jsonl").trim(),
            now(),
        ) else {
            panic!("expected stale lock update");
        };
        let state = Arc::new(Mutex::new(AppState::new()));

        record_limit_update(&state, clear);
        record_limit_update(&state, stale);

        let state = state.lock().unwrap();
        assert_eq!(state.lockout_target_time, None);
        assert_eq!(state.latest_rate_limit_event_time, Some(clear_event_time));
    }
}
