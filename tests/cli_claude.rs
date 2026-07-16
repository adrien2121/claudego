#![cfg(unix)]

mod support;

#[test]
fn custom_command_arguments_are_forwarded_literally() {
    let (code, output) =
        support::run_in_pty(&["claude", "--", "/usr/bin/printf", "%s", "claude --help"]);

    assert_eq!(code, 0, "{}", String::from_utf8_lossy(&output));
    assert!(String::from_utf8_lossy(&output).contains("claude --help"));
}
