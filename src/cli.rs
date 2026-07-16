use anyhow::Result;
use clap::Parser;
use std::ffi::OsString;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
}

pub fn command_from_args(default_program: &str, args: Vec<OsString>) -> Result<CommandSpec> {
    if args.first().is_some_and(|arg| arg == "--") {
        let Some(program) = args.get(1).cloned() else {
            anyhow::bail!("custom command is missing after '--'");
        };
        return Ok(CommandSpec {
            program,
            args: args.into_iter().skip(2).collect(),
        });
    }
    Ok(CommandSpec {
        program: OsString::from(default_program),
        args,
    })
}

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Rate-limit-aware wrapper for Claude Code and Codex CLI",
    after_help = "Providers: claude, codex"
)]
pub struct Cli {
    /// Prevent the system from sleeping while botsitter is running.
    #[arg(long, short = 'p')]
    pub prevent_sleep: bool,

    /// Show logs in a new terminal window.
    #[arg(long, short = 'l')]
    pub show_logs: bool,

    #[arg(
        required = true,
        num_args = 1..,
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "PROVIDER [ARGS...]"
    )]
    pub provider_and_args: Vec<OsString>,
}

#[cfg(all(test, unix))]
mod tests {
    use super::command_from_args;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn command_arguments_preserve_non_utf8_bytes() {
        let value = OsString::from_vec(vec![b'a', 0xff, b'b']);
        let command = command_from_args("claude", vec![value.clone()]).unwrap();

        assert_eq!(command.args, vec![value]);
    }

    #[test]
    fn custom_program_preserves_non_utf8_bytes() {
        let program = OsString::from_vec(vec![b'c', 0xff]);
        let command =
            command_from_args("claude", vec![OsString::from("--"), program.clone()]).unwrap();

        assert_eq!(command.program, program);
    }
}
