use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "ateam", version, about = "Multi-machine AI skills sync")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Skip auto pull/commit/push for this invocation.
    #[arg(long, global = true)]
    pub no_sync: bool,

    /// Show extra detail (paths, SHAs, per-agent links).
    #[arg(short = 'v', long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand)]
pub enum Command {
    /// Bootstrap: scaffold a fresh ateam-config repo or clone an existing one.
    Init(InitArgs),

    /// Install a skill package (Vercel-compatible flag surface).
    Add(AddArgs),

    /// Materialize the lockfile: install all locked skills.
    Apply(ApplyArgs),

    /// Refresh tree SHAs from upstream and refetch any drifted skills.
    Update(UpdateArgs),

    /// Remove a skill from the lockfile and uninstall it.
    Remove(RemoveArgs),

    /// List locked skills with their sources.
    List(ListArgs),

    /// Show what's locked vs installed vs drifted.
    Status,

    /// Adopt a locally-installed skill into the lockfile.
    Import(ImportArgs),

    /// Manage per-machine project alias map.
    #[command(subcommand)]
    Project(ProjectCommand),

    /// Self-update: download the latest ateam release and replace this binary.
    Upgrade,
}

#[derive(Parser)]
pub struct InitArgs {
    /// Git URL to clone. Mutually exclusive with --scaffold.
    pub git_url: Option<String>,

    /// Scaffold a fresh empty repo instead of cloning.
    #[arg(long)]
    pub scaffold: bool,

    /// Override default repo location (~/.config/ateam/). Writes pointer file.
    #[arg(long)]
    pub repo: Option<PathBuf>,

    /// Comma-separated profile list for this machine.
    #[arg(long, value_delimiter = ',')]
    pub profiles: Vec<String>,
}

#[derive(Parser)]
pub struct AddArgs {
    /// owner/repo shorthand, full git URL, or local path.
    pub source: String,

    /// List discovered skills in the source instead of installing.
    #[arg(long)]
    pub list: bool,

    /// Specific skill names to install. Supports `*` for all.
    #[arg(long, value_name = "NAME")]
    pub skill: Vec<String>,

    /// Equivalent to --skill '*'.
    #[arg(long)]
    pub all: bool,

    /// Target agents. Repeatable. `*` = all enabled.
    #[arg(short = 'a', long = "agent", value_name = "NAME")]
    pub agents: Vec<String>,

    /// Skip confirmation prompts (non-interactive).
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Force global scope (Vercel-compat). Overrides cwd auto-detection.
    #[arg(short = 'g', long)]
    pub global: bool,

    /// Annotate lockfile entry with profile gates.
    #[arg(long, value_name = "NAME")]
    pub profile: Vec<String>,

    /// Install into a registered project's native agent dirs.
    #[arg(long, value_name = "ALIAS")]
    pub project: Option<String>,

    /// Pin to a specific git ref/tag/commit.
    #[arg(long)]
    pub r#ref: Option<String>,
}

#[derive(Parser)]
pub struct ApplyArgs {
    /// Show planned writes without making them.
    #[arg(long)]
    pub dry_run: bool,

    /// Restrict to specific agents.
    #[arg(short = 'a', long = "agent", value_name = "NAME")]
    pub agents: Vec<String>,

    /// Restrict to one project's entries.
    #[arg(long, value_name = "ALIAS")]
    pub project: Option<String>,

    /// Move existing real dirs aside instead of refusing.
    #[arg(long)]
    pub force: bool,
}

#[derive(Parser)]
pub struct UpdateArgs {
    /// Specific skill names to update. Empty = all.
    pub names: Vec<String>,
}

#[derive(Parser)]
pub struct RemoveArgs {
    pub name: String,
}

#[derive(Parser)]
pub struct ListArgs {
    /// Show only entries scoped to this project alias.
    #[arg(long, value_name = "ALIAS")]
    pub project: Option<String>,
}

#[derive(Parser)]
pub struct ImportArgs {
    pub name: String,

    /// Override detected upstream source.
    #[arg(long, value_name = "SOURCE")]
    pub upstream: Option<String>,

    /// Tag the imported entry with a project alias.
    #[arg(long, value_name = "ALIAS")]
    pub project: Option<String>,
}

#[derive(Subcommand)]
pub enum ProjectCommand {
    /// Add or update an alias→path mapping for this machine.
    #[command(alias = "register")]
    Add { alias: String, path: PathBuf },
    /// Show this machine's alias→path map.
    List,
    /// Remove an alias from this machine's map.
    #[command(alias = "rm")]
    Remove { alias: String },
}

pub fn dispatch(cli: Cli) -> Result<()> {
    let no_sync = cli.no_sync;
    match cli.command {
        Command::Init(args) => crate::commands::init::run(args),
        Command::Add(args) => crate::commands::add::run(args, no_sync),
        Command::Apply(args) => crate::commands::apply::run(args, no_sync),
        Command::Update(args) => crate::commands::update::run(args, no_sync),
        Command::Remove(args) => crate::commands::remove::run(args, no_sync),
        Command::List(args) => crate::commands::list::run(args),
        Command::Status => crate::commands::status::run(),
        Command::Import(args) => crate::commands::import::run(args, no_sync),
        Command::Project(cmd) => crate::commands::project::run(cmd),
        Command::Upgrade => crate::self_update::force_upgrade(),
    }
}
