use crate::ui;
use anyhow::{Context, Result};
use std::path::Path;
use std::process::{Command, Output};

/// Whether auto-sync should run for this invocation.
pub fn enabled(no_sync_flag: bool) -> bool {
    if no_sync_flag {
        return false;
    }
    match std::env::var("ATEAM_NO_SYNC") {
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
    let out = run(repo, &["pull", "--ff-only"])?;
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
    if stderr.contains("There is no tracking information") || stderr.contains("no tracking information") {
        // No upstream configured for current branch yet — fine.
        return Ok(());
    }
    ui::warn("`git pull --ff-only` failed");
    ui::detail(stderr.trim().to_string());
    Ok(())
}

/// Stage tracked ateam files, commit if there are changes, push if remote exists.
/// Returns whether a commit was made.
pub fn commit_and_push(repo: &Path, message: &str) -> Result<bool> {
    if !is_git_repo(repo) {
        return Ok(false);
    }
    let _ = run(
        repo,
        &["add", "ateam.toml", "ateam.lock.toml", "skills", "instructions"],
    )?;

    let diff = run(repo, &["diff", "--cached", "--quiet"])?;
    if diff.status.success() {
        // No staged changes.
        return Ok(false);
    }

    let commit = run(repo, &["commit", "-m", message])?;
    if !commit.status.success() {
        ui::warn("git commit failed");
        ui::detail(String::from_utf8_lossy(&commit.stderr).trim().to_string());
        return Ok(false);
    }

    push_with_retry(repo)?;
    Ok(true)
}

fn push_with_retry(repo: &Path) -> Result<()> {
    if !has_remote(repo)? {
        ui::detail("no git remote configured; skipping push");
        return Ok(());
    }
    let attempt = run(repo, &["push"])?;
    if attempt.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&attempt.stderr);
    if is_offline(&stderr) {
        ui::warn("remote unreachable, commit retained locally");
        return Ok(());
    }
    if stderr.contains("rejected") && (stderr.contains("non-fast-forward") || stderr.contains("fetch first")) {
        let step = ui::step("remote moved, rebasing and retrying push");
        let rebase = run(repo, &["pull", "--rebase"])?;
        if !rebase.status.success() {
            step.fail("rebase failed; commit retained locally");
            ui::detail(String::from_utf8_lossy(&rebase.stderr).trim().to_string());
            return Ok(());
        }
        let retry = run(repo, &["push"])?;
        if retry.status.success() {
            step.ok("rebased and pushed");
            return Ok(());
        }
        step.fail("push still failed after rebase; commit retained locally");
        ui::detail(String::from_utf8_lossy(&retry.stderr).trim().to_string());
        return Ok(());
    }
    if stderr.contains("does not appear to be a git repository") {
        ui::detail("git remote not reachable; skipping push");
        return Ok(());
    }
    ui::warn("git push failed");
    ui::detail(stderr.trim().to_string());
    Ok(())
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

// ---------------------------------------------------------------------------
// Commit-message helpers (deterministic per-command messages)

pub fn msg_add(source: &str, skills: &[String]) -> String {
    let list = skills.join(", ");
    format!("add {} :: {}", source, list)
}

pub fn msg_remove(skill: &str) -> String {
    format!("remove :: {}", skill)
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

fn short(sha: &str) -> String {
    sha.chars().take(7).collect()
}
