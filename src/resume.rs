use crate::harness::{ResumeOutcome, ResumeSink};
use crate::pty_bridge::SharedPtyWriter;
use std::io::Write;

pub struct PtyResumeSink {
    writer: SharedPtyWriter,
}

impl PtyResumeSink {
    pub fn new(writer: SharedPtyWriter) -> Self {
        Self { writer }
    }
}

impl ResumeSink for PtyResumeSink {
    fn resume(&self) -> ResumeOutcome {
        let mut writer = self
            .writer
            .lock()
            .unwrap_or_else(|error| error.into_inner());
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
}

#[cfg(test)]
mod tests {
    use super::PtyResumeSink;
    use crate::harness::{ResumeOutcome, ResumeSink};
    use crate::pty_bridge::SharedPtyWriter;
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

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
        let sink = PtyResumeSink::new(writer);

        assert_eq!(sink.resume(), ResumeOutcome::Sent);
        assert_eq!(&*bytes.lock().unwrap(), b"continue\r");
    }
}
