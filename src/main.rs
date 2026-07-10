use anyhow::Result;
use clap::Parser;
use claudego::app;
use claudego::cli::CommandSpec;
use keepawake::Builder;

/// A fire-and-forget wrapper for the claude CLI that automatically handles rate limits.
#[derive(Parser, Debug)]
#[command(
    version,
    about = "A fire-and-forget wrapper for the claude CLI that automatically handles rate limits.",
    author = "Adrien Adam"
)]
struct Cli {
    /// Prevent the system from sleeping while claudego is running.
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
async fn main() -> Result<()> {
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

    app::run(cli.show_logs, command_spec).await
}
