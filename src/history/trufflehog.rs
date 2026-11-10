//! Optional trufflehog pass over the import candidates.
//!
//! trufflehog finds credentials that hoard's built-in rules miss. It is asked
//! once for the whole batch — one file per candidate in a temporary directory,
//! then `<bin> filesystem --json --no-update --no-verification <dir>` — because
//! spawning it per command would dominate the runtime.
//!
//! Any problem with the binary is reported as an error and the caller carries
//! on with the built-in redaction only: importing history must never fail
//! because a scanner is missing or misbehaves.

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// How the `--trufflehog` flag was configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Use `trufflehog` when it is on `PATH` (the default).
    Auto,
    /// Never scan, use the built-in redaction only.
    Off,
    /// Use this binary; a bare name is looked up on `PATH`.
    Binary(PathBuf),
}

impl Mode {
    /// Parse the `--trufflehog` value: `auto`, `off` or a binary path.
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "" | "auto" => Self::Auto,
            "off" => Self::Off,
            binary => Self::Binary(PathBuf::from(binary)),
        }
    }

    /// The binary to run, if one is available.
    pub fn resolve(&self) -> Option<PathBuf> {
        match self {
            Self::Off => None,
            Self::Auto => find_binary("trufflehog"),
            Self::Binary(path) if path.components().count() > 1 => {
                path.is_file().then(|| path.clone())
            }
            Self::Binary(path) => find_binary(&path.to_string_lossy()),
        }
    }
}

fn find_binary(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// One secret trufflehog reported, tied back to the candidate it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Index into the slice of candidate texts.
    pub index: usize,
    /// The exact secret text, replaced verbatim in the candidate.
    pub raw: String,
    /// Detector name, e.g. `Github`, used for the placeholder name.
    pub detector: String,
}

/// Run one batched scan over `texts`.
pub fn run(binary: &Path, texts: &[&str]) -> Result<Vec<Finding>, String> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    // The candidates contain exactly the secrets this pass exists to catch, so
    // the scratch directory must be private to this user and must always be
    // removed: `tempfile` creates it 0700 and drops it on every exit path.
    let dir = tempfile::Builder::new()
        .prefix("hoard-history-")
        .tempdir()
        .map_err(|err| format!("could not create a temporary directory: {err}"))?;
    for (index, text) in texts.iter().enumerate() {
        fs::write(dir.path().join(format!("{index:05}.txt")), text)
            .map_err(|err| format!("could not write a candidate for scanning: {err}"))?;
    }

    let output = Command::new(binary)
        .args([
            "filesystem",
            "--json",
            "--no-update",
            "--no-verification",
            // Report unverified hits too; newer trufflehog defaults hide them.
            "--results=verified,unverified,unknown",
        ])
        .arg(dir.path())
        .output();
    drop(dir);

    let output = output.map_err(|err| format!("could not run {}: {err}", binary.display()))?;
    let findings = parse_findings(&String::from_utf8_lossy(&output.stdout));
    if findings.is_empty() && !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let first = stderr.lines().find(|line| !line.trim().is_empty());
        return Err(format!(
            "{} exited with {}{}",
            binary.display(),
            output.status,
            first
                .map(|line| format!(": {}", line.trim()))
                .unwrap_or_default()
        ));
    }
    Ok(findings)
}

/// Parse trufflehog's JSONL output into findings.
pub fn parse_findings(stdout: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for line in stdout.lines() {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        // Info records (`{"level":"info-0",...}`) carry no `Raw`.
        let Some(raw) = record
            .get("Raw")
            .and_then(Value::as_str)
            .filter(|raw| !raw.is_empty())
        else {
            continue;
        };
        let Some(file) = record
            .pointer("/SourceMetadata/Data/Filesystem/file")
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(index) = Path::new(file)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.parse::<usize>().ok())
        else {
            continue;
        };
        findings.push(Finding {
            index,
            raw: raw.to_string(),
            detector: record
                .get("DetectorName")
                .and_then(Value::as_str)
                .unwrap_or("secret")
                .to_string(),
        });
    }
    findings
}

/// What [`apply`] managed to do.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Applied {
    /// Candidates whose text changed.
    pub candidates: usize,
    /// Findings actually replaced in a candidate.
    pub findings: usize,
    /// Candidates holding a finding that could not be located verbatim, so the
    /// secret may still be there. The caller must not import these.
    pub unplaced: Vec<usize>,
}

/// Replace every finding's exact span in `texts` with a `#name!` placeholder.
///
/// A finding whose `Raw` is not present verbatim (trufflehog also reports
/// decoded and trimmed forms) is reported back as unplaced instead of being
/// silently skipped: claiming a redaction that did not happen is worse than
/// dropping the candidate.
pub fn apply(findings: &[Finding], texts: &mut [String], token: &str, ending: &str) -> Applied {
    let mut placeholders = Placeholders::new(token, ending);
    let mut seeded = HashSet::new();
    let mut changed = HashSet::new();
    let mut applied = 0usize;
    let mut unplaced = Vec::new();
    // Longest secret first, so a secret containing another one stays intact.
    let mut order: Vec<&Finding> = findings.iter().collect();
    order.sort_by_key(|finding| std::cmp::Reverse(finding.raw.len()));

    for finding in order {
        let Some(text) = texts.get_mut(finding.index) else {
            continue;
        };
        if !text.contains(&finding.raw) {
            if !unplaced.contains(&finding.index) {
                unplaced.push(finding.index);
            }
            continue;
        }
        if seeded.insert(finding.index) {
            placeholders.seed(text);
        }
        let placeholder = placeholders.get(&finding.raw, &finding.detector);
        *text = text.replace(&finding.raw, &placeholder);
        changed.insert(finding.index);
        applied += 1;
    }
    Applied {
        candidates: changed.len(),
        findings: applied,
        unplaced,
    }
}

/// `#name!` placeholders that keep the same secret on the same name.
struct Placeholders {
    token: String,
    ending: String,
    /// Parameter names already used, so a name is never assigned twice.
    taken: HashSet<String>,
    /// (secret) -> placeholder, so a repeated secret stays one parameter.
    assigned: HashMap<String, String>,
}

impl Placeholders {
    fn new(token: &str, ending: &str) -> Self {
        Self {
            token: token.to_string(),
            ending: ending.to_string(),
            taken: HashSet::new(),
            assigned: HashMap::new(),
        }
    }

    /// Record the parameter names a candidate already contains.
    fn seed(&mut self, text: &str) {
        if self.token.is_empty() || self.ending.is_empty() {
            return;
        }
        let mut rest = text;
        while let Some(start) = rest.find(&self.token) {
            let after = &rest[start + self.token.len()..];
            let Some(end) = after.find(&self.ending) else {
                break;
            };
            self.taken.insert(after[..end].to_string());
            rest = &after[end + self.ending.len()..];
        }
    }

    fn get(&mut self, raw: &str, detector: &str) -> String {
        if let Some(existing) = self.assigned.get(raw) {
            return existing.clone();
        }
        let base = slug(detector);
        let mut name = base.clone();
        let mut suffix = 2;
        while self.taken.contains(&name) {
            name = format!("{base}{suffix}");
            suffix += 1;
        }
        self.taken.insert(name.clone());
        let placeholder = format!("{}{name}{}", self.token, self.ending);
        self.assigned.insert(raw.to_string(), placeholder.clone());
        placeholder
    }
}

/// `PrivateKey` -> `private_key`, `AWS` -> `aws`, `Github` -> `github`.
pub fn slug(detector: &str) -> String {
    let mut slug = String::new();
    let mut previous_lower = false;
    for ch in detector.chars() {
        if ch.is_ascii_uppercase() && previous_lower {
            slug.push('_');
        }
        previous_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        for lower in ch.to_lowercase() {
            slug.push(if lower.is_ascii_alphanumeric() {
                lower
            } else {
                '_'
            });
        }
    }
    let slug = slug.trim_matches('_').replace("__", "_");
    if slug.is_empty() {
        "secret".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod test_trufflehog {
    use super::*;
    use std::io::Write;

    fn canned() -> String {
        [
            r#"{"level":"info-0","logger":"trufflehog","msg":"running source"}"#,
            r#"{"SourceMetadata":{"Data":{"Filesystem":{"file":"00000.txt","line":1}}},"DetectorName":"Github","Raw":"ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
            r#"{"SourceMetadata":{"Data":{"Filesystem":{"file":"00001.txt","line":2}}},"DetectorName":"PrivateKey","Raw":"-----BEGIN RSA PRIVATE KEY-----"}"#,
            r#"{"SourceMetadata":{"Data":{"Filesystem":{"file":"00003.txt","line":1}}},"DetectorName":"AWS","Raw":"AKIAIOSFODNN7EXAMPLE"}"#,
        ]
        .join("\n")
    }

    #[test]
    fn parses_canned_jsonl_and_maps_files_to_indices() {
        let findings = parse_findings(&canned());
        assert_eq!(findings.len(), 3);
        assert_eq!(findings[0].index, 0);
        assert_eq!(findings[0].detector, "Github");
        assert_eq!(findings[1].index, 1);
        assert_eq!(findings[2].index, 3);
    }

    #[test]
    fn replaces_the_exact_span_with_the_detector_placeholder() {
        let mut texts = vec![
            "curl -H 'Authorization: token ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' https://api"
                .to_string(),
            "echo keep".to_string(),
        ];
        let findings = parse_findings(&canned());
        let applied = apply(&findings, &mut texts, "#", "!");
        assert_eq!(applied.findings, 1);
        assert_eq!(applied.candidates, 1);
        // The PrivateKey finding points at a candidate that does not contain
        // its `Raw`, and the AWS finding points past the end of `texts`: both
        // must be reported rather than silently counted as redacted.
        assert_eq!(applied.unplaced, vec![1]);
        assert_eq!(
            texts[0],
            "curl -H 'Authorization: token #github!' https://api"
        );
        assert_eq!(texts[1], "echo keep");
    }

    #[test]
    fn same_secret_keeps_one_placeholder_and_slugs_the_detector() {
        let raw = "ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let findings = vec![
            Finding {
                index: 0,
                raw: raw.to_string(),
                detector: "Github".to_string(),
            },
            Finding {
                index: 0,
                raw: raw.to_string(),
                detector: "Github".to_string(),
            },
        ];
        let mut texts = vec![format!("a {raw} b {raw}")];
        apply(&findings, &mut texts, "#", "!");
        assert_eq!(texts[0], "a #github! b #github!");
        assert_eq!(slug("PrivateKey"), "private_key");
        assert_eq!(slug("AWS"), "aws");
        assert_eq!(slug("Github"), "github");
    }

    #[test]
    fn distinct_secrets_with_the_same_detector_get_suffixed_names() {
        let findings = vec![
            Finding {
                index: 0,
                raw: "sk-first".to_string(),
                detector: "OpenAI".to_string(),
            },
            Finding {
                index: 0,
                raw: "sk-second".to_string(),
                detector: "OpenAI".to_string(),
            },
        ];
        let mut texts = vec!["x sk-first y sk-second".to_string()];
        apply(&findings, &mut texts, "#", "!");
        // Longest secret first, so "sk-second" gets the plain name.
        assert_eq!(texts[0], "x #open_ai2! y #open_ai!");
    }

    #[test]
    fn existing_parameters_are_not_reused() {
        let findings = vec![Finding {
            index: 0,
            raw: "ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            detector: "Github".to_string(),
        }];
        let mut texts = vec!["echo #github! ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()];
        apply(&findings, &mut texts, "#", "!");
        assert_eq!(texts[0], "echo #github! #github2!");
    }

    #[test]
    fn a_missing_binary_is_reported_as_an_error() {
        let err = run(Path::new("/definitely/not/trufflehog"), &["echo hi"]).unwrap_err();
        assert!(err.contains("could not run"), "{err}");
    }

    #[test]
    fn a_failing_binary_is_reported_as_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("broken-trufflehog");
        let mut file = fs::File::create(&script).unwrap();
        writeln!(file, "#!/bin/sh\necho 'boom' >&2\nexit 3").unwrap();
        drop(file);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
            // A freshly written script can hit ETXTBSY when it is exec'd before
            // the write side is fully released; give the kernel a moment.
            std::thread::sleep(std::time::Duration::from_millis(20));
            let err = run(&script, &["echo hi"]).unwrap_err();
            assert!(err.contains("exited with"), "{err}");
        }
    }

    #[test]
    fn mode_parsing_and_resolution() {
        assert_eq!(Mode::parse("auto"), Mode::Auto);
        assert_eq!(Mode::parse("off"), Mode::Off);
        assert_eq!(
            Mode::parse("/usr/bin/trufflehog"),
            Mode::Binary(PathBuf::from("/usr/bin/trufflehog"))
        );
        assert!(Mode::Off.resolve().is_none());
        assert!(Mode::Binary(PathBuf::from("/nope/th")).resolve().is_none());
    }
}
