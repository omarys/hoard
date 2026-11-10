use crate::core::HoardCmd;
use crate::core::trove::Trove;
use crate::headless::{CliError, OutputMode};

/// Weights of the individual matches, most identifying field first.
///
/// A curated tag outranks an incidental name substring: searching`kubernetes`
/// should reach a command tagged `kubernetes` before one that merely mentions
/// it in a file path.
const NAME_EXACT: u32 = 100;
const TAG_EXACT: u32 = 45;
const NAME_CONTAINS: u32 = 40;
const TAG_CONTAINS: u32 = 25;
const NAMESPACE_CONTAINS: u32 = 20;
const DESCRIPTION_CONTAINS: u32 = 10;
const COMMAND_CONTAINS: u32 = 5;

/// Matches in ranked order, best first.
///
/// A multi-word query scores every word separately and keeps only commands
/// where *every* word matches; when that yields nothing, commands matching any
/// word are returned instead. Ties break on `(namespace, name)` so the result
/// is deterministic. An empty query returns the trove as stored.
#[cfg(test)]
pub fn search<'a>(trove: &'a Trove, query: &str) -> Vec<&'a HoardCmd> {
    search_scored(trove, query)
        .into_iter()
        .map(|(command, _)| command)
        .collect()
}

/// The same ranking as [`search`], with the lexical score of every hit.
///
/// The score is the raw sum of the per-word weights, so it orders results and
/// tells an agent how well a candidate matched; it is not normalized. Entries
/// returned for an empty query score `0`.
pub fn search_scored<'a>(trove: &'a Trove, query: &str) -> Vec<(&'a HoardCmd, u32)> {
    let words: Vec<String> = query
        .split_whitespace()
        .map(|word| word.to_lowercase())
        .collect();
    if words.is_empty() {
        return trove.commands.iter().map(|command| (command, 0)).collect();
    }

    let mut hits = ranked(trove, &words, true);
    if hits.is_empty() {
        hits = ranked(trove, &words, false);
    }
    hits.into_iter()
        .map(|(score, command)| (command, score))
        .collect()
}

/// Score candidates in descending order; `require_all` selects AND or OR matching.
///
/// The OR fallback only keeps *structural* matches (a name, tag or namespace
/// hit). A command that merely mentions both words somewhere in its body is not
/// an answer to the query, and handing one to an agent that will run it is
/// worse than admitting there is no match.
fn ranked<'a>(trove: &'a Trove, words: &[String], require_all: bool) -> Vec<(u32, &'a HoardCmd)> {
    let mut ranked: Vec<(u32, &HoardCmd)> = trove
        .commands
        .iter()
        .filter_map(|command| {
            let mut score = 0;
            let mut matched = 0;
            let mut strongest = 0;
            for word in words {
                let word_score = word_score(command, word);
                if word_score > 0 {
                    matched += 1;
                    score += word_score;
                    strongest = strongest.max(word_score);
                }
            }
            let hit = if require_all {
                matched == words.len()
            } else {
                matched > 0 && strongest >= TAG_CONTAINS
            };
            hit.then_some((score, command))
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.namespace.cmp(&b.1.namespace))
            .then_with(|| a.1.name.cmp(&b.1.name))
    });
    ranked
}

/// The best score one query word earns against one command.
fn word_score(command: &HoardCmd, word: &str) -> u32 {
    let name = command.name.to_lowercase();
    if name == word {
        return NAME_EXACT;
    }
    if name.contains(word) {
        return NAME_CONTAINS;
    }
    if command.tags.iter().any(|tag| tag.to_lowercase() == word) {
        return TAG_EXACT;
    }
    if command
        .tags
        .iter()
        .any(|tag| tag.to_lowercase().contains(word))
    {
        return TAG_CONTAINS;
    }
    if command.namespace.to_lowercase().contains(word) {
        return NAMESPACE_CONTAINS;
    }
    if command.description.to_lowercase().contains(word) {
        return DESCRIPTION_CONTAINS;
    }
    if command.command.to_lowercase().contains(word) {
        return COMMAND_CONTAINS;
    }
    0
}

/// What `hoard list --best` / `--raw` should print, and its exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BestOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Render the single best match for `query`.
///
/// `raw` prints the stored command alone (for piping into a shell), a JSON mode
/// one JSON object, and neither the usual table row. A miss prints nothing on
/// stdout, explains itself on stderr and asks for exit code 1 in human mode;
/// in a JSON mode it prints the structured error payload and asks for the
/// `CommandNotFound` exit code instead.
pub fn best(trove: &Trove, query: &str, mode: OutputMode, raw: bool) -> BestOutput {
    let Some((command, score)) = search_scored(trove, query).into_iter().next() else {
        let error = CliError::not_found(
            format!("no match for {query:?}"),
            crate::headless::suggest(query, trove.entry_keys()),
        );
        return BestOutput {
            stdout: if mode.is_json() {
                format!("{}\n", mode.serialize(&error.to_json()))
            } else {
                String::new()
            },
            stderr: if mode.is_json() {
                String::new()
            } else {
                format!("{}\n", error.message)
            },
            exit_code: error.exit_code(),
        };
    };
    // `--json` wins over `--raw`: json mode is a promise about the stream, and
    // a caller that asked for both gets the parseable one.
    let stdout = if mode.is_json() {
        let value = if mode.is_compact() {
            crate::core::trove::command_compact_json(command, Some(score))
        } else {
            crate::core::trove::command_json(command)
        };
        format!("{}\n", mode.serialize(&value))
    } else if raw {
        format!("{}\n", crate::headless::sanitize(&command.command))
    } else {
        format!(
            "{}\n",
            Trove::from_commands(std::slice::from_ref(command)).render_table()
        )
    };
    BestOutput {
        stdout,
        stderr: String::new(),
        exit_code: 0,
    }
}

#[cfg(test)]
mod test_filter {
    use super::*;

    fn command(name: &str, namespace: &str, source: &str, tags: &str) -> HoardCmd {
        HoardCmd::default()
            .with_name(name)
            .with_namespace(namespace)
            .with_command(source)
            .with_tags_raw(tags)
    }

    #[test]
    fn search_ranks_an_exact_name_above_a_command_body_hit() {
        let trove = Trove::from_commands(&[
            command("compose", "dev", "containers ps", ""),
            command("containers", "dev", "docker ps", ""),
        ]);
        let hits = search(&trove, "containers");
        assert_eq!(
            hits.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["containers", "compose"]
        );
    }

    #[test]
    fn search_prefers_a_curated_tag_over_a_name_substring() {
        let trove = Trove::from_commands(&[
            command(
                "git-diff-check-docs-kubernetes-handoff",
                "pi",
                "git diff --check",
                "git,vcs",
            ),
            command("pods", "pi", "kubectl get pods", "kubernetes,kubectl"),
        ]);
        assert_eq!(search(&trove, "kubernetes")[0].name, "pods");
    }

    #[test]
    fn search_requires_all_words_and_falls_back_to_any_word() {
        let trove = Trove::from_commands(&[
            command("status", "dev", "git status", "vcs"),
            command("compose", "dev", "docker compose up -d", "containers"),
        ]);
        let both = search(&trove, "git compose");
        // AND yields nothing, so any-word matching takes over — but only
        // structural hits qualify: `git status` matched "git" in its body
        // alone, which is not an answer.
        assert_eq!(
            both.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["compose"]
        );
        let only = search(&trove, "git status");
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].name, "status");
        assert_eq!(search(&trove, "vcs")[0].name, "status");
        assert_eq!(search(&trove, "").len(), 2);
    }

    #[test]
    fn best_raw_prints_only_the_command() {
        let trove = Trove::from_commands(&[command(
            "compose-up",
            "dev",
            "docker compose up -d",
            "containers",
        )]);
        let output = best(&trove, "compose", OutputMode::Human, true);
        assert_eq!(output.stdout, "docker compose up -d\n");
        assert_eq!(output.stderr, "");
        assert_eq!(output.exit_code, 0);
        // --json wins over --raw: a JSON stream stays parseable.
        let json = best(&trove, "compose", OutputMode::Json, true);
        assert_eq!(json.exit_code, 0);
        let payload: serde_json::Value = serde_json::from_str(json.stdout.trim()).unwrap();
        assert_eq!(payload["command"], "docker compose up -d");
        assert_eq!(json.stderr, "");
    }

    #[test]
    fn best_without_a_match_reports_command_not_found() {
        let trove = Trove::from_commands(&[command("status", "dev", "git status", "")]);
        let output = best(&trove, "kubernetes", OutputMode::Human, false);
        assert_eq!(output.exit_code, 2);
        assert_eq!(output.stdout, "");
        assert_eq!(output.stderr, "no match for \"kubernetes\"\n");
        assert_eq!(
            best(&Trove::default(), "", OutputMode::Human, true).exit_code,
            2
        );
    }

    #[test]
    fn best_miss_is_a_structured_error_in_json_mode() {
        let trove = Trove::from_commands(&[command("pods", "kube", "kubectl get pods", "")]);
        let output = best(&trove, "podz", OutputMode::JsonCompact, false);
        assert_eq!(output.exit_code, 2);
        assert_eq!(output.stderr, "");
        let json: serde_json::Value = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(json["ok"], false);
        assert_eq!(json["error_type"], "CommandNotFound");
        assert_eq!(json["suggested_keys"][0], "kube/pods");
    }

    #[test]
    fn scores_order_hits_and_survive_into_compact_json() {
        let trove = Trove::from_commands(&[
            command("compose", "dev", "containers ps", ""),
            command("containers", "dev", "docker ps", ""),
        ]);
        let scored = search_scored(&trove, "containers");
        assert_eq!(scored[0].0.name, "containers");
        assert!(scored[0].1 > scored[1].1);
        let compact = best(&trove, "containers", OutputMode::JsonCompact, false);
        let json: serde_json::Value = serde_json::from_str(&compact.stdout).unwrap();
        assert_eq!(json["k"], "containers");
        assert_eq!(json["s"], 100);
    }
}
