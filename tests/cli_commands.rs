//! `hoard new` and `hoard export` must work without a terminal when every
//! selecting flag is supplied. `new` must never archive a secret.

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn test_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join(".config/hoard");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("config.yml"),
        "version: 2.0.0\ndefault_namespace: archive\nquery_prefix: '>'\nread_from_current_directory: true\nparameter_token: '#'\nparameter_ending_token: '!'\n",
    )
    .unwrap();
    home
}

fn hoard(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hoard"))
        .args(args)
        .env("HOME", home)
        .current_dir(home)
        // No terminal: any prompt aborts the process instead of hanging.
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn success(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn commands(home: &Path, filter: &str) -> serde_yaml::Value {
    let yaml = success(hoard(home, &["list", "--json", "-f", filter]));
    serde_yaml::from_slice(&yaml).unwrap()
}

#[test]
fn scripted_new_is_non_interactive_and_redacts_secrets() {
    let home = test_home();
    let command = "curl -u 'admin:hunter2xyz' https://localhost:9200/_cluster/health";
    let output = hoard(
        home.path(),
        &[
            "new",
            "-n",
            "opensearch-health",
            "-c",
            command,
            "-d",
            "Check OpenSearch cluster health.",
            "-t",
            "kubernetes,health",
            "--namespace",
            "kubernetes",
        ],
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    success(output);
    assert!(
        stderr.contains("Replaced 1 secret(s)"),
        "expected a redaction notice, got: {stderr}"
    );
    assert!(
        !stderr.contains("hunter2xyz"),
        "the secret must not be echoed: {stderr}"
    );

    let listing = commands(home.path(), "opensearch-health");
    let entry = &listing["commands"].as_sequence().unwrap()[0];
    assert_eq!(
        entry["command"].as_str(),
        Some("curl -u 'admin:#password!' https://localhost:9200/_cluster/health")
    );
    assert_eq!(entry["kind"].as_str(), Some("shell"));
    assert_eq!(entry["namespace"].as_str(), Some("kubernetes"));
    assert_eq!(
        entry["description"].as_str(),
        Some("Check OpenSearch cluster health.")
    );
    assert_eq!(
        entry["tags"].as_sequence().unwrap(),
        &vec![
            serde_yaml::Value::from("kubernetes"),
            serde_yaml::Value::from("health")
        ]
    );
}

#[test]
fn scripted_new_defaults_namespace_and_rejects_duplicate_names() {
    let home = test_home();
    success(hoard(
        home.path(),
        &[
            "new",
            "-n",
            "uptime",
            "-c",
            "uptime -p",
            "-d",
            "Human readable uptime.",
        ],
    ));
    let listing = commands(home.path(), "uptime");
    assert_eq!(
        listing["commands"].as_sequence().unwrap()[0]["namespace"].as_str(),
        Some("archive")
    );

    let duplicate = hoard(
        home.path(),
        &[
            "new",
            "-n",
            "uptime",
            "-c",
            "uptime -s",
            "-d",
            "Different command.",
        ],
    );
    assert!(
        !duplicate.status.success(),
        "a scripted duplicate must fail"
    );
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("already exists"));
    assert_eq!(
        success(hoard(home.path(), &["pick", "-n", "uptime", "--raw"])),
        b"uptime -p"
    );
}

#[test]
fn scripted_new_validates_names_and_tags() {
    let home = test_home();
    let bad_name = hoard(
        home.path(),
        &["new", "-n", "two words", "-c", "echo hi", "-d", "Bad name."],
    );
    assert!(!bad_name.status.success());
    assert!(String::from_utf8_lossy(&bad_name.stderr).contains("whitespace"));

    let bad_tags = hoard(
        home.path(),
        &[
            "new",
            "-n",
            "tags",
            "-c",
            "echo hi",
            "-d",
            "Bad tags.",
            "-t",
            "my tag",
        ],
    );
    assert!(!bad_tags.status.success());
    assert!(String::from_utf8_lossy(&bad_tags.stderr).contains("Tags can't contain whitespaces"));

    let missing_command = hoard(home.path(), &["new", "-n", "no-command"]);
    assert!(!missing_command.status.success());
    assert!(String::from_utf8_lossy(&missing_command.stderr).contains("--command"));
}

#[test]
fn scripted_export_writes_all_or_selected_namespaces() {
    let home = test_home();
    success(hoard(
        home.path(),
        &[
            "new",
            "-n",
            "uptime",
            "-c",
            "uptime -p",
            "-d",
            "Human readable uptime.",
            "-t",
            "linux",
        ],
    ));
    success(hoard(
        home.path(),
        &[
            "new",
            "-n",
            "pods",
            "-c",
            "kubectl get pods -A",
            "-d",
            "Every pod.",
            "--namespace",
            "kubernetes",
        ],
    ));

    let all = home.path().join("all.yml");
    let stdout = success(hoard(
        home.path(),
        &["export", "--all", "--path", all.to_str().unwrap()],
    ));
    assert!(String::from_utf8_lossy(&stdout).contains("Exported 2 entries"));
    let yaml = fs::read_to_string(&all).unwrap();
    assert_eq!(yaml.matches("- name:").count(), 2);
    assert!(yaml.contains("namespace: kubernetes"));
    assert!(yaml.contains("description: Every pod."));

    let subset = home.path().join("k8s.yml");
    success(hoard(
        home.path(),
        &[
            "export",
            "--namespace",
            "kubernetes",
            "--path",
            subset.to_str().unwrap(),
        ],
    ));
    let yaml = fs::read_to_string(&subset).unwrap();
    assert_eq!(yaml.matches("- name:").count(), 1);
    assert!(yaml.contains("name: pods") && !yaml.contains("uptime -p"));
}

#[test]
fn scripted_export_fails_loudly() {
    let home = test_home();
    success(hoard(
        home.path(),
        &[
            "new",
            "-n",
            "uptime",
            "-c",
            "uptime -p",
            "-d",
            "Human readable uptime.",
        ],
    ));

    let unknown = hoard(
        home.path(),
        &[
            "export",
            "--namespace",
            "nope",
            "--path",
            home.path().join("x.yml").to_str().unwrap(),
        ],
    );
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("Available namespaces: archive"));

    let bad_path = hoard(home.path(), &["export", "--all", "--path", ""]);
    assert!(!bad_path.status.success());
    assert!(String::from_utf8_lossy(&bad_path.stderr).contains("No valid path"));

    let empty = test_home();
    let nothing = hoard(
        empty.path(),
        &[
            "export",
            "--all",
            "--path",
            empty.path().join("y.yml").to_str().unwrap(),
        ],
    );
    assert!(!nothing.status.success());
    assert!(String::from_utf8_lossy(&nothing.stderr).contains("Nothing to export"));
}

/// A temp HOME seeded with a fake `.zsh_history` and one fake agent session.
fn history_home() -> tempfile::TempDir {
    let home = test_home();
    fs::write(
        home.path().join(".zsh_history"),
        ": 1700000000:0;git status\n\
         : 1700000001:0;git  status\n\
         : 1700000002:0;ls\n\
         : 1700000003:0;    kubectl get pods -A\n\
         : 1700000004:0;for i in 1 2; do\n  echo $i\ndone\n\
         : 1700000005:0;curl -H 'Authorization: token ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' https://api.github.com\n",
    )
    .unwrap();

    let sessions = home.path().join("sessions/--home-user--");
    fs::create_dir_all(&sessions).unwrap();
    fs::write(
        sessions.join("1700000000000_abcd.jsonl"),
        concat!(
            r#"{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"bash","arguments":{"command":"jq '.items | length' data.json"}}]}}"#,
            "\n",
            r##"{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"write","arguments":{"path":"/tmp/cleanup.py","content":"import os\n\nprint(os.getcwd())\n"}}]}}"##,
            "\n",
        ),
    )
    .unwrap();
    home
}

fn scan_json(home: &Path, extra: &[&str]) -> (String, serde_yaml::Value) {
    let mut args = vec!["history", "scan", "--json", "--trufflehog", "off"];
    args.extend_from_slice(extra);
    let stdout = success(hoard(home, &args));
    let text = String::from_utf8(stdout).unwrap();
    let json: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
    (text, json)
}

#[test]
fn history_scan_reports_counts_and_never_prints_the_secret() {
    let home = history_home();
    let sessions = home.path().join("sessions");
    let (text, json) = scan_json(home.path(), &["--sessions-dir", sessions.to_str().unwrap()]);

    assert_eq!(json["zsh"]["raw"].as_u64(), Some(6));
    assert_eq!(json["zsh"]["candidates"].as_u64(), Some(4));
    assert_eq!(json["zsh"]["duplicates"].as_u64(), Some(1));
    assert_eq!(json["zsh"]["noise"].as_u64(), Some(1));
    assert_eq!(json["zsh"]["redacted"].as_u64(), Some(1));
    assert_eq!(json["pi"]["raw"].as_u64(), Some(2));
    assert_eq!(json["pi"]["candidates"].as_u64(), Some(2));
    assert_eq!(json["candidates"].as_u64(), Some(6));
    assert_eq!(json["session_files"].as_u64(), Some(1));

    assert!(
        !text.contains("ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        "the secret must never be printed: {text}"
    );
    assert!(text.contains("#token!"), "expected a placeholder: {text}");
}

#[test]
fn history_import_dry_run_writes_nothing() {
    let home = history_home();
    let sessions = home.path().join("sessions");
    let output = hoard(
        home.path(),
        &[
            "history",
            "import",
            "--all",
            "--dry-run",
            "--trufflehog",
            "off",
            "--sessions-dir",
            sessions.to_str().unwrap(),
        ],
    );
    let stdout = String::from_utf8(success(output)).unwrap();
    assert!(stdout.contains("would add"), "{stdout}");
    assert!(stdout.contains("nothing was written"), "{stdout}");
    assert!(stdout.contains("#token!"), "{stdout}");

    let pick = hoard(home.path(), &["pick", "-n", "git-status", "--raw"]);
    assert!(!pick.status.success(), "a dry run must not write the trove");
}

#[test]
fn history_import_lands_entries_in_per_source_namespaces() {
    let home = history_home();
    let sessions = home.path().join("sessions");
    success(hoard(
        home.path(),
        &[
            "history",
            "import",
            "--all",
            "--trufflehog",
            "off",
            "--sessions-dir",
            sessions.to_str().unwrap(),
        ],
    ));

    let listing = commands(home.path(), "");
    let entries = listing["commands"].as_sequence().unwrap();
    let find = |name: &str| {
        entries
            .iter()
            .find(|entry| entry["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("missing entry {name}"))
    };

    let shell = find("kubectl-get-pods-a");
    assert_eq!(shell["namespace"].as_str(), Some("history"));
    assert_eq!(shell["command"].as_str(), Some("kubectl get pods -A"));
    assert_eq!(shell["kind"].as_str(), Some("shell"));
    assert_eq!(
        shell["tags"].as_sequence().unwrap(),
        &vec![
            serde_yaml::Value::from("kubectl"),
            serde_yaml::Value::from("kubernetes"),
            serde_yaml::Value::from("k8s"),
            serde_yaml::Value::from("imported"),
            serde_yaml::Value::from("zsh")
        ]
    );

    // Multi-line commands are imported whole, named after their first line.
    let script = find("for-i-in-1-2-do");
    assert!(
        script["command"]
            .as_str()
            .unwrap()
            .contains("\n  echo $i\ndone")
    );

    let redacted = entries
        .iter()
        .find(|entry| {
            entry["command"]
                .as_str()
                .is_some_and(|command| command.starts_with("curl -H 'Authorization: token "))
        })
        .expect("the curl entry should be imported with a placeholder");
    let command = redacted["command"].as_str().unwrap();
    assert!(command.contains("#"), "{command}");
    assert!(!command.contains("ghp_"), "{command}");

    // Agent session commands land in `pi`; the written file keeps its source.
    let session = find("jq-items-length-data-json");
    assert_eq!(session["namespace"].as_str(), Some("pi"));
    let python = find("import-os");
    assert_eq!(python["namespace"].as_str(), Some("pi"));
    assert_eq!(python["kind"].as_str(), Some("python"));
    assert!(
        python["description"]
            .as_str()
            .unwrap()
            .starts_with("Python script captured from pi session on 2023-11-14"),
        "{}",
        python["description"].as_str().unwrap()
    );

    // Filtered noise never reaches the trove.
    assert!(
        !entries
            .iter()
            .any(|entry| entry["command"].as_str() == Some("ls")),
        "`ls` must be filtered out"
    );
}

#[test]
fn history_import_tolerates_a_broken_trufflehog_binary() {
    let home = history_home();
    let sessions = home.path().join("sessions");
    let output = hoard(
        home.path(),
        &[
            "history",
            "import",
            "--all",
            "--dry-run",
            "--trufflehog",
            "/bin/false",
            "--sessions-dir",
            sessions.to_str().unwrap(),
        ],
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let stdout = String::from_utf8(success(output)).unwrap();
    assert!(stderr.contains("built-in redaction only"), "{stderr}");
    assert!(stdout.contains("#token!"), "{stdout}");
}

#[test]
fn history_import_redacts_the_exact_trufflehog_span() {
    let home = history_home();
    fs::write(
        home.path().join(".zsh_history"),
        ": 1700000000:0;deploy --acme ACME_TOKEN_XYZ --env prod\n",
    )
    .unwrap();

    // A stand-in scanner: reports whatever candidate file holds the token.
    let scanner = home.path().join("fake-trufflehog");
    fs::write(
        &scanner,
        "#!/bin/sh\n\
         for arg in \"$@\"; do dir=\"$arg\"; done\n\
         for file in \"$dir\"/*.txt; do\n\
           if grep -q ACME_TOKEN_XYZ \"$file\"; then\n\
             printf '{\"SourceMetadata\":{\"Data\":{\"Filesystem\":{\"file\":\"%s\",\"line\":1}}},\"DetectorName\":\"AcmeToken\",\"Raw\":\"ACME_TOKEN_XYZ\"}\\n' \"$file\"\n\
           fi\n\
         done\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&scanner, fs::Permissions::from_mode(0o755)).unwrap();
    }

    success(hoard(
        home.path(),
        &[
            "history",
            "import",
            "--all",
            "--source",
            "zsh",
            "--trufflehog",
            scanner.to_str().unwrap(),
        ],
    ));

    let listing = commands(home.path(), "deploy");
    let command = listing["commands"].as_sequence().unwrap()[0]["command"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(command, "deploy --acme #acme_token! --env prod");
}
