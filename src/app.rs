use crate::cli;
use crate::logging;
use crate::models::AppState;
use crate::monitor;
use crate::pty_bridge;
use crate::terminal::RawModeGuard;
use anyhow::Result;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

pub fn run() -> Result<()> {
    let args = cli::parse(std::env::args().skip(1))?;

    if args.show_help {
        print!("{}", cli::help_text());
        return Ok(());
    }

    let state = Arc::new(Mutex::new(AppState::new(args.show_logs)));

    if args.show_logs {
        wait_after_showing_log_instructions();
    }

    let mut session = pty_bridge::spawn_command_in_pty(args.command)?;
    let _guard = RawModeGuard::init()?;

    pty_bridge::spawn_output_reader(session.reader);
    pty_bridge::spawn_input_writer(Arc::clone(&session.writer));
    pty_bridge::spawn_resize_poller(session.master, session.initial_size);
    monitor::spawn_lockout_monitor(state, Arc::clone(&session.writer));

    let _ = session.child.wait()?;
    Ok(())
}

fn wait_after_showing_log_instructions() {
    logging::reset_log_file();
    logging::log_to_file("System initialized successfully. Starting passive monitoring.");

    let log_path = logging::log_path();
    println!("------------------------------------------------------------");
    println!("[System] Streaming live logs to {}", log_path.display());
    println!("[System] To view real-time logs, run this in a separate window:");
    println!("         tail -f {}", log_path.display());
    println!("------------------------------------------------------------");
    print!("Press ENTER to boot Claude Code...");

    let _ = io::stdout().flush();
    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input);
}
