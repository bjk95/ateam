use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

const DEFAULT_REPO_DIR_NAME: &str = "ateam";
const POINTER_FILE_NAME: &str = "ateam.toml";

#[derive(Deserialize)]
struct Pointer {
    repo: PathBuf,
}

/// Resolve the ateam repo path: pointer file at `~/.config/ateam.toml` wins,
/// else the default `~/.config/ateam/` directory.
pub fn resolve_repo() -> Result<PathBuf> {
    let cfg_home = config_home()?;
    let pointer = cfg_home.join(POINTER_FILE_NAME);
    if pointer.exists() {
        let raw = std::fs::read_to_string(&pointer)
            .with_context(|| format!("reading pointer {}", pointer.display()))?;
        let parsed: Pointer = toml::from_str(&raw)
            .with_context(|| format!("parsing pointer {}", pointer.display()))?;
        return Ok(expand_tilde(&parsed.repo));
    }
    let default = cfg_home.join(DEFAULT_REPO_DIR_NAME);
    if default.join("ateam.toml").exists() {
        return Ok(default);
    }
    Err(anyhow!(
        "no ateam repo found.\n  expected pointer file at {} or default repo at {}.\n  run `ateam init` to bootstrap.",
        pointer.display(),
        default.display()
    ))
}

/// Default repo location, regardless of whether it exists yet.
pub fn default_repo() -> Result<PathBuf> {
    Ok(config_home()?.join(DEFAULT_REPO_DIR_NAME))
}

/// Pointer file path (used when user opted into a non-default repo location).
pub fn pointer_file() -> Result<PathBuf> {
    Ok(config_home()?.join(POINTER_FILE_NAME))
}

/// Write the pointer file. Caller should only do this when repo path != default.
pub fn write_pointer(repo: &Path) -> Result<()> {
    let path = pointer_file()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let body = format!("repo = {}\n", toml_string(repo));
    std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Remove the pointer file (when caller resets to default location).
pub fn remove_pointer() -> Result<()> {
    let path = pointer_file()?;
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

/// XDG-style `~/.config`. We don't use `directories::BaseDirs::config_dir()`
/// because on macOS that returns `~/Library/Application Support`, which
/// breaks parity with the plan and surprises users who expect `~/.config`.
/// Honors `$XDG_CONFIG_HOME` if set.
pub fn config_home() -> Result<PathBuf> {
    if let Some(custom) = std::env::var_os("XDG_CONFIG_HOME") {
        let p = PathBuf::from(custom);
        if !p.as_os_str().is_empty() {
            return Ok(p);
        }
    }
    Ok(home_dir()?.join(".config"))
}

fn expand_tilde(p: &Path) -> PathBuf {
    if let Ok(s) = p.strip_prefix("~") {
        if let Some(home) = directories::BaseDirs::new() {
            return home.home_dir().join(s);
        }
    }
    p.to_path_buf()
}

fn toml_string(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{}\"", s)
}

// ---------------------------------------------------------------------------
// Per-skill paths inside the repo

pub fn lockfile(repo: &Path) -> PathBuf {
    repo.join("ateam.lock.toml")
}

pub fn repo_config(repo: &Path) -> PathBuf {
    repo.join("ateam.toml")
}

pub fn machine_config(repo: &Path) -> PathBuf {
    repo.join(".ateam").join("machine.toml")
}

pub fn manifest_file(repo: &Path) -> PathBuf {
    repo.join(".ateam").join("manifest.toml")
}

pub fn cache_dir(repo: &Path) -> PathBuf {
    repo.join(".ateam").join("cache")
}

pub fn cache_tmp_dir(repo: &Path) -> PathBuf {
    cache_dir(repo).join(".tmp")
}

pub fn local_skills_dir(repo: &Path) -> PathBuf {
    repo.join("skills")
}

pub fn instructions_dir(repo: &Path) -> PathBuf {
    repo.join("instructions")
}

pub fn instructions_template(repo: &Path) -> PathBuf {
    crate::instructions::template_path(repo)
}

// ---------------------------------------------------------------------------
// Per-agent install targets

/// Per-agent skill install path under a given install root (`~` or a project root).
pub fn agent_skill_path(install_root: &Path, agent: &str, skill_name: &str) -> Result<PathBuf> {
    let def = crate::agents::lookup(agent)
        .ok_or_else(|| anyhow!("unknown agent `{}`", agent))?;
    let subdir = def
        .skills_subdir
        .ok_or_else(|| anyhow!("agent `{}` has no skills directory", agent))?;
    Ok(install_root.join(subdir).join(skill_name))
}

pub fn home_dir() -> Result<PathBuf> {
    let dirs = directories::BaseDirs::new()
        .ok_or_else(|| anyhow!("could not determine home dir"))?;
    Ok(dirs.home_dir().to_path_buf())
}

/// Render a path with `$HOME` collapsed to `~`. Falls back to the absolute
/// display when the path isn't under home or when home is unresolvable.
pub fn display_path(p: &Path) -> String {
    if let Some(dirs) = directories::BaseDirs::new() {
        let home = dirs.home_dir();
        if let Ok(rest) = p.strip_prefix(home) {
            if rest.as_os_str().is_empty() {
                return "~".to_string();
            }
            return format!("~/{}", rest.display());
        }
    }
    p.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_skill_path_matches_known_layout() {
        let root = PathBuf::from("/tmp/install-root");
        assert_eq!(
            agent_skill_path(&root, "claude-code", "foo").unwrap(),
            PathBuf::from("/tmp/install-root/.claude/skills/foo"),
        );
        assert_eq!(
            agent_skill_path(&root, "codex", "foo").unwrap(),
            PathBuf::from("/tmp/install-root/.codex/skills/foo"),
        );
        assert_eq!(
            agent_skill_path(&root, "opencode", "foo").unwrap(),
            PathBuf::from("/tmp/install-root/.config/opencode/skills/foo"),
        );
        assert_eq!(
            agent_skill_path(&root, "gemini", "foo").unwrap(),
            PathBuf::from("/tmp/install-root/.gemini/skills/foo"),
        );
    }

    #[test]
    fn agent_skill_path_rejects_unknown_agent() {
        let root = PathBuf::from("/tmp/install-root");
        let err = agent_skill_path(&root, "no-such-agent", "foo").unwrap_err();
        assert!(format!("{err}").contains("unknown agent"));
    }
}
