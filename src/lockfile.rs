use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lockfile {
    #[serde(default, rename = "skill")]
    pub skills: Vec<SkillEntry>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<InstructionsEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionsEntry {
    #[serde(default = "default_instructions_agents", skip_serializing_if = "is_default_agents")]
    pub agents: Vec<String>,
}

fn default_instructions_agents() -> Vec<String> {
    vec!["*".into()]
}

impl Default for InstructionsEntry {
    fn default() -> Self {
        Self {
            agents: default_instructions_agents(),
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

    #[serde(default = "default_agents", skip_serializing_if = "is_default_agents")]
    pub agents: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

fn default_agents() -> Vec<String> {
    vec!["*".into()]
}

fn is_default_agents(v: &[String]) -> bool {
    v.len() == 1 && v[0] == "*"
}

impl Lockfile {
    pub fn load(repo: &Path) -> Result<Self> {
        let path = crate::paths::lockfile(repo);
        if !path.exists() {
            return Ok(Self {
                skills: Vec::new(),
                instructions: None,
            });
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let lock: Lockfile = toml::from_str(&raw)
            .with_context(|| format!("parsing {}", path.display()))?;
        validate_no_duplicate_names(&lock.skills)
            .with_context(|| format!("in {}", path.display()))?;
        Ok(lock)
    }

    pub fn write(&self, repo: &Path) -> Result<()> {
        validate_no_duplicate_names(&self.skills)
            .context("refusing to write lockfile with duplicate skill names")?;
        let path = crate::paths::lockfile(repo);
        let body = if self.skills.is_empty() && self.instructions.is_none() {
            "# ateam lockfile — managed by `ateam`\n".to_string()
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
            Some('-') => {
                if !prev_dash && !out.is_empty() {
                    out.push('-');
                    prev_dash = true;
                }
            }
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
        assert_eq!(normalize_skill_name("Convex Best Practices").unwrap(), "convex-best-practices");
        assert_eq!(normalize_skill_name("frontend-design").unwrap(), "frontend-design");
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
                    agents: vec!["*".into()],
                    profiles: vec![],
                    project: None,
                },
                SkillEntry {
                    name: "a".into(),
                    source: "local:skills/a".into(),
                    path: None,
                    git_ref: None,
                    tree_sha: None,
                    agents: vec!["*".into()],
                    profiles: vec![],
                    project: None,
                },
            ],
            instructions: None,
        };
        assert!(validate_no_duplicate_names(&lock.skills).is_err());
    }

    #[test]
    fn instructions_table_round_trips() {
        let lock = Lockfile {
            skills: Vec::new(),
            instructions: Some(InstructionsEntry::default()),
        };
        let s = toml::to_string_pretty(&lock).unwrap();
        assert!(s.contains("[instructions]"));
        let parsed: Lockfile = toml::from_str(&s).unwrap();
        assert!(parsed.instructions.is_some());
    }
}
