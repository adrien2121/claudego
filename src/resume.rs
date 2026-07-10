use crate::pty_bridge::SharedPtyWriter;
use std::io::Write;
use tokio::sync::mpsc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamResumeCommand {
    Continue,
}

#[derive(Clone)]
pub enum ResumeTarget {
    Pty(SharedPtyWriter),
    StreamJson(mpsc::UnboundedSender<StreamResumeCommand>),
}

impl ResumeTarget {
    pub fn resume(&self) -> Result<(), String> {
        match self {
            ResumeTarget::Pty(writer) => {
                let mut writer = writer.lock().unwrap_or_else(|e| e.into_inner());
                writer
                    .write_all(b"continue\r")
                    .map_err(|e| format!("failed to write PTY continue command: {e}"))?;
                writer
                    .flush()
                    .map_err(|e| format!("failed to flush PTY continue command: {e}"))
            }
            ResumeTarget::StreamJson(tx) => tx
                .send(StreamResumeCommand::Continue)
                .map_err(|_| "stream-json runner is no longer available".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ResumeTarget, StreamResumeCommand};
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

        ResumeTarget::Pty(writer).resume().expect("resume succeeds");

        assert_eq!(&*bytes.lock().unwrap(), b"continue\r");
    }

    #[tokio::test]
    async fn stream_resume_sends_continue_command() {
        let (tx, mut rx) = mpsc::unbounded_channel();

        ResumeTarget::StreamJson(tx)
            .resume()
            .expect("resume succeeds");

        assert_eq!(rx.recv().await, Some(StreamResumeCommand::Continue));
    }
}
