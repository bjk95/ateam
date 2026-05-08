use crate::cli::{ApplyArgs, HarnessCommand};
use crate::config::RepoConfig;
use crate::git_sync;
use crate::paths;
use crate::ui;
use anyhow::{anyhow, Result};
use console::style;

pub fn run(cmd: HarnessCommand, no_sync: bool) -> Result<()> {
    match cmd {
        HarnessCommand::List => list(),
        HarnessCommand::Add { ids } => add(ids, no_sync),
        HarnessCommand::Remove { ids } => remove(ids, no_sync),
    }
}

fn list() -> Result<()> {
    let repo = paths::resolve_repo()?;
    let repo_cfg = RepoConfig::load(&repo)?;

    // Column widths sized to header+content. Held outside of styled() because
    // ANSI escape codes from console::style break width-based padding.
    let id_w = std::cmp::max(
        "ID".len(),
        crate::harness::all().map(|a| a.id.len()).max().unwrap_or(0),
    );
    let status_w = "disabled".len();
    let skills_w = std::cmp::max(
        "SKILLS DIR".len(),
        crate::harness::all()
            .filter_map(|a| a.skills_subdir.map(|s| s.len() + 2)) // +2 for "~/"
            .max()
            .unwrap_or(0),
    );

    let header = format!(
        "{:<id_w$}  {:<status_w$}  {:<skills_w$}  {}",
        "ID",
        "STATUS",
        "SKILLS DIR",
        "INSTRUCTIONS FILE",
        id_w = id_w,
        status_w = status_w,
        skills_w = skills_w,
    );
    ui::plain(format!("{}", style(header).bold()));

    for def in crate::harness::all() {
        let enabled = repo_cfg.enabled_harnesses.iter().any(|a| a == def.id);
        let status = if enabled { "enabled" } else { "disabled" };
        let skills = def
            .skills_subdir
            .map(|s| format!("~/{}", s))
            .unwrap_or_else(|| "—".to_string());
        let instr = def
            .instructions_file
            .map(|s| format!("~/{}", s))
            .unwrap_or_else(|| "—".to_string());
        let line = format!(
            "{:<id_w$}  {:<status_w$}  {:<skills_w$}  {}",
            def.id,
            status,
            skills,
            instr,
            id_w = id_w,
            status_w = status_w,
            skills_w = skills_w,
        );
        if enabled {
            ui::plain(line);
        } else {
            ui::plain(format!("{}", style(line).dim()));
        }
    }
    Ok(())
}

fn add(ids: Vec<String>, no_sync: bool) -> Result<()> {
    validate_harness_ids(&ids)?;

    let repo = paths::resolve_repo()?;
    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    let mut repo_cfg = RepoConfig::load(&repo)?;
    let plan = plan_add(&repo_cfg.enabled_harnesses, &ids);

    if plan.added.is_empty() {
        for id in &plan.already_present {
            ui::ok(format!("{} already enabled", id));
        }
        return Ok(());
    }

    repo_cfg.enabled_harnesses = plan.next;
    repo_cfg.write(&repo)?;

    for id in &plan.already_present {
        ui::ok(format!("{} already enabled", id));
    }
    for id in &plan.added {
        ui::ok(format!("enabled {}", id));
    }

    crate::commands::apply::run(
        ApplyArgs {
            dry_run: false,
            harnesses: Vec::new(),
            project: None,
            force: false,
            copy: false,
        },
        true,
    )?;

    if git_sync::enabled(no_sync) {
        let msg = git_sync::msg_harness_add(&plan.added);
        let _ = git_sync::commit_and_push(&repo, &msg);
    }

    Ok(())
}

fn remove(ids: Vec<String>, no_sync: bool) -> Result<()> {
    validate_harness_ids(&ids)?;

    let repo = paths::resolve_repo()?;
    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    let mut repo_cfg = RepoConfig::load(&repo)?;
    let plan = plan_remove(&repo_cfg.enabled_harnesses, &ids)?;

    if plan.removed.is_empty() {
        for id in &plan.already_absent {
            ui::ok(format!("{} already disabled", id));
        }
        return Ok(());
    }

    repo_cfg.enabled_harnesses = plan.next;
    repo_cfg.write(&repo)?;

    for id in &plan.already_absent {
        ui::ok(format!("{} already disabled", id));
    }
    for id in &plan.removed {
        ui::ok(format!("disabled {}", id));
    }

    crate::commands::apply::run(
        ApplyArgs {
            dry_run: false,
            harnesses: Vec::new(),
            project: None,
            force: false,
            copy: false,
        },
        true,
    )?;

    if git_sync::enabled(no_sync) {
        let msg = git_sync::msg_harness_remove(&plan.removed);
        let _ = git_sync::commit_and_push(&repo, &msg);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Pure helpers (testable without filesystem)

fn validate_harness_ids(ids: &[String]) -> Result<()> {
    let valid: Vec<&'static str> = crate::harness::ids().collect();
    for id in ids {
        if !valid.iter().any(|v| v == id) {
            return Err(anyhow!(
                "unknown harness `{}`\n  valid: {}",
                id,
                valid.join(", ")
            ));
        }
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct AddPlan {
    next: Vec<String>,
    added: Vec<String>,
    already_present: Vec<String>,
}

fn plan_add(current: &[String], to_add: &[String]) -> AddPlan {
    let mut next = current.to_vec();
    let mut added = Vec::new();
    let mut already_present = Vec::new();
    for id in to_add {
        if next.iter().any(|a| a == id) {
            already_present.push(id.clone());
        } else {
            next.push(id.clone());
            added.push(id.clone());
        }
    }
    AddPlan {
        next,
        added,
        already_present,
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RemovePlan {
    next: Vec<String>,
    removed: Vec<String>,
    already_absent: Vec<String>,
}

fn plan_remove(current: &[String], to_remove: &[String]) -> Result<RemovePlan> {
    let mut next = current.to_vec();
    let mut removed = Vec::new();
    let mut already_absent = Vec::new();
    for id in to_remove {
        if let Some(pos) = next.iter().position(|a| a == id) {
            next.remove(pos);
            removed.push(id.clone());
        } else {
            already_absent.push(id.clone());
        }
    }
    if !removed.is_empty() && next.is_empty() {
        return Err(anyhow!(
            "cannot remove last enabled harness (would disable agents).\n  use 'agents harness add <id>' first, or edit agents.toml manually if you really want this."
        ));
    }
    Ok(RemovePlan {
        next,
        removed,
        already_absent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn validate_accepts_registry_ids() {
        assert!(validate_harness_ids(&s(&["claude-code", "codex"])).is_ok());
        assert!(validate_harness_ids(&s(&["opencode", "gemini"])).is_ok());
    }

    #[test]
    fn validate_rejects_unknown_id() {
        let err = validate_harness_ids(&s(&["not-an-agent"])).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("unknown harness `not-an-agent`"), "got: {msg}");
        assert!(msg.contains("claude-code"), "got: {msg}");
        assert!(msg.contains("gemini"), "got: {msg}");
    }

    #[test]
    fn validate_rejects_when_any_unknown_in_batch() {
        let err = validate_harness_ids(&s(&["claude-code", "fake"])).unwrap_err();
        assert!(format!("{err}").contains("unknown harness `fake`"));
    }

    #[test]
    fn plan_add_appends_new_id() {
        let plan = plan_add(&s(&["claude-code", "codex"]), &s(&["gemini"]));
        assert_eq!(plan.next, s(&["claude-code", "codex", "gemini"]));
        assert_eq!(plan.added, s(&["gemini"]));
        assert!(plan.already_present.is_empty());
    }

    #[test]
    fn plan_add_idempotent_for_existing() {
        let plan = plan_add(&s(&["claude-code", "codex"]), &s(&["codex"]));
        assert_eq!(plan.next, s(&["claude-code", "codex"]));
        assert!(plan.added.is_empty());
        assert_eq!(plan.already_present, s(&["codex"]));
    }

    #[test]
    fn plan_add_mixed_new_and_existing() {
        let plan = plan_add(&s(&["claude-code"]), &s(&["claude-code", "gemini"]));
        assert_eq!(plan.next, s(&["claude-code", "gemini"]));
        assert_eq!(plan.added, s(&["gemini"]));
        assert_eq!(plan.already_present, s(&["claude-code"]));
    }

    #[test]
    fn plan_remove_drops_id() {
        let plan = plan_remove(&s(&["claude-code", "codex", "gemini"]), &s(&["gemini"])).unwrap();
        assert_eq!(plan.next, s(&["claude-code", "codex"]));
        assert_eq!(plan.removed, s(&["gemini"]));
        assert!(plan.already_absent.is_empty());
    }

    #[test]
    fn plan_remove_idempotent_for_missing() {
        let plan = plan_remove(&s(&["claude-code", "codex"]), &s(&["gemini"])).unwrap();
        assert_eq!(plan.next, s(&["claude-code", "codex"]));
        assert!(plan.removed.is_empty());
        assert_eq!(plan.already_absent, s(&["gemini"]));
    }

    #[test]
    fn plan_remove_refuses_to_empty_the_list() {
        let err = plan_remove(&s(&["claude-code"]), &s(&["claude-code"])).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("cannot remove last enabled harness"),
            "got: {msg}"
        );
    }

    #[test]
    fn plan_remove_already_empty_with_only_absents_is_ok() {
        // If the user already has an empty list and asks to remove a missing
        // agent, we don't trip the "would empty" guard — nothing was actually
        // removed, so nothing changes.
        let plan = plan_remove(&s(&[]), &s(&["claude-code"])).unwrap();
        assert!(plan.removed.is_empty());
        assert_eq!(plan.already_absent, s(&["claude-code"]));
    }
}
