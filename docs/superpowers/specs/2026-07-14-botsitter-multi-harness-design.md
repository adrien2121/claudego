# Botsitter Multi-Harness Design

## Goal

Rename `claudego` to `botsitter` and support interactive Claude Code and Codex CLI sessions in one rate-limit-aware wrapper.

Claude keeps its existing interactive and stream-JSON behavior. Codex support is interactive-only in this release.

## Identity

- GitHub repository: `adrien2121/botsitter`
- Cargo package and library: `botsitter`
- Main binary: `botsitter`
- Log viewer: `botsitter-logs`
- Runtime files: `botsitter-<pid>.log` and `botsitter-<pid>.port`

This is a hard rename. There is no `claudego` compatibility binary or temp-file migration. Historical specs, plans, and provider-specific fixtures keep their original names when those names describe historical or Claude-specific behavior.

## CLI

```text
botsitter [OPTIONS] claude [CLAUDE_ARGS...]
botsitter [OPTIONS] codex  [CODEX_ARGS...]

botsitter [OPTIONS] claude -- <CUSTOM COMMAND...>
botsitter [OPTIONS] codex  -- <CUSTOM COMMAND...>
```

Examples:

```bash
botsitter claude
botsitter codex --model gpt-5.4
botsitter --prevent-sleep claude --model opus
botsitter claude -- caffeinate claude
```

Rules:

- Provider selection is mandatory. There is no default provider.
- Wrapper options precede the provider.
- Arguments are forwarded literally without shell parsing.
- `botsitter --help` shows wrapper help.
- `botsitter <provider> --help` forwards help to that provider.
- The provider selects session discovery, rate-limit parsing, and resume behavior even when a custom command is used.
- `botsitter codex exec ...` is rejected with a clear unsupported-mode error in this release.

## Architecture

Use a small `ProviderKind { Claude, Codex }` enum and direct `match` statements. Do not add a trait, plugin registry, provider config file, database, or app-server client.

Shared runtime responsibilities remain provider-neutral:

- PTY lifecycle and terminal resizing
- stdin/stdout forwarding
- sleep prevention
- logger lifecycle and viewer connection
- lockout timer and exactly-once resume dispatch
- child exit propagation

Provider-specific behavior supplies:

- default executable
- session root
- JSONL event parser
- supported runner modes
- resume behavior

Claude uses the existing `~/.claude/projects` watcher, textual rate-limit parser, PTY `continue`, and stream-JSON restart behavior.

Codex uses `$CODEX_HOME/sessions`, defaulting to `~/.codex/sessions`, and the existing PTY `continue` mechanism. Codex non-interactive resume is deferred.

## Codex Rate-Limit Data

Codex persists session transcripts under `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl`. Rate-limit snapshots appear in records shaped like:

```json
{
  "timestamp": "2026-07-15T01:58:37.161Z",
  "type": "event_msg",
  "payload": {
    "type": "token_count",
    "rate_limits": {
      "primary": {
        "used_percent": 100.0,
        "window_minutes": 300,
        "resets_at": 1784672717
      },
      "secondary": null,
      "rate_limit_reached_type": null
    }
  }
}
```

The Codex app-server protocol exposes equivalent rate-limit fields, but running an app-server solely for quota reads adds unnecessary process and JSON-RPC plumbing. Botsitter therefore watches the persisted JSONL records, matching the current Claude architecture.

Sources:

- [Codex app-server rate-limit protocol](https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md#7-rate-limits-chatgpt)
- [Codex rate-limit parser](https://github.com/openai/codex/blob/main/codex-rs/codex-api/src/rate_limits.rs)
- [Codex manual](https://developers.openai.com/codex/codex-manual.md)

## Codex Lockout Rules

Only `event_msg` records whose payload type is `token_count` can change Codex lockout state.

1. Parse the top-level event timestamp and ignore events older than the current watermark.
2. Ignore a missing or null `rate_limits` snapshot without clearing existing state.
3. A primary or secondary window is saturated only when `used_percent >= 100` and `resets_at` is a valid future Unix timestamp.
4. If multiple windows are saturated, wait until the latest reset so all blocking windows have cleared.
5. A newer valid snapshot with no saturated windows clears the lockout. This supports normal expiry and externally consumed reset credits.
6. At the target time, send `continue\r` exactly once to the active Codex PTY.

Malformed snapshots or saturated windows without usable reset timestamps produce sanitized diagnostics and no guessed resume time. Session content, account identifiers, balances, and raw JSONL lines are never logged.

## Concurrency and Chronology

Codex quotas are account-wide, so the newest valid rate-limit snapshot under the active `CODEX_HOME` is authoritative even when another local Codex session wrote it. Existing event-time watermarking prevents an older file event from replacing newer state.

Each Botsitter process owns only its child PTY and sends resume input only to that child. Multiple Botsitter processes may observe the same account-wide reset and resume independently.

## Error Handling

- Missing provider executable: return the spawn error unchanged with provider context.
- Missing session root: create it when possible; otherwise run the child and report monitoring is unavailable through diagnostics.
- Watcher failure: preserve existing bounded retry behavior.
- Unknown or changed Codex schema: ignore it and log a sanitized parser diagnostic.
- Missing reset timestamp: do not invent a wait duration or send `continue`.
- PTY resume failure: preserve existing definite/ambiguous failure handling.
- Codex non-interactive invocation: fail before spawning with an explicit interactive-only message.

## Testing

Add the smallest coverage that proves each new boundary:

- CLI provider selection, literal forwarding, custom-command forwarding, and help routing
- existing Claude tests unchanged except required hard-rename updates
- Codex fixtures for primary, secondary, both saturated, cleared, null, malformed, and stale snapshots
- `$CODEX_HOME` override and default path behavior
- chronology tests proving newer snapshots win and below-100 snapshots clear lockout
- fake Codex PTY integration test proving limit detection, waiting, and one `continue`
- package, viewer, and PID-scoped runtime-path rename checks

Verification gates:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo bench --bench pty_hot_path --no-run
cargo bench --bench runtime_candidates --no-run
```

A fake CLI test cannot prove that a real exhausted Codex TUI accepts `continue`; record that as a live validation requirement before claiming production parity.

## Documentation and Release

Update active source, tests, comments, `Cargo.toml`, `Cargo.lock`, installers, binary names, runtime paths, and README references from `claudego` to `botsitter`.

Keep the README short and user-facing:

- purpose
- Claude Code and Codex CLI prerequisites
- Cargo install command
- provider-selection examples
- sleep prevention and log-viewer commands
- Claude interactive/stream-JSON behavior
- Codex interactive-only limitation

After local changes and verification pass:

1. Rename the GitHub repository from `adrien2121/claudego` to `adrien2121/botsitter`.
2. Verify the new repository and Cargo install URLs.
3. Use only the verified new URL in README and installers.

The local checkout directory remains unchanged. GitHub redirects are useful migration behavior but are not treated as verification of release assets or install commands.

## Deferred

- `codex exec --json` monitoring and `codex exec resume`
- app-server rate-limit reads or subscriptions
- generic third-party provider plugins
- provider configuration files
- compatibility aliases
- migration of old temp logs
- unrelated refactoring
