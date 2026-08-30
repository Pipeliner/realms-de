use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{error::ErrorKind, Args, Parser, Subcommand};
use helm_core::Palette;
use helm_theme::{generation::GenerationPublicationOutcome, ThemeOutputChange};

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
struct RootArgs {
    #[arg(long)]
    config_root: Option<PathBuf>,
}

#[derive(Args)]
pub struct LintArgs {
    #[arg(long)]
    config_root: Option<PathBuf>,
    #[arg(long)]
    palette: Option<PathBuf>,
}

pub trait Env {
    fn var_os(&self, name: &str) -> Option<OsString>;
}

struct ProcessEnv;

impl Env for ProcessEnv {
    fn var_os(&self, name: &str) -> Option<OsString> {
        std::env::var_os(name)
    }
}

pub fn run<I, S>(args: I, env: &impl Env) -> ExitCode
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
            command: ThemeCommand::Apply(args),
        }) => {
            let root = match args.config_root {
                Some(root) => root,
                None => match default_config_root(env) {
                    Ok(root) => root,
                    Err(error) => return usage_error(&error),
                },
            };
            run_apply(&root)
        }
        Command::Theme(Theme {
            command: ThemeCommand::Diff(args),
        }) => {
            let root = match args.config_root {
                Some(root) => root,
                None => match default_config_root(env) {
                    Ok(root) => root,
                    Err(error) => return usage_error(&error),
                },
            };
            run_diff(&root)
        }
        Command::Theme(Theme {
            command: ThemeCommand::Lint(args),
        }) => {
            let root = match &args.palette {
                Some(_) => PathBuf::new(),
                None => match args.config_root.clone() {
                    Some(root) => root,
                    None => match default_config_root(env) {
                        Ok(root) => root,
                        Err(error) => return usage_error(&error),
                    },
                },
            };
            match run_lint(&args, &root) {
                Ok(exit) => exit,
                Err(error) => failure(&error),
            }
        }
    }
}

fn run_apply(root: &Path) -> ExitCode {
    match helm_theme::apply(root) {
        Ok(GenerationPublicationOutcome::Committed(generation)) => {
            println!(
                "generation {} selected for future launches",
                generation.as_str()
            );
            ExitCode::SUCCESS
        }
        Ok(GenerationPublicationOutcome::CommittedWithCleanupPending { generation, cause }) => {
            println!(
                "generation {} selected for future launches",
                generation.as_str()
            );
            eprintln!(
                "warning: generation {} is durably selected for future launches; committed cleanup is pending: {}",
                generation.as_str(),
                escaped_cause(&cause),
            );
            ExitCode::SUCCESS
        }
        Ok(GenerationPublicationOutcome::OutcomeAmbiguous { candidate, cause }) => {
            eprintln!(
                "theme apply failed: activation is unconfirmed for candidate {}; inspect generation state before retrying: {}",
                candidate.as_str(),
                escaped_cause(&cause),
            );
            ExitCode::from(6)
        }
        Err(error) => failure(&error.to_string()),
    }
}

fn escaped_cause(cause: &str) -> String {
    serde_json::to_string(cause).expect("serializing a string cannot fail")
}

fn run_diff(root: &Path) -> ExitCode {
    match helm_theme::diff(root) {
        Ok(changes) if changes.is_empty() => ExitCode::SUCCESS,
        Ok(changes) => {
            for change in changes {
                print_change(change);
            }
            ExitCode::from(1)
        }
        Err(error) => failure(&error.to_string()),
    }
}

fn print_change(change: ThemeOutputChange) {
    match change {
        ThemeOutputChange::Added(path) => println!("added {}", path.display()),
        ThemeOutputChange::Removed(path) => println!("removed {}", path.display()),
        ThemeOutputChange::ByteDifferent(path) => println!("byte-different {}", path.display()),
    }
}

fn default_config_root(env: &impl Env) -> Result<PathBuf, String> {
    env.var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            env.var_os("HOME")
                .map(|path| PathBuf::from(path).join(".config"))
        })
        .ok_or_else(|| "no XDG_CONFIG_HOME or HOME for helm configuration".into())
}

pub fn run_lint(args: &LintArgs, root: &Path) -> Result<ExitCode, String> {
    let palette = match &args.palette {
        Some(path) => Palette::load(path).map_err(|error| error.to_string())?,
        None => helm_theme::load_palette(root).map_err(|error| error.to_string())?,
    };
    let report = helm_theme::lint(&palette);
    for separation in &report.separations {
        println!(
            "{}/{}: {:.1}°",
            separation.a, separation.b, separation.degrees
        );
    }
    for finding in &report.findings {
        eprintln!("{finding}");
    }
    Ok(if report.is_clean() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn failure(error: &str) -> ExitCode {
    eprintln!("error: {error}");
    ExitCode::from(1)
}

fn usage_error(error: &str) -> ExitCode {
    eprintln!("error: {error}");
    ExitCode::from(2)
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
