# PTY Benchmark Per-Run TMPDIR Isolation Design

Date: 2026-07-11

## Goal

Make the PTY hot-path benchmark's watcher/logger evidence deterministic by giving every monitored run its own temporary logger directory.

The benchmark must keep proving both live watcher readiness and persistent log evidence without depending on process-global `claudego.log` and `claudego.port` files.

## Root Cause

`claudego` derives its logger paths from `std::env::temp_dir()` and uses fixed filenames:

- `claudego.log`
- `claudego.port`

The benchmark currently checks once for an active logger, then runs six monitored scenarios against those global paths. Another `claudego` process can start after the check and delete or replace either file.

The failure was reproduced with two processes:

1. Process A emitted watcher readiness to its live TCP client.
2. Process B started and recreated the process-global logger files.
3. Process A's live stream still contained watcher readiness.
4. The final `claudego.log` belonged to process B and did not contain watcher readiness.

This produces the benchmark failure `persistent log is missing watcher readiness` even though the monitored process initialized correctly.

The logger's buffered write and graceful shutdown are not the failing boundary. The benchmark is reading a replaceable global pathname that is not owned by the run it is validating.

## Scope

Change benchmark behavior and its evidence note only:

- `benches/pty_hot_path.rs`
- the concurrency note in `BACKLOG.md`

Do not change production logger behavior, logger filenames, monitor behavior, PTY behavior, dependencies, or benchmark performance thresholds.

This design supersedes the process-global logger assumption in `2026-07-10-pty-hot-path-benchmark-design.md` for benchmark runs.

## Design

### Per-Run Paths

Each existing benchmark `run_dir` owns a `tmp` directory:

```text
<run_dir>/
  expected-count
  flood-gate
  home/
  tmp/
    claudego.log
    claudego.port
```

For monitored scenarios, the benchmark creates `<run_dir>/tmp` before spawning `claudego`.

The direct scenario remains unchanged because it does not start `claudego` or consume logger evidence.

### Child Environment

The monitored `CommandBuilder` sets:

- `HOME=<run_dir>/home` for the synthetic Claude project fixture;
- `TMPDIR=<run_dir>/tmp` for logger isolation.

`portable_pty::CommandBuilder::env` accepts OS-native strings, so paths must be passed without adding a UTF-8-only conversion.

On the benchmark's validated Unix/macOS target, `std::env::temp_dir()` uses `TMPDIR` when it is set. Production `claudego` therefore continues using its existing path functions while the benchmark child resolves logger files inside its run directory.

### Evidence Lookup

The benchmark driver derives both logger paths from the same per-run `tmp` directory passed to the child:

- live logger connection reads `<run_dir>/tmp/claudego.port`;
- persistent evidence reads `<run_dir>/tmp/claudego.log`.

There is no fallback to the process-global paths. A missing run-local port or log remains a named benchmark failure.

The existing assertions remain:

- live TCP output must report watcher readiness before the flood gate opens;
- persistent output must contain watcher readiness;
- deferred runs must contain scan-deferral evidence;
- idle runs must not contain scan-deferral evidence.

### Global Preflight

Keep the existing startup check for another active global `claudego` logger. It protects measurement quality from avoidable competing work.

It is no longer a correctness mechanism. A process that starts after preflight cannot replace the benchmark child's run-local logger files.

Do not delete process-global logger files before each monitored run. Only the initial preflight may clean stale global files.

### Cleanup

The existing `ScratchDir` owns all run-local logger files. Its current drop cleanup removes them with the rest of the benchmark fixture.

No new cleanup mechanism or dependency is required.

## Error Handling

Fail the named scenario when:

- its run-local temporary directory cannot be created;
- its run-local port file never becomes reachable before the deadline;
- its run-local persistent log cannot be read after child exit;
- required watcher or scan-deferral evidence is absent; or
- existing output-integrity, child-status, sidecar, or timeout checks fail.

Errors should retain the existing concise scenario context. Include the run-local path when a port or log file cannot be read so a failed fixture can be identified before cleanup.

## Regression Strategy

Use the benchmark's existing startup self-check as the smallest permanent regression seam.

Centralize derivation of the run-local temporary, port, and log paths. The self-check must prove that:

- log and port paths are children of the selected run-local temporary directory;
- two different run directories produce different logger paths; and
- neither run-local logger path equals the process-global preflight path.

The full benchmark remains the behavior-level regression: it can only connect and validate persistent evidence if the child environment and driver paths agree.

Verification must include a controlled collision probe in which another `claudego` process replaces the global logger files after a benchmark child starts. The monitored run's local live and persistent evidence must remain intact.

## Documentation

Replace the `BACKLOG.md` statement that another process must not run because logger files are global.

The replacement must say:

- benchmark logger evidence is isolated per run; and
- other `claudego` processes should still be stopped because competing work invalidates performance comparisons.

Do not modify recorded benchmark numbers as part of this fix.

## Verification

Required commands:

```bash
cargo bench --bench pty_hot_path --no-run
cargo bench --bench pty_hot_path
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

The benchmark requires localhost binding and must run outside a network-restricted sandbox.

Success means:

- the path-isolation self-check passes;
- the controlled global-file collision cannot alter run-local evidence;
- all nine benchmark runs preserve byte integrity and logger assertions;
- no repeatable throughput or stall regression is attributable to path isolation; and
- formatting, tests, and strict Clippy pass.

## Non-Goals

- No production logger redesign.
- No per-process logger filename change outside the benchmark.
- No lock file or cross-process coordination protocol.
- No retry that hides missing evidence.
- No per-line persistent-log flush.
- No new crate.
- No change to benchmark scenarios, run duration, thresholds, or decision wording.
- No repair of the missing baseline run details in `BACKLOG.md`.

## References

- Rust temporary-directory behavior: <https://doc.rust-lang.org/std/env/fn.temp_dir.html>
- `portable_pty::CommandBuilder::env`: <https://docs.rs/portable-pty/0.8.1/portable_pty/cmdbuilder/struct.CommandBuilder.html#method.env>
- Existing benchmark design: `docs/superpowers/specs/2026-07-10-pty-hot-path-benchmark-design.md`
