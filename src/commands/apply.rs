use crate::cli::ApplyArgs;
use crate::commands::apply_instructions;
use crate::config::{MachineConfig, RepoConfig};
use crate::git_sync;
use crate::install;
use crate::lockfile::{Lockfile, SkillEntry};
use crate::manifest::{self, EntryKind, Manifest, ManifestEntry};
use crate::paths;
use crate::source::{github, Source};
use crate::ui;
use anyhow::{anyhow, bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

pub fn run(args: ApplyArgs, no_sync: bool) -> Result<()> {
    let repo = paths::resolve_repo()?;
    let repo_cfg = RepoConfig::load(&repo)?;
    let mut machine = MachineConfig::load(&repo)?;

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

    let scan_step = (!args.dry_run).then(|| ui::step("checking skills"));

    for entry in &lock.skills {
        if !entry.active {
            continue;
        }
        if !profile_match(&machine, &entry.profiles) {
            continue;
        }
        if let Some(s) = &scan_step {
            s.set_msg(format!("checking {}", entry.name));
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
                            ui::warn(format!(
                                "project `{}` path does not exist: {}",
                                alias,
                                paths::display_path(path)
                            ));
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
                ui::fail(format!("resolve {} — {:#}", entry.name, e));
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
                                ui::fail(format!("refetch {} — {:#}", entry.name, e));
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
                ui::detail(format!(
                    "{} → {}",
                    paths::display_path(&link),
                    paths::display_path(&canonical)
                ));
                materialized += 1;
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
                | install::LinkOutcome::MovedAside { .. }
                | install::LinkOutcome::AutoHealed => {
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
                    ui::warn(format!(
                        "refused to install {} for {}: real dir at {} (rerun with --force)",
                        entry.name,
                        agent,
                        paths::display_path(&link)
                    ));
                }
            }
        }
    }

    if let Some(s) = scan_step {
        s.finish();
    }

    // Instructions render-and-write pass.
    let home = paths::home_dir()?;
    let instructions_outcome = apply_instructions::apply(
        &repo,
        &home,
        &repo_cfg,
        &mut updated_lock,
        &mut machine,
        &prev_manifest,
        &mut new_manifest,
        args.dry_run,
        args.force,
    )?;
    let instructions_written = instructions_outcome.written;
    if instructions_outcome.lockfile_dirty {
        lockfile_dirty = true;
    }
    if instructions_outcome.instructions_skip_set && !args.dry_run {
        machine.write(&repo)?;
    }

    // Removal: paths in old manifest not in new plan get unlinked.
    if !args.dry_run {
        let new_paths: HashSet<&Path> =
            new_manifest.entries.iter().map(|e| e.path.as_path()).collect();
        for prev in &prev_manifest.entries {
            if !new_paths.contains(prev.path.as_path()) {
                let result = match prev.kind {
                    EntryKind::Symlink => install::uninstall_path(&prev.path),
                    EntryKind::Copy => install::uninstall_copy(&prev.path),
                };
                if let Err(e) = result {
                    ui::warn(format!(
                        "couldn't remove {}: {:#}",
                        paths::display_path(&prev.path),
                        e
                    ));
                }
            }
        }
        new_manifest.write(&repo)?;
    }

    for (alias, names) in &unregistered_aliases {
        ui::warn(format!(
            "unregistered project: {} (used by {})",
            alias,
            names.join(", ")
        ));
        ui::plain(format!("  run: ateam project add {} <path>", alias));
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

    let skill_word = |n: usize| if n == 1 { "skill" } else { "skills" };
    if args.dry_run {
        ui::ok(format!(
            "dry run: would apply {} {}",
            materialized,
            skill_word(materialized)
        ));
    } else {
        let suffix = if instructions_written > 0 {
            format!(" + {} instruction file(s)", instructions_written)
        } else {
            String::new()
        };
        ui::ok(format!(
            "applied {} {}{}",
            materialized,
            skill_word(materialized),
            suffix
        ));
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
            let snapshot = paths::local_skills_dir(repo).join(&entry.name);
            if snapshot.exists() {
                Ok(snapshot)
            } else {
                // Cold install — refetch from upstream into the synced snapshot.
                refetch_for_entry(repo, entry).context("fetching cold snapshot")?;
                Ok(paths::local_skills_dir(repo).join(&entry.name))
            }
        }
    }
}

fn refetch_for_entry(repo: &Path, entry: &SkillEntry) -> Result<()> {
    let source = Source::from_lockfile_string(&entry.source)?;

    // Registry-resolved entries (skills.sh blob fallback at add time) lack an
    // upstream subpath. Refetch them from skills.sh, not the upstream tree.
    if entry.path.is_none() {
        if let Source::Github { owner, repo: r } = &source {
            return refetch_via_registry(repo, owner, r, &entry.name);
        }
    }

    let path = entry
        .path
        .clone()
        .ok_or_else(|| anyhow!("lockfile entry `{}` missing `path`", entry.name))?;
    match source {
        Source::Github { owner, repo: r } => {
            let r_ref = entry.git_ref.clone().unwrap_or_else(|| github::default_branch_fallback().to_string());
            let commit_sha = github::resolve_ref(&owner, &r, &r_ref)?;
            // If the upstream subpath has moved/been removed, fall through to
            // skills.sh — the registry's blob endpoint often still serves a
            // snapshot. Heals lockfile entries left over from before the
            // skills/<name>/ snapshot migration.
            match refetch_github(repo, &owner, &r, &commit_sha, &path, &entry.name) {
                Ok(()) => Ok(()),
                Err(_) => refetch_via_registry(repo, &owner, &r, &entry.name),
            }
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

fn refetch_via_registry(repo: &Path, owner: &str, repo_name: &str, skill_name: &str) -> Result<()> {
    let slug = crate::source::skills_sh::to_slug(skill_name);
    let download = crate::source::skills_sh::fetch(owner, repo_name, &slug)?
        .ok_or_else(|| anyhow!("skills.sh has no entry for {}/{}/{}", owner, repo_name, slug))?;
    let slot = install::prepare_cache_slot(repo, skill_name)?;
    for file in &download.files {
        let dest = slot.tmp.join(&file.path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, &file.contents)?;
    }
    slot.commit()?;
    Ok(())
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
