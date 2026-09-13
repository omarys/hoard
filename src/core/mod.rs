pub mod error;
pub mod parameters;
pub mod trove;

use crate::core::error::HoardError;
use crate::core::trove::Trove;
use crate::gui::prompts::{prompt_input, prompt_input_validate, prompt_select_with_options};
use rand::RngExt;
use rand::distr::Alphanumeric;
use serde::{Deserialize, Serialize};
use std::time;

fn default_time() -> time::SystemTime {
    time::SystemTime::now()
}

/// Storage for a single saved command.
///
/// A `HoardCmd` stores the following parameters:
/// - `name`: The name of the command by which it is referenced
/// - `command`: The terminal command to be stored and executed
/// - `description`: A description of the command for the user
/// - `tags`: A list of tags to be used for searching
/// - `created`: The date and time the command was created
/// - `modified`: The date and time the command was last modified
/// - `last_used`: The date and time the command was last used
/// - `usage_count`: The number of times the command has been used
/// - `is_favorite`: A flag to indicate if the command is a favorite
/// - `is_hidden`: A flag to indicate if the command is hidden
/// - `is_deleted`: A flag to indicate if the command is deleted
/// - `namespace`: The namespace the command belongs to
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoardCmd {
    /// The name of the command by which it is referenced
    pub name: String,

    /// The terminal command to be stored and executed
    pub command: String,

    /// A description of the command for the user
    pub description: String,

    /// A list of tags to be used for searching
    #[serde(default)]
    pub tags: Vec<String>,

    /// The date and time the command was created
    #[serde(default = "default_time")]
    pub created: time::SystemTime,

    /// The date and time the command was last modified
    #[serde(default = "default_time")]
    pub modified: time::SystemTime,

    /// The date and time the command was last used
    #[serde(default = "default_time")]
    pub last_used: time::SystemTime,

    /// The number of times the command has been used
    #[serde(default)]
    pub usage_count: usize,

    /// A flag to indicate if the command is a favorite
    #[serde(default)]
    pub is_favorite: bool,

    /// A flag to indicate if the command is hidden
    #[serde(default)]
    pub is_hidden: bool,

    /// A flag to indicate if the command is deleted
    #[serde(default)]
    pub is_deleted: bool,

    /// The namespace the command belongs to
    pub namespace: String,
}

/// Two commands are considered equal when their identifying fields match.
impl PartialEq for HoardCmd {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.namespace == other.namespace
            && self.command == other.command
            && self.description == other.description
            && self.tags == other.tags
    }
}

impl HoardCmd {
    /// Create a new `HoardCmd` with default values
    pub fn default() -> Self {
        Self {
            name: String::new(),
            command: String::new(),
            description: String::new(),
            tags: Vec::new(),
            created: time::SystemTime::now(),
            modified: time::SystemTime::now(),
            last_used: time::SystemTime::now(),
            usage_count: 0,
            is_favorite: false,
            is_hidden: false,
            is_deleted: false,
            namespace: String::new(),
        }
    }

    /// Set the name of the command
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn with_name(self, name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..self
        }
    }

    /// Set the command to be stored and executed
    pub fn with_command(self, command: &str) -> Self {
        Self {
            command: command.to_string(),
            ..self
        }
    }

    /// Set the namespace of the command
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn with_namespace(self, namespace: &str) -> Self {
        Self {
            namespace: namespace.to_string(),
            ..self
        }
    }

    /// Check if a command is valid for saving.
    /// A valid command cannot be an empty string.
    pub fn is_command_valid(command: &str) -> Result<(), HoardError> {
        if command.is_empty() {
            return Err(HoardError::InvalidCommand);
        }
        Ok(())
    }

    /// Check if a command is valid.
    /// A valid command must have:
    /// - A name that is not empty
    /// - A command that is not empty
    /// - A namespace that is not empty
    /// - `created/modified/last_used` that is not the `UNIX_EPOCH`
    pub fn is_valid(&self) -> bool {
        !self.name.is_empty()
            && !self.command.is_empty()
            && !self.namespace.is_empty()
            && self.created != time::UNIX_EPOCH
            && self.modified != time::UNIX_EPOCH
            && self.last_used != time::UNIX_EPOCH
    }

    /// Check if a name is valid for saving.
    /// A valid name cannot be empty and cannot contain whitespaces.
    pub fn is_name_valid(name: &str) -> Result<(), HoardError> {
        if name.is_empty() {
            return Err(HoardError::EmptyName);
        }
        if name.contains(' ') {
            return Err(HoardError::NameWithWhitespace);
        }
        Ok(())
    }

    /// Check if the tags are valid for saving.
    /// A valid tag vector cannot be empty.
    pub fn are_tags_valid(tags: &str) -> Result<(), HoardError> {
        if tags.is_empty() {
            return Err(HoardError::EmptyTags);
        }
        Ok(())
    }

    /// Return vector of tags as a comma separated string.
    ///
    /// # Example
    /// ```
    /// use hoardlib::command::HoardCmd;
    ///
    /// let mut cmd = HoardCmd::default();
    /// cmd.tags.push("tag1".to_string());
    /// cmd.tags.push("tag2".to_string());
    /// cmd.tags.push("tag3".to_string());
    ///
    /// assert_eq!(cmd.get_tags_as_string(), "tag1,tag2,tag3");
    /// ```
    pub fn get_tags_as_string(&self) -> String {
        self.tags.join(",")
    }

    /// Set the tags of the command from a comma separated string.
    pub fn with_tags_raw(self, tags: &str) -> Self {
        if tags.trim().is_empty() {
            return self;
        }
        Self {
            tags: tags.split(',').map(str::trim).map(str::to_string).collect(),
            ..self
        }
    }

    /// Add a random suffix to the name of the command
    pub fn with_random_name_suffix(self) -> Self {
        let suffix: String = rand::rng()
            .sample_iter(&Alphanumeric)
            .take(4)
            .map(char::from)
            .collect();
        Self {
            name: format!("{}-{suffix}", self.name),
            ..self
        }
    }

    /// Prompts the user for a command string, with optional default value and
    /// parameter tokens.
    ///
    /// The user can mark unknown parameters with `parameter_token` and name the
    /// parameter with any string, ending it with `parameter_ending_token`.
    pub fn with_command_string_input(
        self,
        default_value: Option<String>,
        parameter_token: &str,
        parameter_ending_token: &str,
    ) -> Self {
        let base_prompt = format!(
            "Command to hoard ( Mark unknown parameters with '{parameter_token}'. Name the parameter with any string and end it with '{parameter_ending_token}' )\n"
        );
        let command_string = prompt_input(&base_prompt, false, default_value);
        Self {
            command: command_string,
            ..self
        }
    }

    /// Prompts the user for tags, with an optional default value, and validates
    /// the input (comma separated, no whitespaces).
    pub fn with_tags_input(self, default_value: Option<String>) -> Self {
        let tag_validator = |input: &String| -> Result<(), String> {
            if input.contains(' ') {
                Err("Tags can't contain whitespaces".to_string())
            } else {
                Ok(())
            }
        };
        let tags = prompt_input_validate(
            "Give your command some optional tags ( comma separated )",
            true,
            default_value,
            Some(tag_validator),
        );
        self.with_tags_raw(&tags)
    }

    /// Prompts the user to pick a namespace, offering to create a new one.
    pub fn with_namespace_input(self, selection: &[&str]) -> Self {
        let mut selection = selection.to_vec();
        selection.push("New namespace");

        let selected = prompt_select_with_options("Namespace of the command", &selection);

        let mut selected_namespace: String = selection[selected].to_string();
        if selected_namespace == "New namespace" {
            selected_namespace = prompt_input(
                "Namespace of the command",
                false,
                Some("default".to_string()),
            );
        }

        Self {
            namespace: selected_namespace,
            ..self
        }
    }

    fn with_name_input_prompt(
        self,
        default_value: Option<String>,
        trove: &Trove,
        prompt_string: &str,
    ) -> Self {
        let namespace = self.namespace.clone();
        let command_names = &trove.commands;
        let validator = move |input: &String| -> Result<(), String> {
            if input.contains(' ') {
                Err("The name can't contain whitespaces".to_string())
            } else if command_names
                .iter()
                .any(|x| x.namespace == namespace && x.name == *input)
            {
                Err(
                    "A command with same name exists in the this namespace. Input a different name"
                        .to_string(),
                )
            } else {
                Ok(())
            }
        };
        let name = prompt_input_validate(prompt_string, false, default_value, Some(validator));
        Self { name, ..self }
    }

    /// Prompts the user for a command name, validating it against `trove`.
    pub fn with_name_input(self, default_value: Option<String>, trove: &Trove) -> Self {
        self.with_name_input_prompt(default_value, trove, "Name your command")
    }

    /// Prompts the user for a description, with a default value.
    pub fn with_description_input(self, default_value: String) -> Self {
        let description_string =
            prompt_input("Describe what the command does", false, Some(default_value));
        Self {
            description: description_string,
            ..self
        }
    }

    /// Update `last_used` to the current time.
    pub fn mut_update_last_used(&mut self) {
        self.last_used = time::SystemTime::now();
    }

    /// Increase the usage count of the command.
    pub fn mut_increase_usage_count(&mut self) -> &mut Self {
        self.usage_count += 1;
        self
    }
}

/// Splits a comma separated tag string, dropping all whitespace.
/// Legacy semantics: whitespace is stripped before splitting so free-form
/// input like `my tag` never produces a tag containing a space.
pub fn string_to_tags(tags: &str) -> Vec<String> {
    tags.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .split(',')
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod test_commands {
    use super::*;

    #[test]
    fn one_tag_as_string() {
        let command = HoardCmd::default().with_tags_raw("foo");
        assert_eq!("foo", command.get_tags_as_string());
    }

    #[test]
    fn no_tag_as_string() {
        let command = HoardCmd::default();
        assert_eq!("", command.get_tags_as_string());
    }

    #[test]
    fn multiple_tags_as_string() {
        let command = HoardCmd::default().with_tags_raw("foo,bar");
        assert_eq!("foo,bar", command.get_tags_as_string());
    }

    #[test]
    fn parse_single_tag() {
        let command = HoardCmd::default().with_tags_raw("foo");
        assert_eq!(vec!["foo".to_string()], command.tags);
    }

    #[test]
    fn parse_multiple_tags() {
        let command = HoardCmd::default().with_tags_raw("foo,bar");
        assert_eq!(vec!["foo".to_string(), "bar".to_string()], command.tags);
    }

    #[test]
    fn parse_whitespace_in_tags() {
        let command = HoardCmd::default().with_tags_raw("foo, bar");
        assert_eq!(vec!["foo".to_string(), "bar".to_string()], command.tags);
    }
    #[test]
    fn parse_no_whitespace_in_tags() {
        let command = HoardCmd::default().with_tags_raw("foo,bar");
        assert_eq!(vec!["foo".to_string(), "bar".to_string()], command.tags);
    }

    #[test]
    fn parse_multiple_whitespace_in_tags() {
        let command = HoardCmd::default().with_tags_raw("foo,   bar");
        assert_eq!(vec!["foo".to_string(), "bar".to_string()], command.tags);
    }

    #[test]
    fn parse_special_characters_in_tags() {
        let command = HoardCmd::default().with_tags_raw("foo@, bar#");
        assert_eq!(vec!["foo@".to_string(), "bar#".to_string()], command.tags);
    }

    #[test]
    fn parse_empty_string() {
        let command = HoardCmd::default().with_tags_raw("");
        assert_eq!(Vec::<String>::new(), command.tags);
    }

    #[test]
    fn parse_string_with_only_whitespaces() {
        let command = HoardCmd::default().with_tags_raw("   ");
        assert_eq!(Vec::<String>::new(), command.tags);
    }

    #[test]
    fn validation_errors() {
        assert_eq!(
            HoardCmd::is_command_valid(""),
            Err(HoardError::InvalidCommand)
        );
        assert_eq!(HoardCmd::is_name_valid(""), Err(HoardError::EmptyName));
        assert_eq!(
            HoardCmd::is_name_valid("a b"),
            Err(HoardError::NameWithWhitespace)
        );
        assert_eq!(HoardCmd::are_tags_valid(""), Err(HoardError::EmptyTags));
        assert!(HoardCmd::is_command_valid("echo hi").is_ok());
    }
}
