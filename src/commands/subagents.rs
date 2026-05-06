//! Subagent management — single-file `.md` agents installed under
//! `~/.claude/agents/<name>.md` and `~/.codex/agents/<name>.md`.
//!
//! Mirrors `commands/add.rs` / `commands/remove.rs` / `commands/list.rs` but
//! against `Lockfile::subagents` instead of `Lockfile::skills`. Snapshot lives
//! at `<repo>/agents/<name>.md` (single file, vendored to git).

use crate::cli::{SubagentAddArgs, SubagentRemoveArgs};
use crate::config::RepoConfig;
use crate::git_sync;
use crate::install;
use crate::lockfile::{Lockfile, SubagentEntry};
use crate::manifest::Manifest;
use crate::paths;
use crate::source::{github, Source};
use crate::ui;
use anyhow::{anyhow, bail, Context, Result};
use console::style;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// add

pub fn add(args: SubagentAddArgs, no_sync: bool) -> Result<()> {
    let repo = paths::resolve_repo()?;
    let repo_cfg = RepoConfig::load(&repo)?;

    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    let source = Source::parse_with(&args.source, args.dangerously_accept_openclaw_risks)?;
    ui::diamond(format!("Source: {}", args.source));

    let targets = resolve_add_targets(&args, &source)?;
    if targets.is_empty() {
        bail!("specify --subagent <name> (repeatable) or --path <file>");
    }

    let harnesses = resolve_harnesses(&args.harnesses, &repo_cfg);
    if harnesses.is_empty() {
        bail!("no harnesses with subagent support are enabled (claude-code, codex)");
    }

    let mut lock = Lockfile::load(&repo)?;
    let mut installed: Vec<String> = Vec::new();
    let mut had_error = false;

    for target in &targets {
        match install_one(&repo, &source, target, &args, &harnesses) {
            Ok(entry) => {
                lock.upsert_subagent(entry);
                lock.write(&repo).context("writing lockfile after upsert")?;
                installed.push(target.name.clone());
                ui::ok(format!("installed subagent {}", target.name));
            }
            Err(e) => {
                had_error = true;
                ui::fail(format!("install {} — {:#}", target.name, e));
            }
        }
    }

    if installed.is_empty() {
        if had_error {
            bail!("no subagents installed (all failed)");
        }
        return Ok(());
    }

    if git_sync::enabled(no_sync) {
        let msg = msg_subagent_add(&source.lockfile_string(), &installed);
        if let Err(e) = git_sync::commit_and_push(&repo, &msg) {
            ui::warn(format!("auto-sync failed: {:#}", e));
            ui::detail("local change saved; rerun a mutating command to retry");
        }
    }

    Ok(())
}

/// One unit of work for `add`: resolves to `(name, path-within-source)`.
struct AddTarget {
    name: String,
    path_in_source: String,
}

fn resolve_add_targets(args: &SubagentAddArgs, source: &Source) -> Result<Vec<AddTarget>> {
    // --path mode: explicit single-file pointer. Name comes from --subagent
    // (first entry) or the file stem.
    if let Some(p) = &args.path {
        let stem = Path::new(p)
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("--path `{}` has no file stem", p))?;
        let name = match args.subagent.first() {
            Some(n) => n.clone(),
            None => stem.to_string(),
        };
        return Ok(vec![AddTarget {
            name,
            path_in_source: p.clone(),
        }]);
    }

    // Local file source: the source itself is the subagent file.
    if let Source::Local { path } = source {
        if path.extension().and_then(|s| s.to_str()) == Some("md") {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| anyhow!("local file `{}` has no stem", path.display()))?;
            let name = match args.subagent.first() {
                Some(n) => n.clone(),
                None => stem.to_string(),
            };
            // The fetcher reads the file at this path verbatim — there's no
            // path-within-source.
            return Ok(vec![AddTarget {
                name,
                path_in_source: String::new(),
            }]);
        }
    }

    // Default: look for `agents/<name>.md` in the source for each --subagent.
    if args.subagent.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(args.subagent.len());
    for raw in &args.subagent {
        let name = raw.trim().to_string();
        if name.is_empty() {
            continue;
        }
        out.push(AddTarget {
            name: name.clone(),
            path_in_source: format!("agents/{}.md", name),
        });
    }
    Ok(out)
}

fn install_one(
    repo: &Path,
    source: &Source,
    target: &AddTarget,
    args: &SubagentAddArgs,
    harnesses: &[String],
) -> Result<SubagentEntry> {
    let (content, resolved_ref) =
        fetch_file(source, args.r#ref.as_deref(), &target.path_in_source)?;
    let file_sha = sha256_hex(content.as_bytes());

    // Snapshot the file into <repo>/agents/<name>.md. git_sync will commit it.
    let snapshot = paths::local_subagent_path(repo, &target.name);
    if let Some(parent) = snapshot.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    write_atomically(&snapshot, &content)?;

    // Symlink the canonical snapshot into each harness's agents dir.
    let install_root = paths::home_dir()?;
    let mut linked: Vec<PathBuf> = Vec::new();
    for harness in harnesses {
        let Some(link) = paths::harness_subagent_path(&install_root, harness, &target.name)? else {
            continue;
        };
        match install::install_symlink(&link, &snapshot, false)? {
            install::LinkOutcome::Created
            | install::LinkOutcome::Replaced
            | install::LinkOutcome::AlreadyCorrect
            | install::LinkOutcome::AutoHealed => {
                linked.push(link.clone());
                ui::detail(format!("linked {}", paths::display_path(&link)));
            }
            install::LinkOutcome::MovedAside { backup } => {
                linked.push(link.clone());
                ui::warn(format!(
                    "moved aside existing file at {} → {}",
                    paths::display_path(&link),
                    paths::display_path(&backup)
                ));
            }
            install::LinkOutcome::Refused => {
                ui::warn(format!(
                    "refused: real file at {} (use `ateam apply --force` to overwrite)",
                    paths::display_path(&link)
                ));
            }
        }
    }

    Ok(SubagentEntry {
        name: target.name.clone(),
        source: source.lockfile_string(),
        path: if target.path_in_source.is_empty() {
            None
        } else {
            Some(target.path_in_source.clone())
        },
        git_ref: resolved_ref,
        file_sha: Some(file_sha),
        harnesses: if args.harnesses.is_empty() {
            vec!["*".into()]
        } else {
            args.harnesses.clone()
        },
        profiles: args.profile.clone(),
        project: None,
        active: true,
        upstream: None,
    })
}

/// Fetch the subagent file contents. Returns `(body, pinned_ref)` — the ref
/// is recorded in the lockfile only when the user passed `--ref` (so future
/// `update` runs can re-resolve from the source's default branch).
fn fetch_file(
    source: &Source,
    git_ref: Option<&str>,
    path_in_source: &str,
) -> Result<(String, Option<String>)> {
    match source {
        Source::Github { owner, repo } => {
            let r = match git_ref {
                Some(r) => r.to_string(),
                None => github::default_branch(owner, repo),
            };
            let body =
                github::read_file_at_ref(owner, repo, &r, path_in_source).with_context(|| {
                    format!("fetching {}/{}@{}: {}", owner, repo, r, path_in_source)
                })?;
            Ok((body, git_ref.map(|s| s.to_string())))
        }
        Source::Git { url } => {
            let tmp = tempdir_in_repo()?;
            crate::source::git::clone(url, git_ref, &tmp.path)?;
            let file = tmp.path.join(path_in_source);
            let body = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            Ok((body, git_ref.map(|s| s.to_string())))
        }
        Source::Local { path } => {
            let abs = if path.is_absolute() {
                path.clone()
            } else {
                std::env::current_dir()?.join(path)
            };
            let file = if path_in_source.is_empty() {
                abs
            } else {
                abs.join(path_in_source)
            };
            if !file.exists() {
                bail!("local file not found: {}", file.display());
            }
            let body = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            Ok((body, None))
        }
    }
}

fn resolve_harnesses(requested: &[String], repo_cfg: &RepoConfig) -> Vec<String> {
    let want_all = requested.is_empty() || requested.iter().any(|r| r == "*");
    let candidates: Vec<String> = if want_all {
        repo_cfg.enabled_harnesses.clone()
    } else {
        requested.to_vec()
    };
    // Drop any harness that doesn't define a subagents_subdir. Keeps the
    // user's `--harness` list authoritative for skills, while silently no-op'ing
    // for harnesses where subagents make no sense.
    candidates
        .into_iter()
        .filter(|id| {
            crate::harness::lookup(id)
                .and_then(|d| d.subagents_subdir)
                .is_some()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// remove

pub fn remove(args: SubagentRemoveArgs, no_sync: bool) -> Result<()> {
    let repo = paths::resolve_repo()?;

    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    let mut lock = Lockfile::load(&repo)?;

    if !confirm_remove(&args)? {
        ui::warn("aborted");
        return Ok(());
    }

    let manifest = Manifest::load(&repo)?;
    let mut removed: Vec<String> = Vec::new();
    let mut had_error = false;

    for name in &args.names {
        if lock.find_subagent(name).is_none() {
            ui::warn(format!("subagent `{}` not in lockfile", name));
            continue;
        }
        match remove_one(&repo, name, &manifest) {
            Ok(()) => {
                lock.remove_subagent(name);
                removed.push(name.clone());
                ui::ok(format!("removed {}", name));
            }
            Err(e) => {
                had_error = true;
                ui::fail(format!("remove {} — {:#}", name, e));
            }
        }
    }

    if removed.is_empty() {
        if had_error {
            bail!("no subagents removed (all failed)");
        }
        return Ok(());
    }

    lock.write(&repo)?;

    if git_sync::enabled(no_sync) {
        let msg = msg_subagent_remove(&removed);
        if let Err(e) = git_sync::commit_and_push(&repo, &msg) {
            ui::warn(format!("auto-sync failed: {:#}", e));
            ui::detail("local change saved; rerun a mutating command to retry");
        }
    }

    Ok(())
}

fn remove_one(repo: &Path, name: &str, manifest: &Manifest) -> Result<()> {
    // Uninstall any symlinks the manifest knows about for this subagent.
    let snapshot = paths::local_subagent_path(repo, name);
    for entry in &manifest.entries {
        if entry.target == snapshot {
            install::uninstall_path(&entry.path)?;
        }
    }
    // Best-effort sweep of expected harness paths in case the manifest is
    // stale (e.g. user removed before `apply` had recorded the install).
    let install_root = paths::home_dir()?;
    for harness in crate::harness::ids() {
        if let Some(link) = paths::harness_subagent_path(&install_root, harness, name)? {
            // Only remove if it's a symlink pointing at our snapshot.
            if let Ok(target) = std::fs::read_link(&link) {
                if target == snapshot {
                    let _ = install::uninstall_path(&link);
                }
            }
        }
    }
    // Drop the snapshot file itself.
    if snapshot.exists() {
        std::fs::remove_file(&snapshot)
            .with_context(|| format!("removing snapshot {}", snapshot.display()))?;
    }
    Ok(())
}

fn confirm_remove(args: &SubagentRemoveArgs) -> Result<bool> {
    if args.yes {
        return Ok(true);
    }
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        return Ok(true);
    }
    ui::plain(format!(
        "remove {} subagent{}?",
        args.names.len(),
        if args.names.len() == 1 { "" } else { "s" }
    ));
    for n in &args.names {
        ui::detail(format!("  {}", n));
    }
    let mut buf = String::new();
    eprint!("[y/N] ");
    std::io::stdin()
        .read_line(&mut buf)
        .context("reading confirmation")?;
    Ok(matches!(
        buf.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

// ---------------------------------------------------------------------------
// list

pub fn list() -> Result<()> {
    let repo = paths::resolve_repo()?;
    let lock = Lockfile::load(&repo)?;
    if lock.subagents.is_empty() {
        ui::plain("no subagents locked");
        return Ok(());
    }
    for entry in &lock.subagents {
        let dot = if entry.active { "●" } else { "○" };
        let line = format!(
            "{} {}  {}",
            style(dot).cyan(),
            style(&entry.name).bold(),
            entry.source
        );
        ui::plain(line);
        if let Some(p) = &entry.path {
            ui::detail(format!("path: {}", p));
        }
        if !entry.profiles.is_empty() {
            ui::detail(format!("profiles: {}", entry.profiles.join(", ")));
        }
        if !entry.harnesses.iter().any(|h| h == "*") {
            ui::detail(format!("harnesses: {}", entry.harnesses.join(", ")));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// helpers

fn write_atomically(path: &Path, content: &str) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let suffix: u64 = rand::random();
    let stem = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = parent.join(format!(".{}.tmp.{:016x}", stem, suffix));
    std::fs::write(&tmp, content).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} → {}", tmp.display(), path.display()))?;
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().fold(String::with_capacity(64), |mut acc, b| {
        use std::fmt::Write;
        let _ = write!(acc, "{:02x}", b);
        acc
    })
}

struct TempDir {
    path: PathBuf,
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn tempdir_in_repo() -> Result<TempDir> {
    let base = paths::resolve_repo()?;
    let tmp_root = paths::cache_tmp_dir(&base);
    std::fs::create_dir_all(&tmp_root)
        .with_context(|| format!("creating {}", tmp_root.display()))?;
    let suffix: u64 = rand::random();
    let path = tmp_root.join(format!("subagent-fetch-{:016x}", suffix));
    std::fs::create_dir_all(&path).with_context(|| format!("creating {}", path.display()))?;
    Ok(TempDir { path })
}

fn msg_subagent_add(source: &str, names: &[String]) -> String {
    if names.len() == 1 {
        format!("subagent add: {} ({})", names[0], source)
    } else {
        format!("subagent add: {} ({})", names.join(", "), source,)
    }
}

fn msg_subagent_remove(names: &[String]) -> String {
    if names.len() == 1 {
        format!("subagent remove: {}", names[0])
    } else {
        format!("subagent remove: {}", names.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn resolve_targets_with_explicit_path_uses_stem_for_name() {
        let args = SubagentAddArgs {
            source: "foo/bar".into(),
            subagent: vec![],
            path: Some("agents/some/code-reviewer.md".into()),
            harnesses: vec![],
            yes: false,
            profile: vec![],
            r#ref: None,
            dangerously_accept_openclaw_risks: false,
        };
        let src = Source::Github {
            owner: "foo".into(),
            repo: "bar".into(),
        };
        let targets = resolve_add_targets(&args, &src).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "code-reviewer");
        assert_eq!(targets[0].path_in_source, "agents/some/code-reviewer.md");
    }

    #[test]
    fn resolve_targets_path_with_subagent_override_uses_override() {
        let args = SubagentAddArgs {
            source: "foo/bar".into(),
            subagent: vec!["renamed".into()],
            path: Some("agents/code-reviewer.md".into()),
            harnesses: vec![],
            yes: false,
            profile: vec![],
            r#ref: None,
            dangerously_accept_openclaw_risks: false,
        };
        let src = Source::Github {
            owner: "foo".into(),
            repo: "bar".into(),
        };
        let targets = resolve_add_targets(&args, &src).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "renamed");
    }

    #[test]
    fn resolve_targets_default_path_under_agents_dir() {
        let args = SubagentAddArgs {
            source: "foo/bar".into(),
            subagent: vec!["a".into(), "b".into()],
            path: None,
            harnesses: vec![],
            yes: false,
            profile: vec![],
            r#ref: None,
            dangerously_accept_openclaw_risks: false,
        };
        let src = Source::Github {
            owner: "foo".into(),
            repo: "bar".into(),
        };
        let targets = resolve_add_targets(&args, &src).unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].path_in_source, "agents/a.md");
        assert_eq!(targets[1].path_in_source, "agents/b.md");
    }

    #[test]
    fn resolve_targets_local_md_file_uses_stem() {
        let args = SubagentAddArgs {
            source: "/tmp/foo.md".into(),
            subagent: vec![],
            path: None,
            harnesses: vec![],
            yes: false,
            profile: vec![],
            r#ref: None,
            dangerously_accept_openclaw_risks: false,
        };
        let src = Source::Local {
            path: PathBuf::from("/tmp/foo.md"),
        };
        let targets = resolve_add_targets(&args, &src).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "foo");
        assert_eq!(targets[0].path_in_source, "");
    }

    #[test]
    fn sha256_hex_known_vector() {
        // sha256("abc") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
