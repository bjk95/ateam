use crate::cli::ApplyArgs;
use crate::commands::apply_instructions;
use crate::config::{MachineConfig, RepoConfig};
use crate::git_sync;
use crate::install;
use crate::lockfile::{Lockfile, SkillEntry, SubagentEntry};
use crate::manifest::{EntryKind, Manifest};
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

    if !args.dry_run && git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    if !args.dry_run {
        install::sweep_tmp(&repo).ok();
    }

    let lock = Lockfile::load(&repo)?;
    let prev_manifest = Manifest::load(&repo)?;
    let mut new_manifest = Manifest::default();

    let mut unregistered_aliases: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut materialized = 0usize;
    let mut lockfile_dirty = false;

    let target_harnesses: Option<BTreeSet<String>> = if args.harnesses.is_empty() {
        None
    } else {
        Some(args.harnesses.iter().cloned().collect())
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

        let harnesses = resolve_harnesses(entry, &repo_cfg);
        let canonical = match resolve_canonical(&repo, entry, args.dry_run) {
            Ok(p) => p,
            Err(e) => {
                ui::fail(format!("resolve {} — {:#}", entry.name, e));
                continue;
            }
        };

        // Detect tree_sha drift for github sources during apply.
        if !args.dry_run {
            if let (Ok(Source::Github { owner, repo: r }), Some(_)) = (
                Source::from_lockfile_string(&entry.source),
                entry.tree_sha.as_ref(),
            ) {
                if let Some(path) = &entry.path {
                    let r_ref = entry
                        .git_ref
                        .clone()
                        .unwrap_or_else(|| github::default_branch(&owner, &r));
                    match github::resolve_ref(&owner, &r, &r_ref) {
                        Ok(commit_sha) => {
                            match github::subtree_sha(&owner, &r, &commit_sha, path) {
                                Ok(Some(latest)) => {
                                    if Some(&latest) != entry.tree_sha.as_ref() {
                                        let old = entry.tree_sha.clone().unwrap_or_default();
                                        ui::detail(format!(
                                            "drift detected for {}; refetching",
                                            entry.name
                                        ));
                                        if let Err(e) = refetch_github(
                                            &repo,
                                            &owner,
                                            &r,
                                            &commit_sha,
                                            path,
                                            &entry.name,
                                        ) {
                                            ui::fail(format!("refetch {} — {:#}", entry.name, e));
                                        } else {
                                            if let Some(pos) = updated_lock
                                                .skills
                                                .iter()
                                                .position(|s| s.name == entry.name)
                                            {
                                                updated_lock.skills[pos].tree_sha =
                                                    Some(latest.clone());
                                                lockfile_dirty = true;
                                                ui::detail(format!(
                                                    "{} tree_sha {} → {}",
                                                    entry.name,
                                                    short(&old),
                                                    short(&latest)
                                                ));
                                            }
                                        }
                                    }
                                }
                                Ok(None) => ui::detail(format!(
                                    "couldn't check drift for {}: path `{}` missing at {}",
                                    entry.name,
                                    path,
                                    short(&commit_sha)
                                )),
                                Err(e) => ui::detail(format!(
                                    "couldn't check drift for {}: {:#}",
                                    entry.name, e
                                )),
                            }
                        }
                        Err(e) => ui::detail(format!(
                            "couldn't resolve {}/{}@{} for {}: {:#}",
                            owner, r, r_ref, entry.name, e
                        )),
                    }
                }
            }
        }

        if args.dry_run {
            for harness in &harnesses {
                if let Some(filter) = &target_harnesses {
                    if !filter.contains(harness) {
                        continue;
                    }
                }
                let link = paths::harness_skill_path(&install_root, harness, &entry.name)?;
                ui::detail(format!(
                    "{} → {}",
                    paths::display_path(&link),
                    paths::display_path(&canonical)
                ));
                materialized += 1;
            }
            continue;
        }

        for harness in &harnesses {
            if let Some(filter) = &target_harnesses {
                if !filter.contains(harness) {
                    continue;
                }
            }
            let link = paths::harness_skill_path(&install_root, harness, &entry.name)?;
            match install::install_symlink(&link, &canonical, args.force)? {
                install::LinkOutcome::Created
                | install::LinkOutcome::Replaced
                | install::LinkOutcome::AlreadyCorrect
                | install::LinkOutcome::MovedAside
                | install::LinkOutcome::AutoHealed => {
                    new_manifest.entries.push(prev_manifest.tracked_entry(
                        link.clone(),
                        EntryKind::Symlink,
                        entry.name.clone(),
                        harness.clone(),
                        canonical.clone(),
                    ));
                    materialized += 1;
                }
                install::LinkOutcome::Refused => {
                    ui::warn(format!(
                        "refused to install {} for {}: real dir at {} (rerun with --force)",
                        entry.name,
                        harness,
                        paths::display_path(&link)
                    ));
                }
            }
        }
    }

    if let Some(s) = scan_step {
        s.finish();
    }

    // Subagent render-and-write pass — canonical files in `<repo>/agents/` get
    // rendered into each harness's native format (Claude/OpenCode/Gemini get
    // Markdown + YAML frontmatter; Codex gets a TOML file).
    let subagent_step = (!args.dry_run).then(|| ui::step("checking subagents"));
    for entry in &lock.subagents {
        if !entry.active {
            continue;
        }
        if !profile_match(&machine, &entry.profiles) {
            continue;
        }
        if let Some(s) = &subagent_step {
            s.set_msg(format!("checking {}", entry.name));
        }

        let install_root = match resolve_subagent_install_root(
            entry,
            &args,
            &mut machine,
            &mut unregistered_aliases,
        ) {
            Some(root) => root,
            None => continue,
        };

        let snapshot = paths::local_subagent_path(&repo, &entry.name);
        if !snapshot.exists() {
            ui::warn(format!(
                "subagent `{}` canonical missing at {}",
                entry.name,
                paths::display_path(&snapshot)
            ));
            continue;
        }
        let canonical = match crate::subagent::Subagent::load(&snapshot) {
            Ok(c) => c,
            Err(e) => {
                ui::fail(format!("parse {} — {:#}", entry.name, e));
                continue;
            }
        };

        let prev_paths: HashSet<&Path> = prev_manifest
            .entries
            .iter()
            .filter(|e| matches!(e.kind, EntryKind::Copy))
            .map(|e| e.path.as_path())
            .collect();

        let harnesses = resolve_subagent_harnesses(entry, &repo_cfg);
        for harness in &harnesses {
            if let Some(filter) = &target_harnesses {
                if !filter.contains(harness) {
                    continue;
                }
            }
            let Some(out_path) =
                crate::subagent::harness_install_path(&install_root, harness, &entry.name)?
            else {
                continue;
            };
            let Some(rendered) = crate::subagent::render_for_harness(&canonical, harness)? else {
                continue;
            };

            if args.dry_run {
                ui::detail(format!(
                    "{} ← {}",
                    paths::display_path(&out_path),
                    paths::display_path(&snapshot)
                ));
                materialized += 1;
                continue;
            }

            let was_managed = prev_paths.contains(out_path.as_path());
            match install::install_copy(&out_path, &rendered, was_managed, args.force)? {
                install::CopyOutcome::Written | install::CopyOutcome::MovedAside { .. } => {
                    new_manifest.entries.push(prev_manifest.tracked_entry(
                        out_path.clone(),
                        EntryKind::Copy,
                        entry.name.clone(),
                        harness.clone(),
                        snapshot.clone(),
                    ));
                    materialized += 1;
                }
                install::CopyOutcome::Refused => {
                    ui::warn(format!(
                        "refused to install subagent {} for {}: real file at {} (rerun with --force)",
                        entry.name,
                        harness,
                        paths::display_path(&out_path)
                    ));
                }
            }
        }
    }
    if let Some(s) = subagent_step {
        s.finish();
    }

    let home = paths::home_dir()?;
    let instructions_step =
        (!args.dry_run && args.project.is_none()).then(|| ui::step("checking instructions"));
    let instructions_outcome = if args.project.is_none() {
        match apply_instructions::apply(
            &repo,
            &home,
            &repo_cfg,
            &mut updated_lock,
            &mut machine,
            &prev_manifest,
            &mut new_manifest,
            target_harnesses.as_ref(),
            args.dry_run,
            args.force,
        ) {
            Ok(outcome) => {
                if let Some(step) = instructions_step {
                    step.finish();
                }
                outcome
            }
            Err(e) => {
                if let Some(step) = instructions_step {
                    step.fail("instructions failed");
                }
                return Err(e);
            }
        }
    } else {
        apply_instructions::ApplyOutcome::default()
    };
    let instructions_written = instructions_outcome.written;
    if instructions_outcome.lockfile_dirty {
        lockfile_dirty = true;
    }
    if instructions_outcome.instructions_skip_set && !args.dry_run {
        machine.write(&repo)?;
    }

    // Removal: paths in old manifest not in new plan get unlinked.
    if !args.dry_run {
        preserve_out_of_scope_manifest_entries(
            &prev_manifest,
            &mut new_manifest,
            target_harnesses.as_ref(),
            args.project.as_deref(),
            args.project
                .as_deref()
                .and_then(|alias| machine.projects.get(alias).map(PathBuf::as_path)),
            &updated_lock,
        );
        let new_paths: HashSet<&Path> = new_manifest
            .entries
            .iter()
            .map(|e| e.path.as_path())
            .collect();
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
        ui::plain(format!("  run: agents project add {} <path>", alias));
    }

    if lockfile_dirty && !args.dry_run {
        updated_lock.write(&repo)?;
    }

    if !args.dry_run && git_sync::enabled(no_sync) && lockfile_dirty {
        let msg = git_sync::msg_apply(materialized);
        if let Err(e) = git_sync::commit_and_push(&repo, &msg) {
            ui::warn(format!("auto-sync failed: {:#}", e));
            ui::detail("local change saved; rerun a mutating command to retry");
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

fn preserve_out_of_scope_manifest_entries(
    prev_manifest: &Manifest,
    new_manifest: &mut Manifest,
    target_harnesses: Option<&BTreeSet<String>>,
    target_project: Option<&str>,
    target_project_root: Option<&Path>,
    lock: &Lockfile,
) {
    for prev in &prev_manifest.entries {
        if !is_out_of_scope(
            prev,
            target_harnesses,
            target_project,
            target_project_root,
            lock,
        ) {
            continue;
        }
        if new_manifest
            .entries
            .iter()
            .any(|entry| entry.path == prev.path)
        {
            continue;
        }
        new_manifest.entries.push(prev.clone());
    }
}

fn is_out_of_scope(
    entry: &crate::manifest::ManifestEntry,
    target_harnesses: Option<&BTreeSet<String>>,
    target_project: Option<&str>,
    target_project_root: Option<&Path>,
    lock: &Lockfile,
) -> bool {
    if let Some(filter) = target_harnesses {
        if !filter.contains(&entry.harness) {
            return true;
        }
    }
    if let Some(project) = target_project {
        return !manifest_entry_matches_project(entry, lock, project, target_project_root);
    }
    false
}

fn manifest_entry_matches_project(
    entry: &crate::manifest::ManifestEntry,
    lock: &Lockfile,
    project: &str,
    project_root: Option<&Path>,
) -> bool {
    if let Some(entry_project) = manifest_entry_project(entry, lock) {
        return entry_project == project;
    }
    project_root.is_some_and(|root| entry.path.starts_with(root))
}

fn manifest_entry_project<'a>(
    entry: &crate::manifest::ManifestEntry,
    lock: &'a Lockfile,
) -> Option<&'a str> {
    lock.find(&entry.skill)
        .and_then(|skill| skill.project.as_deref())
        .or_else(|| {
            lock.find_subagent(&entry.skill)
                .and_then(|subagent| subagent.project.as_deref())
        })
}

fn profile_match(machine: &MachineConfig, gates: &[String]) -> bool {
    if gates.is_empty() {
        return true;
    }
    gates
        .iter()
        .any(|g| machine.profiles.iter().any(|p| p == g))
}

fn resolve_harnesses(entry: &SkillEntry, repo_cfg: &RepoConfig) -> Vec<String> {
    if entry.harnesses.iter().any(|a| a == "*") {
        repo_cfg.enabled_harnesses.clone()
    } else {
        entry.harnesses.clone()
    }
}

fn resolve_subagent_harnesses(entry: &SubagentEntry, repo_cfg: &RepoConfig) -> Vec<String> {
    let candidates: Vec<String> = if entry.harnesses.iter().any(|a| a == "*") {
        repo_cfg.enabled_harnesses.clone()
    } else {
        entry.harnesses.clone()
    };
    // Filter to harnesses that actually support subagents (claude-code, codex
    // today). Skipped harnesses are silently no-op'd rather than warned about.
    candidates
        .into_iter()
        .filter(|id| {
            crate::harness::lookup(id)
                .and_then(|d| d.subagents_subdir)
                .is_some()
        })
        .collect()
}

fn resolve_subagent_install_root(
    entry: &SubagentEntry,
    args: &ApplyArgs,
    machine: &mut MachineConfig,
    unregistered: &mut BTreeMap<String, Vec<String>>,
) -> Option<PathBuf> {
    match &entry.project {
        Some(alias) => {
            if let Some(filter) = &args.project {
                if filter != alias {
                    return None;
                }
            }
            match machine.projects.get(alias) {
                Some(path) if path.exists() => Some(path.clone()),
                Some(path) => {
                    ui::warn(format!(
                        "project `{}` path does not exist: {}",
                        alias,
                        paths::display_path(path)
                    ));
                    None
                }
                None => {
                    unregistered
                        .entry(alias.clone())
                        .or_default()
                        .push(entry.name.clone());
                    None
                }
            }
        }
        None => {
            if args.project.is_some() {
                return None;
            }
            paths::home_dir().ok()
        }
    }
}

fn resolve_canonical(repo: &Path, entry: &SkillEntry, dry_run: bool) -> Result<PathBuf> {
    let source = Source::from_lockfile_string(&entry.source)?;
    match source {
        Source::Local { path } => crate::source::local::resolve(repo, &path),
        Source::Github { .. } | Source::Git { .. } => {
            let snapshot = paths::local_skills_dir(repo).join(&entry.name);
            if snapshot.exists() || dry_run {
                Ok(snapshot)
            } else {
                ui::detail(format!("snapshot missing for {}; refetching", entry.name));
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
            let r_ref = entry
                .git_ref
                .clone()
                .unwrap_or_else(|| github::default_branch(&owner, &r));
            let commit_sha = github::resolve_ref(&owner, &r, &r_ref)?;
            // If the upstream subpath has moved/been removed, fall through to
            // skills.sh — the registry's blob endpoint often still serves a
            // snapshot. Heals lockfile entries left over from before the
            // skills/<name>/ snapshot migration.
            match refetch_github(repo, &owner, &r, &commit_sha, &path, &entry.name) {
                Ok(()) => Ok(()),
                Err(e) => {
                    ui::detail(format!(
                        "github refetch failed for {}; trying registry: {:#}",
                        entry.name, e
                    ));
                    refetch_via_registry(repo, &owner, &r, &entry.name)
                }
            }
        }
        Source::Git { url } => {
            let tmp_root = paths::tmp_dir(repo);
            std::fs::create_dir_all(&tmp_root)?;
            let suffix: u64 = rand::random();
            let work = tmp_root.join(format!("git-{:016x}", suffix));
            crate::source::git::clone(&url, entry.git_ref.as_deref(), &work)?;
            let src_dir = work.join(&path);
            let slot = install::prepare_cache_slot(repo, &entry.name)?;
            install::copy_dir_recursive(&src_dir, &slot.tmp)?;
            canonicalize_snapshot(&entry.name, &slot.tmp)?;
            slot.commit()?;
            let _ = std::fs::remove_dir_all(&work);
            Ok(())
        }
        Source::Local { .. } => Ok(()),
    }
}

fn short(sha: &str) -> String {
    sha.chars().take(7).collect()
}

fn refetch_via_registry(repo: &Path, owner: &str, repo_name: &str, skill_name: &str) -> Result<()> {
    let slug = crate::source::skills_sh::to_slug(skill_name);
    let download = crate::source::skills_sh::fetch(owner, repo_name, &slug)?.ok_or_else(|| {
        anyhow!(
            "skills.sh has no entry for {}/{}/{}",
            owner,
            repo_name,
            slug
        )
    })?;
    let slot = install::prepare_cache_slot(repo, skill_name)?;
    for file in &download.files {
        let dest = slot.tmp.join(file.relative_path()?);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, &file.contents)?;
    }
    canonicalize_snapshot(skill_name, &slot.tmp)?;
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
    let tmp_root = paths::tmp_dir(repo);
    std::fs::create_dir_all(&tmp_root)?;
    let suffix: u64 = rand::random();
    let work = tmp_root.join(format!("fetch-{:016x}", suffix));
    std::fs::create_dir_all(&work)?;
    let pkg_root = github::fetch_tarball(owner, repo_name, commit_sha, &work)?;
    let src_dir = pkg_root.join(sub_path);
    if !src_dir.exists() {
        bail!(
            "path `{}` not found in {}/{}@{}",
            sub_path,
            owner,
            repo_name,
            commit_sha
        );
    }
    let slot = install::prepare_cache_slot(repo, skill_name)?;
    install::copy_dir_recursive(&src_dir, &slot.tmp)?;
    canonicalize_snapshot(skill_name, &slot.tmp)?;
    slot.commit()?;
    let _ = std::fs::remove_dir_all(&work);
    Ok(())
}

fn canonicalize_snapshot(skill_name: &str, dir: &Path) -> Result<()> {
    if let Some(repair) = crate::discover::canonicalize_skill_dir(dir, skill_name)? {
        for diagnostic in repair.diagnostics {
            ui::warn(format!("repaired {}: {}", skill_name, diagnostic));
        }
    }
    Ok(())
}
