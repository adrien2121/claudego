use anyhow::{Context, Result};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::io::{BufRead, BufReader};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

/// Returns the path to the file that stores the port number for the live log server.
fn port_path() -> PathBuf {
    std::env::temp_dir().join("claudego.port")
}

fn main() -> Result<()> {
    println!("Waiting for claudego session to start...");

    // This outer loop allows the logger to automatically recover and wait for a new
    // session if the main `claudego` process is restarted.
    loop {
        // First, try to connect immediately in case the session is already running.
        if try_connect_and_stream().is_ok() {
            // If streaming finishes (e.g., connection lost), we just loop again.
            println!("\nConnection to claudego process lost. Waiting for it to restart...");
            continue;
        }

        // If we can't connect, set up a watcher on the port file's parent directory.
        // This is far more efficient than polling the file in a loop.
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher: RecommendedWatcher = notify::recommended_watcher(tx)
            .context("Failed to create filesystem watcher")?;

        // Watch the temp directory for the creation of our port file.
        watcher
            .watch(&std::env::temp_dir(), RecursiveMode::NonRecursive)
            .context("Failed to start watching temp directory")?;

        // Block and wait for a relevant file system event.
        for res in rx {
            match res {
                Ok(event) if is_relevant_port_file_event(&event) => {
                    // The port file was created or written to. Give it a moment to settle,
                    // then try connecting.
                    thread::sleep(Duration::from_millis(50));
                    if try_connect_and_stream().is_ok() {
                        println!("\nConnection to claudego process lost. Waiting for it to restart...");
                    }
                    // Break the inner `for` loop to re-establish the watcher.
                    break;
                }
                _ => {
                    // Ignore other events or errors.
                }
            }
        }
    }
}

/// Checks if a filesystem event is relevant to the `claudego.port` file.
fn is_relevant_port_file_event(event: &notify::Event) -> bool {
    use notify::EventKind;
    matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_))
        && event.paths.iter().any(|p| p.ends_with("claudego.port"))
}

/// Attempts to read the port file, connect to the TCP server, and stream logs.
/// Returns an error if any step fails, allowing the caller to retry.
fn try_connect_and_stream() -> Result<()> {
    let port_str = std::fs::read_to_string(port_path()).context("Port file not found or unreadable")?;
    let port = port_str
        .trim()
        .parse::<u16>()
        .context("Could not parse port number")?;

    // No unwrap! Handle the parse result gracefully.
    let addr: SocketAddr = format!("127.0.0.1:{}", port)
        .parse()
        .context("Could not form valid socket address")?;

    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .context("Failed to connect to claudego process")?;

    println!("Connected. Streaming logs from claudego process...\n");
    let mut reader = BufReader::new(stream);
    let mut line = String::new();

    // Read from the stream until the connection is closed. `read_line` returns 0 on EOF.
    while reader.read_line(&mut line)? > 0 {
        print!("{}", line);
        line.clear();
    }

    Ok(())
}
