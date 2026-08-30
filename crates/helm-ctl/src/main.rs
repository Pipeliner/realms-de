use std::{ffi::OsString, process::ExitCode};

use clap::{error::ErrorKind, Args, Parser, Subcommand};

#[derive(Parser)]
#[command(name = "helmctl")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Theme(Theme),
}

#[derive(Args)]
#[command(disable_help_subcommand = true)]
struct Theme {
    #[command(subcommand)]
    command: ThemeCommand,
}

#[derive(Subcommand)]
enum ThemeCommand {
    Apply(RootArgs),
    Lint(LintArgs),
    Diff(RootArgs),
}

#[derive(Args)]
struct RootArgs {}

#[derive(Args)]
struct LintArgs {}

pub trait Env {
    fn var_os(&self, name: &str) -> Option<OsString>;
}

struct ProcessEnv;

impl Env for ProcessEnv {
    fn var_os(&self, name: &str) -> Option<OsString> {
        std::env::var_os(name)
    }
}

pub fn run<I, S>(args: I, _env: &impl Env) -> ExitCode
where
    I: IntoIterator<Item = S>,
    S: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            let exit = match error.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => ExitCode::SUCCESS,
                _ => ExitCode::from(2),
            };
            let _ = error.print();
            return exit;
        }
    };

    match cli.command {
        Command::Theme(Theme {
            command: ThemeCommand::Apply(_),
        })
        | Command::Theme(Theme {
            command: ThemeCommand::Lint(_),
        })
        | Command::Theme(Theme {
            command: ThemeCommand::Diff(_),
        }) => ExitCode::SUCCESS,
    }
}

pub fn run_from<I, S>(args: I) -> ExitCode
where
    I: IntoIterator<Item = S>,
    S: Into<OsString> + Clone,
{
    run(args, &ProcessEnv)
}

fn main() -> ExitCode {
    run(std::env::args_os(), &ProcessEnv)
}
