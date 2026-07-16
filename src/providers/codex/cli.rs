use crate::cli::{command_from_args, CommandSpec};
use anyhow::Result;
use std::ffi::{OsStr, OsString};
use std::path::Path;

const INTERACTIVE_ONLY: &str = "Codex support is interactive-only; 'codex exec' is unsupported";

pub(super) fn command(args: Vec<OsString>) -> Result<CommandSpec> {
    let command = command_from_args("codex", args)?;
    if is_codex_program(&command.program)
        && command.args.first().and_then(|arg| arg.to_str()) == Some("exec")
    {
        anyhow::bail!(INTERACTIVE_ONLY);
    }
    Ok(command)
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
            vec![OsString::from("exec"), OsString::from("--json")],
            vec![
                OsString::from("--"),
                OsString::from("codex"),
                OsString::from("exec"),
            ],
            vec![
                OsString::from("--"),
                OsString::from("/opt/bin/codex"),
                OsString::from("exec"),
            ],
        ] {
            assert_eq!(command(args).unwrap_err().to_string(), INTERACTIVE_ONLY);
        }
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
