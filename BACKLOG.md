# Backlog

Only unfinished performance and reliability work is listed here, in priority order.

## P0 — Measure the PTY hot path

- [x] Add a repeatable synthetic high-output PTY benchmark with the monitor active.
- [x] Verify byte-for-byte output integrity and record throughput and output stalls.
- [x] Compare results with file scanning idle versus deferred during active output.
- [x] Change or replace the PTY implementation only if measurements show it remains the bottleneck.

Done when the benchmark is repeatable and gives enough evidence to keep or change the current PTY bridge.

### Evidence — 2026-07-10

Command: `cargo bench --bench pty_hot_path`

Host: macOS 15.6.1; arm64; rustc 1.96.1 (31fca3adb 2026-06-26)

commit: e7b14b2a0b613a11c54dbdf8dd59ab7b1a374abd

Another `claudego` process must not run concurrently because the logger files are process-global.

Baseline

```text
direct median throughput: 281.00 MiB/s
monitor-idle median throughput: 221.67 MiB/s
scan-deferred median throughput: 223.20 MiB/s
```

After stdout lock + 64 KiB buffer

```text
direct run 1: verified_bytes=2342191104 elapsed=8.000s throughput=279.21 MiB/s p99_gap=0.007ms max_gap=10.684ms stalls>=100ms=0
direct run 2: verified_bytes=2313682944 elapsed=8.000s throughput=275.81 MiB/s p99_gap=0.007ms max_gap=3.962ms stalls>=100ms=0
direct run 3: verified_bytes=2343370752 elapsed=8.000s throughput=279.35 MiB/s p99_gap=0.008ms max_gap=12.558ms stalls>=100ms=0
monitor-idle run 1: verified_bytes=1935736832 elapsed=8.000s throughput=230.76 MiB/s p99_gap=0.011ms max_gap=1.405ms stalls>=100ms=0
monitor-idle run 2: verified_bytes=1898512384 elapsed=8.000s throughput=226.31 MiB/s p99_gap=0.010ms max_gap=11.724ms stalls>=100ms=0
monitor-idle run 3: verified_bytes=2038104064 elapsed=8.000s throughput=242.96 MiB/s p99_gap=0.010ms max_gap=2.920ms stalls>=100ms=0
scan-deferred run 1: verified_bytes=1929445376 elapsed=8.000s throughput=230.01 MiB/s p99_gap=0.011ms max_gap=1.253ms stalls>=100ms=0
scan-deferred run 2: verified_bytes=1913389056 elapsed=8.000s throughput=228.09 MiB/s p99_gap=0.010ms max_gap=1.121ms stalls>=100ms=0
scan-deferred run 3: verified_bytes=1647968256 elapsed=8.000s throughput=196.45 MiB/s p99_gap=0.021ms max_gap=5.160ms stalls>=100ms=0
Medians:
direct median throughput: 279.21 MiB/s
monitor-idle median throughput: 230.76 MiB/s
scan-deferred median throughput: 228.09 MiB/s
```

PTY decision: TUNE the current bridge first by locking stdout once and enlarging its read buffer.

Monitor decision: KEEP current deferral behavior; deferred work adds no measured boundary violation.

Upgrade/replace decision: NOT JUSTIFIED; reconsider portable-pty only after the ordered local optimization and a full rerun still isolate bridge cost.

## P1 — Add end-to-end streaming stress coverage

- [ ] Run a synthetic NDJSON child through the full stream runner; verify raw output and signal detection.
- [ ] Verify the file-watcher fixture still detects a limit when stream events lack the signal.
- [ ] Stress the logger with many diagnostics and a slow TCP client; verify the main output path never blocks.

Done when these paths are covered by deterministic tests without requiring a live Claude account.

## P1 — Validate authenticated resume behavior

- [ ] Log in to Claude Code and capture a successful normal `stream-json` session.
- [ ] Verify `--resume <session_id>` produces an authenticated resumed response.
- [ ] Test live `--input-format stream-json` continuation only if the print-mode process stays alive.
- [ ] Keep restart-with-`--resume` unless direct evidence proves live continuation is reliable.

Done when the chosen resume path is backed by captured successful output. This is currently blocked by local Claude authentication.

## P2 — Profile startup scanning

- [ ] Benchmark startup with small and large `~/.claude/projects` histories.
- [ ] Measure file enumeration, metadata reads, and reverse JSONL scanning separately.
- [ ] Optimize only the measured bottleneck and preserve newest-limit detection behavior.

Done when startup cost is measured and any worthwhile optimization has a regression test.

## P2 — Measure memory and log footprint

- [ ] Record peak memory during long PTY and `stream-json` output runs.
- [ ] Stress log rotation, the bounded startup buffer, parser pressure, and disconnected clients.
- [ ] Tighten limits only if measurements show excessive memory or disk use.

Done when long-running memory and log growth are demonstrably bounded.
