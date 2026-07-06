# claudego

Run Claude in a PTY and auto-continue after rate-limit reset.
A smart wrapper for the `claude` CLI that automatically continues your session after a rate-limit cooldown.

## What it Does

`claudego` is a "fire-and-forget" wrapper for the `claude` CLI. It runs your command as usual, but works silently in the background to monitor for rate-limit errors across all of Claude's session files on your system.

When it detects a rate limit (e.g., "Please try again in 5 hours."), it calculates the reset time and waits. Once the cooldown is over, it automatically injects a `continue` command into your session, letting you resume your work seamlessly.

The intelligent polling mechanism is designed to be efficient: it checks infrequently when the reset is hours away and more often as the time gets closer. For a detailed view of this process, you can use the `--show-logs` flag.

## How it Works

`claudego` is built in Rust.

1.  **Pseudo-Terminal (PTY):** It spawns the `claude` command within a PTY. This allows `claudego` to act as a terminal, capturing all output from `claude` and enabling it to send input (like the `continue` command) programmatically.

2.  **Log File Monitoring:** It asynchronously watches Claude's session log files. When a file is modified, `claudego` reads it to check for rate-limit messages.

3.  **Rate-Limit Detection:** Using regular expressions, it parses the log files for patterns like "Please try again in X hours/minutes." and extracts the cooldown duration.

4.  **Asynchronous Waiting:** Once a rate limit is detected, `claudego` calculates the exact reset time. It then enters an efficient, asynchronous wait state. The polling interval is adaptive: it starts long and shortens as the reset time approaches to minimize resource usage.

5.  **Automatic Resumption:** When the cooldown period ends, `claudego` injects the `continue\n` command into the PTY of the specific `claude` process it is managing. This is equivalent to you typing `continue` in that terminal window. The command will apply to whichever conversation is currently active within your `claude` session, even if you've used commands like `/resume` to switch contexts.


When you hit a rate limit, you will see messages like this in the log file:

```
You've reached your usage limit. Please try again in 5 hours.
[claudego] Rate limit detected. Resuming automatically in 4h 59m 55s...
```

## Usage

**Basic Usage:**

To start a simple, monitored `claude` session, just run the command by itself.
```bash
claudego
```

**Run `claude` with specific arguments:**
```bash
claudego -- claude --model opus "Summarize this document for me"
```
```bash
claudego -- caffeinate -s headroom wrap claude
```

### Viewing Logs (`claudego-logs`)
If you run `claudego --show-logs`, a second terminal will open to display logs.

The installation also includes a companion `claudego-logs` command. It can be used in a separate terminal while `claudego` is running.
This will automatically find the correct log file for your system and tail it in real-time. Press `Ctrl+C` to exit.

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
iwr https://raw.githubusercontent.com/adrien2121/claudego/main/install.ps1 -useb | iex
```
