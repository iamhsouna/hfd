mod api;
mod cli;
mod commands;
mod config;
mod download;
mod progress;
mod store;
mod tui;
mod update;
mod util;

use anyhow::Result;
use clap::{CommandFactory, Parser};

use cli::{Cli, Command};

#[tokio::main]
async fn main() -> Result<()> {
    let args = with_default_command(std::env::args().collect());
    let cli = Cli::parse_from(args);
    let config = config::load();
    let settings = commands::Settings::new(&cli, &config);

    match cli.command {
        Some(Command::Download(args)) => commands::download(&settings, &args).await,
        Some(Command::Analyze(args)) => commands::analyze(&settings, &args).await,
        Some(Command::Search(args)) => commands::search(&settings, &args).await,
        Some(Command::List(args)) => commands::list(&settings, &args).await,
        Some(Command::Info(args)) => commands::info(&settings, &args).await,
        Some(Command::Config(args)) => commands::config_cmd(&args),
        Some(Command::Update(args)) => {
            update::run(update::UpdateOptions {
                check_only: args.check,
                force: args.force,
                version: args.tag.clone(),
            })
            .await
        }
        Some(Command::Version) => {
            println!("hfd {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        None => {
            Cli::command().print_help()?;
            println!();
            Ok(())
        }
    }
}

/// Allow `hfd owner/repo` as shorthand for `hfd download owner/repo`.
fn with_default_command(mut args: Vec<String>) -> Vec<String> {
    if args.len() <= 1 {
        return args;
    }
    let has_command = args
        .iter()
        .skip(1)
        .any(|arg| cli::KNOWN_COMMANDS.contains(&arg.as_str()) || arg == "help");
    let has_help = args
        .iter()
        .any(|arg| arg == "--help" || arg == "-h" || arg == "--version" || arg == "-V");

    if !has_command && !has_help {
        args.insert(1, "download".to_string());
    }
    args
}
