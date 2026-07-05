# claudego

Run Claude in a PTY and auto-continue after rate-limit reset.
A smart wrapper for the `claude` CLI that automatically continues your session after a rate-limit cooldown.

## What it Does

`claudego` is a "fire-and-forget" wrapper for the `claude` CLI. It runs your command as usual, but works silently in the background to monitor for rate-limit errors across all of Claude's session files on your system.

When it detects a rate limit (e.g., "Please try again in 5 hours."), it doesn't interrupt you. It calculates the reset time and waits. Once the cooldown is over, it automatically injects a `continue` command into your session, letting you resume your work seamlessly.

The intelligent polling mechanism is designed to be efficient: it checks infrequently when the reset is hours away and more often as the time gets closer. For a detailed view of this process, you can use the `--show-logs` flag.

## How it Works (with Logging)

While `claudego` operates silently by default, enabling logs with `-l` or `--show-logs` reveals what's happening behind the scenes. When you hit a rate limit, you will see messages like this in the log file:

```
You've reached your usage limit. Please try again in 5 hours.
[claudego] Rate limit detected. Resuming automatically in 4h 59m 55s...
```

## Usage

`claudego` can be run directly to launch `claude`, or it can be used to wrap any custom command and its arguments.

**Default Behavior (run `claude`):**

Running `claudego` by itself is the simplest way to start a monitored `claude` session.
```bash
claudego
```

**Run `claude` with specific arguments:**
```bash
claudego -- claude --model opus "Summarize this document for me"
```

### Options

*   `-l`, `--show-logs`: Enables diagnostic logging. When this flag is active, `claudego` will print instructions on how to view live logs and write detailed operational information to a log file.
    *   **macOS / Linux**: `/tmp/claudego.log`
    *   **Windows**: `%TEMP%\claudego.log`

## Installation

---

### macOS / Linux

You can install `claudego` with the following command. The script will install the binary to `$HOME/.local/bin`.

```bash
curl -fsSL https://raw.githubusercontent.com/adrien2121/claudego/main/install.sh | sh
```

Please ensure `$HOME/.local/bin` is in your `PATH` environment variable.

### Windows

You can install `claudego` with this one-line command in PowerShell. This will download the latest release and place it in a local directory.

```powershell
iwr https://raw.githubusercontent.com/adrienadam/claudego/main/install.ps1 -useb | iex
```
