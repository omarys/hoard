<p align="center">
<img src=img/hoard_icon.png width="400">
</p>

<p align="center">
  <a href="https://github.com/Hyde46/hoard/actions/workflows/test.yml">
    <img src="https://img.shields.io/github/actions/workflow/status/hyde46/hoard/test.yml?style=flat-square" />
  </a>
  <a href="https://crates.io/crates/hoard-rs"
    ><img
      src="https://img.shields.io/crates/v/hoard-rs?style=flat-square"
      alt="Crates.io version"
  /></a>
  <a href="https://github.com/Hyde46/hoard/issues"
    ><img
      src="https://img.shields.io/github/issues/Hyde46/hoard?style=flat-square"
      alt="Crates.io version"
  /></a>
  <a href="https://crates.io/crates/hoard-rs"
    ><img
      src="https://img.shields.io/github/license/Hyde46/hoard?style=flat-square"
      alt="Crates.io version"
  /></a>
</p>

<p align="center">
<em> command organizer tool to hoard all your precious commands 💎🐉</em>
</p>

![Example usage](img/hoard_usage_example.gif)

## ✨ What's new

- 🤖 **Headless mode for agents**: `--json` on every subcommand, plus `--compact`
  (`--tokens`) for a minified `{"k","c","d","p","s"}` projection, `hoard eval` for
  parameter substitution, `hoard schema` for the machine-readable contract, and one
  deterministic exit code per failure class. See *Headless contract (agents)*.
- 🧩 **Templated commands**: `{{name}}` and `{{name=default}}` placeholders next to the
  legacy `#name!` markers; `hoard eval <name> -p k=v` renders an entry without running it.
- 🧹 The `hoard sync` cloud feature is gone: no account, no sync server, no
  `sync_server_url`/`api_token` in the config. Move the trove between machines with a
  YAML export in a private repository (see *Synchronize commands with another terminal*).
- 📤 `hoard export` is scriptable: `--all` exports every entry, `--namespace a,b`
  a subset, both without prompts. A bad path or an unknown namespace now fails
  instead of reporting `Nothing selected`.
- Archive Python files with `hoard new --script FILE -n NAME -d SUMMARY`.
  Source and descriptions stay in the trove. Use `hoard pick -n NAME --raw`
  to recover the original source, or select an entry for a Python 3 invocation.
- 💾 Commands are stored in a **SQLite database** (`trove.db`) instead of a YAML file —
  faster writes, single-row updates, WAL journaling. Legacy `trove.yml` files are
  imported automatically on first run; YAML remains only for import and export.
- 🎨 **Dracula colorscheme** as the default theme (ratatui UI + prompt dialogs).
- 📦 All dependencies bumped to current versions: ratatui 0.30 + crossterm (terminal
  backend), dialoguer 0.12, reqwest 0.13, clap 4.6, comfy-table 8, rand 0.10, thiserror 2.
- 🧹 Idiomatic Rust refresher: edition 2024, `thiserror` error types, no `unwrap()` in
  production paths, dead code and abandoned dependencies removed (the GPT integration
  now calls the OpenAI HTTP API directly instead of a dead wrapper crate).
- 🧪 `cargo clippy --all-targets -- -D warnings` and 60+ tests pass; a `pre-commit`
  config runs fmt / check / clippy / tests on every commit.

Full history in [CHANGES.md](CHANGES.md).

#### What is a command organizer?

A command organizer lets you save commands that you often use, but are too complicated or long to remember.
For every **hoarded** command, `hoard` saves

- the command ( parameterized with a customizable token, default `#` )
- name
- description
- namespace where it lives in
- tags ( Optional )

If you get lost in your massive command history, and can't find for example a specific `docker` command out of thousand `docker` commands you've already ran,
just **hoard** it. With a **name** and **description** it will be much easier to find again. When you look for that command again a month later, take a look at your **hoarded** commands.

`hoard` is **not** supposed to replace shell history finder like `fzf` `atuin` or similar utilities. It rather should be used in conjunction with them.

Now with chatGPT integration 💎

## :love_letter: Table of contents

- [📦 Install](#install)
- [🤸 Usage](#usage)
- [:zap: Shortcuts](#shortcuts)

<a name="install"/>

## 📦 Install

### From source

It's best to use [rustup](https://rustup.rs/) to get setup with a Rust
toolchain, then you can run:

```
cargo install hoard-rs
```

Or build it yourself:

```
cargo build --release
```

Find the binaries in `./target/release/hoard`
Move it to wherever you need it ( Like `/usr/local/bin/hoard` )
Or run

```
cargo install --path .
```

### Linux

Tested on:

- Ubuntu
- Arch
- EndeavourOS

Install `hoard` by running

```
./install.sh
```

If you are running `fish` shell

```
LATEST_RELEASE=1.4.2 ./install.fish
```

### AUR (Arch Linux)

```
paru -S hoard
```

### Homebrew (macOS)

```
brew tap Hyde46/hoard
brew install hoard
```

### MacPorts (macOS)

```
sudo port install hoard-cli
```

More info [here](https://ports.macports.org/port/hoard-cli/)

### Windows

Best to install from source, good luck

## Autocomplete with Shell plugin

Install `hoard` as a plugin to enable autocomplete.
Depending on your shell, run one of the following commands.
To keep it installed for your next shell session, add the `source` command with an absolute path to your `.bashrc` or copy-paste the plugins content to your `.bashrc`.

#### bash

```
source src/shell/hoard.bash
```

#### zsh

```
source src/shell/hoard.zsh
```

#### fish

```
source src/shell/hoard.fish
```

### Nix

The hoard package is in the [nixpkgs](https://search.nixos.org/packages?channel=unstable&from=0&size=50&sort=relevance&type=packages&query=hoard) package repository.

Either install it with `nix-env -iA hoard`, get it temporarily with `nix-shell -p hoard` or add it to your configuration.

<a name="usage"/>

## 🤸 Usage

#### Save a new command

```
hoard new
```

If a parameter is not known when saving the command, put a `#` ( Or your customized token from your `~/.config/hoard/config.yml` )
You can also name your parameters like this:

```
echo "My name is #first and I live at #city. Did I tell you my name, #first?"
```

When putting `#first` you only have to do it once for each occurrence in the command.
A parameter name is defined as everything followed by the token until the first space character is found.
Alternatively you can determine where the named parameter ends by putting a `!` ( Or your customized token from your `~/.config/hoard.config.yml`)

```
echo "My name is #first named parameter! and I live at #city. Did I tell you my name, #first?"
```

#### Save a Python script

```sh
hoard new --script ./summarize-jsonl.py \
  --name summarize-jsonl \
  --namespace data \
  --tags python,jsonl,report \
  --description "Summarize event counts from a JSONL log. Takes a file path and prints a JSON report. Requires Python 3."
```

After initial Hoard setup, this saves without prompts. `--name` and a nonblank
`--description` are required. `--summary` is an alias for `--description`.
The namespace defaults to your configured default, and tags are optional.
`--script` and `--command` cannot be combined. An existing entry with the same
name and namespace is rejected rather than overwritten.

Hoard copies the entire UTF-8 source into SQLite with `kind: python`, preserving
indentation, comments, quotes and trailing newlines. It does not keep a reference
to the original file, execute it, check Python syntax, or install dependencies.
Include a purpose summary, inputs, outputs, dependencies and any destructive
effects in the description. Only the script itself is archived, not sibling
modules, data files or virtual environments.

Validate syntax without running the script before saving:

```sh
python3 -c 'import pathlib, sys; p = pathlib.Path(sys.argv[1]); compile(p.read_bytes(), str(p), "exec")' ./summarize-jsonl.py
```

Python comments and strings are never treated as Hoard parameters, even with a
custom token. Use `argparse`, `sys.argv` or environment variables for inputs
instead of inserting `#parameter!` placeholders into Python source.

Inspect or restore the exact source:

```sh
hoard pick --name summarize-jsonl --raw
# Choose a destination that does not already exist.
hoard pick --name summarize-jsonl --raw > ./summarize-jsonl.saved.py
```

Inspect the summary and source together with
`hoard list --json --filter summarize-jsonl`. The structured listing is JSON
(see *Agent retrieval* below), while `hoard export` remains the YAML/trove-file
path. `hoard list --simple` and the interactive list also show descriptions.

Selecting a Python entry in the shell plugin, or using `hoard pick -n NAME`
without `--raw`, prints a single-line `python3 -c` invocation. Hoard does not run
it. The invocation encodes the source to preserve quoting across Bash, Zsh and
Fish. You can append arguments, and stdin remains available to the script.
For large scripts that exceed command-line size limits, or scripts that depend
on `__file__`, restore a `.py` file with `--raw` and run that file instead.

`hoard edit --name summarize-jsonl` opens the source in `$VISUAL` or `$EDITOR`,
then prompts for its description and tags. SQLite backups and YAML
import/export preserve the script kind and summary. Existing entries without a
kind remain shell commands. Use this updated fork on each machine that reads
Python entries; older clients do not understand the Python kind and may lose it
when saving.

#### Search through command trove

```
<Ctrl-h>
```

Or alternatively, if not installed as a plugin, the interactive search can still be performed, though without autocomplete. This assumes the user to copy the command by mouse from the UI

```
hoard list
```

When running `hoard list` as a shell plugin and selecting a parameterized command, `hoard` will ask for all missing parameters to input before sending the complete command to your shell input.

Commands are stored in a local SQLite database at `~/.config/hoard/trove.db` (WAL mode, tuned for low-latency single-user writes).
If there is a `trove.db` file present in the local directory, `hoard` will only load this database and not display your "global" trove! A legacy `trove.yml` next to the database is imported automatically on first run.
( Edit ~/.config/hoard/config.yml `read_from_current_directory` to disable )

#### Import history

`hoard history` finds commands you already ran and never hoarded. It reads zsh
history (`$HISTFILE`, else `~/.zsh_history`, extended format included) and the
tool calls of your pi / pi-bolt agent sessions (`~/.pi/agent/sessions` and
`~/.pi-bolt/agent/sessions`), then offers the candidates.

```sh
# Read-only report: what would be imported, and what was filtered out.
hoard history scan
hoard history scan --source zsh --json

# Interactive multi-select, then save the chosen entries.
hoard history import

# Without prompts; --limit caps the count and --dry-run writes nothing.
hoard history import --all --limit 20 --dry-run
hoard history import --all
```

Entries are filed by origin — shell history into the `history` namespace, agent
session commands into `pi` — and tagged `imported` plus the source name. Pass
`--namespace NS` to override both. Names are slugified from the first line, so a
multi-line command is stored whole under a name from its first line.

Navigation-only commands (`cd`, `ls`, `pwd`, `clear`, ...) and duplicates are
dropped. Secrets found in a candidate are replaced by `#name!` parameters, the
same convention `hoard run` substitutes, and an entry that is nothing but a
secret is skipped. `--trufflehog auto` (the default) adds a second pass through
[trufflehog](https://github.com/trufflesecurity/trufflehog) when it is on `PATH`;
use `off` to rely on the built-in rules only, or pass a path to a specific
binary. A missing or failing scanner is reported as a warning and the import
continues.

Imported entries are also tagged for retrieval: the program they run, a few
synonyms (`kubectl` → `kubernetes`, `k8s`), and up to three intents
(`secrets`, `network`, `deploy`, ...). Intents include tool families such as
`kubernetes`, `containers` and `aws`, so a command run through a wrapper
(`rtk kubectl logs ...`) still answers a search for its tool. Tags end with
`imported` and the source tag. Intent words must stand alone: `cargo clippy` is `test`, `--no-check-certificate`
is not, and `log-collection` is not a log command. A leading `cd <path>` is
context rather than the program, so `cd /tmp && cargo build` is tagged `cargo`.
Tags are lowercase and deduplicated and an entry
never carries more than eight of them. Tags only affect searching: they never
change which candidates are imported.

#### Agent retrieval

`hoard list` is the machine-readable retrieval path:

```sh
# JSON: {version, commands:[{name, command, kind, description, namespace,
# tags, created, params}]}, pretty-printed with two spaces, best match first.
hoard list --json
hoard list --json --filter kubernetes

# The single best match, as a table row or as one JSON object.
hoard list --best --filter deploy
hoard list --best --json --filter deploy

# Only the stored command, so an agent can pipe it into a shell.
hoard list --raw --filter deploy
```

Ranking scores a name hit highest, then an exact tag, a name substring, a tag
substring, the namespace, the description and the command body, so a curated tag
beats an incidental mention in a longer name. A multi-word filter requires every
word to match somewhere and falls back to any-word matching when that finds
nothing — but the fallback only keeps structural hits (name, tag or namespace),
so a command that merely mentions both words in its body reports `no match`
instead of handing an agent the wrong line to run. Ties break on
`(namespace, name)`. Commands found by a filter are
returned in that ranked order, and `--best` takes the first. When nothing
matches, `--best` and `--raw` print nothing on
stdout, explain the miss on stderr and exit with status 2 (`CommandNotFound`). `--raw` implies
`--best`; when both `--raw` and `--json` are given, `--json` wins so the stream stays
parseable. `hoard export` remains the YAML/trove-file path;
`list --json` is not a trove file.

#### Headless contract (agents)

Everything an agent needs is reachable without a terminal. Add `--json` (`-j`) to any
subcommand; `--compact` (alias `--tokens`) implies it and switches to a minified,
single-line payload.

```sh
# Dense retrieval: {"commands":[{"k":..,"c":..,"d":..,"p":[..],"s":..}]}
# `d` is omitted when empty, `p` when the command has no placeholders, and `s`
# (a lexical score, highest first) only appears on a scored query.
hoard list --json --compact --filter kubernetes
hoard list --best --json --compact --filter deploy

# Render an entry without running it. Human mode prints the command alone.
# --json: {ok, name, command, kind, namespace, params} with descriptive keys;
# --compact: {ok, k, c, p}. A repeated key is rejected, not last-wins.
hoard eval logs -p container_id=web
hoard eval logs -p container_id=web --json
hoard eval logs -p container_id=web --json --compact

# The contract itself, so a model can build calls instead of guessing.
hoard schema --json --compact
```

`{{name}}` marks a required placeholder, `{{name=default}}` an optional one; both are
derived from the stored template, so no parameter list can drift out of sync. `eval`
renders the legacy `#name!` markers as well, which keeps a trove written before
placeholders existed usable. A Python entry is source, not a shell template: it has no
parameters, `eval` returns its source untouched (a `{{braces}}` in a script is source
text) and passing `-p` to one is an `InvalidParameter`.

Failures are data, never prose:

```json
{
  "ok": false,
  "status": "error",
  "error_type": "InvalidParameter",
  "message": "missing required parameter(s): container_id",
  "missing_params": ["container_id"],
  "suggested_keys": ["container_id"]
}
```

| exit | `error_type` | meaning |
| --- | --- | --- |
| 0 | — | success |
| 1 | `ParseError` | the invocation itself is invalid (bad flag, missing argument) |
| 2 | `CommandNotFound` | the entry or query matched nothing |
| 3 | `InvalidParameter` | missing, unknown or malformed parameter |
| 4 | `RuntimeError` | I/O, database or validation failure |

In `--json` mode the payload goes to **stdout** even on failure, so a caller parses one
document per call. Without it the message stays on **stderr** and stdout stays empty.
ANSI colors and cursor sequences are stripped whenever stdout is not a TTY or `--json`
is given, so `hoard pick -n NAME --raw` and `hoard list --compact` are safe to pipe.

`pick` returns the **stored source** (placeholders intact); `eval` is the step that
renders it. That split keeps retrieval one call and execution one call, with no
interactive prompt in between.

Subcommands that need a person never guess in headless mode: `edit` refuses with
`InvalidParameter` (exit 3), and `history import` requires an explicit `--all` (or
`--dry-run`) instead of importing everything behind the caller's back. `new` prints its
`{ok, saved, k, ns, c, p}` payload only *after* the trove was written, so a payload on
stdout is always a claim that survives the process.

#### chatGPT integration

When running `hoard list` or through the hoard shortcut, press `Ctrl-a` to be asked for a prompt.
Desribe what your command is supposed to do, and chatGPT will create and hoard a new command based on your prompt.

If you dont already have an OpenAI account to generate an API key, head to [this](https://platform.openai.com/account/api-keys) page. ( Given the low amount of tokens consumed, this is extremly cheap. ~ 0.01$ for 30 generated commands )

You either need to set the env variable

```bash
export OPENAI_API_KEY=abcabcabc
```

Or for a permanent solution set `gpt_api_key` in your `~/.config/hoard/config.yml`

chatGPT does not always reply in the same format. Sometimes the resulting command will be added to the description. Use the edit mode `<TAB>/<Ctrl-E>` to edit the hoarded command to its correct form.

#### Synchronize commands with another terminal

For a private, reviewable archive, keep a YAML export in a private Git repository. This fork preserves shell commands, Python source, entry kinds, descriptions, and tags in the export:

```sh
# On the source system. Exports every entry, Python sources included.
hoard export --all --path ./trove.yml
git add trove.yml
git commit -m "Update Hoard entries"
git push
```

`--all` and `--namespace` select entries without prompting, so the export can run
unattended: use `hoard export --namespace kubernetes,git --path ./trove.yml` for a
subset. (For a one-off filtered archive, `hoard export --namespace a,b --path
./trove.yml` is the YAML path; `hoard list --json --filter <query>` emits JSON
for agents, not a trove file.)

Inspect the export for credentials and other sensitive values before committing it. On another system:

```sh
git pull
hoard import --uri ./trove.yml
```

Prefer one system writing the export at a time. YAML is text, so changes can be reviewed and merged. Do not track `trove.db` in Git: SQLite is binary, produces poor diffs and merges, and may have uncheckpointed data in `trove.db-wal`. Copying a stopped Hoard database is suitable for a one-time migration, but YAML is the better format for ongoing source-controlled synchronization.

#### Delete a command

```
hoard remove <name>
```

#### Delete all commands in a namespace

```
hoard remove_namespace <namespace_name>
```

#### Edit a command

```
hoard edit <name>
```

#### Info

Shows location of config file and trove database

```
hoard info
```

#### Colorscheme

hoard ships with the [Dracula](https://draculatheme.com) colorscheme. The colors used by the
interactive UI are configurable via `primary_color`, `secondary_color`, `tertiary_color` and
`command_color` in `~/.config/hoard/config.yml`.

#### Set parameter token

Change parameter token of config file

```
hoard set_parameter_token <parameter_token>
```

#### Import other trove files from `trove.yml` or urls pointing to a trove.yml file

```
hoard import /path/to/trove.yml
```

or

```
hoard import https://troves.com/new_trove.yml
```

#### Export trove file

```
hoard export /path/to/exported/trove.yml
```

<a name="shortcuts"/>

## :zap: Hoard list shortcuts

Show list of commands in the GUI

```
<F1>
```

Next item in command list

```
<Ctrl-N> / <Down-Arrow>
```

Previous item in command list

```
<Ctrl-P> / <Ctrl-Y> / <Up-Arrow>
```

Next namespace tab

```
<Ctrl-L> / <Right-Arrow>
```

Previous namespace tab

```
<Ctrl-H> / <Left-Arrow>
```

Select command

```
<Enter>
```

Quit

```
<Esc> / <Ctrl-D> / <Ctrl-C> / <Ctrl-G>
```
