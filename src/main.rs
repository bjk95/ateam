mod cli;
mod commands;
mod config;
mod discover;
mod git_sync;
mod install;
mod instructions;
mod lockfile;
mod manifest;
mod paths;
mod repo_lock;
mod self_update;
mod source;
mod ui;
mod upstream;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("ateam=warn")),
        )
        .with_target(false)
        .without_time()
        .init();

    eprintln!();
    eprintln!("{}", cli::banner());
    eprintln!();

    let cli = cli::parse();
    ui::set_verbose(cli.verbose);

    if !matches!(cli.command, cli::Command::Upgrade) {
        self_update::maybe_check();
    }

    if let Err(e) = cli::dispatch(cli) {
        ui::fail(format!("{:#}", e));
        std::process::exit(1);
    }
}
