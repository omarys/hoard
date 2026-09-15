use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn test_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join(".config/hoard");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("config.yml"),
        "version: 2.0.0\ndefault_namespace: archive\nquery_prefix: '>'\nread_from_current_directory: true\n",
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

fn success(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn archives_source_and_summary_and_restores_exact_bytes() {
    let home = test_home();
    // Use the project-local trove and verify that legacy shell entries still work.
    fs::write(
        home.path().join("trove.yml"),
        "version: 2.0.0\ncommands:\n- name: shell\n  namespace: default\n  command: echo shell\n  description: A shell command\n",
    )
    .unwrap();
    let source = "#!/usr/bin/env python3\r\n# Keep #comments! and @tokens.\r\nimport sys\r\nprint(\"it's café: $HOME `uname` \\\\ end\")\r\nprint(sys.argv[1])\r\nprint(sys.stdin.read(), end=\"\")\r\n\r\n";
    fs::write(home.path().join("report.py"), source).unwrap();
    success(hoard(
        home.path(),
        &[
            "new",
            "--script",
            "report.py",
            "-n",
            "report",
            "--summary",
            "Report arguments and stdin without expanding shell syntax.",
            "-t",
            "python,report",
        ],
    ));
    // The archive must not depend on the original file remaining on disk.
    fs::remove_file(home.path().join("report.py")).unwrap();
    assert_eq!(
        success(hoard(home.path(), &["pick", "-n", "report", "--raw"])),
        source.as_bytes()
    );
    assert_eq!(
        success(hoard(home.path(), &["pick", "-n", "shell"])),
        b"echo shell\n"
    );
    let yaml = success(hoard(
        home.path(),
        &["list", "--json", "-f", "Report arguments"],
    ));
    let listing: serde_yaml::Value = serde_yaml::from_slice(&yaml).unwrap();
    let entries = listing["commands"].as_sequence().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["kind"].as_str(), Some("python"));
    assert_eq!(entries[0]["namespace"].as_str(), Some("archive"));
    assert_eq!(entries[0]["command"].as_str(), Some(source));
    assert_eq!(
        entries[0]["description"].as_str(),
        Some("Report arguments and stdin without expanding shell syntax.")
    );
    assert!(home.path().join("trove.db").exists());
    assert!(!home.path().join(".config/hoard/trove.db").exists());

    // Import/export uses the same YAML serializer as the structured listing.
    let exported = success(hoard(home.path(), &["list", "--json"]));
    let restored = test_home();
    fs::write(restored.path().join("trove.yml"), exported).unwrap();
    assert_eq!(
        success(hoard(restored.path(), &["pick", "-n", "report", "--raw"])),
        source.as_bytes()
    );
}

#[test]
fn rejects_invalid_script_saves_without_overwriting_entries() {
    let home = test_home();
    fs::write(home.path().join("script.py"), "print('saved')").unwrap();
    let valid = [
        "new",
        "--script",
        "script.py",
        "-n",
        "saved",
        "-d",
        "Print a message.",
        "--namespace",
        "python",
    ];
    success(hoard(home.path(), &valid));
    for args in [
        vec!["new", "--script", "script.py", "-n", "missing-summary"],
        vec!["new", "--script", "script.py", "-d", "Missing name"],
        vec!["new", "--script", "script.py", "-n", "blank", "-d", " \t\n"],
        vec![
            "new",
            "--script",
            "missing.py",
            "-n",
            "missing",
            "-d",
            "Unreadable source",
        ],
        vec![
            "new",
            "--script",
            "script.py",
            "-n",
            "bad name",
            "-d",
            "Invalid name",
        ],
        vec![
            "new",
            "--script",
            "script.py",
            "-n",
            "empty-ns",
            "-d",
            "Invalid namespace",
            "--namespace",
            " ",
        ],
        vec![
            "new",
            "--script",
            "script.py",
            "-n",
            "conflict",
            "-d",
            "Ambiguous source",
            "-c",
            "echo shell",
        ],
        valid.to_vec(),
    ] {
        let output = hoard(home.path(), &args);
        assert!(!output.status.success(), "unexpected success for {args:?}");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
    }
    for content in [b" \n\t".as_slice(), b"\xff", b"print('nul')\0"] {
        fs::write(home.path().join("invalid.py"), content).unwrap();
        assert!(
            !hoard(
                home.path(),
                &[
                    "new",
                    "--script",
                    "invalid.py",
                    "-n",
                    "invalid",
                    "-d",
                    "Invalid source"
                ]
            )
            .status
            .success()
        );
    }
    assert_eq!(
        success(hoard(home.path(), &["pick", "-n", "saved", "--raw"])),
        b"print('saved')"
    );
    let yaml = success(hoard(home.path(), &["list", "--json"]));
    let listing: serde_yaml::Value = serde_yaml::from_slice(&yaml).unwrap();
    assert_eq!(listing["commands"].as_sequence().unwrap().len(), 1);
    assert_eq!(listing["commands"][0]["namespace"].as_str(), Some("python"));
}

#[test]
#[cfg(unix)]
fn picked_script_runs_with_arguments_and_stdin_without_shell_expansion() {
    let home = test_home();
    fs::write(
        home.path().join("script.py"),
        "# #comments! are not Hoard parameters\nimport sys\nprint(\"it's café: $HOME `uname` \\\\ end\")\nprint(sys.argv[1])\nprint(sys.stdin.read(), end='')\n",
    )
    .unwrap();
    success(hoard(
        home.path(),
        &[
            "new",
            "--script",
            "script.py",
            "-n",
            "run",
            "-d",
            "Echo arguments and stdin.",
        ],
    ));
    let invocation =
        String::from_utf8(success(hoard(home.path(), &["pick", "-n", "run"]))).unwrap();
    assert_eq!(invocation.lines().count(), 1);
    let mut child = Command::new("sh")
        .args(["-c", &format!("{} 'hello world'", invocation.trim_end())])
        .current_dir(home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"input\n").unwrap();
    let output = success(child.wait_with_output().unwrap());
    assert_eq!(
        output,
        "it's café: $HOME `uname` \\ end\nhello world\ninput\n".as_bytes()
    );
}
