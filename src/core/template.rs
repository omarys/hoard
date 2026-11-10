//! `{{name}}` placeholders — the agent-facing parameter syntax.
//!
//! Commands can carry named placeholders instead of relying on the interactive
//! `#param!` markers:
//!
//! ```text
//! docker exec -it {{container_id}} sh -lc 'ls {{path}}'
//! git log --oneline {{ref=HEAD}}
//! ```
//!
//! A placeholder is `{{` + an identifier + optional `=default` + `}}`.
//! Anything else between double braces (shell brace expansion, literals) is
//! left untouched, so an existing trove never gets corrupted by this syntax.
//!
//! The parameters are *derived* from the stored template rather than stored
//! next to it: the template is the single source of truth, so a placeholder can
//! never drift out of sync with a parameter list.

use std::collections::BTreeMap;

/// One placeholder of a command template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamSpec {
    /// Placeholder name, e.g. `container_id`.
    pub name: String,
    /// Value used when no `--param` is supplied; makes the parameter optional.
    pub default: Option<String>,
}

impl ParamSpec {
    /// A parameter without a default must be supplied by the caller.
    pub fn required(&self) -> bool {
        self.default.is_none()
    }
}

/// A span of the template that a placeholder occupies.
struct Span {
    start: usize,
    end: usize,
    spec: ParamSpec,
}

/// Why a template could not be rendered.
///
/// Both lists are sorted and deduplicated so the payload an agent receives is
/// byte-for-byte reproducible.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderError {
    /// Required placeholders without a value.
    pub missing: Vec<String>,
    /// Supplied keys that the template does not use.
    pub unknown: Vec<String>,
}

impl RenderError {
    /// Whether the template rendered successfully.
    pub fn is_empty(&self) -> bool {
        self.missing.is_empty() && self.unknown.is_empty()
    }

    /// One-line explanation, used for the `message` field of the error payload.
    pub fn message(&self) -> String {
        match (self.missing.is_empty(), self.unknown.is_empty()) {
            (false, true) => format!("missing required parameter(s): {}", self.missing.join(", ")),
            (true, false) => format!("unknown parameter(s): {}", self.unknown.join(", ")),
            (false, false) => format!(
                "missing required parameter(s): {}; unknown parameter(s): {}",
                self.missing.join(", "),
                self.unknown.join(", ")
            ),
            (true, true) => String::new(),
        }
    }
}

/// Spans of the legacy `#name!` dialect, named after the text between the
/// tokens.
///
/// A shebang (`#!`) or a marker spanning a newline is not a parameter; the
/// human-facing markers must never turn a Python script into a prompt.
fn scan_legacy(template: &str, start: &str, end: &str) -> Vec<Span> {
    if start.is_empty() || end.is_empty() {
        return Vec::new();
    }
    let mut spans = Vec::new();
    let mut index = 0;
    while index < template.len() {
        let Some(open) = template[index..].find(start) else {
            break;
        };
        let open_at = index + open;
        let name_start = open_at + start.len();
        let Some(close) = template[name_start..].find(end) else {
            break;
        };
        let name_end = name_start + close;
        let name = template[name_start..name_end].trim();
        if !name.is_empty() && !name.contains('\n') {
            spans.push(Span {
                start: open_at,
                end: name_end + end.len(),
                spec: ParamSpec {
                    name: name.to_string(),
                    default: None,
                },
            });
        }
        index = name_end + end.len();
    }
    spans
}

/// Placeholder spans plus legacy spans, ordered, overlaps dropped (first wins).
fn merged_spans(template: &str, legacy: Option<(&str, &str)>) -> Vec<Span> {
    let mut spans = scan(template);
    if let Some((start, end)) = legacy {
        spans.extend(scan_legacy(template, start, end));
    }
    spans.sort_by_key(|span| span.start);
    let mut merged: Vec<Span> = Vec::with_capacity(spans.len());
    for span in spans {
        if merged.last().is_none_or(|last| span.start >= last.end) {
            merged.push(span);
        }
    }
    merged
}

/// Whether `name` is a usable placeholder identifier.
///
/// Deliberately narrow: identifiers only, so `{{1..3}}` or `{{ a b }}` stay
/// literal text instead of becoming a required parameter. A leading digit is
/// rejected for the same reason: shell brace expansions such as `{1..3}` must
/// survive untouched.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// Parse the inside of a `{{ }}` pair, or `None` when it is not a placeholder.
fn parse_spec(inner: &str) -> Option<ParamSpec> {
    match inner.split_once('=') {
        Some((name, default)) => {
            let name = name.trim();
            is_identifier(name).then(|| ParamSpec {
                name: name.to_string(),
                default: Some(default.trim().to_string()),
            })
        }
        None => {
            let name = inner.trim();
            is_identifier(name).then(|| ParamSpec {
                name: name.to_string(),
                default: None,
            })
        }
    }
}

/// Every placeholder span of `template`, in source order.
fn scan(template: &str) -> Vec<Span> {
    let bytes = template.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0;
    while index + 1 < bytes.len() {
        if &bytes[index..index + 2] != b"{{" {
            index += 1;
            continue;
        }
        let Some(close) = template[index + 2..].find("}}") else {
            break;
        };
        let inner_end = index + 2 + close;
        let Some(spec) = parse_spec(&template[index + 2..inner_end]) else {
            // Not a placeholder: keep scanning after this `{{`.
            index += 2;
            continue;
        };
        spans.push(Span {
            start: index,
            end: inner_end + 2,
            spec,
        });
        index = inner_end + 2;
    }
    spans
}

/// The placeholders of `template`, first occurrence first, duplicates merged.
///
/// When a name repeats, the first occurrence decides whether it has a default.
pub fn param_specs(template: &str) -> Vec<ParamSpec> {
    specs_of(&scan(template))
}

/// Every parameter of `template`: `{{name}}` placeholders plus the legacy
/// `#name!` markers between `start` and `end`.
///
/// The legacy dialect is the interactive one (`hoard pick` prompts for it);
/// `eval` renders both so a trove written before placeholders existed stays
/// usable headlessly.
pub fn param_specs_with(template: &str, start: &str, end: &str) -> Vec<ParamSpec> {
    specs_of(&merged_spans(template, Some((start, end))))
}

fn specs_of(spans: &[Span]) -> Vec<ParamSpec> {
    let mut specs: Vec<ParamSpec> = Vec::new();
    for span in spans {
        match specs.iter_mut().find(|spec| spec.name == span.spec.name) {
            // A default anywhere makes the parameter optional, whichever
            // occurrence happened to carry it.
            Some(existing) => {
                if existing.default.is_none() {
                    existing.default = span.spec.default.clone();
                }
            }
            None => specs.push(span.spec.clone()),
        }
    }
    specs
}

/// The placeholder names of `template`, in source order.
pub fn param_names(template: &str) -> Vec<String> {
    param_specs(template)
        .into_iter()
        .map(|spec| spec.name)
        .collect()
}

/// Parameter names of `template`, legacy `#name!` markers included, so a
/// headless caller sees exactly what `eval` will ask it for.
pub fn param_names_with(template: &str, start: &str, end: &str) -> Vec<String> {
    param_specs_with(template, start, end)
        .into_iter()
        .map(|spec| spec.name)
        .collect()
}

/// Whether the template has any placeholder at all.
#[cfg(test)]
pub fn is_parameterized(template: &str) -> bool {
    !param_specs(template).is_empty()
}

/// Substitute `values` into `template`.
///
/// Values are inserted verbatim: quoting and escaping stay the caller's job, so
/// a value can carry a path with spaces when the command quotes it.
#[cfg(test)]
pub fn render(template: &str, values: &BTreeMap<String, String>) -> Result<String, RenderError> {
    render_with(template, values, None)
}

/// Substitute `values` into `template`, accepting the legacy `#name!` markers
/// between `start` and `end` as parameters too.
pub fn render_with(
    template: &str,
    values: &BTreeMap<String, String>,
    legacy: Option<(&str, &str)>,
) -> Result<String, RenderError> {
    let spans = merged_spans(template, legacy);
    let specs = specs_of(&spans);

    let mut error = RenderError {
        missing: specs
            .iter()
            .filter(|spec| spec.required() && !values.contains_key(&spec.name))
            .map(|spec| spec.name.clone())
            .collect(),
        unknown: values
            .keys()
            .filter(|key| !specs.iter().any(|spec| &spec.name == *key))
            .cloned()
            .collect(),
    };
    error.missing.sort();
    error.missing.dedup();
    error.unknown.sort();
    error.unknown.dedup();
    if !error.is_empty() {
        return Err(error);
    }

    let mut rendered = String::with_capacity(template.len());
    let mut cursor = 0;
    // One value per parameter name: a repeated placeholder must render the same
    // text every time, whichever occurrence carried the default.
    let defaults: BTreeMap<&str, &str> = specs
        .iter()
        .filter_map(|spec| {
            spec.default
                .as_deref()
                .map(|value| (spec.name.as_str(), value))
        })
        .collect();
    for span in spans {
        rendered.push_str(&template[cursor..span.start]);
        match values.get(&span.spec.name) {
            Some(value) => rendered.push_str(value),
            None => rendered.push_str(
                defaults
                    .get(span.spec.name.as_str())
                    .copied()
                    .unwrap_or_default(),
            ),
        }
        cursor = span.end;
    }
    rendered.push_str(&template[cursor..]);
    Ok(rendered)
}

#[cfg(test)]
mod test_template {
    use super::*;

    fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn finds_placeholders_in_source_order() {
        let template = "docker exec -it {{container_id}} sh -lc 'ls {{path}}'";
        assert_eq!(param_names(template), vec!["container_id", "path"]);
    }

    #[test]
    fn repeated_placeholders_are_one_parameter() {
        let template = "cp {{src}} {{dst}} && ls {{src}}";
        let specs = param_specs(template);
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].name, "src");
    }

    #[test]
    fn defaults_make_a_parameter_optional() {
        let template = "git log --oneline {{ref=HEAD}} {{n=10}}";
        let specs = param_specs(template);
        assert!(!specs[0].required());
        assert_eq!(specs[1].default.as_deref(), Some("10"));
        assert_eq!(
            render(template, &values(&[])).unwrap(),
            "git log --oneline HEAD 10"
        );
    }

    #[test]
    fn supplied_values_win_over_defaults() {
        let template = "kubectl logs {{pod}} -n {{ns=default}}";
        let rendered = render(template, &values(&[("pod", "api-0"), ("ns", "edge")])).unwrap();
        assert_eq!(rendered, "kubectl logs api-0 -n edge");
    }

    #[test]
    fn substitutes_every_occurrence_verbatim() {
        let template = "ssh {{host}} 'cat /etc/hostname' && ssh {{host}} uptime";
        assert_eq!(
            render(template, &values(&[("host", "10.0.0.1")])).unwrap(),
            "ssh 10.0.0.1 'cat /etc/hostname' && ssh 10.0.0.1 uptime"
        );
    }

    #[test]
    fn reports_missing_parameters_sorted() {
        let template = "docker {{verb}} {{container_id}} {{path}}";
        let error = render(template, &values(&[("path", "/tmp")])).unwrap_err();
        assert_eq!(error.missing, vec!["container_id", "verb"]);
        assert!(error.unknown.is_empty());
        assert_eq!(
            error.message(),
            "missing required parameter(s): container_id, verb"
        );
    }

    #[test]
    fn reports_unknown_parameters() {
        let template = "kubectl get pods -n {{ns}}";
        let error =
            render(template, &values(&[("ns", "edge"), ("namespace", "edge")])).unwrap_err();
        assert!(error.missing.is_empty());
        assert_eq!(error.unknown, vec!["namespace"]);
    }

    #[test]
    fn arguments_without_placeholders_are_rejected() {
        let error = render("git status", &values(&[("ns", "x")])).unwrap_err();
        assert_eq!(error.unknown, vec!["ns"]);
    }

    #[test]
    fn plain_commands_render_untouched() {
        for template in [
            "git status",
            "awk '{print $1}'",
            "for i in {1..3}; do echo $i; done",
            "echo ${HOME}",
        ] {
            assert_eq!(render(template, &values(&[])).unwrap(), template);
            assert!(!is_parameterized(template));
        }
    }

    #[test]
    fn non_identifier_braces_stay_literal() {
        for template in ["echo {{1..3}}", "echo {{ a b }}", "echo {{}}", "echo {{}}"] {
            assert_eq!(
                render(template, &values(&[])).unwrap(),
                template,
                "{template} must stay literal"
            );
        }
    }

    #[test]
    fn spaces_inside_braces_are_tolerated() {
        assert_eq!(
            render("ls {{ path }}", &values(&[("path", "/tmp")])).unwrap(),
            "ls /tmp"
        );
    }

    #[test]
    fn unclosed_braces_do_not_panic() {
        for template in ["echo {{", "echo {{path", "{{", "}}", "{{{"] {
            assert_eq!(render(template, &values(&[])).unwrap(), template);
        }
    }

    #[test]
    fn values_are_inserted_verbatim() {
        let template = "cat {{file}}";
        assert_eq!(
            render(template, &values(&[("file", "'/tmp/my file.txt'")])).unwrap(),
            "cat '/tmp/my file.txt'"
        );
    }

    #[test]
    fn legacy_markers_are_parameters_too() {
        let template = "docker exec -it #container_id! sh -lc 'ls #path!'";
        let specs = param_specs_with(template, "#", "!");
        assert_eq!(
            specs
                .iter()
                .map(|spec| spec.name.as_str())
                .collect::<Vec<_>>(),
            vec!["container_id", "path"]
        );
        assert!(specs.iter().all(ParamSpec::required));
        assert_eq!(
            render_with(
                template,
                &values(&[("container_id", "web"), ("path", "/srv")]),
                Some(("#", "!"))
            )
            .unwrap(),
            "docker exec -it web sh -lc 'ls /srv'"
        );
    }

    #[test]
    fn legacy_markers_are_invisible_without_their_dialect() {
        let template = "grep -n '#name!' file";
        assert!(param_specs(template).is_empty());
        assert_eq!(render(template, &values(&[])).unwrap(), template);
        // ... and their absence is not a missing parameter.
        assert_eq!(render_with(template, &values(&[]), None).unwrap(), template);
    }

    #[test]
    fn both_dialects_render_in_one_pass() {
        let template = "scp {{src}} box:#dest!";
        let rendered = render_with(
            template,
            &values(&[("src", "a.txt"), ("dest", "/tmp")]),
            Some(("#", "!")),
        )
        .unwrap();
        assert_eq!(rendered, "scp a.txt box:/tmp");
    }

    #[test]
    fn shebangs_and_multiline_markers_are_not_parameters() {
        let script = "#!/usr/bin/env bash\nset -e\nls #dir!";
        let specs = param_specs_with(script, "#", "!");
        assert_eq!(
            specs
                .iter()
                .map(|spec| spec.name.as_str())
                .collect::<Vec<_>>(),
            vec!["dir"]
        );
        assert!(
            !param_specs_with("#!/bin/sh\necho hi", "#", "!")
                .iter()
                .any(|spec| spec.name.contains('\n'))
        );
        assert!(param_specs_with("#!/bin/sh\necho hi", "#", "!").is_empty());
    }

    #[test]
    fn an_unmatched_legacy_marker_is_left_alone() {
        let template = "echo '#not_closed'";
        assert!(param_specs_with(template, "#", "!").is_empty());
        assert_eq!(
            render_with(template, &values(&[]), Some(("#", "!"))).unwrap(),
            template
        );
    }
}
