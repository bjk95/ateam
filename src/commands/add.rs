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

pub fn run(mut args: AddArgs, no_sync: bool) -> Result<()> {
    normalize_all_flag(&mut args);
    let repo = paths::resolve_repo()?;
    let repo_cfg = RepoConfig::load(&repo)?;
    let mut machine_cfg = MachineConfig::load(&repo)?;

    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    let source = Source::parse_with(&args.source, args.dangerously_accept_openclaw_risks)?;

    ui::diamond(format!("Source: {}", args.source));

    // Fetch the package into a tmp working dir so we can discover its skills.
    let work_dir = tempdir(&repo)?;
    let package_root = fetch_package(&source, args.r#ref.as_deref(), &work_dir.path)?;
    ui::diamond("Repository cloned");
    let mut discovered = walk_package(&package_root)
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

    // Registry fallback: for any --skill <name> not in the cloned tree, consult
    // skills.sh's blob endpoint. Covers skills that have been renamed or moved
    // upstream but are still served from the registry's snapshot cache.
    resolve_via_registry(&source, &args, &package_root, &mut discovered);

    ui::detail(format!("source: {}", source.lockfile_string()));

    let selection = pick_skills(&discovered, &args)?;
    if selection.is_empty() {
        ui::warn("no matching skills selected — pass --skill <name> or --all");
        return Ok(());
    }

    let install_root = resolve_install_root(&args, &mut machine_cfg, &repo)?;
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
        if let Err(e) = git_sync::commit_and_push(&repo, &msg) {
            ui::warn(format!("auto-sync failed: {:#}", e));
            ui::detail("local change saved; rerun a mutating command to retry");
        }
    }

    Ok(())
}

fn fetch_package(source: &Source, git_ref: Option<&str>, dest: &Path) -> Result<PathBuf> {
    match source {
        Source::Github { owner, repo } => {
            let resolved;
            let r = match git_ref {
                Some(r) => r,
                None => {
                    resolved = github::default_branch(owner, repo);
                    resolved.as_str()
                }
            };
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

// Vercel parity: --all is a triple-flag override (skill='*', agent='*', -y).
// Without this, drop-in scripts that pass only `--all` would still hit the
// per-agent prompt and the non-TTY confirmation guard.
fn normalize_all_flag(args: &mut AddArgs) {
    if !args.all {
        return;
    }
    if args.harnesses.is_empty() {
        args.harnesses = vec!["*".into()];
    }
    args.yes = true;
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
        let normalized = crate::lockfile::normalize_skill_name(raw)?;
        wanted.insert(crate::discover::standard_skill_name(&normalized));
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
        let available: Vec<&str> = discovered.iter().map(|s| s.name.as_str()).collect();
        for name in &missing {
            let suggestions = closest_matches(name, &available, 3);
            if suggestions.is_empty() {
                ui::warn(format!("skill `{}` not found in source", name));
            } else {
                ui::warn(format!(
                    "skill `{}` not found in source — did you mean: {}?",
                    name,
                    suggestions.join(", ")
                ));
            }
        }
        ui::detail("run with `--list` to see all available skills");
    }

    if out.is_empty() {
        bail!("no skills installed — none of the requested --skill names matched");
    }

    Ok(out)
}

/// For every `--skill <name>` not satisfied by the cloned tree, fall back to
/// skills.sh's blob-download endpoint. On a hit, materialize the snapshot
/// files into the package_root and append a synthetic `DiscoveredSkill` so the
/// existing install pipeline picks it up unchanged.
fn resolve_via_registry(
    source: &Source,
    args: &AddArgs,
    package_root: &Path,
    discovered: &mut Vec<DiscoveredSkill>,
) {
    let (owner, repo) = match source {
        Source::Github { owner, repo } => (owner.clone(), repo.clone()),
        _ => return, // registry only knows GitHub-hosted skills
    };

    let want_all = args.all || args.skill.iter().any(|s| s == "*");
    if want_all || args.skill.is_empty() {
        return;
    }

    let mut found_names: HashSet<String> = discovered.iter().map(|s| s.name.clone()).collect();

    for raw in &args.skill {
        let normalized = match crate::lockfile::normalize_skill_name(raw) {
            Ok(n) => crate::discover::standard_skill_name(&n),
            Err(_) => continue,
        };
        if found_names.contains(&normalized) {
            continue;
        }

        let slug = crate::source::skills_sh::to_slug(&normalized);
        let download = match crate::source::skills_sh::fetch(&owner, &repo, &slug) {
            Ok(Some(d)) => d,
            Ok(None) => continue,
            Err(e) => {
                ui::warn(format!("registry lookup failed for `{}`: {:#}", normalized, e));
                continue;
            }
        };

        let skill_dir = package_root.join(&normalized);
        if let Err(e) = std::fs::create_dir_all(&skill_dir) {
            ui::warn(format!("registry write failed for `{}`: {:#}", normalized, e));
            continue;
        }
        let mut wrote_skill_md = false;
        let mut write_err = None;
        for file in &download.files {
            let dest = skill_dir.join(&file.path);
            if let Some(parent) = dest.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    write_err = Some(e);
                    break;
                }
            }
            if let Err(e) = std::fs::write(&dest, &file.contents) {
                write_err = Some(e);
                break;
            }
            if file.path == "SKILL.md" {
                wrote_skill_md = true;
            }
        }
        if let Some(e) = write_err {
            ui::warn(format!("registry write failed for `{}`: {:#}", normalized, e));
            continue;
        }
        if !wrote_skill_md {
            ui::warn(format!(
                "registry response for `{}` had no SKILL.md — skipping",
                normalized
            ));
            continue;
        }

        match crate::discover::walk_package(&skill_dir) {
            Ok(mut new_skills) if !new_skills.is_empty() => {
                ui::diamond(format!("Resolved `{}` via skills.sh", normalized));
                let mut added = new_skills.remove(0);
                added.source_hash = download.hash.clone();
                found_names.insert(added.name.clone());
                discovered.push(added);
            }
            _ => {
                ui::warn(format!(
                    "registry SKILL.md for `{}` failed to parse — skipping",
                    normalized
                ));
            }
        }
    }
}

fn closest_matches<'a>(wanted: &str, available: &[&'a str], n: usize) -> Vec<&'a str> {
    let mut scored: Vec<(usize, &'a str)> = available
        .iter()
        .map(|s| (levenshtein(wanted, s), *s))
        .collect();
    scored.sort_by_key(|(d, _)| *d);
    scored
        .into_iter()
        // Edit distance must fit within roughly half the longer of the two
        // names — keeps noise out for things like "azure-observability"
        // when no skill is actually close.
        .filter(|(d, name)| *d <= (wanted.len().max(name.len()) / 2).max(2))
        .take(n)
        .map(|(_, s)| s)
        .collect()
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let n = a.len();
    let m = b.len();
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr = vec![0; m + 1];
    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

fn resolve_install_root(
    args: &AddArgs,
    machine: &mut MachineConfig,
    repo: &Path,
) -> Result<InstallRoot> {
    if let Some(alias) = &args.project {
        let path = machine
            .projects
            .get(alias)
            .ok_or_else(|| anyhow!("project alias `{}` not registered (run `agents project register`)", alias))?;
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
    // Unregistered git repo: offer to auto-register the repo dir as a project
    // so users get project-scoped installs without a separate `project add` step.
    if let Some(git_root) = git_toplevel(&cwd) {
        let alias = git_root
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "project".to_string());
        if let Some(true) = prompt_install_scope(&alias, &git_root, args.yes)? {
            machine.projects.insert(alias.clone(), git_root.clone());
            machine.write(repo)?;
            ui::ok(format!(
                "registered project {} → {}",
                alias,
                paths::display_path(&git_root)
            ));
            return Ok(InstallRoot::Project {
                alias,
                path: git_root,
            });
        }
    }
    Ok(InstallRoot::Global)
}

fn git_toplevel(cwd: &Path) -> Option<PathBuf> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let trimmed = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

/// Some(true) = project, Some(false) = global, None = treat as global (non-TTY without -y).
fn prompt_install_scope(alias: &str, git_root: &Path, assume_yes: bool) -> Result<Option<bool>> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        return Ok(if assume_yes { Some(true) } else { None });
    }
    use dialoguer::{theme::ColorfulTheme, Select};
    let project_label = format!(
        "Project (auto-register {} → {})",
        alias,
        paths::display_path(git_root)
    );
    let choice = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Install scope")
        .items(&[project_label.as_str(), "Global (~)"])
        .default(0)
        .interact()?;
    Ok(Some(choice == 0))
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
    if args.harnesses.is_empty() || args.harnesses.iter().any(|a| a == "*") {
        repo_cfg.enabled_harnesses.clone()
    } else {
        args.harnesses.clone()
    }
}

fn install_one(
    repo: &Path,
    source: &Source,
    args: &AddArgs,
    skill: &DiscoveredSkill,
    package_root: &Path,
    install_root: &InstallRoot,
    harnesses: &[String],
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
            // For local sources whose path doesn't already point under
            // skills/<name>/, leave the symlink target at the live source dir.
            // For local:skills/<name>/ (the canonical author-in-repo case),
            // skill.dir already IS skills/<name>/.
            skill.dir.clone()
        }
        Source::Github { .. } | Source::Git { .. } => {
            // Snapshot into <repo>/skills/<name>/ so the content travels with
            // the agents-config repo via git instead of being refetched on
            // every machine.
            let slot = install::prepare_cache_slot(repo, &skill.name)?;
            install::copy_dir_recursive(&skill.dir, &slot.tmp)?;
            if let Some(repair) =
                crate::discover::canonicalize_skill_dir(&slot.tmp, &skill.name)?
            {
                for diagnostic in repair.diagnostics {
                    ui::warn(format!("repaired {}: {}", skill.name, diagnostic));
                }
            }
            slot.commit()?
        }
    };
    ui::detail(format!("snapshotted to {}", paths::display_path(&canonical)));

    let harness_list: Vec<String> = if args.harnesses.is_empty() || args.harnesses.iter().any(|a| a == "*") {
        vec!["*".into()]
    } else {
        args.harnesses.clone()
    };

    // For now, install to local-machine paths only. `apply` does the same
    // walk for everything in the lockfile; `add` runs apply-equivalent for
    // the new entry so the user sees skills available immediately.
    let install_root_path = match install_root {
        InstallRoot::Global => paths::home_dir()?,
        InstallRoot::Project { path, .. } => path.clone(),
    };

    let mut linked: Vec<PathBuf> = Vec::new();
    for harness in harnesses {
        let link = paths::harness_skill_path(&install_root_path, harness, &skill.name)?;
        if args.copy {
            match install::install_copy_dir(&link, &canonical, false, false)? {
                install::CopyDirOutcome::Refused => {
                    ui::warn(format!(
                        "refused to install {} for {}: real dir at {} (rerun with `agents apply --copy --force`)",
                        skill.name,
                        harness,
                        paths::display_path(&link)
                    ));
                }
                _ => {
                    linked.push(link);
                }
            }
            continue;
        }
        match install::install_symlink(&link, &canonical, false)? {
            install::LinkOutcome::Refused => {
                ui::warn(format!(
                    "refused to install {} for {}: real dir at {} (rerun with `agents apply --force`)",
                    skill.name,
                    harness,
                    paths::display_path(&link)
                ));
            }
            _ => {
                linked.push(link);
            }
        }
    }

    // Always pin a version. Order: registry-provided hash (skills.sh blob),
    // upstream-provided sha (github tree / git ls-remote), content hash of the
    // local snapshot. Falling all the way back to content_hash guarantees every
    // entry has a `tree_sha` field, so consumers can compare without nullchecks.
    let tree_sha = skill
        .source_hash
        .clone()
        .or_else(|| match source {
            Source::Github { owner, repo: r } => {
                let git_ref = args
                    .r#ref
                    .clone()
                    .unwrap_or_else(|| github::default_branch(owner, r));
                let commit_sha = github::resolve_ref(owner, r, &git_ref).unwrap_or_else(|e| {
                    tracing::warn!("could not resolve ref for {}/{}@{}: {}", owner, r, git_ref, e);
                    String::new()
                });
                if commit_sha.is_empty() {
                    None
                } else {
                    let path_str = rel_skill_dir.to_string_lossy().into_owned();
                    github::subtree_sha(owner, r, &commit_sha, &path_str)
                        .ok()
                        .flatten()
                }
            }
            Source::Git { url } => {
                let git_ref = args.r#ref.clone().unwrap_or_else(|| "HEAD".into());
                crate::source::git::ls_remote_sha(url, &git_ref).ok().flatten()
            }
            Source::Local { .. } => None,
        })
        .or_else(|| crate::source::local::content_hash(&canonical).ok());

    let entry_path = match source {
        Source::Local { path } => Some(path.to_string_lossy().into_owned()),
        _ if skill.source_hash.is_some() => {
            // Registry-resolved skills (skills.sh blob): no upstream subpath.
            // The snapshot is canonical; we don't pretend to know where in the
            // upstream tree it lives.
            None
        }
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
            harnesses: harness_list,
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
    let root = paths::tmp_dir(repo);
    std::fs::create_dir_all(&root)
        .with_context(|| format!("creating {}", root.display()))?;
    let suffix: u64 = rand::random();
    let p = root.join(format!("fetch-{:016x}", suffix));
    std::fs::create_dir_all(&p)
        .with_context(|| format!("creating {}", p.display()))?;
    Ok(TempDir { path: p })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(name: &str) -> DiscoveredSkill {
        DiscoveredSkill {
            name: name.into(),
            description: None,
            dir: PathBuf::new(),
            source_hash: None,
        }
    }

    fn args_with_skills(names: &[&str]) -> AddArgs {
        AddArgs {
            source: "test".into(),
            list: false,
            skill: names.iter().map(|s| (*s).into()).collect(),
            all: false,
            harnesses: vec![],
            yes: false,
            global: false,
            profile: vec![],
            project: None,
            r#ref: None,
            copy: false,
            dangerously_accept_openclaw_risks: false,
        }
    }

    #[test]
    fn pick_skills_bails_when_no_named_skill_matches() {
        let discovered = vec![skill("foo"), skill("bar")];
        let args = args_with_skills(&["typo"]);
        let err = pick_skills(&discovered, &args).unwrap_err();
        assert!(
            err.to_string().contains("no skills installed"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn pick_skills_returns_partial_when_some_match() {
        let discovered = vec![skill("foo"), skill("bar")];
        let args = args_with_skills(&["foo", "typo"]);
        let out = pick_skills(&discovered, &args).expect("partial match should succeed");
        let names: Vec<&str> = out.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["foo"]);
    }

    fn empty_args() -> AddArgs {
        AddArgs {
            source: "test".into(),
            list: false,
            skill: vec![],
            all: false,
            harnesses: vec![],
            yes: false,
            global: false,
            profile: vec![],
            project: None,
            r#ref: None,
            copy: false,
            dangerously_accept_openclaw_risks: false,
        }
    }

    #[test]
    fn pick_skills_matches_standard_length_name() {
        let raw = "a".repeat(crate::discover::MAX_NAME_CHARS + 10);
        let canonical = "a".repeat(crate::discover::MAX_NAME_CHARS);
        let discovered = vec![skill(&canonical)];
        let mut args = empty_args();
        args.skill = vec![raw];

        let out = pick_skills(&discovered, &args).unwrap();
        let names: Vec<&str> = out.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec![canonical.as_str()]);
    }

    #[test]
    fn normalize_all_flag_implies_agent_star_and_yes() {
        let mut args = empty_args();
        args.all = true;
        normalize_all_flag(&mut args);
        assert_eq!(args.harnesses, vec!["*".to_string()]);
        assert!(args.yes);
    }

    #[test]
    fn normalize_all_flag_preserves_explicit_agents() {
        let mut args = empty_args();
        args.all = true;
        args.harnesses = vec!["claude".into()];
        normalize_all_flag(&mut args);
        assert_eq!(args.harnesses, vec!["claude".to_string()]);
        assert!(args.yes);
    }

    #[test]
    fn normalize_all_flag_no_op_without_all() {
        let mut args = empty_args();
        args.skill = vec!["foo".into()];
        normalize_all_flag(&mut args);
        assert!(args.harnesses.is_empty());
        assert!(!args.yes);
    }
}
