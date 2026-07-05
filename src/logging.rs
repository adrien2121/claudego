use chrono::Local;
use std::fs::OpenOptions;
use std::io::Write;

pub const LOG_PATH: &str = "/tmp/claudego.log";

pub fn reset_log_file() {
    let _ = std::fs::remove_file(LOG_PATH);
}

pub fn log_to_file(msg: &str) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(LOG_PATH) {
        let _ = writeln!(file, "[{}] {}", Local::now().format("%H:%M:%S"), msg);
    }
}
