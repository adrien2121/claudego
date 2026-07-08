use crate::logging;
use crate::models::AppState;
use crate::monitor;
use crate::pty_bridge;
use crate::terminal::RawModeGuard;
use anyhow::Result;

use std::sync::{Arc, Mutex};

use crate::cli::CommandSpec;

pub async fn run(show_logs: bool, command_spec: CommandSpec) -> Result<()> {
    // Unconditionally start logging
    // 1. Clear the old log file before the logger thread starts.
    logging::reset_log_file();
    // 2. Initialize the new asynchronous logger.
    // It now returns a handle and a receiver to signal when the TCP server is ready.
    let (logger_handle, logger_ready_rx) = logging::init_logging();
    logging::log_to_file("System initialized. Logger active. Starting passive monitoring.");
    let state = Arc::new(Mutex::new(AppState::new()));

    if show_logs {
        // Wait for the logger thread to signal that the TCP server is bound and ready.
        // This prevents a race condition where claudego-logs starts before the port is known.
        // We use a blocking recv here because it's part of the initial setup, before the main async logic.
        if logger_ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .is_ok()
        {
            open_logs_terminal();
        } else {
            println!("[System] Warning: Live log viewer failed to start (logger did not become ready).");
        }
    }

    let mut session = pty_bridge::spawn_command_in_pty(command_spec)?;
    let _guard = RawModeGuard::init()?;

    pty_bridge::spawn_output_reader(session.reader, Arc::clone(&state));
    pty_bridge::spawn_input_writer(Arc::clone(&session.writer));
    pty_bridge::spawn_resize_poller(session.master, session.initial_size);
    monitor::spawn_lockout_monitor(state, Arc::clone(&session.writer));

    // Move the blocking `wait` call to a dedicated thread to avoid blocking the tokio runtime.
    let child_wait_handle = tokio::task::spawn_blocking(move || session.child.wait());
    let _ = child_wait_handle.await??;

    logging::log_to_file("[System] Child process exited. Shutting down.");
    // Gracefully shut down the logger, ensuring all messages are flushed.
    logging::shutdown_logging(logger_handle).await;

    Ok(())
}

fn open_logs_terminal() {
    println!("[System] Live log streaming enabled.");
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
