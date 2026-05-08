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

    fn write_local_skill_source(&self, name: &str) -> PathBuf {
        let source = self.home.join("sources").join(name);
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            source.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {name} skill.\n---\n\nBody.\n"),
        )
        .unwrap();
        source
    }

    fn write_subagent_source(&self, name: &str) -> PathBuf {
        let source = self.home.join("subagents").join(format!("{name}.md"));
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(
            &source,
            format!("---\nname: {name}\ndescription: Review code.\n---\n\nReview the code.\n"),
        )
        .unwrap();
        source
    }

    fn write_instructions_template(&self, body: &str) {
        std::fs::create_dir_all(self.repo.join("instructions")).unwrap();
        std::fs::write(self.repo.join("instructions/instructions.md.hbs"), body).unwrap();
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

    fn codex_instructions_path(&self) -> PathBuf {
        self.home.join(".codex/AGENTS.md")
    }

    fn claude_instructions_path(&self) -> PathBuf {
        self.home.join(".claude/CLAUDE.md")
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

fn git_output(args: &[&str], cwd: &Path) -> Output {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
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
fn immediate_add_then_remove_uninstalls_skill_target() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    let source = fx.write_local_skill_source("alpha");
    let source_parent = source.parent().unwrap().to_str().unwrap();

    fx.assert_success(&[
        "--quiet",
        "--no-sync",
        "skills",
        "add",
        source_parent,
        "--skill",
        "alpha",
        "--global",
        "--harness",
        "codex",
        "-y",
    ]);
    let link = fx.codex_skill_path("alpha");
    assert!(std::fs::symlink_metadata(&link).is_ok());
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        fx.repo.join("skills/alpha")
    );
    let lock = std::fs::read_to_string(fx.repo.join("agents.lock.toml")).unwrap();
    assert!(lock.contains("source = \"local:skills/alpha\""));
    assert!(lock.contains("upstream = \"local:"));

    fx.assert_success(&["--quiet", "--no-sync", "skills", "remove", "alpha", "-y"]);

    assert!(std::fs::symlink_metadata(fx.codex_skill_path("alpha")).is_err());
}

#[test]
fn subagent_add_auto_commit_includes_canonical_file() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    std::fs::write(fx.repo.join(".gitignore"), ".agents/\n").unwrap();
    git(&["init", "--quiet"], &fx.repo);
    git(&["config", "user.email", "test@example.com"], &fx.repo);
    git(&["config", "user.name", "Test User"], &fx.repo);
    git(&["add", "agents.toml", ".gitignore"], &fx.repo);
    git(&["commit", "--quiet", "-m", "init"], &fx.repo);
    let source = fx.write_subagent_source("reviewer");

    fx.assert_success(&[
        "--quiet",
        "subagents",
        "add",
        source.to_str().unwrap(),
        "-y",
    ]);

    let tracked = git_output(&["ls-files", "agents/reviewer.md"], &fx.repo);
    assert_eq!(
        String::from_utf8_lossy(&tracked.stdout).trim(),
        "agents/reviewer.md"
    );
    let committed = git_output(
        &["show", "--name-only", "--pretty=format:", "HEAD"],
        &fx.repo,
    );
    assert!(
        String::from_utf8_lossy(&committed.stdout).contains("agents/reviewer.md"),
        "subagent canonical file was not committed"
    );
}

#[test]
fn apply_harness_filter_limits_global_instructions() {
    let fx = Fixture::new();
    fx.write_repo_config(&["claude-code", "codex", "opencode", "gemini"]);
    std::fs::write(fx.repo.join("agents.lock.toml"), "").unwrap();
    fx.write_instructions_template("{{#if codex}}CODEX{{/if}}{{#if claude}}CLAUDE{{/if}}\n");

    fx.assert_success(&["--quiet", "--no-sync", "apply", "--harness", "codex"]);

    assert_eq!(
        std::fs::read_to_string(fx.codex_instructions_path()).unwrap(),
        "CODEX\n"
    );
    assert!(!fx.claude_instructions_path().exists());
    assert!(!fx.home.join(".config/opencode/AGENTS.md").exists());
    assert!(!fx.home.join(".gemini/GEMINI.md").exists());
}

#[test]
fn apply_project_filter_preserves_global_instructions() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    std::fs::write(fx.repo.join("agents.lock.toml"), "").unwrap();
    fx.write_instructions_template("GLOBAL\n");

    fx.assert_success(&["--quiet", "--no-sync", "apply"]);
    assert_eq!(
        std::fs::read_to_string(fx.codex_instructions_path()).unwrap(),
        "GLOBAL\n"
    );

    fx.assert_success(&["--quiet", "--no-sync", "apply", "--project", "canva"]);

    assert_eq!(
        std::fs::read_to_string(fx.codex_instructions_path()).unwrap(),
        "GLOBAL\n"
    );
}
