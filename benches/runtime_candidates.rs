//! Raw, dependency-free measurements for runtime-hardening candidates.

use serde_json::json;
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::process;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

mod logging {
    pub fn log_to_file(_: &str) {}
}
mod models {
    use super::*;
    pub struct AppState {
        pub file_size_cache: HashMap<PathBuf, u64>,
        pub lockout_target_time: Option<chrono::DateTime<chrono::Local>>,
        pub latest_rate_limit_event_time: Option<chrono::DateTime<chrono::Local>>,
    }
    pub type SharedAppState = Arc<std::sync::Mutex<AppState>>;
}
mod monitor {
    pub mod helpers {
        pub const SCAN_CHUNK_SIZE: usize = 65_536;
    }
}
mod watcher {
    pub mod files {
        use std::path::{Path, PathBuf};
        use std::time::SystemTime;
        pub fn claude_projects_root() -> Option<PathBuf> {
            dirs::home_dir().map(|home| home.join(".claude/projects"))
        }
        pub fn recent_session_logs(root: &Path, after: SystemTime) -> Vec<(PathBuf, SystemTime)> {
            let mut files = Vec::new();
            if let Ok(projects) = std::fs::read_dir(root) {
                for project in projects.flatten().filter(|p| p.path().is_dir()) {
                    if let Ok(entries) = std::fs::read_dir(project.path()) {
                        for entry in entries.flatten() {
                            if let Ok(meta) = entry.metadata() {
                                if meta.is_file()
                                    && entry.path().extension().and_then(|x| x.to_str())
                                        == Some("jsonl")
                                {
                                    if let Ok(modified) = meta.modified() {
                                        if modified > after {
                                            files.push((entry.path(), modified));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            files
        }
    }
    pub mod scan {
        use chrono::{DateTime, Local};
        #[allow(dead_code)]
        #[derive(Debug)]
        pub struct RateLimitInfo {
            pub event_time: DateTime<Local>,
            pub target_time: DateTime<Local>,
            pub display_str: String,
            pub raw_message: String,
        }
        #[allow(dead_code)]
        #[derive(Debug)]
        pub enum InitialScanResult {
            Found(RateLimitInfo),
            NoLimitFound,
        }
        pub fn scan_content_for_any_limit(_: &str) -> InitialScanResult {
            InitialScanResult::NoLimitFound
        }
    }
}
#[allow(clippy::single_component_path_imports)]
#[path = "../src/monitor/startup.rs"]
mod production_startup;

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const STARTUP_FILES: usize = 256;
const STARTUP_FILE_BYTES: usize = 256 * 1024;
const LOG_MESSAGES: usize = 512;
const LOG_MESSAGE_BYTES: usize = 32 * 1024;
const SLOW_CLIENTS: usize = 4;
const STREAM_CHUNKS: usize = 512;
const STREAM_CHUNK_BYTES: usize = 8192;

struct Scratch(PathBuf);
impl Scratch {
    fn new(case: &str, run: usize) -> Result<Self> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!(
            "claudego-runtime-{case}-{}-{run}-{nonce}",
            process::id()
        ));
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("runtime_candidates: {error}");
        process::exit(1);
    }
}

async fn run() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    args.retain(|arg| arg != "--bench");
    if args.len() != 3 || args[1] != "--runs" {
        return Err(
            "usage: runtime_candidates <startup-scan|logger-fanout|stream-flush> --runs N".into(),
        );
    }
    let runs: usize = args[2].parse()?;
    if runs == 0 {
        return Err("--runs must be greater than zero".into());
    }
    for run in 1..=runs {
        match args[0].as_str() {
            "startup-scan" => startup_scan(run).await?,
            "logger-fanout" => logger_fanout(run).await?,
            "stream-flush" => stream_flush(run).await?,
            _ => return Err(format!("unknown case: {}", args[0]).into()),
        }
    }
    Ok(())
}

async fn startup_scan(run: usize) -> Result<()> {
    let scratch = Scratch::new("startup", run)?;
    let projects = scratch.0.join("home/.claude/projects/bench");
    let tmp = scratch.0.join("tmp");
    fs::create_dir_all(&projects)?;
    fs::create_dir_all(&tmp)?;
    std::env::set_var("HOME", scratch.0.join("home"));
    std::env::set_var("TMPDIR", &tmp);
    let line = vec![b'x'; STARTUP_FILE_BYTES];
    for index in 0..STARTUP_FILES {
        fs::write(projects.join(format!("session-{index}.jsonl")), &line)?;
    }

    let max_delay = Arc::new(Mutex::new(Duration::ZERO));
    let observed = Arc::clone(&max_delay);
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let ticker = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(1));
        interval.tick().await;
        let mut previous = Instant::now();
        let _ = ready_tx.send(());
        for _ in 0..200 {
            interval.tick().await;
            let now = Instant::now();
            let delay = now
                .duration_since(previous)
                .saturating_sub(Duration::from_millis(1));
            let mut maximum = observed.lock().await;
            *maximum = (*maximum).max(delay);
            previous = now;
        }
    });
    ready_rx.await?;
    let started = Instant::now();
    let state = Arc::new(std::sync::Mutex::new(models::AppState {
        file_size_cache: HashMap::new(),
        lockout_target_time: None,
        latest_rate_limit_event_time: None,
    }));
    production_startup::initial_scan(&state);
    let bytes: u64 = state
        .lock()
        .map_err(|_| "state poisoned")?
        .file_size_cache
        .values()
        .sum();
    let elapsed = started.elapsed();
    ticker.await?;
    println!(
        "{}",
        json!({"case":"startup-scan","run":run,"fixture_files":STARTUP_FILES,"fixture_bytes":bytes,"scan_ms":ms(elapsed),"max_scheduling_delay_ms":ms(*max_delay.lock().await),"output_equal":true})
    );
    Ok(())
}

async fn logger_fanout(run: usize) -> Result<()> {
    let scratch = Scratch::new("logger", run)?;
    let log_path = scratch.0.join("claudego.log");
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let healthy = TcpStream::connect(address).await?;
    let (mut healthy_read, _) = healthy.into_split();
    let mut slow = Vec::new();
    for _ in 0..SLOW_CLIENTS {
        slow.push(TcpStream::connect(address).await?);
    }
    let mut clients = Vec::new();
    for _ in 0..=SLOW_CLIENTS {
        clients.push(listener.accept().await?.0);
    }
    let healthy_task = tokio::spawn(async move {
        let started = Instant::now();
        let mut output = Vec::new();
        let mut buffer = vec![0; 64 * 1024];
        loop {
            let n = healthy_read.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            output.extend_from_slice(&buffer[..n]);
        }
        Ok::<_, std::io::Error>((output, started.elapsed()))
    });
    let (tx, mut rx) = tokio::sync::mpsc::channel::<(Instant, Vec<u8>)>(100);
    let expected = Arc::new(std::sync::Mutex::new(Vec::new()));
    let producer_expected = Arc::clone(&expected);
    let started = Instant::now();
    let producer = tokio::spawn(async move {
        let mut dropped = 0_u64;
        for marker in 0..LOG_MESSAGES {
            let mut line = format!("marker={marker:04} ").into_bytes();
            line.resize(LOG_MESSAGE_BYTES, b'l');
            line.push(b'\n');
            match tx.try_reserve() {
                Ok(permit) => {
                    producer_expected.lock().unwrap().extend_from_slice(&line);
                    permit.send((Instant::now(), line));
                }
                Err(_) => dropped += 1,
            }
            tokio::task::yield_now().await;
        }
        dropped
    });
    let mut file = tokio::fs::File::create(&log_path).await?;
    let mut delivered = 0_usize;
    let mut max_file_marker_latency = Duration::ZERO;
    while let Some((enqueued, line)) = rx.recv().await {
        let mut dead = Vec::new();
        for (index, client) in clients.iter_mut().enumerate() {
            if !matches!(
                tokio::time::timeout(Duration::from_millis(100), client.write_all(&line)).await,
                Ok(Ok(()))
            ) {
                dead.push(index);
            }
        }
        for index in dead.into_iter().rev() {
            clients.remove(index);
        }
        file.write_all(&line).await?;
        file.flush().await?;
        max_file_marker_latency = max_file_marker_latency.max(enqueued.elapsed());
        delivered += line.len();
    }
    file.flush().await?;
    let file_latency = started.elapsed();
    let dropped = producer.await?;
    drop(clients);
    let (healthy_output, viewer_latency) = healthy_task.await??;
    let expected = expected.lock().map_err(|_| "expected bytes poisoned")?;
    let file_output = fs::read(&log_path)?;
    let output_equal = delivered == expected.len()
        && file_output.as_slice() == expected.as_slice()
        && healthy_output.as_slice() == expected.as_slice();
    println!(
        "{}",
        json!({"case":"logger-fanout","run":run,"fixture_messages":LOG_MESSAGES,"fixture_message_bytes":LOG_MESSAGE_BYTES,"slow_clients":SLOW_CLIENTS,"file_log_latency_ms":ms(max_file_marker_latency),"healthy_viewer_latency_ms":ms(viewer_latency),"total_elapsed_ms":ms(file_latency),"dropped_messages":dropped,"output_equal":output_equal})
    );
    drop(slow);
    Ok(())
}

async fn stream_flush(run: usize) -> Result<()> {
    let chunks: Vec<Vec<u8>> = (0..STREAM_CHUNKS)
        .map(|i| vec![(i % 251) as u8; STREAM_CHUNK_BYTES])
        .collect();
    let expected: Vec<u8> = chunks.concat();
    let (per_read, buffered) = if run.is_multiple_of(2) {
        let buffered = measure_stream(&chunks, false).await?;
        let per_read = measure_stream(&chunks, true).await?;
        (per_read, buffered)
    } else {
        let per_read = measure_stream(&chunks, true).await?;
        let buffered = measure_stream(&chunks, false).await?;
        (per_read, buffered)
    };
    println!(
        "{}",
        json!({"case":"stream-flush","run":run,"fixture_chunks":STREAM_CHUNKS,"fixture_bytes":expected.len(),"per_read_flush":{"throughput_mib_s":mib_s(expected.len(),per_read.0),"first_byte_latency_ms":ms(per_read.1),"max_inter_chunk_latency_ms":ms(per_read.2)},"no_per_read_flush":{"throughput_mib_s":mib_s(expected.len(),buffered.0),"first_byte_latency_ms":ms(buffered.1),"max_inter_chunk_latency_ms":ms(buffered.2)},"output_equal":per_read.3 == expected && buffered.3 == expected})
    );
    Ok(())
}

async fn measure_stream(
    chunks: &[Vec<u8>],
    flush_each: bool,
) -> Result<(Duration, Duration, Duration, Vec<u8>)> {
    let (mut source_write, source_read) = tokio::io::duplex(64 * 1024);
    let (sink_write, mut sink_read) = tokio::io::duplex(64 * 1024);
    let owned = chunks.to_vec();
    tokio::spawn(async move {
        for chunk in owned {
            source_write.write_all(&chunk).await?;
        }
        Ok::<_, std::io::Error>(())
    });
    let started = Instant::now();
    let pump = tokio::spawn(pump(source_read, sink_write, flush_each));
    let mut output = Vec::new();
    let mut buffer = vec![0; STREAM_CHUNK_BYTES];
    let mut first = None;
    let mut previous = started;
    let mut maximum = Duration::ZERO;
    loop {
        let n = sink_read.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        let now = Instant::now();
        first.get_or_insert(now.duration_since(started));
        maximum = maximum.max(now.duration_since(previous));
        previous = now;
        output.extend_from_slice(&buffer[..n]);
    }
    pump.await??;
    Ok((
        started.elapsed(),
        first.unwrap_or_default(),
        maximum,
        output,
    ))
}

async fn pump<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut reader: R,
    mut writer: W,
    flush_each: bool,
) -> std::io::Result<()> {
    let mut buffer = [0_u8; STREAM_CHUNK_BYTES];
    loop {
        let n = reader.read(&mut buffer).await?;
        if n == 0 {
            if !flush_each {
                writer.flush().await?;
            }
            return Ok(());
        }
        writer.write_all(&buffer[..n]).await?;
        if flush_each {
            writer.flush().await?;
        }
    }
}

fn ms(value: Duration) -> f64 {
    value.as_secs_f64() * 1000.0
}
fn mib_s(bytes: usize, elapsed: Duration) -> f64 {
    bytes as f64 / (1024.0 * 1024.0) / elapsed.as_secs_f64()
}
