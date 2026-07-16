#![cfg(unix)]

use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use std::io::Read;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

pub fn run_in_pty(args: &[&str]) -> (u32, Vec<u8>) {
    let pair = NativePtySystem::default()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut reader = pair.master.try_clone_reader().unwrap();
    let (output_tx, output_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut output = Vec::new();
        let _ = reader.read_to_end(&mut output);
        let _ = output_tx.send(output);
    });

    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_botsitter"));
    command.args(args);
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let status = child.wait().unwrap();
    drop(pair.master);
    let output = output_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    (status.exit_code(), output)
}
