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
    /// Existing real dir's content matched our snapshot byte-for-byte and was
    /// removed in place (no `force` needed — the data is already in the
    /// snapshot, so the redundant copy is safe to delete).
    AutoHealed,
    /// Refused because a real file/dir exists. Caller should escalate or skip.
    Refused,
}

/// Create a symlink at `link` pointing at `target`. Idempotent.
/// Replaces existing symlinks unconditionally; refuses on real files unless
/// `force`, with one exception: if the existing real directory's content is
/// byte-for-byte identical to `target`, it's removed silently (covers the
/// "skill installed pre-ateam, then imported" case where both copies still
/// exist on disk).
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
            // Auto-heal: byte-identical copy is redundant; safe to drop.
            if content_matches(link, target).unwrap_or(false) {
                if meta.is_dir() {
                    std::fs::remove_dir_all(link).with_context(|| {
                        format!("removing redundant copy at {}", link.display())
                    })?;
                } else {
                    std::fs::remove_file(link).with_context(|| {
                        format!("removing redundant copy at {}", link.display())
                    })?;
                }
                symlink(target, link).with_context(|| {
                    format!("creating symlink {} → {}", link.display(), target.display())
                })?;
                return Ok(LinkOutcome::AutoHealed);
            }
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

fn content_matches(a: &Path, b: &Path) -> Result<bool> {
    let ha = crate::source::local::content_hash(a)?;
    let hb = crate::source::local::content_hash(b)?;
    Ok(ha == hb)
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
        if let Some(parent) = self.final_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        // Three-step atomic swap: `final_path` is never absent between steps,
        // so concurrent apply runs (cron + interactive, etc.) can't observe a
        // gap that leaves their symlinks dangling.
        let quarantine = quarantine_path(&self.tmp);
        let displaced = match std::fs::rename(&self.final_path, &quarantine) {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => {
                return Err(anyhow!(
                    "moving aside {} → {}: {}",
                    self.final_path.display(),
                    quarantine.display(),
                    e
                ))
            }
        };
        std::fs::rename(&self.tmp, &self.final_path).with_context(|| {
            format!(
                "renaming {} → {}",
                self.tmp.display(),
                self.final_path.display()
            )
        })?;
        if displaced {
            // Best-effort; `sweep_cache_tmp` cleans up any straggler.
            let _ = std::fs::remove_dir_all(&quarantine);
        }
        Ok(self.final_path)
    }
}

fn quarantine_path(tmp: &Path) -> PathBuf {
    let mut name = tmp
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.push_str(".quarantine");
    tmp.with_file_name(name)
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

/// Remove a regular file or directory ateam previously wrote via
/// `install_copy` / `install_copy_dir`. No-op if absent. Refuses to remove
/// symlinks (those go through `uninstall_path`) or other unexpected types.
pub fn uninstall_copy(path: &Path) -> Result<()> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(anyhow!("stat {}: {}", path.display(), e)),
    };
    let ft = meta.file_type();
    if ft.is_file() {
        std::fs::remove_file(path)
            .with_context(|| format!("removing {}", path.display()))?;
        return Ok(());
    }
    if ft.is_dir() {
        std::fs::remove_dir_all(path)
            .with_context(|| format!("removing {}", path.display()))?;
        return Ok(());
    }
    bail!(
        "refusing to delete {} (was foreign or modified)",
        path.display()
    );
}

/// Outcome of trying to install a directory copy.
#[derive(Debug)]
pub enum CopyDirOutcome {
    Created,
    Replaced,
    AlreadyCorrect,
    MovedAside { backup: PathBuf },
    Refused,
}

/// Recursively copy `src` into `dst`. Used for `--copy` mode where filesystems
/// can't reliably handle symlinks. If `was_managed`, an existing dst was put
/// there by ateam's previous apply and may be replaced freely; otherwise a
/// pre-existing dst is moved aside (with `force`) or refused.
pub fn install_copy_dir(
    dst: &Path,
    src: &Path,
    was_managed: bool,
    force: bool,
) -> Result<CopyDirOutcome> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }

    let meta = std::fs::symlink_metadata(dst).ok();
    let Some(meta) = meta else {
        copy_dir_recursive(src, dst)?;
        return Ok(CopyDirOutcome::Created);
    };

    if meta.file_type().is_symlink() {
        // Prior apply ran in symlink mode; swap in a copy.
        std::fs::remove_file(dst)
            .with_context(|| format!("removing existing symlink {}", dst.display()))?;
        copy_dir_recursive(src, dst)?;
        return Ok(CopyDirOutcome::Replaced);
    }

    if was_managed {
        if meta.is_dir() {
            std::fs::remove_dir_all(dst)
                .with_context(|| format!("removing managed copy at {}", dst.display()))?;
        } else {
            std::fs::remove_file(dst)
                .with_context(|| format!("removing managed copy at {}", dst.display()))?;
        }
        copy_dir_recursive(src, dst)?;
        return Ok(CopyDirOutcome::Replaced);
    }

    if meta.is_dir() && content_matches_dir(dst, src).unwrap_or(false) {
        return Ok(CopyDirOutcome::AlreadyCorrect);
    }

    if !force {
        return Ok(CopyDirOutcome::Refused);
    }
    let backup = backup_path(dst);
    std::fs::rename(dst, &backup)
        .with_context(|| format!("moving aside {} → {}", dst.display(), backup.display()))?;
    copy_dir_recursive(src, dst)?;
    Ok(CopyDirOutcome::MovedAside { backup })
}

fn content_matches_dir(a: &Path, b: &Path) -> Result<bool> {
    let ha = crate::source::local::content_hash(a)?;
    let hb = crate::source::local::content_hash(b)?;
    Ok(ha == hb)
}

fn backup_path(p: &Path) -> PathBuf {
    let parent = p.parent().unwrap_or_else(|| Path::new("."));
    let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ts = crate::manifest::now_unix();
    parent.join(format!("{}.bak.{}", name, ts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_marker(dir: &Path, body: &str) {
        std::fs::write(dir.join("marker"), body).unwrap();
    }

    #[test]
    fn commit_replaces_existing_directory() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path();

        let first = prepare_cache_slot(repo, "demo").unwrap();
        write_marker(&first.tmp, "v1");
        let final_path = first.commit().unwrap();
        assert_eq!(std::fs::read_to_string(final_path.join("marker")).unwrap(), "v1");

        let second = prepare_cache_slot(repo, "demo").unwrap();
        write_marker(&second.tmp, "v2");
        let final_path = second.commit().unwrap();
        assert_eq!(std::fs::read_to_string(final_path.join("marker")).unwrap(), "v2");
    }

    #[test]
    fn commit_creates_when_absent() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path();

        let slot = prepare_cache_slot(repo, "demo").unwrap();
        write_marker(&slot.tmp, "v1");
        let final_path = slot.commit().unwrap();
        assert!(final_path.exists());
        assert_eq!(std::fs::read_to_string(final_path.join("marker")).unwrap(), "v1");
    }

    #[test]
    fn install_copy_dir_creates_when_absent() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        write_marker(&src, "v1");
        let dst = tmp.path().join("agents/skill");

        let outcome = install_copy_dir(&dst, &src, false, false).unwrap();
        assert!(matches!(outcome, CopyDirOutcome::Created));
        assert_eq!(std::fs::read_to_string(dst.join("marker")).unwrap(), "v1");
    }

    #[test]
    fn install_copy_dir_replaces_existing_symlink() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        write_marker(&src, "v1");
        let dst = tmp.path().join("dst");
        std::os::unix::fs::symlink(&src, &dst).unwrap();

        let outcome = install_copy_dir(&dst, &src, false, false).unwrap();
        assert!(matches!(outcome, CopyDirOutcome::Replaced));
        assert!(!std::fs::symlink_metadata(&dst).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(dst.join("marker")).unwrap(), "v1");
    }

    #[test]
    fn install_copy_dir_refuses_foreign_dir_without_force() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        write_marker(&src, "v2");
        let dst = tmp.path().join("dst");
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(dst.join("user-file"), "untouched").unwrap();

        let outcome = install_copy_dir(&dst, &src, false, false).unwrap();
        assert!(matches!(outcome, CopyDirOutcome::Refused));
        assert_eq!(std::fs::read_to_string(dst.join("user-file")).unwrap(), "untouched");
    }

    #[test]
    fn install_copy_dir_replaces_managed_dir_freely() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        write_marker(&src, "v2");
        let dst = tmp.path().join("dst");
        std::fs::create_dir_all(&dst).unwrap();
        write_marker(&dst, "v1-stale");

        let outcome = install_copy_dir(&dst, &src, true, false).unwrap();
        assert!(matches!(outcome, CopyDirOutcome::Replaced));
        assert_eq!(std::fs::read_to_string(dst.join("marker")).unwrap(), "v2");
    }

    #[test]
    fn uninstall_copy_removes_directory() {
        let tmp = TempDir::new().unwrap();
        let dst = tmp.path().join("dst");
        std::fs::create_dir_all(&dst).unwrap();
        write_marker(&dst, "v1");

        uninstall_copy(&dst).unwrap();
        assert!(!dst.exists());
    }

    #[test]
    fn commit_leaves_no_quarantine_in_tmp() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path();

        let first = prepare_cache_slot(repo, "demo").unwrap();
        write_marker(&first.tmp, "v1");
        first.commit().unwrap();

        let second = prepare_cache_slot(repo, "demo").unwrap();
        write_marker(&second.tmp, "v2");
        second.commit().unwrap();

        let cache_tmp = crate::paths::cache_tmp_dir(repo);
        let leftovers: Vec<_> = std::fs::read_dir(&cache_tmp)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        assert!(leftovers.is_empty(), "cache tmp not empty: {:?}", leftovers);
    }
}
