# PTY Benchmark Per-Run TMPDIR Isolation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every monitored PTY benchmark run own its logger directory so live and persistent watcher evidence cannot be replaced through process-global temporary paths.

**Architecture:** Keep production unchanged. Add one benchmark-local `LoggerPaths` value derived from each existing `run_dir`, pass its `tmp` directory to the monitored child as `TMPDIR`, and make both live and persistent evidence readers consume the same derived paths. Keep the global logger preflight only as a performance-noise guard.

**Tech Stack:** Rust 2021, Rust standard library, existing `portable-pty 0.8.1`, existing custom Cargo benchmark (`harness = false`).

## Global Constraints

- Apply SOLID by keeping path ownership in the benchmark fixture, not production logging; apply KISS and YAGNI with one private value object and no new module, dependency, or protocol.
- Modify only `benches/pty_hot_path.rs` and the concurrency note in `BACKLOG.md`.
- Do not change production logger behavior, logger filenames, monitor behavior, PTY behavior, dependencies, benchmark scenarios, run duration, thresholds, decision wording, or recorded benchmark numbers.
- Each monitored `<run_dir>` owns `tmp/claudego.log` and `tmp/claudego.port`; the direct scenario must not create or consume logger state.
- Set child `HOME=<run_dir>/home` and `TMPDIR=<run_dir>/tmp` with OS-native path values; do not add UTF-8 conversion for either environment value.
- On Unix/macOS, `std::env::temp_dir()` honors `TMPDIR`: <https://doc.rust-lang.org/std/env/fn.temp_dir.html>.
- `portable_pty::CommandBuilder::env` accepts keys and values implementing `AsRef<OsStr>`: <https://docs.rs/portable-pty/0.8.1/portable_pty/cmdbuilder/struct.CommandBuilder.html#method.env>.
- Preserve the initial active-global-logger preflight and its one-time stale-file cleanup. Do not delete global logger files before monitored runs.
- Missing run-local directories, ports, or logs must fail the named scenario; port/log read failures must include the run-local path.
- Reuse `ScratchDir` cleanup. Do not add cleanup code or a temporary-directory crate.
- Keep the existing watcher-ready, scan-deferral, idle, byte-integrity, child-status, sidecar, and timeout assertions.
- The controlled collision probe and full benchmark require localhost binding and must run outside a network-restricted sandbox.
- Required verification: `cargo bench --bench pty_hot_path --no-run`, `cargo bench --bench pty_hot_path`, controlled global-file collision, `cargo fmt --check`, `cargo test`, and `cargo clippy --all-targets --all-features -- -D warnings`.
- Do not stage or commit files under `docs/superpowers/specs/` or `docs/superpowers/plans/`; they are disposable working documents.
- The checkout contains unrelated untracked files. Stage only `benches/pty_hot_path.rs` and `BACKLOG.md`.

---

## File Structure

- Modify `benches/pty_hot_path.rs`: derive run-local logger paths, self-check their isolation, configure monitored child `TMPDIR`, and read live/persistent evidence from those paths.
- Modify `BACKLOG.md`: replace the obsolete global-correctness warning with the new isolation/performance-noise statement; preserve all recorded measurements.

---

### Task 1: Isolate Monitored Logger Evidence Per Run

**Files:**
- Modify: `benches/pty_hot_path.rs:65-559`
- Modify: `BACKLOG.md:22`
- Test: startup self-check inside `benches/pty_hot_path.rs::helper_self_check`

**Interfaces:**
- Consumes:
  - Existing `run_scenario(scenario: Scenario, claudego_exe: &Path, benchmark_exe: &Path, run_dir: &Path) -> BenchResult<RunMetrics>`.
  - Existing production filenames `claudego.log` and `claudego.port`.
  - Existing global helpers `global_log_path() -> PathBuf` and `global_port_path() -> PathBuf` for preflight only.
  - Unix/macOS child behavior where `std::env::temp_dir()` resolves the child `TMPDIR`.
- Produces:
  - Private `LoggerPaths { tmp_dir: PathBuf, log: PathBuf, port: PathBuf }`.
  - Private `logger_paths(run_dir: &Path) -> LoggerPaths`.
  - `finish_scenario(..., logger_paths: &LoggerPaths, deadline: Instant) -> BenchResult<RunMetrics>`.
  - `logger_address(port_path: &Path) -> Option<SocketAddr>`.
  - `start_log_stream(port_path: &Path, deadline: Instant) -> BenchResult<Receiver<String>>`.

- [ ] **Step 1: Add the failing path-isolation self-check**

In `helper_self_check`, insert this block immediately before the final `Ok(())`:

```rust
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
        return Err(failure("run-local logger paths equal global preflight paths"));
    }
```

- [ ] **Step 2: Compile to verify the self-check is red**

Run:

```bash
cargo bench --bench pty_hot_path --no-run
```

Expected: FAIL with Rust `E0425` because `logger_paths` is not defined. Do not continue if it fails for an unrelated reason.

- [ ] **Step 3: Add the minimal path value and route monitored evidence through it**

Add this code after the `impl Drop for ScratchDir` block:

```rust
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
```

Replace `run_scenario` with:

```rust
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
```

Replace `finish_scenario` with:

```rust
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
```

Replace `ensure_no_active_claudego`, `logger_address`, and `start_log_stream` with the following code. Keep `reset_global_log_files`, `global_log_path`, and `global_port_path` unchanged between these functions:

```rust
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
```

- [ ] **Step 4: Compile to verify the self-check is green**

Run:

```bash
cargo bench --bench pty_hot_path --no-run
```

Expected: PASS and finish with an executable path for `benches/pty_hot_path.rs`. Any `logger_paths`, `LoggerPaths`, or signature error means the routing is incomplete.

- [ ] **Step 5: Replace the obsolete backlog concurrency note**

Replace only `BACKLOG.md:22`:

```markdown
Benchmark logger evidence is isolated per run. Stop other `claudego` processes before benchmarking because competing work invalidates performance comparisons.
```

Do not alter the host, commit, baseline, after-optimization output, medians, or decisions below it.

- [ ] **Step 6: Run the full benchmark without competing work**

Run outside the network-restricted sandbox and preserve the complete output:

```bash
cargo bench --bench pty_hot_path
```

Expected: PASS with exactly nine run lines; all runs retain byte-count and logger assertions; output ends with three medians and the existing PTY, monitor, and upgrade/replace decisions. Compare the new medians and stalls with `BACKLOG.md`; if a throughput/stall regression appears, rerun the complete command once and reject the change only if the regression repeats.

- [ ] **Step 7: Prove a global logger collision cannot replace run-local evidence**

Run this controlled probe outside the network-restricted sandbox. It waits for the first monitored run's local port, then starts a second `claudego` using the inherited global `TMPDIR` so it replaces only the global logger files:

```bash
set -euo pipefail
collision_home="$(mktemp -d)"
marker="$(mktemp)"
output="$(mktemp)"
mkdir -p "$collision_home/.claude/projects/benchmark"

cargo bench --bench pty_hot_path >"$output" 2>&1 &
bench_pid=$!

while true; do
    run_port="$(find "${TMPDIR:-/tmp}" -type f -path '*/claudego-pty-hot-path-*/monitor-idle-1/tmp/claudego.port' -newer "$marker" -print -quit 2>/dev/null)"
    if [[ -n "$run_port" ]]; then
        break
    fi
    if ! kill -0 "$bench_pid" 2>/dev/null; then
        wait "$bench_pid" || true
        sed -n '1,240p' "$output"
        exit 1
    fi
    sleep 0.05
done

HOME="$collision_home" target/release/claudego -- /bin/sleep 5
wait "$bench_pid"
sed -n '1,240p' "$output"
test "$(rg -c '^(direct|monitor-idle|scan-deferred) run [123]:' "$output")" -eq 9
rg -n 'Medians:|PTY decision:|Monitor decision:|Upgrade/replace decision:' "$output"
```

Expected: the collision process exits successfully; the benchmark also exits successfully with nine run lines and all four summary/decision headings. Missing run-local live or persistent evidence is a failure even if the global process logged successfully. Ignore collision-run throughput because the competing process intentionally invalidates performance comparison.

- [ ] **Step 8: Run formatting and repository quality gates**

Run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: all commands PASS with no formatting diff, test failure, warning, or Clippy diagnostic.

- [ ] **Step 9: Review scope and commit only the implementation**

Run:

```bash
git diff --check
git diff -- benches/pty_hot_path.rs BACKLOG.md
git status --short
git add benches/pty_hot_path.rs BACKLOG.md
git commit -m "fix: isolate PTY benchmark logger paths"
```

Expected: the diff contains only run-local logger routing and the one backlog sentence; the commit contains exactly `benches/pty_hot_path.rs` and `BACKLOG.md`. Leave specs, plans, `.serena/`, and all unrelated files untracked.
