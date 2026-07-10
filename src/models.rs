use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

pub type OutputActivity = Arc<AtomicU64>;

pub struct AppState {
    pub lockout_target_time: Option<chrono::DateTime<chrono::Local>>,
    /// Increments for live stream/file events; startup scan establishes baseline state.
    pub lockout_revision: u64,
    pub file_size_cache: HashMap<PathBuf, u64>,
    pub last_output_activity: OutputActivity,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            lockout_target_time: None,
            lockout_revision: 0,
            file_size_cache: HashMap::new(),
            last_output_activity: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

pub type SharedAppState = Arc<Mutex<AppState>>;

pub fn mark_output_activity(activity: &AtomicU64) {
    let now_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    activity.store(now_nanos, std::sync::atomic::Ordering::Relaxed);
}

pub fn output_is_hot(activity: &AtomicU64, threshold: std::time::Duration) -> bool {
    let last_activity_ns = activity.load(std::sync::atomic::Ordering::Relaxed);
    if last_activity_ns == 0 {
        return false;
    }

    let now_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    std::time::Duration::from_nanos(now_ns.saturating_sub(last_activity_ns)) < threshold
}

#[cfg(test)]
mod tests {
    use super::{mark_output_activity, output_is_hot, AppState};
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    #[test]
    fn new_state_has_cold_output_activity() {
        let state = AppState::new();

        assert_eq!(state.last_output_activity.load(Ordering::Relaxed), 0);
        assert!(!output_is_hot(
            &state.last_output_activity,
            Duration::from_secs(2)
        ));
    }

    #[test]
    fn marking_activity_makes_output_hot() {
        let state = AppState::new();

        mark_output_activity(&state.last_output_activity);

        assert!(output_is_hot(
            &state.last_output_activity,
            Duration::from_secs(2)
        ));
    }
}
