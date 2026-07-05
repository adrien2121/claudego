mod app;
mod cli;
mod logging;
mod models;
mod monitor;
mod pty_bridge;
mod terminal;
mod time_format;
mod watcher;

use anyhow::Result;

fn main() -> Result<()> {
    app::run()
}
