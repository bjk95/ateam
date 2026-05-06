//! Discover the upstream origin of a skill that has been imported as a local
//! snapshot. Currently looks at Claude's plugin manifest:
//!
//!   `~/.claude/plugins/installed_plugins.json` lists every installed plugin
//!   keyed `<plugin>@<marketplace>` with its on-disk `installPath`. If a skill
//!   directory exists under `<installPath>/skills/<name>`, that plugin's
//!   marketplace is the origin. The marketplace's git source is then resolved
//!   via `~/.claude/plugins/known_marketplaces.json`.
//!
//! Returns `None` for anything that didn't come from a Claude marketplace
//! plugin — local-checkout symlinks, hand-extracted snapshots, etc.
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

/// Look up the upstream repo for a skill by name. `home` is the user's
/// home directory (so tests can point it at a fixture).
pub fn discover(skill_name: &str, home: &Path) -> Option<String> {
    let plugins_dir = home.join(".claude/plugins");
    let installed: InstalledPlugins =
        serde_json::from_str(&std::fs::read_to_string(plugins_dir.join("installed_plugins.json")).ok()?)
            .ok()?;
    let marketplaces: HashMap<String, MarketplaceMeta> =
        serde_json::from_str(&std::fs::read_to_string(plugins_dir.join("known_marketplaces.json")).ok()?)
            .ok()?;

    for (plugin_id, installs) in &installed.plugins {
        let marketplace_name = plugin_id.split('@').nth(1)?;
        for install in installs {
            let skill_dir = install.install_path.join("skills").join(skill_name);
            if skill_dir.exists() {
                let mp = marketplaces.get(marketplace_name)?;
                return match mp.source.source.as_str() {
                    "github" if !mp.source.repo.is_empty() => {
                        Some(format!("github:{}", mp.source.repo))
                    }
                    "git" if !mp.source.url.is_empty() => Some(format!("git:{}", mp.source.url)),
                    _ => None,
                };
            }
        }
    }
    None
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

        let got = discover("brainstorming", home.path());
        assert_eq!(got, Some("github:anthropics/claude-plugins-official".into()));
    }

    #[test]
    fn returns_none_when_skill_not_in_any_plugin() {
        let home = TempDir::new().unwrap();
        let plugins = home.path().join(".claude/plugins");
        write(
            &plugins.join("installed_plugins.json"),
            r#"{"version":2,"plugins":{}}"#,
        );
        write(
            &plugins.join("known_marketplaces.json"),
            r#"{}"#,
        );
        assert_eq!(discover("nope", home.path()), None);
    }

    #[test]
    fn returns_none_when_no_claude_dir() {
        let home = TempDir::new().unwrap();
        assert_eq!(discover("anything", home.path()), None);
    }
}
