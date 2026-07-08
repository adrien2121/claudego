use chrono::Local;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::Receiver as StdReceiver;
use std::sync::OnceLock;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{self, Sender};

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
pub fn init_logging() -> (tokio::task::JoinHandle<()>, StdReceiver<()>) {
    // Clean up the port file from any previous unclean shutdown.
    let _ = fs::remove_file(port_path());

    let (log_tx, log_rx) = mpsc::channel::<LogMessage>(100); // Use a buffered channel
    // This will succeed as we call init_logging() only once.
    let _ = LOGGER_SENDER.set(log_tx);

    // Channel to signal that the TCP listener is ready.
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();

    let handle = tokio::spawn(async move {
        let path = log_path();

        // --- 1. Set up network listener for live logs ---
        let listener = match TcpListener::bind("127.0.0.1:0").await {
            Ok(l) => l,
            Err(_) => { /* Failed to bind, live logging will be unavailable */ return; }
        };
        if let Ok(addr) = listener.local_addr() {
            if fs::write(port_path(), addr.port().to_string()).is_ok() {
                // Successfully wrote the port file. Signal that we are ready.
                let _ = ready_tx.send(()); // Use blocking send for initial setup
            }
        }
        let mut clients: Vec<TcpStream> = Vec::new();

        // Buffer for logs generated before the first client connects. This ensures
        // the log viewer gets the full startup history.
        let mut initial_buffer: Vec<String> = Vec::new();
        let mut has_first_client_connected = false;

        // --- 2. Set up file writer for persistent logs ---
        let mut writer = match tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
        {
            Ok(file) => BufWriter::new(file),
            Err(_) => return, // Cannot open log file, stop logging.
        };

        // Get initial size.
        let mut current_size = tokio::fs::metadata(&path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);

        // --- 3. Main logging loop (selects between clients and messages) ---
        let mut log_rx = log_rx; // Move receiver into the loop
        loop {
            tokio::select! {
                // A. Accept any new incoming connections for live logging.
                Ok((mut stream, _)) = listener.accept() => {
                    let mut client_ok = true;
                    if !has_first_client_connected {
                        // This is the first client. Dump the startup log buffer to it.
                        for line in &initial_buffer {
                            if stream.write_all(line.as_bytes()).await.is_err() {
                                // If we can't even write the buffer, this client is no good.
                                client_ok = false;
                                break;
                            }
                        }
                        if client_ok {
                            // Successfully dumped buffer. Transition to live streaming mode.
                            initial_buffer.clear();
                            initial_buffer.shrink_to_fit();
                            has_first_client_connected = true;
                        }
                    }
                    if client_ok {
                        clients.push(stream);
                    }
                }

                // B. Check for a message from the application.
                Some(msg) = log_rx.recv() => {
                    match msg {
                        LogMessage::Line(mut line) => {
                            line.push('\n'); // Ensure line has a newline for streams

                            // B.1. Send to live-log network clients or buffer if none have connected.
                            if !has_first_client_connected {
                                initial_buffer.push(line.clone());
                            } else {
                                let mut dead_clients = Vec::new();
                                for (i, client) in clients.iter_mut().enumerate() {
                                    if client.write_all(line.as_bytes()).await.is_err() {
                                        dead_clients.push(i);
                                    }
                                }
                                // Remove dead clients in reverse to preserve indices
                                for i in dead_clients.into_iter().rev() {
                                    clients.remove(i);
                                }
                            }

                            // B.2. Write to persistent log file (buffered)
                            // --- Log Rotation Logic ---
                            if current_size > MAX_LOG_SIZE {
                                if writer.flush().await.is_err() { return; }
                                drop(writer); // Close file

                                let backup_path = path.with_extension("log.old");
                                if tokio::fs::rename(&path, &backup_path).await.is_err() { return; }

                                let new_file = match tokio::fs::OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(&path)
                                .await {
                                    Ok(f) => f,
                                    Err(_) => return,
                                };
                                writer = BufWriter::new(new_file);
                                current_size = 0;

                                let rotation_msg = format!("[{}] Log file exceeded {}MB and was rotated. Previous log saved to: {}\n",
                                    Local::now().format("%H:%M:%S"),
                                    MAX_LOG_SIZE / (1024 * 1024),
                                    backup_path.display()
                                );
                                if writer.write_all(rotation_msg.as_bytes()).await.is_ok() {
                                    current_size += rotation_msg.len() as u64;
                                }
                            }

                            // --- Write the actual log message ---
                            let bytes_to_write = line.len() as u64;
                            if writer.write_all(line.as_bytes()).await.is_err() {
                                return;
                            }
                            current_size += bytes_to_write;
                        }
                        LogMessage::Shutdown => {
                            break; // Exit the select loop
                        }
                    }
                }
                else => {
                    // Channel closed, main app has shut down.
                    break;
                }
            }
        }
        // Ensure any remaining buffered content is written to disk before exiting.
        let _ = writer.flush().await;
        // Clean up the port file on graceful shutdown.
        let _ = fs::remove_file(port_path());
    });

    (handle, ready_rx)
}

/// Signals the logger thread to shut down and waits for it to finish.
/// This ensures all pending log messages are flushed to the file.
pub async fn shutdown_logging(handle: tokio::task::JoinHandle<()>) {
    if let Some(sender) = LOGGER_SENDER.get() {
        // The receiver might already be gone if the thread panicked, so ignore errors.
        let _ = sender.send(LogMessage::Shutdown).await;
    }
    // Wait for the logger thread to process all messages and exit.
    let _ = handle.await;
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
        // We use a non-blocking `try_send` because this function is not async.
        let _ = sender.try_send(LogMessage::Line(line));
    }
}

/// Logs a prefix message followed by pre-formatted, multi-line content.
pub fn log_with_content(prefix: &str, content: String) {
    if let Some(sender) = LOGGER_SENDER.get() {
        let prefix_line = format!("[{}] {}", Local::now().format("%H:%M:%S"), prefix);
        // The content is expected to not have a trailing newline, so we add one if needed.
        let full_log = format!("{}\n{}", prefix_line, content.trim_end());

        // Send the combined string as a single message to ensure it's written atomically.
        let _ = sender.try_send(LogMessage::Line(full_log));
    }
}
