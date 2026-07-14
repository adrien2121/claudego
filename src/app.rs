use crate::cli::{select_runner, CommandSpec, RunnerKind};
use crate::logging;
use crate::models::{AppState, ChildOutcome};
use crate::monitor;
use crate::pty_bridge;
use crate::resume::ResumeTarget;
use crate::resume::StreamResumeCommand;
use crate::stream_json;
use crate::terminal::RawModeGuard;
use anyhow::Result;

use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

const PTY_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

enum ReaderDrain {
    Completed,
    JoinFailed(tokio::task::JoinError),
    TimedOut,
}

async fn drain_reader(handle: tokio::task::JoinHandle<()>, timeout: Duration) -> ReaderDrain {
    match tokio::time::timeout(timeout, handle).await {
        Ok(Ok(())) => ReaderDrain::Completed,
        Ok(Err(error)) => ReaderDrain::JoinFailed(error),
        Err(_) => ReaderDrain::TimedOut,
    }
}

async fn run_pty_interactive(
    command_spec: CommandSpec,
    state: Arc<Mutex<AppState>>,
) -> Result<ChildOutcome> {
    let mut session = pty_bridge::spawn_command_in_pty(command_spec)?;
    let _guard = RawModeGuard::init()?;

    let reader_handle = pty_bridge::spawn_output_reader(session.reader, Arc::clone(&state));
    pty_bridge::spawn_input_writer(Arc::clone(&session.writer));
    pty_bridge::spawn_resize_poller(session.master, session.initial_size);
    monitor::spawn_lockout_monitor(state, ResumeTarget::Pty(Arc::clone(&session.writer)));

    let child_wait_handle = tokio::task::spawn_blocking(move || session.child.wait());
    let status = child_wait_handle.await??;
    match drain_reader(reader_handle, PTY_DRAIN_TIMEOUT).await {
        ReaderDrain::Completed => {}
        ReaderDrain::JoinFailed(error) => {
            logging::log_to_file(&format!("[PTY Output Error] reader task failed: {error}"))
        }
        ReaderDrain::TimedOut => logging::log_to_file(&format!(
            "[PTY Output Error] reader drain timed out after {PTY_DRAIN_TIMEOUT:?}"
        )),
    }
    Ok(ChildOutcome::from_pty(status))
}

pub async fn run(show_logs: bool, command_spec: CommandSpec) -> Result<ChildOutcome> {
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
            println!(
                "[System] Warning: Live log viewer failed to start (logger did not become ready)."
            );
        }
    }

    let outcome = match select_runner(&command_spec) {
        RunnerKind::PtyInteractive => run_pty_interactive(command_spec, state).await,
        RunnerKind::StreamJsonPrint => {
            let (resume_tx, resume_rx) = mpsc::unbounded_channel::<StreamResumeCommand>();
            monitor::spawn_lockout_monitor(state.clone(), ResumeTarget::StreamJson(resume_tx));
            stream_json::run_stream_json_print(command_spec, state, resume_rx).await
        }
    };

    logging::log_to_file("[System] Child process exited. Shutting down.");
    // Gracefully shut down the logger, ensuring all messages are flushed.
    logging::shutdown_logging(logger_handle).await;

    outcome
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

#[cfg(test)]
mod tests {
    use super::{drain_reader, ReaderDrain};
    use std::time::Duration;

    #[tokio::test]
    async fn reader_drain_timeout_is_bounded() {
        let reader = tokio::spawn(std::future::pending::<()>());

        let result = drain_reader(reader, Duration::from_millis(10)).await;

        assert!(matches!(result, ReaderDrain::TimedOut));
    }
}
