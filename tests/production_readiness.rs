use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

struct Fixture {
    _tmp: TempDir,
    repo: PathBuf,
    home: PathBuf,
    config: PathBuf,
    cache: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let home = tmp.path().join("home");
        let config = tmp.path().join("config");
        let cache = tmp.path().join("cache");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(cache.join("agents")).unwrap();
        std::fs::write(cache.join("agents/update-check"), "fresh").unwrap();
        std::fs::write(
            config.join("agents.toml"),
            format!("repo = \"{}\"\n", repo.display()),
        )
        .unwrap();
        std::fs::create_dir_all(repo.join(".agents")).unwrap();
        Self {
            _tmp: tmp,
            repo,
            home,
            config,
            cache,
        }
    }

    fn write_repo_config(&self, enabled_harnesses: &[&str]) {
        let harnesses = enabled_harnesses
            .iter()
            .map(|h| format!("\"{}\"", h))
            .collect::<Vec<_>>()
            .join(", ");
        std::fs::write(
            self.repo.join("agents.toml"),
            format!("enabled_harnesses = [{}]\n", harnesses),
        )
        .unwrap();
    }

    fn write_local_skill_lockfile(&self, name: &str) {
        let skill_dir = self.repo.join("skills").join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "skill body\n").unwrap();
        std::fs::write(
            self.repo.join("agents.lock.toml"),
            format!(
                "[[skill]]\nname = \"{name}\"\nsource = \"local:skills/{name}\"\npath = \"skills/{name}\"\nharnesses = [\"codex\"]\n"
            ),
        )
        .unwrap();
    }

    fn write_remote_skill_lockfile(&self, name: &str) {
        std::fs::write(
            self.repo.join("agents.lock.toml"),
            format!(
                "[[skill]]\nname = \"{name}\"\nsource = \"github:example/missing\"\npath = \"skills/{name}\"\nharnesses = [\"codex\"]\n"
            ),
        )
        .unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_agents"))
            .args(args)
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", &self.config)
            .env("XDG_CACHE_HOME", &self.cache)
            .env("NO_COLOR", "1")
            .output()
            .unwrap()
    }

    fn assert_success(&self, args: &[&str]) -> Output {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "command {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn codex_skill_path(&self, name: &str) -> PathBuf {
        self.home.join(".codex/skills").join(name)
    }
}

fn git(args: &[&str], cwd: &Path) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap();
    assert!(status.success(), "git {:?} failed", args);
}

#[test]
fn quiet_suppresses_remote_list_plain_output() {
    let fx = Fixture::new();
    fx.write_repo_config(&[]);
    std::fs::write(fx.repo.join("agents.lock.toml"), "").unwrap();
    git(&["init", "--quiet"], &fx.repo);
    git(
        &["remote", "add", "origin", "https://example.com/repo.git"],
        &fx.repo,
    );

    let output = fx.assert_success(&["--quiet", "remote", "list"]);

    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn apply_dry_run_preserves_tmp_dirs() {
    let fx = Fixture::new();
    fx.write_repo_config(&[]);
    std::fs::write(fx.repo.join("agents.lock.toml"), "").unwrap();
    let stale = fx.repo.join(".agents/tmp/stale-dir");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("marker"), "stale").unwrap();

    fx.assert_success(&["--no-sync", "apply", "--dry-run"]);

    assert!(stale.exists(), "dry-run should not sweep .agents/tmp");
}

#[test]
fn apply_dry_run_does_not_fetch_missing_remote_snapshot() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    fx.write_remote_skill_lockfile("remote-alpha");

    fx.assert_success(&["--no-sync", "apply", "--dry-run"]);

    assert!(!fx.repo.join("skills/remote-alpha").exists());
}

#[test]
fn repeated_apply_keeps_manifest_content_stable() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    fx.write_local_skill_lockfile("alpha");

    fx.assert_success(&["--no-sync", "apply"]);
    let first = std::fs::read_to_string(fx.repo.join(".agents/manifest.toml")).unwrap();
    fx.assert_success(&["--no-sync", "apply"]);
    let second = std::fs::read_to_string(fx.repo.join(".agents/manifest.toml")).unwrap();

    assert_eq!(first, second);
}

#[test]
fn deactivate_removes_copy_mode_skill_install() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    fx.write_local_skill_lockfile("alpha");

    fx.assert_success(&["--quiet", "--no-sync", "apply", "--copy"]);
    assert!(fx.codex_skill_path("alpha").is_dir());

    fx.assert_success(&["--quiet", "--no-sync", "skills", "deactivate", "alpha"]);

    assert!(!fx.codex_skill_path("alpha").exists());
}

#[test]
fn remove_removes_copy_mode_skill_install() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    fx.write_local_skill_lockfile("alpha");

    fx.assert_success(&["--quiet", "--no-sync", "apply", "--copy"]);
    assert!(fx.codex_skill_path("alpha").is_dir());

    fx.assert_success(&["--quiet", "--no-sync", "skills", "remove", "alpha", "-y"]);

    assert!(!fx.codex_skill_path("alpha").exists());
}
