use crate::cli::RemoveArgs;
use crate::git_sync;
use crate::install;
use crate::lockfile::{Lockfile, SkillEntry};
use crate::manifest::Manifest;
use crate::paths;
use crate::ui;
use anyhow::{bail, Result};
use std::io::{IsTerminal, Read};
use std::path::Path;

pub fn run(mut args: RemoveArgs, no_sync: bool) -> Result<()> {
    if args.names.is_empty() && !args.all && !std::io::stdin().is_terminal() {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        args.names = parse_stdin_names(&buf);
    }

    let repo = paths::resolve_repo()?;

    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    let lock = Lockfile::load(&repo)?;
    let targets = resolve_targets(&args, &lock.skills)?;
    if targets.is_empty() {
        bail!("no matching skills to remove");
    }
    drop(lock);

    if !confirm(&args, &targets)? {
        ui::warn("aborted");
        return Ok(());
    }

    let mut removed_names: Vec<String> = Vec::new();
    let mut had_error = false;
    for name in &targets {
        match remove_one(&repo, name) {
            Ok(()) => {
                removed_names.push(name.clone());
                ui::ok(format!("removed {}", name));
            }
            Err(e) => {
                had_error = true;
                ui::fail(format!("remove {} — {:#}", name, e));
            }
        }
    }

    if removed_names.is_empty() {
        if had_error {
            bail!("no skills removed (all failed)");
        }
        return Ok(());
    }

    if git_sync::enabled(no_sync) {
        let msg = git_sync::msg_remove(&removed_names);
        if let Err(e) = git_sync::commit_and_push(&repo, &msg) {
            ui::warn(format!("auto-sync failed: {:#}", e));
            ui::detail("local change saved; rerun a mutating command to retry");
        }
    }

    Ok(())
}

fn parse_stdin_names(input: &str) -> Vec<String> {
    input.split_whitespace().map(String::from).collect()
}

fn resolve_targets(args: &RemoveArgs, skills: &[SkillEntry]) -> Result<Vec<String>> {
    if !args.all && args.names.is_empty() {
        bail!("specify skill name(s) or pass --all");
    }

    let pool: Vec<&SkillEntry> = skills
        .iter()
        .filter(|e| matches_filters(e, args))
        .collect();

    if args.all {
        return Ok(pool.iter().map(|e| e.name.clone()).collect());
    }

    let mut out = Vec::with_capacity(args.names.len());
    let mut missing: Vec<&str> = Vec::new();
    for raw in &args.names {
        let normalized = crate::lockfile::normalize_skill_name(raw)?;
        if pool.iter().any(|e| e.name == normalized) {
            if !out.contains(&normalized) {
                out.push(normalized);
            }
        } else {
            missing.push(raw.as_str());
        }
    }
    if !missing.is_empty() {
        bail!(
            "no skill named `{}` in lockfile (within selected scope)",
            missing.join("`, `")
        );
    }
    Ok(out)
}

fn matches_filters(entry: &SkillEntry, args: &RemoveArgs) -> bool {
    if args.global && entry.project.is_some() {
        return false;
    }
    if !args.harnesses.is_empty() {
        let wanted_all = args.harnesses.iter().any(|a| a == "*");
        let entry_all = entry.harnesses.iter().any(|a| a == "*");
        if !wanted_all && !entry_all {
            let hit = args
                .harnesses
                .iter()
                .any(|wanted| entry.harnesses.iter().any(|have| have == wanted));
            if !hit {
                return false;
            }
        }
    }
    true
}

fn confirm(args: &RemoveArgs, targets: &[String]) -> Result<bool> {
    if args.yes || !std::io::stdin().is_terminal() {
        return Ok(true);
    }
    use dialoguer::{theme::ColorfulTheme, Confirm};
    let prompt = format!(
        "Remove {} skill{}: {}?",
        targets.len(),
        if targets.len() == 1 { "" } else { "s" },
        targets.join(", ")
    );
    let answer = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .default(false)
        .interact()?;
    Ok(answer)
}

fn remove_one(repo: &Path, name: &str) -> Result<()> {
    let mut lock = Lockfile::load(repo)?;
    let removed = match lock.remove(name) {
        Some(e) => e,
        None => bail!("no skill named `{}` in lockfile", name),
    };
    lock.write(repo)?;

    let mut manifest = Manifest::load(repo)?;
    let to_remove: Vec<_> = manifest
        .entries
        .iter()
        .filter(|m| m.skill == name)
        .map(|m| m.path.clone())
        .collect();
    for path in &to_remove {
        if let Err(e) = install::uninstall_path(path) {
            ui::warn(format!(
                "couldn't remove {}: {:#}",
                paths::display_path(path),
                e
            ));
        }
    }
    manifest.entries.retain(|m| m.skill != name);
    manifest.write(repo)?;

    let snapshot = paths::local_skills_dir(repo).join(name);
    let snapshot_managed = if removed.source.starts_with("local:") {
        removed.source == format!("local:skills/{}", name)
    } else {
        true
    };
    if snapshot_managed && snapshot.exists() {
        if let Err(e) = std::fs::remove_dir_all(&snapshot) {
            if e.kind() != std::io::ErrorKind::NotFound {
                ui::warn(format!(
                    "couldn't remove {}: {:#}",
                    paths::display_path(&snapshot),
                    e
                ));
            }
        }
    }
    let legacy_cache = paths::cache_dir(repo).join(name);
    if legacy_cache.exists() {
        if let Err(e) = std::fs::remove_dir_all(&legacy_cache) {
            if e.kind() != std::io::ErrorKind::NotFound {
                ui::warn(format!(
                    "couldn't remove {}: {:#}",
                    paths::display_path(&legacy_cache),
                    e
                ));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, harnesses: &[&str], project: Option<&str>) -> SkillEntry {
        SkillEntry {
            name: name.into(),
            source: format!("local:skills/{}", name),
            path: None,
            git_ref: None,
            tree_sha: None,
            harnesses: harnesses.iter().map(|s| (*s).into()).collect(),
            profiles: vec![],
            project: project.map(|s| s.into()),
            active: true,
            upstream: None,
        }
    }

    fn args(names: &[&str], all: bool, harnesses: &[&str], global: bool) -> RemoveArgs {
        RemoveArgs {
            names: names.iter().map(|s| (*s).into()).collect(),
            all,
            yes: true,
            harnesses: harnesses.iter().map(|s| (*s).into()).collect(),
            global,
        }
    }

    #[test]
    fn requires_names_or_all() {
        let skills = vec![entry("foo", &["*"], None)];
        let err = resolve_targets(&args(&[], false, &[], false), &skills).unwrap_err();
        assert!(err.to_string().contains("specify skill name(s) or pass --all"));
    }

    #[test]
    fn all_returns_every_skill() {
        let skills = vec![entry("foo", &["*"], None), entry("bar", &["*"], None)];
        let out = resolve_targets(&args(&[], true, &[], false), &skills).unwrap();
        assert_eq!(out, vec!["foo", "bar"]);
    }

    #[test]
    fn names_normalized_and_deduped() {
        let skills = vec![entry("foo-bar", &["*"], None)];
        let out = resolve_targets(&args(&["Foo Bar", "foo-bar"], false, &[], false), &skills)
            .unwrap();
        assert_eq!(out, vec!["foo-bar"]);
    }

    #[test]
    fn missing_name_bails_with_message() {
        let skills = vec![entry("foo", &["*"], None)];
        let err = resolve_targets(&args(&["nope"], false, &[], false), &skills).unwrap_err();
        assert!(err.to_string().contains("nope"));
    }

    #[test]
    fn global_filter_excludes_project_scoped() {
        let skills = vec![
            entry("foo", &["*"], None),
            entry("bar", &["*"], Some("canva")),
        ];
        let out = resolve_targets(&args(&[], true, &[], true), &skills).unwrap();
        assert_eq!(out, vec!["foo"]);
    }

    #[test]
    fn agent_filter_keeps_wildcard_entries() {
        let skills = vec![
            entry("foo", &["*"], None),
            entry("bar", &["claude"], None),
            entry("baz", &["codex"], None),
        ];
        let out = resolve_targets(&args(&[], true, &["claude"], false), &skills).unwrap();
        assert_eq!(out, vec!["foo", "bar"]);
    }

    #[test]
    fn agent_filter_with_named_skill_outside_scope_is_missing() {
        let skills = vec![entry("foo", &["codex"], None)];
        let err =
            resolve_targets(&args(&["foo"], false, &["claude"], false), &skills).unwrap_err();
        assert!(err.to_string().contains("foo"));
    }

    #[test]
    fn parse_stdin_names_splits_on_whitespace() {
        assert_eq!(
            parse_stdin_names("foo\nbar\nbaz\n"),
            vec!["foo", "bar", "baz"]
        );
        assert_eq!(parse_stdin_names("  foo bar\tbaz "), vec!["foo", "bar", "baz"]);
        assert_eq!(parse_stdin_names(""), Vec::<String>::new());
        assert_eq!(parse_stdin_names("\n\n"), Vec::<String>::new());
    }
}
