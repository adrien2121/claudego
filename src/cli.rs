use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
}

impl CommandSpec {
    pub fn default_claude() -> Self {
        Self {
            program: "claude".to_string(),
            args: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunnerKind {
    PtyInteractive,
    StreamJsonPrint,
}

pub fn select_runner(command: &CommandSpec) -> RunnerKind {
    let classifier_args = args_before_child_boundary(&command.args);

    if is_claude_program(&command.program)
        && has_print_flag(classifier_args)
        && has_stream_json_output(classifier_args)
    {
        RunnerKind::StreamJsonPrint
    } else {
        RunnerKind::PtyInteractive
    }
}

pub fn stream_json_resume_command(session_id: &str) -> CommandSpec {
    CommandSpec {
        program: "claude".to_string(),
        args: vec![
            "--resume".to_string(),
            session_id.to_string(),
            "-p".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
            "--include-partial-messages".to_string(),
            "continue".to_string(),
        ],
    }
}

fn is_claude_program(program: &str) -> bool {
    Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "claude" || name == "claude.exe")
}

fn args_before_child_boundary(args: &[String]) -> &[String] {
    let boundary = args
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(args.len());
    &args[..boundary]
}

fn has_print_flag(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "-p" || arg == "--print")
}

fn has_stream_json_output(args: &[String]) -> bool {
    args.windows(2)
        .any(|pair| pair[0] == "--output-format" && pair[1] == "stream-json")
        || args.iter().any(|arg| arg == "--output-format=stream-json")
}

#[cfg(test)]
mod tests {
    use super::{select_runner, stream_json_resume_command, CommandSpec, RunnerKind};

    #[test]
    fn default_claude_uses_pty() {
        assert_eq!(
            select_runner(&CommandSpec::default_claude()),
            RunnerKind::PtyInteractive
        );
    }

    #[test]
    fn print_mode_stream_json_uses_stream_runner() {
        let command = CommandSpec {
            program: "claude".to_string(),
            args: vec![
                "-p".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--verbose".to_string(),
                "hello".to_string(),
            ],
        };

        assert_eq!(select_runner(&command), RunnerKind::StreamJsonPrint);
    }

    #[test]
    fn equals_form_stream_json_uses_stream_runner() {
        let command = CommandSpec {
            program: "claude".to_string(),
            args: vec![
                "--output-format=stream-json".to_string(),
                "--print".to_string(),
                "hello".to_string(),
            ],
        };

        assert_eq!(select_runner(&command), RunnerKind::StreamJsonPrint);
    }

    #[test]
    fn stream_json_without_print_stays_pty() {
        let command = CommandSpec {
            program: "claude".to_string(),
            args: vec![
                "--output-format".to_string(),
                "stream-json".to_string(),
                "hello".to_string(),
            ],
        };

        assert_eq!(select_runner(&command), RunnerKind::PtyInteractive);
    }

    #[test]
    fn shell_wrapped_claude_stays_literal() {
        let command = CommandSpec {
            program: "bash".to_string(),
            args: vec![
                "-lc".to_string(),
                "claude -p --output-format stream-json hello".to_string(),
            ],
        };

        assert_eq!(select_runner(&command), RunnerKind::PtyInteractive);
    }

    #[test]
    fn stream_json_flags_after_child_boundary_stay_pty() {
        let command = CommandSpec {
            program: "claude".to_string(),
            args: vec![
                "--".to_string(),
                "-p".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
            ],
        };

        assert_eq!(select_runner(&command), RunnerKind::PtyInteractive);
    }

    #[test]
    fn equals_form_stream_json_after_child_boundary_stays_pty() {
        let command = CommandSpec {
            program: "claude".to_string(),
            args: vec![
                "--".to_string(),
                "--output-format=stream-json".to_string(),
                "--print".to_string(),
            ],
        };

        assert_eq!(select_runner(&command), RunnerKind::PtyInteractive);
    }

    #[test]
    fn builds_minimal_stream_resume_command() {
        assert_eq!(
            stream_json_resume_command("abc123"),
            CommandSpec {
                program: "claude".to_string(),
                args: vec![
                    "--resume".to_string(),
                    "abc123".to_string(),
                    "-p".to_string(),
                    "--output-format".to_string(),
                    "stream-json".to_string(),
                    "--verbose".to_string(),
                    "--include-partial-messages".to_string(),
                    "continue".to_string(),
                ],
            }
        );
    }
}
