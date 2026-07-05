use anyhow::Result;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

pub struct RawModeGuard;

impl RawModeGuard {
    pub fn init() -> Result<Self> {
        enable_raw_mode()?;
        Ok(RawModeGuard)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}
