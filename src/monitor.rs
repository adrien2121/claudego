use crate::logging::log_to_file;
use crate::models::SharedAppState;
use crate::pty_bridge::SharedPtyWriter;
use crate::time_format::format_duration;
use crate::watcher::check_native_session_limit;
use chrono::Local;
use std::io::Write;
use std::thread;
use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::channel;
use notify::{Watcher, RecursiveMode};

const NORMAL_POLL_INTERVAL_SECS: u64 = 5;
const VERY_FAR_COOLDOWN_THRESHOLD_SECS: u64 = 3 * 60 * 60;
const FAR_COOLDOWN_THRESHOLD_SECS: u64 = 60 * 60;
const MEDIUM_COOLDOWN_THRESHOLD_SECS: u64 = 15 * 60;
const NEAR_COOLDOWN_THRESHOLD_SECS: u64 = 5 * 60;
const FINAL_COOLDOWN_THRESHOLD_SECS: u64 = 60;

pub fn spawn_lockout_monitor(state: SharedAppState, writer: SharedPtyWriter) {
    thread::spawn(move || {
        let detected_limit = {
            let mut app_state = state.lock().unwrap();

            if app_state.show_logs && !app_state.is_sleeping {
                log_to_file("[Startup] Performing initial rate limit check...");
            }

            if app_state.is_sleeping {
                None
            } else {
                let limit = check_native_session_limit(&mut app_state);
                if limit.is_some() {
                    app_state.is_sleeping = true;
                }
                limit
            }
        };

        let mut active_target: Option<chrono::DateTime<Local>> = None;
        let mut next_log_time = Instant::now();

        if let Some((target_time, time_str)) = detected_limit {
            log_to_file(&format!(
                "[LOCKOUT DETECTED ON STARTUP] Rate limit hit! Target reset time found: {}",
                time_str
            ));
            active_target = Some(target_time);
        }

        let (tx, rx) = channel();
        let mut watcher = match notify::recommended_watcher(tx) {
            Ok(w) => w,
            Err(e) => {
                log_to_file(&format!("[Watcher Error] Failed to initialize watcher: {}", e));
                return;
            }
        };

        let projects_root = match crate::watcher::files::claude_projects_root() {
            Some(path) => path,
            None => {
                log_to_file("[Watcher Error] Could not determine Claude projects root.");
                return;
            }
        };

        let _ = std::fs::create_dir_all(&projects_root);

        if let Err(e) = watcher.watch(&projects_root, RecursiveMode::Recursive) {
            log_to_file(&format!("[Watcher Error] Failed to watch directory: {}", e));
            return;
        }

        let mut active_files: HashMap<PathBuf, Instant> = HashMap::new();

        loop {
            if let Some(target) = active_target {
                let now = Local::now();
                if now >= target {
                    log_to_file("[Automation Trigger] Reset time reached! Injecting 'continue\\n' command into Claude...");
                    let mut pty_writer = writer.lock().unwrap();
                    let _ = pty_writer.write_all(b"continue\n");
                    let _ = pty_writer.flush();
                    
                    let mut app_state = state.lock().unwrap();
                    app_state.is_sleeping = false;
                    app_state.file_size_cache.clear();
                    active_target = None;
                    log_to_file("[System] Resuming normal passive file monitoring.");
                } else if Instant::now() >= next_log_time {
                    let remaining = (target - now).num_seconds().max(1);
                    let sleep_dur = cooldown_sleep_duration(remaining);
                    let readable_time = format_duration(remaining);
                    let readable_sleep = format_duration(sleep_dur.as_secs() as i64);
                    log_to_file(&format!(
                        "[Lockout Cooldown] Waiting... {} remaining until automated reset. Next log in {}.",
                        readable_time, readable_sleep
                    ));
                    next_log_time = Instant::now() + sleep_dur;
                }
            }

            if let Ok(Ok(event)) = rx.recv_timeout(Duration::from_millis(500)) {
                if let notify::EventKind::Modify(_) | notify::EventKind::Create(_) = event.kind {
                    for path in event.paths {
                        if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                            active_files.insert(path, Instant::now());
                        }
                    }
                }
            }

            let now = Instant::now();
            let mut files_to_scan = Vec::new();

            for (path, last_modified) in &active_files {
                if now.duration_since(*last_modified) >= Duration::from_secs(2) {
                    files_to_scan.push(path.clone());
                }
            }

            for path in files_to_scan {
                active_files.remove(&path);

                let scan_result = {
                    let mut app_state = state.lock().unwrap();
                    let old_size = app_state.file_size_cache.get(&path).copied().unwrap_or(0);
                    let result = crate::watcher::scan::scan_session_log(&path, &mut app_state, true);
                    let new_size = app_state.file_size_cache.get(&path).copied().unwrap_or(0);
                    (result, new_size > old_size)
                };

                let (limit_opt, file_grew) = scan_result;

                if let Some((target_time, time_str)) = limit_opt {
                    log_to_file(&format!(
                        "[LOCKOUT DETECTED] Rate limit hit! Target reset time found: {}",
                        time_str
                    ));
                    state.lock().unwrap().is_sleeping = true;
                    active_target = Some(target_time);
                    next_log_time = Instant::now();
                } else if active_target.is_some() && file_grew {
                    log_to_file("[Lockout Aborted] Detected normal file activity. Rate limit was bypassed early!");
                    state.lock().unwrap().is_sleeping = false;
                    active_target = None;
                }
            }
        }
    });
}

fn cooldown_sleep_duration(remaining_seconds: i64) -> Duration {
    if remaining_seconds <= 0 {
        return Duration::from_secs(0);
    }

    let remaining_seconds = remaining_seconds as u64;
    let cadence_seconds = if remaining_seconds > VERY_FAR_COOLDOWN_THRESHOLD_SECS {
        60 * 60 // 1 hour
    } else if remaining_seconds > FAR_COOLDOWN_THRESHOLD_SECS {
        30 * 60 // 30 minutes
    } else if remaining_seconds > MEDIUM_COOLDOWN_THRESHOLD_SECS {
        5 * 60 // 5 minutes
    } else if remaining_seconds > NEAR_COOLDOWN_THRESHOLD_SECS {
        60 // 1 minute
    } else if remaining_seconds > FINAL_COOLDOWN_THRESHOLD_SECS {
        15 // 15 seconds
    } else {
        NORMAL_POLL_INTERVAL_SECS
    };

    Duration::from_secs(cadence_seconds.min(remaining_seconds))
}
