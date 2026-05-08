use crate::ui;
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::{Command, Output};

/// Whether auto-sync should run for this invocation.
pub fn enabled(no_sync_flag: bool) -> bool {
    if no_sync_flag {
        return false;
    }
    match std::env::var("AGENTS_NO_SYNC") {
        Ok(v) => v != "1" && !v.eq_ignore_ascii_case("true"),
        Err(_) => true,
    }
}

/// Pre-pull stage. Soft-fails if remote is unreachable, hard-errors only on
/// true merge conflict.
pub fn pre_pull(repo: &Path) -> Result<()> {
    if !is_git_repo(repo) {
        return Ok(());
    }
    if !has_remote(repo)? {
        return Ok(());
    }
    let step = ui::step("pulling latest from remote");
    let out = run(repo, &["pull", "--ff-only"])?;
    step.finish();
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if is_offline(&stderr) {
        ui::warn("remote unreachable; using local lockfile state");
        return Ok(());
    }
    if stderr.contains("Not possible to fast-forward") || stderr.contains("diverging branches") {
        anyhow::bail!(
            "git pull --ff-only refused: local and remote have diverged.\n  resolve manually: git -C {} pull --rebase, then re-run.",
            repo.display()
        );
    }
    if has_no_tracking(&stderr) {
        // No upstream configured for current branch yet — fine.
        return Ok(());
    }
    ui::warn("`git pull --ff-only` failed");
    ui::detail(stderr.trim());
    Ok(())
}

/// Explicit git sync: reconcile with remote, then push local commits.
pub fn sync(repo: &Path) -> Result<()> {
    if !is_git_repo(repo) {
        bail!("{} is not a git repository", repo.display());
    }
    if !has_remote(repo)? {
        ui::warn("no git remote configured; skipping sync");
        return Ok(());
    }

    let step = ui::step("pulling latest from remote");
    let pull = run(repo, &["pull", "--rebase", "--autostash"])?;
    step.finish();
    if !pull.status.success() {
        let stderr = String::from_utf8_lossy(&pull.stderr);
        if is_offline(&stderr) {
            ui::warn("remote unreachable; sync skipped");
            return Ok(());
        }
        if has_no_tracking(&stderr) {
            ui::warn("current branch has no upstream; skipping pull");
        } else {
            bail!("git pull --rebase --autostash failed:\n{}", stderr.trim());
        }
    }

    if push_with_retry(repo)? {
        ui::ok("synced with remote");
    }
    Ok(())
}

/// Stage tracked agents files, commit if there are changes, push if remote exists.
/// Returns whether a commit was made.
pub fn commit_and_push(repo: &Path, message: &str) -> Result<bool> {
    if !is_git_repo(repo) {
        return Ok(false);
    }
    let stage_paths = stageable_paths(repo)?;
    if !stage_paths.is_empty() {
        let mut args = vec!["add", "-A", "--"];
        args.extend(stage_paths);
        let _ = run(repo, &args)?;
    }

    let diff = run(repo, &["diff", "--cached", "--quiet"])?;
    if diff.status.success() {
        // No staged changes.
        return Ok(false);
    }

    let step = ui::step("committing changes");
    let commit = run(repo, &["commit", "-m", message])?;
    step.finish();
    if !commit.status.success() {
        ui::warn("git commit failed");
        ui::detail(String::from_utf8_lossy(&commit.stderr).trim());
        return Ok(false);
    }

    let _ = push_with_retry(repo)?;
    Ok(true)
}

fn stageable_paths(repo: &Path) -> Result<Vec<&'static str>> {
    let candidates = [
        "agents.toml",
        "agents.lock.toml",
        "skills",
        "instructions",
        "agents",
    ];
    let mut paths = Vec::new();
    for path in candidates {
        if repo.join(path).exists() || is_tracked_path(repo, path)? {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn is_tracked_path(repo: &Path, path: &str) -> Result<bool> {
    let out = run(repo, &["ls-files", "--error-unmatch", path])?;
    Ok(out.status.success())
}

fn push_with_retry(repo: &Path) -> Result<bool> {
    if !has_remote(repo)? {
        ui::detail("no git remote configured; skipping push");
        return Ok(false);
    }
    let step = ui::step("pushing to remote");
    let attempt = run(repo, &["push"])?;
    step.finish();
    if attempt.status.success() {
        return Ok(true);
    }
    let stderr = String::from_utf8_lossy(&attempt.stderr);
    if is_offline(&stderr) {
        ui::warn("remote unreachable, commit retained locally");
        return Ok(false);
    }
    if stderr.contains("rejected")
        && (stderr.contains("non-fast-forward") || stderr.contains("fetch first"))
    {
        let step = ui::step("remote moved, rebasing and retrying push");
        let rebase = run(repo, &["pull", "--rebase", "--autostash"])?;
        if !rebase.status.success() {
            step.fail("rebase failed; commit retained locally");
            ui::detail(String::from_utf8_lossy(&rebase.stderr).trim());
            return Ok(false);
        }
        let retry = run(repo, &["push"])?;
        if retry.status.success() {
            step.ok("rebased and pushed");
            return Ok(true);
        }
        step.fail("push still failed after rebase; commit retained locally");
        ui::detail(String::from_utf8_lossy(&retry.stderr).trim());
        return Ok(false);
    }
    if stderr.contains("does not appear to be a git repository") {
        ui::detail("git remote not reachable; skipping push");
        return Ok(false);
    }
    ui::warn("git push failed");
    ui::detail(stderr.trim());
    Ok(false)
}

/// Count commits on HEAD that aren't on the upstream tracking branch.
/// Returns `None` when the repo has no remote or no upstream tracking branch
/// configured (i.e., there's nothing to compare against). Soft-fails to `None`
/// on any git error so callers can treat it as a best-effort UX hint.
pub fn unpushed_count(repo: &Path) -> Option<usize> {
    if !is_git_repo(repo) {
        return None;
    }
    if !has_remote(repo).unwrap_or(false) {
        return None;
    }
    let out = run(repo, &["rev-list", "@{u}..HEAD", "--count"]).ok()?;
    if !out.status.success() {
        // No upstream tracking branch yet; nothing to compare.
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

fn is_git_repo(repo: &Path) -> bool {
    repo.join(".git").exists()
}

fn has_remote(repo: &Path) -> Result<bool> {
    let out = run(repo, &["remote"])?;
    if !out.status.success() {
        return Ok(false);
    }
    Ok(!String::from_utf8_lossy(&out.stdout).trim().is_empty())
}

fn run(repo: &Path, args: &[&str]) -> Result<Output> {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .with_context(|| format!("running `git {}`", args.join(" ")))
}

fn is_offline(stderr: &str) -> bool {
    stderr.contains("Could not resolve host")
        || stderr.contains("Could not read from remote repository")
        || stderr.contains("Network is unreachable")
        || stderr.contains("Connection timed out")
        || stderr.contains("Operation timed out")
        || stderr.contains("ssh: Could not resolve hostname")
}

fn has_no_tracking(stderr: &str) -> bool {
    stderr.contains("There is no tracking information")
        || stderr.contains("no tracking information")
        || stderr.contains("has no upstream branch")
}

// ---------------------------------------------------------------------------
// Commit-message helpers (deterministic per-command messages)

pub fn msg_add(source: &str, skills: &[String]) -> String {
    let list = skills.join(", ");
    format!("add {} :: {}", source, list)
}

pub fn msg_remove(skills: &[String]) -> String {
    let list = skills.join(", ");
    format!("remove :: {}", list)
}

pub fn msg_deactivate(skill: &str) -> String {
    format!("deactivate :: {}", skill)
}

pub fn msg_activate(skill: &str) -> String {
    format!("activate :: {}", skill)
}

pub fn msg_update_one(skill: &str, from_sha: &str, to_sha: &str) -> String {
    format!(
        "update :: {} (sha {} → {})",
        skill,
        short(from_sha),
        short(to_sha)
    )
}

pub fn msg_update_bulk(count: usize) -> String {
    format!("update :: {} entries refreshed", count)
}

pub fn msg_import(skill: &str, source: &str) -> String {
    format!("import :: {} ({})", skill, source)
}

pub fn msg_apply(materialized: usize) -> String {
    format!("apply :: {} entries materialized", materialized)
}

pub fn msg_edit(target: &str) -> String {
    format!("edit :: {}", target)
}

pub fn msg_harness_add(ids: &[String]) -> String {
    format!("harness add :: {}", ids.join(", "))
}

pub fn msg_harness_remove(ids: &[String]) -> String {
    format!("harness remove :: {}", ids.join(", "))
}

fn short(sha: &str) -> String {
    sha.chars().take(7).collect()
}
