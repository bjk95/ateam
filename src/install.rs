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

/// Atomically materialize a fetched skill into the cache.
///
/// `prepare()` returns a unique tmp dir to write into. `commit()` renames it
/// into the final cache slot. Failure between the two leaves the tmp untouched
/// (caller can call `sweep_tmp` on next apply to clean up).
pub fn prepare_cache_slot(repo: &Path, skill_name: &str) -> Result<CacheSlot> {
    let cache = crate::paths::cache_dir(repo);
    let tmp_root = crate::paths::cache_tmp_dir(repo);
    std::fs::create_dir_all(&cache)
        .with_context(|| format!("creating {}", cache.display()))?;
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
        final_path: cache.join(skill_name),
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

fn backup_path(p: &Path) -> PathBuf {
    let parent = p.parent().unwrap_or_else(|| Path::new("."));
    let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ts = crate::manifest::now_unix();
    parent.join(format!("{}.bak.{}", name, ts))
}
