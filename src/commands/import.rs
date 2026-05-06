use crate::cli::ImportArgs;
use crate::config::RepoConfig;
use crate::git_sync;
use crate::lockfile::{Lockfile, SkillEntry};
use crate::paths;
use crate::source::Source;
use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};

pub fn run(args: ImportArgs, no_sync: bool) -> Result<()> {
    let repo = paths::resolve_repo()?;
    let _repo_cfg = RepoConfig::load(&repo)?;

    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    let normalized = crate::lockfile::normalize_skill_name(&args.name)?;

    // Hunt across known agent dirs in $HOME for a directory matching the name.
    let mut found: Option<PathBuf> = None;
    let home = paths::home_dir()?;
    for agent_dir in [
        home.join(".claude").join("skills"),
        home.join(".codex").join("skills"),
        home.join(".agents").join("skills"),
    ] {
        let candidate = agent_dir.join(&normalized);
        if candidate.exists() {
            found = Some(candidate);
            break;
        }
    }
    let installed = found.ok_or_else(|| {
        anyhow!(
            "no installed skill found named `{}` in ~/.claude/skills/, ~/.codex/skills/, or ~/.agents/skills/",
            normalized
        )
    })?;

    // If it's a symlink into our own cache, no-op.
    if let Ok(meta) = std::fs::symlink_metadata(&installed) {
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(&installed)?;
            if target.starts_with(paths::cache_dir(&repo)) || target.starts_with(paths::local_skills_dir(&repo)) {
                println!("`{}` already managed by ateam", normalized);
                return Ok(());
            }
        }
    }

    let mut lock = Lockfile::load(&repo)?;
    let entry = build_entry(&repo, &normalized, &installed, &args)?;
    let replaced = lock.upsert(entry);
    lock.write(&repo)?;

    if git_sync::enabled(no_sync) {
        let last = lock
            .find(&normalized)
            .map(|e| e.source.clone())
            .unwrap_or_else(|| "unknown".into());
        let msg = git_sync::msg_import(&normalized, &last);
        let _ = git_sync::commit_and_push(&repo, &msg);
    }

    println!(
        "ateam: {} `{}` (re-run `ateam apply` to materialize symlinks)",
        if replaced { "updated" } else { "imported" },
        normalized
    );
    Ok(())
}

fn build_entry(
    repo: &Path,
    name: &str,
    installed: &Path,
    args: &ImportArgs,
) -> Result<SkillEntry> {
    if let Some(upstream) = &args.upstream {
        let source = Source::parse(upstream)?;
        return Ok(SkillEntry {
            name: name.to_string(),
            source: source.lockfile_string(),
            path: None,
            git_ref: None,
            tree_sha: None,
            agents: vec!["*".into()],
            profiles: vec![],
            project: args.project.clone(),
        });
    }

    // Snapshot: copy the installed dir into <repo>/skills/<name>/ as a local source.
    let dest = paths::local_skills_dir(repo).join(name);
    if dest.exists() {
        bail!("local source already exists at {}", dest.display());
    }
    std::fs::create_dir_all(paths::local_skills_dir(repo))
        .with_context(|| format!("creating {}", paths::local_skills_dir(repo).display()))?;
    crate::install::copy_dir_recursive(installed, &dest)?;
    Ok(SkillEntry {
        name: name.to_string(),
        source: format!("local:skills/{}", name),
        path: Some(format!("skills/{}", name)),
        git_ref: None,
        tree_sha: None,
        agents: vec!["*".into()],
        profiles: vec![],
        project: args.project.clone(),
    })
}
