use crate::cli::{McpAddArgs, McpCommand, McpNameArgs};
use crate::config::{MachineConfig, RepoConfig};
use crate::git_sync;
use crate::lockfile::{Lockfile, McpEntry};
use crate::paths;
use crate::ui;
use anyhow::{bail, Result};
use console::style;
use std::collections::BTreeMap;

pub fn run(cmd: McpCommand, no_sync: bool) -> Result<()> {
    match cmd {
        McpCommand::Add(args) => add(args, no_sync),
        McpCommand::Remove(args) => remove(args, no_sync),
        McpCommand::List => list(),
        McpCommand::Activate(args) => set_active(args, true, no_sync),
        McpCommand::Deactivate(args) => set_active(args, false, no_sync),
    }
}

fn add(args: McpAddArgs, no_sync: bool) -> Result<()> {
    crate::mcp::validate_harnesses(&args.harnesses)?;
    let entry = entry_from_add_args(args)?;
    let repo = paths::resolve_repo()?;
    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }
    let repo_cfg = RepoConfig::load(&repo)?;
    let machine = MachineConfig::load(&repo)?;
    let mut lock = Lockfile::load(&repo)?;
    lock.upsert_mcp(entry.clone());
    lock.write(&repo)?;

    crate::mcp::apply(
        &repo,
        &paths::home_dir()?,
        &repo_cfg,
        &lock.mcps,
        &machine,
        None,
        false,
        false,
    )?;

    if git_sync::enabled(no_sync) {
        let msg = git_sync::msg_mcp_add(&entry.name);
        if let Err(e) = git_sync::commit_and_push(&repo, &msg) {
            ui::warn(format!("auto-sync failed: {:#}", e));
            ui::detail("local change saved; rerun a mutating command to retry");
        }
    }
    ui::ok(format!("added MCP {}", entry.name));
    Ok(())
}

fn entry_from_add_args(args: McpAddArgs) -> Result<McpEntry> {
    let env = parse_env(&args.env)?;
    if args.url.is_some() {
        if !args.command.is_empty() {
            bail!("HTTP MCP servers use --url and cannot also specify a command");
        }
        if !env.is_empty() {
            bail!("--env only applies to stdio MCP servers");
        }
        return Ok(McpEntry {
            name: args.name,
            transport: "http".into(),
            command: None,
            args: Vec::new(),
            env,
            url: args.url,
            bearer_token_env_var: args.bearer_token_env_var,
            harnesses: harnesses_or_wildcard(args.harnesses),
            profiles: args.profile,
            active: true,
        });
    }
    if args.bearer_token_env_var.is_some() {
        bail!("--bearer-token-env-var only applies to HTTP MCP servers");
    }
    let Some(command) = args.command.first().cloned() else {
        bail!("stdio MCP servers require a command after `--`");
    };
    Ok(McpEntry {
        name: args.name,
        transport: "stdio".into(),
        command: Some(command),
        args: args.command.into_iter().skip(1).collect(),
        env,
        url: None,
        bearer_token_env_var: None,
        harnesses: harnesses_or_wildcard(args.harnesses),
        profiles: args.profile,
        active: true,
    })
}

fn harnesses_or_wildcard(harnesses: Vec<String>) -> Vec<String> {
    if harnesses.is_empty() {
        vec!["*".into()]
    } else {
        harnesses
    }
}

fn parse_env(raw: &[String]) -> Result<BTreeMap<String, String>> {
    let mut env = BTreeMap::new();
    for item in raw {
        let Some((key, value)) = item.split_once('=') else {
            bail!("invalid --env `{}`; expected KEY=VALUE", item);
        };
        if key.is_empty() {
            bail!("invalid --env `{}`; key is empty", item);
        }
        env.insert(key.to_string(), value.to_string());
    }
    Ok(env)
}

fn remove(args: McpNameArgs, no_sync: bool) -> Result<()> {
    let repo = paths::resolve_repo()?;
    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }
    let repo_cfg = RepoConfig::load(&repo)?;
    let machine = MachineConfig::load(&repo)?;
    let mut lock = Lockfile::load(&repo)?;
    let mut removed = Vec::new();
    for name in &args.names {
        match lock.remove_mcp(name) {
            Some(mut entry) => {
                entry.active = false;
                removed.push(entry);
            }
            None => bail!("no MCP named `{}` in lockfile", name),
        }
    }
    lock.write(&repo)?;
    let mut apply_entries = lock.mcps.clone();
    apply_entries.extend(removed);
    crate::mcp::apply(
        &repo,
        &paths::home_dir()?,
        &repo_cfg,
        &apply_entries,
        &machine,
        None,
        false,
        false,
    )?;

    if git_sync::enabled(no_sync) {
        let msg = git_sync::msg_mcp_remove(&args.names);
        if let Err(e) = git_sync::commit_and_push(&repo, &msg) {
            ui::warn(format!("auto-sync failed: {:#}", e));
            ui::detail("local change saved; rerun a mutating command to retry");
        }
    }
    ui::ok(format!("removed MCP {}", args.names.join(", ")));
    Ok(())
}

fn set_active(args: McpNameArgs, active: bool, no_sync: bool) -> Result<()> {
    let repo = paths::resolve_repo()?;
    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }
    let repo_cfg = RepoConfig::load(&repo)?;
    let machine = MachineConfig::load(&repo)?;
    let mut lock = Lockfile::load(&repo)?;
    let mut changed = Vec::new();
    for name in &args.names {
        let Some(idx) = lock.mcps.iter().position(|entry| entry.name == *name) else {
            bail!("no MCP named `{}` in lockfile", name);
        };
        if lock.mcps[idx].active == active {
            ui::plain(format!(
                "agents: MCP `{}` already {}",
                name,
                if active { "active" } else { "deactivated" }
            ));
            continue;
        }
        lock.mcps[idx].active = active;
        changed.push(lock.mcps[idx].clone());
    }
    if changed.is_empty() {
        return Ok(());
    }
    lock.write(&repo)?;
    crate::mcp::apply(
        &repo,
        &paths::home_dir()?,
        &repo_cfg,
        &lock.mcps,
        &machine,
        None,
        false,
        active,
    )?;

    if git_sync::enabled(no_sync) {
        let msg = if active {
            git_sync::msg_mcp_activate(&args.names)
        } else {
            git_sync::msg_mcp_deactivate(&args.names)
        };
        if let Err(e) = git_sync::commit_and_push(&repo, &msg) {
            ui::warn(format!("auto-sync failed: {:#}", e));
            ui::detail("local change saved; rerun a mutating command to retry");
        }
    }
    ui::ok(format!(
        "{} MCP {}",
        if active { "activated" } else { "deactivated" },
        args.names.join(", ")
    ));
    Ok(())
}

fn list() -> Result<()> {
    let repo = paths::resolve_repo()?;
    let lock = Lockfile::load(&repo)?;
    if lock.mcps.is_empty() {
        ui::plain("no MCPs locked");
        return Ok(());
    }
    for entry in &lock.mcps {
        let dot = if entry.active { "●" } else { "○" };
        ui::plain(format!(
            "{} {}  {}",
            style(dot).cyan(),
            style(&entry.name).bold(),
            entry.transport
        ));
        if !entry.profiles.is_empty() {
            ui::detail(format!("profiles: {}", entry.profiles.join(", ")));
        }
        if !entry.harnesses.iter().any(|h| h == "*") {
            ui::detail(format!("harnesses: {}", entry.harnesses.join(", ")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_env_rejects_missing_equals() {
        let err = parse_env(&["KEY".into()]).unwrap_err();
        assert!(format!("{err}").contains("KEY=VALUE"));
    }

    #[test]
    fn stdio_entry_uses_first_command_arg_as_command() {
        let entry = entry_from_add_args(McpAddArgs {
            name: "otter".into(),
            url: None,
            bearer_token_env_var: None,
            env: vec![],
            harnesses: vec!["codex".into()],
            profile: vec!["canva".into()],
            command: vec!["otter".into(), "mcp".into(), "serve".into()],
        })
        .unwrap();

        assert_eq!(entry.transport, "stdio");
        assert_eq!(entry.command.as_deref(), Some("otter"));
        assert_eq!(entry.args, vec!["mcp", "serve"]);
        assert_eq!(entry.harnesses, vec!["codex"]);
        assert_eq!(entry.profiles, vec!["canva"]);
    }

    #[test]
    fn http_entry_uses_url() {
        let entry = entry_from_add_args(McpAddArgs {
            name: "supabase".into(),
            url: Some("https://example.com/mcp".into()),
            bearer_token_env_var: Some("TOKEN".into()),
            env: vec![],
            harnesses: vec![],
            profile: vec![],
            command: vec![],
        })
        .unwrap();

        assert_eq!(entry.transport, "http");
        assert_eq!(entry.url.as_deref(), Some("https://example.com/mcp"));
        assert_eq!(entry.bearer_token_env_var.as_deref(), Some("TOKEN"));
        assert_eq!(entry.harnesses, vec!["*"]);
    }

    #[test]
    fn http_entry_rejects_env() {
        let err = entry_from_add_args(McpAddArgs {
            name: "supabase".into(),
            url: Some("https://example.com/mcp".into()),
            bearer_token_env_var: None,
            env: vec!["TOKEN=value".into()],
            harnesses: vec![],
            profile: vec![],
            command: vec![],
        })
        .unwrap_err();

        assert!(format!("{err}").contains("stdio"));
    }
}
