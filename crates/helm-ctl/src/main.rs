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
    Diff(DiffArgs),
}

#[derive(Args)]
struct RootArgs {
    #[arg(long)]
    config_root: Option<PathBuf>,
}

#[derive(Args)]
struct DiffArgs {
    #[arg(long)]
    config_root: Option<PathBuf>,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
pub struct LintArgs {
    #[arg(long)]
    config_root: Option<PathBuf>,
    #[arg(long)]
    palette: Option<PathBuf>,
    #[arg(long)]
    json: bool,
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
            run_diff(&root, args.json)
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
        Ok(outcome) => {
            let report = report_apply_outcome(outcome);
            print!("{}", report.stdout);
            eprint!("{}", report.stderr);
            report.exit
        }
        Err(error) => operational_failure("apply", &error.to_string()),
    }
}

struct ApplyReport {
    exit: ExitCode,
    stdout: String,
    stderr: String,
}

fn report_apply_outcome(outcome: GenerationPublicationOutcome) -> ApplyReport {
    match outcome {
        GenerationPublicationOutcome::Committed(generation) => ApplyReport {
            exit: ExitCode::SUCCESS,
            stdout: format!("generation {} selected for future launches\n", generation.as_str()),
            stderr: String::new(),
        },
        GenerationPublicationOutcome::CommittedWithCleanupPending { generation, cause } => {
            ApplyReport {
                exit: ExitCode::SUCCESS,
                stdout: format!(
                    "generation {} selected for future launches\n",
                    generation.as_str()
                ),
                stderr: format!(
                    "warning: generation {} is durably selected for future launches; committed cleanup is pending: {}\n",
                    generation.as_str(),
                    escaped_cause(&cause),
                ),
            }
        }
        GenerationPublicationOutcome::OutcomeAmbiguous { candidate, cause } => ApplyReport {
            exit: ExitCode::from(6),
            stdout: String::new(),
            stderr: format!(
                "theme apply failed: activation is unconfirmed for candidate {}; inspect generation state before retrying: {}\n",
                candidate.as_str(),
                escaped_cause(&cause),
            ),
        },
    }
}

fn escaped_cause(cause: &str) -> String {
    serde_json::to_string(cause).expect("serializing a string cannot fail")
}

fn run_diff(root: &Path, json: bool) -> ExitCode {
    match helm_theme::diff(root) {
        Ok(changes) if changes.is_empty() => {
            if json {
                println!("{{\"status\":\"identical\",\"changes\":[]}}");
            }
            ExitCode::SUCCESS
        }
        Ok(changes) => {
            if json {
                print_diff_json(&changes);
            } else {
                for change in changes {
                    print_change(change);
                }
            }
            ExitCode::from(1)
        }
        Err(error) => operational_failure("diff", &error.to_string()),
    }
}

fn print_diff_json(changes: &[ThemeOutputChange]) {
    print!("{{\"status\":\"different\",\"changes\":[");
    for (index, change) in changes.iter().enumerate() {
        if index != 0 {
            print!(",");
        }
        let (kind, path) = match change {
            ThemeOutputChange::Added(path) => ("added", path),
            ThemeOutputChange::Removed(path) => ("removed", path),
            ThemeOutputChange::ByteDifferent(path) => ("byte-different", path),
        };
        print!(
            "{{\"kind\":{},\"path\":{}}}",
            serde_json::to_string(kind).expect("serializing a static string cannot fail"),
            serde_json::to_string(&path.to_string_lossy())
                .expect("serializing a path display string cannot fail"),
        );
    }
    println!("]}}");
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
        None => helm_theme::load_lint_palette(root).map_err(|error| error.to_string())?,
    };
    let report = helm_theme::lint(&palette);
    if args.json {
        print_lint_json(&report);
    } else {
        for separation in &report.separations {
            println!(
                "{}/{}: {:.1}°",
                separation.a, separation.b, separation.degrees
            );
        }
        for finding in &report.findings {
            eprintln!("{finding}");
        }
    }
    Ok(if report.is_clean() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn print_lint_json(report: &helm_theme::LintReport) {
    let status = if report.is_clean() { "clean" } else { "fatal" };
    print!(
        "{{\"status\":{},\"separations\":[",
        serde_json::to_string(status).unwrap()
    );
    for (index, separation) in report.separations.iter().enumerate() {
        if index != 0 {
            print!(",");
        }
        print!(
            "{{\"a\":{},\"b\":{},\"degrees\":{}}}",
            serde_json::to_string(separation.a).unwrap(),
            serde_json::to_string(separation.b).unwrap(),
            serde_json::to_string(&separation.degrees).unwrap(),
        );
    }
    print!("],\"findings\":[");
    for (index, finding) in report.findings.iter().enumerate() {
        if index != 0 {
            print!(",");
        }
        print!(
            "{{\"path\":{},\"message\":{},\"fatal\":{}}}",
            serde_json::to_string(&finding.path).unwrap(),
            serde_json::to_string(&finding.message).unwrap(),
            finding.fatal,
        );
    }
    println!("]}}");
}

fn failure(error: &str) -> ExitCode {
    eprintln!("error: {error}");
    ExitCode::from(1)
}

fn operational_failure(command: &str, error: &str) -> ExitCode {
    eprintln!("theme {command} failed: {}", escaped_cause(error));
    ExitCode::from(6)
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

#[cfg(test)]
mod tests {
    use super::*;
    use helm_theme::generation::GenerationId;

    fn generation() -> GenerationId {
        GenerationId::parse("0123456789abcdef0123456789abcdef").expect("valid generation id")
    }

    #[test]
    fn cleanup_pending_reports_selected_generation_with_escaped_warning() {
        let report =
            report_apply_outcome(GenerationPublicationOutcome::CommittedWithCleanupPending {
                generation: generation(),
                cause: "cleanup \"later\"\nretry".into(),
            });

        assert_eq!(report.exit, ExitCode::SUCCESS);
        assert_eq!(
            report.stdout,
            "generation 0123456789abcdef0123456789abcdef selected for future launches\n"
        );
        assert_eq!(
            report.stderr,
            "warning: generation 0123456789abcdef0123456789abcdef is durably selected for future launches; committed cleanup is pending: \"cleanup \\\"later\\\"\\nretry\"\n"
        );
    }

    #[test]
    fn ambiguous_reports_no_success_stdout_and_escaped_cause() {
        let report = report_apply_outcome(GenerationPublicationOutcome::OutcomeAmbiguous {
            candidate: generation(),
            cause: "pointer \"unknown\"\ninspect".into(),
        });

        assert_eq!(report.exit, ExitCode::from(6));
        assert!(report.stdout.is_empty());
        assert_eq!(
            report.stderr,
            "theme apply failed: activation is unconfirmed for candidate 0123456789abcdef0123456789abcdef; inspect generation state before retrying: \"pointer \\\"unknown\\\"\\ninspect\"\n"
        );
    }
}
