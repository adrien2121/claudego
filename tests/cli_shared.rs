use std::process::Command;

#[test]
fn provider_is_required() {
    let output = Command::new(env!("CARGO_BIN_EXE_botsitter"))
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .to_ascii_lowercase()
        .contains("provider"));
}

#[test]
fn root_help_lists_both_providers() {
    let output = Command::new(env!("CARGO_BIN_EXE_botsitter"))
        .arg("--help")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("claude"));
    assert!(stdout.contains("codex"));
}

#[test]
fn unsupported_provider_is_rejected() {
    let output = Command::new(env!("CARGO_BIN_EXE_botsitter"))
        .arg("other")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("unsupported provider 'other'; expected 'claude' or 'codex'"));
}
