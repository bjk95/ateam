use crate::cli::RemoteCommand;
use crate::paths;
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::{Command, Output};

pub fn run(cmd: RemoteCommand) -> Result<()> {
    let repo = paths::resolve_repo()?;
    if !repo.join(".git").exists() {
        bail!("{} is not a git repository", repo.display());
    }
    match cmd {
        RemoteCommand::Add { url } => add(&repo, &url),
        RemoteCommand::List => list(&repo),
    }
}

fn add(repo: &Path, url: &str) -> Result<()> {
    if remote_exists(repo, "origin")? {
        let existing = remote_url(repo, "origin")?;
        bail!(
            "remote `origin` already set to {}. remove with `git -C {} remote remove origin` first.",
            existing,
            repo.display()
        );
    }

    let out = git(repo, &["remote", "add", "origin", url])?;
    if !out.status.success() {
        bail!(
            "git remote add failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    let branch = current_branch(repo)?;
    println!("agents: pushing {} to origin ({})...", branch, url);
    let push = git(repo, &["push", "-u", "origin", &branch])?;
    if !push.status.success() {
        // Roll back the remote so the user isn't left with a half-configured state.
        let _ = git(repo, &["remote", "remove", "origin"]);
        bail!(
            "git push -u origin {} failed:\n{}\n\nremote was rolled back. fix the remote (e.g. create the empty repo on GitHub first) and re-run.",
            branch,
            String::from_utf8_lossy(&push.stderr).trim()
        );
    }
    println!("agents: remote configured. mutating commands now auto-pull/commit/push.");
    Ok(())
}

fn list(repo: &Path) -> Result<()> {
    let out = git(repo, &["remote", "-v"])?;
    if !out.status.success() {
        bail!(
            "git remote -v failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let trimmed = s.trim();
    if trimmed.is_empty() {
        println!("(no remotes configured — run `agents remote add <url>`)");
    } else {
        println!("{}", trimmed);
    }
    Ok(())
}

fn remote_exists(repo: &Path, name: &str) -> Result<bool> {
    let out = git(repo, &["remote"])?;
    if !out.status.success() {
        bail!(
            "git remote failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let s = String::from_utf8_lossy(&out.stdout);
    Ok(s.lines().any(|l| l.trim() == name))
}

fn remote_url(repo: &Path, name: &str) -> Result<String> {
    let out = git(repo, &["remote", "get-url", name])?;
    if !out.status.success() {
        bail!(
            "git remote get-url {} failed: {}",
            name,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn current_branch(repo: &Path) -> Result<String> {
    let out = git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if !out.status.success() {
        bail!(
            "couldn't determine current branch: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if branch.is_empty() || branch == "HEAD" {
        bail!("repo has no commits yet — run `agents apply` (or commit something) before adding a remote");
    }
    Ok(branch)
}

fn git(repo: &Path, args: &[&str]) -> Result<Output> {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .with_context(|| format!("running `git {}`", args.join(" ")))
}
