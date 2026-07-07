use crate::logging::{log_to_file, log_with_content};
use crate::models::SharedAppState;
use crate::monitor::helpers::{DEFER_SCAN_INTERVAL, PTY_BUSY_THRESHOLD};
use crate::pty_bridge::SharedPtyWriter;
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::Instant;

/// Scans a set of changed paths and updates the application state.
pub(super) fn scan_and_update_state(
    paths: HashSet<PathBuf>,
    state: &SharedAppState,
    next_log_time: &mut Instant,
) {
    // --- Busy-Wait Logic ---
    // Before scanning, check if the PTY is actively streaming. If so, wait.
    // This prevents the file scan's I/O from competing with the PTY reader thread,
    // which is a direct cause of the "stalled stream" error.
    loop {
        let pty_is_busy = state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last_pty_activity
            .elapsed() < PTY_BUSY_THRESHOLD;

        if pty_is_busy {
            log_to_file(&format!(
                "[Scan Postponed] Command is actively streaming output. Deferring file scan for {:?}.",
                DEFER_SCAN_INTERVAL
            ));
            std::thread::sleep(DEFER_SCAN_INTERVAL);
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

        let mut new_content = String::new();
        let bytes_read = if let Ok(mut file) = File::open(&path) {
            if file.seek(SeekFrom::Start(old_size)).is_ok() {
                file.read_to_string(&mut new_content).unwrap_or(0)
            } else {
                0
            }
        } else {
            0
        };

        if bytes_read == 0 {
            continue;
        }

        let mtime_after = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok());

        // If the file was modified during our read, we abort processing for this cycle.
        // The file watcher has already picked up the new change, which will trigger a
        // new scan after the debounce period. This ensures we only act on stable data.
        if mtime_before.is_some() && mtime_before != mtime_after {
            log_to_file(&format!(
                "[Scan Aborted] Concurrent modification of {}. Rescheduling.",
                path.display()
            ));
            continue;
        }

        let new_size = old_size + bytes_read as u64;
        // Generate a preview of the new content for the logs.
        let content_preview = crate::monitor::formatters::create_content_preview(&new_content);
        log_with_content(&format!("[File Content] New data in {}:", path.display()), content_preview);

        let limit_opt = crate::watcher::scan::scan_content_for_limit(&new_content);

        let mut app = state.lock().unwrap_or_else(|e| e.into_inner());

        app.file_size_cache.insert(path.clone(), new_size);
        if let Some((target_time, time_str)) = limit_opt {
            log_to_file(&format!("[LOCKOUT DETECTED] Rate limit hit! Target: {}", time_str));
            app.is_sleeping = true;
            app.lockout.target_time = Some(target_time);
            *next_log_time = Instant::now();
        }
    }
}

/// Handles the logic for when a lockout expires.
pub(super) fn handle_expiry(state: &SharedAppState, writer: &SharedPtyWriter) {
    log_to_file("[Trigger] Reset time reached. Injecting 'continue' command…");
    {
        let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
        let _ = w.write_all(b"continue\r");
        let _ = w.flush();
    }
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    s.is_sleeping = false;
    s.lockout.target_time = None;
    s.file_size_cache.clear();
    log_to_file("[System] Resuming passive file monitoring.");
}