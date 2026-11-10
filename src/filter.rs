use crate::core::HoardCmd;
use crate::core::trove::Trove;

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
pub fn search<'a>(trove: &'a Trove, query: &str) -> Vec<&'a HoardCmd> {
    let words: Vec<String> = query
        .split_whitespace()
        .map(|word| word.to_lowercase())
        .collect();
    if words.is_empty() {
        return trove.commands.iter().collect();
    }

    let mut hits = ranked(trove, &words, true);
    if hits.is_empty() {
        hits = ranked(trove, &words, false);
    }
    hits.into_iter().map(|(_, command)| command).collect()
}

/// Score candidates in descending order; `require_all` selects AND or OR matching.
fn ranked<'a>(trove: &'a Trove, words: &[String], require_all: bool) -> Vec<(u32, &'a HoardCmd)> {
    let mut ranked: Vec<(u32, &HoardCmd)> = trove
        .commands
        .iter()
        .filter_map(|command| {
            let mut score = 0;
            let mut matched = 0;
            for word in words {
                let word_score = word_score(command, word);
                if word_score > 0 {
                    matched += 1;
                    score += word_score;
                }
            }
            let hit = if require_all {
                matched == words.len()
            } else {
                matched > 0
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
/// `raw` prints the stored command alone (for piping into a shell), `json` one
/// JSON object, and neither the usual table row. No match prints nothing on
/// stdout, explains itself on stderr and asks for exit code 1.
pub fn best(trove: &Trove, query: &str, json: bool, raw: bool) -> BestOutput {
    let Some(command) = search(trove, query).into_iter().next() else {
        return BestOutput {
            stdout: String::new(),
            stderr: format!("no match for {query:?}\n"),
            exit_code: 1,
        };
    };
    let stdout = if raw {
        format!("{}\n", command.command)
    } else if json {
        format!("{}\n", Trove::command_to_json(command))
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
        // AND yields nothing, so any-word matching takes over.
        assert_eq!(
            both.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["compose", "status"]
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
        let output = best(&trove, "compose", false, true);
        assert_eq!(output.stdout, "docker compose up -d\n");
        assert_eq!(output.stderr, "");
        assert_eq!(output.exit_code, 0);
        // --raw wins over --json.
        assert_eq!(best(&trove, "compose", true, true), output);
    }

    #[test]
    fn best_without_a_match_asks_for_exit_code_one() {
        let trove = Trove::from_commands(&[command("status", "dev", "git status", "")]);
        let output = best(&trove, "kubernetes", false, false);
        assert_eq!(output.exit_code, 1);
        assert_eq!(output.stdout, "");
        assert_eq!(output.stderr, "no match for \"kubernetes\"\n");
        assert_eq!(best(&Trove::default(), "", false, true).exit_code, 1);
    }
}
