use anyhow::Result;
use clap::builder::styling::{AnsiColor, Styles};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

const BANNER_LINES: &[&str] = &[
    " █████╗ ████████╗███████╗ █████╗ ███╗   ███╗",
    "██╔══██╗╚══██╔══╝██╔════╝██╔══██╗████╗ ████║",
    "███████║   ██║   █████╗  ███████║██╔████╔██║",
    "██╔══██║   ██║   ██╔══╝  ██╔══██║██║╚██╔╝██║",
    "██║  ██║   ██║   ███████╗██║  ██║██║ ╚═╝ ██║",
    "╚═╝  ╚═╝   ╚═╝   ╚══════╝╚═╝  ╚═╝╚═╝     ╚═╝",
];

// Subtle truecolor gradient from darker teal-cyan (top) to lighter cyan (bottom).
// Six rows, one RGB tuple each. Modern terminals (iTerm2, Ghostty, Warp, VS Code,
// Terminal.app) all render 24-bit color; older ones may render the nearest 256-color
// approximation, which still preserves the shading direction.
const BANNER_GRADIENT: &[(u8, u8, u8)] = &[
    (0, 156, 178),
    (0, 178, 198),
    (0, 198, 215),
    (28, 215, 230),
    (60, 230, 240),
    (95, 245, 250),
];

pub fn banner() -> String {
    let use_color = console::colors_enabled();
    BANNER_LINES
        .iter()
        .zip(BANNER_GRADIENT.iter())
        .map(|(line, (r, g, b))| {
            if use_color {
                // Truecolor + bold: ESC[1;38;2;R;G;Bm  …  ESC[0m
                format!("\x1b[1;38;2;{};{};{}m{}\x1b[0m", r, g, b, line)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// True when the invocation will land in clap's help/error renderer
/// (bare `ateam`, `--help`, `-h`). Banner is printed before clap takes
/// over so its rendering doesn't touch cursor state in a way that
/// overwrites lines above (some terminals + clap's error path
/// interact badly with `before_help`).
pub fn shows_help() -> bool {
    let mut args = std::env::args().skip(1);
    let first = match args.next() {
        Some(a) => a,
        None => return true, // bare invocation
    };
    matches!(first.as_str(), "--help" | "-h" | "help")
}

pub fn parse() -> Cli {
    Cli::parse()
}

const HELP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Cyan.on_default().bold())
    .usage(AnsiColor::Cyan.on_default().bold())
    .literal(AnsiColor::White.on_default().bold())
    .placeholder(AnsiColor::Cyan.on_default());

#[derive(Parser)]
#[command(
    name = "ateam",
    version,
    about = "Multi-machine AI skills sync",
    styles = HELP_STYLES,
)]
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
