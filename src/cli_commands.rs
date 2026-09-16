use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Only used when `hoard` is run as shell plugin
    #[arg(long)]
    pub autocomplete: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Shows setting file paths
    Info {},

    /// Save a new shell command or Python script
    New {
        /// Name of the new command. Supplying it saves without any prompt, which
        /// also requires --command
        #[arg(short = 'n', long, value_name = "NAME")]
        name: Option<String>,

        /// Comma separated single word tags; without --name hoard prompts for them
        #[arg(short = 't', long, value_name = "TAGS")]
        tags: Option<String>,

        /// The command to save; omit it to be prompted for the command
        #[arg(short = 'c', long, value_name = "COMMAND")]
        command: Option<String>,

        /// Archive a UTF-8 Python file without prompts; requires name and description
        #[arg(long, value_name = "FILE", conflicts_with = "command", requires_all = ["name", "description"])]
        script: Option<PathBuf>,

        /// Namespace for the entry; defaults to the configured default namespace
        #[arg(long, value_name = "NAMESPACE")]
        namespace: Option<String>,

        /// Description/summary; with --name an omitted description stays empty,
        /// while Python scripts always require one
        #[arg(short = 'd', long, alias = "summary", value_name = "DESCRIPTION")]
        description: Option<String>,
    },

    /// List commands saved in trove.
    List {
        /// Apply filter to listed commands
        #[arg(short = 'f', long)]
        filter: Option<String>,

        /// Return hoarded commands in structured format
        #[arg(short = 'j', long)]
        json: bool,

        /// Return hoarded commands in a simplified table view
        #[arg(short = 's', long)]
        simple: bool,
    },

    /// Print a shell invocation for a saved command or Python script
    Pick {
        /// Name of the entry to print
        #[arg(short = 'n', long)]
        name: String,

        /// Print the exact stored source without a wrapper or parameter prompts
        #[arg(long)]
        raw: bool,
    },

    /// Set a custom parameter token
    SetParameterToken {
        /// Parameter token to replace
        #[arg(long)]
        name: String,
    },

    /// Removes a command in the trove by name
    Remove {
        /// command to remove
        #[arg(short = 'n', long)]
        name: String,
    },

    /// Remove all commands of a namespace
    RemoveNamespace {
        /// Namespace to remove
        #[arg(short = 'n', long)]
        namespace: String,
    },

    /// Import a trove file from a local file or URL
    Import {
        /// URL or path to .trove file to import
        #[arg(long)]
        uri: String,
    },

    /// Export a trove file
    Export {
        /// Path where the trove file should be saved to
        #[arg(long)]
        path: String,

        /// Export every entry without prompting
        #[arg(long, conflicts_with = "namespace")]
        all: bool,

        /// Export only these namespaces (comma separated or repeated) without prompting
        #[arg(long = "namespace", value_name = "NAMESPACE", value_delimiter = ',')]
        namespace: Vec<String>,
    },

    /// Edit a saved command
    Edit {
        /// Name of the command to edit
        #[arg(short = 'n', long)]
        name: String,
    },

    /// Print shell config
    ShellConfig {
        /// shell type to print the config for
        #[arg(short = 's', long)]
        shell: String,
    },
}
