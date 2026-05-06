use crate::cli::ProjectCommand;
use crate::config::MachineConfig;
use crate::paths;
use crate::ui;
use anyhow::Result;
use console::style;
use std::path::PathBuf;

pub fn run(cmd: ProjectCommand) -> Result<()> {
    let repo = paths::resolve_repo()?;
    match cmd {
        ProjectCommand::Add { alias, path } => {
            let mut machine = MachineConfig::load(&repo)?;
            let abs = expand(&path);
            machine.projects.insert(alias.clone(), abs.clone());
            machine.write(&repo)?;
            ui::ok(format!(
                "registered project {} → {}",
                alias,
                paths::display_path(&abs)
            ));
        }
        ProjectCommand::List => {
            let machine = MachineConfig::load(&repo)?;
            if machine.projects.is_empty() {
                ui::plain("(no projects registered)");
                return Ok(());
            }
            let width = machine.projects.keys().map(|s| s.len()).max().unwrap_or(0);
            for (alias, path) in &machine.projects {
                ui::plain(format!(
                    "{:<width$}  {}",
                    alias,
                    style(paths::display_path(path)).dim(),
                    width = width
                ));
            }
        }
        ProjectCommand::Remove { alias } => {
            let mut machine = MachineConfig::load(&repo)?;
            if machine.projects.remove(&alias).is_some() {
                machine.write(&repo)?;
                ui::ok(format!("removed project {}", alias));
            } else {
                ui::warn(format!("no project {} registered", alias));
            }
        }
    }
    Ok(())
}

fn expand(p: &PathBuf) -> PathBuf {
    if let Ok(rest) = p.strip_prefix("~") {
        if let Some(dirs) = directories::BaseDirs::new() {
            return dirs.home_dir().join(rest);
        }
    }
    if p.is_absolute() {
        p.clone()
    } else if let Ok(cwd) = std::env::current_dir() {
        cwd.join(p)
    } else {
        p.clone()
    }
}
