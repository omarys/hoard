//! `scripts/import-zsh-history.sh`, driven the way the mise tasks drive it.
//!
//! The wrapper is thin on purpose, so these tests pin the parts hoard cannot
//! pin for it: which history file it reads, that `--dry-run` really writes
//! nothing, that `--json` yields exactly one document (no banner lines mixed
//! into stdout), and that a missing history file is a reported error and not a
//! panic. `HOARD_BIN` points the script at the binary under test.

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const SECRET_PREFIX: &str = "AKIAIOSFODNN7EXAMPLE";

fn script_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/import-zsh-history.sh")
}

/// A HOME whose zsh history holds two hoardable commands and one noise line.
fn test_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join(".config/hoard");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("config.yml"),
        "version: 2.0.0\ndefault_namespace: archive\nquery_prefix: '>'\nread_from_current_directory: true\nparameter_token: '#'\nparameter_ending_token: '!'\n",
    )
    .unwrap();
    fs::write(
        home.path().join(".zsh_history"),
        ": 1700000000:0;docker ps -a --format '{{.Names}}'\n\
         : 1700000001:0;git log --oneline -20\n\
         : 1700000002:0;git log --oneline -20\n\
         : 1700000003:0;ls\n",
    )
    .unwrap();
    home
}

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new("bash")
        .arg(script_path())
        .args(args)
        .env("HOME", home)
        .env("HOARD_BIN", env!("CARGO_BIN_EXE_hoard"))
        .env_remove("HISTFILE")
        .current_dir(home)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Names currently stored, sorted, from `hoard list --json` (optionally filtered
/// to one namespace — the listing has no namespace flag of its own).
fn stored_names(home: &Path, namespace: Option<&str>) -> Vec<String> {
    let output = Command::new(env!("CARGO_BIN_EXE_hoard"))
        .args(["list", "--json"])
        .env("HOME", home)
        .current_dir(home)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut names: Vec<String> = payload["commands"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| match namespace {
            Some(wanted) => entry["namespace"] == wanted,
            None => true,
        })
        .map(|entry| entry["name"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

#[test]
fn imports_the_history_and_skips_what_it_already_stored() {
    let home = test_home();

    let first = run(home.path(), &[]);
    assert!(first.status.success(), "stderr: {}", stderr_of(&first));
    assert!(
        stdout_of(&first).contains("Imported 2 command(s)."),
        "unexpected output: {}",
        stdout_of(&first)
    );
    assert_eq!(stored_names(home.path(), None).len(), 2);

    // Same history, nothing new: the second run must not duplicate entries and
    // must not fail (a cron job calling this must stay quiet and green).
    let second = run(home.path(), &[]);
    assert!(second.status.success(), "stderr: {}", stderr_of(&second));
    assert!(
        stdout_of(&second).contains("Nothing to import"),
        "unexpected output: {}",
        stdout_of(&second)
    );
    assert_eq!(stored_names(home.path(), None).len(), 2);
}

#[test]
fn dry_run_writes_nothing() {
    let home = test_home();

    let output = run(home.path(), &["--dry-run"]);
    assert!(output.status.success(), "stderr: {}", stderr_of(&output));
    let report = stdout_of(&output);
    assert!(report.contains("dry run"), "missing hint: {report}");
    assert!(report.contains("candidates"), "missing report: {report}");
    assert!(
        stored_names(home.path(), None).is_empty(),
        "a dry run persisted entries"
    );

    // …and the real run afterwards still finds the same candidates.
    let output = run(home.path(), &[]);
    assert!(output.status.success(), "stderr: {}", stderr_of(&output));
    assert_eq!(stored_names(home.path(), None).len(), 2);
}

#[test]
fn json_mode_prints_one_document_and_no_banner() {
    let home = test_home();

    let output = run(home.path(), &["--json"]);
    assert!(output.status.success(), "stderr: {}", stderr_of(&output));
    let stdout = stdout_of(&output);
    assert_eq!(
        stdout.lines().count(),
        1,
        "--json must not mix a banner into stdout: {stdout:?}"
    );
    let payload: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["count"], 2);
    assert_eq!(payload["dry_run"], false);
    let imported = payload["imported"].as_array().unwrap();
    assert_eq!(imported.len(), 2);
}

#[test]
fn json_dry_run_reports_without_importing() {
    let home = test_home();

    let output = run(home.path(), &["--dry-run", "--json"]);
    assert!(output.status.success(), "stderr: {}", stderr_of(&output));
    let payload: serde_json::Value = serde_json::from_str(&stdout_of(&output)).unwrap();
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["zsh"]["candidates"], 2);
    assert_eq!(
        payload["zsh"]["duplicates"], 1,
        "the repeated line is a duplicate"
    );
    assert!(stored_names(home.path(), None).is_empty());
}

#[test]
fn missing_history_is_a_reported_error_in_both_modes() {
    let home = test_home();
    let missing = home.path().join("nope");

    let output = run(home.path(), &["--histfile", missing.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stdout_of(&output).is_empty(),
        "text mode must not use stdout"
    );
    assert!(
        stderr_of(&output).contains("no zsh history"),
        "stderr: {}",
        stderr_of(&output)
    );

    let output = run(
        home.path(),
        &["--histfile", missing.to_str().unwrap(), "--json"],
    );
    assert_eq!(output.status.code(), Some(1));
    let payload: serde_json::Value = serde_json::from_str(&stdout_of(&output)).unwrap();
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error_type"], "HistoryNotFound");
}

#[test]
fn histfile_namespace_and_limit_are_honoured() {
    let home = test_home();
    let elsewhere = home.path().join("old_zsh_history");
    fs::write(
        &elsewhere,
        ": 1700000100:0;kubectl get pods -A\n: 1700000101:0;helm list -A\n",
    )
    .unwrap();

    let output = run(
        home.path(),
        &[
            "--histfile",
            elsewhere.to_str().unwrap(),
            "--namespace",
            "shell",
            "--limit",
            "1",
        ],
    );
    assert!(output.status.success(), "stderr: {}", stderr_of(&output));
    assert!(
        stdout_of(&output).contains("Imported 1 command(s)."),
        "unexpected output: {}",
        stdout_of(&output)
    );
    // The identical default history was never read, so its commands are absent.
    assert_eq!(stored_names(home.path(), Some("shell")).len(), 1);
    assert!(stored_names(home.path(), Some("archive")).is_empty());
}

#[test]
fn secrets_in_history_are_redacted_before_storage() {
    let home = test_home();
    fs::write(
        home.path().join(".zsh_history"),
        format!(": 1700000000:0;export AWS_ACCESS_KEY_ID={SECRET_PREFIX}\n"),
    )
    .unwrap();

    let output = run(home.path(), &["--json"]);
    assert!(output.status.success(), "stderr: {}", stderr_of(&output));

    let listing = Command::new(env!("CARGO_BIN_EXE_hoard"))
        .args(["list", "--json"])
        .env("HOME", home.path())
        .current_dir(home.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&listing.stdout).into_owned();
    assert!(
        !stdout.contains(SECRET_PREFIX),
        "an access key reached the trove: {stdout}"
    );
}

#[test]
fn unknown_flag_is_a_parse_error() {
    let home = test_home();
    let output = run(home.path(), &["--bogus"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr_of(&output).contains("unknown argument"),
        "{}",
        stderr_of(&output)
    );
}

#[test]
fn help_lists_every_flag_and_resolution_order() {
    let home = test_home();
    let output = run(home.path(), &["--help"]);
    assert!(output.status.success());
    let text = stdout_of(&output);
    for flag in [
        "--dry-run",
        "--histfile",
        "--namespace",
        "--limit",
        "--trufflehog",
        "--json",
    ] {
        assert!(text.contains(flag), "--help omits {flag}");
    }
    assert!(
        text.contains("HOARD_BIN"),
        "binary resolution is undocumented"
    );
}
