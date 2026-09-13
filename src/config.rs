use crate::gui::prompts::prompt_input;
use crate::store::{TROVE_DB, local_trove_exists};
use crate::theme;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const HOARD_HOMEDIR: &str = ".config/hoard";
pub const HOARD_CONFIG: &str = "config.yml";

/// Defaults for the configurable parts of hoard.
pub(crate) mod defaults {
    pub const DEFAULT_NAMESPACE: &str = "default";
    pub const QUERY_PREFIX: &str = "  >";
    pub const PARAMETER_TOKEN: &str = "#";
    pub const PARAMETER_ENDING_TOKEN: &str = "!";
    pub const SYNC_SERVER_URL: &str = "https://troveserver.herokuapp.com/";
}

/// The hoard configuration.
///
/// Stored as YAML at `~/.config/hoard/config.yml`. The four color fields map
/// to hoard's interactive theme and default to the [Dracula palette](crate::theme).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoardConfig {
    pub version: String,
    pub default_namespace: String,
    pub config_home_path: Option<PathBuf>,
    /// Path of the trove SQLite database.
    pub trove_path: Option<PathBuf>,
    pub query_prefix: String,
    // Color settings (defaults: Dracula)
    pub primary_color: Option<(u8, u8, u8)>,
    pub secondary_color: Option<(u8, u8, u8)>,
    pub tertiary_color: Option<(u8, u8, u8)>,
    pub command_color: Option<(u8, u8, u8)>,
    // Parameter settings
    pub parameter_token: Option<String>,
    /// Token to indicate the end of a named parameter
    pub parameter_ending_token: Option<String>,
    pub read_from_current_directory: Option<bool>,
    // URL to trove sync server
    pub sync_server_url: Option<String>,
    pub api_token: Option<String>,
    pub gpt_api_key: Option<String>,
}

impl Default for HoardConfig {
    fn default() -> Self {
        Self {
            version: VERSION.to_string(),
            default_namespace: defaults::DEFAULT_NAMESPACE.to_string(),
            config_home_path: None,
            trove_path: None,
            query_prefix: defaults::QUERY_PREFIX.to_string(),
            primary_color: Some(theme::FOREGROUND),
            secondary_color: Some(theme::PURPLE),
            tertiary_color: Some(theme::BACKGROUND),
            command_color: Some(theme::GREEN),
            parameter_token: Some(defaults::PARAMETER_TOKEN.to_string()),
            parameter_ending_token: Some(defaults::PARAMETER_ENDING_TOKEN.to_string()),
            read_from_current_directory: Some(Self::default_read_from_current_directory()),
            sync_server_url: Some(defaults::SYNC_SERVER_URL.to_string()),
            api_token: None,
            gpt_api_key: None,
        }
    }
}

impl HoardConfig {
    pub fn new(hoard_home_path: &Path) -> Self {
        Self {
            config_home_path: Some(hoard_home_path.to_path_buf()),
            trove_path: Some(hoard_home_path.join(TROVE_DB)),
            ..Self::default()
        }
    }

    /// Lets the user choose a default namespace on first run.
    pub fn with_default_namespace(self) -> Self {
        let default_namespace = prompt_input(
            "This is the first time running hoard.\nChoose a default namespace where you want to hoard your commands.",
            false,
            Some(defaults::DEFAULT_NAMESPACE.to_string()),
        );
        Self {
            default_namespace,
            ..self
        }
    }

    const fn default_read_from_current_directory() -> bool {
        true
    }
}

/// Loads the hoard config file at `$HOME/.config/hoard/config.yml`, creating
/// a fresh one if it does not exist. If `hoard_home_path` is set, the config
/// is read from that custom path instead.
pub fn load_or_build_config(hoard_home_path: Option<String>) -> Result<HoardConfig> {
    match hoard_home_path {
        Some(custom_path) => load_or_build(&PathBuf::from(custom_path)),
        None => dirs::home_dir()
            .context("No $HOME directory found for hoard config")
            .and_then(|home| load_or_build(&home)),
    }
}

fn load_or_build(path: &Path) -> Result<HoardConfig> {
    // Check if the hoard directory exists; create it if it does not
    let hoard_dir = path.join(HOARD_HOMEDIR);
    fs::create_dir_all(&hoard_dir)?;

    let hoard_config_path = hoard_dir.join(HOARD_CONFIG);
    if !hoard_config_path.exists() {
        let new_config = HoardConfig::new(&hoard_dir).with_default_namespace();
        save_config(&new_config, &hoard_config_path)?;
        return Ok(new_config);
    }

    let mut config: HoardConfig = serde_yaml::from_str(&fs::read_to_string(&hoard_config_path)?)
        .with_context(|| {
            format!(
                "Could not parse config file {}",
                hoard_config_path.display()
            )
        })?;
    append_missing_default_values_to_config(&mut config, &hoard_dir, &hoard_config_path)?;

    if config.parameter_token == config.parameter_ending_token {
        bail!(
            "Your parameter token {:?} equals your ending token {:?}. \
             Please set one of them to another character!",
            config.parameter_token.as_deref().unwrap_or(""),
            config.parameter_ending_token.as_deref().unwrap_or("")
        );
    }

    // Point `trove_path` at the SQLite database, migrating legacy
    // `trove.yml` paths. When a trove is present in the current directory the
    // local one takes precedence ("global" trove is ignored). A changed path
    // is written back so the config file reflects what is actually loaded.
    let legacy_trove_path = config.trove_path.clone();
    config.trove_path =
        if config.read_from_current_directory.unwrap_or(false) && local_trove_exists() {
            Some(Path::new(TROVE_DB).to_path_buf())
        } else {
            legacy_trove_path.as_ref().map(|p| p.with_extension("db"))
        };
    if config.trove_path != legacy_trove_path {
        save_config(&config, &hoard_config_path)?;
    }

    Ok(config)
}

/// Backfills missing configuration fields with their defaults. Mostly for
/// legacy configuration support when new configuration options are added.
/// Returns `true` when something was filled in, so the caller can persist it.
fn append_missing_default_values_to_config(
    loaded_config: &mut HoardConfig,
    hoard_dir: &Path,
    hoard_config_path: &Path,
) -> Result<()> {
    let mut dirty = false;
    if loaded_config.primary_color.is_none() {
        loaded_config.primary_color = Some(theme::FOREGROUND);
        dirty = true;
    }
    if loaded_config.secondary_color.is_none() {
        loaded_config.secondary_color = Some(theme::PURPLE);
        dirty = true;
    }
    if loaded_config.tertiary_color.is_none() {
        loaded_config.tertiary_color = Some(theme::BACKGROUND);
        dirty = true;
    }
    if loaded_config.command_color.is_none() {
        loaded_config.command_color = Some(theme::GREEN);
        dirty = true;
    }
    if loaded_config.trove_path.is_none() {
        loaded_config.trove_path = Some(hoard_dir.join(TROVE_DB));
        dirty = true;
    }
    if loaded_config.parameter_token.is_none() {
        loaded_config.parameter_token = Some(defaults::PARAMETER_TOKEN.to_string());
        dirty = true;
    }
    if loaded_config.parameter_ending_token.is_none() {
        loaded_config.parameter_ending_token = Some(defaults::PARAMETER_ENDING_TOKEN.to_string());
        dirty = true;
    }
    if loaded_config.read_from_current_directory.is_none() {
        loaded_config.read_from_current_directory = Some(false);
        dirty = true;
    }
    if loaded_config.sync_server_url.is_none() {
        loaded_config.sync_server_url = Some(defaults::SYNC_SERVER_URL.to_string());
        dirty = true;
    }

    if dirty {
        save_config(loaded_config, hoard_config_path)?;
    }
    Ok(())
}

/// Overwrites `parameter_token` in the stored config file.
pub fn save_parameter_token(
    config: &HoardConfig,
    config_path: &Path,
    parameter_token: &str,
) -> Result<()> {
    let mut new_config = config.clone();
    new_config.parameter_token = Some(parameter_token.to_string());
    save_config(&new_config, &config_path.join(HOARD_CONFIG))
}

fn save_config(config_to_save: &HoardConfig, config_path: &Path) -> Result<()> {
    let yaml = serde_yaml::to_string(config_to_save)?;
    fs::write(config_path, yaml)
        .with_context(|| format!("Could not write config file {}", config_path.display()))
}

/// Persists `config_to_save` at `base_path/config.yml`.
pub fn save_hoard_config_file(config_to_save: &HoardConfig, base_path: &Path) -> Result<()> {
    save_config(config_to_save, &base_path.join(HOARD_CONFIG))
}

#[cfg(test)]
mod test_config {
    use super::{HOARD_CONFIG, HoardConfig, save_parameter_token};
    use std::fs::File;
    use tempfile::tempdir;

    #[test]
    fn test_save_parameter_token() {
        let tmp_dir = tempdir().unwrap();

        let tmp_path = tmp_dir.path();
        let config = HoardConfig::new(tmp_path);
        assert!(save_parameter_token(&config, tmp_path, "@").is_ok());

        // read config file, and check parameter token.
        let tmp_file = tmp_dir.path().join(HOARD_CONFIG);
        let f = File::open(tmp_file).unwrap();
        let parsed_config = serde_yaml::from_reader::<_, HoardConfig>(f).unwrap();
        assert_eq!(parsed_config.parameter_token, Some(String::from("@")));
    }

    #[test]
    fn defaults_are_dracula() {
        let config = HoardConfig::default();
        assert_eq!(config.primary_color, Some(crate::theme::FOREGROUND));
        assert_eq!(config.secondary_color, Some(crate::theme::PURPLE));
        assert_eq!(config.command_color, Some(crate::theme::GREEN));
    }
}
