use crate::cli::InitArgs;
use crate::config::{MachineConfig, RepoConfig};
use crate::paths;
use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn run(args: InitArgs) -> Result<()> {
    let target = resolve_target(&args)?;
    let mode = resolve_mode(&args)?;

    match mode {
        Mode::Scaffold => scaffold(&target)?,
        Mode::Clone(url) => clone(&url, &target)?,
    }

    if !args.profiles.is_empty() {
        write_machine_profiles(&target, &args.profiles)?;
    } else if !paths::machine_config(&target).exists() {
        // Create empty machine.toml so machine_config::load gets default cleanly.
        MachineConfig::default().write(&target)?;
    }

    write_or_clear_pointer(&target)?;

    println!("ateam initialized at {}", target.display());
    Ok(())
}

enum Mode {
    Scaffold,
    Clone(String),
}

fn resolve_mode(args: &InitArgs) -> Result<Mode> {
    match (args.scaffold, args.git_url.as_ref()) {
        (true, Some(_)) => bail!("pass either --scaffold or a git URL, not both"),
        (true, None) => Ok(Mode::Scaffold),
        (false, Some(url)) => Ok(Mode::Clone(url.clone())),
        (false, None) => prompt_mode(),
    }
}

fn prompt_mode() -> Result<Mode> {
    use dialoguer::{theme::ColorfulTheme, Select};
    let choice = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("ateam-config repo")
        .items(&["Clone an existing git URL", "Scaffold a fresh empty repo"])
        .default(0)
        .interact()?;
    match choice {
        0 => {
            let url: String = dialoguer::Input::with_theme(&ColorfulTheme::default())
                .with_prompt("git URL")
                .interact_text()?;
            Ok(Mode::Clone(url))
        }
        _ => Ok(Mode::Scaffold),
    }
}

fn resolve_target(args: &InitArgs) -> Result<PathBuf> {
    if let Some(repo) = &args.repo {
        Ok(expand_tilde(repo))
    } else {
        paths::default_repo()
    }
}

fn scaffold(target: &Path) -> Result<()> {
    if target.exists() {
        if paths::repo_config(target).exists() {
            // Idempotent: existing repo, leave its files alone.
            ensure_state_dirs(target)?;
            ensure_gitignore(target)?;
            return Ok(());
        }
        if !is_empty(target)? {
            bail!(
                "{} exists and is not an ateam repo. refusing to scaffold over non-empty dir.",
                target.display()
            );
        }
    } else {
        std::fs::create_dir_all(target)
            .with_context(|| format!("creating {}", target.display()))?;
    }

    RepoConfig::default().write(target)?;
    write_empty_lockfile(target)?;
    ensure_gitignore(target)?;
    ensure_state_dirs(target)?;
    git_init_if_needed(target)?;
    initial_commit_if_clean(target)?;

    Ok(())
}

fn clone(url: &str, target: &Path) -> Result<()> {
    if target.exists() && !is_empty(target)? {
        bail!(
            "refusing to clone into non-empty {}",
            target.display()
        );
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }

    let status = Command::new("git")
        .arg("clone")
        .arg(url)
        .arg(target)
        .status()
        .context("running `git clone`")?;
    if !status.success() {
        bail!("git clone failed");
    }

    ensure_state_dirs(target)?;
    Ok(())
}

fn write_empty_lockfile(target: &Path) -> Result<()> {
    let path = paths::lockfile(target);
    std::fs::write(&path, "# ateam lockfile — managed by `ateam`\n")
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn ensure_gitignore(target: &Path) -> Result<()> {
    let path = target.join(".gitignore");
    let needed = ".ateam/\n";
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    if current.lines().any(|l| l.trim() == ".ateam/" || l.trim() == ".ateam") {
        return Ok(());
    }
    let mut new = current;
    if !new.is_empty() && !new.ends_with('\n') {
        new.push('\n');
    }
    new.push_str(needed);
    std::fs::write(&path, new).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn ensure_state_dirs(target: &Path) -> Result<()> {
    std::fs::create_dir_all(paths::cache_dir(target))
        .with_context(|| format!("creating {}", paths::cache_dir(target).display()))?;
    std::fs::create_dir_all(paths::local_skills_dir(target))
        .with_context(|| format!("creating {}", paths::local_skills_dir(target).display()))?;
    Ok(())
}

fn git_init_if_needed(target: &Path) -> Result<()> {
    if target.join(".git").exists() {
        return Ok(());
    }
    let status = Command::new("git")
        .arg("init")
        .arg("--initial-branch=main")
        .arg(target)
        .status()
        .context("running `git init`")?;
    if !status.success() {
        bail!("git init failed");
    }
    Ok(())
}

fn initial_commit_if_clean(target: &Path) -> Result<()> {
    // Only commit if there are tracked-or-staged changes AND no commits exist yet.
    let head = Command::new("git")
        .arg("-C")
        .arg(target)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()?;
    if head.status.success() {
        // Repo already has commits — nothing to do.
        return Ok(());
    }

    let add = Command::new("git")
        .arg("-C")
        .arg(target)
        .args(["add", "ateam.toml", "ateam.lock.toml", ".gitignore"])
        .status()
        .context("git add during init")?;
    if !add.success() {
        bail!("git add failed during init");
    }

    let commit = Command::new("git")
        .arg("-C")
        .arg(target)
        .args(["commit", "-m", "init :: ateam scaffold"])
        .status()
        .context("git commit during init")?;
    if !commit.success() {
        // Could fail if user.email is unset etc. Don't hard-fail init.
        eprintln!("warning: initial commit failed (set up `git config` and commit manually if desired)");
    }
    Ok(())
}

fn write_machine_profiles(target: &Path, profiles: &[String]) -> Result<()> {
    let mut cfg = MachineConfig::load(target)?;
    cfg.profiles = profiles.to_vec();
    cfg.write(target)?;
    Ok(())
}

fn write_or_clear_pointer(target: &Path) -> Result<()> {
    let default = paths::default_repo()?;
    if same_path(target, &default) {
        paths::remove_pointer()?;
    } else {
        paths::write_pointer(target)?;
    }
    Ok(())
}

fn same_path(a: &Path, b: &Path) -> bool {
    let canon_a = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let canon_b = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    canon_a == canon_b
}

fn is_empty(path: &Path) -> Result<bool> {
    Ok(path
        .read_dir()
        .with_context(|| format!("reading {}", path.display()))?
        .next()
        .is_none())
}

fn expand_tilde(p: &Path) -> PathBuf {
    if let Ok(rest) = p.strip_prefix("~") {
        if let Some(dirs) = directories::BaseDirs::new() {
            return dirs.home_dir().join(rest);
        }
    }
    p.to_path_buf()
}

#[allow(dead_code)]
fn _unused() -> Result<()> {
    Err(anyhow!("placeholder"))
}
