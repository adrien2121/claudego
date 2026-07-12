#[cfg(unix)]
#[test]
fn final_pty_bytes_drain_before_shutdown() {
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
    let tmp = std::env::temp_dir().join(format!("claudego-pty-exit-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("create isolated temp directory");
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_claudego"));
    command.args(["--", "/bin/sh", "-c", "printf 'FINAL-PTY-BYTES\\n'"]);
    command.env("TMPDIR", &tmp);
    let mut child = pair.slave.spawn_command(command).expect("spawn claudego");
    drop(pair.slave);
    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        if let Some(status) = child.try_wait().expect("poll claudego") {
            assert!(status.success());
            let output = output_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("capture wrapper output");
            assert!(output.ends_with(b"FINAL-PTY-BYTES\r\n"));
            let log = std::fs::read_to_string(tmp.join("claudego.log")).expect("read isolated log");
            let reader_stop = log.find("[PTY Output] reader stopped").unwrap();
            let shutdown = log
                .find("[System] Child process exited. Shutting down.")
                .unwrap();
            assert!(reader_stop < shutdown);
            let _ = std::fs::remove_dir_all(&tmp);
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
