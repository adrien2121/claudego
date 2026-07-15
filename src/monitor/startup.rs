use crate::logging::log_to_file;
use crate::models::SharedAppState;
use crate::monitor::helpers::SCAN_CHUNK_SIZE;
use crate::watcher::files as watcher_files;
use crate::watcher::scan::{InitialScanResult, RateLimitInfo};
use chrono::Local;
use memchr;
use std::fs::File;
use std::io::{Read, Result as IoResult, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::SystemTime;

fn select_newest(
    current: Option<RateLimitInfo>,
    candidate: RateLimitInfo,
) -> Option<RateLimitInfo> {
    match current {
        Some(current) if current.event_time >= candidate.event_time => Some(current),
        _ => Some(candidate),
    }
}

fn apply_startup_limit(state: &SharedAppState, limit: RateLimitInfo) {
    let active = limit.target_time > Local::now();
    if active {
        log_to_file("  [MATCH] Active 'rate_limit' row found on startup scan.");
        log_to_file(&format!(
            "  [SUCCESS] Valid active limit confirmed! Resets: {}",
            limit.display_str
        ));
        log_to_file(&format!(
            "[LOCKOUT ON STARTUP] Rate limit found. Target: {}",
            limit.display_str
        ));
    }

    let mut app = state.lock().unwrap_or_else(|e| e.into_inner());
    app.latest_rate_limit_event_time = Some(limit.event_time);
    app.lockout_target_time = active.then_some(limit.target_time);
}

/// Scans a single file by reading it backwards in chunks. This is memory-efficient
/// and robustly finds the most recent rate-limit message by correctly handling
/// log lines that are split across chunk boundaries.
fn scan_file_backwards(path: &PathBuf) -> IoResult<InitialScanResult> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();

    if file_size == 0 {
        return Ok(InitialScanResult::NoLimitFound);
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

        // We must use `scan_content_for_any_limit` to correctly stop at the
        // first (most recent) limit, even if it's stale.
        match crate::watcher::scan::scan_content_for_any_limit(content_to_scan) {
            InitialScanResult::Found(limit) => return Ok(InitialScanResult::Found(limit)),
            InitialScanResult::NoLimitFound => { /* Continue to the previous chunk */ }
        }

        current_pos = read_start;
    }

    Ok(InitialScanResult::NoLimitFound)
}

/// Performs the initial scan of all session files on startup.
pub(super) fn initial_scan(state: &SharedAppState) {
    log_to_file("[Startup] Performing initial rate limit check…");
    let initial_files = watcher_files::claude_projects_root()
        .map(|root| watcher_files::recent_session_logs(&root, SystemTime::UNIX_EPOCH))
        .unwrap_or_default();
    let mut latest_limit: Option<RateLimitInfo> = None;

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

        match scan_file_backwards(&path) {
            Ok(InitialScanResult::Found(limit)) => {
                latest_limit = select_newest(latest_limit, limit);
            }
            Ok(InitialScanResult::NoLimitFound) => {
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
    use super::{apply_startup_limit, select_newest};
    #[allow(unused_imports)]
    use crate::models::AppState;
    use crate::watcher::scan::RateLimitInfo;
    #[allow(unused_imports)]
    use chrono::{Datelike, Local, TimeZone, Timelike};
    #[allow(unused_imports)]
    use std::sync::{Arc, Mutex};

    #[allow(dead_code)]
    fn limit(event_hour: u32, target_day: u32, target_hour: u32) -> RateLimitInfo {
        RateLimitInfo {
            event_time: Local
                .with_ymd_and_hms(2026, 7, 12, event_hour, 0, 0)
                .unwrap(),
            target_time: Local
                .with_ymd_and_hms(2026, 7, target_day, target_hour, 0, 5)
                .unwrap(),
            display_str: "fixture".to_string(),
            raw_message: "fixture".to_string(),
        }
    }

    #[test]
    fn newer_session_event_beats_older_weekly_event_with_later_reset() {
        let older_weekly = limit(20, 14, 10);
        let newer_session = limit(23, 13, 0);
        let selected = select_newest(select_newest(None, older_weekly), newer_session).unwrap();
        assert_eq!(selected.event_time.hour(), 23);
        assert_eq!(selected.target_time.day(), 13);
    }

    #[test]
    fn newer_expired_event_suppresses_older_active_event() {
        let older_active = limit(20, 14, 10);
        let mut newer_expired = limit(23, 13, 0);
        newer_expired.target_time = Local.with_ymd_and_hms(2026, 7, 12, 23, 0, 5).unwrap();
        let selected = select_newest(select_newest(None, older_active), newer_expired).unwrap();
        assert_eq!(selected.event_time.hour(), 23);
        assert_eq!(selected.target_time.day(), 12);
    }

    #[test]
    fn newer_overnight_session_event_beats_older_weekly_event() {
        let older_weekly = limit(20, 14, 10);
        let newer_overnight = limit(23, 13, 2);
        let selected = select_newest(select_newest(None, older_weekly), newer_overnight).unwrap();
        assert_eq!(
            selected.target_time,
            Local.with_ymd_and_hms(2026, 7, 13, 2, 0, 5).unwrap()
        );
    }

    #[test]
    fn expired_newest_startup_event_sets_watermark_without_lockout() {
        let state = Arc::new(Mutex::new(AppState::new()));
        let event_time = Local::now() - chrono::Duration::hours(2);
        apply_startup_limit(
            &state,
            RateLimitInfo {
                event_time,
                target_time: Local::now() - chrono::Duration::hours(1),
                display_str: "expired".to_string(),
                raw_message: "expired".to_string(),
            },
        );
        let app = state.lock().unwrap();
        assert_eq!(app.latest_rate_limit_event_time, Some(event_time));
        assert_eq!(app.lockout_target_time, None);
        assert_eq!(app.lockout_revision, 0);
    }
}
