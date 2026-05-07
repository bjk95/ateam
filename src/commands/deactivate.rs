use crate::cli::DeactivateArgs;
use crate::git_sync;
use crate::install;
use crate::lockfile::Lockfile;
use crate::manifest::Manifest;
use crate::paths;
use anyhow::{bail, Result};

pub fn run(args: DeactivateArgs, no_sync: bool) -> Result<()> {
    let repo = paths::resolve_repo()?;

    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    let mut lock = Lockfile::load(&repo)?;
    let idx = match lock.skills.iter().position(|s| s.name == args.name) {
        Some(i) => i,
        None => bail!("no skill named `{}` in lockfile", args.name),
    };
    if !lock.skills[idx].active {
        println!("agents: `{}` already deactivated", args.name);
        return Ok(());
    }
    lock.skills[idx].active = false;
    lock.write(&repo)?;

    // Unlink any symlinks the manifest tracked for this skill, and drop them
    // from the manifest so a future `apply` doesn't try to re-create them.
    let mut manifest = Manifest::load(&repo)?;
    let to_remove: Vec<_> = manifest
        .entries
        .iter()
        .filter(|m| m.skill == args.name)
        .map(|m| m.path.clone())
        .collect();
    for path in &to_remove {
        if let Err(e) = install::uninstall_path(path) {
            eprintln!("agents: warning — couldn't remove {}: {:#}", path.display(), e);
        }
    }
    manifest.entries.retain(|m| m.skill != args.name);
    manifest.write(&repo)?;

    if git_sync::enabled(no_sync) {
        let msg = git_sync::msg_deactivate(&args.name);
        let _ = git_sync::commit_and_push(&repo, &msg);
    }

    println!("agents: deactivated `{}`", args.name);
    Ok(())
}
