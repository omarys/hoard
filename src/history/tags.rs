//! Searchable tags derived from an imported command.
//!
//! History imports used to carry only `imported` and the source tag, so an
//! archived `docker compose up -d` was invisible to a search for "containers".
//! [`tags_for`] derives a small, stable tag set from the command itself — the
//! program, a few synonyms and the intents the command expresses — and appends
//! the fixed import tags. Tags never influence *whether* a command is imported,
//! only how findable the stored entry is.

use crate::core::CommandKind;
use crate::history::Origin;
use regex::Regex;
use std::sync::OnceLock;

/// Upper bound on the tags of one imported entry, so entries stay scannable.
pub const MAX_TAGS: usize = 8;

/// At most this many intent tags are attached.
const MAX_INTENTS: usize = 3;

/// Prefixes that wrap the real program (`sudo time env kubectl …`).
const WRAPPERS: &[&str] = &["sudo", "time", "env", "nohup", "command", "exec", "xargs"];

/// Intent tags in priority order; only the first [`MAX_INTENTS`] matches count.
///
/// The order is the specificity order: `secrets` says more about a command than
/// the generic `files` it may also touch.
const INTENTS: &[(&str, &str)] = &[
    (
        "secrets",
        r"secret|password|passwd|token|api[_-]?key|private[_-]?key|vault|sops",
    ),
    // Tool families, because the program tag is lost when a wrapper runs the
    // tool: `rtk kubectl logs …` is tagged `rtk` but must still answer a search
    // for "kubernetes".
    (
        "kubernetes",
        r"kubectl|\bk8s\b|helm |deployment|statefulset|daemonset|kustomize",
    ),
    ("containers", r"docker|podman|container"),
    ("aws", r"\baws\b|s3api|\becr\b|\becs\b|\bssm\b"),
    (
        "network",
        r"curl|wget|http://|https://|ssh |nc |ping |dig |--host",
    ),
    ("storage", r"s3|bucket|scp|rsync|tar |rclone"),
    (
        "deploy",
        r"apply|deploy|rollout|helm |terraform apply|tofu apply|systemctl (start|restart)",
    ),
    ("logs", r"log|journalctl|tail -f"),
    ("build", r"build|make |compile|install"),
    ("test", r"test|lint|check|fmt|clippy"),
    ("process", r"\bps\b|kill|pgrep|pkill|systemctl"),
    ("files", r"find |rg --files|\bls\b|mkdir|rm -rf"),
    ("permissions", r"chmod|chown|sudo"),
];

/// Program -> related search terms. Deliberately short: every entry is a guess
/// a human would also type, not a taxonomy.
fn synonyms(program: &str) -> &'static [&'static str] {
    match program {
        "kubectl" => &["kubernetes", "k8s"],
        "helm" => &["kubernetes", "helm"],
        "oc" => &["openshift", "kubernetes"],
        "docker" => &["containers"],
        "podman" => &["containers"],
        "docker-compose" => &["containers", "compose"],
        "aws" => &["aws"],
        "tofu" => &["terraform", "iac"],
        "terraform" => &["terraform", "iac"],
        "rg" => &["ripgrep", "search"],
        "fd" => &["find", "files"],
        "grep" => &["search"],
        "curl" => &["http", "network"],
        "wget" => &["http", "network"],
        "http" => &["http", "network"],
        "ssh" => &["ssh", "network"],
        "scp" => &["ssh", "network", "copy"],
        "rsync" => &["copy", "sync"],
        "git" => &["vcs"],
        "gh" => &["github", "vcs"],
        "cargo" => &["rust", "build"],
        "npm" | "pnpm" | "yarn" | "bun" => &["nodejs", "packages"],
        "uv" | "pip" | "pipx" | "poetry" => &["python", "packages"],
        "python" => &["python"],
        "systemctl" => &["systemd", "service"],
        "journalctl" => &["systemd", "logs"],
        "psql" | "mysql" | "sqlite3" => &["database", "sql"],
        "redis-cli" => &["database", "redis"],
        "jq" | "yq" => &["json", "parsing"],
        "openssl" => &["crypto", "tls"],
        "gpg" => &["gpg", "crypto"],
        "sops" => &["secrets", "sops"],
        "age" => &["crypto", "secrets"],
        "base64" => &["encoding"],
        "tar" | "zip" | "unzip" => &["archive"],
        "sed" | "awk" => &["text"],
        "chmod" | "chown" => &["permissions"],
        _ => &[],
    }
}

/// The tags an imported entry carries: program, synonyms, intents, `imported`
/// and the source tag — deduplicated, lowercase and capped at [`MAX_TAGS`].
///
/// `imported` and the source tag are reserved before the cap is applied, so a
/// long synonym list never costs an entry its provenance.
pub fn tags_for(command: &str, kind: CommandKind, source: Origin) -> Vec<String> {
    let program = match kind {
        CommandKind::Python => Some("python".to_string()),
        CommandKind::Shell => program(command),
    };

    let mut tags: Vec<String> = Vec::with_capacity(MAX_TAGS);
    if let Some(program) = &program {
        push_unique(&mut tags, program.clone());
        for synonym in synonyms(program) {
            push_unique(&mut tags, (*synonym).to_string());
        }
    }
    for intent in intents(command) {
        push_unique(&mut tags, intent.to_string());
    }
    // Keep room for the two fixed tags before truncating derived ones.
    tags.truncate(MAX_TAGS - 2);
    push_unique(&mut tags, "imported".to_string());
    push_unique(&mut tags, source.tag().to_string());
    tags.truncate(MAX_TAGS);
    tags
}

/// Append `tag` unless it is already present.
fn push_unique(tags: &mut Vec<String>, tag: String) {
    if !tag.is_empty() && !tags.contains(&tag) {
        tags.push(tag);
    }
}

/// The intent tags of a command, most specific first.
fn intents(command: &str) -> Vec<&'static str> {
    static MATCHERS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    MATCHERS
        .get_or_init(|| {
            INTENTS
                .iter()
                .map(|(name, pattern)| {
                    let regex = Regex::new(&format!("(?i:{pattern})"))
                        .expect("intent patterns are valid regexes");
                    (regex, *name)
                })
                .collect()
        })
        .iter()
        .filter(|(regex, _)| regex.is_match(command))
        .map(|(_, name)| *name)
        .take(MAX_INTENTS)
        .collect()
}

/// The program a shell command runs, as a tag-safe name.
///
/// Leading wrappers (`sudo`, `time`, …), flags and hoard placeholders
/// (`#param!`) are skipped; the path is reduced to its basename and a trailing
/// `.exe` / `.py` is dropped.
fn program(command: &str) -> Option<String> {
    // `&&` is one separator; `|`, `;` and `(` split the surrounding pipeline.
    let normalized = command.replace("&&", " ");
    for token in normalized.split(|c: char| c.is_whitespace() || matches!(c, '|' | ';' | '(')) {
        if token.is_empty() || token.starts_with('-') || token.starts_with('#') {
            continue;
        }
        if WRAPPERS.contains(&token.to_ascii_lowercase().as_str()) {
            continue;
        }
        let base = token
            .rsplit('/')
            .next()
            .unwrap_or(token)
            .to_ascii_lowercase();
        let base = base
            .strip_suffix(".exe")
            .or_else(|| base.strip_suffix(".py"))
            .unwrap_or(&base);
        let tag = sanitize(base);
        if !tag.is_empty() {
            return Some(tag);
        }
    }
    None
}

/// Lowercase ASCII, with every run of other characters collapsed to one `-`.
fn sanitize(raw: &str) -> String {
    let mut tag = String::with_capacity(raw.len());
    for ch in raw.chars() {
        let ch = ch.to_ascii_lowercase();
        if ch.is_ascii_alphanumeric() {
            tag.push(ch);
        } else if !tag.is_empty() && !tag.ends_with('-') {
            tag.push('-');
        }
    }
    while tag.ends_with('-') {
        tag.pop();
    }
    tag
}

#[cfg(test)]
mod test_tags {
    use super::*;

    #[test]
    fn program_skips_wrappers_flags_and_placeholders() {
        assert_eq!(
            tags_for("sudo kubectl get pods -A", CommandKind::Shell, Origin::Zsh),
            vec![
                "kubectl",
                "kubernetes",
                "k8s",
                "permissions",
                "imported",
                "zsh"
            ]
        );
        // A hoard placeholder in front of the program is not the program.
        assert_eq!(program("time #target! uptime"), Some("uptime".into()));
        assert_eq!(program("sudo env /usr/bin/rg --files"), Some("rg".into()));
        // Basename, lowercase and a stripped script suffix.
        assert_eq!(program("./bin/Report.PY --now"), Some("report".into()));
        assert_eq!(
            program("docker compose up -d | tee out"),
            Some("docker".into())
        );
        assert_eq!(program("#git!"), None);
        assert_eq!(program("sudo -i"), None);
    }

    #[test]
    fn synonyms_come_from_the_static_map() {
        assert_eq!(program("docker compose up -d"), Some("docker".into()));
        assert_eq!(
            tags_for("docker compose up -d", CommandKind::Shell, Origin::Pi),
            vec!["docker", "containers", "imported", "pi"]
        );
        assert!(synonyms("kubectl").contains(&"k8s"));
        assert!(synonyms("definitely-not-mapped").is_empty());
    }

    #[test]
    fn tool_family_intent_survives_a_wrapper_program() {
        // `rtk` is the program hoard can see, but the command is about
        // Kubernetes and must answer a search for it.
        assert_eq!(
            tags_for(
                "rtk kubectl logs -n default pod/x",
                CommandKind::Shell,
                Origin::Pi
            ),
            vec!["rtk", "kubernetes", "logs", "imported", "pi"]
        );
        assert!(
            tags_for("aws s3api list-buckets", CommandKind::Shell, Origin::Zsh)
                .contains(&"aws".to_string())
        );
    }

    #[test]
    fn intents_are_capped_at_three_most_specific() {
        // storage, build, files and permissions all match; only the three most
        // specific (in the configured order) survive.
        assert_eq!(
            tags_for(
                "sudo rm -rf /s3/bucket && make build",
                CommandKind::Shell,
                Origin::Zsh,
            ),
            vec!["rm", "storage", "build", "files", "imported", "zsh"]
        );
        // `logs` would rank fourth here, so it never appears.
        assert_eq!(
            tags_for(
                "curl -H 'X-Token: 1' https://s3.example.com | tee build.log",
                CommandKind::Shell,
                Origin::Zsh,
            ),
            vec![
                "curl", "http", "network", "secrets", "storage", "imported", "zsh"
            ]
        );
        assert_eq!(intents("sudo chmod 600 /etc/shadow"), vec!["permissions"]);
        assert!(intents("uptime").is_empty());
    }

    #[test]
    fn python_kind_is_tagged_python() {
        assert_eq!(
            tags_for("import os\nprint(os.name)", CommandKind::Python, Origin::Pi),
            vec!["python", "imported", "pi"]
        );
    }

    #[test]
    fn tags_are_deduped_and_capped() {
        // `network` is both a curl synonym and an intent match.
        assert_eq!(
            tags_for("curl https://example.com", CommandKind::Shell, Origin::Zsh),
            vec!["curl", "http", "network", "imported", "zsh"]
        );
        // The longest possible derivation stays within the cap and keeps provenance.
        let tags = tags_for(
            "scp -r deploy.tar secret-user@host:/s3/bucket",
            CommandKind::Shell,
            Origin::Pi,
        );
        assert!(tags.len() <= MAX_TAGS, "{tags:?}");
        assert_eq!(&tags[..3], ["scp", "ssh", "network"]);
        assert!(tags.contains(&"imported".to_string()), "{tags:?}");
        assert_eq!(tags.last().map(String::as_str), Some("pi"));
        for tag in &tags {
            assert!(
                !tag.is_empty()
                    && tag
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{tag:?} is not tag-safe"
            );
        }
    }
}
