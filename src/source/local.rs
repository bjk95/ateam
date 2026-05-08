use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// Resolve a `local:` source path against the agents repo root.
/// `path_in_source` may be relative ("skills/foo") or absolute.
pub fn resolve(repo: &Path, path_in_source: &Path) -> Result<PathBuf> {
    let resolved = if path_in_source.is_absolute() {
        path_in_source.to_path_buf()
    } else {
        repo.join(path_in_source)
    };
    if !resolved.exists() {
        bail!("local source path {} does not exist", resolved.display());
    }
    Ok(resolved)
}

/// Compute a content hash for change detection (sha256 of all file contents,
/// keyed by relative path, sorted). Used for `local:` drift detection.
pub fn content_hash(dir: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut files: Vec<(PathBuf, PathBuf)> = Vec::new();
    collect_files(dir, dir, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = Sha256::new();
    for (rel, abs) in &files {
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update([0u8]);
        let bytes = std::fs::read(abs).with_context(|| format!("reading {}", abs.display()))?;
        hasher.update(&bytes);
        hasher.update([0u8]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, PathBuf)>) -> Result<()> {
    let entries = std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
            if name.starts_with('.') {
                continue;
            }
        }
        let ft = entry.file_type()?;
        if ft.is_dir() {
            collect_files(root, &path, out)?;
        } else if ft.is_file() {
            let rel = path
                .strip_prefix(root)
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|_| path.clone());
            out.push((rel, path));
        }
    }
    Ok(())
}
