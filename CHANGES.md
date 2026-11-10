# Release notes

All notable changes to this project will be documented in this file.
This project adheres to [Semantic Versioning](http://semver.org/).

## Unreleased
- 🤖 **Agent retrieval through `hoard list`**: `--json` now emits real JSON
  (`{version, commands:[{name, command, kind, description, namespace, tags,
  created}]}`, created as an ISO date or `null`, pretty-printed with two spaces)
  instead of YAML. New `--best` returns only the single highest-scoring match for
  `--filter`, and `--raw` prints just that command for piping into a shell. A miss
  prints nothing on stdout, a `no match for "<filter>"` line on stderr, and exits
  1. `hoard export` stays the YAML/trove-file path.
- 🏷️ History imports carry searchable tags: the program, a few synonyms and up to
  three intents, then `imported` and the source tag (lowercase, deduplicated, at
  most eight). Tags change findability only, not what is imported.
- ✨ `hoard history scan` and `hoard history import` mine commands you already ran
  from zsh history (`$HISTFILE`, else `~/.zsh_history`, extended format,
  multi-line entries and pasted commands included) and from pi / pi-bolt agent
  sessions (`bash` tool calls, written `.py` files, `ctx_execute` Python). `scan`
  is a read-only report of raw entries, candidates, duplicates, noise, redactions
  and secret-only skips (`--json` for machines); `import` is an interactive
  multi-select with `--all`, `--limit N` and `--dry-run`. Candidates are filed per
  source (`history` for shell history, `pi` for sessions, `--namespace` overrides
  both) and tagged `imported`. Navigation noise is filtered, secrets are replaced
  by `#name!` parameters, and an entry that is nothing but a secret is skipped; an
  optional batched trufflehog pass (`--trufflehog auto|off|PATH`) adds
  detector-grade redaction without aborting the import when the binary is missing
  or fails.
- 🧹 **Removed the `hoard sync` cloud feature** (this fork is local-only): the `sync
  register/login/logout/save/get/revert` subcommands, the sync client and its server
  calls, `sync_models.rs`, the `sync_server_url` and `api_token` config keys, the
  SQLite snapshot taken before `sync get` (`trove.db.bk`), plus what only existed to
  serve them (password prompts, the `rem_first_and_last` helper, `Db::backup_to` and
  the `password` cargo feature). Existing `config.yml` files that still carry the sync
  keys keep working — unknown keys are ignored and dropped on the next config write.
  Move a trove between machines with a YAML export in Git instead (see README).
- ✨ `hoard export` is scriptable: `--all` exports the whole trove and
  `--namespace a,b` a subset, both without the interactive namespace/command
  multi-selects, and both conflict with each other. The scripted path reports
  `Exported N entries to PATH` and fails with a non-zero exit for an unknown
  namespace, an empty selection or an unusable `--path`, where the old code printed
  `Nothing selected` and exited 0. Interactive `hoard export --path FILE` is
  unchanged.
- ✨ `hoard new` is scriptable: passing `--name` makes the save fully non-interactive
  (`hoard new -n NAME -c COMMAND [-d DESC] [-t TAGS] [--namespace NS]`). Values that
  are not passed fall back to the configured default namespace and to empty
  description/tags instead of prompting, an existing name/namespace pair is rejected
  instead of overwritten, and the entry is confirmed with `Saved [namespace/name]`.
  `--command` without `--name` still opens the interactive prompts, pre-filled.
- 🔒 Secrets are redacted before an entry is stored, whatever save path is used
  (`hoard new`, `hoard edit`, the TUI create/edit views): passwords in URLs and
  `-u user:pass`, `--password`/`--token`/`--api-key`/`--access-key`/`--secret` flags,
  `Authorization: Bearer ...` headers, `PASSWORD=`/`token:`-style assignments and query
  parameters, AWS/GitHub/Slack/OpenAI key shapes, JWTs and PEM private key blocks are
  replaced by `#password!`-style parameters. The save reports what was replaced (never
  the value itself). Shell expansions (`$TOKEN`, `$(pass show x)`), existing parameters
  and Python sources are left untouched, so redaction is idempotent.
- 🐛 A relative `trove_path` in `config.yml` is resolved against the hoard config
  directory instead of the current directory, so the global trove is reachable from any
  working directory again (previously every other directory silently got its own empty
  `./trove.db`). `trove.yml`/`trove.db` in the current directory still take precedence.
- 🧪 `tests/cli_new.rs` covers non-interactive saves, redaction, name/tag validation and
  duplicate rejection; `src/core/secrets.rs` unit-tests every rule, idempotence,
  deduplication and the Python-source exemption.
- 💾 Trove storage moved from YAML to a SQLite database (`trove.db`, WAL mode).
  Legacy `trove.yml` files are imported automatically on first run.
- 🧹 Refactored for idiomatic Rust (edition 2024): proper error types via `thiserror`,
  no more `unwrap()` in production paths, dead code and unused dependencies removed
  (`tokio`, `array_tool`, `h2`, `chrono`, `serde_json`, ...), blocking HTTP instead of
  a never-awaited async future (URL imports now actually work).
- 🚀 Performance: prepared statements in a single transaction per save, WAL + `NORMAL`
  sync, no full-file rewrites; smaller, faster builds (no tokio "full").
- 🎨 Dracula colorscheme as the new default theme (ratatui UI + dialoguer prompts).
- 🧪 `cargo clippy --all-targets -- -D warnings` now passes (it did not before).
- 📦 All dependencies updated to current major versions, with the code refactored to
  match: ratatui 0.30 + crossterm (replacing ratatui 0.22 + termion), dialoguer 0.12
  (truecolor Dracula prompts), reqwest 0.13, clap 4.6, comfy-table 8 (replacing the
  unmaintained prettytable-rs), rand 0.10, thiserror 2, dirs 7,
  `dotenv` → `dotenvy`, and the archived `chatgpt_blocking_rs` replaced with a direct
  OpenAI API call.

## 2.0.0
- ✨ Tracking how often a command is used or edited and some timestamps
- ✨ Sorting commands by their usage count
- ✨ Display usage count in TUI
- 🧹 Untangled a bunch of spaghetti code. Hopefully now much easier to digest.
- 🔧 Better error handling and better documentation
- 🧪  Bunch more testing
## 1.4.2
- 🐛 Fix a lifetime longevity issue around theming password prompts
## 1.4.1
- ✨ List available namespaces when creating a new command
- Move from tui to ratatui. Thanks [a-kenji](https://github.com/Hyde46/hoard/pull/292)
- 🐛 Fix a bunch of pipelines issues
## 1.4.0
- ✨ chatGPT integration. Generate commands which you have not yet hoarded
- Clean up code
- skip badly chosen release version tags
## 1.3.1
- 🐛 Fix bug, where `hoard pick` would not properly replace named parameterized commands
- Tab highlight now also uses customizable color from config file
- Added update checker on startup, to notify the user about a new version
## 1.3
- ✨ Inline command editing in the GUI. Press `<TAB>` to get started. Only editing the command, its description and the tags are supported for now
- ✨ Inline command deletion in the GUI. Press `<Ctrl-X>` to delete a command in the GUI view
- ✨ Inline command creation in the GUI. Press `<Ctrl-W>` to create a new command in the GUI view
## 1.2
- ✨ You can now synchronize your commands across multiple terminals! Run `hoard sync --help` to start
## 1.1.1
- ✨ Fully named parameters. When saving a parameter, you can now end the parameter name with `!`. This enables you to use spaces in the parameter name. Additionally, using the ending token `!` enables you to use a named parameter in a command where no space is between the parameter token and the rest of the command (Set a different token in your config)
- ✨ Add filtered commands outpout `hoard list --json --filter <query_string>` to enable easier downstream usage
- 👿 Temporarily disabled windows support. Switching the TUI backend to crossterm from termion broke zsh support on MacOS, which is deemed more important until a fix is prepared.
## 1.1.0
- ✨ Named Parameters. You can now add any string after your token `#`. Tokens with the same name will be filled out automatically after being prompted once for them when selecting a command from `hoard list`
## 1.0.1
- ✨ Read out local `trove.yml` file in current directory if available ( Edit ~/.config/hoard/config.yml `read_from_current_directory` to disable )
- ✨ `hoard set_parameter_token` to customize which parameter token is used
- ✨ `hoard info` shows where `config.yml` and `trove.yml` files are located
## 🚀 1.0.0
- ✨ Advanced export allowing subset of namespaces or commands to be exported
- 🐛 Fix bug where selecting a command when running `hoard` as a `zsh` plugin produces gibberish rendered text in the terminal
- ✨ Customizable GUI colors through ~/config/.hoard/config.yml
- ✨ Support parameterized commands. Put '#' in place where a parameter is expected. When running `hoard pick <command_name>` or `hoard list` as a shell plugin and selecting a parameterized command, `hoard` will ask for all missing parameters to input before sending the complete command to your shell input.
- ✨ Press `<F1>` when running `hoard list` to see all shortcuts
- ✨ Make tags optional

Breaking changes:
- 🔨 Find your hoard config files at `~/.config/hoard` (Used to be `~/.config/.hoard`. Copy them over or import your old trove file by running `hoard import ~/.config/.hoard/trove.yml`)

## 0.1.8
- Wrap command text in UI
- Left align command text in UI
- combine import url/path into one argument. Use `hoard import <path/url>`
- When picking a command not as a shell plugin, the command will be printed to the console
- `hoard pick` does not require `--name` argument anymore. To print a command run `hoard pick <name>`
- `hoard remove` does not require `--name` argument anymore. To remove a command run `hoard remove <name>`
- `hoard edit` does not require `--name` argument anymore. To remove a command run `hoard edit <name>`
- Add simple export functionality. Run `hoard export /path/to/exported/trove.yml`
- Enable short aliases for basic commands `new[n]`, `list[l]`, `copy[c]`, `pick[p]`, `remove[r]`, `import[i]`,  `export[x]`, `edit[e]`

## 0.1.7
- 🔧 Fix empty defaults for command field inputs

## 0.1.6
- Edit commands with `hoard edit --name <command_name>`
- Ask user for new command name if a collision is detected on creating a new command and importing troves. Name + namespace have to be unique.

## 0.1.5
- Import other trove files `hoard import --file /path/to/trove.yml`
- Import trove files from url `hoard import --url https://this.trove.com/trove.yml`
- Move config files to `.config/.hoard` instead of `.hoard`

## 0.1.4

- Can now delete all commands in a specific namespace `hoard remove --namespace <name>`
- Add 🐟 `fish` shell autocomplete support
- Add 🐟 `fish` installer script

## 0.1.3

- Strip autocompleted command of its leading spaces

## 0.1.2

- 🐛 Fix 'sending on a disconnected channel' bug when autocompleting

## 0.1.1

- Enable installation with cargo
- Generally improve installation script flow

## 0.1.0

- 🚀 Initial release. All basic features compoleted
- Add zsh support for autocomplete (zsh widget)
- Add Linux Ubuntu installer for bash and zsh support

## 0.2.0-beta

- Add bash support for autocomplete
- Rework list UI
- Add namespace tab
- Enable filtering commands in UI
- Replace crossterm UI backend with termion

## 0.1.2-beta

- When starting `hoard` the first time, it asks the user for a default namespace
- `hoard list` won't show UI anymore and break if no command has been saved before
- Show help if no command is supplied

## 0.1.1-beta

- Add command removal command

## 0.1.0-beta

Initial beta release
