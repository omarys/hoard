use crate::cli_commands::{Cli, Commands, Mode};
use crate::config::defaults;
use crate::config::{
    HoardConfig, load_or_build_config, save_hoard_config_file, save_parameter_token,
};
use crate::core::trove::Trove;
use crate::core::{CommandKind, HoardCmd};
use crate::filter::query_trove;
use crate::gui::commands_gui;
use crate::gui::prompts::{
    Confirmation, prompt_input, prompt_multiselect_options, prompt_password,
    prompt_password_repeat, prompt_yes_or_no,
};
use crate::store::Db;
use crate::sync_models::TokenResponse;
use crate::util::rem_first_and_last;
use anyhow::{Context, Result, ensure};
use base64::Engine as _;
use base64::engine::general_purpose;
use dotenvy::dotenv;
use reqwest::StatusCode;
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
            Commands::Export { path } => self.export_command(&path)?,
            Commands::Edit { name } => self.edit_command(&name)?,
            Commands::ShellConfig { shell } => Self::shell_config_command(&shell),
            Commands::Sync { command } => self.sync(command)?,
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

    fn sync_server_url(&self) -> &str {
        self.config
            .sync_server_url
            .as_deref()
            .unwrap_or(defaults::SYNC_SERVER_URL)
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

        let trove_namespaces = self.trove.namespaces();
        let new_command = HoardCmd::default().with_command_string_input(
            command,
            self.parameter_token(),
            self.parameter_ending_token(),
        );
        let new_command = match namespace {
            Some(namespace) => new_command.with_namespace(&namespace),
            None => new_command.with_namespace_input(&trove_namespaces),
        }
        .with_name_input(name, &self.trove)
        .with_description_input(description.unwrap_or_default())
        .with_tags_input(tags);
        self.trove.add_command(new_command, true)?;
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

    fn export_command(&self, path: &str) -> Result<()> {
        let target_path = Path::new(path);
        if target_path.file_name().is_none() {
            println!("No valid path with filename provided.");
            return Ok(());
        }

        let namespaces = self.trove.namespaces();
        let selected_namespaces = prompt_multiselect_options(
            "Export specific namespaces?",
            "Namespaces to export ( Space to select )",
            &namespaces,
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

        let mut trove_for_export = Trove::default();
        for command in selected_commands {
            trove_for_export.add_command(command.clone(), true)?;
        }
        trove_for_export.to_yaml_file(target_path)
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

    fn trove_backup_path(&self) -> Result<std::path::PathBuf> {
        self.config
            .trove_path
            .as_deref()
            .map(|p| p.with_extension("db.bk"))
            .context("trove path is not configured")
    }

    /// Stores a consistent snapshot of the database before a `sync get` merge.
    fn backup_trove(&self) -> Result<()> {
        self.db.backup_to(&self.trove_backup_path()?)
    }

    /// Restores the database from the backup taken before the last `sync get`.
    fn revert_trove(&mut self) -> Result<()> {
        let backup_path = self.trove_backup_path()?;
        if !backup_path.exists() {
            println!("No trove backup found.");
            return Ok(());
        }
        if matches!(
            prompt_yes_or_no(
                "Found a backup from just before the last time you ran `hoard sync`. Are you sure you want to revert to this state?"
            ),
            Confirmation::Yes
        ) {
            let backup = Db::open(&backup_path)?;
            let commands = backup.commands()?;
            self.db.sync(&commands)?;
            self.trove = Trove::from_commands(&commands);
            println!("Done!");
        } else {
            println!("Keeping current trove database...");
        }
        Ok(())
    }

    fn register_user(&self) -> Result<()> {
        println!("Registering account..");
        let user_email = prompt_input("Email: ", false, None);
        let user_pw = prompt_password_repeat("Password: ");
        let client = reqwest::blocking::Client::new();
        let register_url = format!("{}register", self.sync_server_url());
        let register_body = format!("{{\"password\": \"{user_pw}\",\"email\": \"{user_email}\"}}");
        let body = client
            .post(register_url)
            .body(register_body)
            .header("Content-Type", "application/json")
            .send()?;
        if body.status() == StatusCode::CREATED {
            println!(
                "Created new user! Verification not needed for now. Run `hoard sync login` next.\n\nPlease consider supporting further development and help offset server costs here:\nbuy.stripe.com/9AQ9Bm6Nx4qb6YwaEE\nThis is the only time this message will pop up :)"
            );
        } else {
            println!("Something went all wrong. Try another email.");
        }
        Ok(())
    }

    fn login(&mut self) -> Result<()> {
        println!("Logging in..");
        let user_email = prompt_input("Email: ", false, None);
        let user_pw = prompt_password("Password: ");
        let login_body = format!("{{\"password\": \"{user_pw}\",\"email\": \"{user_email}\"}}");
        let client = reqwest::blocking::Client::new();
        let token_url = format!("{}token/new", self.sync_server_url());
        let body = client
            .get(token_url)
            .body(login_body)
            .header("Content-Type", "application/json")
            .send()?;
        if body.status() == StatusCode::CREATED {
            let token: TokenResponse = body.json()?;
            let b64_token = general_purpose::STANDARD.encode(token.token);
            self.config.api_token = Some(b64_token);
            let config_path = self
                .config
                .config_home_path
                .as_deref()
                .context("config home path is not configured")?;
            save_hoard_config_file(&self.config, config_path)?;
            println!("Success!");
        } else {
            println!("Invalid Email and password combination.");
        }
        Ok(())
    }

    fn get_trove_file(&self) -> Result<Option<Trove>> {
        println!("Syncing ...");
        let token = self
            .config
            .api_token
            .as_deref()
            .context("No API token set, please log in")?;
        let client = reqwest::blocking::Client::new();
        let save_url = format!("{}v1/trove", self.sync_server_url());
        let body = client
            .get(save_url)
            .bearer_auth(token)
            .header("Content-Type", "text/plain")
            .send()?;
        if body.status() == StatusCode::OK {
            // The server wraps the trove YAML in a JSON string
            let escaped_string = body.text()?.replace("\\n", "\n").replace("\\\"", "\"");
            return Trove::from_yaml_str(rem_first_and_last(&escaped_string)).map(Some);
        }
        Ok(None)
    }

    fn sync_safe(&self) -> Result<()> {
        println!("Uploading trove...");
        let token = self
            .config
            .api_token
            .as_deref()
            .context("No API token set, please log in")?;
        let client = reqwest::blocking::Client::new();
        let save_url = format!("{}v1/trove", self.sync_server_url());
        let trove_yaml = self.trove.to_yaml();
        let body = client
            .put(save_url)
            .body(trove_yaml)
            .bearer_auth(token)
            .header("Content-Type", "text/plain")
            .send()?;
        if body.status() == StatusCode::CREATED {
            println!("Done!");
        } else {
            println!("Could not save trove. Is it a valid trove file?");
            println!("{}", body.text()?);
        }
        Ok(())
    }

    pub fn sync(&mut self, command: Mode) -> Result<()> {
        match command {
            Mode::Register => self.register_user()?,
            Mode::Login => {
                if self.is_logged_in() {
                    println!("You are already logged in.");
                } else {
                    self.login()?;
                }
            }
            Mode::Logout => {
                println!("Logging out..");
                self.config.api_token = None;
                let config_path = self
                    .config
                    .config_home_path
                    .as_deref()
                    .context("config home path is not configured")?;
                save_hoard_config_file(&self.config, config_path)?;
            }
            Mode::Save => {
                if !self.is_logged_in() {
                    println!(
                        "Please log in [hoard sync login] or register an account [hoard sync register] to use the sync feature!"
                    );
                    return Ok(());
                }
                self.sync_safe()?;
            }
            Mode::Get => {
                if !self.is_logged_in() {
                    println!(
                        "Please log in [hoard sync login] or register an account [hoard sync register] to use the sync feature!"
                    );
                    return Ok(());
                }
                // Pull trove and merge. A backup is kept to allow reverting if
                // the merge goes wrong or commands are removed by accident.
                if let Some(trove) = self.get_trove_file()? {
                    self.backup_trove()?;
                    if self.trove.merge_trove(&trove) {
                        self.persist()?;
                        println!("All done!");
                    } else {
                        println!("No changes");
                    }
                } else {
                    println!("Could not fetch trove file from your account!");
                }
            }
            Mode::Revert => self.revert_trove()?,
        }
        Ok(())
    }

    const fn is_logged_in(&self) -> bool {
        self.config.api_token.is_some()
    }
}
