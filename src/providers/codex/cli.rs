use crate::cli::{command_from_args, CommandSpec};
use anyhow::Result;
use std::ffi::{OsStr, OsString};
use std::path::Path;

const INTERACTIVE_ONLY: &str = "Codex support is interactive-only; 'codex exec' is unsupported";

pub(super) fn command(args: Vec<OsString>) -> Result<CommandSpec> {
    let command = command_from_args("codex", args)?;
    if is_codex_program(&command.program) && has_exec_subcommand(&command.args) {
        anyhow::bail!(INTERACTIVE_ONLY);
    }
    Ok(command)
}

fn has_exec_subcommand(args: &[OsString]) -> bool {
    let mut index = 0;
    while let Some(arg) = args.get(index).and_then(|arg| arg.to_str()) {
        if arg == "--" {
            return false;
        }
        if arg == "exec" || arg == "e" {
            return true;
        }
        if !arg.starts_with('-') {
            return false;
        }
        if is_variadic_image_option(arg) {
            index += 1;
            while args
                .get(index)
                .is_some_and(|value| !value.to_str().is_some_and(|value| value.starts_with('-')))
            {
                index += 1;
            }
            continue;
        }
        if option_takes_separate_value(arg) {
            index += 1;
        }
        index += 1;
    }
    false
}

fn is_variadic_image_option(arg: &str) -> bool {
    arg == "--image" || arg == "-i"
}

fn option_takes_separate_value(arg: &str) -> bool {
    const LONG: &[&str] = &[
        "--config",
        "--enable",
        "--disable",
        "--remote",
        "--remote-auth-token-env",
        "--model",
        "--local-provider",
        "--profile",
        "--sandbox",
        "--cd",
        "--add-dir",
        "--ask-for-approval",
    ];
    const SHORT: &[&str] = &["-c", "-m", "-p", "-s", "-C", "-a"];

    LONG.contains(&arg) || SHORT.contains(&arg)
}

fn is_codex_program(program: &OsStr) -> bool {
    Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "codex" || name == "codex.exe")
}

#[cfg(test)]
mod tests {
    use super::{command, INTERACTIVE_ONLY};
    use crate::cli::CommandSpec;
    use std::ffi::OsString;

    #[test]
    fn interactive_arguments_are_preserved() {
        assert_eq!(
            command(vec!["--model".into(), "gpt-5.4".into()]).unwrap(),
            CommandSpec {
                program: "codex".into(),
                args: vec!["--model".into(), "gpt-5.4".into()],
            }
        );
    }

    #[test]
    fn default_and_custom_codex_exec_are_rejected() {
        for args in [
            vec![OsString::from("exec")],
            vec![OsString::from("e")],
            vec![OsString::from("exec"), OsString::from("--json")],
            vec![
                OsString::from("--model"),
                OsString::from("gpt-5.4"),
                OsString::from("exec"),
                OsString::from("--help"),
            ],
            vec![
                OsString::from("--"),
                OsString::from("codex"),
                OsString::from("exec"),
            ],
            vec![
                OsString::from("--"),
                OsString::from("/opt/bin/codex"),
                OsString::from("--profile"),
                OsString::from("work"),
                OsString::from("exec"),
            ],
        ] {
            assert_eq!(command(args).unwrap_err().to_string(), INTERACTIVE_ONLY);
        }
    }

    #[test]
    fn option_value_named_exec_remains_interactive_input() {
        assert_eq!(
            command(vec!["--model".into(), "exec".into()]).unwrap(),
            CommandSpec {
                program: "codex".into(),
                args: vec!["--model".into(), "exec".into()],
            }
        );
    }

    #[test]
    fn exec_after_variadic_images_and_later_options_is_rejected() {
        assert_eq!(
            command(vec![
                "--image".into(),
                "/dev/null".into(),
                "/dev/null".into(),
                "--model".into(),
                "gpt-5.4".into(),
                "exec".into(),
                "--help".into(),
            ])
            .unwrap_err()
            .to_string(),
            INTERACTIVE_ONLY
        );
    }

    #[test]
    fn exec_without_option_boundary_remains_an_image_value() {
        assert!(command(vec![
            "--image".into(),
            "/dev/null".into(),
            "exec".into(),
            "--help".into(),
        ])
        .is_ok());
    }

    #[test]
    fn exec_after_attached_image_values_is_rejected() {
        for image in ["--image=/dev/null", "-i/dev/null"] {
            assert_eq!(
                command(vec![image.into(), "exec".into(), "--help".into()])
                    .unwrap_err()
                    .to_string(),
                INTERACTIVE_ONLY
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_image_value_does_not_stop_later_exec_classification() {
        use std::os::unix::ffi::OsStringExt;

        let private_image = OsString::from_vec(vec![b'/', b't', b'm', b'p', b'/', 0xff]);
        assert_eq!(
            command(vec![
                "--image".into(),
                private_image,
                "--model".into(),
                "gpt-5.4".into(),
                "exec".into(),
            ])
            .unwrap_err()
            .to_string(),
            INTERACTIVE_ONLY
        );
    }

    #[test]
    fn custom_command_is_literal() {
        assert_eq!(
            command(vec![
                "--".into(),
                "sh".into(),
                "-c".into(),
                "codex exec".into()
            ])
            .unwrap(),
            CommandSpec {
                program: "sh".into(),
                args: vec!["-c".into(), "codex exec".into()],
            }
        );
    }

    #[test]
    fn child_help_is_preserved() {
        assert_eq!(
            command(vec!["--help".into()]).unwrap(),
            CommandSpec {
                program: "codex".into(),
                args: vec!["--help".into()],
            }
        );
    }
}
