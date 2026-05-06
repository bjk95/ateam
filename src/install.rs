use anyhow::{anyhow, bail, Context, Result};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

/// Outcome of trying to install a single symlink.
#[derive(Debug)]
pub enum LinkOutcome {
    /// Created a new symlink.
    Created,
    /// Existing symlink already pointed at the right target.
    AlreadyCorrect,
    /// Replaced an existing symlink that pointed elsewhere.
    Replaced,
    /// Existing real file/dir was moved aside (only with `force`).
    MovedAside { backup: PathBuf },
    /// Refused because a real file/dir exists. Caller should escalate or skip.
    Refused,
}

/// Create a symlink at `link` pointing at `target`. Idempotent.
/// Replaces existing symlinks unconditionally; refuses on real files unless `force`.
pub fn install_symlink(link: &Path, target: &Path, force: bool) -> Result<LinkOutcome> {
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }

    let metadata = std::fs::symlink_metadata(link).ok();
    if let Some(meta) = metadata {
        if meta.file_type().is_symlink() {
            let existing = std::fs::read_link(link)
                .with_context(|| format!("reading symlink {}", link.display()))?;
            if existing == target {
                return Ok(LinkOutcome::AlreadyCorrect);
            }
            std::fs::remove_file(link)
                .with_context(|| format!("removing existing symlink {}", link.display()))?;
            symlink(target, link)
                .with_context(|| format!("creating symlink {} → {}", link.display(), target.display()))?;
            return Ok(LinkOutcome::Replaced);
        } else {
            if !force {
                return Ok(LinkOutcome::Refused);
            }
            let backup = backup_path(link);
            std::fs::rename(link, &backup)
                .with_context(|| format!("moving aside {} → {}", link.display(), backup.display()))?;
            symlink(target, link)
                .with_context(|| format!("creating symlink {} → {}", link.display(), target.display()))?;
            return Ok(LinkOutcome::MovedAside { backup });
        }
    }

    symlink(target, link)
        .with_context(|| format!("creating symlink {} → {}", link.display(), target.display()))?;
    Ok(LinkOutcome::Created)
}

/// Atomically materialize a fetched skill into `<repo>/skills/<name>/`.
///
/// `prepare()` returns a unique tmp dir to write into. `commit()` renames it
/// into the final snapshot slot. Failure between the two leaves the tmp
/// untouched (caller can call `sweep_tmp` on next apply to clean up). The
/// snapshot lives under `skills/` so it's tracked by git and travels to other
/// machines as part of the ateam-config repo — no per-machine refetch needed.
pub fn prepare_cache_slot(repo: &Path, skill_name: &str) -> Result<CacheSlot> {
    let dest_root = crate::paths::local_skills_dir(repo);
    let tmp_root = crate::paths::cache_tmp_dir(repo);
    std::fs::create_dir_all(&dest_root)
        .with_context(|| format!("creating {}", dest_root.display()))?;
    std::fs::create_dir_all(&tmp_root)
        .with_context(|| format!("creating {}", tmp_root.display()))?;
    let suffix: u64 = rand::random();
    let tmp = tmp_root.join(format!("{}-{:016x}", skill_name, suffix));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp).ok();
    }
    std::fs::create_dir_all(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
    Ok(CacheSlot {
        tmp,
        final_path: dest_root.join(skill_name),
    })
}

pub struct CacheSlot {
    pub tmp: PathBuf,
    pub final_path: PathBuf,
}

impl CacheSlot {
    pub fn commit(self) -> Result<PathBuf> {
        if self.final_path.exists() {
            std::fs::remove_dir_all(&self.final_path).with_context(|| {
                format!("removing existing {}", self.final_path.display())
            })?;
        }
        if let Some(parent) = self.final_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::rename(&self.tmp, &self.final_path).with_context(|| {
            format!(
                "renaming {} → {}",
                self.tmp.display(),
                self.final_path.display()
            )
        })?;
        Ok(self.final_path)
    }
}

/// Sweep stale dirs out of `<repo>/.ateam/cache/.tmp/` from previous failed apply runs.
pub fn sweep_cache_tmp(repo: &Path) -> Result<()> {
    let tmp_root = crate::paths::cache_tmp_dir(repo);
    if !tmp_root.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&tmp_root)
        .with_context(|| format!("reading {}", tmp_root.display()))?
        .flatten()
    {
        let _ = std::fs::remove_dir_all(entry.path());
    }
    Ok(())
}

/// Recursively copy a source directory into a destination directory.
/// Used when a tarball extracted to a subpath inside the package and we need
/// to extract just the skill's subdir into the cache slot.
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    if !src.is_dir() {
        bail!("copy_dir_recursive: src {} is not a directory", src.display());
    }
    std::fs::create_dir_all(dst)
        .with_context(|| format!("creating {}", dst.display()))?;
    for entry in std::fs::read_dir(src)
        .with_context(|| format!("reading {}", src.display()))?
        .flatten()
    {
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let ft = entry.file_type()?;
        if ft.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if ft.is_file() {
            std::fs::copy(&from, &to)
                .with_context(|| format!("copying {} → {}", from.display(), to.display()))?;
        } else if ft.is_symlink() {
            // Preserve symlinks as symlinks.
            let target = std::fs::read_link(&from)?;
            symlink(&target, &to)
                .with_context(|| format!("symlink {} → {}", to.display(), target.display()))?;
        }
    }
    Ok(())
}

/// Remove a path that ateam previously wrote (symlink or empty dir).
/// No-op if absent. Refuses to recursively delete real directories.
pub fn uninstall_path(path: &Path) -> Result<()> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(anyhow!("stat {}: {}", path.display(), e)),
    };
    if meta.file_type().is_symlink() {
        std::fs::remove_file(path)
            .with_context(|| format!("removing symlink {}", path.display()))?;
        return Ok(());
    }
    bail!(
        "refusing to delete non-symlink {} (was foreign or modified)",
        path.display()
    );
}

/// Outcome of trying to write a file copy.
#[derive(Debug)]
pub enum CopyOutcome {
    /// File was written (either fresh or replacing a previously-managed file).
    Written,
    /// Existing pre-existing file was moved aside before writing.
    MovedAside { backup: PathBuf },
    /// Refused because a foreign file exists at the path.
    Refused,
}

/// Atomically write `content` to `path`. If a file exists at `path` and
/// `was_managed` is false (meaning ateam didn't write it last apply), refuse
/// unless `force` is set (in which case the existing file is moved aside).
pub fn install_copy(
    path: &Path,
    content: &str,
    was_managed: bool,
    force: bool,
) -> Result<CopyOutcome> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }

    let exists = std::fs::symlink_metadata(path).is_ok();
    let mut moved: Option<PathBuf> = None;
    if exists && !was_managed {
        if !force {
            return Ok(CopyOutcome::Refused);
        }
        let backup = backup_path(path);
        std::fs::rename(path, &backup)
            .with_context(|| format!("moving aside {} → {}", path.display(), backup.display()))?;
        moved = Some(backup);
    }

    write_atomically(path, content)?;

    Ok(match moved {
        Some(backup) => CopyOutcome::MovedAside { backup },
        None => CopyOutcome::Written,
    })
}

fn write_atomically(path: &Path, content: &str) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .with_context(|| format!("creating {}", parent.display()))?;
    let suffix: u64 = rand::random();
    let tmp = parent.join(format!(
        ".{}.tmp.{:016x}",
        path.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        suffix
    ));
    std::fs::write(&tmp, content).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} → {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Remove a regular file ateam previously wrote via `install_copy`.
/// No-op if absent. Refuses to remove anything that's not a regular file.
pub fn uninstall_copy(path: &Path) -> Result<()> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(anyhow!("stat {}: {}", path.display(), e)),
    };
    if meta.file_type().is_file() {
        std::fs::remove_file(path)
            .with_context(|| format!("removing {}", path.display()))?;
        return Ok(());
    }
    bail!(
        "refusing to delete non-file {} (was foreign or modified)",
        path.display()
    );
}

fn backup_path(p: &Path) -> PathBuf {
    let parent = p.parent().unwrap_or_else(|| Path::new("."));
    let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ts = crate::manifest::now_unix();
    parent.join(format!("{}.bak.{}", name, ts))
}
