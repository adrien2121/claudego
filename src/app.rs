use crate::logging;
use crate::models::AppState;
use crate::monitor;
use crate::pty_bridge;
use crate::terminal::RawModeGuard;
use anyhow::Result;

use std::thread::JoinHandle;
use std::sync::{Arc, Mutex};

use crate::cli::CommandSpec;

/// An RAII guard to ensure the logger is shut down gracefully.
/// When this guard is dropped (at the end of the `run` function's scope),
/// it signals the logger thread to flush all pending messages and terminate.
struct LoggerGuard(Option<JoinHandle<()>>);

impl Drop for LoggerGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            logging::log_to_file("[System] Main process shutting down. Flushing logs...");
            // Send the shutdown signal and wait for the logger thread to finish.
            logging::shutdown_logging(handle);
        }
    }
}

pub fn run(show_logs: bool, command_spec: CommandSpec) -> Result<()> {
    // Unconditionally start logging
    // 1. Clear the old log file before the logger thread starts.
    logging::reset_log_file();
    // 2. Initialize the new asynchronous logger.
    let logger_handle = logging::init_logging();
    // The first log message now goes through the new async system.
    let _logger_guard = LoggerGuard(Some(logger_handle));
    logging::log_to_file("System initialized. Logger active. Starting passive monitoring.");
    let state = Arc::new(Mutex::new(AppState::new()));

    if show_logs {
        open_logs_terminal();
    }

    let mut session = pty_bridge::spawn_command_in_pty(command_spec)?;
    let _guard = RawModeGuard::init()?;

    pty_bridge::spawn_output_reader(session.reader, Arc::clone(&state));
    pty_bridge::spawn_input_writer(Arc::clone(&session.writer));
    pty_bridge::spawn_resize_poller(session.master, session.initial_size);
    monitor::spawn_lockout_monitor(state, Arc::clone(&session.writer));

    let _ = session.child.wait()?;

    logging::log_to_file("[System] Child process exited. Shutting down.");
    Ok(())
}

fn open_logs_terminal() {
    let log_path = logging::log_path();
    println!("[System] Streaming live logs to {}", log_path.display());
    println!("[System] Launching claudego-logs in a new terminal...");

    // Find the absolute path to the claudego-logs binary (assumed to be in the same dir as claudego)
    let logs_bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|dir| dir.join("claudego-logs")))
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "claudego-logs".to_string());

    #[cfg(target_os = "macos")]
    {
        let script = format!(r#"tell application "Terminal" to do script "{}""#, logs_bin);
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .arg("/c")
            .arg(format!("start \"Claudego Logs\" \"{}\"", logs_bin))
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("gnome-terminal")
            .arg("--")
            .arg(&logs_bin)
            .spawn();
    }
}
