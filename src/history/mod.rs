//! Implementation of `hoard history scan` and `hoard history import`.
//!
//! History is full of noise, duplicates and the occasional pasted credential,
//! so both subcommands share one pipeline and differ only in the ending:
//!
//! * `scan` — read-only report of what would be imported.
//! * `import` — the same candidates, selected interactively (or wholesale with
//!   `--all`) and added through the same path `hoard new` uses.
//!
//! Per source: read -> clean -> drop noise -> redact secrets -> dedupe against
//! the trove and within the run -> (optional) trufflehog pass.

pub mod sessions;
pub mod tags;
pub mod trufflehog;
pub mod zsh;

use crate::cli_commands::HistoryArgs;
use crate::core::parameters::{DEFAULT_ENDING_PARAMETER_TOKEN, DEFAULT_PARAMETER_TOKEN};
use crate::core::secrets::{Redacted, redact_secrets};
use crate::core::trove::Trove;
use crate::core::{CommandKind, HoardCmd};
use crate::gui::prompts::prompt_multiselect_options;
use anyhow::Result;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Namespace shell history imports land in.
const SHELL_NAMESPACE: &str = "history";
/// Namespace agent session imports land in.
const PI_NAMESPACE: &str = "pi";
/// How many sample candidates a scan prints.
const SAMPLE_LIMIT: usize = 20;
/// Shorter commands are noise.
const MIN_COMMAND_LEN: usize = 3;
/// Longest name derived from a command, before collision suffixes.
const MAX_NAME_LEN: usize = 48;
/// Separator used to keep the name bookkeeping namespace aware.
const NAME_KEY_SEPARATOR: char = '\u{1f}';

/// Programs that are pure navigation/inspection when used without arguments.
const BARE_NOISE: &[&str] = &[
    "cd", "ls", "ll", "la", "pwd", "clear", "exit", "logout", "history", "jobs", "fg", "bg", "top",
    "htop", "man", "which", "whoami",
];

/// Prompt prefixes that end up in pasted lines.
const PROMPT_PREFIXES: &[&str] = &["❯ ", "$ ", "% "];

/// The source a candidate came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Origin {
    Zsh,
    Pi,
}

impl Origin {
    fn namespace(self) -> &'static str {
        match self {
            Self::Zsh => SHELL_NAMESPACE,
            Self::Pi => PI_NAMESPACE,
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Self::Zsh => "zsh",
            Self::Pi => "pi",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Zsh => "zsh history",
            Self::Pi => "pi sessions",
        }
    }
}

/// What to read and how to redact it.
#[derive(Debug, Clone)]
pub struct Options {
    /// zsh history file, `$HISTFILE` or `~/.zsh_history` when `None`.
    pub histfile: Option<PathBuf>,
    /// Agent session roots; the defaults when empty.
    pub sessions_dir: Vec<PathBuf>,
    /// `auto`, `off` or a trufflehog binary.
    pub trufflehog: String,
    /// Cap on candidates, applied after filtering and dedupe.
    pub limit: Option<usize>,
    /// Read zsh history.
    pub zsh: bool,
    /// Read agent sessions.
    pub pi: bool,
}

impl Options {
    /// Build the options from the CLI flags.
    pub fn from_args(args: &HistoryArgs) -> Self {
        Self {
            histfile: args.histfile.clone(),
            sessions_dir: args.sessions_dir.clone(),
            trufflehog: args.trufflehog.clone(),
            limit: args.limit,
            zsh: args.source.includes_zsh(),
            pi: args.source.includes_pi(),
        }
    }
}

/// An importable command, already cleaned and redacted.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// Shell command or Python source.
    pub kind: CommandKind,
    /// The cleaned source.
    pub command: String,
    /// Namespace this source imports into.
    pub namespace: String,
    /// Capture date, used in the generated Python description.
    pub captured: Option<SystemTime>,
    /// Which source produced it.
    pub source: Origin,
    /// Whether a placeholder was substituted.
    pub redacted: bool,
    /// How many times this command appeared in the sources.
    pub occurrences: usize,
}

impl Candidate {
    /// The command reduced to one line for listing.
    pub fn preview(&self) -> String {
        let first = first_line(&self.command);
        let mut preview: String = first.chars().take(100).collect();
        if self.command.lines().count() > 1 {
            preview.push_str(" …");
        }
        // A command repeated across sessions is the kind worth hoarding.
        if self.occurrences > 1 {
            preview = format!("×{} {preview}", self.occurrences);
        }
        preview
    }
}

/// Per-source counters.
#[derive(Debug, Default, Clone, Copy)]
pub struct Counts {
    pub raw: usize,
    pub candidates: usize,
    pub present: usize,
    pub duplicates: usize,
    pub noise: usize,
    pub redacted: usize,
    pub skipped_secret: usize,
}

/// Counters for both sources.
#[derive(Debug, Default)]
pub struct Stats {
    pub zsh: Counts,
    pub pi: Counts,
    /// Filtered entries per noise reason.
    pub noise_reasons: std::collections::BTreeMap<&'static str, usize>,
}

impl Stats {
    fn for_origin(&mut self, origin: Origin) -> &mut Counts {
        match origin {
            Origin::Zsh => &mut self.zsh,
            Origin::Pi => &mut self.pi,
        }
    }
}

/// Whether the trufflehog pass ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trufflehog {
    /// `--trufflehog off`.
    Off,
    /// Asked for, but the binary is not on `PATH`.
    Unavailable,
    /// Ran and reported these findings.
    Ran { findings: usize },
}

impl Trufflehog {
    fn label(&self) -> String {
        match self {
            Self::Off => "off (built-in redaction only)".to_string(),
            Self::Unavailable => "not installed (built-in redaction only)".to_string(),
            Self::Ran { findings } => format!("{findings} finding(s) redacted"),
        }
    }
}

/// The result of reading every configured source.
#[derive(Debug)]
pub struct Collected {
    pub candidates: Vec<Candidate>,
    pub stats: Stats,
    /// The zsh history file that was read, if any.
    pub histfile: Option<PathBuf>,
    /// Session roots that were scanned.
    pub session_roots: Vec<PathBuf>,
    /// Session transcripts that were read.
    pub session_files: usize,
    pub trufflehog: Trufflehog,
    /// Non-fatal problems, to be reported on stderr.
    pub warnings: Vec<String>,
    /// Parameter token used for redaction.
    token: String,
    /// Parameter ending token used for redaction.
    ending: String,
}

impl Collected {
    /// Read every configured source and build the candidate list.
    pub fn new(trove: &Trove, options: &Options, token: &str, ending: &str) -> Self {
        let token = if token.is_empty() {
            DEFAULT_PARAMETER_TOKEN.to_string()
        } else {
            token.to_string()
        };
        let ending = if ending.is_empty() {
            DEFAULT_ENDING_PARAMETER_TOKEN.to_string()
        } else {
            ending.to_string()
        };

        let mut collected = Self {
            candidates: Vec::new(),
            stats: Stats::default(),
            histfile: None,
            session_roots: Vec::new(),
            session_files: 0,
            trufflehog: Trufflehog::Off,
            warnings: Vec::new(),
            token: token.clone(),
            ending: ending.clone(),
        };

        // Command text already in the trove: never import it twice.
        let existing: HashSet<String> = trove
            .commands
            .iter()
            .map(|command| normalize(&command.command))
            .collect();
        // Commands already picked up from an earlier entry in this run.
        let mut seen: HashMap<String, usize> = HashMap::new();

        if options.zsh {
            let path = options.histfile.clone().or_else(default_histfile);
            if let Some(path) = path
                && path.is_file()
            {
                collected.histfile = Some(path.clone());
                match zsh::parse_file(&path) {
                    Ok(entries) => {
                        for entry in entries {
                            collected.absorb(
                                entry.command,
                                CommandKind::Shell,
                                Origin::Zsh,
                                None,
                                &existing,
                                &mut seen,
                            );
                        }
                    }
                    Err(err) => collected
                        .warnings
                        .push(format!("could not read {}: {err}", path.display())),
                }
            }
        }

        if options.pi {
            let roots = if options.sessions_dir.is_empty() {
                sessions::default_roots()
            } else {
                options.sessions_dir.clone()
            };
            let extracted = sessions::collect(&roots);
            collected.session_roots = roots;
            collected.session_files = extracted.files;
            for session_command in extracted.commands {
                let captured = session_command.captured;
                collected.absorb(
                    session_command.command,
                    session_command.kind,
                    Origin::Pi,
                    captured,
                    &existing,
                    &mut seen,
                );
            }
        }

        // Repeated commands first: those are the ones worth hoarding.
        collected
            .candidates
            .sort_by_key(|candidate| std::cmp::Reverse(candidate.occurrences));

        if let Some(limit) = options.limit {
            collected.candidates.truncate(limit);
        }

        collected.apply_trufflehog(options, &token, &ending);
        collected.candidates.retain(|candidate| {
            let only = is_only_placeholders(&candidate.command, &token, &ending);
            if only {
                collected.stats.for_origin(candidate.source).skipped_secret += 1;
            }
            !only
        });

        // Report the counters that are only knowable once everything ran.
        let mut zsh = Counts::default();
        let mut pi = Counts::default();
        for candidate in &collected.candidates {
            let counts = match candidate.source {
                Origin::Zsh => &mut zsh,
                Origin::Pi => &mut pi,
            };
            counts.candidates += 1;
            counts.redacted += usize::from(candidate.redacted);
        }
        collected.stats.zsh.candidates = zsh.candidates;
        collected.stats.pi.candidates = pi.candidates;
        collected.stats.zsh.redacted += zsh.redacted;
        collected.stats.pi.redacted += pi.redacted;

        collected
    }

    /// Clean, filter, redact and dedupe one raw entry.
    fn absorb(
        &mut self,
        raw: String,
        kind: CommandKind,
        origin: Origin,
        captured: Option<SystemTime>,
        existing: &HashSet<String>,
        seen: &mut HashMap<String, usize>,
    ) {
        self.stats.for_origin(origin).raw += 1;

        let cleaned = match kind {
            CommandKind::Shell => strip_cd_prefix(&clean_shell(&raw)),
            CommandKind::Python => clean_script(&raw),
        };
        if kind == CommandKind::Shell
            && let Some(reason) = noise_reason(&cleaned)
        {
            self.stats.for_origin(origin).noise += 1;
            *self.stats.noise_reasons.entry(reason).or_insert(0) += 1;
            return;
        }

        // Built-in redaction, using the project's `#name!` parameter syntax.
        // Python is source code: rewriting it would corrupt the archived file
        // (see `HoardCmd::with_redacted_secrets`), so a script that carries a
        // recognisable secret is dropped instead of being stored or mangled.
        let token = self.token.clone();
        let ending = self.ending.clone();
        let redacted = match kind {
            CommandKind::Shell => redact_secrets(&cleaned, &token, &ending),
            CommandKind::Python => {
                // Only a recognisable credential blocks a script: the generic
                // "long quoted literal" rule would otherwise drop every script
                // that happens to contain a hex constant or a base64 fixture.
                let found = redact_secrets(&cleaned, &token, &ending);
                if found
                    .redactions
                    .iter()
                    .any(|redaction| redaction.label != "quoted token literal")
                {
                    self.stats.for_origin(origin).skipped_secret += 1;
                    return;
                }
                Redacted {
                    command: cleaned.clone(),
                    redactions: Vec::new(),
                }
            }
        };
        if is_only_placeholders(&redacted.command, &token, &ending) {
            self.stats.for_origin(origin).skipped_secret += 1;
            return;
        }

        // Dedupe on the stored form: the trove keeps redacted commands, so the
        // raw text would not match an entry imported before.
        let key = normalize(&redacted.command);
        if existing.contains(&key) {
            self.stats.for_origin(origin).present += 1;
            return;
        }
        if let Some(index) = seen.get(&key) {
            self.stats.for_origin(origin).duplicates += 1;
            self.candidates[*index].occurrences += 1;
            return;
        }
        seen.insert(key, self.candidates.len());

        self.candidates.push(Candidate {
            kind,
            command: redacted.command,
            namespace: origin.namespace().to_string(),
            captured,
            source: origin,
            redacted: !redacted.redactions.is_empty(),
            occurrences: 1,
        });
    }

    /// One batched trufflehog pass over the collected candidates.
    fn apply_trufflehog(&mut self, options: &Options, token: &str, ending: &str) {
        let mode = trufflehog::Mode::parse(&options.trufflehog);
        if mode == trufflehog::Mode::Off {
            self.trufflehog = Trufflehog::Off;
            return;
        }
        let Some(binary) = mode.resolve() else {
            // An explicit path that does not exist is worth a warning; `auto`
            // simply means "trufflehog is not installed on this machine".
            if let trufflehog::Mode::Binary(path) = &mode {
                self.warnings.push(format!(
                    "trufflehog binary {} not found; using built-in redaction only",
                    path.display()
                ));
            }
            self.trufflehog = Trufflehog::Unavailable;
            return;
        };
        if self.candidates.is_empty() {
            self.trufflehog = Trufflehog::Ran { findings: 0 };
            return;
        }

        let mut texts: Vec<String> = self
            .candidates
            .iter()
            .map(|candidate| candidate.command.clone())
            .collect();
        let borrowed: Vec<&str> = texts.iter().map(String::as_str).collect();

        match trufflehog::run(&binary, &borrowed) {
            Ok(findings) => {
                let applied = trufflehog::apply(&findings, &mut texts, token, ending);
                for (candidate, text) in self.candidates.iter_mut().zip(texts) {
                    if candidate.command != text {
                        candidate.command = text;
                        candidate.redacted = true;
                    }
                }
                if !applied.unplaced.is_empty() {
                    // trufflehog named a secret we could not locate verbatim (it
                    // also reports decoded forms). Dropping the candidate keeps
                    // the guarantee that nothing unredacted is imported.
                    let suspects: HashSet<usize> = applied.unplaced.iter().copied().collect();
                    let mut detectors: Vec<&str> = findings
                        .iter()
                        .filter(|finding| suspects.contains(&finding.index))
                        .map(|finding| finding.detector.as_str())
                        .collect();
                    detectors.sort_unstable();
                    detectors.dedup();
                    self.warnings.push(format!(
                        "trufflehog reported finding(s) that could not be redacted ({}); dropped {} candidate(s)",
                        detectors.join(", "),
                        suspects.len()
                    ));
                    let mut index = 0;
                    self.candidates.retain(|_| {
                        let keep = !suspects.contains(&index);
                        index += 1;
                        keep
                    });
                }
                self.trufflehog = Trufflehog::Ran {
                    findings: applied.findings,
                };
            }
            Err(err) => {
                self.warnings.push(format!(
                    "trufflehog scan skipped ({err}); using built-in redaction only"
                ));
                self.trufflehog = Trufflehog::Unavailable;
            }
        }
    }

    /// Up to [`SAMPLE_LIMIT`] candidates, for the report.
    pub fn samples(&self) -> &[Candidate] {
        let end = self.candidates.len().min(SAMPLE_LIMIT);
        &self.candidates[..end]
    }

    /// The human readable report.
    pub fn report(&self) -> String {
        let mut out = String::new();
        self.write_source(&mut out, Origin::Zsh);
        self.write_source(&mut out, Origin::Pi);
        self.write_noise_reasons(&mut out);
        let _ = writeln!(out, "trufflehog: {}", self.trufflehog.label());
        let _ = writeln!(
            out,
            "candidates: {}",
            self.stats.zsh.candidates + self.stats.pi.candidates
        );
        if self.candidates.is_empty() {
            let _ = writeln!(out, "\nNothing hoardable found.");
            return out;
        }
        let _ = writeln!(out, "\nsample:");
        for (index, candidate) in self.samples().iter().enumerate() {
            let _ = writeln!(
                out,
                "  {:>3}. [{}] {}",
                index + 1,
                candidate.source.label(),
                candidate.preview()
            );
        }
        if self.candidates.len() > SAMPLE_LIMIT {
            let _ = writeln!(
                out,
                "  … {} more, run `hoard history import` to pick",
                self.candidates.len() - SAMPLE_LIMIT
            );
        }
        out
    }

    fn write_source(&self, out: &mut String, origin: Origin) {
        let counts = match origin {
            Origin::Zsh => &self.stats.zsh,
            Origin::Pi => &self.stats.pi,
        };
        let _ = match origin {
            Origin::Zsh => match &self.histfile {
                Some(path) => writeln!(out, "{}: {}", origin.label(), path.display()),
                None => writeln!(
                    out,
                    "{}: not found (set $HISTFILE or pass --histfile)",
                    origin.label()
                ),
            },
            Origin::Pi => {
                if self.session_roots.is_empty() {
                    writeln!(out, "{}: no session directories found", origin.label())
                } else {
                    writeln!(
                        out,
                        "{}: {} file(s) in {} root(s)",
                        origin.label(),
                        self.session_files,
                        self.session_roots.len()
                    )
                }
            }
        };
        let _ = writeln!(out, "  entries read       {:>6}", counts.raw);
        let _ = writeln!(out, "  candidates         {:>6}", counts.candidates);
        let _ = writeln!(out, "  already in trove   {:>6}", counts.present);
        let _ = writeln!(out, "  duplicates         {:>6}", counts.duplicates);
        let _ = writeln!(out, "  noise filtered     {:>6}", counts.noise);
        let _ = writeln!(out, "  secret redacted    {:>6}", counts.redacted);
        let _ = writeln!(out, "  secret-only skipped{:>6}", counts.skipped_secret);
    }

    fn write_noise_reasons(&self, out: &mut String) {
        if self.stats.noise_reasons.is_empty() {
            return;
        }
        let reasons: Vec<String> = self
            .stats
            .noise_reasons
            .iter()
            .map(|(reason, count)| format!("{reason} {count}"))
            .collect();
        let _ = writeln!(out, "noise reasons: {}", reasons.join(", "));
    }

    /// The same numbers as one JSON object.
    pub fn report_json(&self) -> String {
        let source = |counts: &Counts| {
            serde_json::json!({
                "raw": counts.raw,
                "candidates": counts.candidates,
                "present": counts.present,
                "duplicates": counts.duplicates,
                "noise": counts.noise,
                "redacted": counts.redacted,
                "skipped_secret": counts.skipped_secret,
            })
        };
        let samples: Vec<serde_json::Value> = self
            .samples()
            .iter()
            .map(|candidate| {
                serde_json::json!({
                    "namespace": candidate.namespace,
                    "kind": candidate.kind.as_str(),
                    "command": candidate.command,
                })
            })
            .collect();
        serde_json::json!({
            "zsh": source(&self.stats.zsh),
            "pi": source(&self.stats.pi),
            "candidates": self.stats.zsh.candidates + self.stats.pi.candidates,
            "histfile": self.histfile.as_ref().map(|path| path.display().to_string()),
            "session_files": self.session_files,
            "trufflehog": self.trufflehog.label(),
            "samples": samples,
        })
        .to_string()
    }
}

/// What `hoard history import` should do.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// Namespace for every entry instead of the per-source defaults.
    pub namespace: Option<String>,
    /// Skip the picker and import everything.
    pub all: bool,
    /// Report only, write nothing.
    pub dry_run: bool,
}

/// The names that were (or, on a dry run, would be) added.
#[derive(Debug, Default)]
pub struct ImportOutcome {
    pub names: Vec<String>,
    pub dry_run: bool,
}

impl ImportOutcome {
    /// `true` when the caller must persist the trove.
    pub fn must_persist(&self) -> bool {
        !self.dry_run && !self.names.is_empty()
    }
}

/// Add the selected candidates to `trove`.
pub fn import(
    trove: &mut Trove,
    collection: Collected,
    options: &ImportOptions,
) -> Result<ImportOutcome> {
    if collection.candidates.is_empty() {
        println!("Nothing to import. Run `hoard history scan` to see what was filtered out.");
        return Ok(ImportOutcome {
            names: Vec::new(),
            dry_run: options.dry_run,
        });
    }

    let selected = if options.all || options.dry_run {
        // A preview must never wait on a prompt: show what `--all` would take.
        collection.candidates
    } else {
        match pick(&collection.candidates)? {
            Some(selected) => selected,
            None => {
                println!("Nothing selected, nothing imported.");
                return Ok(ImportOutcome {
                    names: Vec::new(),
                    dry_run: options.dry_run,
                });
            }
        }
    };

    let mut taken: HashSet<String> = trove
        .commands
        .iter()
        .map(|command| name_key(&command.namespace, &command.name))
        .collect();
    let mut outcome = ImportOutcome {
        names: Vec::new(),
        dry_run: options.dry_run,
    };

    for candidate in selected {
        let namespace = options
            .namespace
            .clone()
            .unwrap_or_else(|| candidate.namespace.clone());
        let base = slugify_command(&candidate.command);
        let name = reserve_name(&mut taken, &namespace, &base);

        if options.dry_run {
            outcome.names.push(name.clone());
            if outcome.names.len() <= SAMPLE_LIMIT {
                println!(
                    "would add {name:<MAX_NAME_LEN$} [{namespace}] {}",
                    candidate.preview()
                );
            }
            continue;
        }

        let entry = HoardCmd {
            name: name.clone(),
            command: candidate.command.clone(),
            kind: candidate.kind,
            description: description_for(&candidate),
            namespace,
            ..HoardCmd::default().with_tags_raw(
                &tags::tags_for(&candidate.command, candidate.kind, candidate.source).join(","),
            )
        };
        // One rejected command must not cost the rest of the batch.
        match trove.add_command(entry, false) {
            Ok(_) => outcome.names.push(name),
            Err(err) => eprintln!("skipped {name}: {err}"),
        }
    }

    if options.dry_run {
        if outcome.names.len() > SAMPLE_LIMIT {
            println!("… {} more", outcome.names.len() - SAMPLE_LIMIT);
        }
        println!(
            "\nDry run: {} command(s) would be added, nothing was written.",
            outcome.names.len()
        );
    } else {
        println!("Imported {} command(s).", outcome.names.len());
    }
    Ok(outcome)
}

/// Prompt for the candidates to import. `None` means "nothing selected".
fn pick(candidates: &[Candidate]) -> Result<Option<Vec<Candidate>>> {
    let indices: Vec<usize> = (0..candidates.len()).collect();
    let chosen = prompt_multiselect_options(
        "Select the commands to import",
        "space toggles, enter confirms",
        &indices,
        |index| format!("[{:>3}] {}", index + 1, candidates[*index].preview()),
    );
    if chosen.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        chosen
            .into_iter()
            .filter_map(|index| candidates.get(index).cloned())
            .collect(),
    ))
}

/// Description for an imported entry. Shell commands stay empty; a Python
/// script must carry one because `HoardCmd::is_valid` requires it.
fn description_for(candidate: &Candidate) -> String {
    match candidate.kind {
        CommandKind::Shell => String::new(),
        CommandKind::Python => format!(
            "Python script captured from pi session on {}",
            captured_date(candidate.captured)
        ),
    }
}

/// `YYYY-MM-DD` for a capture time, without pulling in a date crate.
fn captured_date(captured: Option<SystemTime>) -> String {
    let Some(captured) = captured else {
        return "an unknown date".to_string();
    };
    let millis = captured
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis();
    let days = i64::try_from(millis / 1_000).unwrap_or_default() / 86_400;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Howard Hinnant's `civil_from_days`, for the days since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = u32::try_from(day_of_year - (153 * shifted_month + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    })
    .unwrap_or(1);
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// `$HISTFILE`, else `~/.zsh_history`.
fn default_histfile() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("HISTFILE").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(path));
    }
    dirs::home_dir().map(|home| home.join(".zsh_history"))
}

/// Strip ANSI CSI and OSC sequences left behind by pasted prompts.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        match chars.peek() {
            // CSI: ESC [ ... final byte in @..~
            Some('[') => {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            // OSC: ESC ] ... BEL or ESC \
            Some(']') => {
                chars.next();
                let mut escaped = false;
                for next in chars.by_ref() {
                    match next {
                        '\u{7}' => break,
                        '\u{1b}' => escaped = true,
                        '\\' if escaped => break,
                        _ => escaped = false,
                    }
                }
            }
            // Two byte escape.
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

/// Remove a leading `$ `/`% `/`❯ ` prompt marker.
fn strip_prompt(text: &str) -> &str {
    for prefix in PROMPT_PREFIXES {
        if let Some(rest) = text.strip_prefix(prefix) {
            return rest;
        }
    }
    text
}

/// Clean one shell history entry: no CR, no ANSI, no prompt, no stray blanks.
pub fn clean_shell(raw: &str) -> String {
    let text = strip_ansi(&raw.replace("\r\n", "\n").replace('\r', ""));
    let text = strip_prompt(text.trim());
    let mut lines: Vec<String> = Vec::new();
    let mut heredoc: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim_end();
        match &heredoc {
            // A heredoc body is data: collapsing it would corrupt the payload.
            Some(terminator) => {
                lines.push(trimmed.to_string());
                if trimmed.trim() == terminator {
                    heredoc = None;
                }
            }
            None => {
                heredoc = heredoc_terminator(trimmed);
                lines.push(collapse_spaces(trimmed));
            }
        }
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

/// Drop a leading `cd <dir> &&` chain.
///
/// Agent sessions prefix almost every command with the directory the agent
/// happens to work in, which makes an otherwise reusable command one-off. The
/// remainder is what gets hoarded; a bare `cd` stays and is filtered as noise.
fn strip_cd_prefix(command: &str) -> String {
    static PREFIX: OnceLock<Regex> = OnceLock::new();
    let regex = PREFIX.get_or_init(|| {
        Regex::new(r#"^(?:cd\s+(?:'[^']*'|"[^"]*"|\S+)\s*&&\s*)+"#)
            .expect("cd prefix pattern is always valid")
    });
    match regex.find(command) {
        Some(found) if found.end() < command.len() => command[found.end()..].to_string(),
        _ => command.to_string(),
    }
}

/// The terminator of a heredoc started on this line, e.g. `EOF` for
/// `cat <<'EOF'`. Only the common quoting forms are recognised.
fn heredoc_terminator(line: &str) -> Option<String> {
    static HEREDOC: OnceLock<Regex> = OnceLock::new();
    let regex = HEREDOC.get_or_init(|| {
        Regex::new(r#"<<-?\s*(?:'([A-Za-z_][A-Za-z0-9_]*)'|"([A-Za-z_][A-Za-z0-9_]*)"|\\?([A-Za-z_][A-Za-z0-9_]*))"#)
            .expect("heredoc pattern is always valid")
    });
    let caps = regex.captures(line)?;
    (1..=3)
        .filter_map(|group| caps.get(group))
        .map(|m| m.as_str().to_string())
        .next()
}

/// Collapse runs of spaces and tabs outside quotes: pasted commands arrive with
/// odd spacing, but spacing inside `'…'`/`"…"` is data. Leading indentation is
/// kept, so continuation lines and heredoc bodies stay readable.
fn collapse_spaces(line: &str) -> String {
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    let (lead, rest) = line.split_at(indent);
    let mut out = String::with_capacity(line.len());
    out.push_str(lead);
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut pending = false;
    for ch in rest.chars() {
        if let Some(open) = quote {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' && open == '"' {
                escaped = true;
            } else if ch == open {
                quote = None;
            }
        } else if ch == ' ' || ch == '\t' {
            pending = true;
        } else {
            if pending {
                out.push(' ');
                pending = false;
            }
            out.push(ch);
            if ch == '\'' || ch == '"' {
                quote = Some(ch);
            }
        }
    }
    out
}

/// Clean a captured Python source; indentation is preserved.
pub fn clean_script(raw: &str) -> String {
    let text = strip_ansi(&raw.replace("\r\n", "\n").replace('\r', ""));
    text.trim_matches('\n').trim().to_string()
}

/// Whitespace-insensitive form used for dedupe.
pub fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Why an entry is noise, if it is.
pub fn noise_reason(command: &str) -> Option<&'static str> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Some("empty");
    }
    if trimmed.chars().count() < MIN_COMMAND_LEN {
        return Some("too short");
    }
    let collapsed = normalize(trimmed);
    if matches!(collapsed.as_str(), "!!" | "sudo !!" | "!-1" | "sudo !-1") {
        return Some("history recall");
    }
    let mut words = collapsed.split(' ');
    let program = words.next().unwrap_or_default();
    let has_args = words.next().is_some();

    // `cd`/`ls` are navigation even with a path, but `cd repo && cargo test`
    // only starts there, so any further shell operator makes it interesting.
    if matches!(program, "cd" | "ls") {
        if !collapsed.contains(['&', ';', '|', '>', '<']) {
            return Some("navigation");
        }
        return None;
    }
    if !has_args && BARE_NOISE.contains(&program) {
        return Some("navigation");
    }
    None
}

/// `true` when nothing but `#name!` placeholders (and punctuation) is left.
pub fn is_only_placeholders(text: &str, token: &str, ending: &str) -> bool {
    if token.is_empty() || ending.is_empty() {
        return false;
    }
    let mut rest = String::with_capacity(text.len());
    let mut cursor = text;
    while let Some(start) = cursor.find(token) {
        let after = &cursor[start + token.len()..];
        let Some(end) = after.find(ending) else {
            break;
        };
        rest.push_str(&cursor[..start]);
        rest.push(' ');
        cursor = &after[end + ending.len()..];
    }
    rest.push_str(cursor);
    !rest.chars().any(char::is_alphanumeric)
}

/// The first non-empty line, trimmed.
pub fn first_line(command: &str) -> &str {
    command
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
}

/// Slug for the name derived from a command's first line.
pub fn slugify_command(command: &str) -> String {
    let mut slug = String::new();
    for ch in first_line(command).chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let mut slug = slug.trim_end_matches('-').to_string();
    if slug.chars().count() > MAX_NAME_LEN {
        let cut: String = slug.chars().take(MAX_NAME_LEN).collect();
        slug = cut.trim_end_matches('-').to_string();
    }
    if slug.is_empty() {
        "imported-command".to_string()
    } else {
        slug
    }
}

fn name_key(namespace: &str, name: &str) -> String {
    format!("{namespace}{NAME_KEY_SEPARATOR}{name}")
}

/// Reserve a name in `namespace`, suffixing with `-2`, `-3`, … on collision.
fn reserve_name(taken: &mut HashSet<String>, namespace: &str, base: &str) -> String {
    let mut candidate = base.to_string();
    let mut suffix = 2;
    while !taken.insert(name_key(namespace, &candidate)) {
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }
    candidate
}

#[cfg(test)]
mod test_history {
    use super::*;

    fn collect_history(content: &str) -> Collected {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".zsh_history");
        std::fs::write(&path, content).unwrap();
        let options = Options {
            histfile: Some(path),
            sessions_dir: vec![dir.path().join("no-sessions")],
            trufflehog: "off".to_string(),
            limit: None,
            zsh: true,
            pi: false,
        };
        Collected::new(&Trove::default(), &options, "#", "!")
    }

    #[test]
    fn strips_prompt_prefixes_and_ansi_sequences() {
        assert_eq!(
            clean_shell("\u{1b}[1;32m❯\u{1b}[0m git status"),
            "git status"
        );
        assert_eq!(clean_shell("   ❯ ls -la\r"), "ls -la");
        assert_eq!(clean_shell("$ kubectl get pods"), "kubectl get pods");
        assert_eq!(clean_shell("\u{1b}]0;title\u{7}docker ps"), "docker ps");
    }

    #[test]
    fn leading_space_paste_is_trimmed_but_script_indentation_survives() {
        assert_eq!(clean_shell("    terraform plan"), "terraform plan");
        assert_eq!(
            clean_shell("python3 - <<'EOF'\n  print(1)\nEOF"),
            "python3 - <<'EOF'\n  print(1)\nEOF"
        );
    }

    #[test]
    fn pasted_spacing_is_collapsed_outside_quotes_only() {
        assert_eq!(
            clean_shell("    kubectl    get   pods   -A   "),
            "kubectl get pods -A"
        );
        assert_eq!(clean_shell("kubectl\tget\tpods"), "kubectl get pods");
        // Spacing inside quotes is data, not formatting.
        assert_eq!(
            clean_shell("grep 'a  b'   file   name"),
            "grep 'a  b' file name"
        );
        assert_eq!(
            clean_shell(r#"awk "{print  $1}"   -F:   f"#),
            r#"awk "{print  $1}" -F: f"#
        );
    }

    #[test]
    fn heredoc_bodies_are_left_verbatim() {
        let pasted = "cat <<'EOF' > Makefile\nall:\n\t\techo  a   b\nEOF";
        assert_eq!(clean_shell(pasted), pasted);
        // Outside the heredoc, spacing is still collapsed.
        assert_eq!(
            clean_shell("ssh   host   'cat <<EOF\n  keep   me\nEOF'"),
            "ssh host 'cat <<EOF\n  keep   me\nEOF'"
        );
    }

    #[test]
    fn navigation_filter_keeps_compound_commands() {
        assert!(noise_reason("cd /tmp").is_some());
        assert!(noise_reason("ls -la").is_some());
        assert!(noise_reason("cd ~/Dev/hoard && cargo test").is_none());
        assert!(noise_reason("ls *.toml | xargs wc -l").is_none());
    }

    #[test]
    fn repeated_commands_are_counted_and_ranked_first() {
        let collection = collect_history(
            ": 1700000000:0;make test\n: 1700000001:0;git status\n: 1700000002:0;git status\n",
        );
        assert_eq!(collection.stats.zsh.duplicates, 1);
        let first = &collection.candidates[0];
        assert_eq!(first.command, "git status");
        assert_eq!(first.occurrences, 2);
        assert_eq!(collection.candidates[1].occurrences, 1);
        assert!(first.preview().starts_with("×2 "));
    }

    #[test]
    fn denylist_filters_navigation_and_recalls_only() {
        for noisy in [
            "cd", "cd /tmp", "ls", "ls -la", "pwd", "clear", "exit", "history", "ll", "!!",
            "sudo !!", "", "a",
        ] {
            assert!(
                noise_reason(noisy).is_some(),
                "{noisy:?} should be filtered"
            );
        }
        for interesting in [
            "git status",
            "docker compose up -d",
            "history | grep kubectl",
            "sudo systemctl restart nginx",
        ] {
            assert!(
                noise_reason(interesting).is_none(),
                "{interesting:?} should be a candidate"
            );
        }
    }

    #[test]
    fn slugify_derives_a_valid_name_from_the_first_line() {
        assert_eq!(slugify_command("kubectl get pods -A"), "kubectl-get-pods-a");
        assert_eq!(
            slugify_command("curl -H 'X: 1' https://example.com"),
            "curl-h-x-1-https-example-com"
        );
        assert_eq!(slugify_command("import os\nprint(os.name)"), "import-os");
        assert_eq!(slugify_command("!!!"), "imported-command");
        let long = slugify_command(&"a".repeat(120));
        assert_eq!(long.chars().count(), MAX_NAME_LEN);
        assert!(HoardCmd::is_name_valid(&long).is_ok());
    }

    #[test]
    fn only_placeholder_detection_ignores_punctuation() {
        assert!(is_only_placeholders("#github!", "#", "!"));
        assert!(is_only_placeholders("  '#github!'  ", "#", "!"));
        assert!(!is_only_placeholders("#github! deploy", "#", "!"));
        assert!(!is_only_placeholders("echo hi", "#", "!"));
    }

    #[test]
    fn duplicate_entries_collapse_and_present_ones_are_skipped() {
        let collection = collect_history(
            ": 1700000000:0;git status\n: 1700000001:0;git  status\n: 1700000002:0;cd /tmp\n: 1700000003:0;ls\n",
        );
        assert_eq!(collection.stats.zsh.raw, 4);
        assert_eq!(collection.stats.zsh.candidates, 1);
        assert_eq!(collection.stats.zsh.duplicates, 1);
        assert_eq!(collection.stats.zsh.noise, 2);
        assert_eq!(collection.candidates[0].command, "git status");
    }

    #[test]
    fn an_entry_that_is_only_a_secret_is_skipped() {
        let collection = collect_history(
            ": 1700000000:0;ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n: 1700000001:0;curl -H 'Authorization: token ghp_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb' https://api.github.com\n",
        );
        assert_eq!(collection.stats.zsh.skipped_secret, 1);
        assert_eq!(collection.stats.zsh.candidates, 1);
        let stored = &collection.candidates[0];
        assert!(stored.redacted);
        // Only the value is replaced; the structure of the command survives.
        assert!(stored.command.contains("#token!"), "{}", stored.command);
        assert!(!stored.command.contains("ghp_"), "{}", stored.command);
        assert!(!stored.command.contains("ghp_"));
    }

    #[test]
    fn missing_trufflehog_binary_falls_back_to_builtin_redaction() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".zsh_history");
        std::fs::write(
            &path,
            ": 1700000000:0;echo ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
        )
        .unwrap();
        let options = Options {
            histfile: Some(path),
            sessions_dir: Vec::new(),
            trufflehog: "/definitely/not/trufflehog".to_string(),
            limit: None,
            zsh: true,
            pi: false,
        };
        let collection = Collected::new(&Trove::default(), &options, "#", "!");
        assert_eq!(collection.trufflehog, Trufflehog::Unavailable);
        assert_eq!(collection.warnings.len(), 1);
        assert_eq!(collection.candidates.len(), 1);
        assert!(collection.candidates[0].redacted);
    }

    #[test]
    fn limit_caps_the_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".zsh_history");
        std::fs::write(&path, ": 1:0;git status\n: 2:0;git log\n: 3:0;git diff\n").unwrap();
        let options = Options {
            histfile: Some(path),
            sessions_dir: Vec::new(),
            trufflehog: "off".to_string(),
            limit: Some(2),
            zsh: true,
            pi: false,
        };
        let collection = Collected::new(&Trove::default(), &options, "#", "!");
        assert_eq!(collection.candidates.len(), 2);
        assert_eq!(collection.stats.zsh.candidates, 2);
    }

    #[test]
    fn json_report_carries_the_same_numbers() {
        let collection = collect_history(": 1700000000:0;git status\n");
        let json: serde_json::Value = serde_json::from_str(&collection.report_json()).unwrap();
        assert_eq!(json["zsh"]["raw"], 1);
        assert_eq!(json["zsh"]["candidates"], 1);
        assert_eq!(json["candidates"], 1);
        assert_eq!(json["samples"][0]["command"], "git status");
    }

    #[test]
    fn civil_dates_are_correct() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_512), (2026, 2, 28));
    }

    #[test]
    fn import_assigns_namespaces_tags_and_python_descriptions() {
        let collection = collect_history(": 1700000000:0;git status\n");
        let mut trove = Trove::default();
        let outcome = import(
            &mut trove,
            collection,
            &ImportOptions {
                namespace: None,
                all: true,
                dry_run: false,
            },
        )
        .unwrap();
        assert!(outcome.must_persist());
        assert_eq!(outcome.names, vec!["git-status".to_string()]);
        let stored = &trove.commands[0];
        assert_eq!(stored.namespace, SHELL_NAMESPACE);
        assert_eq!(stored.tags, vec!["git", "vcs", "imported", "zsh"]);
        assert_eq!(stored.description, "");
        assert!(stored.is_valid());
    }

    #[test]
    fn import_attaches_searchable_tags() {
        let collection = collect_history(": 1700000000:0;sudo kubectl get pods -A\n");
        let mut trove = Trove::default();
        import(
            &mut trove,
            collection,
            &ImportOptions {
                namespace: None,
                all: true,
                dry_run: false,
            },
        )
        .unwrap();
        assert_eq!(
            trove.commands[0].tags,
            vec![
                "kubectl",
                "kubernetes",
                "k8s",
                "permissions",
                "imported",
                "zsh"
            ]
        );
        // Tags add findability only: the imported set is unchanged.
        assert_eq!(
            trove.commands[0].command,
            "sudo kubectl get pods -A".to_string()
        );
    }

    #[test]
    fn import_dry_run_changes_nothing() {
        let collection = collect_history(": 1700000000:0;git status\n");
        let mut trove = Trove::default();
        let outcome = import(
            &mut trove,
            collection,
            &ImportOptions {
                namespace: Some("custom".to_string()),
                all: true,
                dry_run: true,
            },
        )
        .unwrap();
        assert!(!outcome.must_persist());
        assert_eq!(outcome.names.len(), 1);
        assert!(trove.commands.is_empty());
    }

    #[test]
    fn imported_names_do_not_collide_with_existing_entries() {
        let mut trove = Trove::default();
        trove
            .add_command(
                HoardCmd::default()
                    .with_name("git-status")
                    .with_command("git status")
                    .with_namespace(SHELL_NAMESPACE),
                false,
            )
            .unwrap();
        let collection = collect_history(": 1700000000:0;git status -sb\n");
        let outcome = import(
            &mut trove,
            collection,
            &ImportOptions {
                namespace: None,
                all: true,
                dry_run: false,
            },
        )
        .unwrap();
        assert_eq!(outcome.names, vec!["git-status-sb".to_string()]);
    }
}
