#[allow(dead_code)]
#[path = "../src/main.rs"]
mod helmctl;

use std::process::ExitCode;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use helmctl::run_from;

#[test]
fn rejects_missing_or_unknown_theme_verb() {
    assert_eq!(run_from(["helmctl", "theme"]), ExitCode::from(2));
    assert_eq!(run_from(["helmctl", "theme", "reload"]), ExitCode::from(2));
}

#[test]
fn theme_help_lists_only_theme_verbs() {
    let output = Command::new(env!("CARGO_BIN_EXE_helmctl"))
        .args(["theme", "--help"])
        .output()
        .expect("helmctl help runs");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
    assert!(stdout.contains("  apply"));
    assert!(stdout.contains("  lint"));
    assert!(stdout.contains("  diff"));
    assert!(!stdout.contains("  help"));
}

fn helmctl<I, S>(args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_helmctl"))
        .args(args)
        .output()
        .expect("helmctl runs")
}

fn helmctl_without_config_env<I, S>(args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_helmctl"))
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .args(args)
        .output()
        .expect("helmctl runs without configuration environment")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8")
}

fn write_bad_palette(root: &Path) -> PathBuf {
    let path = root.join("bad-palette.toml");
    let palette = include_str!("../../../palette.toml")
        .replace("bright = \"#f2f5fb\"", "bright = \"#101218\"")
        .replace("normal = \"#c2cbde\"", "normal = \"#101218\"");
    std::fs::write(&path, palette).expect("write invalid palette");
    path
}

fn helmctl_at<I, S>(root: &Path, args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_helmctl"))
        .args(args)
        .args(["--config-root", root.to_str().expect("UTF-8 config root")])
        .output()
        .expect("helmctl runs")
}

fn edit_palette(root: &Path, from: &str, to: &str) {
    let path = root.join("helm/palette.toml");
    let palette = std::fs::read_to_string(&path).expect("read seeded palette");
    assert!(palette.contains(from), "palette does not contain {from}");
    std::fs::write(&path, palette.replace(from, to)).expect("edit palette");
}

fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, entries: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(path).expect("read generation tree") {
            let entry = entry.expect("read tree entry");
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .expect("tree entry below root")
                .to_path_buf();
            let metadata = std::fs::symlink_metadata(&path).expect("read tree metadata");
            if metadata.file_type().is_dir() {
                visit(root, &path, entries);
            } else if metadata.file_type().is_symlink() {
                entries.insert(
                    relative,
                    std::fs::read_link(&path)
                        .expect("read tree link")
                        .into_os_string()
                        .into_encoded_bytes(),
                );
            } else {
                entries.insert(relative, std::fs::read(&path).expect("read tree file"));
            }
        }
    }

    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries);
    entries
}

fn assert_sorted_change_lines(output: &str) {
    let lines = output.lines().collect::<Vec<_>>();
    let mut sorted = lines.clone();
    sorted.sort_unstable();
    assert_eq!(lines, sorted, "diff lines are not sorted");
    assert!(lines.iter().all(|line| {
        line.starts_with("added ")
            || line.starts_with("removed ")
            || line.starts_with("byte-different ")
    }));
}

#[test]
fn lint_explicit_palette_needs_no_home_or_xdg_config_home() {
    let temp = tempfile::tempdir().expect("temporary palette root");
    let palette = temp.path().join("palette.toml");
    std::fs::write(&palette, include_str!("../../../palette.toml")).expect("write palette");

    let out = helmctl_without_config_env(["theme", "lint", "--palette", palette.to_str().unwrap()]);

    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("violet/"));
}

#[test]
fn lint_shipped_palette_is_session_independent_and_prints_hue_separations() {
    let temp = tempfile::tempdir().expect("temporary config root");
    let out = helmctl([
        "theme",
        "lint",
        "--config-root",
        temp.path().to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("violet/"));
}

#[test]
fn lint_bad_explicit_palette_exits_one_and_prints_every_fatal_finding() {
    let temp = tempfile::tempdir().expect("temporary palette root");
    let bad = write_bad_palette(temp.path());
    let out = helmctl(["theme", "lint", "--palette", bad.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    let diagnostics = stderr(&out);
    assert!(diagnostics.contains("text.bright"));
    assert!(diagnostics.contains("text.normal"));
}

#[test]
fn diff_after_palette_edit_is_sorted_and_does_not_mutate_generation_tree() {
    let temp = tempfile::tempdir().expect("temporary config root");
    let root = temp.path();
    helm_theme::apply(root).expect("apply initial generation");
    edit_palette(root, "violet    = \"#a692ec\"", "violet    = \"#b07aff\"");
    let before = tree(root);

    let out = helmctl_at(root, ["theme", "diff"]);

    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert_sorted_change_lines(&stdout(&out));
    assert_eq!(tree(root), before, "theme diff mutated the generation tree");
}
