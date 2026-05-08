use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lockfile {
    #[serde(default, rename = "skill")]
    pub skills: Vec<SkillEntry>,

    #[serde(default, rename = "subagent", skip_serializing_if = "Vec::is_empty")]
    pub subagents: Vec<SubagentEntry>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<InstructionsEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionsEntry {
    #[serde(
        default = "default_instructions_harnesses",
        skip_serializing_if = "is_default_harnesses"
    )]
    pub harnesses: Vec<String>,
}

fn default_instructions_harnesses() -> Vec<String> {
    vec!["*".into()]
}

impl Default for InstructionsEntry {
    fn default() -> Self {
        Self {
            harnesses: default_instructions_harnesses(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEntry {
    pub name: String,
    pub source: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_sha: Option<String>,

    #[serde(
        default = "default_harnesses",
        skip_serializing_if = "is_default_harnesses"
    )]
    pub harnesses: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,

    #[serde(default = "default_active", skip_serializing_if = "is_active")]
    pub active: bool,

    /// Origin repo for snapshotted (`local:`) entries — populated automatically
    /// by `agents import` when discoverable. None for non-local sources
    /// (where `source` already encodes the upstream) or when discovery failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
}

/// Single-file lockfile entry for a Claude/Codex subagent. Mirrors `SkillEntry`
/// but the snapshot is one `.md` file (`<repo>/agents/<name>.md`) rather than a
/// directory tree, so `tree_sha` is replaced by `file_sha` (sha256 of the file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentEntry {
    pub name: String,
    pub source: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_sha: Option<String>,

    #[serde(
        default = "default_harnesses",
        skip_serializing_if = "is_default_harnesses"
    )]
    pub harnesses: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,

    #[serde(default = "default_active", skip_serializing_if = "is_active")]
    pub active: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
}

fn default_harnesses() -> Vec<String> {
    vec!["*".into()]
}

fn is_default_harnesses(v: &[String]) -> bool {
    v.len() == 1 && v[0] == "*"
}

fn default_active() -> bool {
    true
}

fn is_active(b: &bool) -> bool {
    *b
}

impl Lockfile {
    pub fn load(repo: &Path) -> Result<Self> {
        let path = crate::paths::lockfile(repo);
        if !path.exists() {
            return Ok(Self {
                skills: Vec::new(),
                subagents: Vec::new(),
                instructions: None,
            });
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let lock: Lockfile =
            toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        validate_no_duplicate_names(&lock.skills)
            .with_context(|| format!("in {}", path.display()))?;
        validate_subpaths(&lock.skills).with_context(|| format!("in {}", path.display()))?;
        validate_no_duplicate_subagents(&lock.subagents)
            .with_context(|| format!("in {}", path.display()))?;
        validate_subagent_subpaths(&lock.subagents)
            .with_context(|| format!("in {}", path.display()))?;
        Ok(lock)
    }

    pub fn write(&self, repo: &Path) -> Result<()> {
        validate_no_duplicate_names(&self.skills)
            .context("refusing to write lockfile with duplicate skill names")?;
        validate_no_duplicate_subagents(&self.subagents)
            .context("refusing to write lockfile with duplicate subagent names")?;
        let path = crate::paths::lockfile(repo);
        let body =
            if self.skills.is_empty() && self.subagents.is_empty() && self.instructions.is_none() {
                "# agents lockfile — managed by `agents`\n".to_string()
            } else {
                toml::to_string_pretty(self).context("serializing lockfile")?
            };
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    /// Insert or replace by name. Returns whether an existing entry was replaced.
    pub fn upsert(&mut self, entry: SkillEntry) -> bool {
        if let Some(pos) = self.skills.iter().position(|s| s.name == entry.name) {
            self.skills[pos] = entry;
            true
        } else {
            self.skills.push(entry);
            false
        }
    }

    pub fn remove(&mut self, name: &str) -> Option<SkillEntry> {
        if let Some(pos) = self.skills.iter().position(|s| s.name == name) {
            Some(self.skills.remove(pos))
        } else {
            None
        }
    }

    pub fn find(&self, name: &str) -> Option<&SkillEntry> {
        self.skills.iter().find(|s| s.name == name)
    }

    pub fn upsert_subagent(&mut self, entry: SubagentEntry) -> bool {
        if let Some(pos) = self.subagents.iter().position(|s| s.name == entry.name) {
            self.subagents[pos] = entry;
            true
        } else {
            self.subagents.push(entry);
            false
        }
    }

    pub fn remove_subagent(&mut self, name: &str) -> Option<SubagentEntry> {
        self.subagents
            .iter()
            .position(|s| s.name == name)
            .map(|pos| self.subagents.remove(pos))
    }

    pub fn find_subagent(&self, name: &str) -> Option<&SubagentEntry> {
        self.subagents.iter().find(|s| s.name == name)
    }
}

fn validate_no_duplicate_names(skills: &[SkillEntry]) -> Result<()> {
    let mut seen: HashSet<&str> = HashSet::new();
    for s in skills {
        if !seen.insert(s.name.as_str()) {
            bail!("duplicate skill name `{}` in lockfile", s.name);
        }
    }
    Ok(())
}

/// Reject `path` fields that could escape the package root during extraction.
/// Skipped for `local:` sources, where `path` records the source location
/// itself (legitimately absolute or containing `..`) rather than a subpath
/// within an extracted tarball.
fn validate_subpaths(skills: &[SkillEntry]) -> Result<()> {
    for s in skills {
        let Some(p) = s.path.as_deref() else { continue };
        if s.source.starts_with("local:") {
            continue;
        }
        crate::source::sanitize_subpath(p)
            .with_context(|| format!("invalid `path` for skill `{}`", s.name))?;
    }
    Ok(())
}

fn validate_no_duplicate_subagents(subagents: &[SubagentEntry]) -> Result<()> {
    let mut seen: HashSet<&str> = HashSet::new();
    for s in subagents {
        if !seen.insert(s.name.as_str()) {
            bail!("duplicate subagent name `{}` in lockfile", s.name);
        }
    }
    Ok(())
}

fn validate_subagent_subpaths(subagents: &[SubagentEntry]) -> Result<()> {
    for s in subagents {
        let Some(p) = s.path.as_deref() else { continue };
        if s.source.starts_with("local:") {
            continue;
        }
        crate::source::sanitize_subpath(p)
            .with_context(|| format!("invalid `path` for subagent `{}`", s.name))?;
    }
    Ok(())
}

/// Vercel-style normalization: spaces and uppercase → kebab-case lowercase.
/// Other special characters are stripped.
pub fn normalize_skill_name(input: &str) -> Result<String> {
    let mut out = String::with_capacity(input.len());
    let mut prev_dash = false;
    for ch in input.chars() {
        let mapped = match ch {
            ' ' | '_' | '-' => Some('-'),
            c if c.is_ascii_alphanumeric() => Some(c.to_ascii_lowercase()),
            _ => None,
        };
        match mapped {
            Some('-') if !prev_dash && !out.is_empty() => {
                out.push('-');
                prev_dash = true;
            }
            Some('-') => {}
            Some(c) => {
                out.push(c);
                prev_dash = false;
            }
            None => {}
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        return Err(anyhow!("skill name `{}` normalized to empty string", input));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_basic() {
        assert_eq!(
            normalize_skill_name("Convex Best Practices").unwrap(),
            "convex-best-practices"
        );
        assert_eq!(
            normalize_skill_name("frontend-design").unwrap(),
            "frontend-design"
        );
        assert_eq!(normalize_skill_name("My_Skill").unwrap(), "my-skill");
    }

    #[test]
    fn duplicate_names_rejected() {
        let lock = Lockfile {
            skills: vec![
                SkillEntry {
                    name: "a".into(),
                    source: "local:skills/a".into(),
                    path: None,
                    git_ref: None,
                    tree_sha: None,
                    harnesses: vec!["*".into()],
                    profiles: vec![],
                    project: None,
                    active: true,
                    upstream: None,
                },
                SkillEntry {
                    name: "a".into(),
                    source: "local:skills/a".into(),
                    path: None,
                    git_ref: None,
                    tree_sha: None,
                    harnesses: vec!["*".into()],
                    profiles: vec![],
                    project: None,
                    active: true,
                    upstream: None,
                },
            ],
            subagents: Vec::new(),
            instructions: None,
        };
        assert!(validate_no_duplicate_names(&lock.skills).is_err());
    }

    #[test]
    fn instructions_table_round_trips() {
        let lock = Lockfile {
            skills: Vec::new(),
            subagents: Vec::new(),
            instructions: Some(InstructionsEntry::default()),
        };
        let s = toml::to_string_pretty(&lock).unwrap();
        assert!(s.contains("[instructions]"));
        let parsed: Lockfile = toml::from_str(&s).unwrap();
        assert!(parsed.instructions.is_some());
    }

    #[test]
    fn active_default_round_trip_omits_field() {
        let lock = Lockfile {
            skills: vec![SkillEntry {
                name: "a".into(),
                source: "local:skills/a".into(),
                path: None,
                git_ref: None,
                tree_sha: None,
                harnesses: vec!["*".into()],
                profiles: vec![],
                project: None,
                active: true,
                upstream: None,
            }],
            subagents: Vec::new(),
            instructions: None,
        };
        let serialized = toml::to_string(&lock).unwrap();
        assert!(
            !serialized.contains("active"),
            "active field leaked: {}",
            serialized
        );
        let parsed: Lockfile = toml::from_str(&serialized).unwrap();
        assert!(
            parsed.skills[0].active,
            "active should default to true on load"
        );
    }

    #[test]
    fn inactive_round_trips_explicitly() {
        let lock = Lockfile {
            skills: vec![SkillEntry {
                name: "a".into(),
                source: "local:skills/a".into(),
                path: None,
                git_ref: None,
                tree_sha: None,
                harnesses: vec!["*".into()],
                profiles: vec![],
                project: None,
                active: false,
                upstream: None,
            }],
            subagents: Vec::new(),
            instructions: None,
        };
        let serialized = toml::to_string(&lock).unwrap();
        assert!(
            serialized.contains("active = false"),
            "missing active=false: {}",
            serialized
        );
        let parsed: Lockfile = toml::from_str(&serialized).unwrap();
        assert!(!parsed.skills[0].active);
    }

    #[test]
    fn legacy_lockfile_loads_as_active() {
        let legacy = r#"[[skill]]
name = "a"
source = "local:skills/a"
"#;
        let parsed: Lockfile = toml::from_str(legacy).unwrap();
        assert!(
            parsed.skills[0].active,
            "missing field should default to active=true"
        );
    }

    #[test]
    fn rejects_parent_dir_subpath_for_remote_source() {
        let entries = vec![SkillEntry {
            name: "a".into(),
            source: "github:foo/bar".into(),
            path: Some("../../etc/passwd".into()),
            git_ref: None,
            tree_sha: None,
            harnesses: vec!["*".into()],
            profiles: vec![],
            project: None,
            active: true,
            upstream: None,
        }];
        let err = validate_subpaths(&entries).unwrap_err();
        let msg = format!("{:#}", err);
        assert!(msg.contains(".."), "error should mention `..`: {}", msg);
        assert!(msg.contains('a'), "error should name the skill: {}", msg);
    }

    #[test]
    fn subagent_round_trips_minimal() {
        let lock = Lockfile {
            skills: Vec::new(),
            subagents: vec![SubagentEntry {
                name: "code-reviewer".into(),
                source: "github:foo/bar".into(),
                path: Some("agents/code-reviewer.md".into()),
                git_ref: None,
                file_sha: None,
                harnesses: vec!["*".into()],
                profiles: vec![],
                project: None,
                active: true,
                upstream: None,
            }],
            instructions: None,
        };
        let serialized = toml::to_string_pretty(&lock).unwrap();
        assert!(
            serialized.contains("[[subagent]]"),
            "missing table header: {}",
            serialized
        );
        let parsed: Lockfile = toml::from_str(&serialized).unwrap();
        assert_eq!(parsed.subagents.len(), 1);
        assert_eq!(parsed.subagents[0].name, "code-reviewer");
        assert!(parsed.subagents[0].active);
    }

    #[test]
    fn duplicate_subagent_names_rejected() {
        let entries = vec![
            SubagentEntry {
                name: "a".into(),
                source: "local:agents/a.md".into(),
                path: None,
                git_ref: None,
                file_sha: None,
                harnesses: vec!["*".into()],
                profiles: vec![],
                project: None,
                active: true,
                upstream: None,
            },
            SubagentEntry {
                name: "a".into(),
                source: "local:agents/a.md".into(),
                path: None,
                git_ref: None,
                file_sha: None,
                harnesses: vec!["*".into()],
                profiles: vec![],
                project: None,
                active: true,
                upstream: None,
            },
        ];
        assert!(validate_no_duplicate_subagents(&entries).is_err());
    }

    #[test]
    fn empty_subagents_field_omitted_in_toml() {
        let lock = Lockfile {
            skills: Vec::new(),
            subagents: Vec::new(),
            instructions: None,
        };
        let serialized = toml::to_string(&lock).unwrap();
        assert!(
            !serialized.contains("subagent"),
            "empty subagents leaked into output: {}",
            serialized
        );
    }

    #[test]
    fn rejects_parent_dir_subpath_for_subagent_remote_source() {
        let entries = vec![SubagentEntry {
            name: "a".into(),
            source: "github:foo/bar".into(),
            path: Some("../../etc/passwd".into()),
            git_ref: None,
            file_sha: None,
            harnesses: vec!["*".into()],
            profiles: vec![],
            project: None,
            active: true,
            upstream: None,
        }];
        let err = validate_subagent_subpaths(&entries).unwrap_err();
        let msg = format!("{:#}", err);
        assert!(msg.contains(".."));
        assert!(msg.contains('a'));
    }

    #[test]
    fn allows_parent_dir_path_for_local_source() {
        let entries = vec![SkillEntry {
            name: "a".into(),
            source: "local:../external".into(),
            path: Some("../external/skills/a".into()),
            git_ref: None,
            tree_sha: None,
            harnesses: vec!["*".into()],
            profiles: vec![],
            project: None,
            active: true,
            upstream: None,
        }];
        validate_subpaths(&entries).expect("local sources may carry .. in path");
    }
}
