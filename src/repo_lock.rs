use anyhow::{bail, Context, Result};
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

/// Exclusive flock guarding all mutations of the repo's lockfile and manifest.
/// Held for the lifetime of a single `ateam` mutating command so concurrent
/// invocations serialize on read-modify-write of `ateam.lock.toml` and
/// `.ateam/manifest.toml`.
#[derive(Debug)]
pub struct RepoLock {
    file: File,
    path: PathBuf,
}

impl RepoLock {
    /// Acquire an exclusive lock on `<repo>/.ateam/lock`.
    ///
    /// Tries non-blocking first. If the lock is held and `no_wait` is true,
    /// returns an error immediately. Otherwise blocks until the holder releases.
    pub fn acquire(repo: &Path, no_wait: bool) -> Result<Self> {
        let path = lock_path(repo);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("opening lock file {}", path.display()))?;

        match file.try_lock_exclusive() {
            Ok(()) => return Ok(Self { file, path }),
            Err(_) if no_wait => {
                bail!(
                    "another `ateam` process holds the lock at {}; rerun without --no-wait or wait for it to finish",
                    path.display()
                );
            }
            Err(_) => {
                crate::ui::detail(format!(
                    "waiting for another ateam process to release {}",
                    path.display()
                ));
            }
        }

        file.lock_exclusive()
            .with_context(|| format!("acquiring exclusive lock on {}", path.display()))?;
        Ok(Self { file, path })
    }
}

impl Drop for RepoLock {
    fn drop(&mut self) {
        if let Err(e) = FileExt::unlock(&self.file) {
            tracing::warn!("releasing lock {}: {}", self.path.display(), e);
        }
    }
}

fn lock_path(repo: &Path) -> PathBuf {
    repo.join(".ateam").join("lock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_creates_lock_file() {
        let tmp = tempfile::tempdir().unwrap();
        let _lock = RepoLock::acquire(tmp.path(), false).expect("first acquire");
        assert!(tmp.path().join(".ateam").join("lock").exists());
    }

    #[test]
    fn no_wait_fails_when_already_held() {
        let tmp = tempfile::tempdir().unwrap();
        let _held = RepoLock::acquire(tmp.path(), false).expect("first acquire");
        let err = RepoLock::acquire(tmp.path(), true).expect_err("expected contention error");
        assert!(
            err.to_string().contains("another `ateam` process"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn release_allows_reacquire() {
        let tmp = tempfile::tempdir().unwrap();
        {
            let _held = RepoLock::acquire(tmp.path(), false).expect("first acquire");
        }
        // Should succeed without blocking now that the previous lock is dropped.
        let _again = RepoLock::acquire(tmp.path(), true).expect("reacquire after drop");
    }
}
