use crate::harness::{LimitState, LimitUpdate, MonitorSpec, ParseOutcome, TranscriptParser};
use crate::logging::log_to_file;
use crate::models::SharedAppState;
use crate::monitor::helpers::SCAN_CHUNK_SIZE;
use crate::watcher::files as watcher_files;
use chrono::{DateTime, Local};
use memchr;
use std::fs::File;
use std::io::{Read, Result as IoResult, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

fn select_newest(current: Option<LimitUpdate>, candidate: LimitUpdate) -> Option<LimitUpdate> {
    match current {
        Some(current) if current.event_time >= candidate.event_time => Some(current),
        _ => Some(candidate),
    }
}

fn apply_startup_limit(state: &SharedAppState, mut update: LimitUpdate) {
    let expired = matches!(
        &update.state,
        LimitState::Locked { target_time, .. } if *target_time <= Local::now()
    );
    if expired {
        update.state = LimitState::Clear;
    }

    if let LimitState::Locked { display, .. } = &update.state {
        log_to_file("  [MATCH] Active rate-limit row found on startup scan.");
        log_to_file(&format!(
            "  [SUCCESS] Valid limit confirmed! Resets: {display}"
        ));
        log_to_file(&format!(
            "[LOCKOUT ON STARTUP] Rate limit found. Target: {display}"
        ));
    }
    crate::monitor::runtime::apply_limit_update(state, update, false);
}

pub(super) fn newest_update_in_content(
    content: &str,
    parser: &dyn TranscriptParser,
    now: DateTime<Local>,
) -> Option<LimitUpdate> {
    content
        .lines()
        .rev()
        .filter_map(|line| match parser.parse_line(line, now) {
            ParseOutcome::Update(update) => Some(update),
            ParseOutcome::Diagnostic(diagnostic) => {
                log_to_file(&format!("[Parser] {}.", diagnostic.message()));
                None
            }
            ParseOutcome::Ignored => None,
        })
        .max_by_key(|update| update.event_time)
}

/// Scans a single file by reading it backwards in chunks. This is memory-efficient
/// and robustly finds the most recent rate-limit message by correctly handling
/// log lines that are split across chunk boundaries.
fn scan_file_backwards(
    path: &PathBuf,
    parser: &dyn TranscriptParser,
    now: DateTime<Local>,
) -> IoResult<Option<LimitUpdate>> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();

    if file_size == 0 {
        return Ok(None);
    }

    let mut buffer = Vec::with_capacity(SCAN_CHUNK_SIZE);
    // `carry_forward` holds a partial line from the start of a chunk, to be
    // prepended to the next chunk read (which is the preceding chunk in the file).
    let mut carry_forward = Vec::new();
    let mut current_pos = file_size;

    while current_pos > 0 {
        let read_start = current_pos.saturating_sub(SCAN_CHUNK_SIZE as u64);
        let read_len = (current_pos - read_start) as usize;

        // Read the next chunk from the file.
        file.seek(SeekFrom::Start(read_start))?;
        buffer.resize(read_len, 0);
        file.read_exact(&mut buffer)?;

        // Prepend the partial line from the previous iteration to complete any split lines.
        buffer.append(&mut carry_forward);

        let content_to_scan: &str;

        // If we are not at the start of the file, the beginning of our buffer might
        // be a partial line. We save it for the next iteration.
        if read_start > 0 {
            if let Some(first_newline_pos) = memchr::memchr(b'\n', &buffer) {
                carry_forward.extend_from_slice(&buffer[..first_newline_pos]);
                // Avoid allocation from `from_utf8_lossy` by using `from_utf8`.
                // If a chunk is not valid UTF-8, it cannot contain our JSON log line,
                // so we can safely skip it.
                match std::str::from_utf8(&buffer[first_newline_pos..]) {
                    Ok(s) => content_to_scan = s,
                    Err(_) => continue, // Skip chunk with invalid UTF-8.
                }
            } else {
                // The whole chunk has no newline, so it's all a partial line.
                // We move the buffer's content to carry_forward for the next iteration.
                // `swap` does this efficiently without a new allocation.
                std::mem::swap(&mut carry_forward, &mut buffer);
                current_pos = read_start;
                continue; // Nothing to scan in this iteration.
            }
        } else {
            // This is the first chunk of the file, so process everything.
            match std::str::from_utf8(&buffer) {
                Ok(s) => content_to_scan = s,
                Err(_) => break, // End of file and it's invalid, nothing more to do.
            }
        }

        // Scanning backwards lets us stop after the newest chunk containing an update.
        if let Some(update) = newest_update_in_content(content_to_scan, parser, now) {
            return Ok(Some(update));
        }

        current_pos = read_start;
    }

    Ok(None)
}

/// Performs the initial scan of all session files on startup.
pub(super) fn initial_scan(state: &SharedAppState, monitor: &MonitorSpec) -> Option<PathBuf> {
    let Some(root) = monitor.root.resolve() else {
        log_to_file("[Startup] Session root is unavailable; monitoring disabled.");
        return None;
    };
    if std::fs::create_dir_all(&root).is_err() {
        log_to_file("[Startup] Session root could not be prepared; monitoring disabled.");
        return None;
    }
    initial_scan_from(state, &root, monitor.parser.as_ref());
    Some(root)
}

pub(super) fn initial_scan_from(
    state: &SharedAppState,
    root: &Path,
    parser: &dyn TranscriptParser,
) {
    log_to_file("[Startup] Performing initial rate limit check…");
    let initial_files = watcher_files::recent_session_logs(root, SystemTime::UNIX_EPOCH);
    let mut latest_limit: Option<LimitUpdate> = None;
    let now = Local::now();

    if !initial_files.is_empty() {
        log_to_file(&format!(
            "[Startup] Scanning {} initial session file(s).",
            initial_files.len()
        ));
    }

    for (path, _) in initial_files {
        // Cache the file size regardless of the scan outcome.
        let file_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .file_size_cache
            .insert(path.clone(), file_size);

        match scan_file_backwards(&path, parser, now) {
            Ok(Some(limit)) => {
                latest_limit = select_newest(latest_limit, limit);
            }
            Ok(None) => {
                // No limit found in the entire file.
            }
            Err(e) => {
                log_to_file(&format!(
                    "[Startup] Error scanning file {}: {}",
                    path.display(),
                    e
                ));
            }
        }
    }

    // Add a blank line for readability after the initial scan section.
    log_to_file("");
    if let Some(limit) = latest_limit {
        apply_startup_limit(state, limit);
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::{apply_startup_limit, initial_scan_from, select_newest};
    use crate::harness::{LimitState, LimitUpdate, ParseOutcome, TranscriptParser};
    #[allow(unused_imports)]
    use crate::models::AppState;
    #[allow(unused_imports)]
    use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
    use std::path::{Path, PathBuf};
    #[allow(unused_imports)]
    use std::sync::{Arc, Mutex};
    use std::time::SystemTime;

    #[derive(Clone)]
    struct FixedParser {
        update: LimitUpdate,
    }

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "botsitter-monitor-startup-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    impl TranscriptParser for FixedParser {
        fn parse_line(&self, line: &str, _now: DateTime<Local>) -> ParseOutcome {
            if line == "limit" {
                ParseOutcome::Update(self.update.clone())
            } else {
                ParseOutcome::Ignored
            }
        }
    }

    #[test]
    fn injected_parser_controls_startup_state() {
        let root = TestDir::new();
        std::fs::write(root.path().join("session.jsonl"), "ignored\nlimit\n").unwrap();
        let event_time = Local::now();
        let target_time = event_time + chrono::Duration::hours(1);
        let state = Arc::new(Mutex::new(AppState::new()));
        let parser = FixedParser {
            update: LimitUpdate {
                event_time,
                state: LimitState::Locked {
                    target_time,
                    display: "fixture".into(),
                },
            },
        };

        initial_scan_from(&state, root.path(), &parser);

        assert_eq!(state.lock().unwrap().lockout_target_time, Some(target_time));
    }

    #[allow(dead_code)]
    fn limit(event_hour: u32, target_day: u32, target_hour: u32) -> LimitUpdate {
        LimitUpdate {
            event_time: Local
                .with_ymd_and_hms(2026, 7, 12, event_hour, 0, 0)
                .unwrap(),
            state: LimitState::Locked {
                target_time: Local
                    .with_ymd_and_hms(2026, 7, target_day, target_hour, 0, 5)
                    .unwrap(),
                display: "fixture".to_string(),
            },
        }
    }

    fn target(update: &LimitUpdate) -> DateTime<Local> {
        match update.state {
            LimitState::Locked { target_time, .. } => target_time,
            LimitState::Clear => panic!("expected locked update"),
        }
    }

    #[test]
    fn newer_session_event_beats_older_weekly_event_with_later_reset() {
        let older_weekly = limit(20, 14, 10);
        let newer_session = limit(23, 13, 0);
        let selected = select_newest(select_newest(None, older_weekly), newer_session).unwrap();
        assert_eq!(selected.event_time.hour(), 23);
        assert_eq!(target(&selected).day(), 13);
    }

    #[test]
    fn newer_expired_event_suppresses_older_active_event() {
        let older_active = limit(20, 14, 10);
        let mut newer_expired = limit(23, 13, 0);
        newer_expired.state = LimitState::Locked {
            target_time: Local.with_ymd_and_hms(2026, 7, 12, 23, 0, 5).unwrap(),
            display: "expired".into(),
        };
        let selected = select_newest(select_newest(None, older_active), newer_expired).unwrap();
        assert_eq!(selected.event_time.hour(), 23);
        assert_eq!(target(&selected).day(), 12);
    }

    #[test]
    fn newer_overnight_session_event_beats_older_weekly_event() {
        let older_weekly = limit(20, 14, 10);
        let newer_overnight = limit(23, 13, 2);
        let selected = select_newest(select_newest(None, older_weekly), newer_overnight).unwrap();
        assert_eq!(
            target(&selected),
            Local.with_ymd_and_hms(2026, 7, 13, 2, 0, 5).unwrap()
        );
    }

    #[test]
    fn expired_newest_startup_event_sets_watermark_without_lockout() {
        let state = Arc::new(Mutex::new(AppState::new()));
        let event_time = Local::now() - chrono::Duration::hours(2);
        apply_startup_limit(
            &state,
            LimitUpdate {
                event_time,
                state: LimitState::Locked {
                    target_time: Local::now() - chrono::Duration::hours(1),
                    display: "expired".to_string(),
                },
            },
        );
        let app = state.lock().unwrap();
        assert_eq!(app.latest_rate_limit_event_time, Some(event_time));
        assert_eq!(app.lockout_target_time, None);
        assert_eq!(app.lockout_revision, 0);
    }
}
