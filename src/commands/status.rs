use crate::config::MachineConfig;
use crate::git_sync;
use crate::lockfile::Lockfile;
use crate::manifest::Manifest;
use crate::paths;
use crate::ui;
use anyhow::Result;
use console::style;

pub fn run() -> Result<()> {
    let repo = paths::resolve_repo()?;
    let lock = Lockfile::load(&repo)?;
    let manifest = Manifest::load(&repo)?;
    let machine = MachineConfig::load(&repo)?;

    let dangling = count_dangling(&manifest);
    let unpushed = git_sync::unpushed_count(&repo).unwrap_or(0);

    // Headline: ✓ when healthy, ⚠ when dangling links or unpushed commits exist.
    let suffix = if machine.profiles.is_empty() {
        String::new()
    } else {
        format!(" · {}", machine.profiles.join(", "))
    };
    let headline = format!("ateam{}", suffix);
    if dangling == 0 && unpushed == 0 {
        ui::ok(&headline);
    } else {
        ui::warn(&headline);
    }

    // Body lines indented two spaces under the headline.
    let n = lock.skills.len();
    ui::plain(format!(
        "  {} {} installed",
        n,
        if n == 1 { "skill" } else { "skills" }
    ));
    if !machine.projects.is_empty() {
        let aliases: Vec<&str> = machine.projects.keys().map(|s| s.as_str()).collect();
        let pn = aliases.len();
        ui::plain(format!(
            "  {} {}: {}",
            pn,
            if pn == 1 { "project" } else { "projects" },
            aliases.join(", ")
        ));
    }
    if dangling > 0 {
        ui::plain(format!(
            "  {}  {} broken links — run: ateam apply",
            style("✗").red(),
            dangling
        ));
    }
    if unpushed > 0 {
        ui::plain(format!(
            "  {}  {} unpushed commit{} — will push on the next mutating command",
            style("⚠").yellow(),
            unpushed,
            if unpushed == 1 { "" } else { "s" }
        ));
    }

    ui::detail(format!("repo: {}", paths::display_path(&repo)));
    ui::detail(format!("manifest: {} entries", manifest.entries.len()));

    Ok(())
}

fn count_dangling(manifest: &Manifest) -> usize {
    let mut n = 0;
    for entry in &manifest.entries {
        match std::fs::symlink_metadata(&entry.path) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    if let Ok(target) = std::fs::read_link(&entry.path) {
                        if !target.exists() {
                            n += 1;
                        }
                    }
                }
            }
            Err(_) => n += 1,
        }
    }
    n
}
