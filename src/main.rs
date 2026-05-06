mod cli;
mod commands;
mod config;
mod paths;

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
    cli::dispatch(cli)
}
