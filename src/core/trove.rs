use crate::config::HoardConfig;
use crate::core::HoardCmd;
use crate::core::error::HoardError;
use crate::core::parameters::Parameterized;
use anyhow::{Context, Result, anyhow};
use comfy_table::{Attribute, Cell, Color, Table, presets::UTF8_FULL};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::Path;

const CARGO_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Container for all stored hoard commands — a `treasure trove` of commands.
///
/// A `Trove` is the in-memory view of the command collection. It is loaded
/// from and persisted to SQLite via [`crate::store::Db`]; the YAML
/// serialization is kept for importing/exporting trove files and for the
/// cloud sync protocol.
#[derive(Debug, Serialize, Clone, Deserialize)]
pub struct Trove {
    /// The hoard version with which the commands are being stored, to
    /// potentially support migrating older collections to new ones.
    pub version: String,
    /// The stored commands.
    pub commands: Vec<HoardCmd>,
    /// Set of all namespaces used in the collection.
    #[serde(default)]
    pub namespaces: HashSet<String>,
}

impl Default for Trove {
    /// Create a new trove collection with the currently running hoard version.
    fn default() -> Self {
        Self {
            version: CARGO_VERSION.to_string(),
            commands: Vec::new(),
            namespaces: HashSet::new(),
        }
    }
}

impl Trove {
    /// Create a new Trove from a slice of commands.
    pub fn from_commands(commands: &[HoardCmd]) -> Self {
        let namespaces = commands
            .iter()
            .map(|c| c.namespace.clone())
            .collect::<HashSet<String>>();

        Self {
            version: CARGO_VERSION.to_string(),
            commands: commands.to_vec(),
            namespaces,
        }
    }

    /// Parse a trove collection from a YAML string.
    pub fn from_yaml_str(trove_string: &str) -> Result<Self> {
        let mut trove: Trove = serde_yaml::from_str(trove_string)
            .with_context(|| "The supplied trove file is invalid".to_string())?;
        trove.refresh_namespaces();
        Ok(trove)
    }

    /// Load a trove collection from a YAML file.
    pub fn from_yaml_file(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Could not read trove file {}", path.display()))?;
        Self::from_yaml_str(&content)
    }

    /// Serialize the trove collection to a YAML string.
    pub fn to_yaml(&self) -> String {
        serde_yaml::to_string(self).expect("serializing a trove cannot fail")
    }

    /// Write the trove collection as a YAML file at `path`.
    pub fn to_yaml_file(&self, path: &Path) -> Result<()> {
        fs::write(path, self.to_yaml())
            .with_context(|| format!("Could not write trove file {}", path.display()))
    }

    fn refresh_namespaces(&mut self) {
        self.namespaces = self.commands.iter().map(|c| c.namespace.clone()).collect();
    }

    /// Given a `HoardCmd`, check if there is a command with the same name and
    /// namespace already in the collection.
    pub fn get_command_collision(&self, command: &HoardCmd) -> Option<HoardCmd> {
        self.commands
            .iter()
            .find(|c| c.namespace == command.namespace && c.name == command.name)
            .cloned()
    }

    /// Get all commands in the trove collection sorted by usage count.
    pub fn get_commands_sorted_by_usage(&self) -> Vec<HoardCmd> {
        let mut commands = self.commands.clone();
        commands.sort_by_key(|c| std::cmp::Reverse(c.usage_count));
        commands
    }

    /// Check whether a command with the same name, namespace and command
    /// string is already in the collection.
    fn is_command_present(&self, command: &HoardCmd) -> bool {
        self.commands.iter().any(|c| {
            c.namespace == command.namespace
                && c.name == command.name
                && c.command == command.command
        })
    }

    /// Add a command to the trove collection.
    ///
    /// Returns `Ok(true)` if the collection changed, `Ok(false)` if a name
    /// collision was resolved without changing anything and `Err` when the
    /// command failed validation.
    ///
    /// When a command with the same name and namespace already exists and
    /// `overwrite_colliding` is set, the existing command is replaced;
    /// otherwise the name collision is sidestepped with a random suffix.
    pub fn add_command(
        &mut self,
        new_command: HoardCmd,
        overwrite_colliding: bool,
    ) -> Result<bool, HoardError> {
        if !new_command.is_valid() {
            return Err(HoardError::InvalidCommandForSave);
        }
        let dirty = match self.get_command_collision(&new_command) {
            // Colliding command is identical, nothing to do
            Some(_) if self.is_command_present(&new_command) => false,
            // Collision present and overwriting is allowed
            Some(colliding) if overwrite_colliding => {
                self.commands.retain(|x| x != &colliding);
                let namespace = new_command.namespace.clone();
                self.commands.push(new_command);
                self.add_namespace(&namespace);
                true
            }
            // Collision present but overwriting is not allowed: random suffix
            Some(_) => {
                let c = new_command.with_random_name_suffix();
                let namespace = c.namespace.clone();
                self.commands.push(c);
                self.add_namespace(&namespace);
                true
            }
            // No collision: add the command and its namespace
            None => {
                self.add_namespace(&new_command.namespace);
                self.commands.push(new_command);
                true
            }
        };
        Ok(dirty)
    }

    /// Add a namespace value to the namespaces set if it is not present yet.
    pub fn add_namespace(&mut self, namespace: &str) {
        self.namespaces.insert(namespace.to_string());
    }

    /// Remove a command from the trove collection by name.
    pub fn remove_command(&mut self, name: &str) -> Result<(), anyhow::Error> {
        let position = self
            .commands
            .iter()
            .position(|x| x.name == name)
            .ok_or_else(|| anyhow!("Command not found [{name}]"))?;
        self.commands.remove(position);
        Ok(())
    }

    /// Update a command's usage count and `last_used` timestamp.
    pub fn update_command_meta(&mut self, command: &HoardCmd) -> Result<(), anyhow::Error> {
        let position = self
            .commands
            .iter()
            .position(|x| x.name == command.name)
            .ok_or_else(|| anyhow!("Command not found [{}]", command.name))?;
        let mut updated_command = command.clone();
        updated_command.mut_increase_usage_count();
        updated_command.mut_update_last_used();
        self.commands[position] = updated_command;
        Ok(())
    }

    /// Remove all commands of a namespace.
    pub fn remove_namespace_commands(&mut self, namespace: &str) -> Result<(), anyhow::Error> {
        let before = self.commands.len();
        self.commands.retain(|x| x.namespace != namespace);
        if self.commands.len() == before {
            return Err(anyhow!("No Commands found in namespace [{namespace}]"));
        }
        Ok(())
    }

    /// All namespaces in the trove, sorted.
    pub fn namespaces(&self) -> Vec<&str> {
        let mut namespaces: Vec<&str> =
            self.commands.iter().map(|c| c.namespace.as_str()).collect();
        namespaces.sort_unstable();
        namespaces.dedup();
        namespaces
    }

    /// Look up a command by name and prompt for its parameter values.
    pub fn pick_command(&self, config: &HoardConfig, name: &str) -> Result<HoardCmd> {
        let command = self
            .commands
            .iter()
            .find(|c| c.name == name)
            .ok_or_else(|| anyhow!("No matching command found with name: {name}"))?;
        Ok(command.clone().with_input_parameters(
            config
                .parameter_token
                .as_deref()
                .unwrap_or(crate::core::parameters::DEFAULT_PARAMETER_TOKEN),
            config
                .parameter_ending_token
                .as_deref()
                .unwrap_or(crate::core::parameters::DEFAULT_ENDING_PARAMETER_TOKEN),
        ))
    }

    /// Replace a command in place and refresh its `last_used` timestamp.
    pub fn update_command_by_name(&mut self, command: &HoardCmd) -> &mut Self {
        if let Some(existing) = self.commands.iter_mut().find(|c| c.name == command.name) {
            *existing = command.clone();
            existing.mut_update_last_used();
        }
        self
    }

    /// Check if the trove collection is empty.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Merge another trove into this one, overwriting collisions.
    ///
    /// Returns `true` when at least one command changed in this trove.
    pub fn merge_trove(&mut self, other: &Self) -> bool {
        other
            .commands
            .iter()
            .map(|c| self.add_command(c.clone(), true))
            .any(|result| result.is_ok())
    }

    /// Print the trove as a table to stdout.
    pub fn print_trove(&self) {
        let mut table = Table::new();
        table.load_style(UTF8_FULL);
        table.force_no_tty();
        table.set_header(["Name", "namespace", "command", "description", "tags"]);
        for command in &self.commands {
            table.add_row(vec![
                Cell::new(&command.name)
                    .fg(Color::Green)
                    .add_attribute(Attribute::Bold),
                Cell::new(&command.namespace),
                Cell::new(&command.command),
                Cell::new(&command.description),
                Cell::new(command.get_tags_as_string()),
            ]);
        }
        // Ignore a closed pipe so `hoard list -s | head` does not panic.
        let _ = writeln!(std::io::stdout().lock(), "{table}");
    }
}

#[cfg(test)]
mod test_commands {
    use super::*;

    fn command(name: &str, namespace: &str, command: &str) -> HoardCmd {
        HoardCmd::default()
            .with_name(name)
            .with_namespace(namespace)
            .with_command(command)
    }

    #[test]
    fn empty_trove() {
        let trove = Trove::default();
        assert!(trove.is_empty());
    }

    #[test]
    fn not_empty_trove() {
        let mut trove = Trove::default();
        let ok = trove.add_command(command("test", "test-namespace", "echo 'test'"), true);
        assert!(ok.is_ok());
        assert!(!trove.is_empty());
    }

    #[test]
    fn trove_namespaces() {
        let namespace1 = "NAMESPACE1";
        let namespace2 = "NAMESPACE2";

        let mut trove = Trove::default();
        trove
            .add_command(command("name1", namespace1, "command1"), true)
            .unwrap();
        trove
            .add_command(command("name2", namespace2, "command2"), true)
            .unwrap();
        trove
            .add_command(command("name3", namespace1, "command3"), true)
            .unwrap();

        assert_eq!(vec![namespace1, namespace2], trove.namespaces());
    }

    #[test]
    fn add_valid_command() {
        let mut trove = Trove::default();
        trove
            .add_command(command("test", "test", "test"), true)
            .unwrap();
        assert!(!trove.is_empty());
        assert!(trove.namespaces.contains("test"));
    }

    #[test]
    fn test_multiple_new_namespaces_added() {
        let mut trove = Trove::default();
        trove
            .add_command(command("test1", "test1", "test1"), true)
            .unwrap();
        trove
            .add_command(command("test2", "test2", "test2"), true)
            .unwrap();
        assert!(!trove.is_empty());
        assert!(trove.namespaces.contains("test1"));
        assert!(trove.namespaces.contains("test2"));
    }

    #[test]
    fn test_add_multiple_commands_same_namespace() {
        let mut trove = Trove::default();
        trove
            .add_command(command("test1", "test", "test1"), true)
            .unwrap();
        trove
            .add_command(command("test2", "test", "test2"), true)
            .unwrap();
        assert!(!trove.is_empty());
        assert!(trove.namespaces.contains("test"));
        assert_eq!(trove.namespaces.len(), 1);
    }

    #[test]
    fn test_add_and_remove_command() {
        let mut trove = Trove::default();
        trove
            .add_command(command("test", "test", "test"), true)
            .unwrap();
        assert!(!trove.is_empty());
        assert!(trove.namespaces.contains("test"));
        trove.remove_command("test").unwrap();
        assert!(trove.is_empty());
        assert!(trove.namespaces.contains("test"));
    }

    #[test]
    fn test_add_command_with_same_name() {
        let mut trove = Trove::default();
        let cmd = command("test", "test", "test");
        trove.add_command(cmd.clone(), true).unwrap();
        assert!(!trove.is_empty());
        assert!(trove.namespaces.contains("test"));
        let ok = trove.add_command(cmd, true);
        assert!(ok.is_ok());
    }

    #[test]
    fn test_remove_nonexistent_command() {
        let mut trove = Trove::default();
        assert!(trove.remove_command("nonexistent").is_err());
    }

    #[test]
    fn test_add_remove_commands_different_namespaces() {
        let mut trove = Trove::default();
        trove
            .add_command(command("test1", "namespace1", "test1"), true)
            .unwrap();
        assert!(trove.namespaces.contains("namespace1"));
        trove
            .add_command(command("test2", "namespace2", "test2"), true)
            .unwrap();
        assert!(trove.namespaces.contains("namespace2"));

        trove.remove_command("test1").unwrap();
        trove.remove_command("test2").unwrap();
        assert!(trove.is_empty());
    }

    #[test]
    fn test_remove_namespace_commands() {
        let mut trove = Trove::default();
        trove.add_command(command("a", "ns1", "a"), true).unwrap();
        trove.add_command(command("b", "ns2", "b"), true).unwrap();
        trove.remove_namespace_commands("ns1").unwrap();
        assert_eq!(trove.commands.len(), 1);
        assert!(trove.remove_namespace_commands("missing").is_err());
    }

    #[test]
    fn test_yaml_roundtrip() {
        let mut trove = Trove::default();
        trove
            .add_command(command("git-status", "git", "git status"), true)
            .unwrap();
        let yaml = trove.to_yaml();
        let parsed = Trove::from_yaml_str(&yaml).unwrap();
        assert_eq!(parsed.commands, trove.commands);
        assert_eq!(parsed.namespaces, trove.namespaces);
    }
}
