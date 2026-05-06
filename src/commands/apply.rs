use crate::cli::ApplyArgs;
use crate::config::{MachineConfig, RepoConfig};
use crate::git_sync;
use crate::install;
use crate::lockfile::{Lockfile, SkillEntry};
use crate::manifest::{self, EntryKind, Manifest, ManifestEntry};
use crate::paths;
use crate::source::{github, Source};
use anyhow::{anyhow, bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

pub fn run(args: ApplyArgs, no_sync: bool) -> Result<()> {
    let repo = paths::resolve_repo()?;
    let repo_cfg = RepoConfig::load(&repo)?;
    let machine = MachineConfig::load(&repo)?;

    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    install::sweep_cache_tmp(&repo).ok();

    let lock = Lockfile::load(&repo)?;
    let prev_manifest = Manifest::load(&repo)?;
    let mut new_manifest = Manifest::default();

    let mut unregistered_aliases: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut materialized = 0usize;
    let mut lockfile_dirty = false;

    let target_agents: Option<BTreeSet<String>> = if args.agents.is_empty() {
        None
    } else {
        Some(args.agents.iter().cloned().collect())
    };

    let mut updated_lock = lock.clone();

    for entry in &lock.skills {
        if !profile_match(&machine, &entry.profiles) {
            continue;
        }

        let install_root = match &entry.project {
            Some(alias) => {
                if let Some(filter) = &args.project {
                    if filter != alias {
                        continue;
                    }
                }
                match machine.projects.get(alias) {
                    Some(path) => {
                        if !path.exists() {
                            eprintln!(
                                "ateam: warning — project `{}` path does not exist: {}",
                                alias,
                                path.display()
                            );
                            continue;
                        }
                        path.clone()
                    }
                    None => {
                        unregistered_aliases
                            .entry(alias.clone())
                            .or_default()
                            .push(entry.name.clone());
                        continue;
                    }
                }
            }
            None => {
                if args.project.is_some() {
                    continue;
                }
                paths::home_dir()?
            }
        };

        let agents = resolve_agents(entry, &repo_cfg);
        let canonical = match resolve_canonical(&repo, entry) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("ateam: failed to resolve `{}`: {:#}", entry.name, e);
                continue;
            }
        };

        // Detect tree_sha drift for github sources during apply.
        if let (Ok(Source::Github { owner, repo: r }), Some(_)) =
            (Source::from_lockfile_string(&entry.source), entry.tree_sha.as_ref())
        {
            if let Some(path) = &entry.path {
                if let Ok(commit_sha) = github::resolve_ref(
                    &owner,
                    &r,
                    entry.git_ref.as_deref().unwrap_or(github::default_branch_fallback()),
                ) {
                    if let Ok(Some(latest)) = github::subtree_sha(&owner, &r, &commit_sha, path) {
                        if Some(&latest) != entry.tree_sha.as_ref() {
                            tracing::info!("drift detected for {} (refetching)", entry.name);
                            if let Err(e) = refetch_github(&repo, &owner, &r, &commit_sha, path, &entry.name) {
                                eprintln!(
                                    "ateam: failed to refetch {}: {:#}",
                                    entry.name, e
                                );
                            } else {
                                if let Some(pos) = updated_lock
                                    .skills
                                    .iter()
                                    .position(|s| s.name == entry.name)
                                {
                                    updated_lock.skills[pos].tree_sha = Some(latest);
                                    lockfile_dirty = true;
                                }
                            }
                        }
                    }
                }
            }
        }

        if args.dry_run {
            for agent in &agents {
                if let Some(filter) = &target_agents {
                    if !filter.contains(agent) {
                        continue;
                    }
                }
                let link = paths::agent_skill_path(&install_root, agent, &entry.name)?;
                println!("would link {} → {}", link.display(), canonical.display());
            }
            continue;
        }

        for agent in &agents {
            if let Some(filter) = &target_agents {
                if !filter.contains(agent) {
                    continue;
                }
            }
            let link = paths::agent_skill_path(&install_root, agent, &entry.name)?;
            match install::install_symlink(&link, &canonical, args.force)? {
                install::LinkOutcome::Created
                | install::LinkOutcome::Replaced
                | install::LinkOutcome::AlreadyCorrect
                | install::LinkOutcome::MovedAside { .. } => {
                    new_manifest.entries.push(ManifestEntry {
                        path: link.clone(),
                        kind: EntryKind::Symlink,
                        skill: entry.name.clone(),
                        agent: agent.clone(),
                        target: canonical.clone(),
                        applied_at: manifest::now_unix(),
                    });
                    materialized += 1;
                }
                install::LinkOutcome::Refused => {
                    eprintln!(
                        "ateam: refused to install {} for {} (real dir at {}; rerun with --force to move aside)",
                        entry.name,
                        agent,
                        link.display()
                    );
                }
            }
        }
    }

    // Removal: paths in old manifest not in new plan get unlinked.
    if !args.dry_run {
        let new_paths: HashSet<&Path> =
            new_manifest.entries.iter().map(|e| e.path.as_path()).collect();
        for prev in &prev_manifest.entries {
            if !new_paths.contains(prev.path.as_path()) {
                if let Err(e) = install::uninstall_path(&prev.path) {
                    eprintln!("ateam: warning — couldn't remove {}: {:#}", prev.path.display(), e);
                }
            }
        }
        new_manifest.write(&repo)?;
    }

    if !unregistered_aliases.is_empty() {
        let total: usize = unregistered_aliases.values().map(|v| v.len()).sum();
        println!(
            "note: {} lockfile {} reference unregistered project alias{}:",
            total,
            if total == 1 { "entry" } else { "entries" },
            if unregistered_aliases.len() == 1 { "" } else { "es" }
        );
        for (alias, names) in &unregistered_aliases {
            println!("  - {} ({})", alias, names.join(", "));
        }
        println!("register with: ateam project add <alias> <path>");
    }

    if lockfile_dirty && !args.dry_run {
        updated_lock.write(&repo)?;
    }

    if !args.dry_run && git_sync::enabled(no_sync) {
        if lockfile_dirty {
            let msg = git_sync::msg_apply(materialized);
            let _ = git_sync::commit_and_push(&repo, &msg);
        }
    }

    if args.dry_run {
        println!("dry run complete (no changes written)");
    } else {
        println!("ateam: applied {} symlink(s)", materialized);
    }

    Ok(())
}

fn profile_match(machine: &MachineConfig, gates: &[String]) -> bool {
    if gates.is_empty() {
        return true;
    }
    gates.iter().any(|g| machine.profiles.iter().any(|p| p == g))
}

fn resolve_agents(entry: &SkillEntry, repo_cfg: &RepoConfig) -> Vec<String> {
    if entry.agents.iter().any(|a| a == "*") {
        repo_cfg.enabled_agents.clone()
    } else {
        entry.agents.clone()
    }
}

fn resolve_canonical(repo: &Path, entry: &SkillEntry) -> Result<PathBuf> {
    let source = Source::from_lockfile_string(&entry.source)?;
    match source {
        Source::Local { path } => crate::source::local::resolve(repo, &path),
        Source::Github { .. } | Source::Git { .. } => {
            let cache = paths::cache_dir(repo).join(&entry.name);
            if cache.exists() {
                Ok(cache)
            } else {
                // Cold cache — refetch from upstream.
                refetch_for_entry(repo, entry).context("fetching cold cache")?;
                Ok(paths::cache_dir(repo).join(&entry.name))
            }
        }
    }
}

fn refetch_for_entry(repo: &Path, entry: &SkillEntry) -> Result<()> {
    let source = Source::from_lockfile_string(&entry.source)?;
    let path = entry
        .path
        .clone()
        .ok_or_else(|| anyhow!("lockfile entry `{}` missing `path`", entry.name))?;
    match source {
        Source::Github { owner, repo: r } => {
            let r_ref = entry.git_ref.clone().unwrap_or_else(|| github::default_branch_fallback().to_string());
            let commit_sha = github::resolve_ref(&owner, &r, &r_ref)?;
            refetch_github(repo, &owner, &r, &commit_sha, &path, &entry.name)
        }
        Source::Git { url } => {
            let tmp_root = paths::cache_tmp_dir(repo);
            std::fs::create_dir_all(&tmp_root)?;
            let suffix: u64 = rand::random();
            let work = tmp_root.join(format!("git-{:016x}", suffix));
            crate::source::git::clone(&url, entry.git_ref.as_deref(), &work)?;
            let src_dir = work.join(&path);
            let slot = install::prepare_cache_slot(repo, &entry.name)?;
            install::copy_dir_recursive(&src_dir, &slot.tmp)?;
            slot.commit()?;
            let _ = std::fs::remove_dir_all(&work);
            Ok(())
        }
        Source::Local { .. } => Ok(()),
    }
}

fn refetch_github(
    repo: &Path,
    owner: &str,
    repo_name: &str,
    commit_sha: &str,
    sub_path: &str,
    skill_name: &str,
) -> Result<()> {
    let tmp_root = paths::cache_tmp_dir(repo);
    std::fs::create_dir_all(&tmp_root)?;
    let suffix: u64 = rand::random();
    let work = tmp_root.join(format!("fetch-{:016x}", suffix));
    std::fs::create_dir_all(&work)?;
    let pkg_root = github::fetch_tarball(owner, repo_name, commit_sha, &work)?;
    let src_dir = pkg_root.join(sub_path);
    if !src_dir.exists() {
        bail!("path `{}` not found in {}/{}@{}", sub_path, owner, repo_name, commit_sha);
    }
    let slot = install::prepare_cache_slot(repo, skill_name)?;
    install::copy_dir_recursive(&src_dir, &slot.tmp)?;
    slot.commit()?;
    let _ = std::fs::remove_dir_all(&work);
    Ok(())
}
