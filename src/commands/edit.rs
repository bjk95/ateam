use crate::cli::{EditArgs, EditTarget};
use crate::git_sync;
use crate::paths;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

const FALLBACK_EDITOR: &str = "vim";

pub fn run(args: EditArgs, no_sync: bool) -> Result<()> {
    let repo = paths::resolve_repo()?;

    let (target_path, label): (PathBuf, &str) = match args.target {
        Some(EditTarget::Instructions) => {
            let path = paths::instructions_template(&repo);
            if !path.exists() {
                bail!(
                    "instructions template not found at {} — run `ateam skills import --instructions` to bootstrap",
                    path.display()
                );
            }
            (path, "instructions")
        }
        None => (repo.clone(), "state"),
    };

    let editor = pick_editor();

    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    spawn_editor(&editor, &target_path)?;

    if git_sync::enabled(no_sync) {
        let msg = git_sync::msg_edit(label);
        let _ = git_sync::commit_and_push(&repo, &msg);
    }

    Ok(())
}

fn pick_editor() -> String {
    for var in ["VISUAL", "EDITOR"] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return v;
            }
        }
    }
    FALLBACK_EDITOR.to_string()
}

fn spawn_editor(editor: &str, path: &Path) -> Result<()> {
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
