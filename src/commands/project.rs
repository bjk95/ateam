use crate::cli::ProjectCommand;
use crate::config::MachineConfig;
use crate::paths;
use anyhow::Result;
use std::path::PathBuf;

pub fn run(cmd: ProjectCommand) -> Result<()> {
    let repo = paths::resolve_repo()?;
    match cmd {
        ProjectCommand::Add { alias, path } => {
            let mut machine = MachineConfig::load(&repo)?;
            let abs = expand(&path);
            machine.projects.insert(alias.clone(), abs.clone());
            machine.write(&repo)?;
            println!("added project `{}` → {}", alias, abs.display());
        }
        ProjectCommand::List => {
            let machine = MachineConfig::load(&repo)?;
            if machine.projects.is_empty() {
                println!("(no projects registered)");
                return Ok(());
            }
            for (alias, path) in &machine.projects {
                println!("{:20}  {}", alias, path.display());
            }
        }
        ProjectCommand::Remove { alias } => {
            let mut machine = MachineConfig::load(&repo)?;
            if machine.projects.remove(&alias).is_some() {
                machine.write(&repo)?;
                println!("removed project alias `{}`", alias);
            } else {
                println!("no project alias `{}` registered", alias);
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
