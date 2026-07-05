use anyhow::{bail, Result};

#[derive(Debug)]
pub struct CliArgs {
    pub show_logs: bool,
    pub show_help: bool,
    pub command: CommandSpec,
}

#[derive(Debug, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
}

impl Default for CommandSpec {
    fn default() -> Self {
        Self {
            program: "claude".to_string(),
            args: Vec::new(),
        }
    }
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<CliArgs> {
    let mut show_logs = false;
    let mut show_help = false;
    let mut command_tokens = Vec::new();
    let mut parsing_child_command = false;

    for arg in args {
        if parsing_child_command {
            command_tokens.push(arg);
            continue;
        }

        match arg.as_str() {
            "--" => parsing_child_command = true,
            "--help" | "-h" => show_help = true,
            "--show-logs" | "-l" => show_logs = true,
            unknown => {
                bail!(
                    "unknown claudego argument '{}'; put the command to run after '--'",
                    unknown
                );
            }
        }
    }

    let command = command_from_tokens(command_tokens);

    Ok(CliArgs {
        show_logs,
        show_help,
        command,
    })
}

pub fn help_text() -> &'static str {
    "\
claudego - run Claude in a PTY and auto-continue after rate-limit reset

Usage:
  claudego [OPTIONS]
  claudego [OPTIONS] -- <command> [args...]

Options:
  -h, --help       Show this help
  -l, --show-logs  Show live log instructions and write diagnostics to a system temp directory

Examples:
  claudego
      Run `claude`

  claudego -- claude --model opus
      Run Claude with arguments

  claudego -- caffeinate -s headroom wrap claude
      Run a wrapped Claude command

Notes:
  The rate-limit monitor always runs. If you do not want monitoring, run `claude` directly.
  Use `--` to separate claudego flags from the command it launches.
  Use `claudego -- claude --help` for Claude's own help.
"
}

fn command_from_tokens(tokens: Vec<String>) -> CommandSpec {
    let mut tokens = tokens.into_iter();
    let Some(program) = tokens.next() else {
        return CommandSpec::default();
    };

    CommandSpec {
        program,
        args: tokens.collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::{help_text, parse, CommandSpec};

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn defaults_to_claude_without_help() {
        let args = parse([]).unwrap();

        assert!(!args.show_logs);
        assert!(!args.show_help);
        assert_eq!(args.command, CommandSpec::default());
    }

    #[test]
    fn parses_claudego_flags_before_command_separator() {
        let args = parse(strings(&[
            "--show-logs",
            "--",
            "caffeinate",
            "-s",
            "headroom",
            "wrap",
            "claude",
            "--model",
            "opus",
        ]))
        .unwrap();

        assert!(args.show_logs);
        assert!(!args.show_help);
        assert_eq!(
            args.command,
            CommandSpec {
                program: "caffeinate".to_string(),
                args: strings(&["-s", "headroom", "wrap", "claude", "--model", "opus"]),
            }
        );
    }

    #[test]
    fn parses_claudego_help_before_separator() {
        let args = parse(strings(&["--help"])).unwrap();

        assert!(args.show_help);
    }

    #[test]
    fn treats_flags_after_separator_as_child_command_args() {
        let args = parse(strings(&["--", "claude", "--help"])).unwrap();

        assert!(!args.show_help);

        assert_eq!(
            args.command,
            CommandSpec {
                program: "claude".to_string(),
                args: strings(&["--help"]),
            }
        );
    }

    #[test]
    fn rejects_unknown_claudego_args_before_separator() {
        let err = parse(strings(&["--model", "opus"])).unwrap_err();

        assert!(err
            .to_string()
            .contains("put the command to run after '--'"));
    }

    #[test]
    fn help_text_mentions_claude_help_passthrough() {
        assert!(help_text().contains("claudego -- claude --help"));
    }
}
