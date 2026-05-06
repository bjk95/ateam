use crate::cli::AddArgs;
use crate::config::{MachineConfig, RepoConfig};
use crate::discover::{walk_package, DiscoveredSkill};
use crate::git_sync;
use crate::install;
use crate::lockfile::{Lockfile, SkillEntry};
use crate::paths;
use crate::source::{github, Source};
use crate::ui;
use anyhow::{anyhow, bail, Context, Result};
use console::style;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub fn run(args: AddArgs, no_sync: bool) -> Result<()> {
    let repo = paths::resolve_repo()?;
    let repo_cfg = RepoConfig::load(&repo)?;
    let machine_cfg = MachineConfig::load(&repo)?;

    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    let source = Source::parse(&args.source)?;

    ui::diamond(format!("Source: {}", args.source));

    // Fetch the package into a tmp working dir so we can discover its skills.
    let work_dir = tempdir(&repo)?;
    let package_root = fetch_package(&source, args.r#ref.as_deref(), &work_dir.path)?;
    ui::diamond("Repository cloned");
    let discovered = walk_package(&package_root)
        .with_context(|| format!("scanning package at {}", package_root.display()))?;
    ui::diamond(format!(
        "Found {} skill{}",
        discovered.len(),
        if discovered.len() == 1 { "" } else { "s" }
    ));

    if args.list {
        print_listing(&discovered);
        return Ok(());
    }

    ui::detail(format!("source: {}", source.lockfile_string()));

    let selection = pick_skills(&discovered, &args)?;
    if selection.is_empty() {
        ui::warn("no matching skills selected — pass --skill <name> or --all");
        return Ok(());
    }

    let install_root = resolve_install_root(&args, &machine_cfg)?;
    let agents = resolve_agents(&args, &repo_cfg);

    let mut lock = Lockfile::load(&repo)?;
    let mut installed: Vec<String> = Vec::new();
    let mut had_error = false;

    for skill in &selection {
        match install_one(
            &repo,
            &source,
            &args,
            skill,
            &package_root,
            &install_root,
            &agents,
        ) {
            Ok((entry, linked)) => {
                lock.upsert(entry);
                installed.push(skill.name.clone());
                lock.write(&repo)
                    .context("writing lockfile after upsert")?;
                ui::ok(format!("installed {}", skill.name));
                for link in &linked {
                    ui::detail(format!("linked {}", paths::display_path(link)));
                }
            }
            Err(e) => {
                had_error = true;
                ui::fail(format!("install {} — {:#}", skill.name, e));
            }
        }
    }

    if installed.is_empty() {
        if had_error {
            bail!("no skills installed (all failed)");
        }
        return Ok(());
    }

    if git_sync::enabled(no_sync) {
        let msg = git_sync::msg_add(&source.lockfile_string(), &installed);
        let _ = git_sync::commit_and_push(&repo, &msg);
    }

    Ok(())
}

fn fetch_package(source: &Source, git_ref: Option<&str>, dest: &Path) -> Result<PathBuf> {
    match source {
        Source::Github { owner, repo } => {
            let r = git_ref.unwrap_or(github::default_branch_fallback());
            github::fetch_tarball(owner, repo, r, dest)
        }
        Source::Git { url } => {
            let target = dest.join("git-source");
            crate::source::git::clone(url, git_ref, &target)?;
            Ok(target)
        }
        Source::Local { path } => {
            let abs = if path.is_absolute() {
                path.clone()
            } else {
                std::env::current_dir()?.join(path)
            };
            if !abs.exists() {
                bail!("local path {} does not exist", abs.display());
            }
            Ok(abs)
        }
    }
}

fn print_listing(skills: &[DiscoveredSkill]) {
    if skills.is_empty() {
        return;
    }
    ui::plain("");
    ui::diamond("Available Skills");
    ui::plain("");
    for s in skills {
        ui::plain(format!("   {}", style(&s.name).cyan()));
        if let Some(desc) = &s.description {
            ui::plain(format!("       {}", desc));
        }
        ui::plain("");
    }
}

fn pick_skills<'a>(
    discovered: &'a [DiscoveredSkill],
    args: &AddArgs,
) -> Result<Vec<&'a DiscoveredSkill>> {
    let want_all = args.all
        || args.skill.iter().any(|s| s == "*");
    if want_all {
        return Ok(discovered.iter().collect());
    }

    if args.skill.is_empty() {
        bail!("specify --skill <name> (repeatable), --all, or --list");
    }

    let mut wanted: HashSet<String> = HashSet::new();
    for raw in &args.skill {
        wanted.insert(crate::lockfile::normalize_skill_name(raw)?);
    }

    let mut out = Vec::new();
    for s in discovered {
        if wanted.contains(&s.name) {
            out.push(s);
        }
    }

    let found_names: HashSet<&str> = out.iter().map(|s| s.name.as_str()).collect();
    let missing: Vec<&String> = wanted.iter().filter(|w| !found_names.contains(w.as_str())).collect();
    if !missing.is_empty() {
        ui::warn(format!(
            "skills not found in source: {}",
            missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        ));
    }

    Ok(out)
}

fn resolve_install_root(args: &AddArgs, machine: &MachineConfig) -> Result<InstallRoot> {
    if let Some(alias) = &args.project {
        let path = machine
            .projects
            .get(alias)
            .ok_or_else(|| anyhow!("project alias `{}` not registered (run `ateam project register`)", alias))?;
        return Ok(InstallRoot::Project {
            alias: alias.clone(),
            path: path.clone(),
        });
    }
    if args.global {
        return Ok(InstallRoot::Global);
    }
    // Auto-detect: walk up from cwd, match any registered project's path.
    let cwd = std::env::current_dir().context("getting cwd")?;
    if let Some((alias, path)) = match_project_for_path(&cwd, machine) {
        return Ok(InstallRoot::Project { alias, path });
    }
    Ok(InstallRoot::Global)
}

fn match_project_for_path(start: &Path, machine: &MachineConfig) -> Option<(String, PathBuf)> {
    let canonical = std::fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
    for (alias, path) in &machine.projects {
        let canonical_p = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        if canonical.starts_with(&canonical_p) {
            return Some((alias.clone(), path.clone()));
        }
    }
    None
}

#[derive(Debug, Clone)]
enum InstallRoot {
    Global,
    Project { alias: String, path: PathBuf },
}

fn resolve_agents(args: &AddArgs, repo_cfg: &RepoConfig) -> Vec<String> {
    if args.agents.is_empty() || args.agents.iter().any(|a| a == "*") {
        repo_cfg.enabled_agents.clone()
    } else {
        args.agents.clone()
    }
}

fn install_one(
    repo: &Path,
    source: &Source,
    args: &AddArgs,
    skill: &DiscoveredSkill,
    package_root: &Path,
    install_root: &InstallRoot,
    agents: &[String],
) -> Result<(SkillEntry, Vec<PathBuf>)> {
    // Path of the skill's directory relative to the package root, used for
    // both lockfile recording and update-detection later.
    let rel_skill_dir = skill
        .dir
        .strip_prefix(package_root)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| skill.dir.clone());

    let canonical = match source {
        Source::Local { .. } => {
            // For local sources, point straight at the on-disk source dir.
            // Don't copy into the cache — symlinks resolve to the live source.
            skill.dir.clone()
        }
        Source::Github { .. } | Source::Git { .. } => {
            let slot = install::prepare_cache_slot(repo, &skill.name)?;
            install::copy_dir_recursive(&skill.dir, &slot.tmp)?;
            slot.commit()?
        }
    };
    ui::detail(format!("cached at {}", paths::display_path(&canonical)));

    let agent_list: Vec<String> = if args.agents.is_empty() || args.agents.iter().any(|a| a == "*") {
        vec!["*".into()]
    } else {
        args.agents.clone()
    };

    // For now, install to local-machine paths only. `apply` does the same
    // walk for everything in the lockfile; `add` runs apply-equivalent for
    // the new entry so the user sees skills available immediately.
    let install_root_path = match install_root {
        InstallRoot::Global => paths::home_dir()?,
        InstallRoot::Project { path, .. } => path.clone(),
    };

    let mut linked: Vec<PathBuf> = Vec::new();
    for agent in agents {
        let link = paths::agent_skill_path(&install_root_path, agent, &skill.name)?;
        match install::install_symlink(&link, &canonical, false)? {
            install::LinkOutcome::Refused => {
                ui::warn(format!(
                    "refused to install {} for {}: real dir at {} (rerun with `ateam apply --force`)",
                    skill.name,
                    agent,
                    paths::display_path(&link)
                ));
            }
            _ => {
                linked.push(link);
            }
        }
    }

    let tree_sha = match source {
        Source::Github { owner, repo: r } => {
            let git_ref = args
                .r#ref
                .clone()
                .unwrap_or_else(|| github::default_branch_fallback().to_string());
            let commit_sha = github::resolve_ref(owner, r, &git_ref).unwrap_or_else(|e| {
                tracing::warn!("could not resolve ref for {}/{}@{}: {}", owner, r, git_ref, e);
                String::new()
            });
            if commit_sha.is_empty() {
                None
            } else {
                let path_str = rel_skill_dir.to_string_lossy().into_owned();
                github::subtree_sha(owner, r, &commit_sha, &path_str).ok().flatten()
            }
        }
        Source::Git { url } => {
            let git_ref = args.r#ref.clone().unwrap_or_else(|| "HEAD".into());
            crate::source::git::ls_remote_sha(url, &git_ref).ok().flatten()
        }
        Source::Local { .. } => crate::source::local::content_hash(&canonical).ok(),
    };

    let entry_path = match source {
        Source::Local { path } => Some(path.to_string_lossy().into_owned()),
        _ => Some(rel_skill_dir.to_string_lossy().into_owned()),
    };

    let project = match install_root {
        InstallRoot::Project { alias, .. } => Some(alias.clone()),
        InstallRoot::Global => None,
    };

    Ok((
        SkillEntry {
            name: skill.name.clone(),
            source: source.lockfile_string(),
            path: entry_path,
            git_ref: args.r#ref.clone(),
            tree_sha,
            agents: agent_list,
            profiles: args.profile.clone(),
            project,
            active: true,
            upstream: None,
        },
        linked,
    ))
}

// ---------------------------------------------------------------------------
// tiny tempdir helper (avoid pulling tempfile crate)

struct TempDir {
    pub path: PathBuf,
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn tempdir(repo: &Path) -> Result<TempDir> {
    let root = paths::cache_tmp_dir(repo);
    std::fs::create_dir_all(&root)
        .with_context(|| format!("creating {}", root.display()))?;
    let suffix: u64 = rand::random();
    let p = root.join(format!("fetch-{:016x}", suffix));
    std::fs::create_dir_all(&p)
        .with_context(|| format!("creating {}", p.display()))?;
    Ok(TempDir { path: p })
}
