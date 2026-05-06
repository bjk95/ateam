use crate::cli::ImportArgs;
use crate::config::RepoConfig;
use crate::git_sync;
use crate::instructions::{self, Tool};
use crate::lockfile::{InstructionsEntry, Lockfile, SkillEntry};
use crate::manifest::{self, EntryKind, Manifest, ManifestEntry};
use crate::paths;
use crate::source::Source;
use crate::ui;
use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};

pub fn run(args: ImportArgs, no_sync: bool) -> Result<()> {
    let repo = paths::resolve_repo()?;
    let _repo_cfg = RepoConfig::load(&repo)?;

    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    if args.instructions {
        let home = paths::home_dir()?;
        let template = import_instructions(&repo, &home)?;
        if git_sync::enabled(no_sync) {
            let msg = "import :: instructions (CLAUDE.md / AGENTS.md)".to_string();
            let _ = git_sync::commit_and_push(&repo, &msg);
        }
        println!(
            "ateam: imported instructions template → {}",
            template.display()
        );
        println!(
            "edit the template to add Handlebars gates ({}), then `ateam apply` to re-render.",
            "{{#if work}}"
        );
        return Ok(());
    }

    let name = args
        .name
        .as_deref()
        .ok_or_else(|| anyhow!("missing skill name (or pass --instructions)"))?;
    let normalized = crate::lockfile::normalize_skill_name(name)?;

    // Hunt across known agent dirs in $HOME for a directory matching the name.
    let mut found: Option<PathBuf> = None;
    let home = paths::home_dir()?;
    for agent_dir in [
        home.join(".claude").join("skills"),
        home.join(".codex").join("skills"),
        home.join(".agents").join("skills"),
    ] {
        let candidate = agent_dir.join(&normalized);
        if candidate.exists() {
            found = Some(candidate);
            break;
        }
    }
    let installed = found.ok_or_else(|| {
        anyhow!(
            "no installed skill found named `{}` in ~/.claude/skills/, ~/.codex/skills/, or ~/.agents/skills/",
            normalized
        )
    })?;

    // If it's a symlink into our own cache, no-op.
    if let Ok(meta) = std::fs::symlink_metadata(&installed) {
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(&installed)?;
            if target.starts_with(paths::cache_dir(&repo)) || target.starts_with(paths::local_skills_dir(&repo)) {
                ui::ok(format!("{} already managed by ateam", normalized));
                return Ok(());
            }
        }
    }

    let mut lock = Lockfile::load(&repo)?;
    let entry = build_entry(&repo, &normalized, &installed, &args)?;
    let replaced = lock.upsert(entry);
    lock.write(&repo)?;

    if git_sync::enabled(no_sync) {
        let last = lock
            .find(&normalized)
            .map(|e| e.source.clone())
            .unwrap_or_else(|| "unknown".into());
        let msg = git_sync::msg_import(&normalized, &last);
        let _ = git_sync::commit_and_push(&repo, &msg);
    }

    ui::ok(format!(
        "{} {}",
        if replaced { "updated" } else { "imported" },
        normalized
    ));
    ui::plain("  run: ateam apply to materialize");
    if let Some(entry) = lock.find(&normalized) {
        ui::detail(format!("source: {}", entry.source));
    }
    Ok(())
}

/// Import the existing global CLAUDE.md / AGENTS.md as the canonical
/// template. Refuses if a template already exists. On success: writes the
/// template, adds [instructions] to the lockfile, and records manifest
/// ownership of whichever output files exist.
pub(crate) fn import_instructions(repo: &Path, home: &Path) -> Result<PathBuf> {
    let template = paths::instructions_template(repo);
    if template.exists() {
        bail!(
            "instructions template already exists at {} — edit it directly instead of re-importing",
            template.display()
        );
    }

    let claude_path = instructions::output_path(home, Tool::Claude);
    let codex_path = instructions::output_path(home, Tool::Codex);
    let claude = read_optional(&claude_path)?;
    let codex = read_optional(&codex_path)?;

    let canonical = match (claude.as_deref(), codex.as_deref()) {
        (None, None) => bail!(
            "nothing to import — neither {} nor {} exists",
            claude_path.display(),
            codex_path.display()
        ),
        (Some(c), None) => c.to_string(),
        (None, Some(x)) => x.to_string(),
        (Some(c), Some(x)) if c == x => c.to_string(),
        (Some(_), Some(_)) => bail!(
            "{} and {} differ — reconcile (or delete one) before importing.",
            claude_path.display(),
            codex_path.display()
        ),
    };

    if let Some(parent) = template.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&template, &canonical)
        .with_context(|| format!("writing {}", template.display()))?;

    let mut lock = Lockfile::load(repo)?;
    if lock.instructions.is_none() {
        lock.instructions = Some(InstructionsEntry::default());
        lock.write(repo)?;
    }

    let mut mf = Manifest::load(repo)?;
    let now = manifest::now_unix();
    for tool in Tool::all() {
        let path = instructions::output_path(home, tool);
        if !path.exists() {
            continue;
        }
        mf.entries.retain(|e| e.path != path);
        mf.entries.push(ManifestEntry {
            path,
            kind: EntryKind::Copy,
            skill: "_instructions".into(),
            agent: tool.agent().into(),
            target: template.clone(),
            applied_at: now,
        });
    }
    mf.write(repo)?;

    Ok(template)
}

fn read_optional(p: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(p) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow!("reading {}: {}", p.display(), e)),
    }
}

fn build_entry(
    repo: &Path,
    name: &str,
    installed: &Path,
    args: &ImportArgs,
) -> Result<SkillEntry> {
    if let Some(upstream) = &args.upstream {
        let source = Source::parse(upstream)?;
        return Ok(SkillEntry {
            name: name.to_string(),
            source: source.lockfile_string(),
            path: None,
            git_ref: None,
            tree_sha: None,
            agents: vec!["*".into()],
            profiles: vec![],
            project: args.project.clone(),
        });
    }

    // Snapshot: copy the installed dir into <repo>/skills/<name>/ as a local source.
    let dest = paths::local_skills_dir(repo).join(name);
    if dest.exists() {
        bail!("local source already exists at {}", dest.display());
    }
    std::fs::create_dir_all(paths::local_skills_dir(repo))
        .with_context(|| format!("creating {}", paths::local_skills_dir(repo).display()))?;
    crate::install::copy_dir_recursive(installed, &dest)?;
    Ok(SkillEntry {
        name: name.to_string(),
        source: format!("local:skills/{}", name),
        path: Some(format!("skills/{}", name)),
        git_ref: None,
        tree_sha: None,
        agents: vec!["*".into()],
        profiles: vec![],
        project: args.project.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RepoConfig;

    struct Fixture {
        repo: tempfile::TempDir,
        home: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let repo = tempfile::tempdir().unwrap();
            let home = tempfile::tempdir().unwrap();
            RepoConfig::default().write(repo.path()).unwrap();
            std::fs::create_dir_all(repo.path().join(".ateam")).unwrap();
            Self { repo, home }
        }
        fn write_claude(&self, body: &str) {
            std::fs::create_dir_all(self.home.path().join(".claude")).unwrap();
            std::fs::write(self.home.path().join(".claude/CLAUDE.md"), body).unwrap();
        }
        fn write_codex(&self, body: &str) {
            std::fs::create_dir_all(self.home.path().join(".codex")).unwrap();
            std::fs::write(self.home.path().join(".codex/AGENTS.md"), body).unwrap();
        }
        fn template(&self) -> PathBuf {
            paths::instructions_template(self.repo.path())
        }
        fn run(&self) -> Result<PathBuf> {
            import_instructions(self.repo.path(), self.home.path())
        }
    }

    #[test]
    fn imports_when_only_claude_exists() {
        let fx = Fixture::new();
        fx.write_claude("hello\n");
        let template = fx.run().unwrap();
        assert_eq!(std::fs::read_to_string(&template).unwrap(), "hello\n");
        let lock = Lockfile::load(fx.repo.path()).unwrap();
        assert!(lock.instructions.is_some());
        let mf = Manifest::load(fx.repo.path()).unwrap();
        assert_eq!(mf.entries.len(), 1);
        assert_eq!(mf.entries[0].agent, "claude-code");
    }

    #[test]
    fn imports_both_when_identical() {
        let fx = Fixture::new();
        fx.write_claude("same\n");
        fx.write_codex("same\n");
        fx.run().unwrap();
        let mf = Manifest::load(fx.repo.path()).unwrap();
        assert_eq!(mf.entries.len(), 2);
    }

    #[test]
    fn errors_when_neither_exists() {
        let fx = Fixture::new();
        let err = fx.run().unwrap_err();
        assert!(format!("{err}").contains("nothing to import"));
    }

    #[test]
    fn errors_when_files_differ() {
        let fx = Fixture::new();
        fx.write_claude("v1\n");
        fx.write_codex("v2\n");
        let err = fx.run().unwrap_err();
        assert!(format!("{err}").contains("differ"));
    }

    #[test]
    fn refuses_when_template_exists() {
        let fx = Fixture::new();
        fx.write_claude("any\n");
        std::fs::create_dir_all(fx.template().parent().unwrap()).unwrap();
        std::fs::write(fx.template(), "existing template").unwrap();
        let err = fx.run().unwrap_err();
        assert!(format!("{err}").contains("already exists"));
    }
}
