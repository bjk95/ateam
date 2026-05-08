use crate::git_sync;
use crate::paths;
use crate::ui;
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

const FALLBACK_EDITOR: &str = "vim";

pub fn run(no_sync: bool) -> Result<()> {
    let repo = paths::resolve_repo()?;
    let editor = pick_editor();

    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    spawn_editor(&editor, &repo)?;

    if git_sync::enabled(no_sync) {
        let msg = git_sync::msg_edit("state");
        if let Err(e) = git_sync::commit_and_push(&repo, &msg) {
            ui::warn(format!("auto-sync failed: {:#}", e));
            ui::detail("local change saved; rerun a mutating command to retry");
        }
    }

    Ok(())
}

pub fn pick_editor() -> String {
    for var in ["VISUAL", "EDITOR"] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return v;
            }
        }
    }
    FALLBACK_EDITOR.to_string()
}

pub fn spawn_editor(editor: &str, path: &Path) -> Result<()> {
    let cmd = format!("{} {}", editor, shell_quote(&path.to_string_lossy()));
    let status = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .status()
        .with_context(|| format!("spawning editor: {}", cmd))?;
    if !status.success() {
        bail!("editor exited with status {}", status);
    }
    Ok(())
}

fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{}'", escaped)
}
