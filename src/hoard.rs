use crate::cli_commands::{Cli, Commands, HistoryCommands};
use crate::config::defaults;
use crate::config::{HoardConfig, load_or_build_config, save_parameter_token};
use crate::core::secrets::Redacted;
use crate::core::template;
use crate::core::trove::Trove;
use crate::core::{CommandKind, HoardCmd};
use crate::filter::{best, search_scored};
use crate::gui::commands_gui;
use crate::gui::prompts::prompt_multiselect_options;
use crate::headless;
use crate::headless::{CliError, ErrorType, OutputMode};
use crate::history;
use crate::schema;
use crate::store::Db;
use anyhow::{Context, Result, bail, ensure};
use dotenvy::dotenv;
use reqwest::Url;
use serde_json::json;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The hoard application: configuration, the in-memory trove and its
/// SQLite-backed persistence.
pub struct Hoard {
    config: HoardConfig,
    trove: Trove,
    db: Db,
    /// How this invocation shapes its output.
    mode: OutputMode,
}

impl Hoard {
    /// Loads configuration, opens the trove database and loads all commands.
    pub fn load(config_home_path: Option<String>, mode: OutputMode) -> Result<Self> {
        dotenv().ok();
        let config = load_or_build_config(config_home_path)?;
        let db_path = config
            .trove_path
            .as_deref()
            .context("trove path is not configured")?;
        let db = Db::open(db_path)?;
        let trove = Trove::from_commands(&db.commands()?);
        Ok(Self {
            config,
            trove,
            db,
            mode,
        })
    }

    /// Runs the parsed CLI command and returns the selected command string
    /// (if any) together with whether it is meant for shell autocomplete.
    pub fn start(&mut self, cli: Cli) -> std::result::Result<(String, bool), CliError> {
        let mut autocomplete_command = String::new();

        match cli.command {
            Commands::Info {} => self.show_info(),
            Commands::New {
                name,
                tags,
                command,
                description,
                script,
                namespace,
            } => self.new_command(name, tags, command, description, script, namespace)?,
            Commands::List {
                filter,
                simple,
                best,
                raw,
            } => {
                autocomplete_command = self
                    .list_commands(simple, filter, best, raw)?
                    .unwrap_or_default()
            }
            Commands::Pick { name, raw } => self.pick_command(&name, raw)?,
            Commands::Eval {
                name,
                param,
                dry_run: _,
            } => self.eval_command(&name, &param)?,
            Commands::Schema {} => self.schema_command(),
            Commands::Remove { name } => self.remove_command(&name)?,
            Commands::RemoveNamespace { namespace } => self.remove_namespace(&namespace)?,
            Commands::SetParameterToken { name } => self.set_parameter_token(&name)?,
            Commands::Import { uri } => self.import_trove(&uri)?,
            Commands::Export {
                path,
                all,
                namespace,
            } => self.export_command(&path, all, &namespace)?,
            Commands::Edit { name } => self.edit_command(&name)?,
            Commands::ShellConfig { shell } => Self::shell_config_command(&shell),
            Commands::History { action } => self.history_command(action)?,
        }

        Ok((autocomplete_command, cli.autocomplete))
    }

    /// Resolve an entry named `spec` (`name` or `namespace/name`), failing with
    /// the `CommandNotFound` payload and probable alternatives.
    fn resolve_entry(&self, spec: &str) -> std::result::Result<&HoardCmd, CliError> {
        self.trove.resolve(spec).ok_or_else(|| {
            let needle = spec.rsplit('/').next().unwrap_or(spec);
            CliError::not_found(
                format!("no entry named '{spec}'"),
                crate::headless::suggest(needle, self.trove.entry_keys()),
            )
        })
    }

    /// Persists the in-memory trove to SQLite.
    fn persist(&self) -> Result<()> {
        self.db.sync(&self.trove.commands)
    }

    /// Prints the paths of the config file and the trove database.
    pub fn show_info(&self) {
        if let Some(config_home_path) = &self.config.config_home_path {
            println!(
                "🔧 Config file is located at {}",
                config_home_path.display()
            );
        }
        if let Some(trove_path) = &self.config.trove_path {
            println!("✨ Trove database is located at {}", trove_path.display());
        }
    }

    fn parameter_token(&self) -> &str {
        self.config
            .parameter_token
            .as_deref()
            .unwrap_or(defaults::PARAMETER_TOKEN)
    }

    fn parameter_ending_token(&self) -> &str {
        self.config
            .parameter_ending_token
            .as_deref()
            .unwrap_or(defaults::PARAMETER_ENDING_TOKEN)
    }

    /// `hoard history scan` and `hoard history import`.
    fn history_command(&mut self, action: HistoryCommands) -> std::result::Result<(), CliError> {
        let token = self.parameter_token().to_string();
        let ending = self.parameter_ending_token().to_string();
        match action {
            HistoryCommands::Scan { args } => {
                let options = history::Options::from_args(&args);
                let collection = history::Collected::new(&self.trove, &options, &token, &ending);
                for warning in &collection.warnings {
                    eprintln!("⚠️  {warning}");
                }
                if self.mode.is_json() {
                    let payload: serde_json::Value =
                        serde_json::from_str(&collection.report_json()).map_err(|err| {
                            CliError::new(ErrorType::ParseError, format!("bad scan report: {err}"))
                        })?;
                    println!("{}", self.mode.serialize(&payload));
                } else {
                    print!("{}", collection.report());
                }
            }
            HistoryCommands::Import {
                args,
                namespace,
                all,
                dry_run,
            } => {
                // Importing by prompt is the only path that changes the trove on
                // a maybe. Headless there is no prompt, so require an explicit
                // `--all` (or a `--dry-run` preview) instead of taking
                // everything behind the caller's back. Checked before reading
                // any history so a refusal costs nothing.
                if !all && !dry_run && (self.mode.is_json() || !headless::stdin_is_tty()) {
                    return Err(CliError::new(
                        ErrorType::InvalidParameter,
                        "`history import` needs --all (or --dry-run) without a terminal",
                    )
                    .with_suggestions(vec!["--all".to_string(), "--dry-run".to_string()]));
                }
                let options = history::Options::from_args(&args);
                let collection = history::Collected::new(&self.trove, &options, &token, &ending);
                for warning in &collection.warnings {
                    eprintln!("⚠️  {warning}");
                }
                let import_options = history::ImportOptions {
                    namespace,
                    all,
                    dry_run,
                    quiet: self.mode.is_json(),
                };
                let outcome = history::import(&mut self.trove, collection, &import_options)?;
                if outcome.must_persist() {
                    self.persist()?;
                }
                if self.mode.is_json() {
                    println!("{}", self.mode.serialize(&outcome.report_json()));
                }
            }
        }
        Ok(())
    }

    fn new_command(
        &mut self,
        name: Option<String>,
        tags: Option<String>,
        command: Option<String>,
        description: Option<String>,
        script: Option<PathBuf>,
        namespace: Option<String>,
    ) -> Result<()> {
        // Without a terminal there is nobody to answer the guided prompts, and
        // dialoguer would fill in defaults over the caller's intent. Name what
        // is missing instead, so an agent can repair its call in one round trip.
        if self.mode.is_json() || !headless::stdin_is_tty() {
            // `(key, how to pass it)` — the key feeds `missing_params`, the hint
            // makes the message actionable without reading `--help`.
            let mut missing: Vec<(&str, &str)> = Vec::new();
            if name.as_deref().is_none_or(|name| name.trim().is_empty()) {
                missing.push(("name", "--name"));
            }
            match (&script, &command) {
                (None, None) => missing.push((
                    "command",
                    "--command, or omit --name to be prompted for every value",
                )),
                (Some(_), _) => {
                    if description
                        .as_deref()
                        .is_none_or(|text| text.trim().is_empty())
                    {
                        missing.push(("description", "--description"));
                    }
                }
                (None, Some(_)) => {}
            }
            if !missing.is_empty() {
                let named: Vec<String> = missing
                    .iter()
                    .map(|(key, hint)| format!("{key} ({hint})"))
                    .collect();
                let keys: Vec<String> = missing.iter().map(|(key, _)| key.to_string()).collect();
                return Err(CliError::new(
                    ErrorType::InvalidParameter,
                    format!("missing required input(s): {}", named.join(", ")),
                )
                .with_suggestions(keys.clone())
                .invalid_parameter(keys)
                .into());
            }
        }
        if let Some(path) = script {
            let name = name.context("Python scripts require --name")?;
            let description = description.context("Python scripts require --description")?;
            ensure!(
                !description.trim().is_empty(),
                "Python scripts require a non-empty description/summary"
            );
            HoardCmd::is_name_valid(&name)?;
            let source = std::fs::read_to_string(&path)
                .with_context(|| format!("Could not read Python script {}", path.display()))?;
            HoardCmd::is_command_valid(&source)?;
            let entry = HoardCmd {
                name,
                command: source,
                kind: CommandKind::Python,
                description,
                namespace: namespace.unwrap_or_else(|| self.config.default_namespace.clone()),
                ..HoardCmd::default().with_tags_raw(&tags.unwrap_or_default())
            };
            if self.trove.get_command_collision(&entry).is_some() {
                // A taken name is a bad argument, not a runtime fault: exit 3
                // tells a caller to fix the call and retry.
                return Err(CliError::new(
                    ErrorType::InvalidParameter,
                    format!(
                        "An entry named '{}' already exists in namespace '{}'",
                        entry.name, entry.namespace
                    ),
                )
                .with_suggestions(vec!["--name <other>".to_string(), "edit".to_string()])
                .into());
            }
            self.trove.add_command(entry.clone(), false)?;
            self.persist()?;
            if self.mode.is_json() {
                // Same payload as the shell path, so one parser covers both.
                println!(
                    "{}",
                    self.mode.serialize(&json!({
                        "ok": true,
                        "saved": format!("{}/{}", entry.namespace, entry.name),
                        "k": entry.name,
                        "ns": entry.namespace,
                        "c": entry.command,
                    }))
                );
            } else {
                println!("Saved [{}/{}]", entry.namespace, entry.name);
            }
            return Ok(());
        }

        let token = self.parameter_token().to_string();
        let ending_token = self.parameter_ending_token().to_string();

        // Providing `--name` selects a non-interactive save: every value that is
        // not passed falls back to a default instead of prompting, so
        // `hoard new -n NAME -c COMMAND` is safe to script. Without `--name`
        // hoard prompts as usual, pre-filling whatever was passed on the CLI.
        let scripted = name.is_some();
        let new_command = match (name.as_deref(), command) {
            (Some(name), Some(command)) => {
                HoardCmd::is_name_valid(name)?;
                HoardCmd::default().with_command(&command)
            }
            (Some(_), None) => bail!(
                "`hoard new --name` also needs --command. Omit --name to be prompted for every value."
            ),
            (None, command) => {
                HoardCmd::default().with_command_string_input(command, &token, &ending_token)
            }
        };
        let (new_command, redacted) = new_command.with_redacted_secrets(&token, &ending_token);
        report_redactions(&redacted);

        let new_command = match (scripted, namespace) {
            (true, namespace) => new_command.with_namespace(
                namespace
                    .as_deref()
                    .unwrap_or(&self.config.default_namespace),
            ),
            (false, Some(namespace)) => new_command.with_namespace(&namespace),
            (false, None) => new_command.with_namespace_input(&self.trove.namespaces()),
        };
        let new_command = match name {
            // Already validated above.
            Some(name) => new_command.with_name(&name),
            None => new_command.with_name_input(None, &self.trove),
        };
        let new_command = match (scripted, description) {
            (true, description) => {
                new_command.with_description(description.as_deref().unwrap_or_default())
            }
            (false, description) => {
                new_command.with_description_input(description.unwrap_or_default())
            }
        };
        let new_command = match tags {
            Some(tags) => {
                ensure!(
                    !tags
                        .split(',')
                        .any(|tag| tag.trim().contains(char::is_whitespace)),
                    "Tags can't contain whitespaces"
                );
                new_command.with_tags_raw(&tags)
            }
            // A scripted save without --tags stores no tags.
            None if scripted => new_command,
            None => new_command.with_tags_input(None),
        };

        // A scripted save must fail loudly instead of replacing an entry.
        if scripted && let Some(existing) = self.trove.get_command_collision(&new_command) {
            return Err(CliError::new(
                ErrorType::InvalidParameter,
                format!(
                    "An entry named '{}' already exists in namespace '{}'. Use `hoard edit` or a different name.",
                    existing.name, existing.namespace
                ),
            )
            .with_suggestions(vec!["--name <other>".to_string(), "edit".to_string()])
            .into());
        }

        let saved = format!("{}/{}", new_command.namespace, new_command.name);
        // Persist first: a payload on stdout is a claim that the trove changed.
        self.trove.add_command(new_command.clone(), true)?;
        self.persist()?;
        if self.mode.is_json() {
            // Echo what was stored (the redacted source, the parameter list) so
            // an agent needs no follow-up `list` round trip.
            let params = template::param_names_with(
                &new_command.command,
                self.parameter_token(),
                self.parameter_ending_token(),
            );
            let mut payload = json!({
                "ok": true,
                "saved": saved,
                "k": new_command.name,
                "ns": new_command.namespace,
                "c": new_command.command,
                "redactions": redacted.redactions.len(),
            });
            if !params.is_empty() {
                payload["p"] = json!(params);
            }
            println!("{}", self.mode.serialize(&payload));
        } else {
            println!("Saved [{saved}]");
        }
        Ok(())
    }

    fn list_commands(
        &mut self,
        is_simple: bool,
        filter: Option<String>,
        is_best: bool,
        is_raw: bool,
    ) -> Result<Option<String>> {
        // `--raw` is `--best` without the decoration. Both print from here so
        // the stored command reaches stdout byte for byte (`main` trims).
        if is_best || is_raw {
            let query = filter.unwrap_or_default();
            let output = best(&self.trove, &query, self.mode, is_raw);
            if !output.stdout.is_empty() {
                let mut stdout = std::io::stdout().lock();
                stdout.write_all(output.stdout.as_bytes())?;
                stdout.flush()?;
            }
            if !output.stderr.is_empty() {
                eprint!("{}", output.stderr);
            }
            if output.exit_code != 0 {
                std::process::exit(output.exit_code);
            }
            return Ok(None);
        }
        if self.trove.is_empty() {
            if self.mode.is_json() {
                return Ok(Some(self.render_trove(&Trove::from_commands(&[]), None)));
            }
            println!("No command hoarded.\nRun [ hoard new ] first to hoard a command.");
        } else if self.mode.is_json() {
            // Structured output for agents, ranked best-first so the first
            // element is the answer. `compact` keeps only the keys a model
            // needs (`k`/`c`/`d`/`p`) and adds the match score `s`.
            let query = filter.unwrap_or_default();
            let hits = search_scored(&self.trove, &query);
            let scores: Vec<u32> = hits.iter().map(|(_, score)| *score).collect();
            let commands: Vec<HoardCmd> =
                hits.iter().map(|(command, _)| (*command).clone()).collect();
            let trove = Trove::from_commands(&commands);
            return Ok(Some(self.render_trove(&trove, Some(&scores))));
        } else if is_simple {
            self.trove.print_trove();
        } else if headless::stdout_is_tty() {
            match commands_gui::run(&mut self.trove, &self.config) {
                Ok(selected_command) => {
                    self.persist()?;
                    if let Some(command) = selected_command
                        && !command.command.is_empty()
                    {
                        return Ok(Some(command.shell_command()));
                    }
                }
                Err(err) => println!("{err}"),
            }
        } else {
            // Without a terminal there is nothing to paint: show the table an
            // agent or a pipe can read instead of failing in the TUI.
            self.trove.print_trove();
        }
        Ok(None)
    }

    /// The trove as JSON, in the projection this invocation asked for.
    fn render_trove(&self, trove: &Trove, scores: Option<&[u32]>) -> String {
        if self.mode.is_compact() {
            trove.to_compact_json(scores)
        } else {
            trove.to_json()
        }
    }

    fn pick_command(&self, name: &str, raw: bool) -> std::result::Result<(), CliError> {
        // Resolve first: an unknown name is CommandNotFound (exit 2) no matter
        // which output shape the caller asked for.
        let resolved = self.resolve_entry(name)?;
        // `--json` wins over `--raw`: a JSON stream stays parseable, whichever
        // flags were combined.
        if self.mode.is_json() {
            println!(
                "{}",
                self.mode.serialize(&json!({
                    "ok": true,
                    "name": resolved.name,
                    "namespace": resolved.namespace,
                    "kind": resolved.kind.as_str(),
                    "command": resolved.shell_command(),
                }))
            );
        } else if raw {
            // Byte-exact: `pick --raw` is how a script round-trips its own
            // source, so no newline is added here.
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(headless::sanitize(&resolved.command).as_bytes())?;
            stdout.flush()?;
        } else {
            // Name the lookup miss before hoard opens any prompt, so an unknown
            // entry exits 2 instead of 4. `pick_command` still owns the
            // interactive parameter prompts for legacy `#param!` markers.
            // `pick_command` prompts for legacy `#param!` markers of *shell*
            // entries; with no terminal that prompt falls back to an empty value
            // and prints a command that looks fine and does nothing. Name the
            // parameters instead, and point at the subcommand that renders them.
            // `{{placeholders}}` are printed verbatim (no prompt, no data loss),
            // and Python sources keep `#name!` as comments.
            let resolved = self.resolve_entry(name)?;
            let placeholders = template::param_names(&resolved.command);
            let params: Vec<String> = if resolved.kind == CommandKind::Shell {
                template::param_names_with(
                    &resolved.command,
                    self.parameter_token(),
                    self.parameter_ending_token(),
                )
                .into_iter()
                .filter(|param| !placeholders.contains(param))
                .collect()
            } else {
                Vec::new()
            };
            if !headless::stdin_is_tty() && !params.is_empty() {
                return Err(CliError::new(
                    ErrorType::InvalidParameter,
                    format!(
                        "`pick` would prompt for {params:?}; use `hoard eval {name} -p key=value`",
                        params = params
                    ),
                )
                .with_suggestions(params.clone())
                .invalid_parameter(params));
            }
            let command = self.trove.pick_command(&self.config, name)?;
            println!("{}", headless::sanitize(&command.shell_command()));
        }
        Ok(())
    }

    /// `hoard eval <name> [--param k=v ...]`: substitute placeholders and show
    /// the invocation without touching the database.
    fn eval_command(&self, spec: &str, params: &[String]) -> std::result::Result<(), CliError> {
        let command = self.resolve_entry(spec)?;
        // The legacy `#name!` markers only mean something to a shell entry; in a
        // Python script they are comments and string content.
        let legacy = matches!(command.kind, crate::core::CommandKind::Shell).then(|| {
            (
                self.parameter_token().to_string(),
                self.parameter_ending_token().to_string(),
            )
        });
        let legacy = legacy
            .as_ref()
            .map(|(start, end)| (start.as_str(), end.as_str()));
        // A Python source is not a shell template: `new --script` stores no
        // parameters, so `eval` must not invent any from braces in the source.
        let is_shell = matches!(command.kind, crate::core::CommandKind::Shell);
        let known = match legacy {
            Some((start, end)) => template::param_specs_with(&command.command, start, end),
            None if is_shell => template::param_specs(&command.command),
            None => Vec::new(),
        };
        if !is_shell && !params.is_empty() {
            // A Python entry has no parameters to fill; silently ignoring `-p`
            // would hide the caller's mistake behind a plausible command.
            return Err(CliError::new(
                ErrorType::InvalidParameter,
                format!(
                    "'{spec}' is a Python entry and takes no parameters; pass arguments to the invocation instead"
                ),
            ));
        }
        let names: Vec<String> = known.iter().map(|spec| spec.name.clone()).collect();
        let mut values = BTreeMap::new();
        for pair in params {
            let (key, value) = match pair.split_once('=') {
                Some((key, value)) if !key.trim().is_empty() => (key.trim(), value.trim()),
                _ => {
                    return Err(CliError::new(
                        ErrorType::InvalidParameter,
                        format!("--param expects KEY=VALUE, got '{pair}'"),
                    )
                    .with_suggestions(names.clone()));
                }
            };
            // Last-wins would hide a caller mistake behind a plausible command.
            if values.insert(key.to_string(), value.to_string()).is_some() {
                return Err(CliError::new(
                    ErrorType::InvalidParameter,
                    format!("parameter '{key}' was passed more than once"),
                )
                .with_suggestions(names.clone()));
            }
        }
        // An empty value renders a command that looks fine and does nothing.
        // Required means required.
        for spec in &known {
            if spec.required()
                && let Some(value) = values.get(&spec.name)
                && value.is_empty()
            {
                return Err(CliError::new(
                    ErrorType::InvalidParameter,
                    format!("parameter '{}' needs a non-empty value", spec.name),
                )
                .with_suggestions(vec![spec.name.clone()])
                .invalid_parameter(vec![spec.name.clone()]));
            }
        }
        let rendered = if is_shell {
            template::render_with(&command.command, &values, legacy)?
        } else {
            command.command.clone()
        };
        let invocation = command.shell_command_with(&rendered);
        match self.mode {
            OutputMode::Human => println!("{}", headless::sanitize(&invocation)),
            OutputMode::Json => println!(
                "{}",
                self.mode.serialize(&json!({
                    "ok": true,
                    "name": command.name,
                    "command": invocation,
                    "kind": command.kind.as_str(),
                    "namespace": command.namespace,
                    "params": values,
                }))
            ),
            OutputMode::JsonCompact => {
                let mut payload = json!({"ok": true, "k": command.name, "c": invocation});
                if !values.is_empty() {
                    payload["p"] = json!(values);
                }
                println!("{}", self.mode.serialize(&payload));
            }
        }
        Ok(())
    }

    /// `hoard schema`: the machine-readable contract for agent callers.
    fn schema_command(&self) {
        let document = schema::document();
        println!("{}", self.mode.serialize(&document));
    }

    fn remove_command(&mut self, command_name: &str) -> std::result::Result<(), CliError> {
        self.resolve_entry(command_name)?;
        self.trove.remove_command(command_name)?;
        if self.mode.is_json() {
            println!(
                "{}",
                self.mode.serialize(&json!({
                    "ok": true,
                    "removed": [command_name],
                }))
            );
        } else {
            println!("Removed [{command_name}]");
        }
        self.persist()?;
        Ok(())
    }

    fn remove_namespace(&mut self, namespace: &str) -> std::result::Result<(), CliError> {
        // An unknown namespace is a lookup miss, not a runtime failure.
        let known: Vec<String> = self
            .trove
            .namespaces()
            .into_iter()
            .map(str::to_string)
            .collect();
        if !known.iter().any(|name| name == namespace) {
            return Err(CliError::not_found(
                format!("no namespace '{namespace}'"),
                headless::suggest(namespace, known),
            ));
        }
        self.trove.remove_namespace_commands(namespace)?;
        if self.mode.is_json() {
            println!(
                "{}",
                self.mode
                    .serialize(&json!({"ok": true, "removed_namespace": namespace}))
            );
        } else {
            println!("Removed all commands of namespace [{namespace}]");
        }
        self.persist()?;
        Ok(())
    }

    fn import_trove(&mut self, uri: &str) -> Result<()> {
        let imported = match Url::parse(uri) {
            Ok(url) if matches!(url.scheme(), "http" | "https") => {
                let trove_yaml = reqwest::blocking::get(url)?.text()?;
                Trove::from_yaml_str(&trove_yaml)?
            }
            _ => Trove::from_yaml_file(Path::new(uri))
                .with_context(|| format!("Not a valid URL or file path: {uri}"))?,
        };
        self.trove.merge_trove(&imported);
        self.persist()
    }

    /// Write a YAML export of the trove.
    ///
    /// `--all` and `--namespace` select entries without prompting so the export
    /// can be scripted; without either, hoard prompts for namespaces and then
    /// for individual commands.
    fn export_command(&self, path: &str, all: bool, namespaces: &[String]) -> Result<()> {
        let target_path = Path::new(path);
        if target_path.file_name().is_none() {
            bail!("No valid path with filename provided.");
        }

        let selected: Vec<HoardCmd> = if all || !namespaces.is_empty() {
            if !all {
                let available = self.trove.namespaces();
                for namespace in namespaces {
                    ensure!(
                        available.contains(&namespace.as_str()),
                        "No entry in namespace '{namespace}'. Available namespaces: {}",
                        available.join(", ")
                    );
                }
            }
            let selected = self
                .trove
                .commands
                .iter()
                .filter(|command| all || namespaces.iter().any(|n| n == &command.namespace))
                .cloned()
                .collect::<Vec<_>>();
            ensure!(!selected.is_empty(), "Nothing to export");
            selected
        } else {
            let available = self.trove.namespaces();
            let selected_namespaces = prompt_multiselect_options(
                "Export specific namespaces?",
                "Namespaces to export ( Space to select )",
                &available,
                |namespace| *namespace,
            );
            if selected_namespaces.is_empty() {
                println!("Nothing selected");
                return Ok(());
            }

            let commands = self
                .trove
                .commands
                .iter()
                .filter(|command| selected_namespaces.contains(&command.namespace.as_str()))
                .collect::<Vec<_>>();

            let selected_commands = prompt_multiselect_options(
                "Export specific commands?",
                "Commands to export ( Space to select )",
                &commands,
                |command| command.name.as_str(),
            );
            if selected_commands.is_empty() {
                println!("Nothing selected");
                return Ok(());
            }
            selected_commands.into_iter().cloned().collect()
        };

        let mut trove_for_export = Trove::default();
        for command in &selected {
            trove_for_export.add_command(command.clone(), true)?;
        }
        trove_for_export.to_yaml_file(target_path)?;
        println!(
            "Exported {} entries to {}",
            selected.len(),
            target_path.display()
        );
        Ok(())
    }

    fn set_parameter_token(&self, parameter_token: &str) -> Result<()> {
        let config_path = self
            .config
            .config_home_path
            .as_deref()
            .context("config home path is not configured")?;
        save_parameter_token(&self.config, config_path, parameter_token)
    }

    fn edit_command(&mut self, command_name: &str) -> Result<()> {
        // `edit` is a guided prompt. Headless, dialoguer would read EOF and save
        // defaults over the entry, so refuse instead of corrupting the trove.
        if self.mode.is_json() || !headless::stdin_is_tty() {
            return Err(CliError::new(
                ErrorType::InvalidParameter,
                "`edit` needs a terminal; use `new` to replace an entry, or `remove` it first",
            )
            .with_suggestions(vec!["new".to_string(), "remove".to_string()])
            .into());
        }
        println!("Editing {command_name}");
        let command_to_edit = self.trove.find_command(command_name).cloned();
        if let Ok(command) = &command_to_edit
            && command.kind == CommandKind::Python
        {
            let Some(source) = dialoguer::Editor::new()
                .extension(".py")
                .trim_newlines(false)
                .edit(&command.command)?
            else {
                return Ok(());
            };
            let edited = command
                .clone()
                .with_command(&source)
                .with_description_input(command.description.clone())
                .with_tags_input(Some(command.get_tags_as_string()));
            self.trove.update_command_by_name(&edited)?;
            return self.persist();
        }

        let trove_namespaces = self.trove.namespaces();
        match command_to_edit {
            Ok(command) => {
                println!("{}", headless::sanitize(&command.command));
                let new_command = HoardCmd::default()
                    .with_command_string_input(
                        Some(command.command.clone()),
                        self.parameter_token(),
                        self.parameter_ending_token(),
                    )
                    .with_name_input(Some(command.name.clone()), &self.trove)
                    .with_description_input(command.description.clone())
                    .with_tags_input(Some(command.get_tags_as_string()))
                    .with_namespace_input(&trove_namespaces);
                let (new_command, redacted) = new_command
                    .with_redacted_secrets(self.parameter_token(), self.parameter_ending_token());
                report_redactions(&redacted);
                self.trove.remove_command(command_name)?;
                self.trove.add_command(new_command, true)?;
                self.persist()?;
            }
            Err(_) => eprintln!("Could not find command {command_name} to edit"),
        }
        Ok(())
    }

    fn shell_config_command(shell: &str) {
        let src = match shell {
            "bash" => include_str!("shell/hoard.bash"),
            "fish" => include_str!("shell/hoard.fish"),
            "zsh" => include_str!("shell/hoard.zsh"),
            s => {
                println!("Unknown shell '{s}'!\nMust be either bash, fish or zsh!");
                return;
            }
        };
        print!("{src}");
    }
}

/// Tells the user which placeholders replaced secrets. The secret values
/// themselves are never printed.
fn report_redactions(redacted: &Redacted) {
    if redacted.is_clean() {
        return;
    }
    eprintln!(
        "⚠️  Replaced {} secret(s) with placeholders before saving: {}\n   \
         Fill them in again when you pick the entry, or read them from a secret manager.",
        redacted.redactions.len(),
        redacted.summary()
    );
}
