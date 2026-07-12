# Runtime Reliability and Performance Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the six proven runtime failure modes in continuation, scanning, watcher recovery, stream parsing, logging, and PTY shutdown, then measure three remaining performance candidates without shipping unproven optimizations.

**Architecture:** Preserve the current runner architecture and dependencies. Add only narrow typed outcomes at the existing ownership boundaries: lockout revisions own retry exhaustion, scans commit cursors only after stable completion, watcher events return continue-or-stop, parser state discards oversized logical lines, logger producers aggregate dropped diagnostics, and `app` owns the PTY reader drain handle.

**Tech Stack:** Rust 2021, Tokio, `portable-pty`, `notify`, `memmap2`, existing Cargo unit/integration tests and benchmark harness.

## Global Constraints

- Reliability changes precede measurements.
- Preserve the current CLI, runner selection, monitoring behavior, raw-output contract, and dependencies.
- Do not replace `portable-pty`, `Arc<Mutex<AppState>>`, mmap scanning, or the existing runner architecture.
- Do not add a generic retry framework, generic child restart policy, dependency, or configuration surface.
- Treat continuation as one initial attempt plus retries after 1, 2, and 4 seconds; never retry an ambiguous PTY write or flush failure.
- Keep the stream parser ceiling at 1 MiB unless captured valid NDJSON proves that insufficient.
- Preserve the existing 64 KiB PTY buffer, stdout lock, per-read flushing, and dirty output telemetry unless a later measurement task explicitly proves a change.
- Benchmark retention requires raw per-run evidence, not medians alone.
- Do not commit this plan or its design spec unless explicitly requested.

## File Structure

- Modify `src/models.rs`: store the lockout revision whose automatic continuation is exhausted.
- Modify `src/resume.rs`: classify success, definite failure, and ambiguous PTY failure.
- Modify `src/monitor/runtime.rs`: own bounded continuation retries and stable single-file scan results.
- Modify `src/monitor/mod.rs`: stop the detached monitor loop after watcher recovery exhaustion.
- Modify `src/stream_json.rs`: bound parser-only logical-line accumulation while preserving raw bytes.
- Modify `src/logging.rs`: aggregate non-blocking channel-full drops into a later diagnostic or shutdown summary.
- Modify `src/pty_bridge.rs`: return the blocking reader task handle without changing its telemetry or forwarding loop.
- Modify `src/app.rs`: drain that reader with a bounded timeout before logger shutdown.
- Modify `tests/pty_exit.rs`: prove final PTY bytes and reader-stop diagnostics precede shutdown.
- Create `benches/runtime_candidates.rs`: isolated measurement harness for startup scheduling delay, slow logger clients, and stream flush behavior; it does not change production behavior.
- Create `docs/superpowers/benchmarks/2026-07-12-runtime-candidates.md`: commands, environment, raw results, and keep/reject decisions.

---

### Task 1: Typed Continuation Outcomes and Revision-Scoped Retry Exhaustion

**Files:**
- Modify: `src/models.rs:8-31`
- Modify: `src/resume.rs:6-35`
- Modify: `src/monitor/runtime.rs:178-220`
- Modify: `src/monitor/mod.rs:40-56`

**Interfaces:**
- Consumes: `ResumeTarget`, `SharedAppState`, expired target, current `lockout_revision`.
- Produces: `ResumeOutcome::{Sent, DefiniteFailure(String), AmbiguousFailure(String)}` and async `handle_expiry(...)`; `AppState::resume_exhausted_revision: Option<u64>` prevents another sequence for the same revision.

- [ ] **Step 1: Add failing state and outcome tests**

Add a scripted test target so retry behavior is deterministic without sleeping:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResumeOutcome {
    Sent,
    DefiniteFailure(String),
    AmbiguousFailure(String),
}

#[test]
fn new_lockout_revision_is_not_exhausted() {
    let mut state = AppState::new();
    state.resume_exhausted_revision = Some(7);
    state.lockout_revision = 8;
    assert_ne!(state.resume_exhausted_revision, Some(state.lockout_revision));
}

#[tokio::test(start_paused = true)]
async fn three_retries_retain_lockout_and_mark_revision_exhausted() {
    let target = Local::now();
    let state = shared_state_with_lockout(target, 3);
    let resume = ScriptedResume::definite_failures(4);
    let task = tokio::spawn(handle_expiry_with(&state, target, resume.clone()));
    tokio::time::advance(Duration::from_secs(7)).await;
    task.await.unwrap();

    let app = state.lock().unwrap();
    assert_eq!(resume.attempts(), 4);
    assert_eq!(app.lockout_target_time, Some(target));
    assert_eq!(app.resume_exhausted_revision, Some(3));
}
```

Keep `ScriptedResume` and `handle_expiry_with` private to `monitor::runtime`; the production wrapper passes `ResumeTarget::resume` and `tokio::time::sleep`.

- [ ] **Step 2: Verify red**

Run: `cargo test monitor::runtime::tests::three_retries_retain_lockout_and_mark_revision_exhausted -- --nocapture`

Expected: compile failure because the outcome, marker, and injected expiry helper do not exist.

- [ ] **Step 3: Classify continuation results minimally**

Implement `ResumeTarget::resume` as:

```rust
pub fn resume(&self) -> ResumeOutcome {
    match self {
        ResumeTarget::Pty(writer) => {
            let mut writer = writer.lock().unwrap_or_else(|e| e.into_inner());
            if let Err(error) = writer.write_all(b"continue\r") {
                return ResumeOutcome::AmbiguousFailure(format!(
                    "failed to write PTY continue command: {error}"
                ));
            }
            match writer.flush() {
                Ok(()) => ResumeOutcome::Sent,
                Err(error) => ResumeOutcome::AmbiguousFailure(format!(
                    "failed to flush PTY continue command: {error}"
                )),
            }
        }
        ResumeTarget::StreamJson(tx) => match tx.send(StreamResumeCommand::Continue) {
            Ok(()) => ResumeOutcome::Sent,
            Err(_) => ResumeOutcome::DefiniteFailure(
                "stream-json runner is no longer available".to_string(),
            ),
        },
    }
}
```

Add `resume_exhausted_revision: Option<u64>` initialized to `None`. `record_lockout` need not clear it: incrementing `lockout_revision` makes the old marker inapplicable.

- [ ] **Step 4: Implement bounded expiry handling**

Capture both target and revision. Attempt immediately, then only definite failures after `[1, 2, 4]` seconds. On success, clear target/cache only if both still match; on ambiguity or exhausted definite failures, retain state and set the revision marker. At the monitor call site, skip `handle_expiry` when `resume_exhausted_revision == Some(lockout_revision)` and wait for watcher events instead of spinning.

```rust
const RESUME_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];
```

- [ ] **Step 5: Verify success, failure, ambiguity, and revision replacement**

Run:

```bash
cargo test resume::tests::
cargo test monitor::runtime::tests::resume_
```

Expected: success clears the matching lockout/cache; four definite failures retain it and stop; an ambiguous failure makes one attempt; a newer revision is never cleared by an older attempt.

- [ ] **Step 6: Commit**

```bash
git add src/models.rs src/resume.rs src/monitor/runtime.rs src/monitor/mod.rs
git commit -m "fix: retain lockout after failed continuation"
```

Before every commit in this plan: stage only intended files, spawn `commit_agent` with the staged diff, verify/correct its message, then commit.

---

### Task 2: Stable Scan Cursor Commit

**Files:**
- Modify: `src/monitor/runtime.rs:14-176,220-310`

**Interfaces:**
- Consumes: one `PathBuf`, old cursor, captured metadata and length from one open `File` handle.
- Produces: private `scan_stable_range(path, old_size) -> Result<StableScan, ScanFailure>`; only `StableScan { new_size, limit, preview }` permits cache commit.

- [ ] **Step 1: Add failing regression tests**

```rust
#[test]
fn failed_scan_does_not_advance_cursor() {
    let path = unique_path("missing.jsonl");
    let state = state_with_cursor(path.clone(), 17);
    let result = scan_one_path(&path, &state);
    assert!(result.is_err());
    assert_eq!(state.lock().unwrap().file_size_cache[&path], 17);
}

#[test]
fn truncated_captured_range_is_retryable_not_a_panic() {
    let file = tempfile_without_dependency(b"0123456789");
    let result = scan_mmap_range_for_test(&file, 0, 20);
    assert!(matches!(result, Err(ScanFailure::Changed)));
}
```

Use the existing timestamp/PID temp-path pattern; do not add `tempfile`.

- [ ] **Step 2: Verify red**

Run: `cargo test monitor::runtime::tests::failed_scan_does_not_advance_cursor`

Expected: compile failure because the scan result seam does not exist.

- [ ] **Step 3: Extract the stable scan result**

```rust
struct StableScan {
    new_size: u64,
    limit: Option<ActiveRateLimitInfo>,
    preview: Option<String>,
}

enum ScanFailure {
    Open(std::io::Error),
    Metadata(std::io::Error),
    Mmap(std::io::Error),
    InvalidUtf8,
    Changed,
}
```

Open once, read `metadata()` from that handle, capture `len` and `modified`, mmap it, reject `new_size > mmap.len()`, scan only `old_size.saturating_sub(4096)..new_size`, then re-read metadata from the same handle. Return `Changed` unless length and modified time match. Keep path-replacement identity checks out of scope because the spec does not define a portable identity contract.

- [ ] **Step 4: Commit only after stable success**

In `scan_and_update_state`, call the helper and insert `new_size` only in the `Ok(StableScan { .. })` arm. Open, metadata, mmap, UTF-8, truncation, and stability errors leave the old cache entry untouched. Preserve overlap scanning and preview behavior.

Rate-limit repeated failures per path with a local `HashMap<PathBuf, Instant>` owned by `spawn_lockout_monitor` and passed to scanning; emit at most once per 30 seconds. Do not create a generic rate limiter or configuration.

- [ ] **Step 5: Verify concurrent append and truncation recovery**

Add one deterministic injected hook between mmap scan and final metadata read. The hook appends/truncates once; first call must return `Changed` and preserve the cursor, second call must commit and detect the appended rate-limit row.

Run: `cargo test monitor::runtime::tests::scan_ -- --nocapture`

Expected: all scan tests pass without panic; failed/unstable attempts preserve the old cursor.

- [ ] **Step 6: Commit**

```bash
git add src/monitor/runtime.rs src/monitor/mod.rs
git commit -m "fix: commit scan cursor after stable reads"
```

---

### Task 3: Stop After Watcher Recovery Exhaustion

**Files:**
- Modify: `src/monitor/mod.rs:25-118`

**Interfaces:**
- Consumes: watcher receiver result and existing `lifecycle::create_watcher()` retries.
- Produces: private `MonitorControl::{Continue, Stop}` from `handle_event_result`; both event-loop branches honor `Stop`.

- [ ] **Step 1: Add a failing control-flow test**

Extract only recovery selection, not a watcher abstraction:

```rust
#[tokio::test]
async fn exhausted_recovery_stops_monitor() {
    let control = recovery_control(async { None }).await;
    assert_eq!(control, MonitorControl::Stop);
}
```

- [ ] **Step 2: Verify red**

Run: `cargo test monitor::tests::exhausted_recovery_stops_monitor`

Expected: compile failure because `MonitorControl` and `recovery_control` do not exist.

- [ ] **Step 3: Return and honor the stop result**

```rust
#[derive(Debug, PartialEq, Eq)]
enum MonitorControl { Continue, Stop }
```

Return `Continue` for events, notify errors, and successful recovery. Return `Stop` after `create_watcher()` returns `None`; log exactly one degraded-operation terminal line there. At both call sites:

```rust
if handle_event_result(event_res, &mut handle, &state, &mut next_log_time).await
    == MonitorControl::Stop
{
    return;
}
```

- [ ] **Step 4: Verify no closed-receiver loop**

Run: `cargo test monitor::tests::`

Expected: recovery exhaustion returns `Stop`; successful recovery returns `Continue`.

- [ ] **Step 5: Commit**

```bash
git add src/monitor/mod.rs
git commit -m "fix: stop monitor after watcher recovery fails"
```

---

### Task 4: Bound Stream Parser Accumulation

**Files:**
- Modify: `src/stream_json.rs:1-67,336-580`
- Inspect: `docs/superpowers/fixtures/*.ndjson`

**Interfaces:**
- Consumes: arbitrary raw byte chunks.
- Produces: byte-for-byte raw forwarding plus best-effort complete lines no larger than `MAX_PARSER_LINE_BYTES`; oversized lines emit one diagnostic and are ignored until newline.

- [ ] **Step 1: Verify the ceiling against captured data**

Run:

```bash
find docs/superpowers/fixtures -name '*.ndjson' -type f -exec awk '{ if (length > max) max=length } END { print FILENAME, max }' {} \;
```

Expected current maximum: 14,049 bytes, safely below 1,048,576. If any valid captured line exceeds 1 MiB, stop and raise the constant to the smallest documented power-of-two ceiling above the observed maximum before continuing.

- [ ] **Step 2: Add the failing oversized-line recovery test**

```rust
#[tokio::test]
async fn oversized_line_preserves_raw_bytes_and_resumes_after_newline() {
    let oversized = vec![b'x'; MAX_PARSER_LINE_BYTES + 1];
    let valid = b"\n{\"type\":\"system\",\"session_id\":\"after-cap\"}\n";
    let input = [oversized.as_slice(), valid].concat();
    let (line_tx, mut line_rx) = mpsc::channel(4);
    let mut output = Vec::new();

    pump_raw_output(
        FragmentedReader::new_owned(input.chunks(8192).map(Vec::from).collect()),
        &mut output,
        line_tx,
        AppState::new().last_output_activity,
    ).await.unwrap();

    assert_eq!(output, input);
    assert_eq!(line_rx.recv().await.as_deref(), Some(
        r#"{"type":"system","session_id":"after-cap"}"#
    ));
    assert!(line_rx.try_recv().is_err());
}
```

- [ ] **Step 3: Verify red**

Run: `cargo test stream_json::tests::oversized_line_preserves_raw_bytes_and_resumes_after_newline`

Expected: failure because current `pending` grows past the cap and the oversized line is forwarded to the parser.

- [ ] **Step 4: Add one bounded parser state machine**

```rust
const MAX_PARSER_LINE_BYTES: usize = 1024 * 1024;

let mut pending = Vec::new();
let mut discard_until_newline = false;
```

For every read: write and flush raw bytes first, then process parser bytes. While discarding, skip through the first newline and resume on the remaining bytes. While accumulating, append no more than the remaining capacity; if a logical line exceeds the cap, clear `pending`, log once, set discard mode, and do not inspect that line for signals. Use slice indexes or `memchr`; do not repeatedly `drain` a growing oversized buffer.

- [ ] **Step 5: Verify exact output and normal parsing**

Run:

```bash
cargo test stream_json::tests::oversized_line_preserves_raw_bytes_and_resumes_after_newline
cargo test stream_json::tests::fragmented_
cargo test --test streaming_stress
```

Expected: raw bytes match exactly, only the post-newline valid record is parsed, and existing stress coverage passes.

- [ ] **Step 6: Commit**

```bash
git add src/stream_json.rs
git commit -m "fix: bound stream parser line buffering"
```

---

### Task 5: Observable Non-Blocking Logger Drops

**Files:**
- Modify: `src/logging.rs:1-260,262-360`

**Interfaces:**
- Consumes: existing `log_to_file` and `log_with_content` calls.
- Produces: one global `AtomicU64` channel-full count; the next successfully reserved line reports the aggregate, or shutdown writes the remaining aggregate. Producers remain non-blocking.

- [ ] **Step 1: Add failing queue-saturation tests**

Test a private helper with a capacity-one channel:

```rust
#[tokio::test]
async fn full_channel_reports_aggregate_on_next_success() {
    let (tx, mut rx) = mpsc::channel(1);
    let dropped = AtomicU64::new(0);
    tx.try_send(LogMessage::Line("occupied".into())).unwrap();

    try_queue_line(&tx, &dropped, "lost one".into());
    try_queue_line(&tx, &dropped, "lost two".into());
    assert_eq!(dropped.load(Ordering::Relaxed), 2);
    assert!(matches!(rx.recv().await, Some(LogMessage::Line(_))));

    try_queue_line(&tx, &dropped, "next".into());
    let Some(LogMessage::Line(line)) = rx.recv().await else { panic!() };
    assert!(line.contains("2 diagnostic message(s) dropped"));
    assert!(line.contains("next"));
    assert_eq!(dropped.load(Ordering::Relaxed), 0);
}
```

- [ ] **Step 2: Verify red**

Run: `cargo test logging::tests::full_channel_reports_aggregate_on_next_success`

Expected: compile failure because the counter/helper do not exist.

- [ ] **Step 3: Centralize producer enqueueing**

Add `static DROPPED_LOG_MESSAGES: AtomicU64`. Use `Sender::try_reserve()` so the counter is swapped only after capacity is secured:

```rust
fn try_queue_line(sender: &Sender<LogMessage>, dropped: &AtomicU64, mut line: String) {
    match sender.try_reserve() {
        Ok(permit) => {
            let count = dropped.swap(0, Ordering::AcqRel);
            if count > 0 {
                line = format!("[Logger] {count} diagnostic message(s) dropped: channel full\n{line}");
            }
            permit.send(LogMessage::Line(line));
        }
        Err(mpsc::error::TrySendError::Full(_)) => {
            dropped.fetch_add(1, Ordering::Relaxed);
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {}
    }
}
```

Route both public logging functions through it. Do not block, retry individual lines, or count closed receivers as saturation.

- [ ] **Step 4: Include a final shutdown aggregate**

Change `LogMessage::Shutdown` to carry `dropped: u64`. `shutdown_logging` atomically takes the remaining count and awaits the shutdown message; `run_logger` writes the summary directly before breaking. This path may await because shutdown is not a hot producer.

- [ ] **Step 5: Verify saturation and current client behavior**

Run: `cargo test logging::tests:: -- --nocapture`

Expected: aggregate count is exact, callers remain non-blocking, startup buffering and slow-client tests still pass.

- [ ] **Step 6: Commit**

```bash
git add src/logging.rs
git commit -m "fix: report dropped logger diagnostics"
```

---

### Task 6: Drain PTY Reader Before Logger Shutdown

**Files:**
- Modify: `src/pty_bridge.rs:43-117`
- Modify: `src/app.rs:15-66`
- Modify: `tests/pty_exit.rs`

**Interfaces:**
- Consumes: blocking reader task returned by `spawn_output_reader`.
- Produces: `tokio::task::JoinHandle<()>`; `app` waits at most `PTY_DRAIN_TIMEOUT` after child exit, logs timeout/join failure, then queues child-exit shutdown and closes logging.

- [ ] **Step 1: Add a failing final-byte integration assertion**

Extend the existing isolated `tests/pty_exit.rs` child fixture to write a unique final marker immediately before exit. Capture stdout and isolated `claudego.log`, then assert:

```rust
assert!(stdout.ends_with(b"FINAL-PTY-BYTES\r\n"));
let reader_stop = log.find("[PTY Output] reader stopped").unwrap();
let shutdown = log.find("[System] Child process exited. Shutting down.").unwrap();
assert!(reader_stop < shutdown);
```

- [ ] **Step 2: Verify the regression test exposes current ordering**

Run: `cargo test --test pty_exit final_pty_bytes_drain_before_shutdown -- --nocapture`

Expected: failure because `app` does not own or await the reader task.

- [ ] **Step 3: Return the existing reader handle**

Change only the signature and final expression:

```rust
pub fn spawn_output_reader(
    mut reader: Box<dyn Read + Send>,
    state: SharedAppState,
) -> tokio::task::JoinHandle<()> {
    // existing telemetry setup unchanged
    tokio::task::spawn_blocking(move || {
        // existing 64 KiB forwarding loop unchanged
    })
}
```

Do not delete or rewrite the dirty telemetry heartbeat; its `done` flag remains owned by reader completion.

- [ ] **Step 4: Drain before shutdown**

In the PTY branch, keep `reader_handle`. After the child wait:

```rust
const PTY_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

match tokio::time::timeout(PTY_DRAIN_TIMEOUT, reader_handle).await {
    Ok(Ok(())) => {}
    Ok(Err(error)) => logging::log_to_file(&format!(
        "[PTY Output Error] reader task failed: {error}"
    )),
    Err(_) => logging::log_to_file(&format!(
        "[PTY Output Error] reader drain timed out after {PTY_DRAIN_TIMEOUT:?}"
    )),
}
```

Only after this block log child exit and call `shutdown_logging`.

- [ ] **Step 5: Verify normal and timeout paths**

Use a private `drain_reader(handle, timeout)` helper so a unit test can pass a permanently pending task and assert `TimedOut` without waiting two seconds. Run:

```bash
cargo test pty_bridge::tests::
cargo test app::tests::reader_drain_timeout_is_bounded
cargo test --test pty_exit -- --nocapture
```

Expected: final bytes and reader-stop log precede shutdown; timeout is diagnosed and exit remains bounded.

- [ ] **Step 6: Commit**

```bash
git add src/pty_bridge.rs src/app.rs tests/pty_exit.rs
git commit -m "fix: drain PTY output before logger shutdown"
```

---

### Task 7: Measure Runtime Candidates Without Production Changes

**Files:**
- Create: `benches/runtime_candidates.rs`
- Modify: `Cargo.toml`
- Create: `docs/superpowers/benchmarks/2026-07-12-runtime-candidates.md`
- Inspect only: `src/monitor/startup.rs`, `src/logging.rs`, `src/stream_json.rs`

**Interfaces:**
- Consumes: isolated history directory, local TCP clients, deterministic chunked byte source.
- Produces: raw per-run scheduling delay, file-log/viewer latency and drop counts, stream throughput/first-byte/inter-chunk latency/exact-output results. No production code change.

- [ ] **Step 1: Add the benchmark target and exact raw-result format**

Add:

```toml
[[bench]]
name = "runtime_candidates"
harness = false
```

The harness accepts exactly one case: `startup-scan`, `logger-fanout`, or `stream-flush`, plus `--runs N`. Each run prints one NDJSON record containing `case`, `run`, fixture size, each requested latency/throughput field, `output_sha256` only if an existing tool supplies it (otherwise exact byte equality), `output_equal`, and `dropped_messages` where relevant. Use stdlib/Tokio and current crate code; add no benchmark framework or dependency.

- [ ] **Step 2: Measure startup scheduling delay**

Create a large isolated Claude history under a run-local `HOME`/`TMPDIR`. Tick a 1 ms Tokio interval while invoking the current synchronous initial scan seam and record maximum scheduling delay. Run at least nine idle-machine trials:

```bash
cargo bench --bench runtime_candidates -- startup-scan --runs 9
```

Record every raw line. Do not implement `spawn_blocking` unless the accepted threshold is written before the run and the observed delay materially exceeds it.

- [ ] **Step 3: Measure logger fan-out**

Connect one healthy reader plus several non-reading clients, enqueue timestamped markers, and record file-log latency, healthy-viewer latency, and aggregated dropped-message count:

```bash
cargo bench --bench runtime_candidates -- logger-fanout --runs 9
```

If material harm is proven, the only next proposal is documenting/enforcing one active viewer. Do not implement per-client tasks or queues in this plan.

- [ ] **Step 4: Measure stream flushing with injected writers**

Run the same deterministic chunks through the existing per-read-flush path and a benchmark-local no-per-read-flush variant. Record throughput, first-byte latency, maximum inter-chunk latency, and exact output equality:

```bash
cargo bench --bench runtime_candidates -- stream-flush --runs 9
```

Do not modify `pump_raw_output` from these results unless responsiveness is preserved under a pre-recorded acceptance threshold and all raw output is equal.

- [ ] **Step 5: Record keep/reject decisions**

In `docs/superpowers/benchmarks/2026-07-12-runtime-candidates.md`, include date, git SHA, OS/toolchain, idle-machine condition, exact commands, fixture sizes, predeclared thresholds, all raw lines, and one decision per candidate: `retain current behavior` or `open a separately reviewed optimization plan`. Do not summarize away outliers.

- [ ] **Step 6: Verify the measurement harness**

Run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo bench --bench runtime_candidates -- startup-scan --runs 1
cargo bench --bench runtime_candidates -- logger-fanout --runs 1
cargo bench --bench runtime_candidates -- stream-flush --runs 1
```

Expected: all gates pass; every smoke case emits one complete raw record; no production source diff is introduced by Task 7.

- [ ] **Step 7: Commit benchmark evidence only if explicitly requested**

```bash
git add Cargo.toml benches/runtime_candidates.rs docs/superpowers/benchmarks/2026-07-12-runtime-candidates.md
git commit -m "bench: measure runtime hardening candidates"
```

The repository preference is to leave plans/designs local. Ask before committing benchmark documentation as well.

---

### Task 8: Full Verification and Acceptance Audit

**Files:**
- Verify all files changed by Tasks 1-7.

**Interfaces:**
- Consumes: completed reliability tasks and raw measurement evidence.
- Produces: a clean acceptance report; no new behavior.

- [ ] **Step 1: Run focused regression coverage**

```bash
cargo test monitor::runtime::tests::
cargo test monitor::tests::
cargo test stream_json::tests::
cargo test logging::tests::
cargo test --test pty_exit -- --nocapture
cargo test --test streaming_stress -- --nocapture
```

Expected: all pass.

- [ ] **Step 2: Run repository gates**

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo bench --bench pty_hot_path
```

Expected: all commands exit zero. Run the PTY benchmark on an otherwise idle machine and retain every raw run; request sandbox approval if required.

- [ ] **Step 3: Audit behavior and diff scope**

Run:

```bash
git diff --check
git diff --stat
git diff -- src/cli.rs
```

Confirm: CLI/runner selection is unchanged; scan failures never commit cursors; failed continuation never clears lockout; watcher exhaustion stops; raw stream bytes remain exact; logger drops are observable; PTY reader completion precedes logger shutdown; no measurement-gated optimization entered production.

- [ ] **Step 4: Final commit only if implementation commits were intentionally squashed**

Do not create an empty or documentation-only commit. If squashing was explicitly requested, stage the intended implementation, spawn `commit_agent`, verify its message, and commit once.
