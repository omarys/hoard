//! Parsing of zsh history files.
//!
//! zsh writes one entry per line. With `EXTENDED_HISTORY` set — the default in
//! most setups — every entry is prefixed with `: <epoch>:<duration>;`, and a
//! command that spans several lines is stored literally: the lines after the
//! header, up to the next `: ` header, belong to the same command.
//!
//! Bytes with the high bit set are *metafied*: `0x83 x` stands for `x | 0x80`,
//! so UTF-8 and control bytes survive a history round-trip. Undecoded
//! metafication is why imported commands sometimes contain `\x83`-garbage.

use std::path::Path;

/// zsh's `Meta` byte, prefix of every metafied character.
const META: u8 = 0x83;

/// One entry of a zsh history file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZshEntry {
    /// The command as stored, metafication and `\r` removed but not trimmed.
    pub command: String,
    /// When the command ran, when the file uses the extended format.
    pub timestamp: Option<i64>,
}

/// Read and parse a zsh history file.
pub fn parse_file(path: &Path) -> std::io::Result<Vec<ZshEntry>> {
    Ok(parse_bytes(&std::fs::read(path)?))
}

/// Parse the raw bytes of a zsh history file.
pub fn parse_bytes(bytes: &[u8]) -> Vec<ZshEntry> {
    let text = unmetafy(bytes);
    let extended = text.split('\n').any(|line| parse_header(line).is_some());

    let mut entries: Vec<ZshEntry> = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if extended {
            if let Some((timestamp, command)) = parse_header(line) {
                entries.push(ZshEntry { command, timestamp });
            } else if let Some(last) = entries.last_mut() {
                // Continuation of a multi-line command.
                last.command.push('\n');
                last.command.push_str(line);
            }
        } else if !line.trim().is_empty() {
            entries.push(ZshEntry {
                command: line.to_string(),
                timestamp: None,
            });
        }
    }
    // The final newline of the file (and blank separator lines) would otherwise
    // become a trailing continuation line of the last command.
    for entry in &mut entries {
        while entry.command.ends_with('\n') {
            entry.command.pop();
        }
    }
    entries
}

/// Decode zsh's metafied bytes: `0x83 x` becomes `x | 0x80`.
fn unmetafy(bytes: &[u8]) -> String {
    if !bytes.contains(&META) {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut bytes = bytes.iter().copied();
    while let Some(byte) = bytes.next() {
        if byte == META {
            match bytes.next() {
                Some(next) => decoded.push(next | 0x80),
                // A trailing Meta byte cannot be decoded; keep it visible.
                None => decoded.push(byte),
            }
        } else {
            decoded.push(byte);
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// Split an extended-format header into `(timestamp, command)`.
///
/// Returns `None` for continuation lines and for plain, non-extended history.
fn parse_header(line: &str) -> Option<(Option<i64>, String)> {
    let rest = line.strip_prefix(": ")?;
    let (epoch, after_epoch) = rest.split_once(':')?;
    if epoch.is_empty() || !epoch.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (duration, command) = after_epoch.split_once(';')?;
    if !duration.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((epoch.parse().ok(), command.to_string()))
}

#[cfg(test)]
mod test_zsh {
    use super::*;

    #[test]
    fn extended_format_carries_timestamp_and_keeps_command() {
        let entries = parse_bytes(b": 1700000000:0;ls -la\n: 1700000005:12;git status\n");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].command, "ls -la");
        assert_eq!(entries[0].timestamp, Some(1_700_000_000));
        assert_eq!(entries[1].command, "git status");
        assert_eq!(entries[1].timestamp, Some(1_700_000_005));
    }

    #[test]
    fn multi_line_entries_are_joined_until_the_next_header() {
        let history =
            b": 1700000000:0;for i in 1 2 3; do\n  echo $i\ndone\n: 1700000001:0;echo after\n";
        let entries = parse_bytes(history);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].command, "for i in 1 2 3; do\n  echo $i\ndone");
        assert_eq!(entries[1].command, "echo after");
    }

    #[test]
    fn leading_space_paste_is_preserved_for_the_cleaner() {
        let entries = parse_bytes(b": 1700000000:0;   kubectl get pods\n");
        assert_eq!(entries[0].command, "   kubectl get pods");
    }

    #[test]
    fn plain_history_yields_one_entry_per_line() {
        let entries = parse_bytes(b"ls\ncd /tmp\necho hi\n");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].command, "ls");
        assert_eq!(entries[2].command, "echo hi");
        assert!(entries.iter().all(|entry| entry.timestamp.is_none()));
    }

    #[test]
    fn metafied_bytes_decode_to_utf8() {
        // `café` is 0xC3 0xA9 in UTF-8, metafied as 0x83 0x43 0x83 0x29.
        let entries = parse_bytes(b": 1700000000:0;echo caf\x83\x43\x83\x29\n");
        assert_eq!(entries[0].command, "echo café");
    }

    #[test]
    fn unmetafy_leaves_plain_bytes_alone_and_decodes_multibyte_text() {
        assert_eq!(unmetafy(b"echo hi"), "echo hi");
        // 日本 is E6 97 A5 E6 9C AC, metafied one byte at a time.
        let metafied = [
            0x83, 0x66, 0x83, 0x17, 0x83, 0x25, 0x83, 0x66, 0x83, 0x1c, 0x83, 0x2c,
        ];
        assert_eq!(unmetafy(&metafied), "日本");
    }

    #[test]
    fn carriage_returns_are_dropped() {
        let entries = parse_bytes(b": 1700000000:0;echo hi\r\n");
        assert_eq!(entries[0].command, "echo hi");
    }
}
