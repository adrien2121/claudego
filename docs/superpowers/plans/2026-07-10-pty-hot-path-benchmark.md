# PTY Hot-Path Benchmark Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a repeatable macOS benchmark that verifies `claudego` PTY output byte-for-byte, measures throughput and stalls with the monitor idle and deferring a scan, and produces enough evidence to keep or tune the current bridge.

**Architecture:** Add one dependency-free, `harness = false` Cargo benchmark. The benchmark executable doubles as its deterministic flood child, while the driver captures each run through an outer `portable-pty`, validates bytes online, and uses the existing logger TCP stream plus isolated `HOME` directories to synchronize the production watcher. Keep production unchanged unless the baseline crosses the spec's decision boundary; then try only the ordered stdout-lock/read-buffer optimization and retain it only if a full rerun improves the measured problem.

**Tech Stack:** Rust 2021, Cargo custom benchmarks, Rust standard library, existing `portable-pty 0.8.1`, existing `notify 6.1.1` behavior exercised through the `claudego` binary.

## Global Constraints

- Validate performance only on the current macOS host; make no Linux or Windows performance claim.
- Run direct control, monitor idle, and scan deferred exactly three times each under Cargo's optimized benchmark profile.
- The direct control has one fewer PTY, so the comparison measures total wrapper cost rather than `portable-pty` cost in isolation.
- The flood child writes deterministic 64 KiB chunks of printable ASCII for eight seconds.
- Validate every received byte online and retain only byte counts and inter-read gaps, not the full output stream.
- The child writes its expected byte count to a sidecar only after payload output completes.
- Measure from the first non-empty outer-PTY read completion to the final non-empty outer-PTY read completion.
- Define an output stall as an inter-read gap of at least 100 ms; report p99, maximum, and stall count.
- Exclude startup, watcher preparation, debounce setup, and EOF wait from the measured output interval.
- Use an isolated `HOME/.claude/projects/benchmark/session.jsonl` for monitor runs; never scan real Claude history.
- Synchronize with the existing watcher-ready log and the real five-second debounce; do not add a production-only benchmark hook.
- The logger path and port file under `std::env::temp_dir()` are process-global. Run scenarios serially and fail before clobbering an active `claudego` logger.
- Fail a named run on pattern/count mismatch, child failure, missing watcher or deferral evidence, missing sidecar, or a 30-second timeout; kill a timed-out child.
- Treat a median throughput loss of at least five percent or a new 100 ms stall according to the spec's direct/idle/deferred decision rules.
- Performance is a host report, not a hardware-independent assertion; rerun when spread makes the five-percent boundary ambiguous.
- Keep per-read flush initially. Change it only after separate evidence proves throughput improves without delaying partial terminal output.
- No terminal-rendering benchmark, CI threshold, monitor bypass, benchmark framework dependency, speculative PTY rewrite, or `portable-pty` upgrade.
- Required final commands: `cargo bench --bench pty_hot_path`, `cargo fmt --check`, `cargo test`, and `cargo clippy --all-targets --all-features -- -D warnings`.
- Cargo officially sets `CARGO_BIN_EXE_<name>` for benchmark targets: <https://doc.rust-lang.org/cargo/reference/environment-variables.html>.
- Rust stdout exposes an explicit reusable lock: <https://doc.rust-lang.org/std/io/struct.Stdout.html>.
- `portable-pty` reads are blocking, so both production and benchmark readers stay on dedicated blocking threads: <https://github.com/wezterm/wezterm/discussions/3739>.
- The current checkout starts clean on all three quality gates: formatting passes, 34 library tests plus one PTY integration test pass, and clippy passes.
- The checkout already contains unrelated untracked files. Each commit below stages only the paths named in that task.

---

## File Structure

- Modify `Cargo.toml`: register the single custom benchmark with `harness = false`.
- Create `benches/pty_hot_path.rs`: own flood-child mode, scratch directories, outer-PTY execution, live watcher synchronization, online integrity checking, metrics, summaries, and decisions.
- Modify `src/pty_bridge.rs` only when the baseline proves the wrapper PTY path is an optimization target: lock stdout once and use a 64 KiB read buffer.
- Modify `BACKLOG.md`: close P0 with the exact host, command, run results, medians, and keep/tune decision observed during execution.

---

### Task 1: Dependency-Free PTY Benchmark

**Files:**
- Modify: `Cargo.toml:1-31`
- Create: `benches/pty_hot_path.rs`

**Interfaces:**
- Consumes:
  - Cargo-provided `env!("CARGO_BIN_EXE_claudego") -> &'static str`.
  - `portable_pty::{NativePtySystem, PtySystem, CommandBuilder, Child, ExitStatus}`.
  - Production log names `std::env::temp_dir().join("claudego.log")` and `std::env::temp_dir().join("claudego.port")`.
  - Production evidence strings `"[System] Event-driven file watcher active. Blocking until events arrive."` and `"Claude is currently streaming output. Deferring file scan"`.
- Produces:
  - Cargo target `pty_hot_path`, invoked by `cargo bench --bench pty_hot_path`.
  - Child protocol `pty_hot_path --flood-child <count-file> [gate-file]`.
  - `Scenario::{Direct, MonitorIdle, ScanDeferred}`.
  - `RunMetrics { verified_bytes: u64, elapsed: Duration, mib_per_second: f64, p99_gap: Duration, max_gap: Duration, stalls: usize }`.
  - Nine per-run result lines, three median lines, a PTY decision, a monitor-competition decision, and an upgrade/replace decision.

- [ ] **Step 1: Write the failing helper check and register the bench target**

Append this target to `Cargo.toml`:

```toml
[[bench]]
name = "pty_hot_path"
harness = false
```

Create `benches/pty_hot_path.rs` with the first executable check:

```rust
use std::time::Duration;

const CHUNK_SIZE: usize = 64 * 1024;

fn main() {
    let pattern = payload_pattern();
    assert_eq!(pattern.len(), CHUNK_SIZE);

    let mut gaps = [
        Duration::from_millis(1),
        Duration::from_millis(2),
        Duration::from_millis(100),
    ];
    assert_eq!(
        nearest_rank_p99(&mut gaps),
        Duration::from_millis(100)
    );
}
```

- [ ] **Step 2: Run the bench compile to verify the check fails**

Run:

```bash
cargo bench --bench pty_hot_path --no-run
```

Expected: FAIL with Rust `E0425` for missing `payload_pattern` and `nearest_rank_p99`.

- [ ] **Step 3: Implement the complete benchmark**

Replace `benches/pty_hot_path.rs` with:

```rust
//! Measures `claudego`'s interactive PTY hot path on the current macOS host.
//!
//! Run with `cargo bench --bench pty_hot_path`. The benchmark reports three
//! runs each for a direct flood-child control, `claudego` with an idle monitor,
//! and `claudego` while active output defers a watcher scan. Every run validates
//! the deterministic byte stream before accepting its performance results.

use portable_pty::{Child, CommandBuilder, ExitStatus, NativePtySystem, PtySize, PtySystem};
use std::error::Error;
use std::fs::{self, File, OpenOptions};
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
    if !args.is_empty() {
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
    let session_path = if scenario.uses_monitor() {
        Some(prepare_monitor_home(run_dir)?)
    } else {
        None
    };

    if scenario.uses_monitor() {
        reset_global_log_files();
    }

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
        command.env(
            "HOME",
            home.to_str()
                .ok_or_else(|| failure("non-UTF-8 benchmark HOME"))?,
        );
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
    deadline: Instant,
) -> BenchResult<RunMetrics> {
    if scenario.uses_monitor() {
        let log_rx = start_log_stream(deadline)?;
        wait_for_log(&log_rx, WATCHER_READY_LOG, deadline)?;

        if scenario == Scenario::ScanDeferred {
            let event_at = Instant::now();
            let session_path =
                session_path.ok_or_else(|| failure("missing deferred scenario session path"))?;
            let mut session = OpenOptions::new().append(true).open(session_path)?;
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
        let logs = fs::read_to_string(global_log_path())
            .map_err(|error| failure(format!("read claudego log: {error}")))?;
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
    fs::write(
        &session_path,
        b"{\"type\":\"benchmark\",\"message\":\"baseline\"}\n",
    )?;
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
    if let Some(address) = logger_address() {
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

fn logger_address() -> Option<SocketAddr> {
    let port = fs::read_to_string(global_port_path()).ok()?;
    format!("127.0.0.1:{}", port.trim()).parse().ok()
}

fn start_log_stream(deadline: Instant) -> BenchResult<Receiver<String>> {
    loop {
        if let Some(address) = logger_address() {
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
            return Err(failure("logger did not become reachable"));
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
```

- [ ] **Step 4: Compile the completed target**

Run:

```bash
cargo bench --bench pty_hot_path --no-run
```

Expected: PASS; Cargo builds the optimized `claudego` binary and the `pty_hot_path` executable without a new crate.

- [ ] **Step 5: Run the benchmark's executable correctness check**

Run:

```bash
cargo bench --bench pty_hot_path
```

Expected:

- Exit status 0.
- Exactly nine named run lines: three `direct`, three `monitor-idle`, and three `scan-deferred`.
- Every run reports a positive `verified_bytes` value, a roughly eight-second measured interval, throughput, p99/max gaps, and a `stalls>=100ms` count.
- Exactly three median-throughput lines.
- One PTY decision, one monitor decision, and `Upgrade/replace decision: NOT JUSTIFIED`.
- Each scan-deferred run reaches watcher readiness, triggers the real five-second debounce, and finds persistent scan-deferral evidence.
- Any timeout, missing sidecar/log evidence, child failure, pattern mismatch, or count mismatch names the failing scenario and exits nonzero instead of printing a valid result.

- [ ] **Step 6: Run the fast regression gates**

Run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: all commands PASS; the existing 34 library tests and `tests/pty_exit.rs` remain green, and clippy includes the new benchmark target.

- [ ] **Step 7: Commit the benchmark**

```bash
git add Cargo.toml benches/pty_hot_path.rs
git commit -m "bench: measure PTY hot path"
```

---

### Task 2: Evidence-Gated PTY Optimization

**Files:**
- Modify only if the Task 1 PTY decision is `TUNE`: `src/pty_bridge.rs:40-55`

**Interfaces:**
- Consumes:
  - Task 1's three baseline medians and all per-run stall counts.
  - `spawn_output_reader(reader: Box<dyn Read + Send>, state: SharedAppState) -> ()`.
- Produces:
  - No public API change.
  - If retained, one stdout lock for the reader thread's lifetime and a 64 KiB stack read buffer.
  - A complete post-change nine-run result set proving integrity and measuring the same scenarios.

- [ ] **Step 1: Apply the decision rule to the baseline**

Use the emitted `PTY decision` line as the gate:

- If it says `KEEP`, skip the remaining steps in this task. Do not touch `src/pty_bridge.rs`.
- If it says `TUNE`, treat the baseline boundary violation as the failing performance check and continue.
- A `TUNE` monitor decision by itself does not authorize a PTY bridge change; record it in Task 3 and leave monitor behavior unchanged until a separate measured design exists.
- If run-to-run spread makes the five-percent comparison ambiguous, rerun `cargo bench --bench pty_hot_path` before changing production code.

- [ ] **Step 2: Implement only the first ordered optimization**

In `src/pty_bridge.rs`, replace:

```rust
        let mut buf = [0u8; 1024];
        let mut stdout = io::stdout();
```

with:

```rust
        let mut buf = [0u8; 64 * 1024];
        let stdout = io::stdout();
        let mut stdout = stdout.lock();
```

Leave the activity update, `write_all`, and per-read `flush` exactly where they are.

- [ ] **Step 3: Format and rerun every scenario**

Run:

```bash
cargo fmt --check
cargo bench --bench pty_hot_path
```

Expected:

- Both commands PASS.
- All nine runs still pass pattern and sidecar-count integrity.
- Keep the change only when the repeated results improve the metric that triggered `TUNE` and introduce no integrity or stall regression.
- For a throughput trigger, `monitor-idle` median throughput must increase; for a stall trigger, wrapper-only stall count or maximum gap must decrease.
- If spread still straddles the five-percent boundary, rerun before deciding.

- [ ] **Step 4: Revert an unsupported optimization explicitly**

If Step 3 does not improve the triggering metric, replace:

```rust
        let mut buf = [0u8; 64 * 1024];
        let stdout = io::stdout();
        let mut stdout = stdout.lock();
```

with the original:

```rust
        let mut buf = [0u8; 1024];
        let mut stdout = io::stdout();
```

Then run:

```bash
cargo fmt --check
cargo test
```

Expected: both commands PASS. Record the baseline decision as `TUNE further` in Task 3; do not change flush behavior or upgrade/replace `portable-pty` in this plan.

- [ ] **Step 5: Verify a retained optimization**

If Step 3 supports the change, run:

```bash
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: both commands PASS with no new warning or regression.

- [ ] **Step 6: Commit only a retained optimization**

```bash
git add src/pty_bridge.rs
git commit -m "perf: reduce PTY output overhead"
```

If the optimization was skipped or reverted, do not create an empty commit.

---

### Task 3: Record P0 Evidence and Final Verification

**Files:**
- Modify: `BACKLOG.md:5-12`

**Interfaces:**
- Consumes:
  - The final accepted nine-run output and three medians from Task 1 or Task 2.
  - `sw_vers -productVersion`, `uname -m`, `rustc --version`, and `git rev-parse HEAD`.
- Produces:
  - A checked P0 list.
  - A dated evidence block containing the exact command, host, architecture, Rust version, code commit, all scenario results, medians, and final keep/tune/upgrade/replace decision.

- [ ] **Step 1: Capture the final accepted evidence**

Run the benchmark once more from a quiet machine with no other `claudego` process:

```bash
cargo bench --bench pty_hot_path
sw_vers -productVersion
uname -m
rustc --version
git rev-parse HEAD
```

Expected:

- Benchmark exit status 0 with the complete nine-run and summary output.
- `sw_vers` prints the macOS product version.
- `uname -m` prints `arm64` on the current host.
- `rustc --version` prints the exact active compiler.
- `git rev-parse HEAD` prints the commit containing the benchmark and any retained PTY optimization.

- [ ] **Step 2: Replace the P0 block with checked acceptance items and literal evidence**

In `BACKLOG.md`:

1. Change all four P0 checklist markers from `[ ]` to `[x]`.
2. Keep the existing `Done when` sentence.
3. Immediately after it, add a `### Evidence — 2026-07-10` section.
4. Add `Command: cargo bench --bench pty_hot_path`.
5. Add one host line containing the literal outputs of `sw_vers -productVersion`, `uname -m`, and `rustc --version`.
6. Add one commit line containing the full literal `git rev-parse HEAD` output.
7. Paste all nine accepted run lines and all three median lines verbatim in a fenced text block.
8. Paste the PTY, monitor, and upgrade/replace decision lines verbatim below the result block.
9. If Task 2 ran, include separate fenced `Baseline` and `After stdout lock + 64 KiB buffer` result blocks so the retained/reverted decision is auditable.
10. State that another `claudego` process must not run concurrently because the logger files are process-global.

Do not round again, recompute from copied text, omit a slow run, or write a Linux/Windows claim.

- [ ] **Step 3: Check the documentation mechanically**

Run:

```bash
rg -n '^- \[x\]' BACKLOG.md
rg -n 'cargo bench --bench pty_hot_path|macOS|arm64|rustc|commit|direct run|monitor-idle run|scan-deferred run|median throughput|PTY decision|Monitor decision|Upgrade/replace decision' BACKLOG.md
```

Expected:

- The first command prints exactly four checked P0 acceptance items.
- The second command finds the command, host/compiler/commit metadata, all three scenario names, medians, and all three decision lines.

- [ ] **Step 4: Run every required final command**

Run:

```bash
cargo bench --bench pty_hot_path
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: all four commands PASS. The benchmark again verifies all nine streams before reporting performance; formatting, all tests, and clippy remain clean.

- [ ] **Step 5: Review the final scope**

Run:

```bash
git status --short
git diff -- Cargo.toml benches/pty_hot_path.rs src/pty_bridge.rs BACKLOG.md
```

Expected:

- Required implementation changes are limited to `Cargo.toml`, `benches/pty_hot_path.rs`, `BACKLOG.md`, and optionally the two-line `src/pty_bridge.rs` optimization.
- No dependency, monitor bypass, production benchmark hook, flush-policy change, PTY rewrite, or unrelated file appears in the diff.
- Pre-existing untracked files remain unstaged unless they are one of the required paths above.

- [ ] **Step 6: Commit the evidence**

```bash
git add BACKLOG.md
git commit -m "docs: record PTY benchmark evidence"
```
