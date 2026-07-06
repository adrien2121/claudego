use crate::logging::log_to_file;
use crate::models::SharedAppState;
use crate::pty_bridge::SharedPtyWriter;
use crate::watcher::files as watcher_files;
use chrono::{DateTime, Local};
use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

/// Performs the initial scan of all session files on startup.
pub(super) fn initial_scan(state: &SharedAppState) {
    log_to_file("[Startup] Performing initial rate limit check…");
    let initial_files = watcher_files::claude_projects_root()
        .map(|root| watcher_files::recent_session_logs(&root, SystemTime::UNIX_EPOCH))
        .unwrap_or_default();
    let mut latest_limit: Option<(DateTime<Local>, String)> = None;

    for (path, _) in initial_files {
        // Scan file (I/O, no lock). old_size is 0 to ensure a full check.
        let (limit_opt, new_size) = crate::watcher::scan::scan_session_log(&path, 0);

        // Update state with file size (brief lock)
        state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .file_size_cache
            .insert(path.clone(), new_size);

        if let Some((target, _)) = &limit_opt {
            if latest_limit.as_ref().map_or(true, |(t, _)| target > t) {
                latest_limit = limit_opt;
            }
        }
    }

    if let Some((target_time, time_str)) = latest_limit {
        log_to_file(&format!("[LOCKOUT ON STARTUP] Rate limit found. Target: {}", time_str));
        let mut app = state.lock().unwrap_or_else(|e| e.into_inner());
        app.is_sleeping = true;
        app.lockout.target_time = Some(target_time);
    }
}

/// Scans a set of changed paths and updates the application state.
pub(super) fn scan_and_update_state(
    paths: HashSet<PathBuf>,
    state: &SharedAppState,
    next_log_time: &mut Instant,
) {
    for path in paths {
        let old_size = state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .file_size_cache
            .get(&path)
            .copied()
            .unwrap_or(0);

        let (limit_opt, new_size) = crate::watcher::scan::scan_session_log(&path, old_size);
        let file_grew = new_size > old_size;

        let mut app = state.lock().unwrap_or_else(|e| e.into_inner());
        app.file_size_cache.insert(path.clone(), new_size);

        if let Some((target_time, time_str)) = limit_opt {
            log_to_file(&format!("[LOCKOUT DETECTED] Rate limit hit! Target: {}", time_str));
            app.is_sleeping = true;
            app.lockout.target_time = Some(target_time);
            *next_log_time = Instant::now();
        } else if app.lockout.target_time.is_some() && file_grew {
            log_to_file("[Lockout Aborted] Normal activity detected. Rate limit bypassed!");
            app.is_sleeping = false;
            app.lockout.target_time = None;
        }
    }
}

/// Checks if the lockout has expired and handles resuming input.
/// Returns `true` if the main loop should `continue`.
pub(super) fn check_and_handle_expiry(
    state: &SharedAppState,
    writer: &SharedPtyWriter,
) -> bool {
    let current_target = state.lock().unwrap_or_else(|e| e.into_inner()).lockout.target_time;

    match current_target {
        Some(t) if Local::now() >= t => {
            log_to_file("[Trigger] Reset time reached. Injecting 'continue\\n'…");
            {
                let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
                let _ = w.write_all(b"continue\n");
                let _ = w.flush();
            }
            {
                let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
                s.is_sleeping = false;
                s.lockout.target_time = None;
                s.file_size_cache.clear();
            }
            log_to_file("[System] Resuming passive file monitoring.");
            true
        }
        Some(_) => {
            log_to_file("[Lockout] Target updated during wait. Re-evaluating…");
            true
        }
        None => false,
    }
}