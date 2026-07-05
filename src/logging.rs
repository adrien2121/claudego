use chrono::Local;
use std::fs::OpenOptions;
use std::io::Write;

use std::path::PathBuf;

pub fn log_path() -> PathBuf {
    std::env::temp_dir().join("claudego.log")
}

pub fn reset_log_file() {
    let _ = std::fs::remove_file(log_path());
}

pub fn log_to_file(msg: &str) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_path()) {
        let _ = writeln!(file, "[{}] {}", Local::now().format("%H:%M:%S"), msg);
    }
}
