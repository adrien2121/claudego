# PTY Hot-Path Benchmark Design

Date: 2026-07-10

## Goal

Complete `BACKLOG.md` P0 with repeatable evidence for the interactive PTY path on the current macOS host.

The benchmark must:

- exercise `claudego` with its monitor active;
- prove byte-for-byte output integrity;
- report throughput and output stalls;
- compare an idle monitor with a file scan deferred during active output; and
- justify keeping, tuning, upgrading, or replacing the current PTY bridge.

Production behavior remains portable. Only the current macOS host is a validated performance target in this work.

## Current Path and Hypotheses

The PTY output path currently performs one blocking PTY read, one activity timestamp update, one stdout write, and one stdout flush per chunk. Its read buffer is 1 KiB.

The blocking reader already runs outside Tokio's asynchronous worker threads, which is appropriate for `portable-pty`. The unmeasured suspects are therefore the small buffer, repeated stdout locking, timestamp updates, and per-read flushing. These are hypotheses, not reasons to replace `portable-pty`.

## Benchmark Shape

Add one custom Cargo benchmark:

- `benches/pty_hot_path.rs`
- a `[[bench]]` entry with `harness = false`
- no new dependency

Run it with:

```bash
cargo bench --bench pty_hot_path
```

Cargo provides the built `claudego` executable to benchmark targets through `CARGO_BIN_EXE_claudego`. The benchmark binary also launches itself in a child mode, avoiding a separate flood utility.

The benchmark driver captures output through an outer PTY. This matches a real terminal-facing `claudego` process without including terminal rendering cost.

## Scenarios

Run each scenario three times using the optimized benchmark profile:

1. **Direct control**: the outer PTY runs the flood child directly.
2. **Monitor idle**: the outer PTY runs `claudego -- <flood child>` with the monitor active and no file event.
3. **Scan deferred**: the same wrapper run uses an isolated `HOME`, triggers a synthetic Claude JSONL change, and proves the monitor deferred scanning while output was hot.

The direct scenario has one fewer PTY. It measures total wrapper cost, not `portable-pty` cost in isolation.

## Workload and Integrity

The flood child writes deterministic 64 KiB chunks of printable ASCII for eight seconds. Printable bytes avoid newline and control-character translation by PTY terminal modes.

The driver:

- validates every received byte against the deterministic pattern as it arrives;
- records received byte count without retaining the full stream; and
- compares that count with the child's expected count written to a temporary sidecar file after output completes.

A missing sidecar, pattern mismatch, or count mismatch fails the benchmark. Performance results are invalid until integrity passes.

## Monitor Synchronization

Monitor scenarios use a temporary `HOME` containing `.claude/projects/benchmark/session.jsonl`; real Claude history is never scanned.

The flood child waits on a temporary gate file. For the deferred scenario, the driver:

1. waits until `claudego` logs that its watcher is active;
2. appends non-limit JSONL data to the synthetic session file;
3. releases the flood gate four seconds into the existing five-second debounce window; and
4. verifies the log records scan deferral while output is hot.

This places active output across the real production debounce boundary without a benchmark-only production hook.

`claudego` currently uses a process-global temporary log path. Benchmark scenarios run serially and must not run beside another `claudego` process.

## Measurements

For every run, record:

- verified bytes;
- elapsed time from first payload byte to final payload byte;
- MiB/s;
- p99 and maximum inter-read gap; and
- count of gaps at least 100 ms.

Report every run plus the median throughput for each scenario. Startup, watcher preparation, and EOF wait are outside the measured output interval.

Performance remains a report, not a hardware-independent test assertion. If run-to-run spread makes the five-percent boundary ambiguous, rerun instead of declaring a result.

## Decision Rules

Treat the PTY path as a worthwhile optimization target when either condition repeats:

- monitor-idle median throughput is at least five percent below direct control; or
- the wrapper introduces stalls of at least 100 ms that are absent from direct control.

Treat monitor work as competing with output when the deferred scenario is at least five percent slower than monitor-idle or introduces new stalls of at least 100 ms.

Keep a production optimization only when repeated results improve the relevant metric without output-integrity or stall regression.

Optimization order:

1. lock stdout once and enlarge the PTY read buffer;
2. rerun all scenarios;
3. investigate further bridge costs only if evidence still warrants it; and
4. upgrade or replace `portable-pty` only if the bridge remains the measured bottleneck.

Per-read flush stays initially because it protects interactive latency. Change it only with evidence that throughput improves without delaying partial terminal output.

If the baseline stays within the decision boundary and has no wrapper-only stalls, keep the current PTY implementation and finish P0 with the benchmark evidence.

## Failure Handling

Fail the named scenario on:

- byte or count mismatch;
- child failure;
- missing watcher readiness or scan-deferral evidence;
- missing sidecar data; or
- a 30-second per-run timeout.

Terminate a benchmark child that exceeds the timeout before returning failure. Correctness failures take priority over performance tuning.

## Comments and Documentation

The benchmark file must begin with concise module documentation covering its purpose, invocation, scenarios, and output.

Inline comments must explain only non-obvious intent:

- why an outer PTY is required;
- why the payload excludes control bytes;
- why the sidecar count is outside the measured stream;
- how the watcher/debounce gate guarantees overlap; and
- which timestamps define a stall.

Do not narrate obvious Rust syntax or restate function names.

After the run, update `BACKLOG.md` P0 with:

- checked acceptance items;
- benchmark command;
- macOS version, architecture, Rust version, and commit;
- scenario results; and
- the keep/tune/replace decision.

## Verification

Required final commands:

```bash
cargo bench --bench pty_hot_path
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## Non-Goals

- No terminal-emulator rendering benchmark.
- No CI performance threshold.
- No Linux or Windows performance claim.
- No benchmark framework dependency.
- No monitor bypass option.
- No speculative PTY rewrite.

## References

- Cargo benchmark executable discovery: <https://doc.rust-lang.org/cargo/reference/environment-variables.html>
- Rust stdout buffering and explicit locking: <https://doc.rust-lang.org/std/io/struct.Stdout.html>
- `portable-pty` blocking-read guidance: <https://github.com/wezterm/wezterm/discussions/3739>
