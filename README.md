# claudego

`claudego` automatically sends `continue` to your active Claude CLI session when your usage limit has been restored. If you are running a long session, you no longer need to manually jumpstart the conversation once your limit is reset.

## What it Does

`claudego` is a "fire-and-forget" wrapper for the `claude` CLI. It runs your command as usual, but works silently in the background to monitor for rate-limit errors across all of Claude's session files on your system.

When it detects a rate limit (e.g., "Please try again in 5 hours."), it calculates the reset time and waits. Once the cooldown is over, it automatically injects a `continue` command into your session, resuming the work without your intervention.

The monitoring process is designed to be highly efficient and non-intrusive. For a detailed view of its operations, you can use the `--show-logs` flag.

When you hit a rate limit, you will see a message like this in your terminal:

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

**Preventing System Sleep:**

For long-running sessions that might span several hours, it's crucial to prevent your computer from going to sleep. `claudego` has a built-in feature for this. Use the `-p` or `--prevent-sleep` flag to keep your system awake.

```bash
claudego -p -- claude
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

## Technical Details

`claudego` employs an efficient, two-phase process to monitor for rate-limit messages without impacting system performance.

1.  **Initial Scan:** On startup, the tool performs a fast, memory-efficient scan of existing Claude session logs. It reads files *backwards* from the end in small chunks, stopping as soon as it finds the most recent rate-limit message.

2.  **Runtime Monitoring:** After the initial scan, `claudego` uses an OS-native file system watcher to monitor for changes. When a log file is updated, it uses memory-mapping to read only the new data. This process is extremely fast and has a low overhead.

To avoid performance issues, the monitor will temporarily pause its scanning activities if it detects that the `claude` CLI is actively streaming a response. This ensures that `claudego`'s background I/O does not interfere with the user's interactive session.