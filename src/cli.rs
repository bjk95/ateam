use anyhow::Result;
use clap::builder::styling::{AnsiColor, Styles};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

const BANNER_LINES: &[&str] = &[
    " █████╗         ████████╗███████╗ █████╗ ███╗   ███╗",
    "██╔══██╗        ╚══██╔══╝██╔════╝██╔══██╗████╗ ████║",
    "███████║ █████╗    ██║   █████╗  ███████║██╔████╔██║",
    "██╔══██║ ╚════╝    ██║   ██╔══╝  ██╔══██║██║╚██╔╝██║",
    "██║  ██║           ██║   ███████╗██║  ██║██║ ╚═╝ ██║",
    "╚═╝  ╚═╝           ╚═╝   ╚══════╝╚═╝  ╚═╝╚═╝     ╚═╝",
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

    /// Fail fast if another `ateam` process holds the repo lock instead of waiting.
    #[arg(long, global = true)]
    pub no_wait: bool,

    /// Show extra detail (paths, SHAs, per-harness links).
    #[arg(short = 'v', long, global = true)]
    pub verbose: bool,

    /// Suppress non-error output (banner, success lines, progress).
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,
}

#[derive(Subcommand)]
pub enum Command {
    /// Bootstrap: scaffold a fresh ateam-config repo or clone an existing one.
    Init(InitArgs),

    /// Materialize the lockfile: install all active locked skills.
    Apply(ApplyArgs),

    /// Show what's locked vs installed vs drifted.
    Status,

    /// Manage skills.
    #[command(subcommand)]
    Skills(SkillsCommand),

    /// Manage per-machine project alias map.
    #[command(subcommand)]
    Project(ProjectCommand),

    /// Self-update: download the latest ateam release and replace this binary.
    Upgrade,

    /// Manage the ateam-config repo's git remote.
    #[command(subcommand)]
    Remote(RemoteCommand),

    /// Validate the instructions template against declared profiles.
    Validate,

    /// Open `$EDITOR` on the ateam state directory.
    Edit,

    /// Manage the instructions template (CLAUDE.md / AGENTS.md source).
    #[command(subcommand)]
    Instructions(InstructionsCommand),

    /// Manage which AI harnesses ateam syncs to (claude-code, codex, opencode, gemini).
    #[command(subcommand)]
    Harness(HarnessCommand),

    /// Manage subagents (single-file `.md` agents installed under `.claude/agents/` and `.codex/agents/`).
    #[command(subcommand)]
    Subagents(SubagentsCommand),
}

#[derive(Subcommand)]
pub enum HarnessCommand {
    /// List every registered harness and whether it's enabled on this repo.
    List,

    /// Enable one or more harnesses (writes to ateam.toml and re-applies).
    Add {
        /// Harness ids to enable. See `ateam harness list` for valid ids.
        #[arg(required = true)]
        ids: Vec<String>,
    },

    /// Disable one or more harnesses (writes to ateam.toml and re-applies).
    Remove {
        /// Harness ids to disable. See `ateam harness list` for valid ids.
        #[arg(required = true)]
        ids: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum InstructionsCommand {
    /// Open the instructions template in `$EDITOR`.
    Edit,

    /// Show a unified diff of new render vs current CLAUDE.md / AGENTS.md.
    Diff,

    /// Print the rendered instructions for each enabled tool to stdout.
    Show,
}

#[derive(Subcommand)]
pub enum SubagentsCommand {
    /// Install a subagent (single .md file) from a source repo or local path.
    Add(SubagentAddArgs),

    /// Remove a subagent from the lockfile and uninstall its symlinks.
    Remove(SubagentRemoveArgs),

    /// List locked subagents with their sources.
    List,
}

#[derive(Parser)]
pub struct SubagentAddArgs {
    /// owner/repo shorthand, full git URL, or local path (file or repo dir).
    pub source: String,

    /// Subagent name(s) to install. Looks for `agents/<name>.md` in the source
    /// by default; override with --path. Repeatable.
    #[arg(long, value_name = "NAME")]
    pub subagent: Vec<String>,

    /// Explicit path within the source repo. Implies a single subagent;
    /// the name is derived from the file stem unless --subagent is also given.
    #[arg(long, value_name = "PATH")]
    pub path: Option<String>,

    /// Target harnesses. Repeatable. `*` = all enabled with subagent support.
    #[arg(short = 'a', long = "harness", value_name = "NAME")]
    pub harnesses: Vec<String>,

    /// Skip confirmation prompts (non-interactive).
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Annotate lockfile entry with profile gates.
    #[arg(long, value_name = "NAME")]
    pub profile: Vec<String>,

    /// Pin to a specific git ref/tag/commit.
    #[arg(long)]
    pub r#ref: Option<String>,

    /// Permit `openclaw/*` sources.
    #[arg(long = "dangerously-accept-openclaw-risks")]
    pub dangerously_accept_openclaw_risks: bool,
}

#[derive(Parser)]
pub struct SubagentRemoveArgs {
    /// Subagent names to remove. Repeatable.
    #[arg(value_name = "NAME", required = true)]
    pub names: Vec<String>,

    /// Skip confirmation prompts.
    #[arg(short = 'y', long)]
    pub yes: bool,
}

#[derive(Subcommand)]
pub enum SkillsCommand {
    /// Install a skill package (Vercel-compatible flag surface).
    Add(AddArgs),

    /// Refresh tree SHAs from upstream and refetch any drifted skills.
    Update(UpdateArgs),

    /// Remove a skill from the lockfile and uninstall it.
    Remove(RemoveArgs),

    /// List locked skills with their sources.
    List(ListArgs),

    /// Adopt locally-installed skills (and global instructions) into the lockfile.
    /// With no arguments: bulk-import every skill in ~/.claude/skills,
    /// ~/.codex/skills, ~/.agents/skills plus the global CLAUDE.md / AGENTS.md.
    Import(ImportArgs),

    /// Deactivate a skill: keep its lockfile entry but unlink it from harnesses.
    Deactivate(DeactivateArgs),

    /// Reactivate a previously-deactivated skill.
    Activate(ActivateArgs),

    /// Print a skill's SKILL.md contents to stdout.
    Show(ShowArgs),

    /// Search the skills.sh registry.
    Find(FindArgs),
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

    /// Vercel-compat: implies --skill "*", --harness "*", and -y.
    #[arg(long)]
    pub all: bool,

    /// Target harnesses. Repeatable. `*` = all enabled.
    #[arg(short = 'a', long = "harness", value_name = "NAME")]
    pub harnesses: Vec<String>,

    /// Skip confirmation prompts (non-interactive).
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Force global scope (Vercel-compat). Overrides cwd auto-detection.
    #[arg(short = 'g', long)]
    pub global: bool,

    /// Annotate lockfile entry with profile gates.
    #[arg(long, value_name = "NAME")]
    pub profile: Vec<String>,

    /// Install into a registered project's native harness dirs.
    #[arg(long, value_name = "ALIAS")]
    pub project: Option<String>,

    /// Pin to a specific git ref/tag/commit.
    #[arg(long)]
    pub r#ref: Option<String>,

    /// Install via recursive copy instead of symlink. For filesystems that
    /// don't handle symlinks reliably (network mounts, some Docker volumes).
    #[arg(long)]
    pub copy: bool,

    /// Permit `openclaw/*` sources, which can shell out at runtime.
    #[arg(long = "dangerously-accept-openclaw-risks")]
    pub dangerously_accept_openclaw_risks: bool,
}

#[derive(Parser)]
pub struct ApplyArgs {
    /// Show planned writes without making them.
    #[arg(long)]
    pub dry_run: bool,

    /// Restrict to specific harnesses.
    #[arg(short = 'a', long = "harness", value_name = "NAME")]
    pub harnesses: Vec<String>,

    /// Restrict to one project's entries.
    #[arg(long, value_name = "ALIAS")]
    pub project: Option<String>,

    /// Move existing real dirs aside instead of refusing.
    #[arg(long)]
    pub force: bool,

    /// Install via recursive copy instead of symlink. For filesystems that
    /// don't handle symlinks reliably (network mounts, some Docker volumes).
    #[arg(long)]
    pub copy: bool,
}

#[derive(Parser)]
pub struct UpdateArgs {
    /// Specific skill names to update. Empty = all.
    pub names: Vec<String>,

    /// Skip confirmation prompts (non-interactive).
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Restrict to entries without a project scope (global-only).
    #[arg(short = 'g', long, conflicts_with = "project")]
    pub global: bool,

    /// Restrict to entries scoped to a registered project alias.
    #[arg(long, value_name = "ALIAS")]
    pub project: Option<String>,
}

#[derive(Parser)]
pub struct RemoveArgs {
    /// Skill names to remove. Repeatable. Use `--all` to target every locked skill.
    #[arg(value_name = "NAME", conflicts_with = "all")]
    pub names: Vec<String>,

    /// Remove every locked skill (within --harness / --global scope if provided).
    #[arg(long)]
    pub all: bool,

    /// Skip confirmation prompts (non-interactive).
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Restrict to entries targeting these harnesses. Repeatable. `*` = all enabled.
    #[arg(short = 'a', long = "harness", value_name = "NAME")]
    pub harnesses: Vec<String>,

    /// Force global scope (Vercel-compat). Restrict to entries with no project alias.
    #[arg(short = 'g', long)]
    pub global: bool,
}

#[derive(Parser)]
pub struct DeactivateArgs {
    pub name: String,
}

#[derive(Parser)]
pub struct ActivateArgs {
    pub name: String,
}

#[derive(Parser)]
pub struct ShowArgs {
    pub name: String,
}

#[derive(Parser)]
pub struct FindArgs {
    /// Search query terms.
    pub query: Vec<String>,
}

#[derive(Parser)]
pub struct ListArgs {
    /// Show only entries scoped to this project alias.
    #[arg(long, value_name = "ALIAS")]
    pub project: Option<String>,

    /// Emit a versioned JSON document instead of styled text.
    #[arg(long, conflicts_with = "names")]
    pub json: bool,

    /// Print only skill names, one per line, with no styling. For piping into
    /// `ateam skills remove`.
    #[arg(long)]
    pub names: bool,
}

#[derive(Parser)]
pub struct ImportArgs {
    /// Skill name to adopt. Omit for bulk import (every skill on disk + instructions).
    pub name: Option<String>,

    /// Only import the global ~/.claude/CLAUDE.md and ~/.codex/AGENTS.md as the
    /// instructions template — skip skills.
    #[arg(long, conflicts_with_all = ["upstream", "project"])]
    pub instructions: bool,

    /// Override detected upstream source. Single-skill mode only.
    #[arg(long, value_name = "SOURCE")]
    pub upstream: Option<String>,

    /// Tag the imported entry with a project alias. Single-skill mode only.
    #[arg(long, value_name = "ALIAS")]
    pub project: Option<String>,
}

#[derive(Subcommand)]
pub enum RemoteCommand {
    /// Set `origin` to the given git URL and push the current branch upstream.
    Add { url: String },
    /// Print the current remote(s).
    List,
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
    let no_wait = cli.no_wait;

    // Mutating commands take an exclusive flock on `<repo>/.ateam/lock` to
    // serialize concurrent read-modify-write of the lockfile and manifest.
    // `init` bootstraps the repo so it has no repo to lock against; read-only
    // commands don't mutate state.
    let _lock = if is_mutating(&cli.command) {
        let repo = crate::paths::resolve_repo()?;
        Some(crate::repo_lock::RepoLock::acquire(&repo, no_wait)?)
    } else {
        None
    };

    match cli.command {
        Command::Init(args) => crate::commands::init::run(args),
        Command::Apply(args) => crate::commands::apply::run(args, no_sync),
        Command::Status => crate::commands::status::run(),
        Command::Skills(cmd) => match cmd {
            SkillsCommand::Add(args) => crate::commands::add::run(args, no_sync),
            SkillsCommand::Update(args) => crate::commands::update::run(args, no_sync),
            SkillsCommand::Remove(args) => crate::commands::remove::run(args, no_sync),
            SkillsCommand::List(args) => crate::commands::list::run(args),
            SkillsCommand::Import(args) => crate::commands::import::run(args, no_sync),
            SkillsCommand::Deactivate(args) => crate::commands::deactivate::run(args, no_sync),
            SkillsCommand::Activate(args) => crate::commands::activate::run(args, no_sync),
            SkillsCommand::Show(args) => crate::commands::show::run(args),
            SkillsCommand::Find(args) => crate::commands::find::run(args, no_sync),
        },
        Command::Project(cmd) => crate::commands::project::run(cmd),
        Command::Upgrade => crate::self_update::force_upgrade(),
        Command::Remote(cmd) => crate::commands::remote::run(cmd),
        Command::Validate => crate::commands::validate::run(),
        Command::Edit => crate::commands::edit::run(no_sync),
        Command::Instructions(cmd) => crate::commands::instructions::run(cmd, no_sync),
        Command::Harness(cmd) => crate::commands::harness::run(cmd, no_sync),
        Command::Subagents(cmd) => match cmd {
            SubagentsCommand::Add(args) => crate::commands::subagents::add(args, no_sync),
            SubagentsCommand::Remove(args) => crate::commands::subagents::remove(args, no_sync),
            SubagentsCommand::List => crate::commands::subagents::list(),
        },
    }
}

fn is_mutating(cmd: &Command) -> bool {
    match cmd {
        Command::Init(_) | Command::Status | Command::Upgrade | Command::Validate => false,
        Command::Apply(_) | Command::Edit => true,
        Command::Skills(s) => match s {
            SkillsCommand::Add(_)
            | SkillsCommand::Update(_)
            | SkillsCommand::Remove(_)
            | SkillsCommand::Import(_)
            | SkillsCommand::Deactivate(_)
            | SkillsCommand::Activate(_) => true,
            SkillsCommand::List(_) | SkillsCommand::Show(_) | SkillsCommand::Find(_) => false,
        },
        Command::Project(p) => match p {
            ProjectCommand::Add { .. } | ProjectCommand::Remove { .. } => true,
            ProjectCommand::List => false,
        },
        Command::Remote(r) => match r {
            RemoteCommand::Add { .. } => true,
            RemoteCommand::List => false,
        },
        Command::Instructions(i) => match i {
            InstructionsCommand::Edit => true,
            InstructionsCommand::Diff | InstructionsCommand::Show => false,
        },
        Command::Harness(a) => match a {
            HarnessCommand::Add { .. } | HarnessCommand::Remove { .. } => true,
            HarnessCommand::List => false,
        },
        Command::Subagents(s) => match s {
            SubagentsCommand::Add(_) | SubagentsCommand::Remove(_) => true,
            SubagentsCommand::List => false,
        },
    }
}
