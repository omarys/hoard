//! Extraction of hoardable commands from pi / pi-bolt agent sessions.
//!
//! A session is a JSONL transcript at
//! `<root>/<cwd-encoded-dir>/<timestamp>_<uuid>.jsonl`. Records look like
//!
//! ```json
//! {"type":"message","message":{"role":"assistant","content":[
//!   {"type":"toolCall","name":"bash","arguments":{"command":"ls"}}]}}
//! ```
//!
//! Only three tool calls are interesting: shell commands (`bash`), Python files
//! written to disk (`write`/`create_file` with a `.py` path) and inline Python
//! (`ctx_execute` with `language: python`). The Anthropic spellings (`tool_use`
//! with `input`) are accepted too, because transcripts are copied around.

use crate::core::CommandKind;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Deepest directory level a session file may live in below a session root.
const MAX_DEPTH: usize = 4;

/// One command found in a session transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCommand {
    /// Shell command or Python source.
    pub kind: CommandKind,
    /// The extracted source, cleaned of transport artefacts by the caller.
    pub command: String,
    /// When the session started, used for the Python entry description.
    pub captured: Option<SystemTime>,
}

/// Session roots that exist on this machine: `~/.pi/agent/sessions` and
/// `~/.pi-bolt/agent/sessions`.
pub fn default_roots() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    [
        home.join(".pi/agent/sessions"),
        home.join(".pi-bolt/agent/sessions"),
    ]
    .into_iter()
    .filter(|root| root.is_dir())
    .collect()
}

/// Session transcripts that were read, and the commands found in them.
#[derive(Debug, Default)]
pub struct Extracted {
    /// Every shell command and Python source found, in file order.
    pub commands: Vec<SessionCommand>,
    /// How many transcript files were read.
    pub files: usize,
}

/// Read every `*.jsonl` below `roots`, in a stable order.
pub fn collect(roots: &[PathBuf]) -> Extracted {
    let mut files = Vec::new();
    for root in roots {
        collect_files(root, &mut files, 0);
    }
    files.sort();

    let mut commands = Vec::new();
    for file in &files {
        let Ok(bytes) = fs::read(file) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        // Failures are scoped to one transcript: ids are only unique within it,
        // so a `call-1` that failed here must not drop `call-1` elsewhere.
        let mut failed = HashSet::new();
        for line in text.lines() {
            if let Ok(record) = serde_json::from_str::<Value>(line) {
                collect_failures(&record, &mut failed);
            }
        }
        let captured = captured_at(file);
        for line in text.lines() {
            let Ok(record) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            extract_record(&record, captured, &failed, &mut commands);
        }
    }
    Extracted {
        commands,
        files: files.len(),
    }
}

/// Record the ids of tool calls whose result reports an error.
fn collect_failures(record: &Value, failed: &mut HashSet<String>) {
    let message = record.get("message").unwrap_or(record);
    let Some(items) = message.get("content").and_then(Value::as_array) else {
        return;
    };
    for item in items {
        let errored = item
            .get("isError")
            .or_else(|| item.get("is_error"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || item
                .get("status")
                .and_then(Value::as_str)
                .is_some_and(|status| matches!(status, "error" | "failed"));
        if !errored {
            continue;
        }
        let id = item
            .get("toolCallId")
            .or_else(|| item.get("tool_use_id"))
            .or_else(|| item.get("id"))
            .and_then(Value::as_str);
        if let Some(id) = id {
            failed.insert(id.to_string());
        }
    }
}

fn collect_files(dir: &Path, files: &mut Vec<PathBuf>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, files, depth + 1);
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            files.push(path);
        }
    }
}

/// The session start time: the timestamp in the file name, else its mtime.
fn captured_at(path: &Path) -> Option<SystemTime> {
    if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str())
        && let Some((stamp, _)) = stem.split_once('_')
        && let Ok(value) = stamp.parse::<i64>()
    {
        // Millisecond timestamps are 13 digits, second timestamps 10.
        let millis = if value > 100_000_000_000 {
            value
        } else {
            value * 1_000
        };
        if let Ok(millis) = u64::try_from(millis) {
            return UNIX_EPOCH.checked_add(Duration::from_millis(millis));
        }
    }
    fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

fn extract_record(
    record: &Value,
    captured: Option<SystemTime>,
    failed: &HashSet<String>,
    out: &mut Vec<SessionCommand>,
) {
    let message = record.get("message").unwrap_or(record);
    let Some(items) = message.get("content").and_then(Value::as_array) else {
        return;
    };
    for item in items {
        extract_tool_call(item, captured, failed, out);
    }
}

fn extract_tool_call(
    item: &Value,
    captured: Option<SystemTime>,
    failed: &HashSet<String>,
    out: &mut Vec<SessionCommand>,
) {
    let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
    if !matches!(kind, "toolCall" | "tool_call" | "tool_use" | "tool") {
        return;
    }
    // A failed call still happened, but its command is not a useful recipe.
    if item
        .get("id")
        .or_else(|| item.get("toolCallId"))
        .and_then(Value::as_str)
        .is_some_and(|id| failed.contains(id))
    {
        return;
    }
    let Some(name) = item
        .get("name")
        .or_else(|| item.get("tool"))
        .and_then(Value::as_str)
    else {
        return;
    };
    let Some(arguments) = item
        .get("arguments")
        .or_else(|| item.get("input"))
        .or_else(|| item.get("parameters"))
    else {
        return;
    };

    let found = match name.to_ascii_lowercase().as_str() {
        "bash" | "shell" => {
            text_field(arguments, &["command", "cmd"]).map(|command| SessionCommand {
                kind: CommandKind::Shell,
                command,
                captured,
            })
        }
        "write" | "create_file" | "write_file" | "create" => {
            let path = text_field(arguments, &["path", "file_path"]).unwrap_or_default();
            let content = text_field(arguments, &["content", "text", "file_text"]);
            match (path.ends_with(".py"), content) {
                (true, Some(content)) => Some(SessionCommand {
                    kind: CommandKind::Python,
                    command: content,
                    captured,
                }),
                _ => None,
            }
        }
        "ctx_execute" => {
            let language = text_field(arguments, &["language"]).unwrap_or_default();
            let is_python = language.to_ascii_lowercase().starts_with("python");
            match (is_python, text_field(arguments, &["code", "script"])) {
                (true, Some(code)) => Some(SessionCommand {
                    kind: CommandKind::Python,
                    command: code,
                    captured,
                }),
                _ => None,
            }
        }
        _ => None,
    };
    out.extend(found);
}

/// First non-empty string value among `keys`.
fn text_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .map(str::to_string)
    })
}

#[cfg(test)]
mod test_sessions {
    use super::*;
    use std::io::Write;

    fn session_root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn write_session(root: &Path, name: &str, lines: &[&str]) {
        let dir = root.join("--home-user--");
        fs::create_dir_all(&dir).unwrap();
        let mut file = fs::File::create(dir.join(name)).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
    }

    #[test]
    fn extracts_bash_python_file_and_inline_python() {
        let root = session_root();
        write_session(
            root.path(),
            "1700000000000_abc.jsonl",
            &[
                r#"{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"bash","arguments":{"command":"kubectl get pods"}}]}}"#,
                r#"{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"write","arguments":{"path":"/tmp/scan.py","content":"print(1)"}}]}}"#,
                r#"{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"ctx_execute","arguments":{"language":"python","code":"print(2)"}}]}}"#,
                r#"{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","name":"bash","arguments":{"command":"import os"}}]}}"#,
            ],
        );
        let commands = collect(&[root.path().to_path_buf()]);
        assert_eq!(commands.files, 1);
        let commands = commands.commands;
        assert_eq!(commands.len(), 4);
        assert_eq!(commands[0].kind, CommandKind::Shell);
        assert_eq!(commands[0].command, "kubectl get pods");
        assert_eq!(commands[1].kind, CommandKind::Python);
        assert_eq!(commands[1].command, "print(1)");
        assert_eq!(commands[2].kind, CommandKind::Python);
        assert_eq!(commands[2].command, "print(2)");
        assert_eq!(commands[3].command, "import os");
        // The timestamp prefix of the file name is the capture date.
        assert!(commands[0].captured.is_some());
    }

    #[test]
    fn accepts_anthropic_tool_use_and_input_spelling() {
        let root = session_root();
        write_session(
            root.path(),
            "1700000000_x.jsonl",
            &[
                r#"{"type":"message","message":{"content":[{"type":"tool_use","name":"bash","input":{"command":"echo hi"}}]}}"#,
                r#"{"type":"message","message":{"content":[{"type":"tool_use","name":"create_file","input":{"file_path":"a.py","content":"x = 1"}}]}}"#,
            ],
        );
        let commands = collect(&[root.path().to_path_buf()]).commands;
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].command, "echo hi");
        assert_eq!(commands[1].command, "x = 1");
    }

    #[test]
    fn non_python_writes_and_broken_lines_are_ignored() {
        let root = session_root();
        write_session(
            root.path(),
            "1700000000_x.jsonl",
            &[
                r##"{"type":"message","message":{"content":[{"type":"toolCall","name":"write","arguments":{"path":"notes.md","content":"# hi"}}]}}"##,
                "{not json",
                r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#,
            ],
        );
        assert!(collect(&[root.path().to_path_buf()]).commands.is_empty());
    }

    #[test]
    fn failed_tool_calls_are_dropped() {
        let root = session_root();
        write_session(
            root.path(),
            "1700000000_x.jsonl",
            &[
                // A failed call: the result arrives in a later record.
                r#"{"type":"message","message":{"content":[{"type":"toolCall","id":"call-bad","name":"bash","arguments":{"command":"kubectl get pod typo"}}]}}"#,
                r#"{"type":"message","message":{"content":[{"type":"toolResult","toolCallId":"call-bad","isError":true,"content":[{"type":"text","text":"Error from server (NotFound)"}]}]}}"#,
                // A successful one, and an error reported via `status`.
                r#"{"type":"message","message":{"content":[{"type":"toolCall","id":"call-good","name":"bash","arguments":{"command":"kubectl get pods"}}]}}"#,
                r#"{"type":"message","message":{"content":[{"type":"toolResult","toolCallId":"call-good","isError":false}]}}"#,
                r#"{"type":"message","message":{"content":[{"type":"toolCall","id":"call-err","name":"bash","arguments":{"command":"hoard nope"}}]}}"#,
                r#"{"type":"message","message":{"content":[{"type":"toolResult","toolCallId":"call-err","status":"error"}]}}"#,
                // A failed python write is dropped too.
                r#"{"type":"message","message":{"content":[{"type":"toolCall","id":"call-py","name":"write","arguments":{"path":"/tmp/broken.py","content":"raise SystemExit(1)"}}]}}"#,
                r#"{"type":"message","message":{"content":[{"type":"toolResult","toolCallId":"call-py","isError":true}]}}"#,
            ],
        );
        let commands = collect(&[root.path().to_path_buf()]).commands;
        assert_eq!(commands.len(), 1, "only the successful call survives");
        assert_eq!(commands[0].command, "kubectl get pods");
    }

    #[test]
    fn missing_roots_are_ignored() {
        assert!(
            collect(&[PathBuf::from("/definitely/not/a/session/root")])
                .commands
                .is_empty()
        );
    }
}
