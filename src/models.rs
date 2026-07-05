use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub is_sleeping: bool,
    pub show_logs: bool,
    pub file_size_cache: HashMap<PathBuf, u64>,
}

impl AppState {
    pub fn new(show_logs: bool) -> Self {
        Self {
            is_sleeping: false,
            show_logs,
            file_size_cache: HashMap::new(),
        }
    }
}

pub type SharedAppState = Arc<Mutex<AppState>>;
