//! Discover the upstream origin of a skill imported as a local snapshot, by
//! looking inside Claude's own config dir. Currently only one signal:
//!
//!   `~/.claude/plugins/installed_plugins.json` lists every installed plugin
//!   keyed `<plugin>@<marketplace>` with its `installPath`. Skills under
//!   `<installPath>/skills/<name>` map to the marketplace's git source via
//!   `~/.claude/plugins/known_marketplaces.json`.
//!
//! Codex has no analogous manifest in `~/.codex/plugins/` (only a cache dir),
//! so we don't read anything there.
//!
//! Anything not installed via a Claude marketplace stays anonymous —
//! `upstream = None` and the list keeps showing just `local`.
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One pass over the user's home directory; returns a map of skill-name →
/// source string (e.g., `github:owner/repo`). Cheap to call per import,
/// expensive to call per skill — call once, look up many.
///
/// Iterates the agent registry and invokes each agent's `upstream_indexer`
/// if it has one. Today only `claude-code` populates this — the loop has
/// one productive iteration and the rest no-op.
pub fn build_index(home: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for def in crate::agents::all() {
        if let Some(indexer) = def.upstream_indexer {
            indexer(home, &mut map);
        }
    }
    map
}

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

pub fn index_claude_marketplaces(home: &Path, map: &mut HashMap<String, String>) {
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
    fn empty_when_no_claude_dir() {
        let home = TempDir::new().unwrap();
        let idx = build_index(home.path());
        assert!(idx.is_empty());
    }

    #[test]
    fn ignores_files_outside_claude_dir() {
        // Even if the user has a git checkout at ~/foo/.agents/skills/bar,
        // we don't pick it up — only inspect ~/.claude.
        let home = TempDir::new().unwrap();
        fs::create_dir_all(home.path().join("foo/.agents/skills/bar")).unwrap();
        let idx = build_index(home.path());
        assert!(idx.is_empty(), "must not look outside ~/.claude");
    }
}
