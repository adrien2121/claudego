#![cfg(unix)]

use std::process::Command;

mod support;

const INTERACTIVE_ONLY: &str = "Codex support is interactive-only; 'codex exec' is unsupported";

fn botsitter(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_botsitter"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn custom_command_arguments_are_forwarded_literally() {
    let (code, output) =
        support::run_in_pty(&["codex", "--", "/usr/bin/printf", "%s", "codex --help"]);

    assert_eq!(code, 0, "{}", String::from_utf8_lossy(&output));
    assert!(String::from_utf8_lossy(&output).contains("codex --help"));
}

#[test]
fn exec_is_rejected() {
    for args in [
        vec!["codex", "exec"],
        vec!["codex", "exec", "--json"],
        vec!["codex", "--", "codex", "exec"],
    ] {
        let output = botsitter(&args);
        assert!(!output.status.success(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(INTERACTIVE_ONLY),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn interactive_arguments_are_not_rejected_as_exec() {
    let (code, output) =
        support::run_in_pty(&["codex", "--", "/usr/bin/true", "--model", "gpt-5.4"]);

    assert_eq!(code, 0, "{}", String::from_utf8_lossy(&output));
}
