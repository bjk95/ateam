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
            if name.starts_with('.') || matches!(name, "node_modules" | "target" | "dist" | "build")
            {
                continue;
            }
        }
        let ft = entry
            .file_type()
            .with_context(|| format!("stat {}", path.display()))?;
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
    let content =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
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

// ============================================================================
// Unmanaged-skill detection
// ============================================================================

/// A skill directory found in an agent's skills dir that ateam isn't tracking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmanagedSkill {
    pub name: String,
    /// Each agent skills dir the skill was found in, in canonical
    /// `agent_skill_dirs` order.
    pub dirs: Vec<PathBuf>,
}

/// Conventional agent-skills directories ateam scans, in canonical order.
pub fn agent_skill_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".claude").join("skills"),
        home.join(".codex").join("skills"),
        home.join(".agents").join("skills"),
    ]
}

/// Scan the agent skills dirs for skills not yet adopted by ateam.
///
/// A directory is unmanaged when it isn't hidden, isn't a symlink whose
/// target lives inside the ateam repo (`cache/` or `skills/`), and its
/// name isn't already in the lockfile. Cross-tool dedup: a skill present
/// in multiple agent dirs returns one entry with all dirs aggregated.
pub fn discover_unmanaged(
    repo: &Path,
    home: &Path,
    lock: &crate::lockfile::Lockfile,
) -> Vec<UnmanagedSkill> {
    use std::collections::BTreeMap;

    let cache = crate::paths::cache_dir(repo);
    let local = crate::paths::local_skills_dir(repo);

    let mut acc: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();

    for dir in agent_skill_dirs(home) {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            let ft = meta.file_type();
            if !ft.is_dir() && !ft.is_symlink() {
                continue;
            }
            if ft.is_symlink() {
                if let Ok(target) = std::fs::read_link(&path) {
                    if target.starts_with(&cache) || target.starts_with(&local) {
                        continue;
                    }
                }
            }
            if lock.find(&name).is_some() {
                continue;
            }
            acc.entry(name).or_default().push(dir.clone());
        }
    }

    acc.into_iter()
        .map(|(name, dirs)| UnmanagedSkill { name, dirs })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::{Lockfile, SkillEntry};
    use tempfile::TempDir;

    fn empty_lock() -> Lockfile {
        Lockfile {
            skills: vec![],
            instructions: None,
        }
    }

    fn drop_skill_dir(home: &Path, agent: &str, name: &str) -> PathBuf {
        let dir = home.join(format!(".{}", agent)).join("skills").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "---\nname: x\n---\n").unwrap();
        dir
    }

    #[test]
    fn unmanaged_skill_detected() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&repo).unwrap();
        drop_skill_dir(&home, "claude", "foo");

        let result = discover_unmanaged(&repo, &home, &empty_lock());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "foo");
        assert_eq!(result[0].dirs, vec![home.join(".claude").join("skills")]);
    }

    #[test]
    fn symlink_into_repo_is_managed() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        let home = tmp.path().join("home");
        let local = crate::paths::local_skills_dir(&repo).join("foo");
        std::fs::create_dir_all(&local).unwrap();
        let claude = home.join(".claude").join("skills");
        std::fs::create_dir_all(&claude).unwrap();
        std::os::unix::fs::symlink(&local, claude.join("foo")).unwrap();

        let result = discover_unmanaged(&repo, &home, &empty_lock());
        assert!(result.is_empty(), "got: {:?}", result);
    }

    #[test]
    fn lockfile_match_is_managed() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&repo).unwrap();
        drop_skill_dir(&home, "claude", "foo");

        let mut lock = empty_lock();
        lock.skills.push(SkillEntry {
            name: "foo".into(),
            source: "local:skills/foo".into(),
            path: Some("skills/foo".into()),
            git_ref: None,
            tree_sha: None,
            agents: vec!["*".into()],
            profiles: vec![],
            project: None,
            active: true,
            upstream: None,
        });

        let result = discover_unmanaged(&repo, &home, &lock);
        assert!(result.is_empty());
    }

    #[test]
    fn cross_tool_dedup() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&repo).unwrap();
        drop_skill_dir(&home, "claude", "foo");
        drop_skill_dir(&home, "codex", "foo");

        let result = discover_unmanaged(&repo, &home, &empty_lock());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "foo");
        assert_eq!(result[0].dirs.len(), 2);
        assert_eq!(
            result[0].dirs,
            vec![
                home.join(".claude").join("skills"),
                home.join(".codex").join("skills"),
            ]
        );
    }

    #[test]
    fn hidden_dir_skipped() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&repo).unwrap();
        drop_skill_dir(&home, "claude", ".hidden");

        let result = discover_unmanaged(&repo, &home, &empty_lock());
        assert!(result.is_empty());
    }

    #[test]
    fn missing_agent_dir_silent() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&repo).unwrap();

        let result = discover_unmanaged(&repo, &home, &empty_lock());
        assert!(result.is_empty());
    }

    #[test]
    fn sorted_by_name() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&repo).unwrap();
        drop_skill_dir(&home, "claude", "c");
        drop_skill_dir(&home, "claude", "a");
        drop_skill_dir(&home, "claude", "b");

        let result = discover_unmanaged(&repo, &home, &empty_lock());
        let names: Vec<&str> = result.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }
}
