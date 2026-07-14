use anyhow::{Context, Result};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

fn requested_pid() -> Result<Option<u32>> {
    let mut args = std::env::args().skip(1);
    let pid = args
        .next()
        .map(|value| value.parse::<u32>().context("PID must be a decimal u32"))
        .transpose()?;
    anyhow::ensure!(args.next().is_none(), "usage: claudego-logs [pid]");
    Ok(pid)
}

fn candidate_port_files(pid: Option<u32>) -> Result<Vec<PathBuf>> {
    candidate_port_files_in(&std::env::temp_dir(), pid)
}

fn candidate_port_files_in(temp_dir: &Path, pid: Option<u32>) -> Result<Vec<PathBuf>> {
    if let Some(pid) = pid {
        return Ok(vec![
            claudego::paths::LoggerPaths::for_pid_in(temp_dir, pid).port,
        ]);
    }
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(temp_dir)?.flatten() {
        if claudego::paths::pid_from_port_path(&entry.path()).is_none() {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) else {
            continue;
        };
        candidates.push((modified, entry.path()));
    }
    candidates.sort_by(|left, right| right.0.cmp(&left.0));
    Ok(candidates.into_iter().map(|(_, path)| path).collect())
}

fn try_connect_and_stream(pid: Option<u32>) -> Result<()> {
    try_connect_from_candidates(candidate_port_files(pid)?)
}

#[cfg(test)]
fn try_connect_and_stream_from(temp_dir: &Path, pid: Option<u32>) -> Result<()> {
    try_connect_from_candidates(candidate_port_files_in(temp_dir, pid)?)
}

fn try_connect_from_candidates(candidates: Vec<PathBuf>) -> Result<()> {
    let mut last_error = None;
    for port_path in candidates {
        match stream_from_port_file(&port_path) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no reachable claudego logger")))
}

fn stream_from_port_file(port_path: &Path) -> Result<()> {
    let port = std::fs::read_to_string(port_path)?.trim().parse::<u16>()?;
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
    println!("Connected. Streaming logs from claudego process...\n");
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    while reader.read_line(&mut line)? > 0 {
        print!("{line}");
        line.clear();
    }
    Ok(())
}

fn is_relevant_port_file_event(event: &notify::Event, pid: Option<u32>) -> bool {
    use notify::EventKind;
    matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_))
        && event.paths.iter().any(|path| match pid {
            Some(pid) => path == &claudego::paths::LoggerPaths::for_pid(pid).port,
            None => claudego::paths::pid_from_port_path(path).is_some(),
        })
}

fn main() -> Result<()> {
    let pid = requested_pid()?;
    println!("Waiting for claudego session to start...");
    loop {
        if try_connect_and_stream(pid).is_ok() {
            println!("\nConnection to claudego process lost. Waiting for it to restart...");
            continue;
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher: RecommendedWatcher =
            notify::recommended_watcher(tx).context("Failed to create filesystem watcher")?;
        watcher
            .watch(&std::env::temp_dir(), RecursiveMode::NonRecursive)
            .context("Failed to start watching temp directory")?;

        for result in rx {
            if let Ok(event) = result {
                if is_relevant_port_file_event(&event, pid) {
                    thread::sleep(Duration::from_millis(50));
                    if try_connect_and_stream(pid).is_ok() {
                        println!(
                            "\nConnection to claudego process lost. Waiting for it to restart..."
                        );
                    }
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_relevant_port_file_event, try_connect_and_stream_from};

    #[test]
    fn bare_mode_skips_newest_unreachable_port() {
        use std::fs::{self, File, FileTimes};
        use std::net::TcpListener;
        use std::time::{Duration, SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "claudego-viewer-test-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let live_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let live_port = live_listener.local_addr().unwrap().port();
        let dead_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let dead_port = dead_listener.local_addr().unwrap().port();
        drop(dead_listener);
        let older = directory.join("claudego-41.port");
        let newer = directory.join("claudego-42.port");
        fs::write(&older, live_port.to_string()).unwrap();
        fs::write(&newer, dead_port.to_string()).unwrap();
        File::options()
            .write(true)
            .open(&older)
            .unwrap()
            .set_times(FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(1)))
            .unwrap();
        let accept = std::thread::spawn(move || {
            let _ = live_listener.accept().unwrap();
        });

        assert!(try_connect_and_stream_from(&directory, None).is_ok());
        accept.join().unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn event_filter_is_exact_for_requested_pid() {
        let event = notify::Event::new(notify::EventKind::Create(notify::event::CreateKind::File))
            .add_path(claudego::paths::LoggerPaths::for_pid(41).port);
        assert!(is_relevant_port_file_event(&event, Some(41)));
        assert!(!is_relevant_port_file_event(&event, Some(42)));
        assert!(is_relevant_port_file_event(&event, None));
    }
}
