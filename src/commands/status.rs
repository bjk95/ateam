use crate::config::MachineConfig;
use crate::lockfile::Lockfile;
use crate::manifest::Manifest;
use crate::paths;
use anyhow::Result;

pub fn run() -> Result<()> {
    let repo = paths::resolve_repo()?;
    let lock = Lockfile::load(&repo)?;
    let manifest = Manifest::load(&repo)?;
    let machine = MachineConfig::load(&repo)?;

    println!("repo: {}", repo.display());
    println!("profiles: [{}]", machine.profiles.join(", "));
    if !machine.projects.is_empty() {
        println!("projects:");
        for (alias, path) in &machine.projects {
            println!("  - {} → {}", alias, path.display());
        }
    }
    println!();

    println!("locked skills: {}", lock.skills.len());
    println!("manifest entries: {}", manifest.entries.len());

    let mut dangling = 0usize;
    for entry in &manifest.entries {
        match std::fs::symlink_metadata(&entry.path) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    if let Ok(target) = std::fs::read_link(&entry.path) {
                        if !target.exists() {
                            dangling += 1;
                        }
                    }
                }
            }
            Err(_) => dangling += 1,
        }
    }
    if dangling > 0 {
        println!("dangling/missing symlinks: {} (run `ateam apply` to repair)", dangling);
    } else {
        println!("symlinks: clean");
    }

    Ok(())
}
