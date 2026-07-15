use crate::harness::{ResumeOutcome as HarnessResumeOutcome, ResumeSink};
use crate::pty_bridge::SharedPtyWriter;
use std::io::Write;
use tokio::sync::mpsc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamResumeCommand {
    Continue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResumeOutcome {
    Sent,
    DefiniteFailure(String),
    AmbiguousFailure(String),
}

#[derive(Clone)]
pub enum ResumeTarget {
    Pty(SharedPtyWriter),
    StreamJson(mpsc::UnboundedSender<StreamResumeCommand>),
}

impl ResumeTarget {
    pub fn resume(&self) -> ResumeOutcome {
        match self {
            ResumeTarget::Pty(writer) => {
                let mut writer = writer.lock().unwrap_or_else(|e| e.into_inner());
                if let Err(error) = writer.write_all(b"continue\r") {
                    return ResumeOutcome::AmbiguousFailure(format!(
                        "failed to write PTY continue command: {error}"
                    ));
                }
                match writer.flush() {
                    Ok(()) => ResumeOutcome::Sent,
                    Err(error) => ResumeOutcome::AmbiguousFailure(format!(
                        "failed to flush PTY continue command: {error}"
                    )),
                }
            }
            ResumeTarget::StreamJson(tx) => match tx.send(StreamResumeCommand::Continue) {
                Ok(()) => ResumeOutcome::Sent,
                Err(_) => ResumeOutcome::DefiniteFailure(
                    "stream-json runner is no longer available".to_string(),
                ),
            },
        }
    }
}

impl ResumeSink for ResumeTarget {
    fn resume(&self) -> HarnessResumeOutcome {
        match ResumeTarget::resume(self) {
            ResumeOutcome::Sent => HarnessResumeOutcome::Sent,
            ResumeOutcome::DefiniteFailure(error) => HarnessResumeOutcome::DefiniteFailure(error),
            ResumeOutcome::AmbiguousFailure(error) => HarnessResumeOutcome::AmbiguousFailure(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ResumeOutcome, ResumeTarget, StreamResumeCommand};
    use crate::pty_bridge::SharedPtyWriter;
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    #[derive(Default)]
    struct MemoryWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for MemoryWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.bytes.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn pty_resume_writes_continue_with_carriage_return() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let writer = MemoryWriter {
            bytes: Arc::clone(&bytes),
        };
        let writer: SharedPtyWriter = Arc::new(Mutex::new(Box::new(writer)));

        assert_eq!(ResumeTarget::Pty(writer).resume(), ResumeOutcome::Sent);

        assert_eq!(&*bytes.lock().unwrap(), b"continue\r");
    }

    #[tokio::test]
    async fn stream_resume_sends_continue_command() {
        let (tx, mut rx) = mpsc::unbounded_channel();

        assert_eq!(ResumeTarget::StreamJson(tx).resume(), ResumeOutcome::Sent);

        assert_eq!(rx.recv().await, Some(StreamResumeCommand::Continue));
    }

    #[tokio::test]
    async fn closed_stream_is_a_definite_failure() {
        let (tx, rx) = mpsc::unbounded_channel();
        drop(rx);

        assert_eq!(
            ResumeTarget::StreamJson(tx).resume(),
            ResumeOutcome::DefiniteFailure("stream-json runner is no longer available".to_string())
        );
    }
}
