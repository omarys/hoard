// Kept clippy-clean at the default level: `cargo clippy --all-targets -- -D warnings`.
// (The pedantic/nursery profiles were never satisfied by this codebase and
// made the CI lint job fail on every commit.)

mod cli_commands;
mod config;
mod core;
mod filter;
mod gpt;
mod gui;
mod hoard;
mod store;
mod sync_models;
mod theme;
mod util;

use anyhow::Result;
use hoard::Hoard;

fn main() -> Result<()> {
    let mut hoard = Hoard::load(None)?;
    let (command, is_autocomplete) = hoard.start()?;
    let output = command.trim();
    if is_autocomplete {
        eprintln!("{output}");
    } else {
        println!("{output}");
    }
    Ok(())
}
