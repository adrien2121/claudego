use chrono::Local;
use std::fs::{OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::OnceLock;
use std::thread::{self, JoinHandle};

/// A message sent to the dedicated logger thread.
enum LogMessage {
    /// A single line of text to be written to the log.
    Line(String),
    /// A signal to flush the buffer and terminate the logger thread.
    Shutdown,
}

/// Global sender for the logging channel.
/// `OnceLock` is a modern and efficient way to handle global static initialization.
static LOGGER_SENDER: OnceLock<Sender<LogMessage>> = OnceLock::new();

/// Initializes the asynchronous logger, spawning a dedicated thread for file I/O.
///
/// This function should be called once at application startup. It returns the handle
/// to the logger thread, which should be used for a graceful shutdown.
pub fn init_logging() -> JoinHandle<()> {
    let (tx, rx) = channel::<LogMessage>();
    // This will succeed as we call init_logging() only once.
    let _ = LOGGER_SENDER.set(tx);

    thread::spawn(move || {
        let path = log_path();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("Failed to open log file for buffered writing.");
        let mut writer = BufWriter::new(file);

        // This loop blocks until a message is received or the channel is closed.
        for msg in rx {
            match msg {
                LogMessage::Line(line) => {
                    if writeln!(writer, "{}", line).is_err() {
                        // Stop logging if we can't write to the file.
                        break;
                    }
                    // Flush the buffer after each line. This is crucial for making
                    // logs visible in real-time to tailing utilities like `claudego-logs`.
                    if writer.flush().is_err() {
                        // If we can't flush, the file is probably gone.
                        break;
                    }
                }
                LogMessage::Shutdown => {
                    break; // Exit the loop to allow the thread to terminate.
                }
            }
        }
        // Ensure any remaining buffered content is written to disk before exiting.
        let _ = writer.flush();
    })
}

/// Signals the logger thread to shut down and waits for it to finish.
/// This ensures all pending log messages are flushed to the file.
pub fn shutdown_logging(handle: JoinHandle<()>) {
    if let Some(sender) = LOGGER_SENDER.get() {
        // The receiver might already be gone if the thread panicked, so ignore errors.
        let _ = sender.send(LogMessage::Shutdown);
    }
    // Wait for the logger thread to process all messages and exit.
    let _ = handle.join();
}

pub fn log_path() -> PathBuf {
    std::env::temp_dir().join("claudego.log")
}

pub fn reset_log_file() {
    let _ = std::fs::remove_file(log_path());
}

pub fn log_to_file(msg: &str) {
    if let Some(sender) = LOGGER_SENDER.get() {
        let line = format!("[{}] {}", Local::now().format("%H:%M:%S"), msg);
        // If the receiver has been dropped, the logger thread is dead.
        // We can't do anything about it, so we ignore the error.
        let _ = sender.send(LogMessage::Line(line));
    }
}

/// Logs a prefix message followed by pre-formatted, multi-line content.
pub fn log_with_content(prefix: &str, content: String) {
    if let Some(sender) = LOGGER_SENDER.get() {
        let prefix_line = format!("[{}] {}", Local::now().format("%H:%M:%S"), prefix);
        // The content is expected to not have a trailing newline, so we add one if needed.
        let full_log = format!("{}\n{}", prefix_line, content.trim_end());

        // Send the combined string as a single message to ensure it's written atomically.
        let _ = sender.send(LogMessage::Line(full_log));
    }
}
