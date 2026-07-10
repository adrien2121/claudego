use crate::logging::{log_to_file, log_with_content};
use crate::models::{output_is_hot, SharedAppState};
use crate::monitor::helpers::{DEFER_SCAN_INTERVAL, PTY_BUSY_THRESHOLD};
use crate::resume::ResumeTarget;
use chrono::{DateTime, Local};
use memmap2;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;
use tokio::time::sleep;

/// To prevent unbounded memory usage when a log file has a very large amount of new
/// content, we cap the content collected for logging to 1 MiB.
const MAX_PREVIEW_CONTENT_SIZE: usize = 1_048_576;

/// When scanning a modified file, re-scan this many bytes from the end of the
/// previously seen content to be robust against missed initial detections.
const SCAN_OVERLAP_BYTES: u64 = 4096;

/// Scans a set of changed paths and updates the application state.
pub(super) async fn scan_and_update_state(
    paths: HashSet<PathBuf>,
    state: &SharedAppState,
    next_log_time: &mut Instant,
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
                "Claude is currently streaming output. Deferring file scan for {:?}.",
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
        let old_size = state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .file_size_cache
            .get(&path)
            .copied()
            .unwrap_or(0); // If not in cache, this is the first time we see it.

        let mtime_before = std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok());
        let new_size = std::fs::metadata(&path)
            .map(|m| m.len())
            .unwrap_or(old_size);

        if new_size <= old_size {
            continue;
        }

        let mtime_after = std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok());

        // If the file was modified during our read, we abort processing for this cycle.
        // The file watcher has already picked up the new change, which will trigger a
        // new scan after the debounce period. This ensures we only act on stable data.
        if mtime_before.is_some() && mtime_before != mtime_after {
            log_to_file(&format!(
                "[Scan Aborted] Concurrent modification of {}. Rescheduling scan.",
                path.display()
            ));
            continue;
        }

        // --- Single-Pass Scan & Preview using Memory-Mapping ---
        // We use a memory map for a highly efficient, single-pass operation.
        // This avoids multiple file reads and complex buffer management.
        let limit_opt = match std::fs::File::open(&path) {
            Ok(file) => {
                // Safety: File is read-only. OS handles paging.
                match unsafe { memmap2::Mmap::map(&file) } {
                    Ok(mmap) => {
                        // To be robust, we don't just scan the new content. We re-scan a small
                        // portion of the old content as well. This helps if the initial scan
                        // missed a limit that was at the very end of the file at startup.
                        let scan_start = old_size.saturating_sub(SCAN_OVERLAP_BYTES);
                        let scan_slice = &mmap[scan_start as usize..new_size as usize];

                        match std::str::from_utf8(scan_slice) {
                            Ok(scan_str) => {
                                // 1. Generate a preview for logging. The preview should only
                                //    show the *truly new* content, not the overlap.
                                let new_content_offset =
                                    old_size.saturating_sub(scan_start) as usize;
                                if new_content_offset < scan_str.len() {
                                    let new_content_for_preview = &scan_str[new_content_offset..];
                                    let preview_len =
                                        new_content_for_preview.len().min(MAX_PREVIEW_CONTENT_SIZE);
                                    let content_preview =
                                        crate::monitor::formatters::create_content_preview(
                                            &new_content_for_preview[..preview_len],
                                        );
                                    log_with_content(
                                        &format!("[File Content] New data in {}:", path.display()),
                                        content_preview,
                                    );
                                }

                                // 2. Scan the entire slice (including overlap) for a rate limit.
                                crate::watcher::scan::scan_content_for_limit(scan_str)
                            }
                            Err(_) => {
                                log_to_file(&format!(
                                    "[Scan Warning] Invalid UTF-8 in new content of {}. Skipping.",
                                    path.display()
                                ));
                                None
                            }
                        }
                    }
                    Err(e) => {
                        log_to_file(&format!(
                            "[Scan Error] Failed to mmap {}: {}",
                            path.display(),
                            e
                        ));
                        None
                    }
                }
            }
            Err(e) => {
                log_to_file(&format!(
                    "[Scan Error] Failed to open {}: {}",
                    path.display(),
                    e
                ));
                None
            }
        };

        // --- Update State ---
        {
            let mut app = state.lock().unwrap_or_else(|e| e.into_inner());
            app.file_size_cache.insert(path, new_size);
        }

        if let Some(limit_info) = limit_opt {
            // These logs were previously inside `watcher::scan::parse_rate_limit_line`.
            // Moving them here makes the parser a pure function.
            log_to_file("  [MATCH] Active 'rate_limit' row found! Parsing contents...");
            log_to_file(&format!(
                "  [Extracted Text] Raw Limit Message: \"{}\"",
                limit_info.raw_message
            ));
            log_to_file(&format!(
                "  [SUCCESS] Valid active limit confirmed! Resets: {}",
                limit_info.display_str
            ));
            record_lockout(state, limit_info, "file watcher");
            *next_log_time = Instant::now();
        }
    }
}

pub(super) fn record_lockout(
    state: &SharedAppState,
    limit_info: crate::watcher::scan::ActiveRateLimitInfo,
    source: &str,
) {
    log_to_file(&format!(
        "[LOCKOUT DETECTED] Rate limit hit from {source}. Target: {}",
        limit_info.display_str
    ));
    let mut app = state.lock().unwrap_or_else(|e| e.into_inner());
    app.lockout_revision = app.lockout_revision.wrapping_add(1);
    app.lockout_target_time = Some(limit_info.target_time);
}

/// Handles the logic for when a lockout expires.
pub(super) fn handle_expiry(
    state: &SharedAppState,
    resume_target: &ResumeTarget,
    expired_target: DateTime<Local>,
) {
    log_to_file("[Trigger] Reset time reached. Injecting 'continue' command…");
    match resume_target.resume() {
        Ok(()) => log_to_file("[System] Resume command sent."),
        Err(error) => log_to_file(&format!("[Resume Error] {error}")),
    }
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    // Atomically check and set: only clear the lockout if it's the one we just handled.
    // This prevents a race condition where a new lockout is detected right as an old one expires.
    if s.lockout_target_time == Some(expired_target) {
        s.lockout_target_time = None;
        s.file_size_cache.clear();
        log_to_file("[System] Resuming passive file monitoring.");
    } else {
        // Another lockout was set in the meantime. Don't clear it.
        log_to_file("[System] Expiry handled, but a newer lockout has already been detected. State not cleared.");
    }
}

#[cfg(test)]
mod tests {
    use super::record_lockout;
    use crate::models::AppState;
    use crate::watcher::scan::ActiveRateLimitInfo;
    use chrono::{Duration, Local};
    use std::sync::{Arc, Mutex};

    #[test]
    fn stream_lockout_updates_shared_state() {
        let state = Arc::new(Mutex::new(AppState::new()));
        let target = Local::now() + Duration::minutes(30);

        record_lockout(
            &state,
            ActiveRateLimitInfo {
                target_time: target,
                display_str: "5:30pm".to_string(),
                raw_message: "Claude limit reached; resets 5:30pm".to_string(),
            },
            "stream-json",
        );

        assert_eq!(state.lock().unwrap().lockout_target_time, Some(target));
    }
}
