#[cfg(unix)]
#[test]
fn wrapper_exits_after_pty_child_exits() {
    use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
    use std::io::Read;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    let pair = NativePtySystem::default()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open PTY");
    let mut reader = pair.master.try_clone_reader().expect("clone PTY reader");
    let (output_tx, output_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut output = Vec::new();
        let _ = reader.read_to_end(&mut output);
        let _ = output_tx.send(output);
    });
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_claudego"));
    command.args(["--", "/bin/echo", "smoke"]);
    let mut child = pair.slave.spawn_command(command).expect("spawn claudego");
    drop(pair.slave);
    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        if let Some(status) = child.try_wait().expect("poll claudego") {
            assert!(status.success());
            return;
        }
        if Instant::now() >= deadline {
            if let Some(pid) = child.process_id() {
                let _ = std::process::Command::new("/bin/kill")
                    .args(["-KILL", &pid.to_string()])
                    .status();
            }
            let output = output_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap_or_default();
            panic!(
                "claudego did not exit after its PTY child exited; output: {}",
                String::from_utf8_lossy(&output)
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
}
