# botsitter

`botsitter` wraps Claude Code and Codex CLI sessions, waits for usage limits to reset, then sends `continue`.

## Requirements

- [Rust/Cargo](https://rustup.rs/)
- [Claude Code](https://code.claude.com/docs/en/setup) and/or [Codex CLI](https://developers.openai.com/codex/cli/)

Install the two product binaries:

```sh
cargo install --git https://github.com/adrien2121/botsitter --bin botsitter --bin botsitter-logs
```

## Usage

Choose a provider for every run:

```sh
botsitter claude
botsitter codex --model gpt-5.4
botsitter --prevent-sleep claude --model opus
```

Pass a custom command after `--`:

```sh
botsitter claude -- caffeinate claude
```

Open live logs for the current run, or attach manually:

```sh
botsitter --show-logs claude
botsitter-logs [pid]
```

Claude supports interactive sessions and its existing stream-JSON resume mode. Codex support is interactive-only; `codex exec` is unsupported.
