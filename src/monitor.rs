use crate::logging::log_to_file;
use crate::models::SharedAppState;
use crate::pty_bridge::SharedPtyWriter;
use crate::time_format::format_duration;
use crate::watcher::check_native_session_limit;
use chrono::Local;
use std::io::Write;
use std::thread;
use std::time::Duration;

const NORMAL_POLL_INTERVAL_SECS: u64 = 5;
const VERY_FAR_COOLDOWN_THRESHOLD_SECS: u64 = 3 * 60 * 60;
const FAR_COOLDOWN_THRESHOLD_SECS: u64 = 60 * 60;
const MEDIUM_COOLDOWN_THRESHOLD_SECS: u64 = 15 * 60;
const NEAR_COOLDOWN_THRESHOLD_SECS: u64 = 5 * 60;
const FINAL_COOLDOWN_THRESHOLD_SECS: u64 = 60;

pub fn spawn_lockout_monitor(state: SharedAppState, writer: SharedPtyWriter) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(NORMAL_POLL_INTERVAL_SECS));

        let detected_limit = {
            let mut app_state = state.lock().unwrap();

            if app_state.show_logs && !app_state.is_sleeping {
                log_to_file(
                    "[Polling] 5s Timer: Checking Claude session files for a rate limit...",
                );
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

        if let Some((target_time, time_str)) = detected_limit {
            log_to_file(&format!(
                "[LOCKOUT DETECTED] Rate limit hit! Target reset time found: {}",
                time_str
            ));
            wait_until_reset(target_time, &state, &writer);
        }
    });
}

fn wait_until_reset(
    target_time: chrono::DateTime<Local>,
    state: &SharedAppState,
    writer: &SharedPtyWriter,
) {
    loop {
        let now = Local::now();
        if now >= target_time {
            break;
        }

        let remaining = (target_time - now).num_seconds().max(1);
        let sleep_duration = cooldown_sleep_duration(remaining);
        let readable_time = format_duration(remaining);
        let readable_sleep = format_duration(sleep_duration.as_secs() as i64);

        log_to_file(&format!(
            "[Lockout Cooldown] Waiting... {} remaining until automated reset. Next check in {}.",
            readable_time, readable_sleep
        ));
        thread::sleep(sleep_duration);
    }

    log_to_file(
        "[Automation Trigger] Reset time reached! Injecting 'continue\\n' command into Claude...",
    );
    let mut pty_writer = writer.lock().unwrap();
    let _ = pty_writer.write_all(b"continue\n");
    let _ = pty_writer.flush();

    let mut app_state = state.lock().unwrap();
    app_state.is_sleeping = false;
    app_state.file_size_cache.clear();
    log_to_file("[System] Resuming normal passive file monitoring.");
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

#[cfg(test)]
mod tests {
    use super::cooldown_sleep_duration;
    use std::time::Duration;

    #[test]
    fn cooldown_checks_less_often_when_reset_is_far_away() {
        assert_eq!(
            cooldown_sleep_duration(4 * 60 * 60),
            Duration::from_secs(60 * 60)
        );
        assert_eq!(
            cooldown_sleep_duration(2 * 60 * 60),
            Duration::from_secs(30 * 60)
        );
        assert_eq!(
            cooldown_sleep_duration(30 * 60),
            Duration::from_secs(5 * 60)
        );
    }

    #[test]
    fn cooldown_checks_more_often_as_reset_gets_closer() {
        assert_eq!(cooldown_sleep_duration(10 * 60), Duration::from_secs(60));
        assert_eq!(cooldown_sleep_duration(2 * 60), Duration::from_secs(15));
        assert_eq!(cooldown_sleep_duration(45), Duration::from_secs(5));
    }

    #[test]
    fn cooldown_never_sleeps_past_the_reset_time() {
        assert_eq!(cooldown_sleep_duration(3), Duration::from_secs(3));
        assert_eq!(cooldown_sleep_duration(0), Duration::from_secs(0));
        assert_eq!(cooldown_sleep_duration(-10), Duration::from_secs(0));
    }
}
