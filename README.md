# claudego

`claudego` wraps Claude Code and resumes rate-limited sessions when usage resets.

## Requirements

- [Claude Code](https://code.claude.com/docs/en/setup) installed, authenticated, and available as `claude`.
- [Rust/Cargo](https://rustup.rs/).

## Install

On macOS, Linux, or Windows, install `claudego` and `claudego-logs` to Cargo's binary directory:

```sh
cargo install --git https://github.com/adrien2121/claudego.git
```

## Usage

Run Claude:

```sh
claudego
```

Pass Claude arguments or run another command after `--`:

```sh
claudego -- claude --model opus
claudego -- <command> [args...]
```

Keep the system awake while running:

```sh
claudego --prevent-sleep
```

Open live logs for the current run, or attach manually:

```sh
claudego --show-logs
claudego-logs [pid]
```

Without a PID, `claudego-logs` follows the newest reachable run and waits for the next one after disconnection.

## Behavior

- Watches local Claude session logs for usage-limit resets, waits, then sends `continue`.
- Interactive commands run in a PTY. Claude with `-p`/`--print` and `--output-format stream-json` resumes by session ID.
- Returns the child process exit code when the platform provides one.
- `--prevent-sleep` depends on OS support. `--show-logs` requires a supported terminal launcher.
