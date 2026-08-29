use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

const CHECKPOINT: &str = r#"schema = "helm-agent-sdd/checkpoint/v1"
issue = 120
reason = "handoff"
created_at = "2026-08-29T12:00:00Z"
current_maturity = "probe"
requested_maturity = "spike"
git_head = "{head}"
question = "Can the focused check reproduce the result?"
success_condition = "One reproducible observation exists."
limitations = []
affected_specs = []
[goal]
statement = "State one durable task goal."
[acceptance]
criteria = ["A concrete condition exists."]
[workspace]
branch = "main"
base = "{head}"
dirty = false
[next_actions]
items = ["Inspect a named live file."]
"#;
const EVIDENCE: &str = r#"{"id":"ev-001","ts":"2026-08-29T12:00:00Z","kind":"command","summary":"Ran focused check","command":"cargo test -p helm-core","exit_code":0,"git_head":"{head}","purpose":"reproduce result"}
"#;

fn git(repo: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap().trim().into()
}
fn head(repo: &Path) -> String {
    git(repo, &["rev-parse", "HEAD"])
}
fn repo() -> tempfile::TempDir {
    let r = tempfile::tempdir().unwrap();
    git(r.path(), &["init", "--initial-branch=main"]);
    git(r.path(), &["config", "user.email", "test@example.invalid"]);
    git(r.path(), &["config", "user.name", "test"]);
    fs::write(r.path().join("README"), "fixture\n").unwrap();
    git(r.path(), &["add", "."]);
    git(r.path(), &["commit", "-m", "initial"]);
    r
}
fn write_record(repo: &Path, checkpoint: String, evidence: String) {
    let work = repo.join(".agent/work/120");
    fs::create_dir_all(&work).unwrap();
    fs::write(work.join("checkpoint.toml"), checkpoint).unwrap();
    fs::write(work.join("evidence.jsonl"), evidence).unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-m", "record"]);
}
fn valid_record(repo: &Path) {
    let parent = head(repo);
    write_record(
        repo,
        CHECKPOINT.replace("{head}", &parent),
        EVIDENCE.replace("{head}", &parent),
    );
}
fn run(repo: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_helm-sdd"))
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap()
}
fn text(o: Output) -> String {
    String::from_utf8(o.stdout).unwrap()
}

fn gate(repo: &Path) -> Output {
    run(
        repo,
        &["gate", "--issue", "120", "--from", "probe", "--to", "spike"],
    )
}

fn assert_single_failure(output: Output, code: &str) {
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(
        text(output),
        format!(
            "{{\"issue\":120,\"from\":\"probe\",\"to\":\"spike\",\"outcome\":\"invalid\",\"obligations\":[{{\"code\":\"{code}\",\"status\":\"invalid\"}}]}}\n"
        )
    );
}

fn carrier_unmet_report(transition_evidence: &str) -> String {
    format!("{{\"issue\":120,\"from\":\"probe\",\"to\":\"spike\",\"outcome\":\"unmet\",\"obligations\":[{{\"code\":\"accepted_refs\",\"status\":\"met\"}},{{\"code\":\"clean_workspace\",\"status\":\"met\"}},{{\"code\":\"decision_provenance\",\"status\":\"met\"}},{{\"code\":\"fresh_checkpoint\",\"status\":\"unmet\"}},{{\"code\":\"fresh_evidence\",\"status\":\"unmet\"}},{{\"code\":\"git_objects\",\"status\":\"met\"}},{{\"code\":\"hygiene\",\"status\":\"met\"}},{{\"code\":\"issue_directory\",\"status\":\"met\"}},{{\"code\":\"schema\",\"status\":\"met\"}},{{\"code\":\"transition\",\"status\":\"met\"}},{{\"code\":\"transition_evidence\",\"status\":\"{transition_evidence}\"}},{{\"code\":\"transition_fields\",\"status\":\"met\"}}]}}\n")
}

fn complete_report(
    outcome: &str,
    clean_workspace: &str,
    fresh_checkpoint: &str,
    fresh_evidence: &str,
    transition_evidence: &str,
    transition_fields: &str,
) -> String {
    format!("{{\"issue\":120,\"from\":\"probe\",\"to\":\"spike\",\"outcome\":\"{outcome}\",\"obligations\":[{{\"code\":\"accepted_refs\",\"status\":\"met\"}},{{\"code\":\"clean_workspace\",\"status\":\"{clean_workspace}\"}},{{\"code\":\"decision_provenance\",\"status\":\"met\"}},{{\"code\":\"fresh_checkpoint\",\"status\":\"{fresh_checkpoint}\"}},{{\"code\":\"fresh_evidence\",\"status\":\"{fresh_evidence}\"}},{{\"code\":\"git_objects\",\"status\":\"met\"}},{{\"code\":\"hygiene\",\"status\":\"met\"}},{{\"code\":\"issue_directory\",\"status\":\"met\"}},{{\"code\":\"schema\",\"status\":\"met\"}},{{\"code\":\"transition\",\"status\":\"met\"}},{{\"code\":\"transition_evidence\",\"status\":\"{transition_evidence}\"}},{{\"code\":\"transition_fields\",\"status\":\"{transition_fields}\"}}]}}\n")
}

fn status(repo: &Path) -> String {
    git(repo, &["status", "--porcelain=v1", "--untracked-files=all"])
}

fn index_snapshot(repo: &Path) -> (Vec<u8>, u64, i64, i64, i64, i64) {
    let path = repo.join(".git/index");
    let bytes = fs::read(&path).unwrap();
    let metadata = fs::metadata(path).unwrap();
    (
        bytes,
        metadata.ino(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    )
}

#[test]
fn valid_gate_emits_complete_canonical_pass_report_without_writing() {
    let r = repo();
    valid_record(r.path());
    let o = run(
        r.path(),
        &["gate", "--issue", "120", "--from", "probe", "--to", "spike"],
    );
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(text(o), "{\"issue\":120,\"from\":\"probe\",\"to\":\"spike\",\"outcome\":\"pass\",\"obligations\":[{\"code\":\"accepted_refs\",\"status\":\"met\"},{\"code\":\"clean_workspace\",\"status\":\"met\"},{\"code\":\"decision_provenance\",\"status\":\"met\"},{\"code\":\"fresh_checkpoint\",\"status\":\"met\"},{\"code\":\"fresh_evidence\",\"status\":\"met\"},{\"code\":\"git_objects\",\"status\":\"met\"},{\"code\":\"hygiene\",\"status\":\"met\"},{\"code\":\"issue_directory\",\"status\":\"met\"},{\"code\":\"schema\",\"status\":\"met\"},{\"code\":\"transition\",\"status\":\"met\"},{\"code\":\"transition_evidence\",\"status\":\"met\"},{\"code\":\"transition_fields\",\"status\":\"met\"}]}\n");
    assert!(git(
        r.path(),
        &["status", "--porcelain=v1", "--untracked-files=all"]
    )
    .is_empty());
}

#[test]
fn clean_gate_does_not_refresh_a_stale_git_index() {
    let r = repo();
    valid_record(r.path());
    std::thread::sleep(Duration::from_millis(1100));
    fs::write(r.path().join("README"), "fixture\n").unwrap();
    let before = index_snapshot(r.path());

    let output = gate(r.path());

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(index_snapshot(r.path()), before);
}
#[test]
fn promote_dry_run_is_same_read_only_assessment() {
    let r = repo();
    valid_record(r.path());
    let o = run(
        r.path(),
        &[
            "promote",
            "--dry-run",
            "--issue",
            "120",
            "--from",
            "probe",
            "--to",
            "spike",
        ],
    );
    assert_eq!(o.status.code(), Some(0));
    assert!(text(o).contains("\"outcome\":\"pass\""));
}
#[test]
fn malformed_cli_reports_only_safe_cli_metadata() {
    let o = Command::new(env!("CARGO_BIN_EXE_helm-sdd"))
        .arg("gate")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(3));
    assert_eq!(text(o), "{\"issue\":null,\"from\":null,\"to\":null,\"outcome\":\"invalid\",\"obligations\":[{\"code\":\"cli\",\"status\":\"invalid\"}]}\n");
}
#[test]
fn missing_record_reports_record_before_git_checks() {
    let r = repo();
    let o = run(
        r.path(),
        &["gate", "--issue", "120", "--from", "probe", "--to", "spike"],
    );
    assert_eq!(o.status.code(), Some(3));
    assert!(text(o).contains("\"code\":\"record\""));
}
#[test]
fn unknown_toml_field_is_schema_before_later_validation() {
    let r = repo();
    let h = head(r.path());
    write_record(
        r.path(),
        format!("{}unknown = \"no\"\n", CHECKPOINT.replace("{head}", &h)),
        EVIDENCE.replace("{head}", &h),
    );
    let o = run(
        r.path(),
        &["gate", "--issue", "120", "--from", "probe", "--to", "spike"],
    );
    assert_eq!(o.status.code(), Some(3));
    assert!(text(o).contains("\"code\":\"schema\""));
}
#[test]
fn secret_like_value_is_hygiene_and_never_echoed() {
    let r = repo();
    let h = head(r.path());
    write_record(
        r.path(),
        CHECKPOINT
            .replace("{head}", &h)
            .replace("handoff", "api_key=verysecretvalue"),
        EVIDENCE.replace("{head}", &h),
    );
    let o = run(
        r.path(),
        &["gate", "--issue", "120", "--from", "probe", "--to", "spike"],
    );
    assert_eq!(o.status.code(), Some(3));
    let report = text(o);
    assert!(report.contains("\"code\":\"hygiene\""));
    assert!(!report.contains("verysecretvalue"));
}
#[test]
fn duplicate_evidence_id_is_schema_invalid() {
    let r = repo();
    let h = head(r.path());
    let e = EVIDENCE.replace("{head}", &h);
    write_record(
        r.path(),
        CHECKPOINT.replace("{head}", &h),
        format!("{e}{e}"),
    );
    let o = run(
        r.path(),
        &["gate", "--issue", "120", "--from", "probe", "--to", "spike"],
    );
    assert_eq!(o.status.code(), Some(3));
    assert!(text(o).contains("\"code\":\"schema\""));
}

#[test]
fn duplicate_json_object_members_are_schema_invalid_even_when_the_survivor_is_safe() {
    let cases = [
        (
            "\"summary\":\"Ran focused check\"",
            "\"summary\":\"secret=hiddenvalue\",\"summary\":\"Ran focused check\"",
        ),
        (
            "\"purpose\":\"reproduce result\"",
            "\"path\":\"README\",\"path\":\"OTHER\",\"purpose\":\"reproduce result\"",
        ),
    ];
    for (needle, duplicate_members) in cases {
        let r = repo();
        let parent = head(r.path());
        write_record(
            r.path(),
            CHECKPOINT.replace("{head}", &parent),
            EVIDENCE
                .replace("{head}", &parent)
                .replace(needle, duplicate_members),
        );
        let output = gate(r.path());
        assert!(!text(output.clone()).contains("hiddenvalue"));
        assert_single_failure(output, "schema");
    }
}
#[test]
fn candidate_is_unsupported_after_schema_validation() {
    let r = repo();
    valid_record(r.path());
    let o = run(
        r.path(),
        &[
            "gate",
            "--issue",
            "120",
            "--from",
            "probe",
            "--to",
            "candidate",
        ],
    );
    assert_eq!(o.status.code(), Some(4));
    assert_eq!(text(o), "{\"issue\":120,\"from\":\"probe\",\"to\":\"candidate\",\"outcome\":\"unsupported\",\"obligations\":[{\"code\":\"accepted_refs\",\"status\":\"met\"},{\"code\":\"decision_provenance\",\"status\":\"met\"},{\"code\":\"git_objects\",\"status\":\"met\"},{\"code\":\"hygiene\",\"status\":\"met\"},{\"code\":\"issue_directory\",\"status\":\"met\"},{\"code\":\"schema\",\"status\":\"met\"},{\"code\":\"transition\",\"status\":\"unsupported\"}]}\n");
}
#[test]
fn mismatched_flags_are_invalid_transition() {
    let r = repo();
    valid_record(r.path());
    let o = run(
        r.path(),
        &[
            "gate",
            "--issue",
            "120",
            "--from",
            "spike",
            "--to",
            "prototype",
        ],
    );
    assert_eq!(o.status.code(), Some(3));
    assert!(text(o).contains("\"code\":\"transition\",\"status\":\"invalid\""));
}
#[test]
fn stale_evidence_makes_fresh_evidence_unmet() {
    let r = repo();
    let stale = head(r.path());
    fs::write(r.path().join("README"), "new parent\n").unwrap();
    git(r.path(), &["add", "README"]);
    git(r.path(), &["commit", "-m", "new parent"]);
    let parent = head(r.path());
    write_record(
        r.path(),
        CHECKPOINT.replace("{head}", &parent),
        EVIDENCE.replace("{head}", &stale),
    );
    let o = run(
        r.path(),
        &["gate", "--issue", "120", "--from", "probe", "--to", "spike"],
    );
    assert_eq!(o.status.code(), Some(2));
    assert!(text(o).contains("\"code\":\"fresh_evidence\",\"status\":\"unmet\""));
}

#[test]
fn refreshed_replacement_carrier_requires_a_complete_current_evidence_snapshot() {
    let r = repo();
    let first_parent = head(r.path());
    valid_record(r.path());
    let initial = gate(r.path());
    assert_eq!(initial.status.code(), Some(0));
    assert_eq!(
        text(initial),
        complete_report("pass", "met", "met", "met", "met", "met")
    );

    fs::write(r.path().join("README"), "new parent\n").unwrap();
    git(r.path(), &["add", "README"]);
    git(r.path(), &["commit", "-m", "non-record change"]);
    let non_record = head(r.path());

    write_record(
        r.path(),
        CHECKPOINT.replace("{head}", &non_record),
        EVIDENCE.replace("{head}", &non_record),
    );
    let refreshed = gate(r.path());
    assert_eq!(refreshed.status.code(), Some(0));
    assert_eq!(
        text(refreshed),
        complete_report("pass", "met", "met", "met", "met", "met")
    );

    git(r.path(), &["checkout", "-b", "stale-snapshot", &non_record]);
    let stale_evidence = format!(
        "{}{{\"id\":\"ev-002\",\"ts\":\"2026-08-29T12:01:00Z\",\"kind\":\"file-observation\",\"path\":\"README\",\"lines\":\"1\",\"git_head\":\"{non_record}\",\"claim\":\"The refreshed fixture exists.\"}}\n",
        EVIDENCE.replace("{head}", &first_parent)
    );
    write_record(
        r.path(),
        CHECKPOINT.replace("{head}", &non_record),
        stale_evidence,
    );
    let stale = gate(r.path());
    assert_eq!(stale.status.code(), Some(2));
    assert_eq!(
        text(stale),
        complete_report("unmet", "met", "met", "unmet", "met", "met")
    );
}

#[test]
fn dirty_workspace_is_unmet() {
    let r = repo();
    valid_record(r.path());
    fs::write(r.path().join("untracked"), "x").unwrap();
    let o = run(
        r.path(),
        &["gate", "--issue", "120", "--from", "probe", "--to", "spike"],
    );
    assert_eq!(o.status.code(), Some(2));
    assert!(text(o).contains("\"code\":\"clean_workspace\",\"status\":\"unmet\""));
}

#[test]
fn root_commit_has_no_freshness_or_transition_evidence() {
    let r = repo();
    let dangling = head(r.path());
    git(r.path(), &["checkout", "--orphan", "root-record"]);
    git(r.path(), &["rm", "--cached", "README"]);
    fs::remove_file(r.path().join("README")).unwrap();
    let work = r.path().join(".agent/work/120");
    fs::create_dir_all(&work).unwrap();
    fs::write(
        work.join("checkpoint.toml"),
        CHECKPOINT.replace("{head}", &dangling),
    )
    .unwrap();
    fs::write(
        work.join("evidence.jsonl"),
        EVIDENCE.replace("{head}", &dangling),
    )
    .unwrap();
    git(r.path(), &["add", "."]);
    git(r.path(), &["commit", "-m", "root record"]);

    let output = gate(r.path());
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(text(output), carrier_unmet_report("unmet"));
}

#[test]
fn merge_commit_has_no_freshness_or_transition_evidence() {
    let r = repo();
    valid_record(r.path());
    git(r.path(), &["branch", "side", "HEAD~1"]);
    git(r.path(), &["checkout", "side"]);
    fs::write(r.path().join("SIDE"), "side\n").unwrap();
    git(r.path(), &["add", "SIDE"]);
    git(r.path(), &["commit", "-m", "side"]);
    git(r.path(), &["checkout", "main"]);
    git(r.path(), &["merge", "--no-ff", "side", "-m", "merge"]);

    let output = gate(r.path());
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(text(output), carrier_unmet_report("unmet"));
}

#[test]
fn carrier_that_adds_an_outside_path_is_not_fresh_but_parent_evidence_is_usable() {
    let r = repo();
    let parent = head(r.path());
    let work = r.path().join(".agent/work/120");
    fs::create_dir_all(&work).unwrap();
    fs::write(
        work.join("checkpoint.toml"),
        CHECKPOINT.replace("{head}", &parent),
    )
    .unwrap();
    fs::write(
        work.join("evidence.jsonl"),
        EVIDENCE.replace("{head}", &parent),
    )
    .unwrap();
    fs::write(r.path().join("OUTSIDE"), "outside\n").unwrap();
    git(r.path(), &["add", "."]);
    git(r.path(), &["commit", "-m", "record and outside"]);

    let output = gate(r.path());
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(text(output), carrier_unmet_report("met"));
}

#[test]
fn executable_record_blob_is_not_a_carrier() {
    let r = repo();
    let parent = head(r.path());
    let work = r.path().join(".agent/work/120");
    fs::create_dir_all(&work).unwrap();
    fs::write(
        work.join("checkpoint.toml"),
        CHECKPOINT.replace("{head}", &parent),
    )
    .unwrap();
    fs::write(
        work.join("evidence.jsonl"),
        EVIDENCE.replace("{head}", &parent),
    )
    .unwrap();
    git(r.path(), &["add", "."]);
    git(
        r.path(),
        &[
            "update-index",
            "--chmod=+x",
            ".agent/work/120/checkpoint.toml",
        ],
    );
    let checkpoint = work.join("checkpoint.toml");
    let mut permissions = fs::metadata(&checkpoint).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(checkpoint, permissions).unwrap();
    git(r.path(), &["commit", "-m", "executable record"]);

    let output = gate(r.path());
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(text(output), carrier_unmet_report("met"));
}

#[test]
fn outside_path_mode_change_is_not_a_carrier() {
    let r = repo();
    let parent = head(r.path());
    let work = r.path().join(".agent/work/120");
    fs::create_dir_all(&work).unwrap();
    fs::write(
        work.join("checkpoint.toml"),
        CHECKPOINT.replace("{head}", &parent),
    )
    .unwrap();
    fs::write(
        work.join("evidence.jsonl"),
        EVIDENCE.replace("{head}", &parent),
    )
    .unwrap();
    git(r.path(), &["add", "."]);
    git(r.path(), &["update-index", "--chmod=+x", "README"]);
    let readme = r.path().join("README");
    let mut permissions = fs::metadata(&readme).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(readme, permissions).unwrap();
    git(r.path(), &["commit", "-m", "record and outside mode"]);

    let output = gate(r.path());
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(text(output), carrier_unmet_report("met"));
}

#[test]
fn executable_parent_record_blob_prevents_a_carrier() {
    let r = repo();
    let initial = head(r.path());
    write_record(
        r.path(),
        CHECKPOINT.replace("{head}", &initial),
        EVIDENCE.replace("{head}", &initial),
    );
    git(
        r.path(),
        &[
            "update-index",
            "--chmod=+x",
            ".agent/work/120/checkpoint.toml",
        ],
    );
    git(r.path(), &["commit", "--amend", "--no-edit"]);
    let parent = head(r.path());
    fs::write(
        r.path().join(".agent/work/120/checkpoint.toml"),
        CHECKPOINT.replace("{head}", &parent),
    )
    .unwrap();
    fs::write(
        r.path().join(".agent/work/120/evidence.jsonl"),
        EVIDENCE.replace("{head}", &parent),
    )
    .unwrap();
    git(r.path(), &["add", "."]);
    git(
        r.path(),
        &[
            "update-index",
            "--chmod=-x",
            ".agent/work/120/checkpoint.toml",
        ],
    );
    git(r.path(), &["commit", "-m", "regular replacement"]);

    let output = gate(r.path());
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(text(output), carrier_unmet_report("met"));
}

#[test]
fn unknown_record_file_is_hygiene_not_schema() {
    let r = repo();
    valid_record(r.path());
    fs::write(r.path().join(".agent/work/120/state.md"), "forbidden\n").unwrap();
    git(r.path(), &["add", "."]);
    git(r.path(), &["commit", "-m", "unknown record file"]);
    assert_single_failure(gate(r.path()), "hygiene");
}

#[test]
fn schema_rejects_wrong_types_bounds_unknown_keys_and_duplicate_ids() {
    let cases = [
        ("issue = 120", "issue = 0"),
        (
            "limitations = []",
            &format!("limitations = [{}]", vec!["\"item\""; 17].join(", ")),
        ),
        (
            "criteria = [\"A concrete condition exists.\"]",
            "criteria = []",
        ),
        ("items = [\"Inspect a named live file.\"]", "items = []"),
        ("dirty = false", "dirty = \"false\""),
    ];
    for (needle, replacement) in cases {
        let r = repo();
        let parent = head(r.path());
        write_record(
            r.path(),
            CHECKPOINT
                .replace("{head}", &parent)
                .replace(needle, replacement),
            EVIDENCE.replace("{head}", &parent),
        );
        assert_single_failure(gate(r.path()), "schema");
    }

    let r = repo();
    let parent = head(r.path());
    let evidence = EVIDENCE.replace("{head}", &parent);
    write_record(
        r.path(),
        CHECKPOINT.replace("{head}", &parent),
        format!("{evidence}{evidence}"),
    );
    assert_single_failure(gate(r.path()), "schema");
}

#[test]
fn schema_rejects_every_checkpoint_scalar_enum_and_array_table_boundary() {
    let cases = [
        (
            "schema = \"helm-agent-sdd/checkpoint/v1\"",
            "schema = \"wrong\"",
        ),
        ("reason = \"handoff\"", "reason = 7"),
        (
            "created_at = \"2026-08-29T12:00:00Z\"",
            "created_at = \"2026-08-29T12:00:00+00:00\"",
        ),
        (
            "current_maturity = \"probe\"",
            "current_maturity = \"candidate\"",
        ),
        (
            "requested_maturity = \"spike\"",
            "requested_maturity = \"production\"",
        ),
        (
            "question = \"Can the focused check reproduce the result?\"",
            "question = true",
        ),
        (
            "success_condition = \"One reproducible observation exists.\"",
            "success_condition = 1",
        ),
        ("limitations = []", "limitations = [1]"),
        ("affected_specs = []", "affected_specs = [1]"),
        (
            "statement = \"State one durable task goal.\"",
            "statement = false",
        ),
        (
            "criteria = [\"A concrete condition exists.\"]",
            &format!("criteria = [{}]", vec!["\"item\""; 17].join(", ")),
        ),
        ("branch = \"main\"", "branch = 1"),
        ("base = \"{head}\"", "base = false"),
        (
            "items = [\"Inspect a named live file.\"]",
            "items = [\"one\", \"two\", \"three\", \"four\"]",
        ),
    ];
    for (needle, replacement) in cases {
        let r = repo();
        let parent = head(r.path());
        write_record(
            r.path(),
            CHECKPOINT
                .replace(needle, replacement)
                .replace("{head}", &parent),
            EVIDENCE.replace("{head}", &parent),
        );
        assert_single_failure(gate(r.path()), "schema");
    }

    let optional_tables = [
        "\n[[completed]]\nclaim = \"Recorded result.\"\nevidence = [\"ev-001\"]\n",
        "\n[[hypotheses]]\nid = \"H1\"\nstatement = \"A proposition.\"\nconfidence = \"low\"\n",
        "\n[[rejected_approaches]]\napproach = \"Try another route.\"\nreason = \"It was not reproducible.\"\nevidence = [\"ev-001\"]\n",
    ];
    for table in optional_tables {
        let r = repo();
        let parent = head(r.path());
        let checkpoint = CHECKPOINT.replace("{head}", &parent);
        write_record(
            r.path(),
            format!("{checkpoint}{table}unknown = \"field\"\n"),
            EVIDENCE.replace("{head}", &parent),
        );
        assert_single_failure(gate(r.path()), "schema");
    }
}

#[test]
fn schema_rejects_every_evidence_scalar_and_collection_boundary() {
    let base = EVIDENCE.trim_end();
    let cases = [
        ("\"id\":\"ev-001\"", "\"id\":\"ev-000\""),
        ("\"id\":\"ev-001\"", "\"id\":1"),
        (
            "\"ts\":\"2026-08-29T12:00:00Z\"",
            "\"ts\":\"2026-08-29T12:00:00+00:00\"",
        ),
        ("\"summary\":\"Ran focused check\"", "\"summary\":false"),
        ("\"command\":\"cargo test -p helm-core\"", "\"command\":1"),
        ("\"exit_code\":0", "\"exit_code\":256"),
        ("\"exit_code\":0", "\"exit_code\":1.5"),
        ("\"purpose\":\"reproduce result\"", "\"purpose\":null"),
    ];
    for (needle, replacement) in cases {
        let r = repo();
        let parent = head(r.path());
        write_record(
            r.path(),
            CHECKPOINT.replace("{head}", &parent),
            format!(
                "{}\n",
                base.replace("{head}", &parent).replace(needle, replacement)
            ),
        );
        assert_single_failure(gate(r.path()), "schema");
    }

    let r = repo();
    let parent = head(r.path());
    let file_observation = format!(
        "{{\"id\":\"ev-001\",\"ts\":\"2026-08-29T12:00:00Z\",\"kind\":\"file-observation\",\"path\":\"README\",\"lines\":7,\"git_head\":\"{parent}\",\"claim\":\"The fixture exists.\"}}\n"
    );
    write_record(
        r.path(),
        CHECKPOINT.replace("{head}", &parent),
        file_observation,
    );
    assert_single_failure(gate(r.path()), "schema");

    let decision_cases = [
        "[]".to_owned(),
        "[\"ev-001\",null]".to_owned(),
        format!("[{}]", vec!["\"ev-001\""; 17].join(",")),
    ];
    for derived_from in decision_cases {
        let r = repo();
        let parent = head(r.path());
        let evidence = format!(
            "{{\"id\":\"ev-001\",\"ts\":\"2026-08-29T12:00:00Z\",\"kind\":\"decision\",\"git_head\":\"{parent}\",\"claim\":\"Choose the focused route.\",\"reason\":\"It is reproducible.\",\"derived_from\":{derived_from}}}\n"
        );
        write_record(r.path(), CHECKPOINT.replace("{head}", &parent), evidence);
        assert_single_failure(gate(r.path()), "schema");
    }
}

#[test]
fn hygiene_rejects_invalid_values_in_every_string_class_without_echoing_them() {
    let prose_cases = [
        "Contains a source delimiter ` here.",
        "Contains three---hyphens.",
        "Contains a tab\there.",
        "Contains non ASCII cafe\u{301}.",
        &"x".repeat(241),
    ];
    for rejected in prose_cases {
        let r = repo();
        let parent = head(r.path());
        write_record(
            r.path(),
            CHECKPOINT
                .replace("{head}", &parent)
                .replace("State one durable task goal.", rejected),
            EVIDENCE.replace("{head}", &parent),
        );
        assert_single_failure(gate(r.path()), "hygiene");
    }

    let command_cases = [
        "/usr/bin/git status",
        "-cargo test",
        "cargo ../test",
        "cargo test /tmp",
        "cargo test fn",
        "cargo test *",
        &"x".repeat(513),
    ];
    for rejected in command_cases {
        let r = repo();
        let parent = head(r.path());
        write_record(
            r.path(),
            CHECKPOINT.replace("{head}", &parent),
            EVIDENCE
                .replace("{head}", &parent)
                .replace("cargo test -p helm-core", rejected),
        );
        assert_single_failure(gate(r.path()), "hygiene");
    }
}

#[test]
fn malformed_document_prefixed_criteria_must_satisfy_the_prose_grammar() {
    for delimiter in [
        "`", "$", "{", "}", ";", "\\", "\"", "'", "<", ">", "|", "&", ":", "=", "---",
    ] {
        let r = repo();
        let parent = head(r.path());
        let criterion = format!("SPEC-0008{delimiter}copied source");
        let encoded = serde_json::to_string(&criterion).unwrap();
        write_record(
            r.path(),
            CHECKPOINT.replace("{head}", &parent).replace(
                "criteria = [\"A concrete condition exists.\"]",
                &format!("criteria = [{encoded}]"),
            ),
            EVIDENCE.replace("{head}", &parent),
        );
        assert_single_failure(gate(r.path()), "hygiene");
    }
}

#[test]
fn hygiene_enforces_branch_path_and_line_grammars() {
    let branch_cases = ["/main", "main..next", "main/", "main.", "main branch"];
    for rejected in branch_cases {
        let r = repo();
        let parent = head(r.path());
        write_record(
            r.path(),
            CHECKPOINT
                .replace("{head}", &parent)
                .replace("branch = \"main\"", &format!("branch = \"{rejected}\"")),
            EVIDENCE.replace("{head}", &parent),
        );
        assert_single_failure(gate(r.path()), "hygiene");
    }

    let path_cases = ["/README", "a//b", "a/./b", "a/../b", ".hidden", "a path"];
    for rejected in path_cases {
        let r = repo();
        let parent = head(r.path());
        let evidence = format!(
            "{{\"id\":\"ev-001\",\"ts\":\"2026-08-29T12:00:00Z\",\"kind\":\"file-observation\",\"path\":\"{rejected}\",\"lines\":\"1\",\"git_head\":\"{parent}\",\"claim\":\"The fixture exists.\"}}\n"
        );
        write_record(r.path(), CHECKPOINT.replace("{head}", &parent), evidence);
        assert_single_failure(gate(r.path()), "hygiene");
    }

    for rejected in ["0", "01", "2-1", "1-0", "1-2-3", "1-"] {
        let r = repo();
        let parent = head(r.path());
        let evidence = format!(
            "{{\"id\":\"ev-001\",\"ts\":\"2026-08-29T12:00:00Z\",\"kind\":\"file-observation\",\"path\":\"README\",\"lines\":\"{rejected}\",\"git_head\":\"{parent}\",\"claim\":\"The fixture exists.\"}}\n"
        );
        write_record(r.path(), CHECKPOINT.replace("{head}", &parent), evidence);
        assert_single_failure(gate(r.path()), "hygiene");
    }
}

#[test]
fn hygiene_applies_each_exact_detector_pattern_to_every_string_value() {
    let detector_cases = [
        "scan -----BEGIN PRIVATE KEY----- now",
        "inspect aaaaaaaaaa.bbbbbbbbbb.cccccccccc now",
        "check AUTHORIZATION : sensitivevalue now",
    ];
    for rejected in detector_cases {
        let r = repo();
        let parent = head(r.path());
        write_record(
            r.path(),
            CHECKPOINT.replace("{head}", &parent),
            EVIDENCE
                .replace("{head}", &parent)
                .replace("cargo test -p helm-core", rejected),
        );
        assert_single_failure(gate(r.path()), "hygiene");
    }
}

#[test]
fn valid_edge_values_for_each_string_class_are_accepted() {
    let r = repo();
    let parent = head(r.path());
    let checkpoint = CHECKPOINT
        .replace("{head}", &parent)
        .replace("branch = \"main\"", "branch = \"feature/a_b.c-1\"");
    let evidence = format!(
        "{{\"id\":\"ev-01\",\"ts\":\"2024-02-29T23:59:59Z\",\"kind\":\"command\",\"summary\":\"Ran focused check\",\"command\":\"cargo test -p helm-core test_name @tag%1+2,3\",\"exit_code\":0,\"git_head\":\"{parent}\",\"purpose\":\"reproduce result\",\"path\":\"crates/helm_core.test-1\"}}\n{{\"id\":\"ev-2\",\"ts\":\"2026-08-29T12:00:00Z\",\"kind\":\"file-observation\",\"path\":\"crates/helm-core/src/ipc.rs\",\"lines\":\"999999999999999999999-1000000000000000000000\",\"git_head\":\"{parent}\",\"claim\":\"The fixture exists.\",\"summary\":\"Inspected focused lines.\"}}\n"
    );
    write_record(r.path(), checkpoint, evidence);
    let output = gate(r.path());
    assert_eq!(output.status.code(), Some(0));
    assert!(text(output).contains("\"outcome\":\"pass\""));
}

#[test]
fn detector_does_not_overmatch_a_single_qualifier_space() {
    let r = repo();
    let parent = head(r.path());
    write_record(
        r.path(),
        CHECKPOINT.replace("{head}", &parent),
        EVIDENCE.replace("{head}", &parent).replace(
            "cargo test -p helm-core",
            "scan -----BEGIN  PRIVATE KEY----- now",
        ),
    );
    assert_eq!(gate(r.path()).status.code(), Some(0));
}

fn add_document(repo: &Path, path: &str, title: &str, status: &str) {
    let path = repo.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, format!("{title}\n\n- **Status:** {status}\n")).unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-m", "document"]);
}

#[test]
fn accepted_spec_and_adr_references_resolve_from_head() {
    let r = repo();
    add_document(
        r.path(),
        "docs/specs/0008-agent-sdd.md",
        "# SPEC 0008 — Agent SDD",
        "Accepted (2026-08-29)",
    );
    add_document(
        r.path(),
        "docs/adr/0014-agent-sdd.md",
        "# ADR 0014 — Agent SDD",
        "Accepted",
    );
    let parent = head(r.path());
    let checkpoint = CHECKPOINT
        .replace("{head}", &parent)
        .replace("affected_specs = []", "affected_specs = [\"SPEC-0008\"]")
        .replace(
            "criteria = [\"A concrete condition exists.\"]",
            "criteria = [\"ADR-0014\"]",
        );
    write_record(r.path(), checkpoint, EVIDENCE.replace("{head}", &parent));
    let output = gate(r.path());
    assert_eq!(output.status.code(), Some(0));
    assert!(text(output).contains("\"accepted_refs\",\"status\":\"met\""));
}

#[test]
fn accepted_reference_rejects_malformed_missing_ambiguous_or_nonaccepted_documents() {
    for malformed in ["SPEC-008", "ADR-0000", "spec-0008"] {
        let r = repo();
        let parent = head(r.path());
        write_record(
            r.path(),
            CHECKPOINT.replace("{head}", &parent).replace(
                "affected_specs = []",
                &format!("affected_specs = [\"{malformed}\"]"),
            ),
            EVIDENCE.replace("{head}", &parent),
        );
        assert_single_failure(gate(r.path()), "accepted_refs");
    }

    let cases = [
        ("# SPEC 0008 — Agent SDD", "Draft"),
        ("# SPEC 0008 - Agent SDD", "Accepted"),
        ("# SPEC 0008 — Agent SDD", "Accepted (2026-8-29)"),
    ];
    for (title, status) in cases {
        let r = repo();
        add_document(r.path(), "docs/specs/0008-agent-sdd.md", title, status);
        let parent = head(r.path());
        write_record(
            r.path(),
            CHECKPOINT
                .replace("{head}", &parent)
                .replace("affected_specs = []", "affected_specs = [\"SPEC-0008\"]"),
            EVIDENCE.replace("{head}", &parent),
        );
        assert_single_failure(gate(r.path()), "accepted_refs");
    }

    let r = repo();
    add_document(
        r.path(),
        "docs/specs/0008-one.md",
        "# SPEC 0008 — One",
        "Accepted",
    );
    add_document(
        r.path(),
        "docs/specs/0008-two.md",
        "# SPEC 0008 — Two",
        "Accepted",
    );
    let parent = head(r.path());
    write_record(
        r.path(),
        CHECKPOINT
            .replace("{head}", &parent)
            .replace("affected_specs = []", "affected_specs = [\"SPEC-0008\"]"),
        EVIDENCE.replace("{head}", &parent),
    );
    assert_single_failure(gate(r.path()), "accepted_refs");
}

#[test]
fn accepted_reference_uses_the_document_at_assessment_head_not_its_parent() {
    let r = repo();
    add_document(
        r.path(),
        "docs/specs/0008-agent-sdd.md",
        "# SPEC 0008 — Agent SDD",
        "Accepted",
    );
    let parent = head(r.path());
    let work = r.path().join(".agent/work/120");
    fs::create_dir_all(&work).unwrap();
    fs::write(
        work.join("checkpoint.toml"),
        CHECKPOINT
            .replace("{head}", &parent)
            .replace("affected_specs = []", "affected_specs = [\"SPEC-0008\"]"),
    )
    .unwrap();
    fs::write(
        work.join("evidence.jsonl"),
        EVIDENCE.replace("{head}", &parent),
    )
    .unwrap();
    fs::write(
        r.path().join("docs/specs/0008-agent-sdd.md"),
        "# SPEC 0008 — Agent SDD\n\n- **Status:** Draft\n",
    )
    .unwrap();
    git(r.path(), &["add", "."]);
    git(r.path(), &["commit", "-m", "record with changed document"]);
    assert_single_failure(gate(r.path()), "accepted_refs");
}

#[test]
fn accepted_reference_requires_exact_lf_terminated_title_and_status_lines() {
    let r = repo();
    let path = r.path().join("docs/specs/0008-agent-sdd.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        "# SPEC 0008 — Agent SDD\r\n\r\n- **Status:** Accepted\r\n",
    )
    .unwrap();
    git(r.path(), &["add", "."]);
    git(r.path(), &["commit", "-m", "crlf document"]);
    let parent = head(r.path());
    write_record(
        r.path(),
        CHECKPOINT
            .replace("{head}", &parent)
            .replace("affected_specs = []", "affected_specs = [\"SPEC-0008\"]"),
        EVIDENCE.replace("{head}", &parent),
    );
    assert_single_failure(gate(r.path()), "accepted_refs");
}

#[test]
fn accepted_reference_requires_a_direct_child_governance_document() {
    let cases = [
        (
            "docs/specs/0008-nested/spec.md",
            "# SPEC 0008 — Nested",
            "affected_specs = []",
            "affected_specs = [\"SPEC-0008\"]",
        ),
        (
            "docs/adr/0014-nested/adr.md",
            "# ADR 0014 — Nested",
            "criteria = [\"A concrete condition exists.\"]",
            "criteria = [\"ADR-0014\"]",
        ),
    ];
    for (path, title, needle, replacement) in cases {
        let r = repo();
        add_document(r.path(), path, title, "Accepted");
        let parent = head(r.path());
        write_record(
            r.path(),
            CHECKPOINT
                .replace("{head}", &parent)
                .replace(needle, replacement),
            EVIDENCE.replace("{head}", &parent),
        );
        assert_single_failure(gate(r.path()), "accepted_refs");
    }
}

#[test]
fn valid_decision_uses_earlier_same_revision_nondecision_evidence() {
    let r = repo();
    let parent = head(r.path());
    let evidence = format!(
        "{}{{\"id\":\"ev-002\",\"ts\":\"2026-08-29T12:01:00Z\",\"kind\":\"decision\",\"git_head\":\"{parent}\",\"claim\":\"Choose the focused route.\",\"reason\":\"It is reproducible.\",\"derived_from\":[\"ev-001\"]}}\n",
        EVIDENCE.replace("{head}", &parent)
    );
    write_record(r.path(), CHECKPOINT.replace("{head}", &parent), evidence);
    let output = gate(r.path());
    assert_eq!(output.status.code(), Some(0));
    assert!(text(output).contains("\"decision_provenance\",\"status\":\"met\""));
}

#[test]
fn decision_provenance_rejects_missing_later_decision_and_cross_revision_parents() {
    let r = repo();
    let old = head(r.path());
    fs::write(r.path().join("README"), "new parent\n").unwrap();
    git(r.path(), &["add", "README"]);
    git(r.path(), &["commit", "-m", "new parent"]);
    let parent = head(r.path());
    let invalid_sets = [
        format!("{{\"id\":\"ev-001\",\"ts\":\"2026-08-29T12:00:00Z\",\"kind\":\"decision\",\"git_head\":\"{parent}\",\"claim\":\"Choose the route.\",\"reason\":\"It is reproducible.\",\"derived_from\":[\"ev-999\"]}}\n"),
        format!("{{\"id\":\"ev-001\",\"ts\":\"2026-08-29T12:00:00Z\",\"kind\":\"decision\",\"git_head\":\"{parent}\",\"claim\":\"Choose the route.\",\"reason\":\"It is reproducible.\",\"derived_from\":[\"ev-002\"]}}\n{{\"id\":\"ev-002\",\"ts\":\"2026-08-29T12:01:00Z\",\"kind\":\"command\",\"summary\":\"Ran focused check\",\"command\":\"cargo test -p helm-core\",\"exit_code\":0,\"git_head\":\"{parent}\",\"purpose\":\"reproduce result\"}}\n"),
        format!("{{\"id\":\"ev-001\",\"ts\":\"2026-08-29T12:00:00Z\",\"kind\":\"decision\",\"git_head\":\"{parent}\",\"claim\":\"First decision.\",\"reason\":\"It is reproducible.\",\"derived_from\":[\"ev-002\"]}}\n{{\"id\":\"ev-002\",\"ts\":\"2026-08-29T12:01:00Z\",\"kind\":\"decision\",\"git_head\":\"{parent}\",\"claim\":\"Second decision.\",\"reason\":\"It is reproducible.\",\"derived_from\":[\"ev-001\"]}}\n"),
        format!("{{\"id\":\"ev-001\",\"ts\":\"2026-08-29T12:00:00Z\",\"kind\":\"command\",\"summary\":\"Ran focused check\",\"command\":\"cargo test -p helm-core\",\"exit_code\":0,\"git_head\":\"{old}\",\"purpose\":\"reproduce result\"}}\n{{\"id\":\"ev-002\",\"ts\":\"2026-08-29T12:01:00Z\",\"kind\":\"decision\",\"git_head\":\"{parent}\",\"claim\":\"Choose the route.\",\"reason\":\"It is reproducible.\",\"derived_from\":[\"ev-001\"]}}\n"),
    ];
    for evidence in invalid_sets {
        let fixture = repo();
        let fixture_old = head(fixture.path());
        fs::write(fixture.path().join("README"), "new parent\n").unwrap();
        git(fixture.path(), &["add", "README"]);
        git(fixture.path(), &["commit", "-m", "new parent"]);
        let fixture_parent = head(fixture.path());
        write_record(
            fixture.path(),
            CHECKPOINT.replace("{head}", &fixture_parent),
            evidence
                .replace(&parent, &fixture_parent)
                .replace(&old, &fixture_old),
        );
        assert_single_failure(gate(fixture.path()), "decision_provenance");
    }
}

#[test]
fn every_step_two_failure_has_exact_precedence_and_safe_output() {
    let r = repo();
    let parent = head(r.path());
    write_record(
        r.path(),
        CHECKPOINT
            .replace("{head}", &parent)
            .replace("issue = 120", "issue = 121"),
        EVIDENCE.replace("{head}", &parent),
    );
    assert_single_failure(gate(r.path()), "issue_directory");

    let r = repo();
    let invalid = "ffffffffffffffffffffffffffffffffffffffff";
    write_record(
        r.path(),
        CHECKPOINT.replace("{head}", invalid),
        EVIDENCE.replace("{head}", invalid),
    );
    let output = gate(r.path());
    assert!(output.stderr.is_empty());
    assert_single_failure(output, "git_objects");

    let r = repo();
    let parent = head(r.path());
    write_record(
        r.path(),
        CHECKPOINT.replace("{head}", &parent).replace(
            &format!("base = \"{parent}\""),
            "base = \"ffffffffffffffffffffffffffffffffffffffff\"",
        ),
        EVIDENCE.replace("{head}", &parent),
    );
    assert_single_failure(gate(r.path()), "git_objects");

    let r = repo();
    let parent = head(r.path());
    write_record(
        r.path(),
        CHECKPOINT
            .replace("{head}", "not-a-commit")
            .replace("handoff", "bad=value"),
        EVIDENCE.replace("{head}", &parent),
    );
    assert_single_failure(gate(r.path()), "hygiene");
}

#[test]
fn non_utf8_record_content_is_schema_invalid_not_a_missing_record() {
    let r = repo();
    let work = r.path().join(".agent/work/120");
    fs::create_dir_all(&work).unwrap();
    fs::write(work.join("checkpoint.toml"), [0xff]).unwrap();
    fs::write(work.join("evidence.jsonl"), b"{}\n").unwrap();
    assert_single_failure(gate(r.path()), "schema");
}

#[test]
fn malformed_cli_and_missing_record_reports_are_exact_and_read_only() {
    let malformed: &[&[&str]] = &[
        &[],
        &["gate"],
        &[
            "promote", "--issue", "120", "--from", "probe", "--to", "spike",
        ],
        &["gate", "--issue", "0", "--from", "probe", "--to", "spike"],
        &[
            "gate", "--issue", "0120", "--from", "probe", "--to", "spike",
        ],
        &[
            "gate",
            "--issue",
            "2147483648",
            "--from",
            "probe",
            "--to",
            "spike",
        ],
        &[
            "gate", "--issue", "120", "--from", "unknown", "--to", "spike",
        ],
        &["gate", "--from", "probe", "--issue", "120", "--to", "spike"],
    ];
    for args in malformed {
        let output = Command::new(env!("CARGO_BIN_EXE_helm-sdd"))
            .args(*args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(3));
        assert!(output.stderr.is_empty());
        assert_eq!(text(output), "{\"issue\":null,\"from\":null,\"to\":null,\"outcome\":\"invalid\",\"obligations\":[{\"code\":\"cli\",\"status\":\"invalid\"}]}\n");
    }

    let r = repo();
    let before = status(r.path());
    let output = gate(r.path());
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(text(output), "{\"issue\":120,\"from\":\"probe\",\"to\":\"spike\",\"outcome\":\"invalid\",\"obligations\":[{\"code\":\"record\",\"status\":\"invalid\"}]}\n");
    assert_eq!(status(r.path()), before);
}

#[test]
fn stale_checkpoint_and_nonancestor_base_have_exact_freshness_vectors() {
    let r = repo();
    let stale = head(r.path());
    fs::write(r.path().join("README"), "new parent\n").unwrap();
    git(r.path(), &["add", "README"]);
    git(r.path(), &["commit", "-m", "new parent"]);
    let parent = head(r.path());
    write_record(
        r.path(),
        CHECKPOINT.replace("{head}", &parent).replace(
            &format!("git_head = \"{parent}\""),
            &format!("git_head = \"{stale}\""),
        ),
        EVIDENCE.replace("{head}", &parent),
    );
    let output = gate(r.path());
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        text(output),
        complete_report("unmet", "met", "unmet", "met", "met", "met")
    );

    let r = repo();
    let main = head(r.path());
    git(r.path(), &["checkout", "--orphan", "unrelated"]);
    git(r.path(), &["rm", "--cached", "README"]);
    fs::remove_file(r.path().join("README")).unwrap();
    fs::write(r.path().join("OTHER"), "unrelated\n").unwrap();
    git(r.path(), &["add", "."]);
    git(r.path(), &["commit", "-m", "unrelated"]);
    let unrelated = head(r.path());
    git(r.path(), &["checkout", "main"]);
    write_record(
        r.path(),
        CHECKPOINT.replace("{head}", &main).replace(
            &format!("base = \"{main}\""),
            &format!("base = \"{unrelated}\""),
        ),
        EVIDENCE.replace("{head}", &main),
    );
    let output = gate(r.path());
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        text(output),
        complete_report("unmet", "met", "unmet", "met", "met", "met")
    );
}

#[test]
fn index_tracked_and_declared_dirty_states_each_make_workspace_unmet_without_writes() {
    for dirty_kind in ["index", "tracked", "declared"] {
        let r = repo();
        valid_record(r.path());
        match dirty_kind {
            "index" => {
                fs::write(r.path().join("README"), "index dirty\n").unwrap();
                git(r.path(), &["add", "README"]);
            }
            "tracked" => fs::write(r.path().join("README"), "tracked dirty\n").unwrap(),
            "declared" => {
                let checkpoint = r.path().join(".agent/work/120/checkpoint.toml");
                let value = fs::read_to_string(&checkpoint).unwrap();
                fs::write(checkpoint, value.replace("dirty = false", "dirty = true")).unwrap();
            }
            _ => unreachable!(),
        }
        let before = status(r.path());
        let output = gate(r.path());
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(status(r.path()), before);
        assert_eq!(
            text(output),
            complete_report("unmet", "unmet", "met", "met", "met", "met")
        );
    }
}

#[test]
fn spike_to_prototype_requires_limitations_and_accepts_file_observation() {
    let r = repo();
    let parent = head(r.path());
    let checkpoint = CHECKPOINT
        .replace("{head}", &parent)
        .replace(
            "current_maturity = \"probe\"",
            "current_maturity = \"spike\"",
        )
        .replace(
            "requested_maturity = \"spike\"",
            "requested_maturity = \"prototype\"",
        )
        .replace(
            "limitations = []",
            "limitations = [\"One limitation remains.\"]",
        );
    let evidence = format!(
        "{{\"id\":\"ev-001\",\"ts\":\"2026-08-29T12:00:00Z\",\"kind\":\"file-observation\",\"path\":\"README\",\"lines\":\"1\",\"git_head\":\"{parent}\",\"claim\":\"The fixture exists.\"}}\n"
    );
    write_record(r.path(), checkpoint, evidence);
    let before = status(r.path());
    let output = run(
        r.path(),
        &[
            "gate",
            "--issue",
            "120",
            "--from",
            "spike",
            "--to",
            "prototype",
        ],
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(status(r.path()), before);
    assert_eq!(text(output), "{\"issue\":120,\"from\":\"spike\",\"to\":\"prototype\",\"outcome\":\"pass\",\"obligations\":[{\"code\":\"accepted_refs\",\"status\":\"met\"},{\"code\":\"clean_workspace\",\"status\":\"met\"},{\"code\":\"decision_provenance\",\"status\":\"met\"},{\"code\":\"fresh_checkpoint\",\"status\":\"met\"},{\"code\":\"fresh_evidence\",\"status\":\"met\"},{\"code\":\"git_objects\",\"status\":\"met\"},{\"code\":\"hygiene\",\"status\":\"met\"},{\"code\":\"issue_directory\",\"status\":\"met\"},{\"code\":\"schema\",\"status\":\"met\"},{\"code\":\"transition\",\"status\":\"met\"},{\"code\":\"transition_evidence\",\"status\":\"met\"},{\"code\":\"transition_fields\",\"status\":\"met\"}]}\n");
}

#[test]
fn empty_spike_limitations_and_failed_command_have_exact_unmet_obligations() {
    let r = repo();
    let parent = head(r.path());
    write_record(
        r.path(),
        CHECKPOINT
            .replace("{head}", &parent)
            .replace(
                "current_maturity = \"probe\"",
                "current_maturity = \"spike\"",
            )
            .replace(
                "requested_maturity = \"spike\"",
                "requested_maturity = \"prototype\"",
            ),
        EVIDENCE.replace("{head}", &parent),
    );
    let output = run(
        r.path(),
        &[
            "gate",
            "--issue",
            "120",
            "--from",
            "spike",
            "--to",
            "prototype",
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    let report = text(output);
    assert!(report.contains("\"transition_fields\",\"status\":\"unmet\""));
    assert!(report.contains("\"transition_evidence\",\"status\":\"met\""));

    let r = repo();
    let parent = head(r.path());
    write_record(
        r.path(),
        CHECKPOINT.replace("{head}", &parent),
        EVIDENCE
            .replace("{head}", &parent)
            .replace("\"exit_code\":0", "\"exit_code\":1"),
    );
    let output = gate(r.path());
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        text(output),
        complete_report("unmet", "met", "met", "met", "unmet", "met")
    );
}

#[test]
fn empty_evidence_reaches_transition_obligations_as_unmet() {
    let r = repo();
    let parent = head(r.path());
    write_record(
        r.path(),
        CHECKPOINT.replace("{head}", &parent),
        String::new(),
    );

    let output = gate(r.path());
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        text(output),
        complete_report("unmet", "met", "met", "met", "unmet", "met")
    );
}

#[test]
fn unsupported_targets_and_invalid_edges_have_exact_reports_and_do_not_write() {
    for target in ["candidate", "production"] {
        let r = repo();
        valid_record(r.path());
        let before = status(r.path());
        let output = run(
            r.path(),
            &["gate", "--issue", "120", "--from", "probe", "--to", target],
        );
        assert_eq!(output.status.code(), Some(4));
        assert_eq!(status(r.path()), before);
        assert_eq!(text(output), format!("{{\"issue\":120,\"from\":\"probe\",\"to\":\"{target}\",\"outcome\":\"unsupported\",\"obligations\":[{{\"code\":\"accepted_refs\",\"status\":\"met\"}},{{\"code\":\"decision_provenance\",\"status\":\"met\"}},{{\"code\":\"git_objects\",\"status\":\"met\"}},{{\"code\":\"hygiene\",\"status\":\"met\"}},{{\"code\":\"issue_directory\",\"status\":\"met\"}},{{\"code\":\"schema\",\"status\":\"met\"}},{{\"code\":\"transition\",\"status\":\"unsupported\"}}]}}\n"));
    }

    for (from, to) in [
        ("probe", "probe"),
        ("spike", "probe"),
        ("probe", "prototype"),
    ] {
        let r = repo();
        valid_record(r.path());
        let output = run(
            r.path(),
            &["gate", "--issue", "120", "--from", from, "--to", to],
        );
        assert_eq!(output.status.code(), Some(3));
        assert!(text(output).contains("\"transition\",\"status\":\"invalid\""));
    }
}

#[test]
fn promote_dry_run_output_is_byte_identical_to_gate_and_read_only() {
    let r = repo();
    valid_record(r.path());
    let before = status(r.path());
    let gate_output = gate(r.path());
    let promote_output = run(
        r.path(),
        &[
            "promote",
            "--dry-run",
            "--issue",
            "120",
            "--from",
            "probe",
            "--to",
            "spike",
        ],
    );
    assert_eq!(promote_output.status.code(), gate_output.status.code());
    assert_eq!(promote_output.stdout, gate_output.stdout);
    assert!(promote_output.stderr.is_empty());
    assert_eq!(status(r.path()), before);
}
