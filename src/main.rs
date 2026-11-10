// Kept clippy-clean at the default level: `cargo clippy --all-targets -- -D warnings`.
// (The pedantic/nursery profiles were never satisfied by this codebase and
// made the CI lint job fail on every commit.)

mod cli_commands;
mod config;
mod core;
mod filter;
mod gpt;
mod gui;
mod headless;
mod history;
mod hoard;
mod schema;
mod store;
mod theme;
mod util;

use clap::Parser;
use cli_commands::Cli;
use headless::{CliError, EXIT_PARSE, ErrorType, OutputMode};
use hoard::Hoard;
use std::process::ExitCode;

fn main() -> ExitCode {
    // A clap failure happens before `Cli` exists, so peek at argv to decide
    // whether the caller asked for machine-readable errors. Bundled shorts
    // (`-sj`) and `--flag=value` both count.
    let mut asked_for_json = false;
    let mut asked_for_compact = false;
    for arg in std::env::args().skip(1) {
        let (long, value) = match arg.split_once('=') {
            Some((flag, value)) => (flag, Some(value)),
            None => (arg.as_str(), None),
        };
        let enabled = value != Some("false") && value != Some("0");
        match long {
            "--json" if enabled => asked_for_json = true,
            "--compact" | "--tokens" if enabled => asked_for_compact = true,
            // `-sj`, `-j`, `-cj`: the flag cluster only exists before a value.
            short if short.starts_with('-') && !short.starts_with("--") => {
                let flags = short
                    .trim_start_matches('-')
                    .chars()
                    .take_while(|c| c.is_alphabetic());
                for flag in flags {
                    match flag {
                        'j' => asked_for_json = true,
                        'c' => asked_for_compact = true,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => return report_parse_error(err, asked_for_json, asked_for_compact),
    };

    let mode = OutputMode::resolve(cli.json, cli.compact);
    match run(cli, mode) {
        Ok(code) => code,
        // Every failure leaves through one door: a deterministic payload plus a
        // stable exit code, never a bare panic message.
        Err(err) => ExitCode::from(err.emit(mode) as u8),
    }
}

fn run(cli: Cli, mode: OutputMode) -> Result<ExitCode, CliError> {
    let mut hoard = Hoard::load(None, mode)?;
    let (command, is_autocomplete) = hoard.start(cli)?;

    // Headless callers get byte-clean output: no cursor movement, no colors.
    let trimmed = command.trim();
    let output = if mode.is_json() || !headless::stdout_is_tty() {
        headless::strip_ansi(trimmed)
    } else {
        trimmed.to_string()
    };
    if output.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    if is_autocomplete {
        eprintln!("{output}");
    } else {
        println!("{output}");
    }
    Ok(ExitCode::SUCCESS)
}

/// Turn a clap error into the uniform headless payload (or clap's own text).
fn report_parse_error(err: clap::Error, asked_for_json: bool, asked_for_compact: bool) -> ExitCode {
    use clap::error::ErrorKind;

    // Help and version are not failures. A missing subcommand is, even though
    // clap prints help for it.
    if matches!(
        err.kind(),
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
    ) {
        let _ = err.print();
        return ExitCode::SUCCESS;
    }

    if asked_for_json {
        let payload = CliError::new(ErrorType::ParseError, err.to_string().trim().to_string());
        let _ = payload.emit(OutputMode::resolve(asked_for_json, asked_for_compact));
        return ExitCode::from(EXIT_PARSE as u8);
    }

    let _ = err.print();
    ExitCode::from(EXIT_PARSE as u8)
}
