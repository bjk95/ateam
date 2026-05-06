mod cli;
mod commands;
mod config;
mod discover;
mod git_sync;
mod install;
mod lockfile;
mod manifest;
mod paths;
mod self_update;
mod source;

use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("ateam=info,warn")),
        )
        .with_target(false)
        .without_time()
        .init();

    let cli = cli::Cli::parse();
    if !matches!(cli.command, cli::Command::Upgrade) {
        self_update::maybe_check();
    }
    cli::dispatch(cli)
}
