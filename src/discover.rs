use anyhow::{anyhow, Context, Result};
use serde_yaml::{Mapping, Value};
use std::path::{Path, PathBuf};

pub(crate) const MAX_NAME_CHARS: usize = 64;
const MAX_DESCRIPTION_CHARS: usize = 1024;
const MAX_COMPATIBILITY_CHARS: usize = 500;

#[derive(Debug, Clone)]
pub struct DiscoveredSkill {
    /// Skill name from frontmatter, normalized (Vercel rules).
    pub name: String,
    /// Human description from frontmatter.
    pub description: Option<String>,
    /// Filesystem path to the skill directory (containing SKILL.md).
    pub dir: PathBuf,
    /// Authoritative version hash carried by the source (e.g., the
    /// `skillsComputedHash` returned by skills.sh's blob endpoint). When set,
    /// `install_one` uses this verbatim as the lockfile's `tree_sha` instead
    /// of computing one from the GitHub tree or from local content.
    pub source_hash: Option<String>,
}

#[derive(Debug)]
struct ParsedSkillFile {
    name: String,
    description: String,
    content: String,
    frontmatter: Mapping,
    defaulted_name_from_dir: bool,
}

#[derive(Debug)]
pub(crate) struct SkillRepair {
    pub diagnostics: Vec<String>,
}

/// Walk a package root, find every `SKILL.md` (anywhere in the tree),
/// parse its YAML frontmatter, and return one entry per skill.
pub fn walk_package(root: &Path) -> Result<Vec<DiscoveredSkill>> {
    let mut out = Vec::new();
    walk(root, &mut out)?;
    Ok(out)
}

pub(crate) fn parse_skill_dir(dir: &Path) -> Result<Option<DiscoveredSkill>> {
    let skill_md = dir.join("SKILL.md");
    if !skill_md.is_file() {
        tracing::warn!("skipping {} (no SKILL.md)", dir.display());
        return Ok(None);
    }
    parse_skill_md(&skill_md)
}

pub(crate) fn canonicalize_skill_dir(
    dir: &Path,
    canonical_name: &str,
) -> Result<Option<SkillRepair>> {
    let skill_md = dir.join("SKILL.md");
    if !skill_md.is_file() {
        tracing::warn!("skipping {} (no SKILL.md)", dir.display());
        return Ok(None);
    }

    let raw = std::fs::read_to_string(&skill_md)
        .with_context(|| format!("reading {}", skill_md.display()))?;
    let Some(parsed) = parse_skill_file(&skill_md, &raw)? else {
        return Ok(None);
    };

    let mut diagnostics = Vec::new();
    let standard_name = standard_skill_name(canonical_name);
    if parsed.defaulted_name_from_dir {
        diagnostics.push(format!(
            "Agent Skills standard requires frontmatter `name`; added `{}` from the skill directory",
            standard_name
        ));
    }
    let oversized_name = if canonical_name != standard_name {
        Some(canonical_name)
    } else if parsed.name.chars().count() > MAX_NAME_CHARS {
        Some(parsed.name.as_str())
    } else {
        None
    };
    if let Some(name) = oversized_name {
        diagnostics.push(format!(
            "Agent Skills standard requires frontmatter `name` to be at most {} characters; rewrote `{}` to `{}`",
            MAX_NAME_CHARS, name, standard_name
        ));
    }
    if parsed.name != standard_name {
        diagnostics.push(format!(
            "Agent Skills standard requires frontmatter `name` to be lowercase kebab-case and match the skill directory; rewrote `{}` to `{}`",
            parsed.name, standard_name
        ));
    }
    if parsed.description != parsed.description.trim() {
        diagnostics.push(
            "Agent Skills standard requires a non-empty `description`; trimmed surrounding whitespace"
                .to_string(),
        );
    }

    let rendered = render_canonical_skill_md(&parsed, &standard_name, &mut diagnostics)?;
    if rendered != raw {
        std::fs::write(&skill_md, rendered)
            .with_context(|| format!("writing {}", skill_md.display()))?;
    }

    Ok(Some(SkillRepair { diagnostics }))
}

fn walk(dir: &Path, out: &mut Vec<DiscoveredSkill>) -> Result<()> {
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
            walk(&path, out)?;
        } else if ft.is_file() && path.file_name() == Some(std::ffi::OsStr::new("SKILL.md")) {
            if let Some(skill) = parse_skill_md(&path)? {
                out.push(skill);
            }
        }
    }
    Ok(())
}

fn parse_skill_md(file: &Path) -> Result<Option<DiscoveredSkill>> {
    let content =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let Some(parsed) = parse_skill_file(file, &content)? else {
        return Ok(None);
    };
    let name = standard_skill_name(&crate::lockfile::normalize_skill_name(&parsed.name)?);
    let dir = file
        .parent()
        .ok_or_else(|| anyhow!("SKILL.md has no parent: {}", file.display()))?
        .to_path_buf();

    Ok(Some(DiscoveredSkill {
        name,
        description: Some(parsed.description),
        dir,
        source_hash: None,
    }))
}

fn parse_skill_file(file: &Path, content: &str) -> Result<Option<ParsedSkillFile>> {
    let parsed = gray_matter::Matter::<gray_matter::engine::YAML>::new().parse(content);
    if parsed.matter.is_empty() || parsed.data.is_none() {
        tracing::warn!("skipping {} (no YAML frontmatter)", file.display());
        return Ok(None);
    }

    let frontmatter = match serde_yaml::from_str::<Value>(&parsed.matter) {
        Ok(Value::Mapping(mapping)) => mapping,
        Ok(_) => {
            tracing::warn!("skipping {} (frontmatter is not a mapping)", file.display());
            return Ok(None);
        }
        Err(e) => {
            tracing::warn!(
                "skipping {} (unparseable YAML frontmatter: {})",
                file.display(),
                e
            );
            return Ok(None);
        }
    };

    let (name, defaulted_name_from_dir) = match string_field(&frontmatter, "name").map(str::trim) {
        Some(name) if !name.is_empty() => (name.to_string(), false),
        _ => {
            let fallback = file
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .trim();
            if fallback.is_empty() {
                tracing::warn!("skipping {} (missing `name` frontmatter)", file.display());
                return Ok(None);
            }
            (fallback.to_string(), true)
        }
    };

    let Some(description) = string_field(&frontmatter, "description").map(str::to_string) else {
        tracing::warn!(
            "skipping {} (missing `description` frontmatter)",
            file.display()
        );
        return Ok(None);
    };
    if description.trim().is_empty() {
        tracing::warn!(
            "skipping {} (empty `description` frontmatter)",
            file.display()
        );
        return Ok(None);
    }
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        tracing::warn!(
            "skipping {} (`description` frontmatter exceeds {} characters)",
            file.display(),
            MAX_DESCRIPTION_CHARS
        );
        return Ok(None);
    }

    Ok(Some(ParsedSkillFile {
        name,
        description,
        content: parsed.content,
        frontmatter,
        defaulted_name_from_dir,
    }))
}

pub(crate) fn standard_skill_name(name: &str) -> String {
    let mut out = name.chars().take(MAX_NAME_CHARS).collect::<String>();
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn render_canonical_skill_md(
    parsed: &ParsedSkillFile,
    canonical_name: &str,
    diagnostics: &mut Vec<String>,
) -> Result<String> {
    let mut mapping = Mapping::new();
    mapping.insert(
        Value::String("name".into()),
        Value::String(canonical_name.to_string()),
    );
    mapping.insert(
        Value::String("description".into()),
        Value::String(parsed.description.trim().to_string()),
    );

    insert_optional_string(
        &parsed.frontmatter,
        &mut mapping,
        "license",
        None,
        diagnostics,
    );
    insert_optional_string(
        &parsed.frontmatter,
        &mut mapping,
        "compatibility",
        Some(MAX_COMPATIBILITY_CHARS),
        diagnostics,
    );
    insert_metadata(&parsed.frontmatter, &mut mapping, diagnostics);
    insert_optional_string(
        &parsed.frontmatter,
        &mut mapping,
        "allowed-tools",
        None,
        diagnostics,
    );

    let yaml = serde_yaml::to_string(&Value::Mapping(mapping)).context("serializing SKILL.md")?;
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&yaml);
    if !yaml.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("---\n");
    if !parsed.content.is_empty() {
        out.push('\n');
        out.push_str(&parsed.content);
    }
    Ok(out)
}

fn insert_optional_string(
    source: &Mapping,
    dest: &mut Mapping,
    field: &str,
    max_chars: Option<usize>,
    diagnostics: &mut Vec<String>,
) {
    let Some(value) = source.get(Value::String(field.into())) else {
        return;
    };
    let Some(raw) = value.as_str() else {
        diagnostics.push(format!(
            "Agent Skills standard requires `{}` to be a string when present; dropped invalid value",
            field
        ));
        return;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        diagnostics.push(format!(
            "Agent Skills standard requires `{}` to be non-empty when present; dropped empty value",
            field
        ));
        return;
    }
    if let Some(max) = max_chars {
        if trimmed.chars().count() > max {
            diagnostics.push(format!(
                "Agent Skills standard requires `{}` to be at most {} characters; dropped invalid value",
                field, max
            ));
            return;
        }
    }
    dest.insert(
        Value::String(field.into()),
        Value::String(trimmed.to_string()),
    );
}

fn insert_metadata(source: &Mapping, dest: &mut Mapping, diagnostics: &mut Vec<String>) {
    let Some(value) = source.get(Value::String("metadata".into())) else {
        return;
    };
    let Value::Mapping(source_metadata) = value else {
        diagnostics.push(
            "Agent Skills standard requires `metadata` to be a string key-value mapping; dropped invalid value"
                .to_string(),
        );
        return;
    };

    let mut metadata = Mapping::new();
    for (key, value) in source_metadata {
        let (Some(key), Some(value)) = (key.as_str(), value.as_str()) else {
            diagnostics.push(
                "Agent Skills standard requires `metadata` to contain only string keys and string values; dropped invalid value"
                    .to_string(),
            );
            return;
        };
        metadata.insert(
            Value::String(key.to_string()),
            Value::String(value.to_string()),
        );
    }
    dest.insert(Value::String("metadata".into()), Value::Mapping(metadata));
}

fn string_field<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a str> {
    mapping
        .get(Value::String(key.into()))
        .and_then(Value::as_str)
}

// ============================================================================
// Unmanaged-skill detection
// ============================================================================

/// A skill directory found in an agent's skills dir that agents isn't tracking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmanagedSkill {
    pub name: String,
    /// Each agent skills dir the skill was found in, in canonical
    /// `harness_skill_dirs` order.
    pub dirs: Vec<PathBuf>,
}

/// Conventional agent-skills directories agents scans, in canonical order.
/// Derived from the agent registry plus the cross-tool `.agents/skills/`
/// alias (honored by Gemini and OpenCode but not bound to any single agent).
pub fn harness_skill_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = crate::harness::all()
        .filter_map(|a| a.skills_subdir.map(|s| home.join(s)))
        .collect();
    dirs.push(home.join(".agents").join("skills"));
    dirs
}

/// Scan the agent skills dirs for skills not yet adopted by agents.
///
/// A directory is unmanaged when it isn't hidden, isn't a symlink whose
/// target lives inside the agents repo (`skills/`), and its name isn't
/// already in the lockfile. Cross-tool dedup: a skill present in multiple
/// agent dirs returns one entry with all dirs aggregated.
pub fn discover_unmanaged(
    repo: &Path,
    home: &Path,
    lock: &crate::lockfile::Lockfile,
) -> Vec<UnmanagedSkill> {
    use std::collections::BTreeMap;

    let local = crate::paths::local_skills_dir(repo);

    let mut acc: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();

    for dir in harness_skill_dirs(home) {
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
            if !path.join("SKILL.md").is_file() {
                continue;
            }
            if ft.is_symlink() {
                if let Ok(target) = std::fs::read_link(&path) {
                    if target.starts_with(&local) {
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
            subagents: vec![],
            mcps: vec![],
            instructions: None,
        }
    }

    fn drop_skill_dir(home: &Path, agent: &str, name: &str) -> PathBuf {
        let dir = home.join(format!(".{}", agent)).join("skills").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "---\nname: x\n---\n").unwrap();
        dir
    }

    fn parse_temp_skill(contents: &str) -> Option<DiscoveredSkill> {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("test-skill");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("SKILL.md");
        std::fs::write(&file, contents).unwrap();
        parse_skill_md(&file).unwrap()
    }

    #[test]
    fn parse_skill_md_rejects_invalid_description() {
        let description = "a".repeat(MAX_DESCRIPTION_CHARS + 1);
        let cases = [
            "---\nname: missing-description\n---\nbody\n".to_string(),
            "---\nname: blank-description\ndescription: \"   \"\n---\nbody\n".to_string(),
            format!("---\nname: long-description\ndescription: {description}\n---\nbody\n"),
        ];

        for contents in cases {
            assert!(parse_temp_skill(&contents).is_none());
        }
    }

    #[test]
    fn parse_skill_md_accepts_description() {
        let skill = parse_temp_skill(
            "---\nname: valid-description\ndescription: Use for validation.\n---\nbody\n",
        )
        .unwrap();
        assert_eq!(skill.description.as_deref(), Some("Use for validation."));
    }

    #[test]
    fn canonicalize_skill_dir_repairs_managed_snapshot() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("alpha");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: Alpha\ndescription: \" Alpha skill. \"\ncompatibility: \"\"\nmetadata:\n  author: tester\n---\nbody\n",
        )
        .unwrap();

        let repair = canonicalize_skill_dir(&dir, "alpha").unwrap().unwrap();
        assert_eq!(repair.diagnostics.len(), 3);

        let repaired = std::fs::read_to_string(dir.join("SKILL.md")).unwrap();
        assert!(repaired.contains("name: alpha"), "{repaired}");
        assert!(repaired.contains("description: Alpha skill."), "{repaired}");
        assert!(!repaired.contains("compatibility"), "{repaired}");
        assert!(repaired.contains("metadata:"), "{repaired}");
        assert!(repaired.contains("body"), "{repaired}");
    }

    #[test]
    fn canonicalize_skill_dir_repairs_missing_and_long_names() {
        let tmp = TempDir::new().unwrap();
        let long_name = "a".repeat(MAX_NAME_CHARS + 10);
        let dir = tmp.path().join(&long_name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\ndescription: Long skill.\n---\nbody\n",
        )
        .unwrap();

        let repair = canonicalize_skill_dir(&dir, &long_name).unwrap().unwrap();
        assert_eq!(repair.diagnostics.len(), 3);

        let repaired = std::fs::read_to_string(dir.join("SKILL.md")).unwrap();
        let standard_name = "a".repeat(MAX_NAME_CHARS);
        assert!(
            repaired.contains(&format!("name: {standard_name}")),
            "{repaired}"
        );
        assert!(!repaired.contains(&long_name), "{repaired}");
    }

    #[test]
    fn canonicalize_skill_dir_logs_long_frontmatter_name() {
        let tmp = TempDir::new().unwrap();
        let standard_name = "a".repeat(MAX_NAME_CHARS);
        let long_name = format!("{standard_name}bbbbbbbbbb");
        let dir = tmp.path().join(&standard_name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {long_name}\ndescription: Long skill.\n---\nbody\n"),
        )
        .unwrap();

        let repair = canonicalize_skill_dir(&dir, &standard_name)
            .unwrap()
            .unwrap();
        assert!(repair
            .diagnostics
            .iter()
            .any(|d| d.contains("at most 64 characters")));

        let repaired = std::fs::read_to_string(dir.join("SKILL.md")).unwrap();
        assert!(
            repaired.contains(&format!("name: {standard_name}")),
            "{repaired}"
        );
        assert!(!repaired.contains(&long_name), "{repaired}");
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
            harnesses: vec!["*".into()],
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
    fn unmanaged_skips_dirs_without_top_level_skill_md() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(home.join(".claude").join("skills").join("container")).unwrap();

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
