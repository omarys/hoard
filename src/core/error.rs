use thiserror::Error;

/// Errors raised while validating or managing hoard commands.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum HoardError {
    /// The command string is empty.
    #[error("Command can't be empty")]
    InvalidCommand,
    /// The command name is empty.
    #[error("Name can't be empty")]
    EmptyName,
    /// The command name contains whitespace.
    #[error("Name can't contain whitespaces")]
    NameWithWhitespace,
    /// The tag list is empty.
    #[error("Tags can't be empty")]
    EmptyTags,
    /// A command failed validation before being saved.
    #[error("cannot save invalid command")]
    InvalidCommandForSave,
}
