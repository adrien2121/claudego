# Backlog

Only unfinished performance and reliability work is listed here, in priority order.

## P1 — Add end-to-end streaming stress coverage

- [x] Run a synthetic NDJSON child through the full stream runner; verify raw output and signal detection.
- [x] Verify the file-watcher fixture still detects a limit when stream events lack the signal.
- [x] Stress the logger with many diagnostics and a slow TCP client; verify the main output path never blocks.

Done when these paths are covered by deterministic tests without requiring a live Claude account.

### Acceptance gates — 2026-07-12

```text
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

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
