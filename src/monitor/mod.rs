use crate::logging::log_to_file;
use crate::models::SharedAppState;
use crate::pty_bridge::SharedPtyWriter;
use crate::time_format::format_duration;
use chrono::{DateTime, Local};
use std::sync::mpsc::RecvTimeoutError;
use std::thread;
use std::time::{Duration, Instant};

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

/// Represents the outcome of a wait operation in the main event loop.
enum WaitOutcome {
    /// A file event was received.
    Event(notify::Event),
    /// The loop should immediately restart.
    ShouldContinue,
    /// The watcher died and could not be recovered.
    WatcherDied,
}

/// Spawns a dedicated thread to monitor Claude log files for rate limit lockouts.
pub fn spawn_lockout_monitor(state: SharedAppState, writer: SharedPtyWriter) {
    thread::spawn(move || {
        // ── 1. Initial full scan (I/O outside lock) ───────────────────
        startup::initial_scan(&state);

        // ── 2. Create OS file watcher ───────────────────────────────────
        let Some(mut handle) = lifecycle::create_watcher() else {
            return;
        };
        log_to_file("[System] Event-driven file watcher active. Blocking until events arrive.");
        let mut next_log_time = Instant::now();

        // ── 3. Event loop ───────────────────────────────────────────────
        loop {
            // Snapshot the current lockout target (short lock).
            let lockout_target = { state.lock().unwrap_or_else(|e| e.into_inner()).lockout.target_time };
            
            // The waiting strategy depends on whether a lockout is currently active.
            let outcome = match lockout_target {
                Some(target) => {
                    // Lockout is active: wait with a timeout until the lockout expires.
                    handle_locked_wait(target, &mut handle, &mut next_log_time, &state, &writer)
                }
                None => handle_unlocked_wait(&mut handle), // No lockout: wait indefinitely for a file event.
            };

            match outcome {
                // Process file events that were received.
                WaitOutcome::ShouldContinue => continue,
                WaitOutcome::WatcherDied => return,
                WaitOutcome::Event(first) => {
                    let paths = events::debounce_events(first, &handle.rx);
                    if !paths.is_empty() {
                        runtime::scan_and_update_state(paths, &state, &mut next_log_time);
                    }
                }
            }
        }
    });
}

/// Handles the waiting logic when a lockout is active.
/// It waits for either a file event or for the lockout duration to expire.
fn handle_locked_wait(
    target: DateTime<Local>,
    handle: &mut WatcherHandle,
    next_log_time: &mut Instant,
    state: &SharedAppState,
    writer: &SharedPtyWriter,
) -> WaitOutcome {
    let now = Local::now();
    // Check if the lockout has expired.
    if now >= target {
        runtime::handle_expiry(state, writer, target);
        return WaitOutcome::ShouldContinue;
    }

    // The loop must wake up periodically to log cooldown progress. The wait
    // timeout is the *minimum* of the time to target vs. time to next log.
    let now_instant = Instant::now();
    let time_to_next_log = next_log_time.saturating_duration_since(now_instant);
    let to_target = (target - now).to_std().unwrap_or(Duration::ZERO);
    let wait_duration = to_target.min(time_to_next_log);
    let event_result = handle.rx.recv_timeout(wait_duration);

    if Instant::now() >= *next_log_time {
        // Log progress periodically during the cooldown.
        let remaining_secs = (target - Local::now()).num_seconds().max(0);
        if remaining_secs > 0 {
            let interval = helpers::cooldown_log_interval(remaining_secs);
            log_to_file(&format!(
                "[Lockout Cooldown] {} remaining. Next log in {}.",
                format_duration(remaining_secs),
                format_duration(interval.as_secs() as i64),
            ));
            *next_log_time = Instant::now() + interval;
        }
    }

    match event_result {
        Ok(Ok(ev)) => WaitOutcome::Event(ev),
        // A `notify` error occurred, but the channel is fine. Continue.
        Ok(Err(_)) => WaitOutcome::ShouldContinue,
        // Timeout means the lockout expired. Loop will re-evaluate.
        Err(RecvTimeoutError::Timeout) => WaitOutcome::ShouldContinue,
        // The watcher channel disconnected. Attempt to recover.
        Err(RecvTimeoutError::Disconnected) => recover_watcher(handle),
    }
}

/// Attempts to recover a disconnected file system watcher.
fn recover_watcher(handle: &mut WatcherHandle) -> WaitOutcome {
    log_to_file("[Watcher] Disconnected. Attempting recovery…");
    match lifecycle::create_watcher() {
        Some(new_handle) => {
            *handle = new_handle;
            WaitOutcome::ShouldContinue
        }
        None => WaitOutcome::WatcherDied,
    }
}

/// Handles the waiting logic when no lockout is active.
/// It blocks indefinitely until a file system event is received.
fn handle_unlocked_wait(handle: &mut WatcherHandle) -> WaitOutcome {
    match handle.rx.recv() {
        Ok(Ok(ev)) => WaitOutcome::Event(ev),
        // A `notify` error occurred, but the channel is fine. Continue.
        Ok(Err(_)) => WaitOutcome::ShouldContinue,
        Err(_) => recover_watcher(handle),
    }
}
