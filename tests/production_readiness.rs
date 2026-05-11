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

    fn codex_config_path(&self) -> PathBuf {
        self.home.join(".codex/config.toml")
    }

    fn claude_config_path(&self) -> PathBuf {
        self.home.join(".claude.json")
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

fn git_stdout(args: &[&str], cwd: &Path) -> String {
    String::from_utf8_lossy(&git_output(args, cwd).stdout).into_owned()
}

fn init_git_repo(repo: &Path) {
    git(&["init", "--quiet", "--initial-branch=main"], repo);
    git(&["config", "user.email", "test@example.com"], repo);
    git(&["config", "user.name", "Test User"], repo);
}

fn write_legacy_copy_manifest(fx: &Fixture, name: &str) {
    let path = fx.codex_skill_path(name);
    let target = fx.repo.join("skills").join(name);
    std::fs::create_dir_all(fx.repo.join(".agents")).unwrap();
    std::fs::write(
        fx.repo.join(".agents/manifest.toml"),
        format!(
            "[[entry]]\npath = \"{}\"\nkind = \"copy\"\nskill = \"{}\"\nharness = \"codex\"\ntarget = \"{}\"\napplied_at = 1\n",
            path.display(),
            name,
            target.display()
        ),
    )
    .unwrap();
}

#[test]
fn root_command_shows_banner() {
    let fx = Fixture::new();

    let output = fx.run(&[]);

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("█████"),
        "root command should print the banner before help\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
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
    assert!(
        std::fs::symlink_metadata(fx.codex_skill_path("alpha"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "skill installs should be symlinks"
    );
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
fn add_replaces_existing_harness_skill_copy_with_symlink() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    let source = fx.write_local_skill_source("alpha");
    let source_parent = source.parent().unwrap().to_str().unwrap();
    let existing = fx.codex_skill_path("alpha");
    std::fs::create_dir_all(&existing).unwrap();
    std::fs::write(existing.join("SKILL.md"), "stale harness copy\n").unwrap();

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

    assert!(
        std::fs::symlink_metadata(&existing)
            .unwrap()
            .file_type()
            .is_symlink(),
        "skill installs should replace existing harness copies with symlinks"
    );
    assert_eq!(
        std::fs::read_link(&existing).unwrap(),
        fx.repo.join("skills/alpha")
    );
}

#[test]
fn apply_replaces_existing_harness_skill_copy_with_symlink() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    fx.write_local_skill_lockfile("alpha");
    let existing = fx.codex_skill_path("alpha");
    std::fs::create_dir_all(&existing).unwrap();
    std::fs::write(existing.join("SKILL.md"), "stale harness copy\n").unwrap();

    fx.assert_success(&["--quiet", "--no-sync", "apply"]);

    assert!(
        std::fs::symlink_metadata(&existing)
            .unwrap()
            .file_type()
            .is_symlink(),
        "apply should replace existing harness copies with symlinks"
    );
    assert_eq!(
        std::fs::read_link(&existing).unwrap(),
        fx.repo.join("skills/alpha")
    );
}

#[test]
fn add_removes_existing_cross_tool_skill_namespace_symlink() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    let source = fx.write_local_skill_source("alpha");
    let source_parent = source.parent().unwrap().to_str().unwrap();
    let namespace_target = fx.home.join("legacy/superpowers/skills");
    let namespace_link = fx.home.join(".agents/skills/superpowers");
    std::fs::create_dir_all(namespace_target.join("alpha")).unwrap();
    std::fs::write(
        namespace_target.join("alpha/SKILL.md"),
        "legacy namespace copy\n",
    )
    .unwrap();
    std::fs::create_dir_all(namespace_link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&namespace_target, &namespace_link).unwrap();

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

    assert!(
        std::fs::symlink_metadata(&namespace_link).is_err(),
        "namespaced cross-tool skill links should be removed"
    );
    assert!(
        namespace_target.join("alpha/SKILL.md").exists(),
        "removing the namespace symlink should not delete its target checkout"
    );
}

#[test]
fn apply_harness_filter_preserves_cross_tool_alias_for_untargeted_skill() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    fx.write_local_skill_lockfile("alpha");
    let namespace_target = fx.home.join("legacy/superpowers/skills");
    let namespace_link = fx.home.join(".agents/skills/superpowers");
    std::fs::create_dir_all(namespace_target.join("alpha")).unwrap();
    std::fs::write(
        namespace_target.join("alpha/SKILL.md"),
        "legacy namespace copy\n",
    )
    .unwrap();
    std::fs::create_dir_all(namespace_link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&namespace_target, &namespace_link).unwrap();

    fx.assert_success(&["--quiet", "--no-sync", "apply", "--harness", "claude-code"]);

    assert!(
        std::fs::symlink_metadata(&namespace_link).is_ok(),
        "harness-filtered apply should not clean aliases for untargeted skills"
    );
}

#[test]
fn deactivate_removes_legacy_copy_install() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    fx.write_local_skill_lockfile("alpha");

    std::fs::create_dir_all(fx.codex_skill_path("alpha")).unwrap();
    std::fs::write(
        fx.codex_skill_path("alpha").join("SKILL.md"),
        "legacy copy\n",
    )
    .unwrap();
    write_legacy_copy_manifest(&fx, "alpha");
    assert!(fx.codex_skill_path("alpha").is_dir());

    fx.assert_success(&["--quiet", "--no-sync", "skills", "deactivate", "alpha"]);

    assert!(!fx.codex_skill_path("alpha").exists());
}

#[test]
fn remove_removes_legacy_copy_install() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    fx.write_local_skill_lockfile("alpha");

    std::fs::create_dir_all(fx.codex_skill_path("alpha")).unwrap();
    std::fs::write(
        fx.codex_skill_path("alpha").join("SKILL.md"),
        "legacy copy\n",
    )
    .unwrap();
    write_legacy_copy_manifest(&fx, "alpha");
    assert!(fx.codex_skill_path("alpha").is_dir());

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

#[test]
fn mcp_add_records_lockfile_and_writes_codex_config() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    std::fs::write(fx.repo.join("agents.lock.toml"), "").unwrap();

    fx.assert_success(&[
        "--quiet",
        "--no-sync",
        "mcp",
        "add",
        "otter",
        "--harness",
        "codex",
        "--profile",
        "canva",
        "--",
        "otter",
        "mcp",
        "serve",
    ]);

    let lock = std::fs::read_to_string(fx.repo.join("agents.lock.toml")).unwrap();
    assert!(lock.contains("[[mcp]]"), "{lock}");
    assert!(lock.contains("name = \"otter\""), "{lock}");
    assert!(lock.contains("profiles = [\"canva\"]"), "{lock}");
    let codex_config = std::fs::read_to_string(fx.codex_config_path()).unwrap();
    assert!(
        codex_config.contains("[mcp_servers.otter]"),
        "{codex_config}"
    );
    let parsed = codex_config.parse::<toml::Value>().unwrap();
    let otter = &parsed["mcp_servers"]["otter"];
    assert_eq!(otter["command"].as_str(), Some("otter"));
    assert_eq!(
        otter["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["mcp", "serve"]
    );
}

#[test]
fn mcp_add_preserves_existing_managed_codex_servers() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    std::fs::write(
        fx.repo.join("agents.lock.toml"),
        r#"
[[mcp]]
name = "context7"
transport = "stdio"
command = "npx"
args = ["-y", "@upstash/context7-mcp"]
harnesses = ["codex"]
"#,
    )
    .unwrap();

    fx.assert_success(&["--quiet", "--no-sync", "apply"]);
    fx.assert_success(&[
        "--quiet",
        "--no-sync",
        "mcp",
        "add",
        "otter",
        "--harness",
        "codex",
        "--",
        "otter",
        "mcp",
        "serve",
    ]);

    let codex_config = std::fs::read_to_string(fx.codex_config_path()).unwrap();
    assert!(
        codex_config.contains("[mcp_servers.context7]"),
        "{codex_config}"
    );
    assert!(
        codex_config.contains("[mcp_servers.otter]"),
        "{codex_config}"
    );
}

#[test]
fn apply_profile_filters_managed_codex_mcps_and_preserves_unmanaged_config() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    std::fs::write(
        fx.repo.join(".agents/machine.toml"),
        "profiles = [\"personal\"]\n",
    )
    .unwrap();
    std::fs::write(
        fx.repo.join("agents.lock.toml"),
        r#"
[[mcp]]
name = "otter"
transport = "stdio"
command = "otter"
args = ["mcp", "serve"]
harnesses = ["codex"]
profiles = ["canva"]

[[mcp]]
name = "supabase"
transport = "http"
url = "https://mcp.supabase.com/mcp"
harnesses = ["codex"]
profiles = ["personal"]
"#,
    )
    .unwrap();
    std::fs::create_dir_all(fx.codex_config_path().parent().unwrap()).unwrap();
    std::fs::write(
        fx.codex_config_path(),
        r#"
model = "gpt-5.3-codex"

[mcp_servers.unmanaged]
command = "keep"

[mcp_servers.otter]
command = "stale"
"#,
    )
    .unwrap();

    fx.assert_success(&["--quiet", "--no-sync", "apply"]);

    let codex_config = std::fs::read_to_string(fx.codex_config_path()).unwrap();
    assert!(
        codex_config.contains("model = \"gpt-5.3-codex\""),
        "{codex_config}"
    );
    assert!(
        codex_config.contains("[mcp_servers.unmanaged]"),
        "{codex_config}"
    );
    assert!(
        !codex_config.contains("[mcp_servers.otter]"),
        "{codex_config}"
    );
    assert!(
        codex_config.contains("[mcp_servers.supabase]"),
        "{codex_config}"
    );
    assert!(
        codex_config.contains("url = \"https://mcp.supabase.com/mcp\""),
        "{codex_config}"
    );
}

#[test]
fn apply_writes_global_claude_mcp_config_and_preserves_unmanaged_entries() {
    let fx = Fixture::new();
    fx.write_repo_config(&["claude-code"]);
    std::fs::write(
        fx.repo.join(".agents/machine.toml"),
        "profiles = [\"work\"]\n",
    )
    .unwrap();
    std::fs::write(
        fx.repo.join("agents.lock.toml"),
        r#"
[[mcp]]
name = "otter"
transport = "stdio"
command = "otter"
args = ["mcp", "serve"]
harnesses = ["claude-code"]
profiles = ["work"]
"#,
    )
    .unwrap();
    std::fs::write(
        fx.claude_config_path(),
        r#"{
  "theme": "dark",
  "mcpServers": {
    "unmanaged": {
      "type": "stdio",
      "command": "keep"
    },
    "otter": {
      "type": "stdio",
      "command": "stale"
    }
  }
}
"#,
    )
    .unwrap();

    fx.assert_success(&["--quiet", "--no-sync", "apply"]);

    let claude_config = std::fs::read_to_string(fx.claude_config_path()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&claude_config).unwrap();
    assert_eq!(parsed["theme"].as_str(), Some("dark"));
    assert_eq!(
        parsed["mcpServers"]["unmanaged"]["command"].as_str(),
        Some("keep")
    );
    assert_eq!(
        parsed["mcpServers"]["otter"]["type"].as_str(),
        Some("stdio")
    );
    assert_eq!(
        parsed["mcpServers"]["otter"]["command"].as_str(),
        Some("otter")
    );
    assert_eq!(
        parsed["mcpServers"]["otter"]["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["mcp", "serve"]
    );
}

#[test]
fn mcp_deactivate_removes_managed_codex_config() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    std::fs::write(
        fx.repo.join("agents.lock.toml"),
        r#"
[[mcp]]
name = "context7"
transport = "stdio"
command = "npx"
args = ["-y", "@upstash/context7-mcp"]
harnesses = ["codex"]
"#,
    )
    .unwrap();

    fx.assert_success(&["--quiet", "--no-sync", "apply"]);
    assert!(std::fs::read_to_string(fx.codex_config_path())
        .unwrap()
        .contains("[mcp_servers.context7]"));

    fx.assert_success(&["--quiet", "--no-sync", "mcp", "deactivate", "context7"]);

    let codex_config = std::fs::read_to_string(fx.codex_config_path()).unwrap();
    assert!(
        !codex_config.contains("[mcp_servers.context7]"),
        "{codex_config}"
    );
    let lock = std::fs::read_to_string(fx.repo.join("agents.lock.toml")).unwrap();
    assert!(lock.contains("active = false"), "{lock}");
}

#[test]
fn apply_removes_codex_mcp_missing_from_lockfile_using_local_state() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    std::fs::write(
        fx.repo.join("agents.lock.toml"),
        r#"
[[mcp]]
name = "context7"
transport = "stdio"
command = "npx"
args = ["-y", "@upstash/context7-mcp"]
harnesses = ["codex"]
"#,
    )
    .unwrap();

    fx.assert_success(&["--quiet", "--no-sync", "apply"]);
    assert!(std::fs::read_to_string(fx.codex_config_path())
        .unwrap()
        .contains("[mcp_servers.context7]"));

    std::fs::write(fx.repo.join("agents.lock.toml"), "").unwrap();
    fx.assert_success(&["--quiet", "--no-sync", "apply"]);

    let codex_config = std::fs::read_to_string(fx.codex_config_path()).unwrap();
    assert!(
        !codex_config.contains("[mcp_servers.context7]"),
        "{codex_config}"
    );
}

#[test]
fn import_default_adopts_existing_mcp_configs() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex", "claude-code"]);
    std::fs::create_dir_all(fx.codex_config_path().parent().unwrap()).unwrap();
    std::fs::write(
        fx.codex_config_path(),
        r#"
[mcp_servers.context7]
command = "npx"
args = ["-y", "@upstash/context7-mcp"]
"#,
    )
    .unwrap();
    std::fs::write(
        fx.claude_config_path(),
        r#"{
  "mcpServers": {
    "otter": {
      "type": "stdio",
      "command": "otter",
      "args": ["mcp", "serve"]
    }
  }
}
"#,
    )
    .unwrap();

    fx.assert_success(&["--quiet", "--no-sync", "import"]);

    let lock = std::fs::read_to_string(fx.repo.join("agents.lock.toml")).unwrap();
    assert!(lock.contains("[[mcp]]"), "{lock}");
    assert!(lock.contains("name = \"context7\""), "{lock}");
    assert!(lock.contains("name = \"otter\""), "{lock}");
    let manifest = std::fs::read_to_string(fx.repo.join(".agents/mcp-manifest.toml")).unwrap();
    assert!(manifest.contains("context7"), "{manifest}");
    assert!(manifest.contains("otter"), "{manifest}");
}

#[test]
fn import_mcp_only_skips_skills_and_instructions() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    fx.write_instructions_template("template should not be replaced\n");
    let skill_dir = fx.home.join(".codex/skills/alpha");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: alpha\ndescription: Alpha skill.\n---\nbody\n",
    )
    .unwrap();
    std::fs::create_dir_all(fx.codex_config_path().parent().unwrap()).unwrap();
    std::fs::write(
        fx.codex_config_path(),
        r#"
[mcp_servers.context7]
command = "npx"
args = ["-y", "@upstash/context7-mcp"]
"#,
    )
    .unwrap();

    fx.assert_success(&["--quiet", "--no-sync", "import", "--mcp"]);

    let lock = std::fs::read_to_string(fx.repo.join("agents.lock.toml")).unwrap();
    assert!(lock.contains("[[mcp]]"), "{lock}");
    assert!(lock.contains("name = \"context7\""), "{lock}");
    assert!(!lock.contains("[[skill]]"), "{lock}");
    assert_eq!(
        std::fs::read_to_string(fx.repo.join("instructions/instructions.md.hbs")).unwrap(),
        "template should not be replaced\n"
    );
}

#[test]
fn auto_sync_commits_when_optional_dirs_are_missing() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    fx.write_local_skill_lockfile("alpha");
    std::fs::write(fx.repo.join(".gitignore"), ".agents/\n").unwrap();
    init_git_repo(&fx.repo);
    git(
        &[
            "add",
            "agents.toml",
            "agents.lock.toml",
            ".gitignore",
            "skills",
        ],
        &fx.repo,
    );
    git(&["commit", "--quiet", "-m", "initial"], &fx.repo);

    fx.assert_success(&["--quiet", "skills", "deactivate", "alpha"]);

    let subject = git_stdout(&["log", "-1", "--pretty=%s"], &fx.repo);
    assert_eq!(subject.trim(), "deactivate :: alpha");
    assert_eq!(git_stdout(&["status", "--short"], &fx.repo), "");
}

#[test]
fn subagent_add_auto_sync_stages_snapshot_and_rendered_file() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    std::fs::write(fx.repo.join("agents.lock.toml"), "# agents lockfile\n").unwrap();
    std::fs::write(fx.repo.join(".gitignore"), ".agents/\n").unwrap();
    let source = fx.repo.parent().unwrap().join("source-demo.md");
    std::fs::write(
        &source,
        "---\nname: demo\ndescription: Demo subagent.\n---\n\nBody.\n",
    )
    .unwrap();
    init_git_repo(&fx.repo);
    git(
        &["add", "agents.toml", "agents.lock.toml", ".gitignore"],
        &fx.repo,
    );
    git(&["commit", "--quiet", "-m", "initial"], &fx.repo);

    fx.assert_success(&[
        "--quiet",
        "subagents",
        "add",
        source.to_str().unwrap(),
        "--subagent",
        "demo",
        "-a",
        "codex",
        "-y",
    ]);

    let files = git_stdout(&["show", "--name-only", "--format=", "HEAD"], &fx.repo);
    assert!(files.contains("agents.lock.toml"), "{files}");
    assert!(files.contains("agents/demo.md"), "{files}");
    assert!(files.contains("agents/rendered/codex/demo.toml"), "{files}");
    let installed = fx.home.join(".codex/agents/demo.toml");
    assert!(
        std::fs::symlink_metadata(&installed)
            .unwrap()
            .file_type()
            .is_symlink(),
        "subagent installs should be symlinks"
    );
    assert_eq!(
        std::fs::read_link(installed).unwrap(),
        fx.repo.join("agents/rendered/codex/demo.toml")
    );
    assert_eq!(git_stdout(&["status", "--short"], &fx.repo), "");
}

#[test]
fn apply_auto_sync_stages_rendered_instructions() {
    let fx = Fixture::new();
    fx.write_repo_config(&["codex"]);
    std::fs::create_dir_all(fx.repo.join("instructions")).unwrap();
    std::fs::write(
        fx.repo.join("instructions/instructions.md.hbs"),
        "hello codex\n",
    )
    .unwrap();
    std::fs::write(
        fx.repo.join("agents.lock.toml"),
        "[instructions]\nharnesses = [\"codex\"]\n",
    )
    .unwrap();
    std::fs::write(fx.repo.join(".gitignore"), ".agents/\n").unwrap();
    init_git_repo(&fx.repo);
    git(
        &[
            "add",
            "agents.toml",
            "agents.lock.toml",
            ".gitignore",
            "instructions/instructions.md.hbs",
        ],
        &fx.repo,
    );
    git(&["commit", "--quiet", "-m", "initial"], &fx.repo);

    fx.assert_success(&["--quiet", "apply"]);

    let files = git_stdout(&["show", "--name-only", "--format=", "HEAD"], &fx.repo);
    assert!(
        files.contains("instructions/rendered/codex/AGENTS.md"),
        "{files}"
    );
    let installed = fx.home.join(".codex/AGENTS.md");
    assert!(
        std::fs::symlink_metadata(&installed)
            .unwrap()
            .file_type()
            .is_symlink(),
        "instruction installs should be symlinks"
    );
    assert_eq!(
        std::fs::read_link(installed).unwrap(),
        fx.repo.join("instructions/rendered/codex/AGENTS.md")
    );
    assert_eq!(git_stdout(&["status", "--short"], &fx.repo), "");
}
