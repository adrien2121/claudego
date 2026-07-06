use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Single source of truth for the current rate-limit lockout.
/// Only modified when a file scan detects (or clears) a limit.
pub struct LockoutState {
    pub target_time: Option<chrono::DateTime<chrono::Local>>,
}

pub struct AppState {
    pub is_sleeping: bool,
    pub file_size_cache: HashMap<PathBuf, u64>,
    pub lockout: LockoutState,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            is_sleeping: false,
            file_size_cache: HashMap::new(),
            lockout: LockoutState { target_time: None },
        }
    }
}

pub type SharedAppState = Arc<Mutex<AppState>>;
