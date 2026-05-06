use crate::commands::apply::{profile_match, resolve_agents};
use crate::commands::apply_instructions::resolve_tools;
use crate::config::{MachineConfig, RepoConfig};
use crate::instructions;
use crate::lockfile::{InstructionsEntry, Lockfile};
use crate::manifest::{EntryKind, Manifest};
use crate::paths;
use anyhow::Result;
use console::style;
use similar::{ChangeTag, TextDiff};
use std::collections::BTreeSet;
use std::path::PathBuf;

pub fn run() -> Result<()> {
    let repo = paths::resolve_repo()?;
    let repo_cfg = RepoConfig::load(&repo)?;
    let machine = MachineConfig::load(&repo)?;
    let lock = Lockfile::load(&repo)?;
    let prev_manifest = Manifest::load(&repo)?;
    let home = paths::home_dir()?;

    let mut any_change = false;

    any_change |= diff_symlinks(&repo, &repo_cfg, &machine, &lock, &prev_manifest, &home)?;
    any_change |= diff_instructions(&repo, &repo_cfg, &machine, &lock, &home)?;

    if !any_change {
        println!("no changes");
    }
    Ok(())
}

fn diff_symlinks(
    _repo: &std::path::Path,
    repo_cfg: &RepoConfig,
    machine: &MachineConfig,
    lock: &Lockfile,
    prev_manifest: &Manifest,
    home: &std::path::Path,
) -> Result<bool> {
    let mut planned: BTreeSet<PathBuf> = BTreeSet::new();
    for entry in &lock.skills {
        if !entry.active {
            continue;
        }
        if !profile_match(machine, &entry.profiles) {
            continue;
        }
        let install_root = match &entry.project {
            Some(alias) => match machine.projects.get(alias) {
                Some(p) if p.exists() => p.clone(),
                _ => continue,
            },
            None => home.to_path_buf(),
        };
        for agent in resolve_agents(entry, repo_cfg) {
            if let Ok(link) = paths::agent_skill_path(&install_root, &agent, &entry.name) {
                planned.insert(link);
            }
        }
    }

    let prev_links: BTreeSet<PathBuf> = prev_manifest
        .entries
        .iter()
        .filter(|e| matches!(e.kind, EntryKind::Symlink))
        .map(|e| e.path.clone())
        .collect();

    let added: Vec<&PathBuf> = planned.difference(&prev_links).collect();
    let removed: Vec<&PathBuf> = prev_links.difference(&planned).collect();

    if added.is_empty() && removed.is_empty() {
        return Ok(false);
    }

    println!("{}", style("--- a/skills (installed)").bold());
    println!("{}", style("+++ b/skills (planned)").bold());
    for p in removed {
        println!("{}", style(format!("- {}", paths::display_path(p))).red());
    }
    for p in added {
        println!("{}", style(format!("+ {}", paths::display_path(p))).green());
    }
    println!();
    Ok(true)
}

fn diff_instructions(
    repo: &std::path::Path,
    repo_cfg: &RepoConfig,
    machine: &MachineConfig,
    lock: &Lockfile,
    home: &std::path::Path,
) -> Result<bool> {
    let template_path = paths::instructions_template(repo);
    if !template_path.exists() || machine.instructions_skip {
        return Ok(false);
    }
    let entry = lock
        .instructions
        .clone()
        .unwrap_or_else(InstructionsEntry::default);
    let tools = resolve_tools(repo_cfg, &entry);
    let template_src = instructions::read_template(repo)?;
    let hostname = instructions::current_hostname();

    let mut printed = false;
    for tool in tools {
        let ctx = instructions::build_context(repo_cfg, machine, &hostname, tool);
        let rendered = instructions::render(&template_src, &ctx)?;
        let out = instructions::output_path(home, tool);
        let current = std::fs::read_to_string(&out).unwrap_or_default();
        if current == rendered {
            continue;
        }
        printed = true;
        let label = paths::display_path(&out);
        println!("{}", style(format!("--- a/{}", label)).bold());
        println!("{}", style(format!("+++ b/{}", label)).bold());
        print_unified_diff(&current, &rendered);
        println!();
    }
    Ok(printed)
}

fn print_unified_diff(old: &str, new: &str) {
    let diff = TextDiff::from_lines(old, new);
    for group in diff.grouped_ops(3) {
        let (old_start, old_len, new_start, new_len) = hunk_header(&group);
        println!(
            "{}",
            style(format!(
                "@@ -{},{} +{},{} @@",
                old_start + 1,
                old_len,
                new_start + 1,
                new_len
            ))
            .cyan()
        );
        for op in group {
            for change in diff.iter_changes(&op) {
                let line = change.to_string();
                let styled = match change.tag() {
                    ChangeTag::Delete => style(format!("-{}", line)).red().to_string(),
                    ChangeTag::Insert => style(format!("+{}", line)).green().to_string(),
                    ChangeTag::Equal => style(format!(" {}", line)).dim().to_string(),
                };
                print!("{}", styled);
            }
        }
    }
}

fn hunk_header(group: &[similar::DiffOp]) -> (usize, usize, usize, usize) {
    let first = group.first().expect("non-empty group");
    let last = group.last().expect("non-empty group");
    let old_start = first.as_tag_tuple().1.start;
    let new_start = first.as_tag_tuple().2.start;
    let old_end = last.as_tag_tuple().1.end;
    let new_end = last.as_tag_tuple().2.end;
    (
        old_start,
        old_end - old_start,
        new_start,
        new_end - new_start,
    )
}
