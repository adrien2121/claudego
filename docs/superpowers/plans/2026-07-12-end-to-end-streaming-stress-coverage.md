# End-to-End Streaming Stress Coverage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add deterministic unit, focused integration, and serialized Unix end-to-end coverage proving the complete `stream-json` path preserves raw child output while parsing signals, scanning files, and serving a slow live-log client.

**Architecture:** Keep production behavior and public APIs unchanged. Extend existing private seams in `stream_json`, `monitor::runtime`, and `logging`; add one fixed Cargo helper binary and one Unix integration test that exposes that helper as `claude`, so the real classifier is exercised. Raw output remains authoritative; parsing and logging remain bounded best-effort side work.

**Tech Stack:** Rust 2021, Tokio, Cargo integration tests, existing `notify`, `chrono`, `serde_json`, and standard-library filesystem/process/TCP APIs.

## Global Constraints

- Implement all three layers: unit, focused integration, and one serialized full E2E.
- Do not change stream semantics, resume behavior, rate-limit schemas, log format, or public CLI behavior.
- Do not add dependencies, benchmarks, retry loops, or a production synthetic-test flag.
- Timeouts are deadlock guards, not performance requirements.
- Write and flush raw bytes before parser `try_send`; saturation may drop parse/log work but may not delay or corrupt output.
- Keep unit/focused coverage portable; gate deterministic process-control E2E with `#[cfg(unix)]`.
- Use one conservative E2E deadline; kill and reap on timeout; include status, stdout, stderr, and isolated log evidence.
- Do not commit this plan or its design spec unless explicitly requested.

## File Structure

- Create `src/bin/stream-stress-child.rs`: two fixed synthetic scenarios.
- Modify `src/stream_json.rs`: unit cases plus one private writer-injection seam for spawned-runner coverage.
- Modify `src/monitor/runtime.rs`: direct test of the existing private scanner seam.
- Modify `src/logging.rs`: extract the existing task body into one private function and test it using loopback TCP.
- Create `tests/streaming_stress.rs`: one serialized Unix real-binary E2E and local stdlib-only helpers.

No fixture library or general test harness: a tiny duplicated expected-byte builder is simpler than exposing test support through production code.

---

### Task 1: Deterministic Synthetic Child and Unit Coverage

**Files:**
- Create: `src/bin/stream-stress-child.rs`
- Modify: `src/stream_json.rs:336-547`

**Interfaces:**
- Consumes: scenario argument `stream-signal` or `no-stream-signal`.
- Produces: fixed fragmented stdout containing ordinary, invalid, unknown, session, optional rate-limit, 1,100 overload lines, and an incomplete tail; writes `READY\n` to stderr before a fixed quiet phase.

- [ ] **Step 1: Add failing/missing unit cases**

Add a test-only `FragmentedReader` implementing `AsyncRead` from a `VecDeque<Vec<u8>>`, then add:

```rust
#[test]
fn ignores_valid_signal_free_event() {
    assert!(matches!(
        parse_stream_line(r#"{"type":"result","result":"ok"}"#),
        StreamLineResult::Ignored
    ));
}

#[tokio::test]
async fn fragmented_overloaded_input_preserves_every_raw_byte() {
    let state = AppState::new();
    let (line_tx, mut line_rx) = mpsc::channel(1);
    line_tx.try_send("already full".to_string()).unwrap();
    let chunks: &[&[u8]] = &[
        b"{\"type\":\"ass",
        b"istant\"}\nnot json\n{\"unknown\":",
        b"true}\n{\"partial\":true}",
    ];
    let expected = chunks.concat();
    let mut output = Vec::new();

    tokio::time::timeout(
        Duration::from_secs(1),
        pump_raw_output(
            FragmentedReader::new(chunks),
            &mut output,
            line_tx,
            state.last_output_activity.clone(),
        ),
    )
    .await
    .expect("pump deadlocked")
    .expect("pump failed");

    assert_eq!(output, expected);
    assert_eq!(line_rx.try_recv().as_deref(), Ok("already full"));
}
```

`FragmentedReader::poll_read` pops exactly one chunk, calls `ReadBuf::put_slice`, and returns `Poll::Ready(Ok(()))); EOF is an empty successful read. Import `Pin`, `Context`, `Poll`, `AsyncRead`, and `ReadBuf`.

- [ ] **Step 2: Run the unit tests and prove the helper is absent**

Run:

```bash
cargo test stream_json::tests::ignores_valid_signal_free_event
cargo test stream_json::tests::fragmented_overloaded_input_preserves_every_raw_byte
cargo run --bin stream-stress-child -- stream-signal
```

Expected: both tests pass against current behavior; the last command fails with `no bin target named 'stream-stress-child'`.

- [ ] **Step 3: Create the fixed helper**

Create `src/bin/stream-stress-child.rs` with constants for the exact records above, this writer, and a two-value argument match:

```rust
fn write_fragmented(out: &mut impl std::io::Write, bytes: &[u8]) -> std::io::Result<()> {
    for chunk in bytes.chunks(7) {
        out.write_all(chunk)?;
        out.flush()?;
        std::thread::yield_now();
    }
    Ok(())
}

fn main() -> std::io::Result<()> {
    let scenario = std::env::args().nth(1).unwrap_or_default();
    if !matches!(scenario.as_str(), "stream-signal" | "no-stream-signal") {
        eprintln!("usage: stream-stress-child <stream-signal|no-stream-signal>");
        std::process::exit(2);
    }

    let mut out = std::io::stdout().lock();
    write_fragmented(&mut out, ORDINARY)?;
    write_fragmented(&mut out, INVALID)?;
    write_fragmented(&mut out, UNKNOWN)?;
    write_fragmented(&mut out, SESSION)?;
    if scenario == "stream-signal" {
        write_fragmented(&mut out, RATE_LIMIT)?;
    }
    for index in 0..1_100 {
        write_fragmented(
            &mut out,
            format!("{{\"type\":\"overload\",\"index\":{index}}}\n").as_bytes(),
        )?;
    }
    write_fragmented(&mut out, b"{\"incomplete\":true}")?;
    eprintln!("READY");
    std::thread::sleep(std::time::Duration::from_secs(3));
    Ok(())
}
```

Use the same session/rate-limit bytes already asserted in `stream_json.rs`, including session ID `11111111-1111-1111-1111-111111111111` and the 2099 `resets 5:30pm` event.

- [ ] **Step 4: Verify determinism**

Run:

```bash
cargo test stream_json::tests::
cargo run --quiet --bin stream-stress-child -- stream-signal > /tmp/stream-stress-1.ndjson 2>/tmp/stream-stress-1.err
cargo run --quiet --bin stream-stress-child -- stream-signal > /tmp/stream-stress-2.ndjson 2>/tmp/stream-stress-2.err
cmp /tmp/stream-stress-1.ndjson /tmp/stream-stress-2.ndjson
```

Expected: tests pass; both runs exit zero; `cmp` exits zero; each stderr contains exactly `READY`.

- [ ] **Step 5: Commit**

```bash
git add src/bin/stream-stress-child.rs src/stream_json.rs
git commit -m "test: add deterministic stream stress fixture"
```

Before every commit in this plan: stage intended files, dispatch `commit_agent` against the staged diff, verify/correct its message, then commit.

---

### Task 2: Spawned Stream Runner with Captured Writers

**Files:**
- Modify: `src/stream_json.rs:68-190,336-547`

**Interfaces:**
- Consumes: existing `CommandSpec`, `SharedAppState`, latest-session mutex, resume receiver, and injected `AsyncWrite + Unpin + Send + 'static` writers.
- Produces: private `run_one_stream_process_with_writers(...)->Result<StreamProcessAction>`; public `run_stream_json_print` remains unchanged.

- [ ] **Step 1: Add the failing focused test**

Add a test-only `SharedVecWriter(Arc<tokio::sync::Mutex<Vec<u8>>>)` implementing `AsyncWrite`, then call:

```rust
let action = run_one_stream_process_with_writers(
    CommandSpec {
        program: env!("CARGO_BIN_EXE_stream-stress-child").to_string(),
        args: vec!["stream-signal".to_string()],
    },
    Arc::clone(&state),
    Arc::clone(&latest_session_id),
    &mut resume_rx,
    SharedVecWriter(Arc::clone(&stdout)),
    SharedVecWriter(Arc::clone(&stderr)),
)
.await
.expect("stream child succeeds");

assert_eq!(action, StreamProcessAction::Exit);
assert_eq!(
    latest_session_id.lock().unwrap().as_deref(),
    Some("11111111-1111-1111-1111-111111111111")
);
assert_eq!(state.lock().unwrap().lockout_revision, 1);
assert_eq!(&*stderr.lock().await, b"READY\n");
assert!(stdout.lock().await.ends_with(b"{\"incomplete\":true}"));
```

`SharedVecWriter::poll_write` uses `try_lock`, appends all bytes, and returns their length; flush/shutdown return ready success.

- [ ] **Step 2: Verify red**

Run: `cargo test stream_json::tests::spawned_stream_child_preserves_output_and_records_signals -- --nocapture`

Expected: compile failure because the private writer seam does not exist. If Cargo does not expose the helper variable to the unit target, move only this test to `tests/streaming_stress.rs` and make the seam `pub(crate)`; do not add runtime binary discovery.

- [ ] **Step 3: Add the minimal seam and required failure propagation**

Rename the current private worker to generic `run_one_stream_process_with_writers`, pass injected writers to both pump tasks, and keep this production wrapper:

```rust
async fn run_one_stream_process(
    command: CommandSpec,
    state: SharedAppState,
    latest_session_id: Arc<Mutex<Option<String>>>,
    resume_rx: &mut mpsc::UnboundedReceiver<StreamResumeCommand>,
) -> Result<StreamProcessAction> {
    run_one_stream_process_with_writers(
        command, state, latest_session_id, resume_rx,
        tokio::io::stdout(), tokio::io::stderr(),
    ).await
}
```

Reject nonzero child status with `anyhow::bail!("stream child exited with status {status}")`, and replace ignored joins with:

```rust
stdout_task.await??;
stderr_task.await??;
parser_task.await?;
```

This changes only failure reporting; successful behavior and public signatures stay intact.

- [ ] **Step 4: Verify**

Run:

```bash
cargo test stream_json::tests::spawned_stream_child_preserves_output_and_records_signals -- --nocapture
cargo test stream_json::tests::
cargo test cli::tests::
```

Expected: all pass; session ID is observed and revision is exactly one.

- [ ] **Step 5: Commit**

```bash
git add src/stream_json.rs
git commit -m "test: cover spawned stream runner"
```

---

### Task 3: Direct Scanner Fallback and Slow Logger Client

**Files:**
- Modify: `src/monitor/runtime.rs:216-241`
- Modify: `src/logging.rs:47-224,262-293`

**Interfaces:**
- Consumes: existing private `scan_and_update_state(HashSet<PathBuf>, &SharedAppState, &mut Instant)`; new private `run_logger(TcpListener, PathBuf, Receiver<LogMessage>, usize, Duration)`.
- Produces: direct fallback state proof and loopback slow-client isolation; production logger constructor/global sender remain unchanged.

- [ ] **Step 1: Add direct scanner test**

Create a unique stdlib temp directory, write a baseline JSONL row, record its length in `AppState.file_size_cache`, append one generated future row, and invoke `scan_and_update_state` directly:

```rust
let now = Local::now();
let target = now + chrono::Duration::hours(2);
let reset = target.format("%-I:%M%P");
let row = format!(
    "{{\"timestamp\":\"{}\",\"error\":\"rate_limit\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"You've hit your session limit · resets {reset} (America/Toronto)\"}}]}}}}\n",
    now.to_rfc3339()
);
OpenOptions::new().append(true).open(&path).unwrap()
    .write_all(row.as_bytes()).unwrap();

scan_and_update_state(HashSet::from([path.clone()]), &state, &mut next_log_time).await;
let app = state.lock().unwrap();
assert_eq!(app.lockout_revision, 1);
assert_eq!(
    app.lockout_target_time.expect("watcher target").format("%-I:%M%P").to_string(),
    reset
);
```

Expected raw-message parsing is already asserted at the pure parser boundary; do not add raw message to `AppState` solely for test visibility.

- [ ] **Step 2: Run scanner test**

Run: `cargo test monitor::runtime::tests::direct_scan_records_one_file_watcher_lockout_from_new_content -- --nocapture`

Expected: PASS with revision one.

- [ ] **Step 3: Add failing logger-loop test**

Bind a real `TcpListener`, connect one non-reading client, submit 5,000 16-KiB lines through a bounded channel using `try_send`, and concurrently write/read `b"sentinel"` through `tokio::io::duplex(64)` under a five-second timeout. Assert exact sentinel bytes, send `LogMessage::Shutdown`, await logger, and remove its temp file.

- [ ] **Step 4: Verify red**

Run: `cargo test logging::tests::non_reading_client_does_not_block_sentinel_work -- --nocapture`

Expected: compile failure because `run_logger` does not exist.

- [ ] **Step 5: Extract only the existing logger loop**

Move the current spawned task body to:

```rust
async fn run_logger(
    listener: TcpListener,
    path: std::path::PathBuf,
    mut log_rx: mpsc::Receiver<LogMessage>,
    max_startup_lines: usize,
    client_write_timeout: Duration,
) {
    // Existing accept, startup buffer, file rotation, receive, client removal,
    // and final writer flush logic moves here unchanged.
}
```

Replace only `MAX_STARTUP_BUFFER_LINES` and `CLIENT_WRITE_TIMEOUT` inside the moved body with parameters. Keep listener binding/port publication and post-loop port-file removal in `init_logging`; spawn `run_logger(listener, paths::log_path(), log_rx, MAX_STARTUP_BUFFER_LINES, CLIENT_WRITE_TIMEOUT)`.

- [ ] **Step 6: Verify focused coverage**

Run:

```bash
cargo test monitor::runtime::tests::direct_scan_records_one_file_watcher_lockout_from_new_content -- --nocapture
cargo test logging::tests:: -- --nocapture
cargo test --lib
```

Expected: all pass. The OS may buffer all diagnostics; required proof is stream/sentinel completion, not deterministic socket eviction.

- [ ] **Step 7: Commit**

```bash
git add src/monitor/runtime.rs src/logging.rs
git commit -m "test: cover scanner and slow log client"
```

---

### Task 4: Serialized Unix Real-Binary E2E

**Files:**
- Create: `tests/streaming_stress.rs`

**Interfaces:**
- Consumes: `env!("CARGO_BIN_EXE_claudego")`, `env!("CARGO_BIN_EXE_stream-stress-child")`, isolated `HOME`/`TMPDIR`, and CLI `claudego -- claude -p --output-format stream-json no-stream-signal`.
- Produces: one `#[cfg(unix)]` test proving classifier routing, exact output, real notify delivery, logger socket handling, one watcher-origin lockout, successful exit, cleanup, and timeout reaping.

- [ ] **Step 1: Add local E2E helpers and failing test**

In `tests/streaming_stress.rs`, implement:

- RAII `TestDir` using `std::env::temp_dir()`, PID, and UNIX-epoch nanos.
- `expected_output()` rebuilding the helper's no-signal bytes exactly.
- `first_difference(actual, expected)` returning the first byte mismatch or shorter length.
- `wait_for_file` and `wait_for_log_text` bounded by five seconds.
- `kill_and_reap` and `wait_with_deadline` that always reap and preserve output.
- One `#[test]` under `#![cfg(unix)]`.

The test must:

1. Create isolated `home/.claude/projects/test`, `tmp`, and `bin`.
2. Symlink the helper to `bin/claude`; basename matters because `select_runner` intentionally accepts only `claude`/`claude.exe`.
3. Seed `session.jsonl` with a baseline row.
4. Launch real `claudego` with isolated `HOME`, `TMPDIR`, and `PATH=bin`, piping stdout/stderr.
5. Wait for `tmp/claudego.port`, connect a non-reading loopback client, and wait for log text `Event-driven file watcher active`.
6. Read stderr on a dedicated thread until the helper emits `READY\n`; retain all bytes.
7. Append one generated now-plus-two-hours rate-limit row to the watched session file.
8. Wait once, up to 15 seconds; on timeout kill, reap, and panic with status/stdout/stderr/log.
9. Assert success, exact stdout, one log occurrence of `[LOCKOUT DETECTED] Rate limit hit from file watcher.`, and removed port file.

Failure formatting must include first differing offset, actual/expected lengths, status, full stderr, and isolated log. No retry.

- [ ] **Step 2: Run red/diagnostic pass**

Run: `cargo test --test streaming_stress -- --nocapture --test-threads=1`

Expected before synchronization is complete: a single evidence-rich failure, never a hung/orphaned process. PTY-shaped output means the helper was not resolved as basename `claude`; missing watcher evidence means readiness/quiet-phase ordering is wrong.

- [ ] **Step 3: Lock synchronization to checked-in behavior**

Append only after both watcher-ready log text and helper `READY`. The helper's three-second quiet phase exceeds the current two-second output-hot threshold, allowing deferred scanning before child exit. If the checked-in constant differs at implementation time, set one concrete helper quiet duration equal to that threshold plus one-second margin; do not expose production constants or add configuration.

- [ ] **Step 4: Verify three independent runs**

Run separately:

```bash
cargo test --test streaming_stress -- --nocapture --test-threads=1
cargo test --test streaming_stress -- --nocapture --test-threads=1
cargo test --test streaming_stress -- --nocapture --test-threads=1
```

Expected: each run passes once; these are development confidence reruns, not retries in test code.

- [ ] **Step 5: Commit**

```bash
git add tests/streaming_stress.rs
git commit -m "test: add streaming stress end-to-end coverage"
```

---

### Task 5: Acceptance Gates and Backlog Closure

**Files:**
- Modify: `BACKLOG.md` only if its P1 checklist still matches the proven work.

**Interfaces:**
- Consumes: Tasks 1-4 and current lint configuration.
- Produces: green repository gates and evidence-backed P1 checkbox updates.

- [ ] **Step 1: Format and review scope**

Run:

```bash
cargo fmt
git diff --check
git diff -- src/bin/stream-stress-child.rs src/stream_json.rs src/monitor/runtime.rs src/logging.rs tests/streaming_stress.rs BACKLOG.md
```

Expected: clean whitespace and no dependency, public API, production flag, retry, benchmark, schema, resume, or log-format change.

- [ ] **Step 2: Run exact acceptance gates**

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: all exit zero with no warnings. Do not close backlog work if an unrelated baseline failure remains; capture its exact output instead.

- [ ] **Step 3: Update only proven P1 checkboxes**

Check only streaming-stress bullets directly proven by Tasks 1-4. Leave authenticated resume and unrelated profiling work untouched. Record the three gate commands, not throughput/timing claims.

- [ ] **Step 4: Verify and commit backlog closure**

```bash
git diff --check -- BACKLOG.md
git add BACKLOG.md
git commit -m "docs: close streaming stress coverage backlog"
```

Use required `commit_agent` review before committing. Do not stage this plan or the design spec.

## Self-Review Record

- Spec coverage: Tasks 1-4 cover fragmented/invalid/unknown/incomplete/overload pumping, parsing, spawned state effects, direct scanner fallback, logger bounds/slow client, and the real binary/watcher/logger path. Task 5 covers all exact gates.
- Scope: one cohesive subsystem; no split plan is needed.
- SOLID/KISS/YAGNI: no dependency, general harness, logger trait, fixture library, production flag, retry utility, or public API.
- Type consistency: names match the current checkout. New private seams are defined before consumers.
- Caveats: Cargo documents `CARGO_BIN_EXE_<name>` for integration tests; move the focused spawned-runner test to the integration target if the unit target cannot resolve it. Tokio documents that dropped children continue by default, so the E2E explicitly kills and awaits them.

