use super::stream::ClaudeStreamRunner;
use crate::cli::{command_from_args, CommandSpec};
use crate::harness::Runner;
use crate::runners::pty::PtyRunner;
use anyhow::Result;
use std::ffi::{OsStr, OsString};
use std::path::Path;

pub(super) fn command(args: Vec<OsString>) -> Result<CommandSpec> {
    command_from_args("claude", args)
}

pub(super) fn runner(command: CommandSpec) -> Box<dyn Runner> {
    if uses_stream_runner(&command) {
        Box::new(ClaudeStreamRunner::new(command))
    } else {
        Box::new(PtyRunner::new(command))
    }
}

fn uses_stream_runner(command: &CommandSpec) -> bool {
    let args = args_before_child_boundary(&command.args);
    is_claude_program(&command.program) && has_print_flag(args) && has_stream_json_output(args)
}

pub(super) fn stream_resume_command(session_id: &str) -> CommandSpec {
    CommandSpec {
        program: "claude".into(),
        args: vec![
            "--resume".into(),
            session_id.into(),
            "-p".into(),
            "--output-format".into(),
            "stream-json".into(),
            "--verbose".into(),
            "--include-partial-messages".into(),
            "continue".into(),
        ],
    }
}

fn is_claude_program(program: &OsStr) -> bool {
    Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "claude" || name == "claude.exe")
}

fn args_before_child_boundary(args: &[OsString]) -> &[OsString] {
    let boundary = args
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(args.len());
    &args[..boundary]
}

fn has_print_flag(args: &[OsString]) -> bool {
    args.iter().any(|arg| arg == "-p" || arg == "--print")
}

fn has_stream_json_output(args: &[OsString]) -> bool {
    args.windows(2)
        .any(|pair| pair[0] == "--output-format" && pair[1] == "stream-json")
        || args.iter().any(|arg| arg == "--output-format=stream-json")
}

#[cfg(test)]
mod tests {
    use super::{command, is_claude_program, stream_resume_command, uses_stream_runner};
    use crate::cli::CommandSpec;
    use std::ffi::{OsStr, OsString};

    #[test]
    fn default_claude_uses_pty() {
        let command = command(Vec::new()).unwrap();
        assert_eq!(
            command,
            CommandSpec {
                program: "claude".into(),
                args: Vec::new(),
            }
        );
        assert!(!uses_stream_runner(&command));
    }

    #[test]
    fn recognizes_claude_program_names() {
        assert!(is_claude_program(OsStr::new("claude")));
        assert!(is_claude_program(OsStr::new("/usr/local/bin/claude")));
        assert!(is_claude_program(OsStr::new("claude.exe")));
        assert!(!is_claude_program(OsStr::new("bash")));
    }

    #[test]
    fn print_mode_stream_json_uses_stream_runner() {
        assert!(uses_stream_runner(&CommandSpec {
            program: "claude".into(),
            args: vec!["-p".into(), "--output-format".into(), "stream-json".into()],
        }));
    }

    #[test]
    fn equals_form_stream_json_uses_stream_runner() {
        assert!(uses_stream_runner(&CommandSpec {
            program: "claude".into(),
            args: vec!["--print".into(), "--output-format=stream-json".into()],
        }));
    }

    #[test]
    fn stream_json_without_print_stays_pty() {
        assert!(!uses_stream_runner(&CommandSpec {
            program: "claude".into(),
            args: vec!["--output-format=stream-json".into()],
        }));
    }

    #[test]
    fn shell_wrapped_claude_stays_literal() {
        assert!(!uses_stream_runner(&CommandSpec {
            program: "bash".into(),
            args: vec!["-lc".into(), "claude -p --output-format stream-json".into()],
        }));
    }

    #[test]
    fn stream_json_flags_after_child_boundary_stay_pty() {
        assert!(!uses_stream_runner(&CommandSpec {
            program: "claude".into(),
            args: vec![
                "--".into(),
                "-p".into(),
                "--output-format".into(),
                "stream-json".into(),
            ],
        }));
    }

    #[test]
    fn equals_form_stream_json_after_child_boundary_stays_pty() {
        assert!(!uses_stream_runner(&CommandSpec {
            program: "claude".into(),
            args: vec![
                "--".into(),
                "--output-format=stream-json".into(),
                "--print".into(),
            ],
        }));
    }

    #[test]
    fn builds_minimal_stream_resume_command() {
        assert_eq!(
            stream_resume_command("abc123"),
            CommandSpec {
                program: "claude".into(),
                args: vec![
                    "--resume".into(),
                    "abc123".into(),
                    "-p".into(),
                    "--output-format".into(),
                    "stream-json".into(),
                    "--verbose".into(),
                    "--include-partial-messages".into(),
                    "continue".into(),
                ],
            }
        );
    }

    #[test]
    fn child_arguments_are_preserved() {
        assert_eq!(
            command(vec![OsString::from("--help")]).unwrap(),
            CommandSpec {
                program: "claude".into(),
                args: vec!["--help".into()]
            }
        );
    }
}
