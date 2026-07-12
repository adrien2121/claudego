use crate::logging::log_to_file;
use crate::models::SharedAppState;
use crate::resume::ResumeTarget;
use crate::time_format::format_duration;
use chrono::Local;
use std::collections::HashMap;
use std::time::Instant;
use tokio::time::{sleep, Duration};

/// Handles debouncing and collecting file system events.
mod events;
/// Contains helper functions for formatting log output.
mod formatters;
/// Contains helper functions for logging intervals and other utilities.
mod helpers;
/// Manages the lifecycle of the file system watcher (creation, recovery).
mod lifecycle;
/// Core logic for handling runtime file events and lockout expiry.
mod runtime;
/// Logic for the initial scan on application startup.
mod startup;

use lifecycle::WatcherHandle;

#[derive(Debug, PartialEq, Eq)]
enum MonitorControl {
    Continue,
    Stop,
}

/// Spawns a dedicated thread to monitor Claude log files for rate limit lockouts.
pub fn spawn_lockout_monitor(state: SharedAppState, resume_target: ResumeTarget) {
    tokio::spawn(async move {
        // ── 1. Initial full scan (I/O outside lock) ───────────────────
        startup::initial_scan(&state);

        // ── 2. Create OS file watcher ───────────────────────────────────
        let Some(mut handle) = lifecycle::create_watcher().await else {
            return;
        };
        log_to_file("[System] Event-driven file watcher active. Blocking until events arrive.");
        let mut next_log_time = Instant::now();
        let mut scan_failure_logs = HashMap::new();

        // ── 3. Event loop ───────────────────────────────────────────────
        loop {
            let (lockout_target, retry_exhausted) = {
                let app = state.lock().unwrap_or_else(|e| e.into_inner());
                (
                    app.lockout_target_time,
                    app.resume_exhausted_revision == Some(app.lockout_revision),
                )
            };

            if let Some(target) = lockout_target {
                // --- Lockout is ACTIVE ---
                let now = Local::now();
                if now >= target {
                    if retry_exhausted {
                        let event_res = handle.rx.recv().await;
                        if handle_event_result(
                            event_res,
                            &mut handle,
                            &state,
                            &mut next_log_time,
                            &mut scan_failure_logs,
                        )
                        .await
                            == MonitorControl::Stop
                        {
                            return;
                        }
                        continue;
                    }
                    runtime::handle_expiry(&state, &resume_target, target).await;
                    continue; // Re-evaluate state immediately
                }

                let time_to_target = (target - now).to_std().unwrap_or(Duration::ZERO);

                // Log progress if it's time
                if Instant::now() >= next_log_time {
                    let remaining_secs = (target - Local::now()).num_seconds().max(0);
                    if remaining_secs > 0 {
                        let interval = helpers::cooldown_log_interval(remaining_secs);
                        log_to_file(&format!(
                            "[Lockout Cooldown] {} remaining. Next log in {}.",
                            format_duration(remaining_secs),
                            format_duration(interval.as_secs() as i64),
                        ));
                        next_log_time = Instant::now() + interval;
                    }
                }

                let time_to_next_log = next_log_time.saturating_duration_since(Instant::now());
                let wait_duration = time_to_target.min(time_to_next_log);

                tokio::select! {
                    _ = sleep(wait_duration) => {
                        // Timer expired, loop will check if lockout is over or log progress.
                    }
                    event_res = handle.rx.recv() => {
                        if handle_event_result(event_res, &mut handle, &state, &mut next_log_time, &mut scan_failure_logs).await
                            == MonitorControl::Stop
                        {
                            return;
                        }
                    }
                }
            } else {
                // --- No lockout, wait for a file event ---
                let event_res = handle.rx.recv().await;
                if handle_event_result(
                    event_res,
                    &mut handle,
                    &state,
                    &mut next_log_time,
                    &mut scan_failure_logs,
                )
                .await
                    == MonitorControl::Stop
                {
                    return;
                }
            }
        }
    });
}

async fn handle_event_result(
    event_res: Option<notify::Result<notify::Event>>,
    handle: &mut WatcherHandle,
    state: &SharedAppState,
    next_log_time: &mut Instant,
    scan_failure_logs: &mut HashMap<std::path::PathBuf, Instant>,
) -> MonitorControl {
    match event_res {
        Some(Ok(first_event)) => {
            let paths = events::debounce_events(first_event, &mut handle.rx).await;
            if !paths.is_empty() {
                runtime::scan_and_update_state(paths, state, next_log_time, scan_failure_logs)
                    .await;
            }
        }
        Some(Err(_)) => { /* A notify error occurred, but the channel is fine. Continue. */ }
        None => {
            // The watcher channel disconnected. Attempt to recover.
            log_to_file("[Watcher] Disconnected. Attempting recovery…");
            return recovery_control(async {
                let new_handle = lifecycle::create_watcher().await?;
                *handle = new_handle;
                Some(())
            })
            .await;
        }
    }

    MonitorControl::Continue
}

async fn recovery_control<T>(
    recovery: impl std::future::Future<Output = Option<T>>,
) -> MonitorControl {
    if recovery.await.is_some() {
        MonitorControl::Continue
    } else {
        log_to_file("[Watcher Error] CRITICAL: Watcher recovery failed. Monitoring has stopped.");
        MonitorControl::Stop
    }
}

pub fn record_lockout(
    state: &SharedAppState,
    limit_info: crate::watcher::scan::ActiveRateLimitInfo,
    source: &str,
) {
    runtime::record_lockout(state, limit_info, source);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn exhausted_recovery_stops_monitor() {
        let control = recovery_control(async { None::<WatcherHandle> }).await;
        assert_eq!(control, MonitorControl::Stop);
    }

    #[tokio::test]
    async fn successful_recovery_continues_monitor() {
        let control = recovery_control(async { Some(()) }).await;
        assert_eq!(control, MonitorControl::Continue);
    }
}
