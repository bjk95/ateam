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

    // Fail fast if any name isn't in the lockfile — preserves the original
    // single-skill error semantics and avoids partial removal.
    let missing: Vec<&String> = args
        .names
        .iter()
        .filter(|n| !lock.skills.iter().any(|s| &&s.name == n))
        .collect();
    if !missing.is_empty() {
        let list = missing
            .iter()
            .map(|n| format!("`{}`", n))
            .collect::<Vec<_>>()
            .join(", ");
        bail!("no skill named {} in lockfile", list);
    }

    let mut manifest = Manifest::load(&repo)?;
    let mut removed_names: Vec<String> = Vec::with_capacity(args.names.len());

    for name in &args.names {
        let removed = match lock.remove(name) {
            Some(e) => e,
            None => continue, // already validated above; unreachable in practice
        };

        // Remove any symlinks the manifest tracked for this skill.
        let to_remove: Vec<_> = manifest
            .entries
            .iter()
            .filter(|m| &m.skill == name)
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
        manifest.entries.retain(|m| &m.skill != name);

        // Wipe the synced snapshot. Local sources whose path points elsewhere
        // (e.g., a hand-authored skill the user keeps in their own dir) stay put;
        // the snapshot under skills/<name>/ is the only thing ateam manages.
        let snapshot = paths::local_skills_dir(&repo).join(name);
        let snapshot_managed = if removed.source.starts_with("local:") {
            // Only delete if source explicitly pointed at skills/<name>.
            removed.source == format!("local:skills/{}", name)
        } else {
            true
        };
        if snapshot_managed && snapshot.exists() {
            if let Err(e) = std::fs::remove_dir_all(&snapshot) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    ui::warn(format!(
                        "couldn't remove {}: {:#}",
                        paths::display_path(&snapshot),
                        e
                    ));
                }
            }
        }
        // Legacy cache copy from before the snapshot-into-skills/ migration.
        let legacy_cache = paths::cache_dir(&repo).join(name);
        if legacy_cache.exists() {
            if let Err(e) = std::fs::remove_dir_all(&legacy_cache) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    ui::warn(format!(
                        "couldn't remove {}: {:#}",
                        paths::display_path(&legacy_cache),
                        e
                    ));
                }
            }
        }

        removed_names.push(name.clone());
    }

    lock.write(&repo)?;
    manifest.write(&repo)?;

    if git_sync::enabled(no_sync) {
        let msg = if removed_names.len() == 1 {
            git_sync::msg_remove(&removed_names[0])
        } else {
            git_sync::msg_remove_bulk(&removed_names)
        };
        if let Err(e) = git_sync::commit_and_push(&repo, &msg) {
            ui::warn(format!("auto-sync failed: {:#}", e));
            ui::detail("local change saved; rerun a mutating command to retry");
        }
    }

    for name in &removed_names {
        ui::ok(format!("removed {}", name));
    }
    Ok(())
}
