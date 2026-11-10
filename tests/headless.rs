//! The headless contract: every invocation an agent makes must produce one
//! parseable payload, one deterministic exit code, and no terminal decoration.
//!
//! These tests drive the real binary with no terminal attached, which is the
//! only way to prove the flags (`--json`, `--compact`), the templating engine
//! (`eval`) and the error payloads agree with each other.

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
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn seed(home: &Path, name: &str, command: &str, description: &str, namespace: &str) {
    let output = hoard(
        home,
        &[
            "new",
            "-n",
            name,
            "-c",
            command,
            "-d",
            description,
            "--namespace",
            namespace,
        ],
    );
    assert!(
        output.status.success(),
        "seeding {name} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn stdout_json(output: &Output) -> serde_json::Value {
    let text = String::from_utf8(output.stdout.clone()).unwrap();
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("stdout is not JSON ({err}): {text:?}"))
}

/// Bytes of stdout, for the density comparisons.
fn stdout_len(output: &Output) -> usize {
    output.stdout.len()
}

#[test]
fn compact_output_drops_every_field_a_model_does_not_need() {
    let home = test_home();
    seed(
        home.path(),
        "logs",
        "docker logs -f {{container_id}}",
        "Tail a container log.",
        "dev",
    );

    let full = hoard(home.path(), &["list", "--json", "-f", "logs"]);
    assert!(full.status.success());
    let entry = &stdout_json(&full)["commands"][0];
    // The verbose projection keeps provenance and the empty tag list.
    assert_eq!(entry["name"], "logs");
    assert_eq!(entry["namespace"], "dev");
    assert!(entry["created"].is_string());
    assert!(entry["tags"].is_array());
    assert_eq!(entry["params"][0]["name"], "container_id");
    assert_eq!(entry["params"][0]["required"], true);

    let dense = hoard(home.path(), &["list", "--json", "--compact", "-f", "logs"]);
    assert!(dense.status.success());
    let entry = &stdout_json(&dense)["commands"][0];
    assert_eq!(entry["k"], "logs");
    assert_eq!(entry["c"], "docker logs -f {{container_id}}");
    assert_eq!(entry["d"], "Tail a container log.");
    assert_eq!(entry["p"][0], "container_id");
    // Nothing else survives: no namespace, kind, created or tags keys. `s`
    // may appear because this listing was filtered (it carries the score).
    let object = entry.as_object().unwrap();
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["c", "d", "k", "p", "s"]);

    // The whole point of --compact is a smaller payload: a third of the bytes is
    // the budget, so a new verbose field cannot creep into the dense shape.
    let (dense_bytes, full_bytes) = (stdout_len(&dense), stdout_len(&full));
    assert!(
        dense_bytes * 100 <= full_bytes * 60,
        "compact ({dense_bytes} B) should be at most 60% of the full projection ({full_bytes} B)"
    );
}

#[test]
fn compact_output_omits_empty_fields_and_adds_a_score_when_searching() {
    let home = test_home();
    // Deliberately no description: the key must vanish, not serialize as "".
    seed(home.path(), "pods", "kubectl get pods", "", "kube");
    seed(
        home.path(),
        "images",
        "podman images",
        "List images.",
        "dev",
    );

    let listed = hoard(home.path(), &["list", "--json", "--compact"]);
    assert!(listed.status.success());
    let commands = stdout_json(&listed)["commands"].as_array().unwrap().clone();
    let pods = commands.iter().find(|entry| entry["k"] == "pods").unwrap();
    assert!(pods.get("d").is_none(), "empty description must be omitted");
    assert!(pods.get("p").is_none(), "no placeholders means no `p` key");

    // A query adds the lexical score `s`, highest first.
    let searched = hoard(home.path(), &["list", "--json", "--compact", "-f", "pods"]);
    assert!(searched.status.success());
    let payload = stdout_json(&searched);
    let first = &payload["commands"][0];
    assert_eq!(first["k"], "pods");
    assert!(
        first["s"].as_u64().unwrap_or(0) > 0,
        "a matching hit must carry a score: {first}"
    );
}

#[test]
fn eval_substitutes_placeholders_and_echoes_what_it_applied() {
    let home = test_home();
    seed(
        home.path(),
        "logs",
        "docker logs --tail {{lines}} -f {{container_id}}",
        "Tail a container log.",
        "dev",
    );

    let rendered = hoard(
        home.path(),
        &[
            "eval",
            "logs",
            "--param",
            "lines=50",
            "--param",
            "container_id=abc123",
            "--json",
        ],
    );
    assert!(
        rendered.status.success(),
        "{}",
        String::from_utf8_lossy(&rendered.stderr)
    );
    let payload = stdout_json(&rendered);
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "docker logs --tail 50 -f abc123");
    assert_eq!(payload["params"]["lines"], "50");
    assert_eq!(payload["params"]["container_id"], "abc123");
    assert_eq!(payload["params"]["name"], serde_json::Value::Null);

    // Compact mode keeps the dense keys only.
    let dense = hoard(
        home.path(),
        &[
            "eval",
            "logs",
            "-p",
            "lines=50",
            "-p",
            "container_id=abc123",
            "--json",
            "--compact",
        ],
    );
    let dense = stdout_json(&dense);
    assert_eq!(dense["k"], "logs");
    assert_eq!(dense["c"], "docker logs --tail 50 -f abc123");
    assert_eq!(dense["p"]["lines"], "50");
    assert!(dense.get("command").is_none(), "no verbose keys when dense");

    // Human mode prints just the command, ready to paste.
    let human = hoard(
        home.path(),
        &["eval", "logs", "-p", "lines=5", "-p", "container_id=web"],
    );
    assert!(human.status.success());
    assert_eq!(
        String::from_utf8_lossy(&human.stdout).trim(),
        "docker logs --tail 5 -f web"
    );
    // ... and never executes or mutates anything.
    let after = hoard(home.path(), &["list", "--json", "-f", "logs"]);
    assert_eq!(
        stdout_json(&after)["commands"][0]["command"],
        "docker logs --tail {{lines}} -f {{container_id}}"
    );
}

#[test]
fn eval_reports_missing_parameters_as_data_not_prose() {
    let home = test_home();
    seed(
        home.path(),
        "rebuild",
        "docker build -t {{tag}} {{path}}",
        "Build an image.",
        "dev",
    );

    let output = hoard(home.path(), &["eval", "rebuild", "-p", "tag=v1", "--json"]);
    assert_eq!(output.status.code(), Some(3), "InvalidParameter exits 3");
    let payload = stdout_json(&output);
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error_type"], "InvalidParameter");
    assert_eq!(payload["missing_params"], serde_json::json!(["path"]));
    assert_eq!(payload["suggested_keys"], serde_json::json!(["path"]));
    assert!(
        payload["message"].as_str().unwrap().contains("path"),
        "the message must name the parameter: {payload}"
    );
}

#[test]
fn eval_rejects_unknown_keys_and_missing_values() {
    let home = test_home();
    seed(home.path(), "deploy", "git push {{remote}}", "Push.", "dev");

    let unknown = hoard(
        home.path(),
        &[
            "eval",
            "deploy",
            "-p",
            "remote=origin",
            "-p",
            "branch=main",
            "--json",
        ],
    );
    assert_eq!(unknown.status.code(), Some(3));
    let payload = stdout_json(&unknown);
    assert_eq!(payload["error_type"], "InvalidParameter");
    assert_eq!(payload["suggested_keys"], serde_json::json!(["branch"]));

    let malformed = hoard(home.path(), &["eval", "deploy", "-p", "remote", "--json"]);
    assert_eq!(malformed.status.code(), Some(3));
    assert_eq!(stdout_json(&malformed)["error_type"], "InvalidParameter");
}

#[test]
fn eval_of_an_unknown_entry_suggests_the_nearest_key() {
    let home = test_home();
    seed(home.path(), "pods", "kubectl get pods", "", "kube");

    let output = hoard(home.path(), &["eval", "podz", "--json"]);
    assert_eq!(output.status.code(), Some(2), "CommandNotFound exits 2");
    let payload = stdout_json(&output);
    assert_eq!(payload["error_type"], "CommandNotFound");
    assert_eq!(payload["suggested_keys"], serde_json::json!(["kube/pods"]));
}

#[test]
fn eval_returns_python_sources_untouched_and_rejects_parameters() {
    let home = test_home();
    let script = home.path().join("report.py");
    fs::write(&script, "print('{{who}}')\n").unwrap();
    let created = hoard(
        home.path(),
        &[
            "new",
            "--script",
            script.to_str().unwrap(),
            "-n",
            "report",
            "-d",
            "Print a name.",
            "--namespace",
            "dev",
            "--json",
        ],
    );
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );

    // A Python source is not a shell template: `{{...}}` inside it is source
    // text, so `new --script` records no parameters and `eval` leaves it alone.
    let created_payload = stdout_json(&created);
    assert!(
        created_payload.get("p").is_none(),
        "a scripted save must not report parameters: {created_payload}"
    );

    let rejected = hoard(
        home.path(),
        &["eval", "report", "-p", "who=world", "--json"],
    );
    assert_eq!(
        rejected.status.code(),
        Some(3),
        "a Python entry has no parameters to fill"
    );
    assert_eq!(stdout_json(&rejected)["error_type"], "InvalidParameter");

    let output = hoard(home.path(), &["eval", "report", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rendered = stdout_json(&output)["command"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        rendered.starts_with("python3 -c '"),
        "python entries must keep the base64 wrapper: {rendered}"
    );

    // The wrapper decodes back to the rendered source.
    let base64 = rendered
        .split("base64.b64decode(\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let decoded = String::from_utf8(base64_decode(base64)).unwrap();
    assert_eq!(
        decoded, "print('{{who}}')\n",
        "the source must survive verbatim"
    );
}

/// Minimal base64 decoder, so the test needs no extra dependency.
fn base64_decode(input: &str) -> Vec<u8> {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in input.bytes() {
        if byte == b'=' {
            break;
        }
        let Some(value) = TABLE.iter().position(|entry| *entry == byte) else {
            continue;
        };
        buffer = (buffer << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    out
}

#[test]
fn schema_describes_the_payload_and_the_exit_codes() {
    let home = test_home();

    let output = hoard(home.path(), &["schema", "--json"]);
    assert!(output.status.success());
    let schema = stdout_json(&output);
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    // It must describe both projections and the error shape.
    let definitions = schema["$defs"].as_object().unwrap();
    for name in [
        "command",
        "compactCommand",
        "list",
        "compactList",
        "evalResult",
        "compactEvalResult",
        "error",
    ] {
        assert!(
            definitions.contains_key(name),
            "schema must define {name}: {:?}",
            definitions.keys().collect::<Vec<_>>()
        );
    }
    let codes = schema["exit_codes"].as_object().unwrap();
    assert_eq!(codes["0"], "ok");
    assert!(codes["2"].as_str().unwrap().starts_with("CommandNotFound"));
    assert!(codes["3"].as_str().unwrap().starts_with("InvalidParameter"));
    // The exact argv an agent should build is listed for each operation.
    let invocations = schema["invocations"].as_object().unwrap();
    for name in ["search", "best", "eval", "pick"] {
        assert!(
            invocations.contains_key(name),
            "schema must document the {name} invocation"
        );
    }
    let eval = &invocations["eval"]["argv"];
    assert_eq!(eval[1], "eval");
    assert_eq!(eval[2], "<name>", "the entry is positional: {eval}");
}

#[test]
fn schema_is_minified_by_compact_but_semantically_identical() {
    let home = test_home();
    let pretty = hoard(home.path(), &["schema", "--json"]);
    let dense = hoard(home.path(), &["schema", "--json", "--compact"]);
    assert!(pretty.status.success() && dense.status.success());
    assert_eq!(stdout_json(&pretty), stdout_json(&dense));
    assert!(stdout_len(&dense) <= stdout_len(&pretty));
    assert!(
        !String::from_utf8_lossy(&dense.stdout).contains("  "),
        "--compact must not pretty-print"
    );
}

#[test]
fn parse_errors_are_json_too_when_json_was_requested() {
    let home = test_home();
    // `pick` requires --name; clap fails before hoard ever runs.
    let output = hoard(home.path(), &["pick", "--json"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a bad invocation is a ParseError (1)"
    );
    let payload = stdout_json(&output);
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error_type"], "ParseError");
    assert!(!payload["message"].as_str().unwrap().is_empty());
}

#[test]
fn human_mode_failures_keep_the_message_on_stderr() {
    let home = test_home();
    let output = hoard(home.path(), &["eval", "nope"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty(), "stdout must stay clean");
    assert!(!output.stderr.is_empty(), "the reason belongs on stderr");
}

#[test]
fn json_output_carries_no_ansi_or_cursor_sequences() {
    let home = test_home();
    // A stored command can carry escape sequences from history.
    seed(
        home.path(),
        "colored",
        "echo \u{1b}[31mred\u{1b}[0m",
        "An escape-carrying command.",
        "dev",
    );

    for args in [
        vec!["list", "--json"],
        vec!["list", "--json", "--compact"],
        vec!["pick", "-n", "colored", "--json"],
        vec!["schema", "--json"],
        // Human mode is the interesting one: the command itself is printed, and
        // `seed` stores raw bytes that serde_json would otherwise have escaped.
        vec!["pick", "-n", "colored"],
        vec!["eval", "colored"],
    ] {
        let output = hoard(home.path(), &args);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(
            !stdout.contains('\u{1b}'),
            "{} leaked an escape sequence: {stdout:?}",
            args.join(" ")
        );
    }
}

#[test]
fn pick_returns_the_invocation_as_json_and_the_source_with_raw() {
    let home = test_home();
    seed(
        home.path(),
        "pods",
        "kubectl get pods {{namespace}}",
        "List pods.",
        "kube",
    );

    let picked = hoard(home.path(), &["pick", "-n", "pods", "--json"]);
    assert!(picked.status.success());
    let payload = stdout_json(&picked);
    assert_eq!(payload["name"], "pods");
    assert_eq!(payload["namespace"], "kube");
    assert_eq!(payload["kind"], "shell");
    assert_eq!(payload["command"], "kubectl get pods {{namespace}}");

    // --json wins over --raw: the stream stays parseable.
    let raw = hoard(home.path(), &["pick", "-n", "pods", "--raw", "--json"]);
    assert!(raw.status.success());
    assert_eq!(
        stdout_json(&raw)["command"],
        "kubectl get pods {{namespace}}"
    );

    // Plain --raw is for piping: byte-exact stored source, no final newline.
    let source = hoard(home.path(), &["pick", "-n", "pods", "--raw"]);
    assert!(source.status.success());
    assert_eq!(
        String::from_utf8_lossy(&source.stdout),
        "kubectl get pods {{namespace}}"
    );
}

#[test]
fn list_best_and_compact_agree_on_the_answer() {
    let home = test_home();
    seed(
        home.path(),
        "pods",
        "kubectl get pods",
        "List pods.",
        "kube",
    );
    seed(home.path(), "compose", "docker compose up", "Start.", "dev");

    let best = hoard(home.path(), &["list", "--best", "-f", "pods", "--json"]);
    assert!(best.status.success());
    assert_eq!(stdout_json(&best)["name"], "pods");

    let compact = hoard(
        home.path(),
        &["list", "--best", "-f", "pods", "--json", "--compact"],
    );
    assert!(compact.status.success());
    let payload = stdout_json(&compact);
    assert_eq!(payload["k"], "pods");
    assert!(payload["s"].as_u64().unwrap_or(0) > 0);
}

#[test]
fn eval_renders_the_legacy_marker_dialect_too() {
    let home = test_home();
    // A trove written before placeholders existed, still using `#name!`.
    seed(
        home.path(),
        "inspect",
        "docker exec -it #container_id! sh -lc 'ls #path!'",
        "Legacy parameterised command.",
        "dev",
    );

    let output = hoard(
        home.path(),
        &[
            "eval",
            "inspect",
            "-p",
            "container_id=web",
            "-p",
            "path=/srv",
            "--json",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        stdout_json(&output)["command"],
        "docker exec -it web sh -lc 'ls /srv'"
    );

    // The legacy names are discoverable from the error payload alone.
    let missing = hoard(home.path(), &["eval", "inspect", "--json"]);
    assert_eq!(missing.status.code(), Some(3));
    assert_eq!(
        stdout_json(&missing)["missing_params"],
        serde_json::json!(["container_id", "path"])
    );
}

#[test]
fn eval_leaves_python_source_markers_alone() {
    let home = test_home();
    let script = home.path().join("legacy.py");
    // `#name!` is a comment here, not a parameter.
    fs::write(&script, "#name!\nprint('hi')\n").unwrap();
    let created = hoard(
        home.path(),
        &[
            "new",
            "--script",
            script.to_str().unwrap(),
            "-n",
            "legacy",
            "-d",
            "A script with a comment.",
            "--namespace",
            "dev",
        ],
    );
    assert!(created.status.success());

    let output = hoard(home.path(), &["eval", "legacy", "--json"]);
    assert!(
        output.status.success(),
        "python entries must not demand comment parameters: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn new_without_the_required_inputs_fails_before_any_prompt() {
    let home = test_home();

    let unnamed = hoard(home.path(), &["new", "-c", "echo hi", "--json"]);
    assert_eq!(unnamed.status.code(), Some(3));
    let payload = stdout_json(&unnamed);
    assert_eq!(payload["error_type"], "InvalidParameter");
    assert_eq!(payload["missing_params"], serde_json::json!(["name"]));

    // An empty trove stays empty: no half-saved entry from a defaulted prompt.
    let listed = hoard(home.path(), &["list", "--json", "--compact"]);
    assert_eq!(stdout_json(&listed), serde_json::json!({"commands": []}));
}

#[test]
fn interactive_only_subcommands_refuse_without_a_terminal() {
    let home = test_home();
    seed(
        home.path(),
        "pods",
        "kubectl get pods",
        "List pods.",
        "kube",
    );

    // `edit` drives dialoguer; headless it would save defaults over the entry.
    let edit = hoard(home.path(), &["edit", "-n", "pods"]);
    assert_eq!(edit.status.code(), Some(3));
    assert!(edit.stdout.is_empty());
    assert!(String::from_utf8_lossy(&edit.stderr).contains("needs a terminal"));

    let edit_json = hoard(home.path(), &["edit", "-n", "pods", "--json"]);
    assert_eq!(edit_json.status.code(), Some(3));
    assert_eq!(stdout_json(&edit_json)["error_type"], "InvalidParameter");

    // The entry is untouched either way.
    let picked = hoard(home.path(), &["pick", "-n", "pods", "--raw"]);
    assert_eq!(String::from_utf8_lossy(&picked.stdout), "kubectl get pods");
}

#[test]
fn tokens_is_the_documented_alias_of_compact() {
    let home = test_home();
    seed(
        home.path(),
        "pods",
        "kubectl get pods",
        "List pods.",
        "kube",
    );

    let compact = hoard(home.path(), &["list", "--json", "--compact"]);
    let tokens = hoard(home.path(), &["list", "--tokens"]);
    assert!(tokens.status.success());
    assert_eq!(stdout_json(&compact), stdout_json(&tokens));
    // The alias is the only spelling that mentions `tokens` in --help.
    let help = hoard(home.path(), &["--help"]);
    assert!(String::from_utf8_lossy(&help.stdout).contains("tokens"));
}

#[test]
fn an_empty_trove_is_an_empty_json_list_not_prose() {
    let home = test_home();
    let output = hoard(home.path(), &["list", "--json", "--compact"]);
    assert!(output.status.success());
    assert_eq!(stdout_json(&output), serde_json::json!({"commands": []}));
}

#[test]
fn json_mode_wins_over_the_human_shortcuts() {
    let home = test_home();
    seed(
        home.path(),
        "pods",
        "kubectl get pods",
        "List pods.",
        "kube",
    );

    // `-s`/`--simple` is a table shortcut; json mode still owes JSON.
    let simple = hoard(home.path(), &["list", "-s", "--json"]);
    assert!(simple.status.success());
    assert!(
        stdout_json(&simple)["commands"].is_array(),
        "`list -s --json` must stay parseable"
    );
}

#[test]
fn repeated_placeholders_share_one_default() {
    let home = test_home();
    seed(
        home.path(),
        "twice",
        "echo {{a=x}} {{a}}",
        "Same placeholder twice.",
        "dev",
    );
    seed(
        home.path(),
        "later",
        "echo {{b}} {{b=y}}",
        "Default after the bare one.",
        "dev",
    );

    let rendered = hoard(home.path(), &["eval", "twice", "--json"]);
    assert!(rendered.status.success());
    assert_eq!(stdout_json(&rendered)["command"], "echo x x");

    let rendered = hoard(home.path(), &["eval", "later", "--json"]);
    assert!(
        rendered.status.success(),
        "a default in any occurrence makes the parameter optional: {}",
        String::from_utf8_lossy(&rendered.stdout)
    );
    assert_eq!(stdout_json(&rendered)["command"], "echo y y");
}

#[test]
fn eval_accepts_values_containing_equals_signs() {
    let home = test_home();
    seed(
        home.path(),
        "grep",
        "grep {{pattern}} {{file}}",
        "Search a file.",
        "dev",
    );

    let output = hoard(
        home.path(),
        &[
            "eval",
            "grep",
            "-p",
            "pattern=a=b",
            "-p",
            "file=x",
            "--json",
        ],
    );
    assert!(output.status.success());
    assert_eq!(stdout_json(&output)["command"], "grep a=b x");
}

#[test]
fn eval_rejects_duplicate_and_empty_required_parameters() {
    let home = test_home();
    seed(
        home.path(),
        "logs",
        "docker logs {{container_id}}",
        "Tail a container.",
        "dev",
    );

    let duplicate = hoard(
        home.path(),
        &[
            "eval",
            "logs",
            "-p",
            "container_id=a",
            "-p",
            "container_id=b",
            "--json",
        ],
    );
    assert_eq!(duplicate.status.code(), Some(3));
    assert_eq!(stdout_json(&duplicate)["error_type"], "InvalidParameter");

    let empty = hoard(
        home.path(),
        &["eval", "logs", "-p", "container_id=", "--json"],
    );
    assert_eq!(empty.status.code(), Some(3));
    assert_eq!(stdout_json(&empty)["missing_params"][0], "container_id");
}

#[test]
fn new_reports_what_it_saved_only_after_persisting() {
    let home = test_home();
    let created = hoard(
        home.path(),
        &[
            "new",
            "-n",
            "pods",
            "-c",
            "kubectl get pods {{namespace}}",
            "-d",
            "List pods.",
            "--json",
        ],
    );
    assert!(created.status.success());
    let payload = stdout_json(&created);
    assert_eq!(payload["saved"], "archive/pods");
    assert_eq!(payload["p"][0], "namespace");

    // A saved script answers with the same shape instead of staying silent.
    let script = home.path().join("hello.py");
    std::fs::write(&script, "print('hello')\n").unwrap();
    let saved = hoard(
        home.path(),
        &[
            "new",
            "--script",
            script.to_str().unwrap(),
            "-n",
            "hello",
            "-d",
            "Say hello.",
            "--json",
        ],
    );
    assert!(saved.status.success());
    assert_eq!(stdout_json(&saved)["saved"], "archive/hello");
}

#[test]
fn headless_history_import_needs_explicit_consent() {
    let home = test_home();
    let output = hoard(home.path(), &["history", "import", "--json"]);
    assert!(!output.status.success(), "no prompt, no silent bulk import");
    let payload = stdout_json(&output);
    assert_eq!(payload["error_type"], "InvalidParameter");
    assert!(
        payload["message"].as_str().unwrap().contains("--all"),
        "the fix belongs in the message: {payload}"
    );
}

#[test]
fn lookup_misses_are_command_not_found() {
    let home = test_home();
    seed(
        home.path(),
        "pods",
        "kubectl get pods",
        "List pods.",
        "kube",
    );

    for args in [
        vec!["pick", "-n", "nope", "--json"],
        vec!["eval", "nope", "--json"],
        vec!["remove-namespace", "-n", "nope", "--json"],
    ] {
        let output = hoard(home.path(), &args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{} must exit 2: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(stdout_json(&output)["error_type"], "CommandNotFound");
    }
}

#[test]
fn parse_errors_honour_bundled_short_flags() {
    let home = test_home();
    // `-sj` is `--simple --json`; the invocation is still invalid.
    let output = hoard(home.path(), &["list", "-sj", "--nope"]);
    assert_eq!(output.status.code(), Some(1));
    let payload = stdout_json(&output);
    assert_eq!(payload["error_type"], "ParseError");
    assert!(payload.get("status").is_some(), "errors carry `status`");
}
