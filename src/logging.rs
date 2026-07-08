use chrono::Local;
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::OnceLock;
use std::thread::{self, JoinHandle};
use std::time::Duration;

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

/// The maximum size of the log file in bytes before it is rotated. (10 MiB)
const MAX_LOG_SIZE: u64 = 10 * 1024 * 1024;

/// Initializes the asynchronous logger, spawning a dedicated thread for file I/O.
///
/// This function should be called once at application startup. It returns the handle
/// to the logger thread, which should be used for a graceful shutdown.
pub fn init_logging() -> (JoinHandle<()>, Receiver<()>) {
    // Clean up the port file from any previous unclean shutdown.
    let _ = fs::remove_file(port_path());

    let (log_tx, log_rx) = channel::<LogMessage>();
    // This will succeed as we call init_logging() only once.
    let _ = LOGGER_SENDER.set(log_tx);

    // Channel to signal that the TCP listener is ready.
    let (ready_tx, ready_rx) = channel::<()>();

    let handle = thread::spawn(move || {
        let path = log_path();

        // --- 1. Set up network listener for live logs ---
        let listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(l) => l,
            Err(_) => { /* Failed to bind, live logging will be unavailable */ return; }
        };
        if let Ok(addr) = listener.local_addr() {
            if fs::write(port_path(), addr.port().to_string()).is_ok() {
                // Successfully wrote the port file. Signal that we are ready.
                let _ = ready_tx.send(());
            }
        }
        listener.set_nonblocking(true).expect("Failed to set listener to non-blocking");
        let mut clients: Vec<TcpStream> = Vec::new();

        // --- 2. Set up file writer for persistent logs ---
        let mut writer = {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .expect("Failed to open log file for buffered writing.");
            BufWriter::new(file)
        };

        // Get initial size. Since we reset on start, this should be 0,
        // but this is more robust in case reset_log_file is changed.
        let mut current_size = path.metadata().map(|m| m.len()).unwrap_or(0);

        // --- 3. Main logging loop (polls for clients and messages) ---
        loop {
            // A. Accept any new incoming connections for live logging.
            if let Ok((stream, _)) = listener.accept() {
                // We don't care about errors here, best-effort delivery.
                let _ = stream.set_nonblocking(true);
                clients.push(stream);
            }

            // B. Check for a message from the application.
            match log_rx.try_recv() {
                Ok(LogMessage::Line(mut line)) => {
                    line.push('\n'); // Ensure line has a newline for streams

                    // B.1. Send to live-log network clients
                    clients.retain(|mut client| {
                        // `retain` keeps elements for which the closure returns true.
                        // If write fails, the closure returns false, removing the client.
                        client.write_all(line.as_bytes()).is_ok()
                    });

                    // B.2. Write to persistent log file (buffered)
                    // --- Log Rotation Logic ---
                    if current_size > MAX_LOG_SIZE {
                        // 1. Flush and close the current file.
                        // We only flush here, during rotation, not on every line.
                        if writer.flush().is_err() { return; }
                        drop(writer); // Explicitly drop to close the file handle.

                        // 2. Rotate the log file.
                        let backup_path = path.with_extension("log.old");
                        if std::fs::rename(&path, &backup_path).is_err() {
                            // Failed to rename, maybe permissions issue. Can't continue.
                            return;
                        }

                        // 3. Open a new file and create a new writer.
                        let new_file = match OpenOptions::new().create(true).append(true).open(&path) {
                            Ok(f) => f,
                            Err(_) => return, // Cannot open new log file, stop logging.
                        };
                        writer = BufWriter::new(new_file);
                        current_size = 0;

                        // 4. Log a message about the rotation.
                        let rotation_msg = format!("[{}] Log file exceeded {}MB and was rotated. Previous log saved to: {}",
                            Local::now().format("%H:%M:%S"),
                            MAX_LOG_SIZE / (1024 * 1024),
                            backup_path.display()
                        );
                        if writeln!(writer, "{}", rotation_msg).is_ok() {
                            current_size += rotation_msg.len() as u64 + 1;
                        }
                    }

                    // --- Write the actual log message ---
                    let bytes_to_write = line.len() as u64;
                    if writer.write_all(line.as_bytes()).is_err() {
                        return;
                    }
                    current_size += bytes_to_write;

                }
                Ok(LogMessage::Shutdown) => {
                    break;
                }
                Err(TryRecvError::Empty) => {
                    // No messages, pause briefly to prevent a busy-loop.
                    thread::sleep(Duration::from_millis(50));
                }
                Err(TryRecvError::Disconnected) => {
                    // The main application has shut down.
                    break;
                }
            }
        }
        // Ensure any remaining buffered content is written to disk before exiting.
        let _ = writer.flush();
        // Clean up the port file on graceful shutdown.
        let _ = fs::remove_file(port_path());
    });

    (handle, ready_rx)
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

fn port_path() -> PathBuf {
    std::env::temp_dir().join("claudego.port")
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
