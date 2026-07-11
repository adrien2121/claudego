use crate::cli::CommandSpec;
use crate::models::{mark_output_activity, SharedAppState};
use anyhow::Result;
use portable_pty::{Child, CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};

pub type SharedPtyWriter = Arc<Mutex<Box<dyn Write + Send>>>;

pub struct PtySession {
    pub child: Box<dyn Child + Send + Sync>,
    pub master: Box<dyn MasterPty + Send>,
    pub reader: Box<dyn Read + Send>,
    pub writer: SharedPtyWriter,
    pub initial_size: TerminalSize,
}

pub type TerminalSize = (u16, u16);

pub fn spawn_command_in_pty(command: CommandSpec) -> Result<PtySession> {
    let initial_size = crossterm::terminal::size().unwrap_or((120, 40));
    let pty_system = NativePtySystem::default();
    let pair = pty_system.openpty(to_pty_size(initial_size))?;

    let child = pair.slave.spawn_command(build_command(command))?;
    drop(pair.slave);

    let reader = pair.master.try_clone_reader()?;
    let writer = Arc::new(Mutex::new(pair.master.take_writer()?));

    Ok(PtySession {
        child,
        master: pair.master,
        reader,
        writer,
        initial_size,
    })
}

pub fn spawn_output_reader(mut reader: Box<dyn Read + Send>, state: SharedAppState) {
    tokio::task::spawn_blocking(move || {
        // Clone the atomic tracker once to avoid locking the state in the loop.
        let activity_tracker = state.lock().unwrap().last_output_activity.clone();
        let mut buf = [0u8; 64 * 1024];
        let stdout = io::stdout();
        let mut stdout = stdout.lock();
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            mark_output_activity(&activity_tracker);

            let _ = stdout.write_all(&buf[..n]);
            let _ = stdout.flush();
        }
    });
}

pub fn spawn_input_writer(writer: SharedPtyWriter) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 1024];
        let mut stdin = io::stdin();
        while let Ok(n) = stdin.read(&mut buf) {
            if n == 0 {
                break;
            }

            let mut pty_writer = writer.lock().expect("PTY writer lock was poisoned");
            if pty_writer.write_all(&buf[..n]).is_err() {
                break;
            }
            let _ = pty_writer.flush();
        }
    });
}

pub fn spawn_resize_poller(master: Box<dyn MasterPty + Send>, initial_size: TerminalSize) {
    #[cfg(unix)]
    std::thread::spawn(move || {
        use signal_hook::consts::SIGWINCH;
        use signal_hook::iterator::Signals;

        let mut current_size = initial_size;
        if let Ok(mut signals) = Signals::new([SIGWINCH]) {
            for _ in signals.forever() {
                if let Ok(new_size) = crossterm::terminal::size() {
                    if new_size != current_size {
                        current_size = new_size;
                        let _ = master.resize(to_pty_size(new_size));
                    }
                }
            }
        }
    });

    #[cfg(not(unix))]
    tokio::spawn(async move {
        let mut current_size = initial_size;
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            if let Ok(new_size) = crossterm::terminal::size() {
                if new_size != current_size {
                    current_size = new_size;
                    let _ = master.resize(to_pty_size(new_size));
                }
            }
        }
    });
}

fn build_command(command: CommandSpec) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(command.program);
    if !command.args.is_empty() {
        cmd.args(command.args);
    }

    if let Ok(current_dir) = std::env::current_dir() {
        let dir_str = current_dir.to_string_lossy().to_string();
        cmd.cwd(&current_dir);
        cmd.env("PWD", &dir_str);
    }

    cmd
}

fn to_pty_size((cols, rows): TerminalSize) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}
