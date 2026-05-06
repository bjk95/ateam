//! Discover the upstream origin of a skill that has been imported as a local
//! snapshot. Two passes, both indexed once per `ateam skills import` run:
//!
//!  1. **Claude marketplace plugins** —
//!     `~/.claude/plugins/installed_plugins.json` lists every installed plugin
//!     keyed `<plugin>@<marketplace>` with its `installPath`. Skills under
//!     `<installPath>/skills/<name>` map to the marketplace's git source via
//!     `~/.claude/plugins/known_marketplaces.json`.
//!
//!  2. **Local git checkouts** — walk a small set of canonical dev parents
//!     (`~`, `~/dev`, `~/work`, `~/code`, `~/projects`) and, for each
//!     immediate child that has a `skills/`, `.agents/skills/`,
//!     `.claude/skills/`, or `.codex/skills/` subdir, read `remote.origin.url`
//!     and map every skill found there to the resulting source string.
//!
//! Skills that don't match either pass stay anonymous — `upstream = None`.
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One pass over the user's home directory; returns a map of skill-name →
/// source string (e.g., `github:owner/repo`). Cheap to call per import,
/// expensive to call per skill — call once, look up many.
pub fn build_index(home: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    index_claude_marketplaces(&mut map, home);
    index_local_checkouts(&mut map, home);
    map
}

// ---------------------------------------------------------------------------
// Claude marketplace plugins

#[derive(Deserialize)]
struct InstalledPlugins {
    #[serde(default)]
    plugins: HashMap<String, Vec<PluginInstall>>,
}

#[derive(Deserialize)]
struct PluginInstall {
    #[serde(rename = "installPath")]
    install_path: PathBuf,
}

#[derive(Deserialize)]
struct MarketplaceMeta {
    source: MarketplaceSource,
}

#[derive(Deserialize)]
struct MarketplaceSource {
    source: String,
    #[serde(default)]
    repo: String,
    #[serde(default)]
    url: String,
}

fn index_claude_marketplaces(map: &mut HashMap<String, String>, home: &Path) {
    let plugins_dir = home.join(".claude/plugins");
    let installed = match std::fs::read_to_string(plugins_dir.join("installed_plugins.json")) {
        Ok(s) => match serde_json::from_str::<InstalledPlugins>(&s) {
            Ok(p) => p,
            Err(_) => return,
        },
        Err(_) => return,
    };
    let marketplaces: HashMap<String, MarketplaceMeta> =
        match std::fs::read_to_string(plugins_dir.join("known_marketplaces.json")) {
            Ok(s) => match serde_json::from_str(&s) {
                Ok(m) => m,
                Err(_) => return,
            },
            Err(_) => return,
        };

    for (plugin_id, installs) in &installed.plugins {
        let marketplace_name = match plugin_id.split('@').nth(1) {
            Some(s) => s,
            None => continue,
        };
        let upstream = match marketplaces.get(marketplace_name) {
            Some(mp) => match mp.source.source.as_str() {
                "github" if !mp.source.repo.is_empty() => format!("github:{}", mp.source.repo),
                "git" if !mp.source.url.is_empty() => format!("git:{}", mp.source.url),
                _ => continue,
            },
            None => continue,
        };
        for install in installs {
            let skills_dir = install.install_path.join("skills");
            let entries = match std::fs::read_dir(&skills_dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                map.entry(name).or_insert_with(|| upstream.clone());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Local git checkouts

const SKILL_SUBDIRS: &[&str] = &[
    "skills",
    ".agents/skills",
    ".claude/skills",
    ".codex/skills",
];

fn index_local_checkouts(map: &mut HashMap<String, String>, home: &Path) {
    let roots: Vec<PathBuf> = [
        home.to_path_buf(),
        home.join("dev"),
        home.join("work"),
        home.join("code"),
        home.join("projects"),
        home.join("src"),
        home.join("repos"),
    ]
    .into_iter()
    .collect();

    for root in &roots {
        let entries = match std::fs::read_dir(root) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let candidate = entry.path();
            if !candidate.is_dir() {
                continue;
            }
            // Only resolve git remote once per candidate, lazily.
            let mut cached_upstream: Option<Option<String>> = None;
            for sub in SKILL_SUBDIRS {
                let skills_dir = candidate.join(sub);
                let skill_entries = match std::fs::read_dir(&skills_dir) {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                let upstream = cached_upstream
                    .get_or_insert_with(|| read_git_origin(&candidate).and_then(parse_git_url))
                    .clone();
                let upstream = match upstream {
                    Some(u) => u,
                    None => continue,
                };
                for skill in skill_entries.flatten() {
                    if !skill.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        continue;
                    }
                    let name = skill.file_name().to_string_lossy().into_owned();
                    map.entry(name).or_insert_with(|| upstream.clone());
                }
            }
        }
    }
}

fn read_git_origin(repo: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("config")
        .arg("--get")
        .arg("remote.origin.url")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if url.is_empty() {
        None
    } else {
        Some(url)
    }
}

/// Normalize a git URL to ateam's source-string convention.
fn parse_git_url(url: String) -> Option<String> {
    // SSH form: git@github.com:owner/repo[.git]
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let trimmed = rest.trim_end_matches('/').trim_end_matches(".git");
        if !trimmed.is_empty() {
            return Some(format!("github:{}", trimmed));
        }
    }
    // HTTPS form: https://github.com/owner/repo[.git]
    for prefix in ["https://github.com/", "http://github.com/"] {
        if let Some(rest) = url.strip_prefix(prefix) {
            let trimmed = rest.trim_end_matches('/').trim_end_matches(".git");
            if !trimmed.is_empty() {
                return Some(format!("github:{}", trimmed));
            }
        }
    }
    // Anything else: keep as raw git URL.
    Some(format!("git:{}", url))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn parses_ssh_github_url() {
        assert_eq!(
            parse_git_url("git@github.com:bjk95/gstack.git".into()),
            Some("github:bjk95/gstack".into())
        );
        assert_eq!(
            parse_git_url("git@github.com:bjk95/gstack".into()),
            Some("github:bjk95/gstack".into())
        );
    }

    #[test]
    fn parses_https_github_url() {
        assert_eq!(
            parse_git_url("https://github.com/bjk95/gstack.git".into()),
            Some("github:bjk95/gstack".into())
        );
        assert_eq!(
            parse_git_url("https://github.com/bjk95/gstack".into()),
            Some("github:bjk95/gstack".into())
        );
    }

    #[test]
    fn parses_other_git_url_as_git() {
        assert_eq!(
            parse_git_url("ssh://git@example.com/foo.git".into()),
            Some("git:ssh://git@example.com/foo.git".into())
        );
    }

    #[test]
    fn discovers_github_marketplace_plugin() {
        let home = TempDir::new().unwrap();
        let plugins = home.path().join(".claude/plugins");
        let install_path = plugins.join("cache/claude-plugins-official/superpowers/5.1.0");
        fs::create_dir_all(install_path.join("skills/brainstorming")).unwrap();

        write(
            &plugins.join("installed_plugins.json"),
            &format!(
                r#"{{"version":2,"plugins":{{"superpowers@claude-plugins-official":[{{"installPath":"{}"}}]}}}}"#,
                install_path.display()
            ),
        );
        write(
            &plugins.join("known_marketplaces.json"),
            r#"{"claude-plugins-official":{"source":{"source":"github","repo":"anthropics/claude-plugins-official"}}}"#,
        );

        let idx = build_index(home.path());
        assert_eq!(
            idx.get("brainstorming"),
            Some(&"github:anthropics/claude-plugins-official".to_string())
        );
    }

    #[test]
    fn discovers_local_checkout_via_git_remote() {
        let home = TempDir::new().unwrap();
        let repo = home.path().join("gstack");
        fs::create_dir_all(repo.join(".agents/skills/gstack-canary")).unwrap();
        fs::create_dir_all(repo.join(".agents/skills/gstack-ship")).unwrap();

        // Real git repo so `git config` works.
        let init = Command::new("git").arg("-C").arg(&repo).arg("init").output().unwrap();
        assert!(init.status.success(), "git init failed: {:?}", init);
        let add = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .arg("remote")
            .arg("add")
            .arg("origin")
            .arg("git@github.com:bjk95/gstack.git")
            .output()
            .unwrap();
        assert!(add.status.success(), "git remote add failed: {:?}", add);

        let idx = build_index(home.path());
        assert_eq!(idx.get("gstack-canary"), Some(&"github:bjk95/gstack".into()));
        assert_eq!(idx.get("gstack-ship"), Some(&"github:bjk95/gstack".into()));
    }

    #[test]
    fn empty_when_nothing_matches() {
        let home = TempDir::new().unwrap();
        let idx = build_index(home.path());
        assert!(idx.is_empty());
    }
}
