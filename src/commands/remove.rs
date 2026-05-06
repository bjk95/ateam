use crate::cli::RemoveArgs;
use crate::git_sync;
use crate::install;
use crate::lockfile::Lockfile;
use crate::manifest::Manifest;
use crate::paths;
use crate::ui;
use anyhow::{bail, Result};

pub fn run(args: RemoveArgs, no_sync: bool) -> Result<()> {
    let repo = paths::resolve_repo()?;

    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    let mut lock = Lockfile::load(&repo)?;
    let removed = match lock.remove(&args.name) {
        Some(e) => e,
        None => bail!("no skill named `{}` in lockfile", args.name),
    };
    lock.write(&repo)?;

    // Remove any symlinks the manifest tracked for this skill.
    let mut manifest = Manifest::load(&repo)?;
    let to_remove: Vec<_> = manifest
        .entries
        .iter()
        .filter(|m| m.skill == args.name)
        .map(|m| m.path.clone())
        .collect();
    for path in &to_remove {
        if let Err(e) = install::uninstall_path(path) {
            ui::warn(format!(
                "couldn't remove {}: {:#}",
                paths::display_path(path),
                e
            ));
        }
    }
    manifest.entries.retain(|m| m.skill != args.name);
    manifest.write(&repo)?;

    // Wipe the synced snapshot. Local sources whose path points elsewhere
    // (e.g., a hand-authored skill the user keeps in their own dir) stay put;
    // the snapshot under skills/<name>/ is the only thing ateam manages.
    let snapshot = paths::local_skills_dir(&repo).join(&args.name);
    let snapshot_managed = if removed.source.starts_with("local:") {
        // Only delete if source explicitly pointed at skills/<name>.
        removed.source == format!("local:skills/{}", args.name)
    } else {
        true
    };
    if snapshot_managed && snapshot.exists() {
        let _ = std::fs::remove_dir_all(&snapshot);
    }
    // Legacy cache copy from before the snapshot-into-skills/ migration.
    let legacy_cache = paths::cache_dir(&repo).join(&args.name);
    if legacy_cache.exists() {
        let _ = std::fs::remove_dir_all(&legacy_cache);
    }

    if git_sync::enabled(no_sync) {
        let msg = git_sync::msg_remove(&args.name);
        if let Err(e) = git_sync::commit_and_push(&repo, &msg) {
            ui::warn(format!("auto-sync failed: {:#}", e));
            ui::detail("local change saved; rerun a mutating command to retry");
        }
    }

    ui::ok(format!("removed {}", args.name));
    Ok(())
}
