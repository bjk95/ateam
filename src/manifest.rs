use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    /// All paths agents wrote in the most recent successful apply.
    #[serde(default, rename = "entry")]
    pub entries: Vec<ManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub path: PathBuf,
    pub kind: EntryKind,
    pub skill: String,
    pub harness: String,
    pub target: PathBuf,
    pub applied_at: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Symlink,
    Copy,
}

impl Manifest {
    pub fn load(repo: &Path) -> Result<Self> {
        let path = crate::paths::manifest_file(repo);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn write(&self, repo: &Path) -> Result<()> {
        let path = crate::paths::manifest_file(repo);
        let body = self.to_toml()?;
        if std::fs::read_to_string(&path).ok().as_deref() == Some(body.as_str()) {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    fn to_toml(&self) -> Result<String> {
        if self.entries.is_empty() {
            Ok("# agents manifest — managed by `agents apply`\n".to_string())
        } else {
            toml::to_string_pretty(self).context("serializing manifest")
        }
    }

    pub fn tracked_entry(
        &self,
        path: PathBuf,
        kind: EntryKind,
        skill: String,
        harness: String,
        target: PathBuf,
    ) -> ManifestEntry {
        let applied_at = self
            .entries
            .iter()
            .find(|entry| {
                entry.path == path
                    && entry.kind == kind
                    && entry.skill == skill
                    && entry.harness == harness
                    && entry.target == target
            })
            .map(|entry| entry.applied_at)
            .unwrap_or_else(now_unix);
        ManifestEntry {
            path,
            kind,
            skill,
            harness,
            target,
            applied_at,
        }
    }
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
