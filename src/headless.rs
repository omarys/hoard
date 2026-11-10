//! Headless output discipline: deterministic payloads, exit codes and errors.
//!
//! Everything an agent-facing invocation prints goes through this module, so a
//! failed run always looks the same and always says *why* in machine-readable
//! form. Human mode keeps the interactive/TUI behaviour untouched.

use crate::core::template::RenderError;
use serde_json::Value;
use std::io::IsTerminal;
use std::io::Write;

/// Exit code for a successful run.
pub const EXIT_OK: i32 = 0;
/// Exit code for an unparsable invocation (bad flags, malformed `--param`).
pub const EXIT_PARSE: i32 = 1;
/// Exit code when the requested entry does not exist.
pub const EXIT_NOT_FOUND: i32 = 2;
/// Exit code when parameters do not fit the command template.
pub const EXIT_INVALID_PARAM: i32 = 3;
/// Exit code for every other failure (I/O, database, validation).
pub const EXIT_RUNTIME: i32 = 4;

/// How output is shaped for this invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputMode {
    /// Interactive/terminal output.
    #[default]
    Human,
    /// Full, pretty-printed JSON.
    Json,
    /// Minified, token-dense JSON.
    JsonCompact,
}

impl OutputMode {
    /// Resolve the mode from the flags and whether stdout is a terminal.
    ///
    /// `--compact` implies JSON: asking for the dense projection is a request
    /// for machine output even when the flag pair is incomplete.
    pub fn resolve(json: bool, compact: bool) -> Self {
        match (json || compact, compact) {
            (false, _) => Self::Human,
            (true, true) => Self::JsonCompact,
            (true, false) => Self::Json,
        }
    }

    /// Whether any JSON is produced (compact included).
    pub fn is_json(self) -> bool {
        !matches!(self, Self::Human)
    }

    /// Whether the dense projection is requested.
    pub fn is_compact(self) -> bool {
        matches!(self, Self::JsonCompact)
    }

    /// Serialize `value` the way this mode expects.
    pub fn serialize(self, value: &Value) -> String {
        if self.is_compact() {
            serde_json::to_string(value).expect("serializing JSON cannot fail")
        } else {
            serde_json::to_string_pretty(value).expect("serializing JSON cannot fail")
        }
    }
}

/// The error classes of the headless contract, each with a fixed exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorType {
    /// The named entry (or the query) matched nothing.
    CommandNotFound,
    /// Parameters were missing, unknown or malformed.
    InvalidParameter,
    /// The invocation itself could not be parsed.
    ParseError,
    /// Anything else: I/O, database, validation failures.
    RuntimeError,
}

impl ErrorType {
    /// The `error_type` string of the JSON payload.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CommandNotFound => "CommandNotFound",
            Self::InvalidParameter => "InvalidParameter",
            Self::ParseError => "ParseError",
            Self::RuntimeError => "RuntimeError",
        }
    }

    /// The exit code this error class always maps to.
    pub fn exit_code(self) -> i32 {
        match self {
            Self::ParseError => EXIT_PARSE,
            Self::CommandNotFound => EXIT_NOT_FOUND,
            Self::InvalidParameter => EXIT_INVALID_PARAM,
            Self::RuntimeError => EXIT_RUNTIME,
        }
    }
}

/// A failure carrying everything the headless payload needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    /// Class of the failure; decides the exit code.
    pub error_type: ErrorType,
    /// Human-readable explanation.
    pub message: String,
    /// Names the caller probably meant, when a lookup failed.
    pub suggested_keys: Vec<String>,
    /// Required parameters that were not supplied.
    pub missing_params: Vec<String>,
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

/// Being a real error type lets any internal helper return a `CliError` with `?`
/// and keep its `error_type` (a plain `anyhow` error would flatten to
/// `RuntimeError`).
impl std::error::Error for CliError {}

impl CliError {
    /// Build an error of `error_type` with no suggestions.
    pub fn new(error_type: ErrorType, message: impl Into<String>) -> Self {
        Self {
            error_type,
            message: message.into(),
            suggested_keys: Vec::new(),
            missing_params: Vec::new(),
        }
    }

    /// Attach guessable alternatives.
    pub fn with_suggestions(mut self, suggested_keys: Vec<String>) -> Self {
        self.suggested_keys = suggested_keys;
        self
    }

    /// An entry the caller asked for does not exist.
    pub fn not_found(message: impl Into<String>, suggested_keys: Vec<String>) -> Self {
        Self::new(ErrorType::CommandNotFound, message).with_suggestions(suggested_keys)
    }

    /// A parameter that does not fit the template.
    pub fn invalid_parameter(mut self, missing: Vec<String>) -> Self {
        self.missing_params = missing;
        self
    }

    /// The exit code for this failure.
    pub fn exit_code(&self) -> i32 {
        self.error_type.exit_code()
    }

    /// The JSON payload for this failure.
    ///
    /// `ok` and `status` are both present: `status` keeps the shape agents are
    /// usually shown (`{"status": "error", ...}`), `ok` reads naturally in Rust
    /// and in shell pipelines.
    pub fn to_json(&self) -> Value {
        let mut payload = serde_json::json!({
            "ok": false,
            "status": "error",
            "error_type": self.error_type.as_str(),
            "message": self.message,
        });
        let object = payload.as_object_mut().expect("payload is an object");
        if !self.missing_params.is_empty() {
            object.insert(
                "missing_params".into(),
                serde_json::json!(self.missing_params),
            );
        }
        if !self.suggested_keys.is_empty() {
            object.insert(
                "suggested_keys".into(),
                serde_json::json!(self.suggested_keys),
            );
        }
        payload
    }

    /// Print the failure and return the process exit code.
    ///
    /// In JSON modes the payload goes to stdout so a single stream carries the
    /// whole answer; in human mode the message goes to stderr as before.
    pub fn emit(&self, mode: OutputMode) -> i32 {
        let mut stdout = std::io::stdout().lock();
        if mode.is_json() {
            // Ignore a closed pipe; the exit code still reports the failure.
            let _ = writeln!(stdout, "{}", mode.serialize(&self.to_json()));
        } else {
            let _ = writeln!(std::io::stderr().lock(), "{}", sanitize(&self.message));
        }
        let _ = stdout.flush();
        self.exit_code()
    }
}

impl From<anyhow::Error> for CliError {
    fn from(error: anyhow::Error) -> Self {
        // A `CliError` travelling through an `anyhow` boundary keeps its class
        // (and therefore its exit code) instead of flattening to `RuntimeError`.
        match error.downcast::<CliError>() {
            Ok(cli) => cli,
            Err(other) => Self::new(ErrorType::RuntimeError, format!("{other:#}")),
        }
    }
}

impl From<crate::core::error::HoardError> for CliError {
    fn from(error: crate::core::error::HoardError) -> Self {
        Self::new(ErrorType::RuntimeError, error.to_string())
    }
}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        Self::new(ErrorType::RuntimeError, error.to_string())
    }
}

impl From<RenderError> for CliError {
    fn from(error: RenderError) -> Self {
        // `missing_params` names exactly what the caller still owes the
        // template; the suggestions repeat them so an agent can patch its call
        // without a second round trip.
        let message = error.message();
        let suggestions = if error.missing.is_empty() {
            error.unknown.clone()
        } else {
            error.missing.clone()
        };
        Self::new(ErrorType::InvalidParameter, message)
            .invalid_parameter(error.missing)
            .with_suggestions(suggestions)
    }
}

/// Whether stdout is a terminal.
pub fn stdout_is_tty() -> bool {
    std::io::stdout().is_terminal()
}

/// Strip escape sequences only when the text cannot be styled.
///
/// A terminal gets the bytes untouched (cheap, and the only place where color
/// is meaningful); a pipe or a file gets plain text, because a downstream
/// parser that meets `\x1b[32m` has to throw the line away.
pub fn sanitize(text: &str) -> std::borrow::Cow<'_, str> {
    if stdout_is_tty() || !text.contains('\u{1b}') {
        std::borrow::Cow::Borrowed(text)
    } else {
        std::borrow::Cow::Owned(strip_ansi(text))
    }
}

/// Whether there is a person at the keyboard to answer a prompt.
pub fn stdin_is_tty() -> bool {
    std::io::stdin().is_terminal()
}

/// Remove ANSI escape sequences (colors, cursor movement, OSC titles).
///
/// Rendering is disabled when stdout is not a terminal, but a stray sequence
/// from a dependency or an imported command would still corrupt the stream an
/// agent parses, so the final write is sanitized.
pub fn strip_ansi(input: &str) -> String {
    if !input.contains('\u{1b}') {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut chars = input.char_indices().peekable();
    while let Some((_, ch)) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        match chars.peek().map(|(_, next)| *next) {
            // CSI: ESC [ ... final byte in @..~
            Some('[') => {
                chars.next();
                for (_, c) in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: ESC ] ... BEL or ESC \
            Some(']') => {
                chars.next();
                while let Some((_, c)) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' && chars.peek().map(|(_, n)| *n) == Some('\\') {
                        chars.next();
                        break;
                    }
                }
            }
            // Any other two-character escape.
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

/// Rank `candidates` by how close they are to `query`, best first.
///
/// Candidates are entry keys (`namespace/name`), and callers usually ask by the
/// bare name, so the leaf after the last `/` is scored too: `podz` suggests
/// `kube/pods`. A substring hit always outranks an edit-distance hit. The typo
/// budget grows slowly (`len/3`, at least 2) so a long unrelated query does not
/// drag in half the trove. Returns at most 3 names.
pub fn suggest(query: &str, candidates: impl IntoIterator<Item = String>) -> Vec<String> {
    let needle = query.to_lowercase();
    let budget = (needle.len() / 3).max(2);
    let mut ranked: Vec<(u8, usize, String)> = Vec::new();
    for candidate in candidates {
        let haystack = candidate.to_lowercase();
        if haystack == needle {
            continue;
        }
        let leaf = haystack.rsplit('/').next().unwrap_or(haystack.as_str());
        let containing = haystack.contains(&needle) || needle.contains(&haystack);
        let distance = edit_distance(&needle, &haystack).min(edit_distance(&needle, leaf));
        if containing {
            ranked.push((0, distance, candidate));
        } else if distance <= budget {
            ranked.push((1, distance, candidate));
        }
    }
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    ranked.truncate(3);
    ranked.into_iter().map(|(_, _, name)| name).collect()
}

/// Levenshtein distance over characters.
fn edit_distance(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    if left.is_empty() {
        return right.len();
    }
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];
    for (i, l) in left.iter().enumerate() {
        current[0] = i + 1;
        for (j, r) in right.iter().enumerate() {
            let substitution = previous[j] + usize::from(l != r);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

#[cfg(test)]
mod test_headless {
    use super::*;

    #[test]
    fn modes_resolve_from_flags() {
        assert_eq!(OutputMode::resolve(false, false), OutputMode::Human);
        assert_eq!(OutputMode::resolve(true, false), OutputMode::Json);
        assert_eq!(OutputMode::resolve(true, true), OutputMode::JsonCompact);
        // `--compact` alone still yields machine output.
        assert_eq!(OutputMode::resolve(false, true), OutputMode::JsonCompact);
    }

    #[test]
    fn every_error_class_has_a_distinct_exit_code() {
        let codes = [
            ErrorType::ParseError.exit_code(),
            ErrorType::CommandNotFound.exit_code(),
            ErrorType::InvalidParameter.exit_code(),
            ErrorType::RuntimeError.exit_code(),
        ];
        assert_eq!(codes, [1, 2, 3, 4]);
        assert_eq!(ErrorType::ParseError.exit_code(), EXIT_PARSE);
        assert_eq!(EXIT_OK, 0);
    }

    #[test]
    fn error_payload_has_the_documented_keys() {
        let error = CliError::new(ErrorType::CommandNotFound, "no entry named pods")
            .with_suggestions(vec!["kube/pods".into()]);
        let json = error.to_json();
        assert_eq!(json["ok"], false);
        assert_eq!(json["status"], "error");
        assert_eq!(json["error_type"], "CommandNotFound");
        assert_eq!(json["message"], "no entry named pods");
        assert_eq!(json["suggested_keys"][0], "kube/pods");
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn missing_parameters_are_reported_as_keys() {
        let error = CliError::from(RenderError {
            missing: vec!["container_id".into()],
            unknown: Vec::new(),
        });
        assert_eq!(error.error_type, ErrorType::InvalidParameter);
        let json = error.to_json();
        assert_eq!(json["status"], "error");
        assert_eq!(json["missing_params"][0], "container_id");
        assert_eq!(json["suggested_keys"][0], "container_id");
        assert_eq!(error.exit_code(), EXIT_INVALID_PARAM);
    }

    #[test]
    fn empty_optional_fields_are_omitted() {
        let json = CliError::new(ErrorType::RuntimeError, "boom").to_json();
        assert!(json.get("missing_params").is_none());
        assert!(json.get("suggested_keys").is_none());
    }

    #[test]
    fn compact_serialization_is_single_line() {
        let value = serde_json::json!({"ok": true, "k": "pods"});
        assert_eq!(
            OutputMode::JsonCompact.serialize(&value),
            "{\"k\":\"pods\",\"ok\":true}"
        );
        assert!(OutputMode::Json.serialize(&value).contains('\n'));
    }

    #[test]
    fn ansi_sequences_are_stripped() {
        assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m"), "red");
        assert_eq!(strip_ansi("\u{1b}[2K\u{1b}[1Gclean"), "clean");
        assert_eq!(strip_ansi("\u{1b}]0;title\u{7}body"), "body");
        assert_eq!(strip_ansi("plain text"), "plain text");
        assert_eq!(strip_ansi("trailing \u{1b}"), "trailing ");
    }

    #[test]
    fn suggestions_prefer_substrings_then_typos() {
        let candidates = vec![
            "kube/pods".to_string(),
            "podman-images".to_string(),
            "git/status".to_string(),
        ];
        assert_eq!(suggest("pods", candidates.clone()), vec!["kube/pods"]);
        assert_eq!(suggest("statuz", candidates.clone()), vec!["git/status"]);
        assert!(suggest("kubernetes", candidates.clone()).is_empty());
    }

    #[test]
    fn edit_distance_is_symmetric_and_exact() {
        assert_eq!(edit_distance("pods", "pods"), 0);
        assert_eq!(edit_distance("pods", "pod"), 1);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
    }
}
