use crate::harness::{LimitState, LimitUpdate, ResumeOutcome, ResumeSink, TranscriptParser};
use crate::logging::log_to_file;
use crate::models::{output_is_hot, SharedAppState};
use crate::monitor::helpers::{DEFER_SCAN_INTERVAL, PTY_BUSY_THRESHOLD};
use chrono::{DateTime, Local};
use memmap2;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::time::sleep;

/// When scanning a modified file, re-scan this many bytes from the end of the
/// previously seen content to be robust against missed initial detections.
const SCAN_OVERLAP_BYTES: u64 = 4096;
const SCAN_FAILURE_LOG_INTERVAL: Duration = Duration::from_secs(30);
const RESUME_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];

trait ResumeAttempt {
    fn resume(&self) -> ResumeOutcome;
}

struct StableScan {
    new_size: u64,
    update: Option<LimitUpdate>,
}

#[derive(Debug)]
enum ScanFailure {
    Open(std::io::Error),
    Metadata(std::io::Error),
    Mmap(std::io::Error),
    InvalidUtf8,
    Changed,
}

impl std::fmt::Display for ScanFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open(error) => write!(formatter, "open failed: {error}"),
            Self::Metadata(error) => write!(formatter, "metadata failed: {error}"),
            Self::Mmap(error) => write!(formatter, "mmap failed: {error}"),
            Self::InvalidUtf8 => formatter.write_str("new content is invalid UTF-8"),
            Self::Changed => formatter.write_str("file changed during scan; retrying"),
        }
    }
}

fn scan_mmap_range(
    file: &File,
    old_size: u64,
    new_size: u64,
    parser: &dyn TranscriptParser,
) -> Result<Option<LimitUpdate>, ScanFailure> {
    // Safety: the file is opened read-only and the captured range is bounds-checked
    // before slicing, so concurrent truncation is reported instead of panicking.
    let mmap = unsafe { memmap2::Mmap::map(file) }.map_err(ScanFailure::Mmap)?;
    if new_size > mmap.len() as u64 {
        return Err(ScanFailure::Changed);
    }
    let mut scan_start = old_size.saturating_sub(SCAN_OVERLAP_BYTES).min(new_size) as usize;
    let new_size = new_size as usize;
    while scan_start < new_size && mmap[scan_start] & 0b1100_0000 == 0b1000_0000 {
        scan_start += 1;
    }
    let scan_slice = &mmap[scan_start..new_size];
    let scan_str = std::str::from_utf8(scan_slice).map_err(|_| ScanFailure::InvalidUtf8)?;
    Ok(super::startup::newest_update_in_content(
        scan_str,
        parser,
        Local::now(),
    ))
}

fn scan_stable_range_with(
    path: &Path,
    old_size: u64,
    parser: &dyn TranscriptParser,
    after_scan: impl FnOnce(),
) -> Result<StableScan, ScanFailure> {
    let file = File::open(path).map_err(ScanFailure::Open)?;
    let before = file.metadata().map_err(ScanFailure::Metadata)?;
    let new_size = before.len();
    let modified = before.modified().map_err(ScanFailure::Metadata)?;
    let update = if new_size > old_size {
        scan_mmap_range(&file, old_size, new_size, parser)?
    } else {
        None
    };
    after_scan();
    let after = file.metadata().map_err(ScanFailure::Metadata)?;
    if after.len() != new_size || after.modified().map_err(ScanFailure::Metadata)? != modified {
        return Err(ScanFailure::Changed);
    }
    Ok(StableScan { new_size, update })
}

fn scan_stable_range(
    path: &Path,
    old_size: u64,
    parser: &dyn TranscriptParser,
) -> Result<StableScan, ScanFailure> {
    scan_stable_range_with(path, old_size, parser, || {})
}

fn scan_one_path(
    path: &Path,
    state: &SharedAppState,
    parser: &dyn TranscriptParser,
) -> Result<StableScan, ScanFailure> {
    let old_size = state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .file_size_cache
        .get(path)
        .copied()
        .unwrap_or(0);
    let scan = scan_stable_range(path, old_size, parser)?;
    state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .file_size_cache
        .insert(path.to_path_buf(), scan.new_size);
    Ok(scan)
}

#[cfg(test)]
fn scan_mmap_range_for_test(
    file: &File,
    old_size: u64,
    new_size: u64,
    parser: &dyn TranscriptParser,
) -> Result<(), ScanFailure> {
    scan_mmap_range(file, old_size, new_size, parser).map(|_| ())
}

impl ResumeAttempt for &dyn ResumeSink {
    fn resume(&self) -> ResumeOutcome {
        (*self).resume()
    }
}

/// Scans a set of changed paths and updates the application state.
pub(super) async fn scan_and_update_state(
    paths: HashSet<PathBuf>,
    state: &SharedAppState,
    next_log_time: &mut Instant,
    failure_logs: &mut HashMap<PathBuf, Instant>,
    parser: &dyn TranscriptParser,
) {
    // --- Busy-Wait Logic ---
    // Before scanning, check if the PTY is actively streaming. If so, wait.
    // This prevents the file scan's I/O from competing with the PTY reader thread,
    // which is a direct cause of the "stalled stream" error.
    let activity_tracker = {
        let app = state.lock().unwrap();
        app.last_output_activity.clone()
    };
    loop {
        if output_is_hot(&activity_tracker, PTY_BUSY_THRESHOLD) {
            log_to_file(&format!(
                "Child output is currently streaming. Deferring file scan for {:?}.",
                DEFER_SCAN_INTERVAL,
            ));
            sleep(DEFER_SCAN_INTERVAL).await;
        } else {
            break; // PTY is quiet, proceed with the scan.
        }
    }

    // Log which files are being scanned.
    let path_names: Vec<_> = paths.iter().map(|p| p.display().to_string()).collect();
    log_to_file(&format!(
        "[File Event] Triggering scan. Changed files:\n{}",
        path_names.join("\n")
    ));

    for path in paths {
        let StableScan { update, .. } = match scan_one_path(&path, state, parser) {
            Ok(scan) => {
                failure_logs.remove(&path);
                scan
            }
            Err(failure) => {
                let now = Instant::now();
                if failure_logs
                    .get(&path)
                    .is_none_or(|last| now.duration_since(*last) >= SCAN_FAILURE_LOG_INTERVAL)
                {
                    log_to_file(&format!("[Scan Error] {}: {failure}", path.display()));
                    failure_logs.insert(path, now);
                }
                continue;
            }
        };
        if let Some(update) = update {
            let display = match &update.state {
                LimitState::Locked { display, .. } => Some(display.clone()),
                LimitState::Clear => None,
            };
            if apply_limit_update(state, update, true) {
                if let Some(display) = display {
                    log_to_file("  [MATCH] Active rate-limit row found! Parsing contents...");
                    log_to_file(&format!(
                        "  [SUCCESS] Valid active limit confirmed! Resets: {display}"
                    ));
                    log_to_file(&format!(
                        "[LOCKOUT DETECTED] Rate limit hit from file watcher. Target: {display}"
                    ));
                } else {
                    log_to_file("[LIMIT UPDATE] Transcript state cleared from file watcher.");
                }
                *next_log_time = Instant::now();
            }
        }
    }
}

pub(super) fn record_lockout(
    state: &SharedAppState,
    limit_info: crate::watcher::scan::RateLimitInfo,
    source: &str,
) {
    log_to_file(&format!(
        "[LOCKOUT DETECTED] Rate limit hit from {source}. Target: {}",
        limit_info.display_str
    ));
    apply_limit_update(
        state,
        LimitUpdate {
            event_time: limit_info.event_time,
            state: LimitState::Locked {
                target_time: limit_info.target_time,
                display: limit_info.display_str,
            },
        },
        true,
    );
}

pub(crate) fn apply_limit_update(
    state: &SharedAppState,
    update: LimitUpdate,
    increment_revision: bool,
) -> bool {
    let mut app = state.lock().unwrap_or_else(|e| e.into_inner());
    if app
        .latest_rate_limit_event_time
        .is_some_and(|watermark| watermark >= update.event_time)
    {
        return false;
    }

    app.latest_rate_limit_event_time = Some(update.event_time);
    app.lockout_target_time = match update.state {
        LimitState::Locked { target_time, .. } => Some(target_time),
        LimitState::Clear => None,
    };
    app.resume_exhausted_revision = None;
    if increment_revision {
        app.lockout_revision = app.lockout_revision.wrapping_add(1);
    }
    true
}

/// Handles the logic for when a lockout expires.
pub(super) async fn handle_expiry(
    state: &SharedAppState,
    resume_sink: &dyn ResumeSink,
    expired_target: DateTime<Local>,
) {
    handle_expiry_with(state, expired_target, resume_sink).await;
}

async fn handle_expiry_with<R: ResumeAttempt>(
    state: &SharedAppState,
    expired_target: DateTime<Local>,
    resume_target: R,
) {
    let expired_revision = state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .lockout_revision;
    log_to_file("[Trigger] Reset time reached. Injecting 'continue' command…");
    let mut outcome = resume_target.resume();
    for delay in RESUME_RETRY_DELAYS {
        match outcome {
            ResumeOutcome::Sent | ResumeOutcome::AmbiguousFailure(_) => break,
            ResumeOutcome::DefiniteFailure(ref error) => {
                log_to_file(&format!("[Resume Error] {error}"));
                sleep(delay).await;
                outcome = resume_target.resume();
            }
        }
    }

    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    let still_current =
        s.lockout_target_time == Some(expired_target) && s.lockout_revision == expired_revision;
    match outcome {
        ResumeOutcome::Sent if still_current => {
            log_to_file("[System] Resume command sent.");
            s.lockout_target_time = None;
            s.file_size_cache.clear();
            log_to_file("[System] Resuming passive file monitoring.");
        }
        ResumeOutcome::Sent => log_to_file("[System] Expiry handled, but a newer lockout has already been detected. State not cleared."),
        ResumeOutcome::DefiniteFailure(error) | ResumeOutcome::AmbiguousFailure(error) => {
            log_to_file(&format!("[Resume Error] {error}"));
            if still_current {
                s.resume_exhausted_revision = Some(expired_revision);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_limit_update, handle_expiry_with, record_lockout, scan_and_update_state,
        scan_mmap_range_for_test, scan_one_path, ScanFailure,
    };
    use crate::harness::{LimitState, LimitUpdate, ResumeOutcome};
    use crate::models::AppState;
    use crate::watcher::scan::RateLimitInfo;
    use crate::watcher::scan::TranscriptLineParser;
    use chrono::{Duration, Local};
    use std::collections::HashSet;
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    const PARSER: TranscriptLineParser = TranscriptLineParser;

    fn unique_path(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "botsitter-runtime-{}-{unique}-{name}",
            std::process::id()
        ))
    }

    fn state_with_cursor(path: std::path::PathBuf, cursor: u64) -> Arc<Mutex<AppState>> {
        let state = Arc::new(Mutex::new(AppState::new()));
        state.lock().unwrap().file_size_cache.insert(path, cursor);
        state
    }

    #[test]
    fn failed_scan_does_not_advance_cursor() {
        let path = unique_path("missing.jsonl");
        let state = state_with_cursor(path.clone(), 17);
        let result = scan_one_path(&path, &state, &PARSER);
        assert!(result.is_err());
        assert_eq!(state.lock().unwrap().file_size_cache[&path], 17);
    }

    #[test]
    fn truncated_captured_range_is_retryable_not_a_panic() {
        let path = unique_path("truncated.jsonl");
        std::fs::write(&path, b"0123456789").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let result = scan_mmap_range_for_test(&file, 0, 20, &PARSER);
        assert!(matches!(result, Err(ScanFailure::Changed)));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn scan_append_is_retryable_then_commits_and_detects_limit() {
        let path = unique_path("append.jsonl");
        std::fs::write(&path, b"{\"type\":\"baseline\"}\n").unwrap();
        let old_size = std::fs::metadata(&path).unwrap().len();
        let state = state_with_cursor(path.clone(), old_size);
        let now = Local::now();
        let reset = (now + Duration::hours(2)).format("%-I:%M%P");
        let row = format!(
            "{{\"timestamp\":\"{}\",\"error\":\"rate_limit\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"You've hit your session limit · resets {reset} (America/Toronto)\"}}]}}}}\n",
            now.to_rfc3339()
        );

        let first = super::scan_stable_range_with(&path, old_size, &PARSER, || {
            OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap()
                .write_all(row.as_bytes())
                .unwrap();
        });
        assert!(matches!(first, Err(ScanFailure::Changed)));
        assert_eq!(state.lock().unwrap().file_size_cache[&path], old_size);

        let retry = scan_one_path(&path, &state, &PARSER).unwrap();
        assert!(retry.update.is_some());
        assert_eq!(
            state.lock().unwrap().file_size_cache[&path],
            std::fs::metadata(&path).unwrap().len()
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn scan_truncation_is_retryable_then_commits_reduced_length() {
        let path = unique_path("truncate.jsonl");
        std::fs::write(&path, b"0123456789").unwrap();
        let state = state_with_cursor(path.clone(), 4);

        let first = super::scan_stable_range_with(&path, 4, &PARSER, || {
            OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(3)
                .unwrap();
        });
        assert!(matches!(first, Err(ScanFailure::Changed)));
        assert_eq!(state.lock().unwrap().file_size_cache[&path], 4);

        scan_one_path(&path, &state, &PARSER).unwrap();
        assert_eq!(state.lock().unwrap().file_size_cache[&path], 3);
        std::fs::remove_file(path).unwrap();
    }

    #[derive(Clone)]
    struct ScriptedResume {
        attempts: Arc<AtomicUsize>,
        outcomes: Arc<Mutex<Vec<ResumeOutcome>>>,
    }

    impl ScriptedResume {
        fn outcomes(outcomes: Vec<ResumeOutcome>) -> Self {
            Self {
                attempts: Arc::new(AtomicUsize::new(0)),
                outcomes: Arc::new(Mutex::new(outcomes)),
            }
        }

        fn definite_failures(count: usize) -> Self {
            Self::outcomes(vec![
                ResumeOutcome::DefiniteFailure(
                    "runner unavailable".to_string()
                );
                count
            ])
        }

        fn resume(&self) -> ResumeOutcome {
            self.attempts.fetch_add(1, Ordering::Relaxed);
            self.outcomes.lock().unwrap().remove(0)
        }

        fn attempts(&self) -> usize {
            self.attempts.load(Ordering::Relaxed)
        }
    }

    impl super::ResumeAttempt for ScriptedResume {
        fn resume(&self) -> ResumeOutcome {
            self.resume()
        }
    }

    fn shared_state_with_lockout(
        target: chrono::DateTime<Local>,
        revision: u64,
    ) -> Arc<Mutex<AppState>> {
        let mut app = AppState::new();
        app.lockout_target_time = Some(target);
        app.lockout_revision = revision;
        Arc::new(Mutex::new(app))
    }

    fn limit(event: chrono::DateTime<Local>, target: chrono::DateTime<Local>) -> RateLimitInfo {
        RateLimitInfo {
            event_time: event,
            target_time: target,
            display_str: target.to_rfc3339(),
            raw_message: "fixture".to_string(),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn three_retries_retain_lockout_and_mark_revision_exhausted() {
        let target = Local::now();
        let state = shared_state_with_lockout(target, 3);
        let resume = ScriptedResume::definite_failures(4);
        let task_state = Arc::clone(&state);
        let task_resume = resume.clone();
        let task =
            tokio::spawn(async move { handle_expiry_with(&task_state, target, task_resume).await });
        tokio::time::advance(std::time::Duration::from_secs(7)).await;
        task.await.unwrap();

        let app = state.lock().unwrap();
        assert_eq!(resume.attempts(), 4);
        assert_eq!(app.lockout_target_time, Some(target));
        assert_eq!(app.resume_exhausted_revision, Some(3));
    }

    #[tokio::test]
    async fn resume_success_clears_matching_lockout_and_cache() {
        let target = Local::now();
        let state = shared_state_with_lockout(target, 3);
        state
            .lock()
            .unwrap()
            .file_size_cache
            .insert(std::path::PathBuf::from("session.jsonl"), 42);
        let resume = ScriptedResume::outcomes(vec![ResumeOutcome::Sent]);

        handle_expiry_with(&state, target, resume.clone()).await;

        let app = state.lock().unwrap();
        assert_eq!(resume.attempts(), 1);
        assert_eq!(app.lockout_target_time, None);
        assert!(app.file_size_cache.is_empty());
        assert_eq!(app.resume_exhausted_revision, None);
    }

    #[tokio::test]
    async fn already_due_new_runtime_event_reaches_expiry_and_attempts_resume() {
        let now = Local::now();
        let due = now - Duration::seconds(1);
        let state = Arc::new(Mutex::new(AppState::new()));
        record_lockout(&state, limit(now, due), "test");
        let resume = ScriptedResume::outcomes(vec![ResumeOutcome::Sent]);

        assert!(state.lock().unwrap().lockout_target_time.unwrap() <= Local::now());
        handle_expiry_with(&state, due, resume.clone()).await;

        assert_eq!(resume.attempts(), 1);
        assert_eq!(state.lock().unwrap().lockout_target_time, None);
    }

    #[tokio::test]
    async fn resume_ambiguity_stops_and_marks_revision_exhausted() {
        let target = Local::now();
        let state = shared_state_with_lockout(target, 3);
        let resume = ScriptedResume::outcomes(vec![ResumeOutcome::AmbiguousFailure(
            "flush uncertain".to_string(),
        )]);

        handle_expiry_with(&state, target, resume.clone()).await;

        let app = state.lock().unwrap();
        assert_eq!(resume.attempts(), 1);
        assert_eq!(app.lockout_target_time, Some(target));
        assert_eq!(app.resume_exhausted_revision, Some(3));
    }

    struct ReplacingResume {
        state: Arc<Mutex<AppState>>,
        replacement: chrono::DateTime<Local>,
    }

    impl super::ResumeAttempt for ReplacingResume {
        fn resume(&self) -> ResumeOutcome {
            let mut app = self.state.lock().unwrap();
            app.lockout_revision += 1;
            app.lockout_target_time = Some(self.replacement);
            ResumeOutcome::Sent
        }
    }

    #[tokio::test]
    async fn resume_success_does_not_clear_newer_revision() {
        let target = Local::now();
        let replacement = target + Duration::minutes(30);
        let state = shared_state_with_lockout(target, 3);

        handle_expiry_with(
            &state,
            target,
            ReplacingResume {
                state: Arc::clone(&state),
                replacement,
            },
        )
        .await;

        let app = state.lock().unwrap();
        assert_eq!(app.lockout_revision, 4);
        assert_eq!(app.lockout_target_time, Some(replacement));
    }

    #[tokio::test]
    async fn direct_scan_records_one_file_watcher_lockout_from_new_content() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "botsitter-direct-scan-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("session.jsonl");
        std::fs::write(&path, "{\"type\":\"baseline\"}\n").unwrap();
        let baseline_len = std::fs::metadata(&path).unwrap().len();

        let state = Arc::new(Mutex::new(AppState::new()));
        state
            .lock()
            .unwrap()
            .file_size_cache
            .insert(path.clone(), baseline_len);

        let now = Local::now();
        let target = now + chrono::Duration::hours(2);
        let reset = target.format("%-I:%M%P").to_string();
        let row = format!(
            "{{\"timestamp\":\"{}\",\"error\":\"rate_limit\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"You've hit your session limit · resets {reset} (America/Toronto)\"}}]}}}}\n",
            now.to_rfc3339()
        );
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(row.as_bytes())
            .unwrap();

        let mut next_log_time = Instant::now();
        scan_and_update_state(
            HashSet::from([path.clone()]),
            &state,
            &mut next_log_time,
            &mut std::collections::HashMap::new(),
            &PARSER,
        )
        .await;

        let app = state.lock().unwrap();
        assert_eq!(app.lockout_revision, 1);
        assert_eq!(
            app.lockout_target_time
                .expect("watcher target")
                .format("%-I:%M%P")
                .to_string(),
            reset
        );
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stream_lockout_updates_shared_state() {
        let state = Arc::new(Mutex::new(AppState::new()));
        let target = Local::now() + Duration::minutes(30);

        record_lockout(
            &state,
            RateLimitInfo {
                event_time: target,
                target_time: target,
                display_str: "5:30pm".to_string(),
                raw_message: "fixture".to_string(),
            },
            "stream-json",
        );

        assert_eq!(state.lock().unwrap().lockout_target_time, Some(target));
    }

    #[test]
    fn newer_event_replaces_target_even_when_target_is_earlier() {
        let now = Local::now();
        let state = shared_state_with_lockout(now + Duration::hours(4), 9);
        state.lock().unwrap().latest_rate_limit_event_time = Some(now);
        record_lockout(
            &state,
            limit(now + Duration::minutes(1), now + Duration::hours(1)),
            "test",
        );
        let app = state.lock().unwrap();
        assert_eq!(app.lockout_target_time, Some(now + Duration::hours(1)));
        assert_eq!(
            app.latest_rate_limit_event_time,
            Some(now + Duration::minutes(1))
        );
        assert_eq!(app.lockout_revision, 10);
    }

    #[test]
    fn older_and_equal_events_preserve_target_revision_and_exhaustion() {
        let now = Local::now();
        let original_target = now + Duration::hours(2);
        let state = shared_state_with_lockout(original_target, 7);
        {
            let mut app = state.lock().unwrap();
            app.latest_rate_limit_event_time = Some(now);
            app.resume_exhausted_revision = Some(7);
        }
        record_lockout(
            &state,
            limit(now - Duration::seconds(1), now + Duration::hours(3)),
            "older",
        );
        record_lockout(&state, limit(now, now + Duration::hours(4)), "equal");
        let app = state.lock().unwrap();
        assert_eq!(app.lockout_target_time, Some(original_target));
        assert_eq!(app.lockout_revision, 7);
        assert_eq!(app.resume_exhausted_revision, Some(7));
    }

    #[test]
    fn newer_clear_update_clears_lockout() {
        let state = Arc::new(Mutex::new(AppState::new()));
        let event_time = Local::now();
        let target_time = event_time + chrono::Duration::hours(1);

        apply_limit_update(
            &state,
            LimitUpdate {
                event_time,
                state: LimitState::Locked {
                    target_time,
                    display: "fixture".into(),
                },
            },
            true,
        );
        apply_limit_update(
            &state,
            LimitUpdate {
                event_time: event_time + chrono::Duration::seconds(1),
                state: LimitState::Clear,
            },
            true,
        );

        let app = state.lock().unwrap();
        assert_eq!(app.lockout_target_time, None);
        assert_eq!(
            app.latest_rate_limit_event_time,
            Some(event_time + chrono::Duration::seconds(1))
        );
    }

    #[test]
    fn stale_clear_update_cannot_replace_newer_lockout() {
        let state = Arc::new(Mutex::new(AppState::new()));
        let event_time = Local::now();
        let target_time = event_time + chrono::Duration::hours(1);

        apply_limit_update(
            &state,
            LimitUpdate {
                event_time,
                state: LimitState::Locked {
                    target_time,
                    display: "fixture".into(),
                },
            },
            true,
        );
        apply_limit_update(
            &state,
            LimitUpdate {
                event_time: event_time - chrono::Duration::seconds(1),
                state: LimitState::Clear,
            },
            true,
        );

        assert_eq!(state.lock().unwrap().lockout_target_time, Some(target_time));
    }

    #[test]
    fn overlap_scan_aligns_to_utf8_character_boundaries() {
        let path = unique_path("utf8-overlap.jsonl");
        let prefix = "a".repeat(4095);
        let content = format!("{prefix}é{{\"type\":\"event\"}}\n");
        std::fs::write(&path, content.as_bytes()).unwrap();
        let file = std::fs::File::open(&path).unwrap();

        let result = scan_mmap_range_for_test(&file, 4097, content.len() as u64, &PARSER);

        assert!(result.is_ok());
        std::fs::remove_file(path).unwrap();
    }
}
