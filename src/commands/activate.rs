use crate::cli::{ActivateArgs, ApplyArgs};
use crate::git_sync;
use crate::lockfile::Lockfile;
use crate::paths;
use anyhow::{bail, Result};

pub fn run(args: ActivateArgs, no_sync: bool) -> Result<()> {
    let repo = paths::resolve_repo()?;

    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    let mut lock = Lockfile::load(&repo)?;
    let idx = match lock.skills.iter().position(|s| s.name == args.name) {
        Some(i) => i,
        None => bail!("no skill named `{}` in lockfile", args.name),
    };
    if lock.skills[idx].active {
        println!("ateam: `{}` already active", args.name);
        return Ok(());
    }
    lock.skills[idx].active = true;
    lock.write(&repo)?;

    // Re-materialize. apply already handles cold-cache refetch and is idempotent
    // for entries that are still active; pass --no-sync to avoid double-pull.
    crate::commands::apply::run(
        ApplyArgs {
            dry_run: false,
            agents: Vec::new(),
            project: None,
            force: false,
            copy: false,
        },
        true,
    )?;

    if git_sync::enabled(no_sync) {
        let msg = git_sync::msg_activate(&args.name);
        let _ = git_sync::commit_and_push(&repo, &msg);
    }

    println!("ateam: activated `{}`", args.name);
    Ok(())
}
