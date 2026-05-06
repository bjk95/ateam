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

    // Wipe cache copy if there is one. Local sources stay on disk.
    if !removed.source.starts_with("local:") {
        let cache = paths::cache_dir(&repo).join(&args.name);
        if cache.exists() {
            let _ = std::fs::remove_dir_all(&cache);
        }
    }

    if git_sync::enabled(no_sync) {
        let msg = git_sync::msg_remove(&args.name);
        let _ = git_sync::commit_and_push(&repo, &msg);
    }

    ui::ok(format!("removed {}", args.name));
    Ok(())
}
