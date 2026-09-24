//! Detection and redaction of secrets in shell commands before they are stored.
//!
//! Hoard must never archive credentials. Every save path runs the command
//! string through [`redact_secrets`], which replaces detected secret *values*
//! with parameter placeholders (`#password!`) so the command stays reusable
//! while the secret stays out of the trove and out of every export or backup.
//!
//! Only values are replaced: flags, variable names, usernames and structure are
//! preserved. Already parameterized values, shell expansions (`$TOKEN`,
//! `$(pass show x)`) and non-secret literals are left untouched, so the
//! transformation is idempotent.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use regex::{Captures, Regex};

use crate::core::{CommandKind, HoardCmd};

/// A secret value that was replaced by a parameter placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redaction {
    /// The placeholder written into the command, e.g. `#password!`.
    pub placeholder: String,
    /// What matched, e.g. `password flag`. Never contains the secret itself.
    pub label: &'static str,
}

/// A command string together with everything that was redacted from it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Redacted {
    /// The command with secret values replaced by placeholders.
    pub command: String,
    /// One entry per distinct placeholder.
    pub redactions: Vec<Redaction>,
}

impl Redacted {
    /// `true` when nothing had to be redacted.
    pub fn is_clean(&self) -> bool {
        self.redactions.is_empty()
    }

    /// Human readable list of the applied placeholders, e.g.
    /// `#password! (password flag)`. Never contains the secret itself.
    pub fn summary(&self) -> String {
        self.redactions
            .iter()
            .map(|r| format!("{} ({})", r.placeholder, r.label))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// How a rule locates the secret inside its match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// The whole match is the secret (known key formats, private key blocks).
    Whole,
    /// `scheme://user:secret@host`, secret in the `secret` group.
    UrlPassword,
    /// `-u user:secret`, the `value` group holds `user:secret`.
    UserFlag,
    /// `--password value`, secret in the `value` group.
    FlagValue,
    /// `key=value`, `key: value` or `?key=value`, base name from the `kw` group.
    Assignment,
    /// `Authorization: Bearer <secret>`, secret in the `value` group.
    AuthHeader,
}

/// One redaction rule: what it matches and which placeholder name it uses.
struct Rule {
    /// Placeholder base name, e.g. `password`.
    base: &'static str,
    /// Label shown in the save notice.
    label: &'static str,
    shape: Shape,
    regex: Regex,
}

/// Errors here are programming mistakes, not user input, so the rules are
/// compiled once and any panic would be caught by the tests.
fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        let rule = |base, label, shape, pattern: &str| Rule {
            base,
            label,
            shape,
            regex: Regex::new(pattern).expect("redaction pattern is always valid"),
        };
        vec![
            rule(
                "private-key",
                "private key block",
                Shape::Whole,
                r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
            ),
            rule(
                "password",
                "URL password",
                Shape::UrlPassword,
                r"(?P<scheme>[a-zA-Z][a-zA-Z0-9+.\-]*://)(?P<user>[^/\s:@]{1,64}):(?P<secret>[^/\s@]{1,512})@",
            ),
            rule(
                "password",
                "user flag password",
                Shape::UserFlag,
                r#"(?P<flag>(?:-u|--user))(?P<sep>=|\s+)(?P<value>"[^"]*"|'[^']*'|[^\s"']+)"#,
            ),
            rule(
                "password",
                "password flag",
                Shape::FlagValue,
                concat!(
                    r#"(?P<flag>--(?:password|passwd))(?P<sep>=|\s+)"#,
                    r#"(?P<value>"[^"]*"|'[^']*'|[^\s"']+)"#
                ),
            ),
            rule(
                "token",
                "token flag",
                Shape::FlagValue,
                concat!(
                    r#"(?P<flag>--(?:token|auth-token))(?P<sep>=|\s+)"#,
                    r#"(?P<value>"[^"]*"|'[^']*'|[^\s"']+)"#
                ),
            ),
            rule(
                "api-key",
                "api key flag",
                Shape::FlagValue,
                concat!(
                    r#"(?P<flag>--(?:api[_-]?key|apikey))(?P<sep>=|\s+)"#,
                    r#"(?P<value>"[^"]*"|'[^']*'|[^\s"']+)"#
                ),
            ),
            rule(
                "access-key",
                "access key flag",
                Shape::FlagValue,
                concat!(
                    r#"(?P<flag>--access[_-]?key)(?P<sep>=|\s+)"#,
                    r#"(?P<value>"[^"]*"|'[^']*'|[^\s"']+)"#
                ),
            ),
            rule(
                "secret",
                "secret flag",
                Shape::FlagValue,
                concat!(
                    r#"(?P<flag>--(?:client[_-]?secret|secret))(?P<sep>=|\s+)"#,
                    r#"(?P<value>"[^"]*"|'[^']*'|[^\s"']+)"#
                ),
            ),
            rule(
                "token",
                "authorization header",
                Shape::AuthHeader,
                r#"(?i)(?P<flag>authorization\s*:\s*(?:bearer|basic|token)\s+)(?P<value>"[^"]*"|'[^']*'|[^\s"']+)"#,
            ),
            rule(
                "secret",
                "secret assignment",
                Shape::Assignment,
                concat!(
                    r"(?i)(?P<key>[A-Za-z0-9_.\-]*)(?P<kw>password|passwd|secret|token|",
                    r"api[_-]?key|access[_-]?key|client[_-]?secret)(?P<sep>\s*[:=]\s*)",
                    r#"(?P<value>"[^"]*"|'[^']*'|[^\s"'&]+)"#
                ),
            ),
            rule(
                "access-key-id",
                "AWS access key id",
                Shape::Whole,
                r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b",
            ),
            rule(
                "github-token",
                "GitHub token",
                Shape::Whole,
                r"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})\b",
            ),
            rule(
                "slack-token",
                "Slack token",
                Shape::Whole,
                r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b",
            ),
            rule(
                "api-key",
                "API key",
                Shape::Whole,
                r"\bsk-[A-Za-z0-9_-]{20,}\b",
            ),
            rule(
                "jwt",
                "JWT",
                Shape::Whole,
                r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{4,}\b",
            ),
        ]
    })
}

/// Replace secrets found in `command` with parameter placeholders.
pub fn redact_secrets(command: &str, token: &str, ending_token: &str) -> Redacted {
    let mut redactor = Redactor::new(command, token, ending_token);
    let mut current = command.to_string();
    for rule in rules() {
        current = rule
            .regex
            .replace_all(&current, |caps: &Captures| apply(rule, caps, &mut redactor))
            .into_owned();
    }
    let command = redactor.sweep_remaining(&current);
    Redacted {
        command,
        redactions: redactor.redactions,
    }
}

/// `true` when a matched value is worth replacing: long enough to be a
/// credential and not a reference, a number or an existing placeholder.
fn should_redact(value: &str, token: &str, ending_token: &str) -> bool {
    let value = value.trim();
    if value.len() < 4 {
        return false;
    }
    if !value.chars().any(char::is_alphanumeric) || value.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    if value.starts_with('$') || value.starts_with('`') || value.starts_with("!{") {
        return false;
    }
    // Already a parameter placeholder: keeps redaction idempotent.
    if !token.is_empty()
        && !ending_token.is_empty()
        && let Some(name) = value
            .strip_prefix(token)
            .and_then(|rest| rest.strip_suffix(ending_token))
    {
        return name.is_empty();
    }
    true
}

/// Split a possibly quoted value into (opening quote, inner value, closing quote).
fn quoted(value: &str) -> (&str, &str, &str) {
    for quote in ['"', '\''] {
        if value.len() >= 2 && value.starts_with(quote) && value.ends_with(quote) {
            let quote = if quote == '"' { "\"" } else { "'" };
            return (quote, &value[1..value.len() - 1], quote);
        }
    }
    ("", value, "")
}

fn apply(rule: &Rule, caps: &Captures, redactor: &mut Redactor) -> String {
    let original = caps.get(0).map_or("", |m| m.as_str());
    let (token, ending) = (redactor.token, redactor.ending);
    match rule.shape {
        Shape::Whole => {
            if !should_redact(original, token, ending) {
                return original.to_string();
            }
            redactor.placeholder(rule.base, rule.label, original)
        }
        Shape::UrlPassword => {
            let secret = caps.name("secret").map_or("", |m| m.as_str());
            if !should_redact(secret, token, ending) {
                return original.to_string();
            }
            let placeholder = redactor.placeholder(rule.base, rule.label, secret);
            format!(
                "{}{}:{}@",
                caps.name("scheme").map_or("", |m| m.as_str()),
                caps.name("user").map_or("", |m| m.as_str()),
                placeholder
            )
        }
        Shape::UserFlag => {
            let value = caps.name("value").map_or("", |m| m.as_str());
            let (open, inner, close) = quoted(value);
            // `user:secret` only; URLs are handled by the URL rule.
            let Some((user, secret)) = inner.split_once(':') else {
                return original.to_string();
            };
            if inner.contains("://") || !should_redact(secret, token, ending) {
                return original.to_string();
            }
            let placeholder = redactor.placeholder(rule.base, rule.label, secret);
            format!(
                "{}{}{}{}:{}{}",
                caps.name("flag").map_or("", |m| m.as_str()),
                caps.name("sep").map_or("", |m| m.as_str()),
                open,
                user,
                placeholder,
                close
            )
        }
        Shape::FlagValue | Shape::AuthHeader => {
            let value = caps.name("value").map_or("", |m| m.as_str());
            let (open, inner, close) = quoted(value);
            if !should_redact(inner, token, ending) {
                return original.to_string();
            }
            let placeholder = redactor.placeholder(rule.base, rule.label, inner);
            format!(
                "{}{}{}{}{}",
                caps.name("flag").map_or("", |m| m.as_str()),
                caps.name("sep").map_or("", |m| m.as_str()),
                open,
                placeholder,
                close
            )
        }
        Shape::Assignment => {
            let value = caps.name("value").map_or("", |m| m.as_str());
            let (open, inner, close) = quoted(value);
            if !should_redact(inner, token, ending) {
                return original.to_string();
            }
            let base = caps.name("kw").map_or(rule.base.to_string(), |m| {
                m.as_str().to_ascii_lowercase().replace(['_', '.'], "-")
            });
            let placeholder = redactor.placeholder(&base, rule.label, inner);
            format!(
                "{}{}{}{}{}{}",
                caps.name("key").map_or("", |m| m.as_str()),
                caps.name("kw").map_or("", |m| m.as_str()),
                caps.name("sep").map_or("", |m| m.as_str()),
                open,
                placeholder,
                close
            )
        }
    }
}

/// Replace `needle` outside every `token..ending` placeholder, so an
/// already-inserted placeholder can never be rewritten again.
fn replace_outside(
    command: &str,
    needle: &str,
    replacement: &str,
    placeholder_re: Option<&Regex>,
) -> String {
    let Some(re) = placeholder_re else {
        return command.replace(needle, replacement);
    };
    let mut out = String::with_capacity(command.len());
    let mut last = 0;
    for found in re.find_iter(command) {
        out.push_str(&command[last..found.start()].replace(needle, replacement));
        out.push_str(found.as_str());
        last = found.end();
    }
    out.push_str(&command[last..].replace(needle, replacement));
    out
}

/// Allocates unique placeholder names and records what was redacted.
struct Redactor<'a> {
    token: &'a str,
    ending: &'a str,
    /// Names already used as parameters in the original command.
    taken: HashSet<String>,
    /// (base, secret) -> placeholder, so repeated secrets share one parameter.
    assigned: HashMap<(String, String), String>,
    /// Matches an existing parameter placeholder.
    placeholder_re: Option<Regex>,
    redactions: Vec<Redaction>,
}

impl<'a> Redactor<'a> {
    fn new(command: &str, token: &'a str, ending: &'a str) -> Self {
        let mut taken = HashSet::new();
        let mut placeholder_re = None;
        if !token.is_empty() && !ending.is_empty() {
            let pattern = format!("{}(.*?){}", regex::escape(token), regex::escape(ending));
            if let Ok(re) = Regex::new(&pattern) {
                for caps in re.captures_iter(command) {
                    if let Some(name) = caps.get(1) {
                        taken.insert(name.as_str().to_string());
                    }
                }
                placeholder_re = Some(re);
            }
        }
        Self {
            token,
            ending,
            taken,
            assigned: HashMap::new(),
            placeholder_re,
            redactions: Vec::new(),
        }
    }

    /// Replace any remaining occurrence of an identified secret, e.g. the same
    /// credential repeated under a key no rule recognises. Longest secret
    /// first, so a secret that contains another one cannot corrupt it.
    fn sweep_remaining(&mut self, command: &str) -> String {
        if self.assigned.is_empty() {
            return command.to_string();
        }
        let mut secrets: Vec<(String, String)> = self
            .assigned
            .iter()
            .map(|((_, secret), placeholder)| (secret.clone(), placeholder.clone()))
            .collect();
        // Deterministic order; one placeholder per secret, longest secret first.
        secrets.sort();
        secrets.dedup_by(|a, b| a.0 == b.0);
        secrets.sort_by_key(|(secret, _)| std::cmp::Reverse(secret.len()));

        let mut current = command.to_string();
        for (secret, placeholder) in secrets {
            if current.contains(&secret) {
                current = replace_outside(
                    &current,
                    &secret,
                    &placeholder,
                    self.placeholder_re.as_ref(),
                );
            }
        }
        current
    }

    /// Placeholder for `secret`, reusing one name when the same value repeats.
    fn placeholder(&mut self, base: &str, label: &'static str, secret: &str) -> String {
        let base = if base.is_empty() { "secret" } else { base };
        let key = (base.to_string(), secret.to_string());
        if let Some(existing) = self.assigned.get(&key) {
            return existing.clone();
        }
        let mut name = base.to_string();
        let mut suffix = 2;
        while self.taken.contains(&name) {
            name = format!("{base}{suffix}");
            suffix += 1;
        }
        self.taken.insert(name.clone());
        let placeholder = format!("{}{name}{}", self.token, self.ending);
        self.assigned.insert(key, placeholder.clone());
        self.redactions.push(Redaction {
            placeholder: placeholder.clone(),
            label,
        });
        placeholder
    }
}

impl HoardCmd {
    /// Replace secrets in the stored command with parameter placeholders.
    ///
    /// Python sources are returned untouched: they are source code, not shell
    /// invocations, so rewriting them would corrupt the archived file.
    pub fn with_redacted_secrets(self, token: &str, ending_token: &str) -> (Self, Redacted) {
        if self.kind == CommandKind::Python {
            return (self, Redacted::default());
        }
        let redacted = redact_secrets(&self.command, token, ending_token);
        let command = redacted.command.clone();
        (Self { command, ..self }, redacted)
    }
}

#[cfg(test)]
mod test_secrets {
    use super::*;

    fn redact(command: &str) -> Redacted {
        redact_secrets(command, "#", "!")
    }

    #[test]
    fn redacts_url_password() {
        let r = redact("curl https://admin:s3cretpw@example.internal/api");
        assert_eq!(
            r.command,
            "curl https://admin:#password!@example.internal/api"
        );
        assert_eq!(r.redactions.len(), 1);
    }

    #[test]
    fn redacts_user_flag_credentials() {
        let r = redact(r#"kubectl exec pod -- curl -u "admin:Os!3jN9kRmW" https://localhost:9200"#);
        assert_eq!(
            r.command,
            r#"kubectl exec pod -- curl -u "admin:#password!" https://localhost:9200"#
        );
    }

    #[test]
    fn redacts_password_and_token_flags() {
        assert_eq!(
            redact("tool --password=hunter2xyz --token abcdef123456").command,
            "tool --password=#password! --token #token!"
        );
        assert_eq!(
            redact("tool --api-key AKIAIOSFODNN7EXAMPLE").command,
            "tool --api-key #api-key!"
        );
    }

    #[test]
    fn redacts_authorization_header() {
        assert_eq!(
            redact("curl -H 'Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.abcdefgh'")
                .command,
            "curl -H 'Authorization: Bearer #token!'"
        );
    }

    #[test]
    fn redacts_assignments_and_env() {
        assert_eq!(
            redact("docker run -e KC_BOOTSTRAP_ADMIN_PASSWORD=change_me image").command,
            "docker run -e KC_BOOTSTRAP_ADMIN_PASSWORD=#password! image"
        );
        assert_eq!(
            redact("curl 'https://host/x?token=abcdef123456&page=2'").command,
            "curl 'https://host/x?token=#token!&page=2'"
        );
    }

    #[test]
    fn redacts_known_key_formats() {
        assert_eq!(
            redact("aws configure set aws_access_key_id AKIAIOSFODNN7EXAMPLE").command,
            "aws configure set aws_access_key_id #access-key-id!"
        );
        assert_eq!(
            redact("git clone https://ghp_abcdefghijklmnopqrstuvwxyz012345@github.com/x/y").command,
            "git clone https://#github-token!@github.com/x/y"
        );
    }

    #[test]
    fn redacts_private_key_block() {
        let command =
            "printf '%s' '-----BEGIN RSA PRIVATE KEY-----\nMIIabc\n-----END RSA PRIVATE KEY-----'";
        assert_eq!(redact(command).command, "printf '%s' '#private-key!'");
    }

    #[test]
    fn keeps_references_expansions_and_short_values() {
        for command in [
            "curl -u admin:$PASS https://localhost:9200",
            "curl -H \"Authorization: Bearer $TOKEN\" https://host",
            "tool --password=$(pass show db)",
            "docker run -e PASSWORD=abc image",
            "tool --token=#token! --password=#password!",
            "ss -ltnp 'sport = :8443'",
        ] {
            let r = redact(command);
            assert_eq!(r.command, command, "must stay untouched");
            assert!(r.is_clean());
        }
    }

    #[test]
    fn is_idempotent_and_shares_repeated_secrets() {
        let once = redact("tool --password=hunter2xyz --other=hunter2xyz");
        let twice = redact(&once.command);
        assert_eq!(once.command, twice.command);
        assert!(twice.is_clean(), "second pass must not redact again");
        assert_eq!(
            once.command,
            "tool --password=#password! --other=#password!"
        );
    }

    #[test]
    fn disambiguates_distinct_secrets_and_existing_parameters() {
        let r = redact("tool --password=firstsecret --password=secondsecret #password!");
        assert_eq!(
            r.command,
            "tool --password=#password2! --password=#password3! #password!"
        );
        assert_eq!(r.redactions.len(), 2);
        assert!(r.summary().contains("#password2! (password flag)"));
    }

    #[test]
    fn never_touches_python_sources() {
        let script = HoardCmd {
            kind: CommandKind::Python,
            ..HoardCmd::default().with_command("token = 'AKIAIOSFODNN7EXAMPLE'")
        };
        let (result, redacted) = script.with_redacted_secrets("#", "!");
        assert_eq!(result.command, "token = 'AKIAIOSFODNN7EXAMPLE'");
        assert!(redacted.is_clean());
        assert!(redacted.summary().is_empty());
    }
}
