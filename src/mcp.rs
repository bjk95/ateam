use crate::config::{MachineConfig, RepoConfig};
use crate::lockfile::{Lockfile, McpEntry};
use crate::paths;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use toml::Value as TomlValue;

#[derive(Debug, Default, Clone, Copy)]
pub struct ApplyOutcome {
    pub materialized: usize,
    pub changed: bool,
}

pub fn apply(
    repo: &Path,
    home: &Path,
    repo_cfg: &RepoConfig,
    entries: &[McpEntry],
    machine: &MachineConfig,
    target_harnesses: Option<&BTreeSet<String>>,
    dry_run: bool,
    honor_profiles: bool,
) -> Result<ApplyOutcome> {
    let mut outcome = ApplyOutcome::default();
    let previous_state = McpState::load(repo)?;
    let mut next_state = previous_state.outside_targets(target_harnesses);
    let mut state_touched = false;
    for harness in supported_target_harnesses(repo_cfg, target_harnesses, &previous_state) {
        let managed_names = managed_names(repo_cfg, entries, &previous_state, &harness);
        let managed: Vec<&McpEntry> = entries
            .iter()
            .filter(|entry| entry_targets_harness(entry, repo_cfg, &harness))
            .collect();
        if managed_names.is_empty() {
            continue;
        }
        state_touched = true;
        let desired: Vec<&McpEntry> = managed
            .iter()
            .copied()
            .filter(|entry| entry.active)
            .filter(|entry| !honor_profiles || profile_match(machine, &entry.profiles))
            .collect();
        outcome.materialized += desired.len();
        if dry_run {
            if let Some(path) = paths::harness_mcp_config_path(home, &harness)? {
                for entry in desired {
                    crate::ui::detail(format!("{}: {}", paths::display_path(&path), entry.name));
                }
            }
            continue;
        }
        let changed = match harness.as_str() {
            "codex" => reconcile_codex(home, &managed_names, &desired)?,
            "claude-code" => reconcile_claude(home, &managed_names, &desired)?,
            _ => false,
        };
        outcome.changed |= changed;
        for entry in desired {
            next_state.entries.push(McpStateEntry {
                name: entry.name.clone(),
                harness: harness.clone(),
            });
        }
    }
    next_state.sort();
    if !dry_run && state_touched {
        outcome.changed |= next_state.write(repo)?;
    }
    Ok(outcome)
}

pub fn validate_harnesses(ids: &[String]) -> Result<()> {
    let valid: Vec<&'static str> = crate::harness::ids().collect();
    for id in ids {
        if id == "*" {
            continue;
        }
        let Some(def) = crate::harness::lookup(id) else {
            bail!("unknown harness `{}`\n  valid: {}", id, valid.join(", "));
        };
        if def.mcp_config_file.is_none() {
            bail!("harness `{}` does not support managed MCP config yet", id);
        }
    }
    Ok(())
}

#[derive(Debug, Default)]
pub struct ImportOutcome {
    pub imported: usize,
    pub skipped_managed: usize,
    pub errors: Vec<(String, String)>,
}

impl ImportOutcome {
    pub fn changed(&self) -> bool {
        self.imported > 0
    }
}

pub fn import_from_existing(
    home: &Path,
    repo_cfg: &RepoConfig,
    lock: &mut Lockfile,
) -> Result<ImportOutcome> {
    let mut outcome = ImportOutcome::default();
    for def in crate::harness::all().filter(|def| def.mcp_config_file.is_some()) {
        if !repo_cfg.enabled_harnesses.iter().any(|h| h == def.id) {
            continue;
        }
        let entries = match read_harness_entries(home, def.id) {
            Ok(entries) => entries,
            Err(e) => {
                outcome.errors.push((def.id.to_string(), format!("{e:#}")));
                continue;
            }
        };
        for entry in entries {
            import_entry(lock, &mut outcome, entry);
        }
    }
    Ok(outcome)
}

fn read_harness_entries(home: &Path, harness: &str) -> Result<Vec<McpEntry>> {
    match harness {
        "codex" => read_codex_entries(home, harness),
        "claude-code" => read_claude_entries(home, harness),
        _ => Ok(Vec::new()),
    }
}

fn read_codex_entries(home: &Path, harness: &str) -> Result<Vec<McpEntry>> {
    let path = paths::harness_mcp_config_path(home, harness)?
        .ok_or_else(|| anyhow!("{} has no MCP config path", harness))?;
    let Some(raw) = read_optional(&path)? else {
        return Ok(Vec::new());
    };
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let root = raw
        .parse::<toml::Table>()
        .with_context(|| format!("parsing {}", path.display()))?;
    let Some(servers) = root.get("mcp_servers").and_then(|value| value.as_table()) else {
        return Ok(Vec::new());
    };
    servers
        .iter()
        .map(|(name, value)| {
            let table = value.as_table().ok_or_else(|| {
                anyhow!("mcp_servers.{} in {} is not a table", name, path.display())
            })?;
            mcp_from_toml(name, table, harness)
        })
        .collect()
}

fn mcp_from_toml(name: &str, table: &toml::Table, harness: &str) -> Result<McpEntry> {
    if let Some(url) = table.get("url").and_then(|value| value.as_str()) {
        return Ok(McpEntry {
            name: name.to_string(),
            transport: "http".into(),
            command: None,
            args: Vec::new(),
            env: BTreeMap::new(),
            url: Some(url.to_string()),
            bearer_token_env_var: table
                .get("bearer_token_env_var")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            harnesses: vec![harness.to_string()],
            profiles: Vec::new(),
            active: true,
        });
    }

    let command = table
        .get("command")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("MCP `{}` is missing command or url", name))?;
    Ok(McpEntry {
        name: name.to_string(),
        transport: "stdio".into(),
        command: Some(command.to_string()),
        args: toml_string_array(table.get("args"), name, "args")?,
        env: toml_string_map(table.get("env"), name, "env")?,
        url: None,
        bearer_token_env_var: None,
        harnesses: vec![harness.to_string()],
        profiles: Vec::new(),
        active: true,
    })
}

fn read_claude_entries(home: &Path, harness: &str) -> Result<Vec<McpEntry>> {
    let path = paths::harness_mcp_config_path(home, harness)?
        .ok_or_else(|| anyhow!("{} has no MCP config path", harness))?;
    let Some(raw) = read_optional(&path)? else {
        return Ok(Vec::new());
    };
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let root = serde_json::from_str::<JsonValue>(&raw)
        .with_context(|| format!("parsing {}", path.display()))?;
    let Some(servers) = root.get("mcpServers").and_then(|value| value.as_object()) else {
        return Ok(Vec::new());
    };
    servers
        .iter()
        .map(|(name, value)| {
            let object = value.as_object().ok_or_else(|| {
                anyhow!("mcpServers.{} in {} is not an object", name, path.display())
            })?;
            mcp_from_json(name, object, harness)
        })
        .collect()
}

fn mcp_from_json(
    name: &str,
    object: &serde_json::Map<String, JsonValue>,
    harness: &str,
) -> Result<McpEntry> {
    if let Some(url) = object.get("url").and_then(|value| value.as_str()) {
        return Ok(McpEntry {
            name: name.to_string(),
            transport: "http".into(),
            command: None,
            args: Vec::new(),
            env: BTreeMap::new(),
            url: Some(url.to_string()),
            bearer_token_env_var: None,
            harnesses: vec![harness.to_string()],
            profiles: Vec::new(),
            active: true,
        });
    }

    let command = object
        .get("command")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("MCP `{}` is missing command or url", name))?;
    Ok(McpEntry {
        name: name.to_string(),
        transport: "stdio".into(),
        command: Some(command.to_string()),
        args: json_string_array(object.get("args"), name, "args")?,
        env: json_string_map(object.get("env"), name, "env")?,
        url: None,
        bearer_token_env_var: None,
        harnesses: vec![harness.to_string()],
        profiles: Vec::new(),
        active: true,
    })
}

fn import_entry(lock: &mut Lockfile, outcome: &mut ImportOutcome, entry: McpEntry) {
    let Some(existing) = lock
        .mcps
        .iter_mut()
        .find(|locked| locked.name == entry.name)
    else {
        lock.mcps.push(entry);
        outcome.imported += 1;
        return;
    };
    if !same_server_config(existing, &entry) {
        outcome.errors.push((
            entry.name,
            "already exists in lockfile with different config".into(),
        ));
        return;
    }
    let before = existing.harnesses.clone();
    merge_harness(&mut existing.harnesses, &entry.harnesses[0]);
    if existing.harnesses == before {
        outcome.skipped_managed += 1;
    } else {
        outcome.imported += 1;
    }
}

fn same_server_config(a: &McpEntry, b: &McpEntry) -> bool {
    a.transport == b.transport
        && a.command == b.command
        && a.args == b.args
        && a.env == b.env
        && a.url == b.url
        && a.bearer_token_env_var == b.bearer_token_env_var
}

fn merge_harness(harnesses: &mut Vec<String>, harness: &str) {
    if harnesses.iter().any(|item| item == "*" || item == harness) {
        return;
    }
    harnesses.push(harness.to_string());
}

fn toml_string_array(value: Option<&TomlValue>, name: &str, field: &str) -> Result<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let array = value
        .as_array()
        .ok_or_else(|| anyhow!("MCP `{}` field `{}` is not an array", name, field))?;
    array
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or_else(|| anyhow!("MCP `{}` field `{}` contains a non-string", name, field))
        })
        .collect()
}

fn toml_string_map(
    value: Option<&TomlValue>,
    name: &str,
    field: &str,
) -> Result<BTreeMap<String, String>> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let table = value
        .as_table()
        .ok_or_else(|| anyhow!("MCP `{}` field `{}` is not a table", name, field))?;
    table
        .iter()
        .map(|(key, value)| {
            value
                .as_str()
                .map(|string| (key.clone(), string.to_string()))
                .ok_or_else(|| anyhow!("MCP `{}` field `{}` contains a non-string", name, field))
        })
        .collect()
}

fn json_string_array(value: Option<&JsonValue>, name: &str, field: &str) -> Result<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let array = value
        .as_array()
        .ok_or_else(|| anyhow!("MCP `{}` field `{}` is not an array", name, field))?;
    array
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or_else(|| anyhow!("MCP `{}` field `{}` contains a non-string", name, field))
        })
        .collect()
}

fn json_string_map(
    value: Option<&JsonValue>,
    name: &str,
    field: &str,
) -> Result<BTreeMap<String, String>> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("MCP `{}` field `{}` is not an object", name, field))?;
    object
        .iter()
        .map(|(key, value)| {
            value
                .as_str()
                .map(|string| (key.clone(), string.to_string()))
                .ok_or_else(|| anyhow!("MCP `{}` field `{}` contains a non-string", name, field))
        })
        .collect()
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(raw) => Ok(Some(raw)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow!("reading {}: {}", path.display(), e)),
    }
}

fn supported_target_harnesses(
    repo_cfg: &RepoConfig,
    target_harnesses: Option<&BTreeSet<String>>,
    state: &McpState,
) -> Vec<String> {
    crate::harness::all()
        .filter(|def| def.mcp_config_file.is_some())
        .filter(|def| target_matches(target_harnesses, def.id))
        .filter(|def| {
            repo_cfg.enabled_harnesses.iter().any(|h| h == def.id)
                || state.entries.iter().any(|entry| entry.harness == def.id)
        })
        .map(|def| def.id.to_string())
        .collect()
}

fn entry_targets_harness(entry: &McpEntry, repo_cfg: &RepoConfig, harness: &str) -> bool {
    if !repo_cfg.enabled_harnesses.iter().any(|h| h == harness) {
        return false;
    }
    if entry.harnesses.iter().any(|h| h == "*") {
        return true;
    }
    entry.harnesses.iter().any(|h| h == harness)
}

fn target_matches(target_harnesses: Option<&BTreeSet<String>>, harness: &str) -> bool {
    target_harnesses
        .map(|filter| filter.contains("*") || filter.contains(harness))
        .unwrap_or(true)
}

fn managed_names(
    repo_cfg: &RepoConfig,
    entries: &[McpEntry],
    state: &McpState,
    harness: &str,
) -> BTreeSet<String> {
    entries
        .iter()
        .filter(|entry| entry_targets_harness(entry, repo_cfg, harness))
        .map(|entry| entry.name.clone())
        .chain(
            state
                .entries
                .iter()
                .filter(|entry| entry.harness == harness)
                .map(|entry| entry.name.clone()),
        )
        .collect()
}

fn profile_match(machine: &MachineConfig, gates: &[String]) -> bool {
    if gates.is_empty() {
        return true;
    }
    gates
        .iter()
        .any(|gate| machine.profiles.iter().any(|profile| profile == gate))
}

fn reconcile_codex(
    home: &Path,
    managed_names: &BTreeSet<String>,
    desired: &[&McpEntry],
) -> Result<bool> {
    let path = paths::harness_mcp_config_path(home, "codex")?
        .ok_or_else(|| anyhow!("codex has no MCP config path"))?;
    let before = std::fs::read_to_string(&path).unwrap_or_default();
    let mut root = if before.trim().is_empty() {
        toml::map::Map::new()
    } else {
        before
            .parse::<toml::Table>()
            .with_context(|| format!("parsing {}", path.display()))?
    };

    let servers_empty = {
        let servers = ensure_toml_table(&mut root, "mcp_servers")?;
        for name in managed_names {
            servers.remove(name);
        }
        for entry in desired {
            servers.insert(entry.name.clone(), codex_server_value(entry)?);
        }
        servers.is_empty()
    };
    if servers_empty {
        root.remove("mcp_servers");
    }

    let after = if root.is_empty() {
        String::new()
    } else {
        toml::to_string_pretty(&TomlValue::Table(root)).context("serializing Codex MCP config")?
    };
    write_if_changed(&path, &before, &after)
}

fn ensure_toml_table<'a>(root: &'a mut toml::Table, key: &str) -> Result<&'a mut toml::Table> {
    let value = root
        .entry(key.to_string())
        .or_insert_with(|| TomlValue::Table(toml::map::Map::new()));
    if !value.is_table() {
        *value = TomlValue::Table(toml::map::Map::new());
    }
    value
        .as_table_mut()
        .ok_or_else(|| anyhow!("{} was not a table", key))
}

fn codex_server_value(entry: &McpEntry) -> Result<TomlValue> {
    let mut table = toml::map::Map::new();
    match entry.transport.as_str() {
        "stdio" => {
            let command = entry
                .command
                .as_ref()
                .ok_or_else(|| anyhow!("stdio MCP `{}` is missing command", entry.name))?;
            table.insert("command".into(), TomlValue::String(command.clone()));
            if !entry.args.is_empty() {
                table.insert(
                    "args".into(),
                    TomlValue::Array(
                        entry
                            .args
                            .iter()
                            .map(|arg| TomlValue::String(arg.clone()))
                            .collect(),
                    ),
                );
            }
            if !entry.env.is_empty() {
                table.insert("env".into(), string_table(&entry.env));
            }
        }
        "http" => {
            let url = entry
                .url
                .as_ref()
                .ok_or_else(|| anyhow!("http MCP `{}` is missing url", entry.name))?;
            table.insert("url".into(), TomlValue::String(url.clone()));
            if let Some(env_var) = &entry.bearer_token_env_var {
                table.insert(
                    "bearer_token_env_var".into(),
                    TomlValue::String(env_var.clone()),
                );
            }
        }
        other => bail!("unsupported MCP transport `{}` for `{}`", other, entry.name),
    }
    Ok(TomlValue::Table(table))
}

fn string_table(values: &BTreeMap<String, String>) -> TomlValue {
    let mut table = toml::map::Map::new();
    for (key, value) in values {
        table.insert(key.clone(), TomlValue::String(value.clone()));
    }
    TomlValue::Table(table)
}

fn reconcile_claude(
    home: &Path,
    managed_names: &BTreeSet<String>,
    desired: &[&McpEntry],
) -> Result<bool> {
    let path = paths::harness_mcp_config_path(home, "claude-code")?
        .ok_or_else(|| anyhow!("claude-code has no MCP config path"))?;
    let before = std::fs::read_to_string(&path).unwrap_or_default();
    let mut root = if before.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str::<JsonValue>(&before)
            .with_context(|| format!("parsing {}", path.display()))?
    };
    if !root.is_object() {
        root = serde_json::json!({});
    }
    let object = root.as_object_mut().expect("root object");
    let servers_value = object
        .entry("mcpServers".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !servers_value.is_object() {
        *servers_value = serde_json::json!({});
    }
    let servers_empty = {
        let servers = servers_value.as_object_mut().expect("servers object");
        for name in managed_names {
            servers.remove(name);
        }
        for entry in desired {
            servers.insert(entry.name.clone(), claude_server_value(entry)?);
        }
        servers.is_empty()
    };
    if servers_empty {
        object.remove("mcpServers");
    }

    let after = if root.as_object().is_some_and(|object| object.is_empty()) {
        String::new()
    } else {
        serde_json::to_string_pretty(&root).context("serializing Claude MCP config")? + "\n"
    };
    write_if_changed(&path, &before, &after)
}

fn claude_server_value(entry: &McpEntry) -> Result<JsonValue> {
    match entry.transport.as_str() {
        "stdio" => {
            let command = entry
                .command
                .as_ref()
                .ok_or_else(|| anyhow!("stdio MCP `{}` is missing command", entry.name))?;
            let mut value = serde_json::json!({
                "type": "stdio",
                "command": command,
                "args": entry.args,
                "env": entry.env,
            });
            if entry.args.is_empty() {
                value.as_object_mut().expect("server object").remove("args");
            }
            Ok(value)
        }
        "http" => {
            let url = entry
                .url
                .as_ref()
                .ok_or_else(|| anyhow!("http MCP `{}` is missing url", entry.name))?;
            Ok(serde_json::json!({
                "type": "http",
                "url": url,
            }))
        }
        other => bail!("unsupported MCP transport `{}` for `{}`", other, entry.name),
    }
}

fn write_if_changed(path: &Path, before: &str, after: &str) -> Result<bool> {
    if before == after {
        return Ok(false);
    }
    crate::install::write_atomically(path, after)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct McpState {
    #[serde(default, rename = "entry")]
    entries: Vec<McpStateEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct McpStateEntry {
    name: String,
    harness: String,
}

impl McpState {
    fn load(repo: &Path) -> Result<Self> {
        let path = state_path(repo);
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

    fn outside_targets(&self, target_harnesses: Option<&BTreeSet<String>>) -> Self {
        Self {
            entries: self
                .entries
                .iter()
                .filter(|entry| !target_matches(target_harnesses, &entry.harness))
                .cloned()
                .collect(),
        }
    }

    fn sort(&mut self) {
        self.entries
            .sort_by(|a, b| a.harness.cmp(&b.harness).then(a.name.cmp(&b.name)));
        self.entries.dedup();
    }

    fn write(&self, repo: &Path) -> Result<bool> {
        let path = state_path(repo);
        let body = if self.entries.is_empty() {
            "# agents MCP manifest - managed by `agents apply`\n".to_string()
        } else {
            toml::to_string_pretty(self).context("serializing MCP manifest")?
        };
        if std::fs::read_to_string(&path).ok().as_deref() == Some(body.as_str()) {
            return Ok(false);
        }
        crate::install::write_atomically(&path, &body)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(true)
    }
}

fn state_path(repo: &Path) -> std::path::PathBuf {
    repo.join(".agents/mcp-manifest.toml")
}
