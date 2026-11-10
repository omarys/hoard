use clap::{Args, Parser, Subcommand, ValueEnum};
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

        /// Return only the single best match for --filter
        #[arg(long)]
        best: bool,

        /// Print only the stored command of the best match, for piping
        #[arg(long)]
        raw: bool,
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

    /// Find and import commands worth hoarding from shell history and agent sessions
    History {
        #[command(subcommand)]
        action: HistoryCommands,
    },
}

/// Where `hoard history` looks for commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum HistorySource {
    /// zsh history ($HISTFILE, else ~/.zsh_history)
    Zsh,
    /// pi / pi-bolt agent session transcripts
    Pi,
    /// Both sources
    #[default]
    All,
}

impl HistorySource {
    /// Whether the zsh history file is read.
    pub fn includes_zsh(self) -> bool {
        matches!(self, Self::Zsh | Self::All)
    }

    /// Whether agent session transcripts are read.
    pub fn includes_pi(self) -> bool {
        matches!(self, Self::Pi | Self::All)
    }
}

/// Flags shared by `hoard history scan` and `hoard history import`.
#[derive(Debug, Default, Clone, Args)]
pub struct HistoryArgs {
    /// Which sources to read
    #[arg(long, value_enum, default_value_t = HistorySource::All)]
    pub source: HistorySource,

    /// Shell history file; defaults to $HISTFILE, else ~/.zsh_history
    #[arg(long, value_name = "PATH")]
    pub histfile: Option<PathBuf>,

    /// Agent session directory; repeatable. Defaults to ~/.pi/agent/sessions
    /// and ~/.pi-bolt/agent/sessions when they exist
    #[arg(long, value_name = "PATH")]
    pub sessions_dir: Vec<PathBuf>,

    /// Secret scanning: auto (use trufflehog when installed), off, or a binary path
    #[arg(long, value_name = "auto|off|PATH", default_value = "auto")]
    pub trufflehog: String,

    /// Maximum number of candidates to consider
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,
}

/// Subcommands of `hoard history`.
#[derive(Subcommand)]
pub enum HistoryCommands {
    /// Report hoardable commands without changing anything
    Scan {
        #[command(flatten)]
        args: HistoryArgs,

        /// Print the report as a single JSON object
        #[arg(long)]
        json: bool,
    },

    /// Add commands from shell history and agent sessions to the trove
    Import {
        #[command(flatten)]
        args: HistoryArgs,

        /// Namespace for every imported entry instead of the per-source defaults
        #[arg(long, value_name = "NAMESPACE")]
        namespace: Option<String>,

        /// Import every candidate without prompting
        #[arg(long)]
        all: bool,

        /// Report what would be imported without prompting or writing anything
        #[arg(long)]
        dry_run: bool,
    },
}
