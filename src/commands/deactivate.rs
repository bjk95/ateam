use crate::cli::DeactivateArgs;
use crate::git_sync;
use crate::install;
use crate::lockfile::Lockfile;
use crate::manifest::{EntryKind, Manifest};
use crate::paths;
use crate::ui;
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
        ui::plain(format!("agents: `{}` already deactivated", args.name));
        return Ok(());
    }
    lock.skills[idx].active = false;
    lock.write(&repo)?;

    // Uninstall any paths the manifest tracked for this skill, and drop them
    // from the manifest so a future `apply` doesn't try to re-create them.
    let mut manifest = Manifest::load(&repo)?;
    let to_remove: Vec<_> = manifest
        .entries
        .iter()
        .filter(|m| m.skill == args.name)
        .cloned()
        .collect();
    for entry in &to_remove {
        let result = match entry.kind {
            EntryKind::Symlink => install::uninstall_path(&entry.path),
            EntryKind::Copy => install::uninstall_copy(&entry.path),
        };
        if let Err(e) = result {
            ui::warn(format!("couldn't remove {}: {:#}", entry.path.display(), e));
        }
    }
    manifest.entries.retain(|m| m.skill != args.name);
    manifest.write(&repo)?;

    if git_sync::enabled(no_sync) {
        let msg = git_sync::msg_deactivate(&args.name);
        if let Err(e) = git_sync::commit_and_push(&repo, &msg) {
            ui::warn(format!("auto-sync failed: {:#}", e));
            ui::detail("local change saved; rerun a mutating command to retry");
        }
    }

    ui::plain(format!("agents: deactivated `{}`", args.name));
    Ok(())
}
