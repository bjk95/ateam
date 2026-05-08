use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

/// Shallow-clone a git URL to `dest`. If `git_ref` is supplied, checks it out.
pub fn clone(url: &str, git_ref: Option<&str>, dest: &Path) -> Result<()> {
    if dest.exists() {
        bail!("clone target {} already exists", dest.display());
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut cmd = Command::new("git");
    cmd.arg("clone").arg("--depth").arg("1");
    if let Some(r) = git_ref {
        cmd.arg("--branch").arg(r);
    }
    cmd.arg(url).arg(dest);
    let status = cmd.status().context("spawning `git clone`")?;
    if !status.success() {
        bail!("git clone failed for {}", url);
    }
    Ok(())
}

/// `git ls-remote <url> <ref>` returning the commit SHA, or None if missing.
pub fn ls_remote_sha(url: &str, git_ref: &str) -> Result<Option<String>> {
    let out = Command::new("git")
        .arg("ls-remote")
        .arg(url)
        .arg(git_ref)
        .output()
        .context("running `git ls-remote`")?;
    if !out.status.success() {
        bail!(
            "git ls-remote {} {} failed: {}",
            url,
            git_ref,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let first_line = text.lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        return Ok(None);
    }
    let sha = first_line
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    if sha.is_empty() {
        Ok(None)
    } else {
        Ok(Some(sha))
    }
}
