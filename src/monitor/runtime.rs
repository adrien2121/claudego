use crate::logging::{log_to_file, log_with_content};
use crate::models::SharedAppState;
use crate::monitor::helpers::{DEFER_SCAN_INTERVAL, PTY_BUSY_THRESHOLD};
use crate::pty_bridge::SharedPtyWriter;
use chrono::{DateTime, Local};
use memmap2;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::time::{sleep, Duration};

/// To prevent unbounded memory usage when a log file has a very large amount of new
/// content, we cap the content collected for logging to 1 MiB.
const MAX_PREVIEW_CONTENT_SIZE: usize = 1_048_576;

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
        app.last_pty_activity.clone()
    };
    loop {
        let last_activity_ns = activity_tracker.load(Ordering::Relaxed);
        let pty_is_busy = if last_activity_ns == 0 {
            // If the timestamp is 0, it's uninitialized. Assume not busy.
            false
        } else {
            // Check if the duration since the last activity is within our threshold.
            let now_ns = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64;
            let elapsed_ns = now_ns.saturating_sub(last_activity_ns);
            Duration::from_nanos(elapsed_ns) < PTY_BUSY_THRESHOLD
        };

        if pty_is_busy {
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
    let path_names: Vec<_> = paths
        .iter()
        .map(|p| p.display().to_string())
        .collect();
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
            .unwrap_or(0);

        let mtime_before = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());
        let new_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(old_size);

        if new_size <= old_size {
            continue;
        }

        let mtime_after = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());

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
                        // Slice the mmap to only the new content.
                        let new_content_slice = &mmap[old_size as usize..new_size as usize];
                        let new_content_str = String::from_utf8_lossy(new_content_slice);

                        // 1. Generate a preview for logging from the start of the new content.
                        let preview_len = new_content_str.len().min(MAX_PREVIEW_CONTENT_SIZE);
                        let content_preview =
                            crate::monitor::formatters::create_content_preview(&new_content_str[..preview_len]);
                        log_with_content(&format!("[File Content] New data in {}:", path.display()), content_preview);

                        // 2. Scan the new content for a rate limit.
                        crate::watcher::scan::scan_content_for_limit(&new_content_str)
                    }
                    Err(e) => {
                        log_to_file(&format!("[Scan Error] Failed to mmap {}: {}", path.display(), e));
                        None
                    }
                }
            }
            Err(e) => {
                log_to_file(&format!("[Scan Error] Failed to open {}: {}", path.display(), e));
                None
            }
        };

        // --- Update State ---
        let mut app = state.lock().unwrap_or_else(|e| e.into_inner());
        app.file_size_cache.insert(path, new_size);

        if let Some((target_time, time_str)) = limit_opt {
            log_to_file(&format!("[LOCKOUT DETECTED] Rate limit hit! Target: {}", time_str));
            app.is_sleeping = true;
            app.lockout.target_time = Some(target_time);
            *next_log_time = Instant::now();
        }
    }
}

/// Handles the logic for when a lockout expires.
pub(super) fn handle_expiry(state: &SharedAppState, writer: &SharedPtyWriter, expired_target: DateTime<Local>) {
    log_to_file("[Trigger] Reset time reached. Injecting 'continue' command…");
    {
        let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
        let _ = w.write_all(b"continue\r");
        let _ = w.flush();
    }
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    // Atomically check and set: only clear the lockout if it's the one we just handled.
    // This prevents a race condition where a new lockout is detected right as an old one expires.
    if s.lockout.target_time == Some(expired_target) {
        s.is_sleeping = false;
        s.lockout.target_time = None;
        s.file_size_cache.clear();
        log_to_file("[System] Resuming passive file monitoring.");
    } else {
        // Another lockout was set in the meantime. Don't clear it.
        log_to_file("[System] Expiry handled, but a newer lockout has already been detected. State not cleared.");
    }
}