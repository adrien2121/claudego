use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Single source of truth for the current rate-limit lockout.
/// Only modified when a file scan detects (or clears) a limit.
pub struct LockoutState {
    pub target_time: Option<chrono::DateTime<chrono::Local>>,
}

pub struct AppState {
    pub is_sleeping: bool,
    pub file_size_cache: HashMap<PathBuf, u64>,
    pub lockout: LockoutState,
    /// Timestamp of the last time data was received from the PTY.
    pub last_pty_activity: Instant,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            is_sleeping: false,
            file_size_cache: HashMap::new(),
            lockout: LockoutState { target_time: None },
            // Initialize to a long time ago so it's not considered busy on startup.
            last_pty_activity: Instant::now()
                .checked_sub(std::time::Duration::from_secs(9999))
                .unwrap_or_else(Instant::now),
        }
    }
}

pub type SharedAppState = Arc<Mutex<AppState>>;
