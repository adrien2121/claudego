use std::time::Duration;

// ── Tuning constants ────────────────────────────────────────────────────────

/// How long to coalesce rapid-fire events before scanning.
pub(super) const DEBOUNCE_DURATION: Duration = Duration::from_millis(1000);

/// Max retry attempts when creating the OS file watcher.
pub(super) const WATCHER_MAX_RETRIES: u32 = 3;

// Cooldown log cadence thresholds (seconds remaining → log interval).
const VERY_FAR_THRESHOLD_SECS: u64 = 3 * 60 * 60; // > 3 h  → log every 1 h
const FAR_THRESHOLD_SECS: u64 = 60 * 60; //       > 1 h  → every 30 m
const MEDIUM_THRESHOLD_SECS: u64 = 15 * 60; //       > 15 m → every 5 m
const NEAR_THRESHOLD_SECS: u64 = 5 * 60; //       > 5 m  → every 1 m
const FINAL_THRESHOLD_SECS: u64 = 60; //       > 1 m  → every 15 s
const MIN_LOG_INTERVAL_SECS: u64 = 5; //       else   → every 5 s

/// Adaptive log interval: frequent when close to reset, sparse when far away.
pub(super) fn cooldown_log_interval(remaining_seconds: i64) -> Duration {
    if remaining_seconds <= 0 {
        return Duration::from_secs(0);
    }
    let r = remaining_seconds as u64;
    let secs = if r > VERY_FAR_THRESHOLD_SECS {
        60 * 60
    } else if r > FAR_THRESHOLD_SECS {
        30 * 60
    } else if r > MEDIUM_THRESHOLD_SECS {
        5 * 60
    } else if r > NEAR_THRESHOLD_SECS {
        60
    } else if r > FINAL_THRESHOLD_SECS {
        15
    } else {
        MIN_LOG_INTERVAL_SECS
    };
    Duration::from_secs(secs.min(r))
}