# botsitter

`botsitter` wraps Claude Code or Codex CLI and resumes interactive sessions when usage resets.

## Requirements

- [Claude Code](https://code.claude.com/docs/en/setup) or [Codex CLI](https://developers.openai.com/codex/cli/)
- [Rust/Cargo](https://rustup.rs/)

## Install

```sh
cargo install --git https://github.com/adrien2121/botsitter.git --bin botsitter --bin botsitter-logs
```

## Usage

```sh
botsitter claude
botsitter codex --model gpt-5.4
botsitter --prevent-sleep claude --model opus
botsitter claude -- caffeinate claude
botsitter --show-logs codex
botsitter-logs [pid]
```

Wrapper options go before `claude` or `codex`. Arguments after the provider are forwarded literally. Put `--` after the provider to run a custom command.

Claude supports interactive sessions and print mode with `--output-format stream-json`. Codex support is interactive-only; `codex exec` is not supported.
