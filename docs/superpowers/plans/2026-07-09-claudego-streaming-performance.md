# Claudego Streaming Performance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `claudego` keep Claude output smooth by using PTY mode for interactive sessions and stdio `stream-json` mode for Claude print-mode sessions.

**Architecture:** Keep `CommandSpec` literal: no shell parsing and no wrapper-only command rewriting except the explicit stream-json resume command. `src/cli.rs` classifies commands, `src/app.rs` chooses a runner, `src/pty_bridge.rs` keeps the interactive PTY path raw, and new `src/stream_json.rs` owns stdio streaming plus cheap NDJSON signals. The monitor remains the fallback file watcher; it resumes through a small enum instead of knowing whether the child is PTY-backed or stream-json-backed.

**Tech Stack:** Rust 2021, `tokio`, `clap`, `portable-pty`, `crossterm`, `notify`, `serde_json::Value`, `chrono`, standard library atomics and channels.

## Global Constraints

- User-visible Claude output must always win over diagnostics, log previews, and background file scanning.
- Choose `StreamJsonPrint` when the command invokes Claude print mode with `--output-format stream-json`.
- Otherwise choose `PtyInteractive`.
- The classifier must preserve the existing literal `CommandSpec` behavior around `--`; no shell parsing.
- PTY output path: read PTY output and write to stdout immediately; update only a cheap activity marker; do not pretty-format, parse deeply, or scan files on that path.
- Stream JSON path: run Claude with piped stdout/stderr, pass stdout through unchanged, parse NDJSON lines in parallel for cheap signals, preserve raw output even if parsing fails.
- The first implementation should use `serde_json::Value` for the stream envelope. Replace with typed structs only if fixtures prove stable enough.
- In PTY mode, file watcher is primary rate-limit detection.
- In stream-json mode, parsed stream events are primary when they include the needed signal; file watcher is fallback for missing or changed stream fields.
- Logging must be bounded and lossy for diagnostics; never drop user-visible Claude output.
- No full SDK rewrite.
- No new runtime dependency unless fixtures prove stdio parsing is insufficient.
- No optional monitor bypass; running `claude` directly already bypasses `claudego`.
- Required final checks: `cargo fmt --check`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`.
- Verified external CLI contract: Claude Code documents `--include-partial-messages` as requiring `--print` and `--output-format stream-json`, and documents `--resume` for resuming a session.

---

## File Structure

- Modify `src/cli.rs`: own `CommandSpec`, runner classification, stream-json resume command construction, classifier tests.
- Modify `src/models.rs`: rename the output activity marker, add marker helpers, add `Default`.
- Modify `src/pty_bridge.rs`: keep the PTY path raw and switch to the shared output marker.
- Modify `src/app.rs`: initialize logger/state once, select runner, start monitor with the right resume target.
- Create `src/resume.rs`: define the minimal resume enum used by monitor/runtime.
- Create `src/stream_json.rs`: stdio runner, raw output pump, NDJSON parser, session/rate-limit extraction, stream tests.
- Modify `src/lib.rs`: export `resume` and `stream_json`.
- Modify `src/monitor/mod.rs`: accept `ResumeTarget` instead of `SharedPtyWriter`.
- Modify `src/monitor/runtime.rs`: defer scans on any hot output, apply lockouts from stream/file sources, call `ResumeTarget` on expiry.
- Modify `src/monitor/startup.rs`: clippy cleanup only.
- Modify `src/monitor/formatters.rs`: delete pretty JSON previews from diagnostics.
- Modify `src/logging.rs`: bound the live-log startup buffer and disconnect slow clients.

---

### Task 1: Baseline Gates Cleanup

**Files:**
- Modify: `src/bin/claudego-logs.rs`
- Modify: `src/app.rs`
- Modify: `src/lib.rs`
- Modify: `src/logging.rs`
- Modify: `src/models.rs`
- Modify: `src/monitor/events.rs`
- Modify: `src/monitor/formatters.rs`
- Modify: `src/monitor/helpers.rs`
- Modify: `src/monitor/lifecycle.rs`
- Modify: `src/monitor/mod.rs`
- Modify: `src/monitor/runtime.rs`
- Modify: `src/monitor/startup.rs`
- Modify: `src/paths.rs`
- Modify: `src/pty_bridge.rs`
- Modify: `src/main.rs`
- Test: existing unit tests in `src/lib.rs`

**Interfaces:**
- Consumes: existing `AppState::new() -> AppState`.
- Produces: `impl Default for AppState`, no behavior change.

- [ ] **Step 1: Run the current quality gates and capture the known failures**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Expected:

```text
cargo fmt --check: FAIL with formatting diffs
cargo clippy --all-targets --all-features -- -D warnings: FAIL with new_without_default, needless_borrow, unnecessary_map_or, needless_borrows_for_generic_args
cargo test: PASS, 6 tests
```

- [ ] **Step 2: Apply formatting**

Run:

```bash
cargo fmt
```

Expected: no stdout on success.

- [ ] **Step 3: Add `Default` for `AppState`**

In `src/models.rs`, keep `AppState::new()` and add:

```rust
impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 4: Fix clippy-only expressions**

In `src/monitor/startup.rs`, change:

```rust
match crate::watcher::scan::scan_content_for_any_limit(&content_to_scan) {
```

to:

```rust
match crate::watcher::scan::scan_content_for_any_limit(content_to_scan) {
```

In `src/monitor/startup.rs`, change:

```rust
if latest_limit
    .as_ref()
    .map_or(true, |l| limit.target_time > l.target_time)
{
    latest_limit = Some(limit);
}
```

to:

```rust
if latest_limit
    .as_ref()
    .is_none_or(|l| limit.target_time > l.target_time)
{
    latest_limit = Some(limit);
}
```

In `src/pty_bridge.rs`, change:

```rust
if let Ok(mut signals) = Signals::new(&[SIGWINCH]) {
```

to:

```rust
if let Ok(mut signals) = Signals::new([SIGWINCH]) {
```

- [ ] **Step 5: Verify cleanup**

Run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Expected:

```text
cargo fmt --check: PASS
cargo test: PASS, 6 tests
cargo clippy --all-targets --all-features -- -D warnings: PASS
```

- [ ] **Step 6: Commit**

```bash
git add src
git commit -m "chore: clean rust quality gates"
```

---

### Task 2: Command Classifier

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/main.rs`
- Test: `src/cli.rs`

**Interfaces:**
- Consumes: existing `CommandSpec { program: String, args: Vec<String> }`.
- Produces:
  - `CommandSpec::default_claude() -> CommandSpec`
  - `RunnerKind::{PtyInteractive, StreamJsonPrint}`
  - `select_runner(command: &CommandSpec) -> RunnerKind`
  - `stream_json_resume_command(session_id: &str) -> CommandSpec`

- [ ] **Step 1: Write the failing classifier tests**

Append to `src/cli.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::{select_runner, stream_json_resume_command, CommandSpec, RunnerKind};

    #[test]
    fn default_claude_uses_pty() {
        assert_eq!(
            select_runner(&CommandSpec::default_claude()),
            RunnerKind::PtyInteractive
        );
    }

    #[test]
    fn print_mode_stream_json_uses_stream_runner() {
        let command = CommandSpec {
            program: "claude".to_string(),
            args: vec![
                "-p".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--verbose".to_string(),
                "hello".to_string(),
            ],
        };

        assert_eq!(select_runner(&command), RunnerKind::StreamJsonPrint);
    }

    #[test]
    fn equals_form_stream_json_uses_stream_runner() {
        let command = CommandSpec {
            program: "claude".to_string(),
            args: vec![
                "--output-format=stream-json".to_string(),
                "--print".to_string(),
                "hello".to_string(),
            ],
        };

        assert_eq!(select_runner(&command), RunnerKind::StreamJsonPrint);
    }

    #[test]
    fn stream_json_without_print_stays_pty() {
        let command = CommandSpec {
            program: "claude".to_string(),
            args: vec![
                "--output-format".to_string(),
                "stream-json".to_string(),
                "hello".to_string(),
            ],
        };

        assert_eq!(select_runner(&command), RunnerKind::PtyInteractive);
    }

    #[test]
    fn shell_wrapped_claude_stays_literal() {
        let command = CommandSpec {
            program: "bash".to_string(),
            args: vec![
                "-lc".to_string(),
                "claude -p --output-format stream-json hello".to_string(),
            ],
        };

        assert_eq!(select_runner(&command), RunnerKind::PtyInteractive);
    }

    #[test]
    fn builds_minimal_stream_resume_command() {
        assert_eq!(
            stream_json_resume_command("abc123"),
            CommandSpec {
                program: "claude".to_string(),
                args: vec![
                    "--resume".to_string(),
                    "abc123".to_string(),
                    "-p".to_string(),
                    "--output-format".to_string(),
                    "stream-json".to_string(),
                    "--verbose".to_string(),
                    "--include-partial-messages".to_string(),
                    "continue".to_string(),
                ],
            }
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test cli::tests --lib
```

Expected:

```text
FAIL with unresolved imports for select_runner, stream_json_resume_command, RunnerKind
```

- [ ] **Step 3: Implement the classifier**

Replace `src/cli.rs` with:

```rust
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
}

impl CommandSpec {
    pub fn default_claude() -> Self {
        Self {
            program: "claude".to_string(),
            args: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunnerKind {
    PtyInteractive,
    StreamJsonPrint,
}

pub fn select_runner(command: &CommandSpec) -> RunnerKind {
    if is_claude_program(&command.program)
        && has_print_flag(&command.args)
        && has_stream_json_output(&command.args)
    {
        RunnerKind::StreamJsonPrint
    } else {
        RunnerKind::PtyInteractive
    }
}

pub fn stream_json_resume_command(session_id: &str) -> CommandSpec {
    CommandSpec {
        program: "claude".to_string(),
        args: vec![
            "--resume".to_string(),
            session_id.to_string(),
            "-p".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
            "--include-partial-messages".to_string(),
            "continue".to_string(),
        ],
    }
}

fn is_claude_program(program: &str) -> bool {
    Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "claude" || name == "claude.exe")
}

fn has_print_flag(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "-p" || arg == "--print")
}

fn has_stream_json_output(args: &[String]) -> bool {
    args.windows(2)
        .any(|pair| pair[0] == "--output-format" && pair[1] == "stream-json")
        || args
            .iter()
            .any(|arg| arg == "--output-format=stream-json")
}
```

Keep the test module from Step 1 below this implementation.

- [ ] **Step 4: Use the default constructor in `main.rs`**

In `src/main.rs`, replace:

```rust
CommandSpec {
    program: "claude".to_string(),
    args: vec![],
}
```

with:

```rust
CommandSpec::default_claude()
```

- [ ] **Step 5: Verify classifier**

Run:

```bash
cargo test cli::tests --lib
cargo fmt --check
```

Expected:

```text
cargo test cli::tests --lib: PASS, 6 tests
cargo fmt --check: PASS
```

- [ ] **Step 6: Commit**

```bash
git add src/cli.rs src/main.rs
git commit -m "feat: classify stream-json claude commands"
```

---

### Task 3: Shared Output Activity Marker

**Files:**
- Modify: `src/models.rs`
- Modify: `src/pty_bridge.rs`
- Modify: `src/monitor/runtime.rs`
- Test: `src/models.rs`

**Interfaces:**
- Consumes: `AppState::new() -> AppState`.
- Produces:
  - `pub type OutputActivity = Arc<AtomicU64>`
  - `mark_output_activity(activity: &AtomicU64)`
  - `output_is_hot(activity: &AtomicU64, threshold: Duration) -> bool`
  - `AppState.last_output_activity: OutputActivity`

- [ ] **Step 1: Write failing marker tests**

Append to `src/models.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::{mark_output_activity, output_is_hot, AppState};
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    #[test]
    fn new_state_has_cold_output_activity() {
        let state = AppState::new();

        assert_eq!(state.last_output_activity.load(Ordering::Relaxed), 0);
        assert!(!output_is_hot(
            &state.last_output_activity,
            Duration::from_secs(2)
        ));
    }

    #[test]
    fn marking_activity_makes_output_hot() {
        let state = AppState::new();

        mark_output_activity(&state.last_output_activity);

        assert!(output_is_hot(
            &state.last_output_activity,
            Duration::from_secs(2)
        ));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test models::tests --lib
```

Expected:

```text
FAIL with unknown field last_output_activity and unresolved imports mark_output_activity, output_is_hot
```

- [ ] **Step 3: Implement marker helpers**

In `src/models.rs`, replace `last_pty_activity` with:

```rust
pub type OutputActivity = Arc<AtomicU64>;

pub struct AppState {
    pub lockout_target_time: Option<chrono::DateTime<chrono::Local>>,
    pub file_size_cache: HashMap<PathBuf, u64>,
    pub last_output_activity: OutputActivity,
}
```

Update `AppState::new()`:

```rust
pub fn new() -> Self {
    Self {
        lockout_target_time: None,
        file_size_cache: HashMap::new(),
        last_output_activity: Arc::new(AtomicU64::new(0)),
    }
}
```

Add helpers below `impl Default for AppState`:

```rust
pub fn mark_output_activity(activity: &AtomicU64) {
    let now_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    activity.store(now_nanos, std::sync::atomic::Ordering::Relaxed);
}

pub fn output_is_hot(activity: &AtomicU64, threshold: std::time::Duration) -> bool {
    let last_activity_ns = activity.load(std::sync::atomic::Ordering::Relaxed);
    if last_activity_ns == 0 {
        return false;
    }

    let now_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    std::time::Duration::from_nanos(now_ns.saturating_sub(last_activity_ns)) < threshold
}
```

- [ ] **Step 4: Update PTY reader**

In `src/pty_bridge.rs`, remove:

```rust
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};
```

Add:

```rust
use crate::models::mark_output_activity;
```

In `spawn_output_reader`, replace:

```rust
let activity_tracker = state.lock().unwrap().last_pty_activity.clone();
```

with:

```rust
let activity_tracker = state.lock().unwrap().last_output_activity.clone();
```

Replace the timestamp block inside the read loop with:

```rust
mark_output_activity(&activity_tracker);
```

Keep `stdout.write_all()` and `stdout.flush()` in the loop.

- [ ] **Step 5: Update monitor deferral**

In `src/monitor/runtime.rs`, remove:

```rust
use std::sync::atomic::Ordering;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
```

Use:

```rust
use crate::models::{output_is_hot, SharedAppState};
use std::time::Instant;
```

Replace the busy-loop check with:

```rust
let activity_tracker = {
    let app = state.lock().unwrap();
    app.last_output_activity.clone()
};
loop {
    if output_is_hot(&activity_tracker, PTY_BUSY_THRESHOLD) {
        log_to_file(&format!(
            "Claude is currently streaming output. Deferring file scan for {:?}.",
            DEFER_SCAN_INTERVAL,
        ));
        sleep(DEFER_SCAN_INTERVAL).await;
    } else {
        break;
    }
}
```

- [ ] **Step 6: Verify marker rename**

Run:

```bash
cargo test models::tests --lib
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Expected:

```text
cargo test models::tests --lib: PASS, 2 tests
cargo test: PASS
cargo clippy --all-targets --all-features -- -D warnings: PASS
```

- [ ] **Step 7: Commit**

```bash
git add src/models.rs src/pty_bridge.rs src/monitor/runtime.rs
git commit -m "refactor: share output activity tracking"
```

---

### Task 4: Resume Target For Monitor

**Files:**
- Create: `src/resume.rs`
- Modify: `src/lib.rs`
- Modify: `src/monitor/mod.rs`
- Modify: `src/monitor/runtime.rs`
- Modify: `src/app.rs`
- Test: `src/resume.rs`

**Interfaces:**
- Consumes: `SharedPtyWriter`, `SharedAppState`.
- Produces:
  - `StreamResumeCommand::Continue`
  - `ResumeTarget::{Pty(SharedPtyWriter), StreamJson(UnboundedSender<StreamResumeCommand>)}`
  - `ResumeTarget::resume(&self) -> Result<(), String>`
  - `monitor::spawn_lockout_monitor(state: SharedAppState, resume_target: ResumeTarget)`

- [ ] **Step 1: Write failing resume tests**

Create `src/resume.rs` with:

```rust
use crate::pty_bridge::SharedPtyWriter;
use tokio::sync::mpsc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamResumeCommand {
    Continue,
}

#[derive(Clone)]
pub enum ResumeTarget {
    Pty(SharedPtyWriter),
    StreamJson(mpsc::UnboundedSender<StreamResumeCommand>),
}

impl ResumeTarget {
    pub fn resume(&self) -> Result<(), String> {
        Err("resume not wired".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{ResumeTarget, StreamResumeCommand};
    use crate::pty_bridge::SharedPtyWriter;
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    #[derive(Default)]
    struct MemoryWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for MemoryWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.bytes.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn pty_resume_writes_continue_with_carriage_return() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let writer = MemoryWriter {
            bytes: Arc::clone(&bytes),
        };
        let writer: SharedPtyWriter = Arc::new(Mutex::new(Box::new(writer)));

        ResumeTarget::Pty(writer).resume().expect("resume succeeds");

        assert_eq!(&*bytes.lock().unwrap(), b"continue\r");
    }

    #[tokio::test]
    async fn stream_resume_sends_continue_command() {
        let (tx, mut rx) = mpsc::unbounded_channel();

        ResumeTarget::StreamJson(tx).resume().expect("resume succeeds");

        assert_eq!(rx.recv().await, Some(StreamResumeCommand::Continue));
    }
}
```

- [ ] **Step 2: Export `resume` and verify tests fail**

Add to `src/lib.rs`:

```rust
pub mod resume;
```

Run:

```bash
cargo test resume::tests --lib
```

Expected:

```text
FAIL with "resume not wired"
```

- [ ] **Step 3: Implement `ResumeTarget::resume`**

In `src/resume.rs`, replace:

```rust
impl ResumeTarget {
    pub fn resume(&self) -> Result<(), String> {
        Err("resume not wired".to_string())
    }
}
```

with:

```rust
impl ResumeTarget {
    pub fn resume(&self) -> Result<(), String> {
        match self {
            ResumeTarget::Pty(writer) => {
                let mut writer = writer.lock().unwrap_or_else(|e| e.into_inner());
                writer
                    .write_all(b"continue\r")
                    .map_err(|e| format!("failed to write PTY continue command: {e}"))?;
                writer
                    .flush()
                    .map_err(|e| format!("failed to flush PTY continue command: {e}"))
            }
            ResumeTarget::StreamJson(tx) => tx
                .send(StreamResumeCommand::Continue)
                .map_err(|_| "stream-json runner is no longer available".to_string()),
        }
    }
}
```

- [ ] **Step 4: Verify resume targets**

Run:

```bash
cargo test resume::tests --lib
```

Expected:

```text
PASS, 2 tests
```

- [ ] **Step 5: Make monitor accept `ResumeTarget`**

In `src/monitor/mod.rs`, replace:

```rust
use crate::pty_bridge::SharedPtyWriter;
```

with:

```rust
use crate::resume::ResumeTarget;
```

Change:

```rust
pub fn spawn_lockout_monitor(state: SharedAppState, writer: SharedPtyWriter) {
```

to:

```rust
pub fn spawn_lockout_monitor(state: SharedAppState, resume_target: ResumeTarget) {
```

Change the expiry call:

```rust
runtime::handle_expiry(&state, &writer, target);
```

to:

```rust
runtime::handle_expiry(&state, &resume_target, target);
```

- [ ] **Step 6: Make runtime use `ResumeTarget`**

In `src/monitor/runtime.rs`, replace:

```rust
use crate::pty_bridge::SharedPtyWriter;
```

with:

```rust
use crate::resume::ResumeTarget;
```

Replace `handle_expiry` with:

```rust
pub(super) fn handle_expiry(
    state: &SharedAppState,
    resume_target: &ResumeTarget,
    expired_target: DateTime<Local>,
) {
    log_to_file("[Trigger] Reset time reached. Resuming Claude session.");

    match resume_target.resume() {
        Ok(()) => log_to_file("[System] Resume command sent."),
        Err(error) => log_to_file(&format!("[Resume Error] {error}")),
    }

    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    if s.lockout_target_time == Some(expired_target) {
        s.lockout_target_time = None;
        s.file_size_cache.clear();
        log_to_file("[System] Resuming passive file monitoring.");
    } else {
        log_to_file("[System] Expiry handled, but a newer lockout has already been detected. State not cleared.");
    }
}
```

- [ ] **Step 7: Keep app PTY behavior unchanged**

In `src/app.rs`, add:

```rust
use crate::resume::ResumeTarget;
```

Change:

```rust
monitor::spawn_lockout_monitor(state, Arc::clone(&session.writer));
```

to:

```rust
monitor::spawn_lockout_monitor(state, ResumeTarget::Pty(Arc::clone(&session.writer)));
```

- [ ] **Step 8: Verify PTY behavior still passes tests**

Run:

```bash
cargo test resume::tests --lib
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Expected:

```text
cargo test resume::tests --lib: PASS, 2 tests
cargo test: PASS
cargo clippy --all-targets --all-features -- -D warnings: PASS
```

- [ ] **Step 9: Commit**

```bash
git add src/resume.rs src/lib.rs src/monitor/mod.rs src/monitor/runtime.rs src/app.rs
git commit -m "refactor: route monitor resumes through target enum"
```

---

### Task 5: Stream JSON Signal Parser

**Files:**
- Create: `src/stream_json.rs`
- Modify: `src/lib.rs`
- Modify: `src/watcher/mod.rs`
- Modify: `src/watcher/scan.rs`
- Test: `src/stream_json.rs`

**Interfaces:**
- Consumes: `watcher::reset_time::parse_reset_time(log_time, content_text)`.
- Produces:
  - `StreamJsonSignal { session_id: Option<String>, rate_limit: Option<ActiveRateLimitInfo> }`
  - `StreamLineResult::{Signal(StreamJsonSignal), Ignored, InvalidJson}`
  - `parse_stream_line(line: &str) -> StreamLineResult`
  - `watcher::scan::active_rate_limit_from_message(log_time, content_text) -> Option<ActiveRateLimitInfo>`

- [ ] **Step 1: Expose the reset parser inside the crate**

In `src/watcher/mod.rs`, change:

```rust
mod reset_time;
```

to:

```rust
pub(crate) mod reset_time;
```

In `src/watcher/scan.rs`, add:

```rust
pub(crate) fn active_rate_limit_from_message(
    log_time: DateTime<Local>,
    content_text: &str,
) -> Option<ActiveRateLimitInfo> {
    let (target_time, display_str) = reset_time::parse_reset_time(log_time, content_text)?;
    if Local::now() > target_time {
        return None;
    }

    Some(ActiveRateLimitInfo {
        target_time,
        display_str,
        raw_message: content_text.to_string(),
    })
}
```

Then replace the duplicated construction in `parse_rate_limit_line` with:

```rust
let Some(limit) = active_rate_limit_from_message(log_time, content_text) else {
    return RateLimitLine::Stale;
};

RateLimitLine::Active(limit)
```

- [ ] **Step 2: Write failing parser tests**

Create `src/stream_json.rs` with:

```rust
use crate::watcher::scan::ActiveRateLimitInfo;

#[derive(Debug)]
pub struct StreamJsonSignal {
    pub session_id: Option<String>,
    pub rate_limit: Option<ActiveRateLimitInfo>,
}

#[derive(Debug)]
pub enum StreamLineResult {
    Signal(StreamJsonSignal),
    Ignored,
    InvalidJson,
}

pub fn parse_stream_line(line: &str) -> StreamLineResult {
    if serde_json::from_str::<serde_json::Value>(line).is_err() {
        StreamLineResult::InvalidJson
    } else {
        StreamLineResult::Ignored
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_stream_line, StreamLineResult};

    #[test]
    fn invalid_json_is_reported_without_failing_raw_output() {
        assert!(matches!(
            parse_stream_line("not json"),
            StreamLineResult::InvalidJson
        ));
    }

    #[test]
    fn extracts_session_id_from_stream_event() {
        let result = parse_stream_line(
            r#"{"type":"system","session_id":"11111111-1111-1111-1111-111111111111"}"#,
        );

        let StreamLineResult::Signal(signal) = result else {
            panic!("expected signal");
        };
        assert_eq!(
            signal.session_id.as_deref(),
            Some("11111111-1111-1111-1111-111111111111")
        );
        assert!(signal.rate_limit.is_none());
    }

    #[test]
    fn extracts_rate_limit_from_nested_stream_event() {
        let result = parse_stream_line(
            r#"{"type":"error","timestamp":"2099-07-09T10:00:00-04:00","error":"rate_limit","message":{"content":[{"type":"text","text":"Claude limit reached; resets 5:30pm"}]}}"#,
        );

        let StreamLineResult::Signal(signal) = result else {
            panic!("expected signal");
        };
        let limit = signal.rate_limit.expect("rate limit");
        assert_eq!(limit.display_str, "5:30pm");
        assert_eq!(limit.raw_message, "Claude limit reached; resets 5:30pm");
    }

    #[test]
    fn ignores_unknown_json_shape() {
        assert!(matches!(
            parse_stream_line(r#"{"type":"assistant","message":{"content":[]}}"#),
            StreamLineResult::Ignored
        ));
    }
}
```

- [ ] **Step 3: Export `stream_json`**

Add to `src/lib.rs`:

```rust
pub mod stream_json;
```

- [ ] **Step 4: Run parser tests**

Run:

```bash
cargo test stream_json::tests --lib
```

Expected:

```text
FAIL with missing session/rate-limit signals
```

- [ ] **Step 5: Implement parser**

In `src/stream_json.rs`, add imports:

```rust
use crate::watcher::scan::active_rate_limit_from_message;
use chrono::{DateTime, Local};
use serde_json::Value;
```

Replace `parse_stream_line` with:

```rust
pub fn parse_stream_line(line: &str) -> StreamLineResult {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return StreamLineResult::InvalidJson;
    };

    let session_id = find_string_field(&value, &["session_id", "sessionId"]);
    let rate_limit = extract_rate_limit(&value);

    if session_id.is_some() || rate_limit.is_some() {
        StreamLineResult::Signal(StreamJsonSignal {
            session_id,
            rate_limit,
        })
    } else {
        StreamLineResult::Ignored
    }
}
```

Add below `parse_stream_line`:

```rust
fn extract_rate_limit(value: &Value) -> Option<ActiveRateLimitInfo> {
    let timestamp = find_string_field(value, &["timestamp"])?;
    let log_time = DateTime::parse_from_rfc3339(&timestamp)
        .ok()?
        .with_timezone(&Local);
    let message = find_rate_limit_text(value)?;

    active_rate_limit_from_message(log_time, &message)
}

fn find_rate_limit_text(value: &Value) -> Option<String> {
    find_string_value(value, &|text| {
        let lower = text.to_ascii_lowercase();
        (lower.contains("rate_limit") || lower.contains("limit"))
            && lower.contains("reset")
    })
}

fn find_string_field(value: &Value, names: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for name in names {
                if let Some(Value::String(text)) = map.get(*name) {
                    return Some(text.clone());
                }
            }
            map.values().find_map(|child| find_string_field(child, names))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|child| find_string_field(child, names)),
        _ => None,
    }
}

fn find_string_value(value: &Value, matches: &dyn Fn(&str) -> bool) -> Option<String> {
    match value {
        Value::String(text) if matches(text) => Some(text.clone()),
        Value::Array(items) => items
            .iter()
            .find_map(|child| find_string_value(child, matches)),
        Value::Object(map) => map
            .values()
            .find_map(|child| find_string_value(child, matches)),
        _ => None,
    }
}
```

- [ ] **Step 6: Verify parser**

Run:

```bash
cargo test stream_json::tests --lib
```

Expected:

```text
PASS, 4 tests
```

- [ ] **Step 7: Verify existing watcher parser still works**

Run:

```bash
cargo test watcher:: --lib
cargo clippy --all-targets --all-features -- -D warnings
```

Expected:

```text
cargo test watcher:: --lib: PASS
cargo clippy --all-targets --all-features -- -D warnings: PASS
```

- [ ] **Step 8: Commit**

```bash
git add src/stream_json.rs src/lib.rs src/watcher/mod.rs src/watcher/scan.rs
git commit -m "feat: parse stream-json session signals"
```

---

### Task 6: Stream JSON Runner And App Wiring

**Files:**
- Modify: `src/stream_json.rs`
- Modify: `src/app.rs`
- Modify: `src/lib.rs`
- Test: `src/stream_json.rs`

**Interfaces:**
- Consumes:
  - `select_runner(command: &CommandSpec) -> RunnerKind`
  - `stream_json_resume_command(session_id: &str) -> CommandSpec`
  - `ResumeTarget::StreamJson(UnboundedSender<StreamResumeCommand>)`
  - `parse_stream_line(line: &str) -> StreamLineResult`
- Produces:
  - `run_stream_json_print(command: CommandSpec, state: SharedAppState, resume_rx: UnboundedReceiver<StreamResumeCommand>) -> Result<()>`
  - `pump_raw_output(reader, writer, line_tx, activity) -> io::Result<()>`

- [ ] **Step 1: Add raw-output pump tests**

Append to the `#[cfg(test)]` module in `src/stream_json.rs`:

```rust
use crate::models::{output_is_hot, AppState};
use std::time::Duration;
use tokio::sync::mpsc;

#[tokio::test]
async fn pump_preserves_raw_bytes_and_emits_complete_lines() {
    let state = AppState::new();
    let (line_tx, mut line_rx) = mpsc::channel(8);
    let input = b"{\"type\":\"one\"}\n{\"type\":\"two\"}\n".as_slice();
    let mut output = Vec::new();

    pump_raw_output(input, &mut output, line_tx, state.last_output_activity.clone())
        .await
        .expect("pump succeeds");

    assert_eq!(output, b"{\"type\":\"one\"}\n{\"type\":\"two\"}\n");
    assert_eq!(line_rx.recv().await.as_deref(), Some("{\"type\":\"one\"}"));
    assert_eq!(line_rx.recv().await.as_deref(), Some("{\"type\":\"two\"}"));
    assert!(output_is_hot(
        &state.last_output_activity,
        Duration::from_secs(2)
    ));
}

#[tokio::test]
async fn pump_keeps_incomplete_tail_raw_without_parsing_it() {
    let state = AppState::new();
    let (line_tx, mut line_rx) = mpsc::channel(8);
    let input = b"{\"partial\":true}".as_slice();
    let mut output = Vec::new();

    pump_raw_output(input, &mut output, line_tx, state.last_output_activity.clone())
        .await
        .expect("pump succeeds");

    assert_eq!(output, b"{\"partial\":true}");
    assert!(line_rx.try_recv().is_err());
}
```

- [ ] **Step 2: Run pump tests to verify they fail**

Run:

```bash
cargo test stream_json::tests::pump_ --lib
```

Expected:

```text
FAIL with unresolved function pump_raw_output
```

- [ ] **Step 3: Implement raw-output pump**

Add to `src/stream_json.rs` imports:

```rust
use crate::cli::{stream_json_resume_command, CommandSpec};
use crate::logging::log_to_file;
use crate::models::{mark_output_activity, OutputActivity, SharedAppState};
use crate::resume::StreamResumeCommand;
use anyhow::Result;
use std::io;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};
```

Add:

```rust
pub(crate) async fn pump_raw_output<R, W>(
    mut reader: R,
    mut writer: W,
    line_tx: mpsc::Sender<String>,
    activity: OutputActivity,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = [0_u8; 8192];
    let mut pending = Vec::new();

    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }

        writer.write_all(&buf[..n]).await?;
        writer.flush().await?;
        mark_output_activity(&activity);

        pending.extend_from_slice(&buf[..n]);
        while let Some(newline) = pending.iter().position(|byte| *byte == b'\n') {
            let line_bytes: Vec<u8> = pending.drain(..=newline).collect();
            if let Ok(line) = String::from_utf8(line_bytes) {
                let line = line.trim_end_matches(['\r', '\n']).to_string();
                let _ = line_tx.try_send(line);
            }
        }
    }
}
```

- [ ] **Step 4: Add process loop**

Add below `pump_raw_output` in `src/stream_json.rs`:

```rust
pub async fn run_stream_json_print(
    command: CommandSpec,
    state: SharedAppState,
    mut resume_rx: mpsc::UnboundedReceiver<StreamResumeCommand>,
) -> Result<()> {
    let latest_session_id = Arc::new(Mutex::new(None::<String>));
    let mut next_command = command;

    loop {
        let exit_after_child = run_one_stream_process(
            next_command.clone(),
            Arc::clone(&state),
            Arc::clone(&latest_session_id),
            &mut resume_rx,
        )
        .await?;

        if exit_after_child {
            return Ok(());
        }

        let Some(session_id) = latest_session_id
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        else {
            log_to_file("[Stream JSON] Cannot resume: no session id was observed in stream output.");
            return Ok(());
        };

        next_command = stream_json_resume_command(&session_id);
    }
}

async fn run_one_stream_process(
    command: CommandSpec,
    state: SharedAppState,
    latest_session_id: Arc<Mutex<Option<String>>>,
    resume_rx: &mut mpsc::UnboundedReceiver<StreamResumeCommand>,
) -> Result<bool> {
    let mut child = Command::new(&command.program)
        .args(&command.args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let stdin = child.stdin.take();
    let activity = state.lock().unwrap().last_output_activity.clone();

    let (line_tx, line_rx) = mpsc::channel(1024);
    let stdout_task = tokio::spawn(pump_raw_output(
        stdout,
        tokio::io::stdout(),
        line_tx,
        activity.clone(),
    ));
    let stderr_task = tokio::spawn(pump_raw_output(
        stderr,
        tokio::io::stderr(),
        mpsc::channel(1).0,
        activity,
    ));
    let parser_task = tokio::spawn(parse_stream_lines(
        line_rx,
        Arc::clone(&state),
        latest_session_id,
    ));

    let mut stdin = stdin;
    let child_exited_during_lockout = loop {
        while let Ok(StreamResumeCommand::Continue) = resume_rx.try_recv() {
            if let Some(stdin) = stdin.as_mut() {
                if stdin.write_all(b"{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"continue\"}]}}\n").await.is_ok()
                    && stdin.flush().await.is_ok()
                {
                    log_to_file("[Stream JSON] Sent stream-json continue input to live process.");
                    continue;
                }
            }

            log_to_file("[Stream JSON] Live continue input unavailable; restarting with --resume when session id is available.");
            child.start_kill()?;
            let _ = child.wait().await;
            break true;
        }

        match timeout(Duration::from_millis(100), child.wait()).await {
            Ok(status) => {
                let status = status?;
                log_to_file(&format!("[Stream JSON] Claude exited with status {status}."));
                break state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .lockout_target_time
                    .is_some();
            }
            Err(_) => {}
        }
    };

    let _ = stdout_task.await;
    let _ = stderr_task.await;
    let _ = parser_task.await;

    Ok(!child_exited_during_lockout)
}

async fn parse_stream_lines(
    mut line_rx: mpsc::Receiver<String>,
    state: SharedAppState,
    latest_session_id: Arc<Mutex<Option<String>>>,
) {
    while let Some(line) = line_rx.recv().await {
        match parse_stream_line(&line) {
            StreamLineResult::Signal(signal) => {
                if let Some(session_id) = signal.session_id {
                    *latest_session_id.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(session_id);
                }
                if let Some(limit) = signal.rate_limit {
                    crate::monitor::record_lockout(&state, limit, "stream-json");
                }
            }
            StreamLineResult::InvalidJson => {
                log_to_file("[Stream JSON] Invalid NDJSON line; raw output was preserved.");
            }
            StreamLineResult::Ignored => {}
        }
    }
}
```

- [ ] **Step 5: Add monitor lockout recorder**

In `src/monitor/mod.rs`, add:

```rust
pub fn record_lockout(
    state: &SharedAppState,
    limit_info: crate::watcher::scan::ActiveRateLimitInfo,
    source: &str,
) {
    runtime::record_lockout(state, limit_info, source);
}
```

In `src/monitor/runtime.rs`, add:

```rust
pub(super) fn record_lockout(
    state: &SharedAppState,
    limit_info: crate::watcher::scan::ActiveRateLimitInfo,
    source: &str,
) {
    log_to_file(&format!(
        "[LOCKOUT DETECTED] Rate limit hit from {source}. Target: {}",
        limit_info.display_str
    ));
    let mut app = state.lock().unwrap_or_else(|e| e.into_inner());
    app.lockout_target_time = Some(limit_info.target_time);
}
```

Then replace the file-scan duplicate lockout assignment in `scan_and_update_state` with:

```rust
record_lockout(state, limit_info, "file watcher");
*next_log_time = Instant::now();
```

- [ ] **Step 6: Wire app selection**

In `src/app.rs`, add:

```rust
use crate::cli::{select_runner, RunnerKind};
use crate::resume::StreamResumeCommand;
use crate::stream_json;
use tokio::sync::mpsc;
```

Replace the PTY-only runner block with:

```rust
match select_runner(&command_spec) {
    RunnerKind::PtyInteractive => {
        let mut session = pty_bridge::spawn_command_in_pty(command_spec)?;
        let _guard = RawModeGuard::init()?;

        pty_bridge::spawn_output_reader(session.reader, Arc::clone(&state));
        pty_bridge::spawn_input_writer(Arc::clone(&session.writer));
        pty_bridge::spawn_resize_poller(session.master, session.initial_size);
        monitor::spawn_lockout_monitor(state, ResumeTarget::Pty(Arc::clone(&session.writer)));

        let child_wait_handle = tokio::task::spawn_blocking(move || session.child.wait());
        let _ = child_wait_handle.await??;
    }
    RunnerKind::StreamJsonPrint => {
        let (resume_tx, resume_rx) = mpsc::unbounded_channel::<StreamResumeCommand>();
        monitor::spawn_lockout_monitor(state.clone(), ResumeTarget::StreamJson(resume_tx));
        stream_json::run_stream_json_print(command_spec, state, resume_rx).await?;
    }
}
```

- [ ] **Step 7: Verify stream module and app compile**

Run:

```bash
cargo test stream_json::tests --lib
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Expected:

```text
cargo test stream_json::tests --lib: PASS, 6 tests
cargo test: PASS
cargo clippy --all-targets --all-features -- -D warnings: PASS
```

- [ ] **Step 8: Commit**

```bash
git add src/stream_json.rs src/app.rs src/monitor/mod.rs src/monitor/runtime.rs src/lib.rs
git commit -m "feat: run claude stream-json through stdio"
```

---

### Task 7: Bounded Diagnostic Logging

**Files:**
- Modify: `src/logging.rs`
- Modify: `src/monitor/formatters.rs`
- Test: `src/logging.rs`
- Test: `src/monitor/formatters.rs`

**Interfaces:**
- Consumes: `log_to_file(msg: &str)`, `create_content_preview(new_content: &str) -> String`.
- Produces:
  - bounded startup live-log buffer
  - timeout-protected live client writes
  - raw, bounded preview lines without pretty JSON

- [ ] **Step 1: Write failing logging tests**

Append to `src/logging.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::{push_startup_line, write_with_timeout};
    use std::collections::VecDeque;
    use std::time::Duration;

    #[test]
    fn startup_buffer_keeps_newest_lines_only() {
        let mut buffer = VecDeque::new();

        for i in 0..5 {
            push_startup_line(&mut buffer, format!("line {i}\n"), 3);
        }

        assert_eq!(
            buffer.into_iter().collect::<Vec<_>>(),
            vec![
                "line 2\n".to_string(),
                "line 3\n".to_string(),
                "line 4\n".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn slow_writer_times_out() {
        let (mut writer, _reader) = tokio::io::duplex(1);
        let bytes = vec![b'x'; 1024 * 1024];

        assert!(!write_with_timeout(&mut writer, &bytes, Duration::from_millis(1)).await);
    }
}
```

Append to `src/monitor/formatters.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::create_content_preview;

    #[test]
    fn preview_keeps_json_compact() {
        let preview = create_content_preview(r#"{"type":"event","nested":{"value":1}}"#);

        assert_eq!(
            preview,
            "    > {\"type\":\"event\",\"nested\":{\"value\":1}}\n"
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test logging::tests --lib
cargo test monitor::formatters::tests --lib
```

Expected:

```text
cargo test logging::tests --lib: FAIL with unresolved push_startup_line and write_with_timeout
cargo test monitor::formatters::tests --lib: FAIL while JSON is pretty-printed
```

- [ ] **Step 3: Bound startup buffer and client writes**

In `src/logging.rs`, add imports:

```rust
use std::collections::VecDeque;
use std::time::Duration;
use tokio::io::AsyncWrite;
use tokio::time::timeout;
```

Add constants:

```rust
const MAX_STARTUP_BUFFER_LINES: usize = 500;
const CLIENT_WRITE_TIMEOUT: Duration = Duration::from_millis(100);
```

Add helpers:

```rust
fn push_startup_line(buffer: &mut VecDeque<String>, line: String, max_lines: usize) {
    if buffer.len() == max_lines {
        buffer.pop_front();
    }
    buffer.push_back(line);
}

async fn write_with_timeout<W>(writer: &mut W, bytes: &[u8], duration: Duration) -> bool
where
    W: AsyncWrite + Unpin,
{
    timeout(duration, writer.write_all(bytes))
        .await
        .is_ok_and(|result| result.is_ok())
}
```

Change:

```rust
let mut initial_buffer: Vec<String> = Vec::new();
```

to:

```rust
let mut initial_buffer: VecDeque<String> = VecDeque::new();
```

Change startup buffering:

```rust
initial_buffer.push(line.clone());
```

to:

```rust
push_startup_line(&mut initial_buffer, line.clone(), MAX_STARTUP_BUFFER_LINES);
```

Change client writes:

```rust
if client.write_all(line.as_bytes()).await.is_err() {
    dead_clients.push(i);
}
```

to:

```rust
if !write_with_timeout(client, line.as_bytes(), CLIENT_WRITE_TIMEOUT).await {
    dead_clients.push(i);
}
```

Change first-client buffer writes:

```rust
if stream.write_all(line.as_bytes()).await.is_err() {
    client_ok = false;
    break;
}
```

to:

```rust
if !write_with_timeout(&mut stream, line.as_bytes(), CLIENT_WRITE_TIMEOUT).await {
    client_ok = false;
    break;
}
```

- [ ] **Step 4: Delete pretty JSON previews**

In `src/monitor/formatters.rs`, remove:

```rust
use serde_json;
```

Replace the loop body with:

```rust
for line in lines.iter().take(MAX_LINES_TO_LOG) {
    let trimmed_line = line.trim();
    if trimmed_line.is_empty() {
        continue;
    }

    writeln!(writer, "    > {}", line.trim_end())?;
}
```

- [ ] **Step 5: Verify diagnostic logging**

Run:

```bash
cargo test logging::tests --lib
cargo test monitor::formatters::tests --lib
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Expected:

```text
cargo test logging::tests --lib: PASS, 2 tests
cargo test monitor::formatters::tests --lib: PASS, 1 test
cargo test: PASS
cargo clippy --all-targets --all-features -- -D warnings: PASS
```

- [ ] **Step 6: Commit**

```bash
git add src/logging.rs src/monitor/formatters.rs
git commit -m "fix: bound diagnostic logging"
```

---

### Task 8: Synthetic Runner Regression Tests

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/stream_json.rs`
- Modify: `src/monitor/runtime.rs`
- Test: `src/cli.rs`
- Test: `src/stream_json.rs`
- Test: `src/monitor/runtime.rs`

**Interfaces:**
- Consumes: all interfaces from Tasks 2-7.
- Produces: regression coverage for classifier, stream parsing, fallback lockout recording, and output hot deferral.

- [ ] **Step 1: Add monitor lockout unit test**

Append to `src/monitor/runtime.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::record_lockout;
    use crate::models::AppState;
    use crate::watcher::scan::ActiveRateLimitInfo;
    use chrono::{Duration, Local};
    use std::sync::{Arc, Mutex};

    #[test]
    fn stream_lockout_updates_shared_state() {
        let state = Arc::new(Mutex::new(AppState::new()));
        let target = Local::now() + Duration::minutes(30);

        record_lockout(
            &state,
            ActiveRateLimitInfo {
                target_time: target,
                display_str: "5:30pm".to_string(),
                raw_message: "Claude limit reached; resets 5:30pm".to_string(),
            },
            "stream-json",
        );

        assert_eq!(
            state.lock().unwrap().lockout_target_time,
            Some(target)
        );
    }
}
```

- [ ] **Step 2: Add stream parser pressure test**

Append to `src/stream_json.rs` tests:

```rust
#[tokio::test]
async fn pump_never_blocks_when_line_channel_is_full() {
    let state = AppState::new();
    let (line_tx, _line_rx) = mpsc::channel(1);
    line_tx.try_send("{\"filled\":true}".to_string()).unwrap();
    let input = b"{\"type\":\"one\"}\n{\"type\":\"two\"}\n".as_slice();
    let mut output = Vec::new();

    let result = tokio::time::timeout(
        Duration::from_secs(1),
        pump_raw_output(input, &mut output, line_tx, state.last_output_activity.clone()),
    )
    .await;

    assert!(result.is_ok());
    assert_eq!(output, b"{\"type\":\"one\"}\n{\"type\":\"two\"}\n");
}
```

- [ ] **Step 3: Run targeted regressions**

Run:

```bash
cargo test cli::tests --lib
cargo test stream_json::tests --lib
cargo test monitor::runtime::tests --lib
```

Expected:

```text
cargo test cli::tests --lib: PASS
cargo test stream_json::tests --lib: PASS
cargo test monitor::runtime::tests --lib: PASS
```

- [ ] **Step 4: Run full local verification**

Run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build
```

Expected:

```text
cargo fmt --check: PASS
cargo test: PASS
cargo clippy --all-targets --all-features -- -D warnings: PASS
cargo build: PASS
```

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs src/stream_json.rs src/monitor/runtime.rs
git commit -m "test: cover streaming runner regressions"
```

---

### Task 9: Real Claude Stream Verification Gate

**Files:**
- Create: `docs/superpowers/fixtures/claude-stream-json-normal.ndjson`
- Create: `docs/superpowers/fixtures/claude-stream-json-rate-limit.ndjson`
- Create: `docs/superpowers/fixtures/claude-stream-json-session.ndjson`
- Modify: `docs/superpowers/specs/2026-07-09-claudego-streaming-performance-design.md`

**Interfaces:**
- Consumes: real Claude Code CLI output.
- Produces: fixtures proving whether stream-json resume should use live `--input-format stream-json`, process restart with `--resume`, or fallback logging.

- [ ] **Step 1: Capture normal stream-json output**

Run:

```bash
claude -p --output-format stream-json --verbose --include-partial-messages "Reply with the word ok." > docs/superpowers/fixtures/claude-stream-json-normal.ndjson
```

Expected:

```text
Command exits 0.
docs/superpowers/fixtures/claude-stream-json-normal.ndjson contains one JSON object per line.
At least one line contains a session id field or proves no session id is emitted for this command.
```

- [ ] **Step 2: Capture session/resume output**

Run:

```bash
SESSION_ID="$(grep -m1 -Eo '[0-9a-fA-F-]{36}' docs/superpowers/fixtures/claude-stream-json-normal.ndjson)"
claude --resume "$SESSION_ID" -p --output-format stream-json --verbose --include-partial-messages "Reply with the word resumed." > docs/superpowers/fixtures/claude-stream-json-session.ndjson
```

Expected:

```text
Command exits 0 when SESSION_ID is present.
If SESSION_ID is empty, record "no session id emitted by normal stream-json output" in the spec verification section.
```

- [ ] **Step 3: Capture rate-limit shape when naturally available**

Run only when the account is actually rate-limited:

```bash
claude -p --output-format stream-json --verbose --include-partial-messages "Trigger a minimal response." > docs/superpowers/fixtures/claude-stream-json-rate-limit.ndjson
```

Expected:

```text
If Claude returns a rate-limit event, the fixture contains the exact stream envelope used by the installed CLI.
If no rate limit is available, create docs/superpowers/fixtures/claude-stream-json-rate-limit.ndjson with a single line copied from the existing Claude JSONL file watcher fixture that contains error "rate_limit".
```

- [ ] **Step 4: Document observed resume behavior**

Append this section to `docs/superpowers/specs/2026-07-09-claudego-streaming-performance-design.md`:

```markdown
## Verified Stream-JSON Behavior

- Normal stream-json fixture: `docs/superpowers/fixtures/claude-stream-json-normal.ndjson`
- Session resume fixture: `docs/superpowers/fixtures/claude-stream-json-session.ndjson`
- Rate-limit fixture: `docs/superpowers/fixtures/claude-stream-json-rate-limit.ndjson`
- Resume command validated: `claude --resume <session_id> -p --output-format stream-json --verbose --include-partial-messages "continue"`
- Live stdin continuation: not enabled unless a fixture proves Claude keeps the print-mode process alive and accepts `--input-format stream-json` continuation.
```

- [ ] **Step 5: Verify fixtures exercise parser**

Run:

```bash
cargo test stream_json::tests --lib
cargo fmt --check
```

Expected:

```text
cargo test stream_json::tests --lib: PASS
cargo fmt --check: PASS
```

- [ ] **Step 6: Commit**

```bash
git add docs/superpowers/specs/2026-07-09-claudego-streaming-performance-design.md docs/superpowers/fixtures src/stream_json.rs
git commit -m "test: capture claude stream-json fixtures"
```

---

## Final Verification

Run:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build
```

Expected:

```text
cargo fmt --check: PASS
cargo test: PASS
cargo clippy --all-targets --all-features -- -D warnings: PASS
cargo build: PASS
```

Manual smoke tests:

```bash
cargo run -- -- claude --help
cargo run -- -- claude -p --output-format stream-json --verbose --include-partial-messages "Reply with ok."
```

Expected:

```text
claude --help path uses PTY and remains interactive-compatible.
stream-json path prints Claude NDJSON to stdout without wrapper formatting.
When stream parser cannot understand a line, raw stdout still appears unchanged.
```
