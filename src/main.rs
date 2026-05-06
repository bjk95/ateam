mod agents;
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

    let cli = cli::parse();
    ui::set_verbose(cli.verbose);
    // `--quiet` and `skills list --json` both suppress the banner and any
    // non-error UI output. JSON consumers (editor extensions) need stdout to
    // be a clean document; quiet users want a silent run.
    let json_list = matches!(
        &cli.command,
        cli::Command::Skills(cli::SkillsCommand::List(args)) if args.json
    );
    let suppress_banner = cli.quiet || json_list;
    ui::set_quiet(cli.quiet || json_list);

    if !suppress_banner {
        eprintln!();
        eprintln!("{}", cli::banner());
        eprintln!();
    }

    if !matches!(cli.command, cli::Command::Upgrade) {
        self_update::maybe_check();
    }

    if let Err(e) = cli::dispatch(cli) {
        ui::fail(format!("{:#}", e));
        std::process::exit(1);
    }
}
