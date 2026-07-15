use anyhow::Result;
use botsitter::app;
use botsitter::providers;
use clap::Parser;
use keepawake::Builder;

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

    let args = cli.command.into_iter().map(Into::into).collect();
    let outcome = app::run(cli.show_logs, providers::claude::prepare(args)?).await?;
    drop(_awake_guard);
    Ok(std::process::ExitCode::from(outcome.wrapper_code()))
}
