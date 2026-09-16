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
