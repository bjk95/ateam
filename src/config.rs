use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    #[serde(default)]
    pub declared_profiles: Vec<String>,
    #[serde(default = "default_harnesses")]
    pub enabled_harnesses: Vec<String>,
}

fn default_harnesses() -> Vec<String> {
    crate::harness::ids().map(String::from).collect()
}

impl Default for RepoConfig {
    fn default() -> Self {
        Self {
            declared_profiles: Vec::new(),
            enabled_harnesses: default_harnesses(),
        }
    }
}

impl RepoConfig {
    pub fn load(repo: &Path) -> Result<Self> {
        let path = crate::paths::repo_config(repo);
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn write(&self, repo: &Path) -> Result<()> {
        let path = crate::paths::repo_config(repo);
        let body = toml::to_string_pretty(self)
            .context("serializing repo config")?;
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MachineConfig {
    #[serde(default)]
    pub profiles: Vec<String>,
    #[serde(default)]
    pub projects: BTreeMap<String, PathBuf>,
    /// If set, `agents apply` skips writing instruction files on this machine.
    /// Recorded when the user picks "skip" at the first-run collision prompt.
    #[serde(default, skip_serializing_if = "is_false")]
    pub instructions_skip: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl MachineConfig {
    pub fn load(repo: &Path) -> Result<Self> {
        let path = crate::paths::machine_config(repo);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn write(&self, repo: &Path) -> Result<()> {
        let path = crate::paths::machine_config(repo);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let body = toml::to_string_pretty(self)
            .context("serializing machine config")?;
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_agents_includes_claude_and_codex() {
        let defaults = default_harnesses();
        assert!(defaults.contains(&"claude-code".to_string()));
        assert!(defaults.contains(&"codex".to_string()));
    }

    #[test]
    fn default_agents_count_matches_registry() {
        assert_eq!(default_harnesses().len(), crate::harness::REGISTRY.len());
    }

    #[test]
    fn default_agents_includes_all_four() {
        let defaults = default_harnesses();
        assert_eq!(defaults.len(), 4);
        assert!(defaults.contains(&"claude-code".to_string()));
        assert!(defaults.contains(&"codex".to_string()));
        assert!(defaults.contains(&"opencode".to_string()));
        assert!(defaults.contains(&"gemini".to_string()));
    }
}
