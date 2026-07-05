use crate::logging::log_to_file;
use crate::models::AppState;
use chrono::{DateTime, Local};
use std::time::{Duration, SystemTime};

pub mod files;
mod reset_time;
pub mod scan;

pub fn check_native_session_limit(state: &mut AppState) -> Option<(DateTime<Local>, String)> {
    let projects_root = files::claude_projects_root()?;
    let time_cap = SystemTime::now().checked_sub(Duration::from_secs(36000))?;

    if state.show_logs {
        log_to_file("[Watcher] Performing a global scan across all Claude workspace folders...");
    }

    let mut session_logs = files::recent_session_logs(&projects_root, time_cap);

    if session_logs.is_empty() {
        if state.show_logs {
            log_to_file(
                "[Watcher] Scan ended: No active session logs found anywhere on your machine.",
            );
        }
        return None;
    }

    session_logs.sort_by(|a, b| b.1.cmp(&a.1));

    if !files::any_file_changed(&session_logs, &state.file_size_cache) {
        if state.show_logs {
            log_to_file("[Watcher] Scan ended: No project files have grown since the last check.");
        }
        return None;
    }

    let mut active_limit = None;

    for (path, _) in session_logs {
        let should_scan = active_limit.is_none();
        let scan_result = scan::scan_session_log(&path, state, should_scan);

        if active_limit.is_none() {
            active_limit = scan_result;
        }
    }

    active_limit
}
