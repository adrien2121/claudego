mod cli;
mod root;
mod transcript;

use crate::harness::{MonitorSpec, RunPlan};
use crate::runners::pty::PtyRunner;
use anyhow::Result;
use std::ffi::OsString;
use std::sync::Arc;

pub fn prepare(args: Vec<OsString>) -> Result<RunPlan> {
    let command = cli::command(args)?;
    Ok(RunPlan {
        monitor: MonitorSpec {
            root: Arc::new(root::CodexRoot),
            parser: Arc::new(transcript::CodexTranscriptParser),
        },
        runner: Box::new(PtyRunner::new(command)),
    })
}
