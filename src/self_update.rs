use anyhow::{anyhow, Result};
use axoupdater::{AxoUpdater, AxoupdateError, ReleaseSource, ReleaseSourceType, Version};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const TTL: Duration = Duration::from_secs(24 * 3600);
const REPO_OWNER: &str = "bjk95";
const REPO_NAME: &str = "agents";
const APP_NAME: &str = "agents";

pub(crate) fn is_cache_fresh(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(mtime) = meta.modified() else {
        return false;
    };
    mtime.elapsed().map(|e| e < TTL).unwrap_or(false)
}

fn cache_path() -> Result<PathBuf> {
    if let Some(custom) = std::env::var_os("XDG_CACHE_HOME") {
        let p = PathBuf::from(custom);
        if !p.as_os_str().is_empty() {
            return Ok(p.join("agents").join("update-check"));
        }
    }
    let dirs =
        directories::BaseDirs::new().ok_or_else(|| anyhow!("could not determine home dir"))?;
    Ok(dirs
        .home_dir()
        .join(".cache")
        .join("agents")
        .join("update-check"))
}

fn touch_cache(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    std::fs::write(path, now.to_string())?;
    Ok(())
}

fn build_updater() -> AxoUpdater {
    let mut u = AxoUpdater::new_for(APP_NAME);
    // Populates install_prefix (and source/current_version) from the receipt
    // the cargo-dist shell installer writes. Without this, run_sync fails with
    // NotConfigured { missing_field: "install_prefix" }.
    // Older installs may not have a receipt, so fall back to this executable's
    // install prefix in that case.
    if let Err(AxoupdateError::NoReceipt { .. }) = u.load_receipt() {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(prefix) = install_prefix_for_exe(&exe) {
                u.set_install_dir(prefix.to_string_lossy().into_owned());
            }
        }
    }
    u.set_release_source(ReleaseSource {
        release_type: ReleaseSourceType::GitHub,
        owner: REPO_OWNER.into(),
        name: REPO_NAME.into(),
        app_name: APP_NAME.into(),
    });
    if let Ok(version) = env!("CARGO_PKG_VERSION").parse::<Version>() {
        let _ = u.set_current_version(version);
    }
    u
}

fn install_prefix_for_exe(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    if dir.file_name().is_some_and(|name| name == "bin") {
        if let Some(parent) = dir.parent() {
            return Some(parent.to_path_buf());
        }
    }
    Some(dir.to_path_buf())
}

fn run_update(loud: bool) -> Result<Option<(String, String)>> {
    let mut u = build_updater();
    if loud {
        u.enable_installer_output();
    } else {
        u.disable_installer_output();
    }
    match u.run_sync()? {
        Some(result) => {
            let from = result
                .old_version
                .map(|v| v.to_string())
                .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
            let to = result.new_version.to_string();
            Ok(Some((from, to)))
        }
        None => Ok(None),
    }
}

pub(crate) fn maybe_check() {
    let _ = (|| -> Result<()> {
        let cache = cache_path()?;
        if is_cache_fresh(&cache) {
            return Ok(());
        }
        match run_update(false) {
            Ok(Some((from, to))) => {
                eprintln!("agents: updated {} → {}", from, to);
                touch_cache(&cache)?;
            }
            Ok(None) => {
                touch_cache(&cache)?;
            }
            Err(_) => {
                // network / rate-limit / permission — leave cache untouched
                // so the next invocation retries.
            }
        }
        Ok(())
    })();
}

pub(crate) fn force_upgrade() -> Result<()> {
    match run_update(true)? {
        Some((from, to)) => println!("agents: updated {} → {}", from, to),
        None => println!("agents: already at latest ({})", env!("CARGO_PKG_VERSION")),
    }
    if let Ok(cache) = cache_path() {
        let _ = touch_cache(&cache);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::TempDir;

    #[test]
    fn cache_freshness_boundary() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("check");

        assert!(
            !is_cache_fresh(&path),
            "missing file should not be considered fresh"
        );

        File::create(&path).unwrap();
        let file = File::options().write(true).open(&path).unwrap();

        let just_under_24h = SystemTime::now() - Duration::from_secs(24 * 3600 - 60);
        file.set_modified(just_under_24h).unwrap();
        assert!(
            is_cache_fresh(&path),
            "file with mtime 23h59m ago should be fresh"
        );

        let just_over_24h = SystemTime::now() - Duration::from_secs(24 * 3600 + 60);
        file.set_modified(just_over_24h).unwrap();
        assert!(
            !is_cache_fresh(&path),
            "file with mtime 24h01m ago should be stale"
        );
    }

    #[test]
    fn updater_has_install_dir_without_receipt() {
        let dir = TempDir::new().unwrap();
        let old_config_path = std::env::var_os("AXOUPDATER_CONFIG_PATH");
        std::env::set_var("AXOUPDATER_CONFIG_PATH", dir.path());

        let updater = build_updater();
        let install_root = updater.install_prefix_root();
        let current_exe = std::env::current_exe().unwrap();
        let expected = install_prefix_for_exe(&current_exe)
            .unwrap()
            .to_string_lossy()
            .into_owned();

        match old_config_path {
            Some(value) => std::env::set_var("AXOUPDATER_CONFIG_PATH", value),
            None => std::env::remove_var("AXOUPDATER_CONFIG_PATH"),
        }

        let install_root =
            install_root.expect("missing receipt should fall back to current executable dir");
        assert_eq!(install_root.as_str(), expected);
    }

    #[test]
    fn install_prefix_for_exe_strips_bin_dir() {
        let dir = TempDir::new().unwrap();
        let exe = dir.path().join("bin").join("agents");

        let prefix = install_prefix_for_exe(&exe).unwrap();

        assert_eq!(prefix, dir.path());
    }
}
