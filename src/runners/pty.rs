use crate::cli::CommandSpec;
use crate::harness::{RunContext, RunFuture, Runner};
use crate::logging;
use crate::models::ChildOutcome;
use crate::monitor;
use crate::pty_bridge;
use crate::resume::PtyResumeSink;
use crate::terminal::RawModeGuard;
use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;

const PTY_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

enum ReaderDrain {
    Completed,
    JoinFailed(tokio::task::JoinError),
    TimedOut,
}

pub struct PtyRunner {
    command: CommandSpec,
}

impl PtyRunner {
    pub fn new(command: CommandSpec) -> Self {
        Self { command }
    }
}

impl Runner for PtyRunner {
    fn run(self: Box<Self>, context: RunContext) -> RunFuture {
        Box::pin(async move { run_pty(self.command, context).await })
    }
}

async fn drain_reader(handle: tokio::task::JoinHandle<()>, timeout: Duration) -> ReaderDrain {
    match tokio::time::timeout(timeout, handle).await {
        Ok(Ok(())) => ReaderDrain::Completed,
        Ok(Err(error)) => ReaderDrain::JoinFailed(error),
        Err(_) => ReaderDrain::TimedOut,
    }
}

async fn run_pty(command: CommandSpec, context: RunContext) -> Result<ChildOutcome> {
    let mut session = pty_bridge::spawn_command_in_pty(command)?;
    let _guard = RawModeGuard::init()?;

    let reader_handle = pty_bridge::spawn_output_reader(session.reader, Arc::clone(&context.state));
    pty_bridge::spawn_input_writer(Arc::clone(&session.writer));
    pty_bridge::spawn_resize_poller(session.master, session.initial_size);
    let resume_sink = Arc::new(PtyResumeSink::new(Arc::clone(&session.writer)));
    monitor::spawn_lockout_monitor(context.state, context.monitor, resume_sink);

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
