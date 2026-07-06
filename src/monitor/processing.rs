use crate::logging::log_to_file;
use crate::models::SharedAppState;
use crate::pty_bridge::SharedPtyWriter;
use crate::watcher::files as watcher_files;
use chrono::{DateTime, Local};
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

/// Performs the initial scan of all session files on startup.
pub(super) fn initial_scan(state: &SharedAppState) {
    log_to_file("[Startup] Performing initial rate limit check…");
    let initial_files = watcher_files::claude_projects_root()
        .map(|root| watcher_files::recent_session_logs(&root, SystemTime::UNIX_EPOCH))
        .unwrap_or_default();
    let mut latest_limit: Option<(DateTime<Local>, String)> = None;

    if !initial_files.is_empty() {
        log_to_file(&format!("[Startup] Scanning {} initial session file(s).", initial_files.len()));
    }

    for (path, _) in initial_files {
        // Perform the scan directly and then update the state in a single lock.
        let (limit_opt, new_size) = crate::watcher::scan::scan_session_log(&path, 0);
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

        // Add a blank line for readability between file scans during startup.
        log_to_file("");
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
        // --- Lock 1: Read old size ---
        let old_size = {
            state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .file_size_cache
                .get(&path)
                .copied()
                .unwrap_or(0)
        };

        // --- Perform all I/O and scanning without holding a lock ---
        // This is inefficient as the file is read twice: once here for logging,
        // and once inside `scan_session_log`. A future refactor could combine these.
        let mut new_content = String::new();
        if let Ok(mut file) = File::open(&path) {
            if file.seek(SeekFrom::Start(old_size)).is_ok() {
                let _ = file.read_to_string(&mut new_content);
            }
        }
        let (limit_opt, new_size) = crate::watcher::scan::scan_session_log(&path, old_size);

        // --- Log content if any was found ---
        if !new_content.trim().is_empty() {
            let preview = crate::monitor::formatters::format_file_content_preview(&new_content);
            log_to_file(&format!("[File Content] New data in {}:\n{}", path.display(), preview));
        }

        // --- Lock 2: Update state with all results from the scan ---
        let mut app = state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        app.file_size_cache.insert(path, new_size);
        let file_grew = new_size > old_size;
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
            log_to_file("[Trigger] Reset time reached. Injecting 'continue' command…");
            {
                let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
                let _ = w.write_all(b"continue\r");
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
            // Lockout is active, but the target time has not been reached.
            // Return `false` to allow the main loop to perform its periodic
            // sleep and cooldown logging. Returning `true` would cause a busy-wait.
            false
        }
        None => false,
    }
}