use crate::cli_commands::{Cli, Commands};
use crate::config::defaults;
use crate::config::{HoardConfig, load_or_build_config, save_parameter_token};
use crate::core::secrets::Redacted;
use crate::core::trove::Trove;
use crate::core::{CommandKind, HoardCmd};
use crate::filter::query_trove;
use crate::gui::commands_gui;
use crate::gui::prompts::prompt_multiselect_options;
use crate::store::Db;
use anyhow::{Context, Result, bail, ensure};
use dotenvy::dotenv;
use reqwest::Url;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The hoard application: configuration, the in-memory trove and its
/// SQLite-backed persistence.
pub struct Hoard {
    config: HoardConfig,
    trove: Trove,
    db: Db,
}

impl Hoard {
    /// Loads configuration, opens the trove database and loads all commands.
    pub fn load(config_home_path: Option<String>) -> Result<Self> {
        dotenv().ok();
        let config = load_or_build_config(config_home_path)?;
        let db_path = config
            .trove_path
            .as_deref()
            .context("trove path is not configured")?;
        let db = Db::open(db_path)?;
        let trove = Trove::from_commands(&db.commands()?);
        Ok(Self { config, trove, db })
    }

    /// Runs the parsed CLI command and returns the selected command string
    /// (if any) together with whether it is meant for shell autocomplete.
    pub fn start(&mut self, cli: Cli) -> Result<(String, bool)> {
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
                json,
                simple,
            } => {
                autocomplete_command = self
                    .list_commands(simple, json, filter)?
                    .unwrap_or_default()
            }
            Commands::Pick { name, raw } => self.pick_command(&name, raw)?,
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
        }

        Ok((autocomplete_command, cli.autocomplete))
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

    fn new_command(
        &mut self,
        name: Option<String>,
        tags: Option<String>,
        command: Option<String>,
        description: Option<String>,
        script: Option<PathBuf>,
        namespace: Option<String>,
    ) -> Result<()> {
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
            ensure!(
                self.trove.get_command_collision(&entry).is_none(),
                "An entry named '{}' already exists in namespace '{}'",
                entry.name,
                entry.namespace
            );
            self.trove.add_command(entry, false)?;
            return self.persist();
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
            bail!(
                "An entry named '{}' already exists in namespace '{}'. Use `hoard edit` or a different name.",
                existing.name,
                existing.namespace
            );
        }

        let saved = format!("{}/{}", new_command.namespace, new_command.name);
        self.trove.add_command(new_command, true)?;
        println!("Saved [{saved}]");
        self.persist()
    }

    fn list_commands(
        &mut self,
        is_simple: bool,
        is_structured: bool,
        filter: Option<String>,
    ) -> Result<Option<String>> {
        if self.trove.is_empty() {
            println!("No command hoarded.\nRun [ hoard new ] first to hoard a command.");
        } else if is_simple {
            self.trove.print_trove();
        } else if is_structured {
            // Structured output (YAML), filtered by `filter`
            let query = filter.unwrap_or_default();
            let filtered_trove = query_trove(&self.trove, &query);
            return Ok(Some(filtered_trove.to_yaml()));
        } else {
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
        }
        Ok(None)
    }

    fn pick_command(&self, name: &str, raw: bool) -> Result<()> {
        if raw {
            let command = self.trove.find_command(name)?;
            std::io::stdout()
                .lock()
                .write_all(command.command.as_bytes())?;
        } else {
            let command = self.trove.pick_command(&self.config, name)?;
            println!("{}", command.shell_command());
        }
        Ok(())
    }

    fn remove_command(&mut self, command_name: &str) -> Result<()> {
        self.trove.remove_command(command_name)?;
        println!("Removed [{command_name}]");
        self.persist()
    }

    fn remove_namespace(&mut self, namespace: &str) -> Result<()> {
        self.trove.remove_namespace_commands(namespace)?;
        println!("Removed all commands of namespace [{namespace}]");
        self.persist()
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
                println!("{}", command.command);
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
