use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct DiscoveredSkill {
    /// Skill name from frontmatter, normalized (Vercel rules).
    pub name: String,
    /// Optional human description from frontmatter.
    pub description: Option<String>,
    /// Filesystem path to the skill directory (containing SKILL.md).
    pub dir: PathBuf,
    /// Path of SKILL.md relative to the package root (e.g. `skills/foo/SKILL.md`).
    pub rel_skill_md: PathBuf,
    /// Authoritative version hash carried by the source (e.g., the
    /// `skillsComputedHash` returned by skills.sh's blob endpoint). When set,
    /// `install_one` uses this verbatim as the lockfile's `tree_sha` instead
    /// of computing one from the GitHub tree or from local content.
    pub source_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Frontmatter {
    name: String,
    #[serde(default)]
    description: Option<String>,
}

/// Walk a package root, find every `SKILL.md` (anywhere in the tree),
/// parse its YAML frontmatter, and return one entry per skill.
pub fn walk_package(root: &Path) -> Result<Vec<DiscoveredSkill>> {
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<DiscoveredSkill>) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Skip dotfiles and conventional ignore dirs.
        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
            if name.starts_with('.') || matches!(name, "node_modules" | "target" | "dist" | "build") {
                continue;
            }
        }
        let ft = entry.file_type().with_context(|| format!("stat {}", path.display()))?;
        if ft.is_dir() {
            walk(root, &path, out)?;
        } else if ft.is_file() && path.file_name() == Some(std::ffi::OsStr::new("SKILL.md")) {
            if let Some(skill) = parse_skill_md(root, &path)? {
                out.push(skill);
            }
        }
    }
    Ok(())
}

fn parse_skill_md(root: &Path, file: &Path) -> Result<Option<DiscoveredSkill>> {
    let content = std::fs::read_to_string(file)
        .with_context(|| format!("reading {}", file.display()))?;
    let parsed = gray_matter::Matter::<gray_matter::engine::YAML>::new().parse(&content);
    let frontmatter: Frontmatter = match parsed.data {
        Some(data) => data
            .deserialize()
            .with_context(|| format!("parsing frontmatter in {}", file.display()))?,
        None => {
            // SKILL.md without frontmatter: ignore — Vercel/Claude skills require it.
            tracing::warn!("skipping {} (no YAML frontmatter)", file.display());
            return Ok(None);
        }
    };

    let name = crate::lockfile::normalize_skill_name(&frontmatter.name)?;
    let dir = file
        .parent()
        .ok_or_else(|| anyhow!("SKILL.md has no parent: {}", file.display()))?
        .to_path_buf();
    let rel_skill_md = file
        .strip_prefix(root)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| file.to_path_buf());

    Ok(Some(DiscoveredSkill {
        name,
        description: frontmatter.description,
        dir,
        rel_skill_md,
        source_hash: None,
    }))
}
