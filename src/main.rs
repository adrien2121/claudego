use anyhow::Result;
use botsitter::app;
use botsitter::cli::{select_runner, CommandSpec, RunnerKind};
use botsitter::harness::{MonitorSpec, RunPlan, SessionRoot};
use botsitter::runners::pty::PtyRunner;
use botsitter::stream_json::StreamJsonRunner;
use clap::Parser;
use keepawake::Builder;
use std::path::PathBuf;
use std::sync::Arc;

struct LegacySessionRoot;

impl SessionRoot for LegacySessionRoot {
    fn resolve(&self) -> Option<PathBuf> {
        dirs::home_dir().map(|home| home.join(".claude/projects"))
    }
}

fn prepare(command: CommandSpec) -> RunPlan {
    let monitor = MonitorSpec {
        root: Arc::new(LegacySessionRoot),
        parser: Arc::new(botsitter::watcher::scan::TranscriptLineParser),
    };
    let runner = match select_runner(&command) {
        RunnerKind::PtyInteractive => Box::new(PtyRunner::new(command)) as _,
        RunnerKind::StreamJsonPrint => Box::new(StreamJsonRunner::new(command)) as _,
    };
    RunPlan { monitor, runner }
}

/// A fire-and-forget wrapper for the claude CLI that automatically handles rate limits.
#[derive(Parser, Debug)]
#[command(
    version,
    about = "A fire-and-forget wrapper for the claude CLI that automatically handles rate limits.",
    author = "Adrien Adam"
)]
struct Cli {
    /// Prevent the system from sleeping while botsitter is running.
    #[arg(long, short = 'p')]
    prevent_sleep: bool,

    /// Show logs in a new terminal window.
    #[arg(long, short = 'l')]
    show_logs: bool,

    /// Arguments to pass to the 'claude' command.
    #[arg(last = true, name = "COMMAND")]
    command: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<std::process::ExitCode> {
    let cli = Cli::parse();

    let _awake_guard = if cli.prevent_sleep {
        Some(
            Builder::default()
                .idle(true) // Prevents the system from idle sleeping
                .sleep(true) // Prevents the system from sleeping (OS permitting)
                .create()?,
        )
    } else {
        None
    };

    let command_spec = if cli.command.is_empty() {
        // If no command is provided, default to running 'claude' by itself.
        CommandSpec::default_claude()
    } else {
        let mut iter = cli.command.into_iter();
        let program = iter.next().unwrap(); // Safe due to is_empty check
        let args = iter.collect();
        CommandSpec { program, args }
    };

    let outcome = app::run(cli.show_logs, prepare(command_spec)).await?;
    drop(_awake_guard);
    Ok(std::process::ExitCode::from(outcome.wrapper_code()))
}
