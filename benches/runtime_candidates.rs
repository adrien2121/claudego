//! Raw, dependency-free measurements for runtime-hardening candidates.

use serde_json::json;
use std::error::Error;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const STARTUP_FILES: usize = 256;
const STARTUP_FILE_BYTES: usize = 256 * 1024;
const LOG_MESSAGES: usize = 16;
const LOG_MESSAGE_BYTES: usize = 1024 * 1024;
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

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
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
    fs::create_dir_all(&projects)?;
    let line = vec![b'x'; STARTUP_FILE_BYTES];
    for index in 0..STARTUP_FILES {
        fs::write(projects.join(format!("session-{index}.jsonl")), &line)?;
    }

    let max_delay = Arc::new(Mutex::new(Duration::ZERO));
    let observed = Arc::clone(&max_delay);
    let ticker = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(1));
        interval.tick().await;
        let mut previous = Instant::now();
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
    tokio::task::yield_now().await;
    let started = Instant::now();
    let bytes = synchronous_initial_scan(&projects)?;
    let elapsed = started.elapsed();
    ticker.await?;
    println!(
        "{}",
        json!({"case":"startup-scan","run":run,"fixture_files":STARTUP_FILES,"fixture_bytes":bytes,"scan_ms":ms(elapsed),"max_scheduling_delay_ms":ms(*max_delay.lock().await),"output_equal":true})
    );
    Ok(())
}

fn synchronous_initial_scan(root: &Path) -> Result<u64> {
    let mut total = 0;
    let mut buffer = vec![0_u8; 64 * 1024];
    for entry in fs::read_dir(root)? {
        let mut file = File::open(entry?.path())?;
        let size = file.metadata()?.len();
        total += size;
        let mut position = size;
        while position > 0 {
            let start = position.saturating_sub(buffer.len() as u64);
            file.seek(SeekFrom::Start(start))?;
            file.read_exact(&mut buffer[..(position - start) as usize])?;
            position = start;
        }
    }
    Ok(total)
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
    let expected = LOG_MESSAGES * (LOG_MESSAGE_BYTES + 1);
    let healthy_task = tokio::spawn(async move {
        let started = Instant::now();
        let mut bytes = 0;
        let mut buffer = vec![0; 64 * 1024];
        while bytes < expected {
            bytes += healthy_read.read(&mut buffer).await?;
        }
        Ok::<_, std::io::Error>((bytes, started.elapsed()))
    });
    let payload = vec![b'l'; LOG_MESSAGE_BYTES];
    let mut line = payload.clone();
    line.push(b'\n');
    let started = Instant::now();
    let mut file = tokio::fs::File::create(&log_path).await?;
    let mut dropped = 0_u64;
    for _ in 0..LOG_MESSAGES {
        for client in &mut clients {
            if tokio::time::timeout(Duration::from_millis(100), client.write_all(&line))
                .await
                .is_err()
            {
                dropped += 1;
            }
        }
        file.write_all(&payload).await?;
        file.write_all(b"\n").await?;
    }
    file.flush().await?;
    let file_latency = started.elapsed();
    drop(clients);
    let (healthy_bytes, viewer_latency) = healthy_task.await??;
    let output_equal =
        fs::metadata(&log_path)?.len() as usize == expected && healthy_bytes == expected;
    println!(
        "{}",
        json!({"case":"logger-fanout","run":run,"fixture_messages":LOG_MESSAGES,"fixture_message_bytes":LOG_MESSAGE_BYTES,"slow_clients":SLOW_CLIENTS,"file_log_latency_ms":ms(file_latency),"healthy_viewer_latency_ms":ms(viewer_latency),"dropped_messages":dropped,"output_equal":output_equal})
    );
    drop(slow);
    Ok(())
}

async fn stream_flush(run: usize) -> Result<()> {
    let chunks: Vec<Vec<u8>> = (0..STREAM_CHUNKS)
        .map(|i| vec![(i % 251) as u8; STREAM_CHUNK_BYTES])
        .collect();
    let expected: Vec<u8> = chunks.concat();
    let per_read = measure_stream(&chunks, true).await?;
    let buffered = measure_stream(&chunks, false).await?;
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
