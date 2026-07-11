//! Measures `claudego`'s interactive PTY hot path on the current macOS host.
//!
//! Run with `cargo bench --bench pty_hot_path`. The benchmark reports three
//! runs each for a direct flood-child control, `claudego` with an idle monitor,
//! and `claudego` while active output defers a watcher scan. Every run validates
//! the deterministic byte stream before accepting its performance results.

use portable_pty::{Child, CommandBuilder, ExitStatus, NativePtySystem, PtySize, PtySystem};
use std::error::Error;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type BenchResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

const RUNS_PER_SCENARIO: usize = 3;
const CHUNK_SIZE: usize = 64 * 1024;
const OUTER_READ_BUFFER_SIZE: usize = 64 * 1024;
const FLOOD_DURATION: Duration = Duration::from_secs(8);
const RUN_TIMEOUT: Duration = Duration::from_secs(30);
const STALL_THRESHOLD: Duration = Duration::from_millis(100);
const DEFERRED_GATE_DELAY: Duration = Duration::from_secs(4);
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const WATCHER_READY_LOG: &str =
    "[System] Event-driven file watcher active. Blocking until events arrive.";
const SCAN_DEFERRED_LOG: &str = "Claude is currently streaming output. Deferring file scan";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scenario {
    Direct,
    MonitorIdle,
    ScanDeferred,
}

impl Scenario {
    const ALL: [Self; 3] = [Self::Direct, Self::MonitorIdle, Self::ScanDeferred];

    fn label(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::MonitorIdle => "monitor-idle",
            Self::ScanDeferred => "scan-deferred",
        }
    }

    fn uses_monitor(self) -> bool {
        self != Self::Direct
    }
}

#[derive(Debug)]
struct RunMetrics {
    verified_bytes: u64,
    elapsed: Duration,
    mib_per_second: f64,
    p99_gap: Duration,
    max_gap: Duration,
    stalls: usize,
}

struct ScenarioResults {
    scenario: Scenario,
    runs: Vec<RunMetrics>,
}

struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    fn new() -> BenchResult<Self> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path =
            std::env::temp_dir().join(format!("claudego-pty-hot-path-{}-{nonce}", process::id()));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct LoggerPaths {
    tmp_dir: PathBuf,
    log: PathBuf,
    port: PathBuf,
}

fn logger_paths(run_dir: &Path) -> LoggerPaths {
    let tmp_dir = run_dir.join("tmp");
    let log = tmp_dir.join("claudego.log");
    let port = tmp_dir.join("claudego.port");
    LoggerPaths { tmp_dir, log, port }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("pty_hot_path: {error}");
        process::exit(1);
    }
}

fn run() -> BenchResult<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .first()
        .is_some_and(|argument| argument == "--flood-child")
    {
        return flood_child(&args[1..]);
    }
    if !args.is_empty() && args != ["--bench"] {
        return Err(failure(format!("unexpected benchmark arguments: {args:?}")));
    }
    if !cfg!(target_os = "macos") {
        return Err(failure(
            "pty_hot_path performance results are validated only on macOS",
        ));
    }

    helper_self_check()?;
    ensure_no_active_claudego()?;

    let scratch = ScratchDir::new()?;
    let benchmark_exe = std::env::current_exe()?;
    let claudego_exe = PathBuf::from(env!("CARGO_BIN_EXE_claudego"));
    let mut all_results = Vec::new();

    println!(
        "Running {RUNS_PER_SCENARIO} serial runs per scenario; do not run another claudego process."
    );

    for scenario in Scenario::ALL {
        let mut runs = Vec::with_capacity(RUNS_PER_SCENARIO);
        for run_number in 1..=RUNS_PER_SCENARIO {
            let run_dir = scratch
                .path
                .join(format!("{}-{run_number}", scenario.label()));
            fs::create_dir(&run_dir)?;
            let metrics = run_scenario(scenario, &claudego_exe, &benchmark_exe, &run_dir).map_err(
                |error| {
                    failure(format!(
                        "{} run {run_number} failed: {error}",
                        scenario.label()
                    ))
                },
            )?;
            print_run(scenario, run_number, &metrics);
            runs.push(metrics);
        }
        all_results.push(ScenarioResults { scenario, runs });
    }

    print_summary(&all_results);
    Ok(())
}

fn helper_self_check() -> BenchResult<()> {
    let pattern = payload_pattern();
    if pattern.len() != CHUNK_SIZE {
        return Err(failure("payload pattern has the wrong size"));
    }

    let wrapped = [&pattern[CHUNK_SIZE - 2..], &pattern[..2]].concat();
    validate_bytes(&wrapped, (CHUNK_SIZE - 2) as u64, &pattern)?;
    if validate_bytes(b"!", 1, &pattern).is_ok() {
        return Err(failure("pattern mismatch self-check was not detected"));
    }

    let mut gaps = [
        Duration::from_millis(1),
        Duration::from_millis(2),
        Duration::from_millis(100),
    ];
    if nearest_rank_p99(&mut gaps) != Duration::from_millis(100) {
        return Err(failure("p99 self-check failed"));
    }

    let first = logger_paths(Path::new("first-run"));
    let second = logger_paths(Path::new("second-run"));
    for paths in [&first, &second] {
        if paths.log.parent() != Some(paths.tmp_dir.as_path())
            || paths.port.parent() != Some(paths.tmp_dir.as_path())
        {
            return Err(failure("logger paths are not children of run-local tmp"));
        }
    }
    if first.log == second.log || first.port == second.port {
        return Err(failure("different runs share logger paths"));
    }
    let global_log = global_log_path();
    let global_port = global_port_path();
    if first.log == global_log
        || second.log == global_log
        || first.port == global_port
        || second.port == global_port
    {
        return Err(failure(
            "run-local logger paths equal global preflight paths",
        ));
    }
    Ok(())
}

fn flood_child(args: &[String]) -> BenchResult<()> {
    if !(1..=2).contains(&args.len()) {
        return Err(failure(
            "usage: pty_hot_path --flood-child <count-file> [gate-file]",
        ));
    }

    let count_path = Path::new(&args[0]);
    if let Some(gate_path) = args.get(1).map(Path::new) {
        while !gate_path.exists() {
            thread::sleep(POLL_INTERVAL);
        }
    }

    let chunk = payload_pattern();
    let started = Instant::now();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let mut written = 0_u64;

    while started.elapsed() < FLOOD_DURATION {
        stdout.write_all(&chunk)?;
        written += chunk.len() as u64;
    }
    stdout.flush()?;

    // The count travels through a sidecar so metadata never contaminates the measured PTY stream.
    fs::write(count_path, written.to_string())?;
    Ok(())
}

fn payload_pattern() -> Vec<u8> {
    // Printable ASCII excludes newlines and control bytes that PTY terminal modes may translate.
    (0..CHUNK_SIZE)
        .map(|index| b'!' + (index % 94) as u8)
        .collect()
}

fn run_scenario(
    scenario: Scenario,
    claudego_exe: &Path,
    benchmark_exe: &Path,
    run_dir: &Path,
) -> BenchResult<RunMetrics> {
    let count_path = run_dir.join("expected-count");
    let gate_path = run_dir.join("flood-gate");
    let logger_paths = logger_paths(run_dir);
    let session_path = if scenario.uses_monitor() {
        fs::create_dir(&logger_paths.tmp_dir).map_err(|error| {
            failure(format!(
                "create run-local logger directory {}: {error}",
                logger_paths.tmp_dir.display()
            ))
        })?;
        Some(prepare_monitor_home(run_dir)?)
    } else {
        None
    };

    let command = if scenario == Scenario::Direct {
        let mut command = CommandBuilder::new(benchmark_exe);
        command.args([
            "--flood-child",
            count_path
                .to_str()
                .ok_or_else(|| failure("non-UTF-8 count path"))?,
        ]);
        command
    } else {
        let mut command = CommandBuilder::new(claudego_exe);
        command.args([
            "--",
            benchmark_exe
                .to_str()
                .ok_or_else(|| failure("non-UTF-8 benchmark path"))?,
            "--flood-child",
            count_path
                .to_str()
                .ok_or_else(|| failure("non-UTF-8 count path"))?,
            gate_path
                .to_str()
                .ok_or_else(|| failure("non-UTF-8 gate path"))?,
        ]);
        let home = run_dir.join("home");
        command.env("HOME", &home);
        command.env("TMPDIR", &logger_paths.tmp_dir);
        command
    };

    let pty_system = NativePtySystem::default();
    // The outer PTY makes claudego face a terminal without measuring terminal rendering.
    let pair = pty_system.openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let reader = pair.master.try_clone_reader()?;
    let deadline = Instant::now() + RUN_TIMEOUT;
    let mut child = pair.slave.spawn_command(command)?;
    drop(pair.slave);
    let output_rx = spawn_output_reader(reader);

    let outcome = finish_scenario(
        scenario,
        child.as_mut(),
        output_rx,
        session_path.as_deref(),
        &gate_path,
        &count_path,
        &logger_paths,
        deadline,
    );
    if outcome.is_err() {
        terminate_child(child.as_mut());
    }
    outcome
}

fn finish_scenario(
    scenario: Scenario,
    child: &mut dyn Child,
    output_rx: Receiver<BenchResult<RunMetrics>>,
    session_path: Option<&Path>,
    gate_path: &Path,
    count_path: &Path,
    logger_paths: &LoggerPaths,
    deadline: Instant,
) -> BenchResult<RunMetrics> {
    if scenario.uses_monitor() {
        let log_rx = start_log_stream(&logger_paths.port, deadline)?;
        wait_for_log(&log_rx, WATCHER_READY_LOG, deadline)?;

        if scenario == Scenario::ScanDeferred {
            let event_at = Instant::now();
            let session_path =
                session_path.ok_or_else(|| failure("missing deferred scenario session path"))?;
            let mut session = File::create(session_path)?;
            session.write_all(
                b"{\"type\":\"benchmark\",\"message\":\"synthetic non-limit event\"}\n",
            )?;
            session.flush()?;

            // Releasing at t+4s makes output hot when the real 5s debounce expires at t+5s.
            let release_at = event_at + DEFERRED_GATE_DELAY;
            sleep_until(release_at, deadline)?;
        }
        drop(File::create(gate_path)?);
    }

    let metrics = receive_output(output_rx, deadline)?;
    let status = wait_for_child(child, deadline)?;
    if !status.success() {
        return Err(failure(format!("child exited unsuccessfully: {status:?}")));
    }

    verify_expected_count(count_path, metrics.verified_bytes)?;

    if scenario.uses_monitor() {
        let logs = fs::read_to_string(&logger_paths.log).map_err(|error| {
            failure(format!(
                "read claudego log {}: {error}",
                logger_paths.log.display()
            ))
        })?;
        if !logs.contains(WATCHER_READY_LOG) {
            return Err(failure("persistent log is missing watcher readiness"));
        }
        if scenario == Scenario::ScanDeferred && !logs.contains(SCAN_DEFERRED_LOG) {
            return Err(failure("persistent log is missing scan deferral"));
        }
        if scenario == Scenario::MonitorIdle && logs.contains(SCAN_DEFERRED_LOG) {
            return Err(failure("idle monitor unexpectedly deferred a scan"));
        }
    }

    Ok(metrics)
}

fn prepare_monitor_home(run_dir: &Path) -> BenchResult<PathBuf> {
    let session_path = run_dir
        .join("home")
        .join(".claude/projects/benchmark/session.jsonl");
    let parent = session_path
        .parent()
        .ok_or_else(|| failure("session path has no parent"))?;
    fs::create_dir_all(parent)?;
    Ok(session_path)
}

fn spawn_output_reader(reader: Box<dyn Read + Send>) -> Receiver<BenchResult<RunMetrics>> {
    let (tx, rx) = mpsc::sync_channel(1);
    let _reader_thread = thread::spawn(move || {
        let _ = tx.send(read_output(reader));
    });
    rx
}

fn read_output(mut reader: Box<dyn Read + Send>) -> BenchResult<RunMetrics> {
    let pattern = payload_pattern();
    let mut buffer = vec![0_u8; OUTER_READ_BUFFER_SIZE];
    let mut verified_bytes = 0_u64;
    let mut first_read = None;
    let mut previous_read = None;
    let mut final_read = None;
    let mut gaps = Vec::new();

    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            // Some PTY masters signal final closure as an I/O error; the sidecar count still makes truncation fatal.
            Err(_) if verified_bytes > 0 => break,
            Err(error) => return Err(error.into()),
        };

        // Stall timestamps are completions of consecutive non-empty reads from the outer PTY.
        let read_at = Instant::now();
        if let Some(previous) = previous_read {
            gaps.push(read_at.duration_since(previous));
        } else {
            first_read = Some(read_at);
        }
        previous_read = Some(read_at);
        final_read = Some(read_at);

        validate_bytes(&buffer[..read], verified_bytes, &pattern)?;
        verified_bytes += read as u64;
    }

    let first_read = first_read.ok_or_else(|| failure("received no payload bytes"))?;
    let final_read = final_read.ok_or_else(|| failure("received no final payload byte"))?;
    let elapsed = final_read.duration_since(first_read);
    if elapsed.is_zero() {
        return Err(failure("measured output interval is zero"));
    }

    let stalls = gaps.iter().filter(|gap| **gap >= STALL_THRESHOLD).count();
    let max_gap = gaps.iter().copied().max().unwrap_or(Duration::ZERO);
    let p99_gap = nearest_rank_p99(&mut gaps);
    let mib_per_second = verified_bytes as f64 / (1024.0 * 1024.0) / elapsed.as_secs_f64();

    Ok(RunMetrics {
        verified_bytes,
        elapsed,
        mib_per_second,
        p99_gap,
        max_gap,
        stalls,
    })
}

fn validate_bytes(bytes: &[u8], stream_offset: u64, pattern: &[u8]) -> BenchResult<()> {
    let mut checked = 0;
    while checked < bytes.len() {
        let pattern_offset = (stream_offset as usize + checked) % pattern.len();
        let length = (pattern.len() - pattern_offset).min(bytes.len() - checked);
        let actual = &bytes[checked..checked + length];
        let expected = &pattern[pattern_offset..pattern_offset + length];
        if actual != expected {
            let mismatch = actual
                .iter()
                .zip(expected)
                .position(|(actual, expected)| actual != expected)
                .expect("different slices contain a mismatching byte");
            return Err(failure(format!(
                "pattern mismatch at byte {}: expected {}, received {}",
                stream_offset + checked as u64 + mismatch as u64,
                expected[mismatch],
                actual[mismatch]
            )));
        }
        checked += length;
    }
    Ok(())
}

fn nearest_rank_p99(gaps: &mut [Duration]) -> Duration {
    if gaps.is_empty() {
        return Duration::ZERO;
    }
    gaps.sort_unstable();
    let rank = (gaps.len() * 99).div_ceil(100);
    gaps[rank - 1]
}

fn receive_output(
    output_rx: Receiver<BenchResult<RunMetrics>>,
    deadline: Instant,
) -> BenchResult<RunMetrics> {
    match output_rx.recv_timeout(time_left(deadline)?) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(failure("30-second run timeout")),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(failure("outer PTY reader disconnected")),
    }
}

fn wait_for_child(child: &mut dyn Child, deadline: Instant) -> BenchResult<ExitStatus> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            return Err(failure("30-second run timeout"));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn terminate_child(child: &mut dyn Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn sleep_until(target: Instant, deadline: Instant) -> BenchResult<()> {
    if target >= deadline {
        return Err(failure("30-second run timeout before gate release"));
    }
    thread::sleep(target.saturating_duration_since(Instant::now()));
    Ok(())
}

fn verify_expected_count(count_path: &Path, verified_bytes: u64) -> BenchResult<()> {
    let expected = fs::read_to_string(count_path)
        .map_err(|error| failure(format!("read expected-count sidecar: {error}")))?
        .trim()
        .parse::<u64>()
        .map_err(|error| failure(format!("parse expected-count sidecar: {error}")))?;
    if expected != verified_bytes {
        return Err(failure(format!(
            "count mismatch: child wrote {expected} bytes, driver verified {verified_bytes}"
        )));
    }
    Ok(())
}

fn ensure_no_active_claudego() -> BenchResult<()> {
    let global_port = global_port_path();
    if let Some(address) = logger_address(&global_port) {
        if TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
            return Err(failure(
                "another claudego logger is active; stop it before benchmarking",
            ));
        }
    }
    reset_global_log_files();
    Ok(())
}

fn reset_global_log_files() {
    let _ = fs::remove_file(global_log_path());
    let _ = fs::remove_file(global_port_path());
}

fn global_log_path() -> PathBuf {
    std::env::temp_dir().join("claudego.log")
}

fn global_port_path() -> PathBuf {
    std::env::temp_dir().join("claudego.port")
}

fn logger_address(port_path: &Path) -> Option<SocketAddr> {
    let port = fs::read_to_string(port_path).ok()?;
    format!("127.0.0.1:{}", port.trim()).parse().ok()
}

fn start_log_stream(port_path: &Path, deadline: Instant) -> BenchResult<Receiver<String>> {
    loop {
        if let Some(address) = logger_address(port_path) {
            if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_millis(100)) {
                let (tx, rx) = mpsc::channel();
                let _log_thread = thread::spawn(move || {
                    for line in BufReader::new(stream).lines() {
                        let Ok(line) = line else {
                            break;
                        };
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                });
                return Ok(rx);
            }
        }
        if Instant::now() >= deadline {
            return Err(failure(format!(
                "logger did not become reachable via {}",
                port_path.display()
            )));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn wait_for_log(log_rx: &Receiver<String>, needle: &str, deadline: Instant) -> BenchResult<()> {
    loop {
        match log_rx.recv_timeout(time_left(deadline)?) {
            Ok(line) if line.contains(needle) => return Ok(()),
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err(failure(format!("missing log evidence: {needle}")));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(failure("logger stream disconnected before readiness"));
            }
        }
    }
}

fn time_left(deadline: Instant) -> BenchResult<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        Err(failure("30-second run timeout"))
    } else {
        Ok(remaining)
    }
}

fn print_run(scenario: Scenario, run_number: usize, metrics: &RunMetrics) {
    println!(
        "{} run {run_number}: verified_bytes={} elapsed={:.3}s throughput={:.2} MiB/s p99_gap={:.3}ms max_gap={:.3}ms stalls>=100ms={}",
        scenario.label(),
        metrics.verified_bytes,
        metrics.elapsed.as_secs_f64(),
        metrics.mib_per_second,
        milliseconds(metrics.p99_gap),
        milliseconds(metrics.max_gap),
        metrics.stalls,
    );
}

fn print_summary(results: &[ScenarioResults]) {
    println!("Medians:");
    for result in results {
        println!(
            "{} median throughput: {:.2} MiB/s",
            result.scenario.label(),
            median_throughput(&result.runs)
        );
    }

    let direct = scenario_results(results, Scenario::Direct);
    let idle = scenario_results(results, Scenario::MonitorIdle);
    let deferred = scenario_results(results, Scenario::ScanDeferred);
    let direct_median = median_throughput(&direct.runs);
    let idle_median = median_throughput(&idle.runs);
    let deferred_median = median_throughput(&deferred.runs);
    let direct_stalls = has_stalls(&direct.runs);
    let idle_stalls = has_stalls(&idle.runs);
    let deferred_stalls = has_stalls(&deferred.runs);

    let wrapper_target = idle_median <= direct_median * 0.95 || (!direct_stalls && idle_stalls);
    let monitor_competes =
        deferred_median <= idle_median * 0.95 || (!idle_stalls && deferred_stalls);

    if wrapper_target {
        println!(
            "PTY decision: TUNE the current bridge first by locking stdout once and enlarging its read buffer."
        );
    } else {
        println!(
            "PTY decision: KEEP the current bridge; wrapper overhead is inside the five-percent boundary with no wrapper-only stalls."
        );
    }

    if monitor_competes {
        println!(
            "Monitor decision: TUNE monitor scheduling; deferred work crosses the five-percent or new-stall boundary."
        );
    } else {
        println!(
            "Monitor decision: KEEP current deferral behavior; deferred work adds no measured boundary violation."
        );
    }

    println!(
        "Upgrade/replace decision: NOT JUSTIFIED; reconsider portable-pty only after the ordered local optimization and a full rerun still isolate bridge cost."
    );
}

fn scenario_results(results: &[ScenarioResults], scenario: Scenario) -> &ScenarioResults {
    results
        .iter()
        .find(|result| result.scenario == scenario)
        .expect("all scenarios are always recorded")
}

fn median_throughput(runs: &[RunMetrics]) -> f64 {
    let mut values: Vec<f64> = runs.iter().map(|run| run.mib_per_second).collect();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn has_stalls(runs: &[RunMetrics]) -> bool {
    runs.iter().any(|run| run.stalls > 0)
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn failure(message: impl Into<String>) -> Box<dyn Error + Send + Sync> {
    io::Error::other(message.into()).into()
}
