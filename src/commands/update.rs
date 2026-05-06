use crate::cli::UpdateArgs;
use crate::git_sync;
use crate::install;
use crate::lockfile::Lockfile;
use crate::paths;
use crate::source::{github, Source};
use crate::ui;
use anyhow::{Context, Result};
use std::path::Path;

pub fn run(args: UpdateArgs, no_sync: bool) -> Result<()> {
    let repo = paths::resolve_repo()?;

    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    let mut lock = Lockfile::load(&repo)?;
    let names: Vec<String> = if args.names.is_empty() {
        lock.skills
            .iter()
            .filter(|s| s.active)
            .map(|s| s.name.clone())
            .collect()
    } else {
        args.names.clone()
    };

    let mut changed: Vec<(String, String, String)> = Vec::new(); // (name, old, new)

    let n = names.len();
    {
        let _step = ui::step(format!(
            "checking {} {}",
            n,
            if n == 1 { "skill" } else { "skills" }
        ));
        for name in &names {
            let entry_idx = match lock.skills.iter().position(|s| &s.name == name) {
                Some(i) => i,
                None => {
                    ui::warn(format!("`{}` not in lockfile", name));
                    continue;
                }
            };

            if !lock.skills[entry_idx].active {
                ui::warn(format!("skipping `{}` (deactivated)", name));
                continue;
            }

            let entry = lock.skills[entry_idx].clone();
            let source = match Source::from_lockfile_string(&entry.source) {
                Ok(s) => s,
                Err(e) => {
                    ui::warn(format!("bad source for `{}`: {:#}", name, e));
                    continue;
                }
            };

            match check_and_refetch(&repo, &source, &entry) {
                Ok(Some(new_sha)) => {
                    let old = entry.tree_sha.clone().unwrap_or_default();
                    lock.skills[entry_idx].tree_sha = Some(new_sha.clone());
                    changed.push((name.clone(), old, new_sha));
                }
                Ok(None) => {
                    tracing::debug!("{} up to date", name);
                }
                Err(e) => {
                    ui::warn(format!("couldn't check `{}`: {:#}", name, e));
                }
            }
        }
    }

    if changed.is_empty() {
        ui::ok("all skills up to date");
        return Ok(());
    }

    lock.write(&repo).context("writing updated lockfile")?;
    for (name, old, new) in &changed {
        ui::ok(format!("updated {}", name));
        ui::detail(format!("{} → {}", short(old), short(new)));
    }

    if git_sync::enabled(no_sync) {
        let msg = if changed.len() == 1 {
            git_sync::msg_update_one(&changed[0].0, &changed[0].1, &changed[0].2)
        } else {
            git_sync::msg_update_bulk(changed.len())
        };
        if let Err(e) = git_sync::commit_and_push(&repo, &msg) {
            ui::warn(format!("auto-sync failed: {:#}", e));
            ui::detail("local change saved; rerun a mutating command to retry");
        }
    }

    Ok(())
}

fn check_and_refetch(
    repo: &Path,
    source: &Source,
    entry: &crate::lockfile::SkillEntry,
) -> Result<Option<String>> {
    // Registry-resolved entries (path is None, source is github): refresh by
    // re-hitting skills.sh's blob endpoint and comparing hashes.
    if entry.path.is_none() {
        if let Source::Github { owner, repo: r } = source {
            let slug = crate::source::skills_sh::to_slug(&entry.name);
            let download = match crate::source::skills_sh::fetch(owner, r, &slug)? {
                Some(d) => d,
                None => return Ok(None),
            };
            let latest = match download.hash.clone() {
                Some(h) => h,
                None => return Ok(None),
            };
            if Some(&latest) == entry.tree_sha.as_ref() {
                return Ok(None);
            }
            let slot = install::prepare_cache_slot(repo, &entry.name)?;
            for file in &download.files {
                let dest = slot.tmp.join(&file.path);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&dest, &file.contents)?;
            }
            slot.commit()?;
            return Ok(Some(latest));
        }
        return Ok(None);
    }

    let path = match &entry.path {
        Some(p) => p.clone(),
        None => return Ok(None),
    };
    match source {
        Source::Github { owner, repo: r } => {
            let r_ref = entry
                .git_ref
                .clone()
                .unwrap_or_else(|| github::default_branch_fallback().to_string());
            let commit_sha = github::resolve_ref(owner, r, &r_ref)?;
            let latest = match github::subtree_sha(owner, r, &commit_sha, &path)? {
                Some(s) => s,
                None => return Ok(None),
            };
            if Some(&latest) == entry.tree_sha.as_ref() {
                return Ok(None);
            }
            // Refetch.
            refetch_github(repo, owner, r, &commit_sha, &path, &entry.name)?;
            Ok(Some(latest))
        }
        Source::Git { url } => {
            let r_ref = entry.git_ref.clone().unwrap_or_else(|| "HEAD".into());
            let latest = match crate::source::git::ls_remote_sha(url, &r_ref)? {
                Some(s) => s,
                None => return Ok(None),
            };
            if Some(&latest) == entry.tree_sha.as_ref() {
                return Ok(None);
            }
            // Refetch via fresh shallow clone.
            let tmp_root = paths::cache_tmp_dir(repo);
            std::fs::create_dir_all(&tmp_root)?;
            let suffix: u64 = rand::random();
            let work = tmp_root.join(format!("git-{:016x}", suffix));
            crate::source::git::clone(url, entry.git_ref.as_deref(), &work)?;
            let src_dir = work.join(&path);
            let slot = install::prepare_cache_slot(repo, &entry.name)?;
            install::copy_dir_recursive(&src_dir, &slot.tmp)?;
            slot.commit()?;
            let _ = std::fs::remove_dir_all(&work);
            Ok(Some(latest))
        }
        Source::Local { path: p } => {
            let abs = crate::source::local::resolve(repo, p)?;
            let latest = crate::source::local::content_hash(&abs)?;
            if Some(&latest) == entry.tree_sha.as_ref() {
                Ok(None)
            } else {
                Ok(Some(latest))
            }
        }
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
    let slot = install::prepare_cache_slot(repo, skill_name)?;
    install::copy_dir_recursive(&src_dir, &slot.tmp)?;
    slot.commit()?;
    let _ = std::fs::remove_dir_all(&work);
    Ok(())
}

fn short(s: &str) -> String {
    s.chars().take(7).collect()
}
