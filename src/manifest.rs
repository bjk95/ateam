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
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let body = if self.entries.is_empty() {
            "# agents manifest — managed by `agents apply`\n".to_string()
        } else {
            toml::to_string_pretty(self).context("serializing manifest")?
        };
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    pub fn contains_path(&self, p: &Path) -> bool {
        self.entries.iter().any(|e| e.path == p)
    }
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
