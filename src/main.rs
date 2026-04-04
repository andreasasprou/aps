mod cli;
mod common;
mod costs;
mod profiles;
mod auth;
mod ui;
mod usage;

use clap::Parser;
use cli::{Cli, Commands, LabelCommands};

fn main() {
    let cli = Cli::parse();
    let plain = cli.plain || std::env::var("NO_COLOR").is_ok();

    if plain {
        colored::control::set_override(false);
    }

    let result = match cli.command {
        Commands::Save { tool, from_token, from_refresh_token, label } => {
            profiles::save(&tool, from_token.as_deref(), from_refresh_token.as_deref(), label.as_deref())
        }
        Commands::Load { tool } => profiles::load(&tool),
        Commands::List { tool } => profiles::list(tool.as_deref()),
        Commands::Current { tool } => profiles::current(tool.as_deref()),
        Commands::Status { all, tool } => usage::status(all, tool.as_deref()),
        Commands::Delete { tool } => profiles::delete(&tool),
        Commands::Label { command } => match command {
            LabelCommands::Set { tool, id, label } => profiles::label_set(&tool, &id, &label),
            LabelCommands::Clear { tool, id } => profiles::label_clear(&tool, &id),
            LabelCommands::Rename { tool, from, to } => profiles::label_rename(&tool, &from, &to),
        },
        Commands::Costs => costs::costs(),
        Commands::Doctor => profiles::doctor(),
    };

    if let Err(e) = result {
        ui::print_error(&format!("{:#}", e));
        std::process::exit(1);
    }
}
