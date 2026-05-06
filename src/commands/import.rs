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
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub fn run(args: ImportArgs, no_sync: bool) -> Result<()> {
    let repo = paths::resolve_repo()?;
    let _repo_cfg = RepoConfig::load(&repo)?;

    if args.instructions && args.name.is_some() {
        bail!("`--instructions` is mutually exclusive with a skill name");
    }

    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    let home = paths::home_dir()?;

    if args.instructions {
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

    if args.name.is_none() {
        return run_bulk(&repo, &home, no_sync);
    }

    run_single(&repo, &home, &args, no_sync)
}

// ---------------------------------------------------------------------------
// Single-skill import (the original behavior)

fn run_single(repo: &Path, home: &Path, args: &ImportArgs, no_sync: bool) -> Result<()> {
    let name = args.name.as_deref().unwrap();
    let normalized = crate::lockfile::normalize_skill_name(name)?;

    let installed = find_installed(home, &normalized).ok_or_else(|| {
        anyhow!(
            "no installed skill found named `{}` in ~/.claude/skills/, ~/.codex/skills/, or ~/.agents/skills/",
            normalized
        )
    })?;

    if is_managed_by_ateam(repo, &installed)? {
        ui::ok(format!("{} already managed by ateam", normalized));
        return Ok(());
    }

    let mut lock = Lockfile::load(repo)?;
    let entry = build_entry(repo, &normalized, &installed, args)?;
    let replaced = lock.upsert(entry);
    lock.write(repo)?;

    if git_sync::enabled(no_sync) {
        let last = lock
            .find(&normalized)
            .map(|e| e.source.clone())
            .unwrap_or_else(|| "unknown".into());
        let msg = git_sync::msg_import(&normalized, &last);
        let _ = git_sync::commit_and_push(repo, &msg);
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

// ---------------------------------------------------------------------------
// Bulk import: scoop everything in ~/.claude/skills, ~/.codex/skills, ~/.agents/skills,
// plus the global instructions, into the lockfile.

fn run_bulk(repo: &Path, home: &Path, no_sync: bool) -> Result<()> {
    println!("ateam: scanning ~/.claude/skills, ~/.codex/skills, ~/.agents/skills...");

    let mut lock = Lockfile::load(repo)?;
    let outcome = bulk_import_skills(repo, home, &mut lock)?;

    if outcome.imported > 0 || !outcome.errors.is_empty() {
        lock.write(repo)?;
    }

    let instructions_template = match import_instructions(repo, home) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("ateam: instructions skipped — {e:#}");
            None
        }
    };

    println!();
    println!(
        "ateam: imported {} skill(s); skipped {} already managed",
        outcome.imported, outcome.skipped_managed
    );
    if !outcome.errors.is_empty() {
        println!("  errors:");
        for (name, err) in &outcome.errors {
            println!("    - {name}: {err}");
        }
    }
    if let Some(p) = &instructions_template {
        println!("  instructions template → {}", p.display());
    }
    if outcome.imported > 0 || instructions_template.is_some() {
        println!();
        println!("run `ateam apply` to materialize symlinks for the new entries.");
    }

    if git_sync::enabled(no_sync) && (outcome.imported > 0 || instructions_template.is_some()) {
        let msg = format!(
            "import :: bulk ({} skill(s){})",
            outcome.imported,
            if instructions_template.is_some() {
                " + instructions"
            } else {
                ""
            }
        );
        let _ = git_sync::commit_and_push(repo, &msg);
    }

    Ok(())
}

#[derive(Debug, Default)]
pub(crate) struct BulkOutcome {
    pub imported: usize,
    pub skipped_managed: usize,
    pub errors: Vec<(String, String)>,
}

pub(crate) fn bulk_import_skills(
    repo: &Path,
    home: &Path,
    lock: &mut Lockfile,
) -> Result<BulkOutcome> {
    let mut outcome = BulkOutcome::default();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for dir in agent_skill_dirs(home) {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => bail!("reading {}: {e}", dir.display()),
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if !ft.is_dir() && !ft.is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if !seen.insert(name.clone()) {
                continue;
            }
            let installed = entry.path();

            if is_managed_by_ateam(repo, &installed)? {
                outcome.skipped_managed += 1;
                continue;
            }
            if lock.find(&name).is_some() {
                outcome.skipped_managed += 1;
                continue;
            }

            let dest = paths::local_skills_dir(repo).join(&name);
            let already_snapshotted = dest.exists();

            if !already_snapshotted {
                if let Err(e) = std::fs::create_dir_all(paths::local_skills_dir(repo)) {
                    outcome.errors.push((name.clone(), format!("{e:#}")));
                    continue;
                }
                // Resolve symlinks before snapshotting so we copy real content.
                let src = std::fs::canonicalize(&installed).unwrap_or_else(|_| installed.clone());
                if !src.is_dir() {
                    outcome
                        .errors
                        .push((name.clone(), format!("{} is not a directory", src.display())));
                    continue;
                }
                if let Err(e) = crate::install::copy_dir_recursive(&src, &dest) {
                    outcome.errors.push((name.clone(), format!("{e:#}")));
                    continue;
                }
            }

            lock.upsert(SkillEntry {
                name: name.clone(),
                source: format!("local:skills/{}", name),
                path: Some(format!("skills/{}", name)),
                git_ref: None,
                tree_sha: None,
                agents: vec!["*".into()],
                profiles: vec![],
                project: None,
            });
            outcome.imported += 1;
            if already_snapshotted {
                println!("  + {name} (adopted existing snapshot)");
            } else {
                println!("  + {name}");
            }
        }
    }
    Ok(outcome)
}

fn agent_skill_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".claude").join("skills"),
        home.join(".codex").join("skills"),
        home.join(".agents").join("skills"),
    ]
}

fn find_installed(home: &Path, normalized: &str) -> Option<PathBuf> {
    for dir in agent_skill_dirs(home) {
        let candidate = dir.join(normalized);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn is_managed_by_ateam(repo: &Path, installed: &Path) -> Result<bool> {
    let Ok(meta) = std::fs::symlink_metadata(installed) else {
        return Ok(false);
    };
    if !meta.file_type().is_symlink() {
        return Ok(false);
    }
    let target = std::fs::read_link(installed)
        .with_context(|| format!("reading symlink {}", installed.display()))?;
    Ok(target.starts_with(paths::cache_dir(repo))
        || target.starts_with(paths::local_skills_dir(repo)))
}

// ---------------------------------------------------------------------------
// Instructions import (shared by --instructions and bulk).

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

    let canonical = pick_canonical(&claude_path, claude.as_deref(), &codex_path, codex.as_deref())?;

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

fn pick_canonical(
    claude_path: &Path,
    claude: Option<&str>,
    codex_path: &Path,
    codex: Option<&str>,
) -> Result<String> {
    match (claude, codex) {
        (None, None) => bail!(
            "nothing to import — neither {} nor {} exists",
            claude_path.display(),
            codex_path.display()
        ),
        (Some(c), None) => Ok(c.to_string()),
        (None, Some(x)) => Ok(x.to_string()),
        (Some(c), Some(x)) if c == x => Ok(c.to_string()),
        (Some(c), Some(x)) => prompt_pick(claude_path, c, codex_path, x),
    }
}

fn prompt_pick(
    claude_path: &Path,
    claude: &str,
    codex_path: &Path,
    codex: &str,
) -> Result<String> {
    use dialoguer::{theme::ColorfulTheme, Select};
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        bail!(
            "{} and {} differ — reconcile (or delete one) before importing.",
            claude_path.display(),
            codex_path.display()
        );
    }
    let items = [
        format!("Claude  — {} ({} bytes)", claude_path.display(), claude.len()),
        format!("Codex   — {} ({} bytes)", codex_path.display(), codex.len()),
    ];
    let choice = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("CLAUDE.md and AGENTS.md differ. Which one should be the canonical template?")
        .items(&items)
        .default(0)
        .interact()?;
    Ok(if choice == 0 {
        claude.to_string()
    } else {
        codex.to_string()
    })
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
    let src = std::fs::canonicalize(installed).unwrap_or_else(|_| installed.to_path_buf());
    crate::install::copy_dir_recursive(&src, &dest)?;
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
        fn write_skill(&self, agent: &str, name: &str, body: &str) {
            let dir = self
                .home
                .path()
                .join(format!(".{}", agent))
                .join("skills")
                .join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("SKILL.md"), body).unwrap();
        }
        fn run_instructions(&self) -> Result<PathBuf> {
            import_instructions(self.repo.path(), self.home.path())
        }
    }

    #[test]
    fn imports_when_only_claude_exists() {
        let fx = Fixture::new();
        fx.write_claude("hello\n");
        let template = fx.run_instructions().unwrap();
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
        fx.run_instructions().unwrap();
        let mf = Manifest::load(fx.repo.path()).unwrap();
        assert_eq!(mf.entries.len(), 2);
    }

    #[test]
    fn errors_when_neither_exists() {
        let fx = Fixture::new();
        let err = fx.run_instructions().unwrap_err();
        assert!(format!("{err}").contains("nothing to import"));
    }

    #[test]
    fn non_interactive_diff_still_errors() {
        // cargo test runs without a tty → prompt_pick falls through to error.
        let fx = Fixture::new();
        fx.write_claude("v1\n");
        fx.write_codex("v2\n");
        let err = fx.run_instructions().unwrap_err();
        assert!(format!("{err}").contains("differ"));
    }

    #[test]
    fn refuses_when_template_exists() {
        let fx = Fixture::new();
        fx.write_claude("any\n");
        std::fs::create_dir_all(fx.template().parent().unwrap()).unwrap();
        std::fs::write(fx.template(), "existing template").unwrap();
        let err = fx.run_instructions().unwrap_err();
        assert!(format!("{err}").contains("already exists"));
    }

    #[test]
    fn bulk_imports_skills_from_both_dirs() {
        let fx = Fixture::new();
        fx.write_skill("claude", "alpha", "alpha body");
        fx.write_skill("codex", "beta", "beta body");
        // Same skill name in both — should dedupe (claude wins, scanned first).
        fx.write_skill("claude", "shared", "claude version");
        fx.write_skill("codex", "shared", "codex version");

        let mut lock = Lockfile::load(fx.repo.path()).unwrap();
        let outcome = bulk_import_skills(fx.repo.path(), fx.home.path(), &mut lock).unwrap();

        assert_eq!(outcome.imported, 3, "expected alpha, beta, shared");
        assert_eq!(outcome.skipped_managed, 0);
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);

        let names: Vec<_> = lock.skills.iter().map(|s| s.name.clone()).collect();
        assert!(names.contains(&"alpha".to_string()));
        assert!(names.contains(&"beta".to_string()));
        assert!(names.contains(&"shared".to_string()));

        // Snapshot of `shared` is the claude version (first seen).
        let shared_body = std::fs::read_to_string(
            fx.repo.path().join("skills/shared/SKILL.md"),
        )
        .unwrap();
        assert_eq!(shared_body, "claude version");
    }

    #[test]
    fn bulk_skips_already_in_lockfile() {
        let fx = Fixture::new();
        fx.write_skill("claude", "alpha", "body");
        let mut lock = Lockfile::load(fx.repo.path()).unwrap();
        lock.skills.push(SkillEntry {
            name: "alpha".into(),
            source: "github:foo/bar".into(),
            path: Some("skills/alpha".into()),
            git_ref: None,
            tree_sha: None,
            agents: vec!["*".into()],
            profiles: vec![],
            project: None,
        });
        let outcome = bulk_import_skills(fx.repo.path(), fx.home.path(), &mut lock).unwrap();
        assert_eq!(outcome.imported, 0);
        assert_eq!(outcome.skipped_managed, 1);
    }

    #[test]
    fn bulk_adopts_orphan_snapshot_dirs() {
        // Simulate a partial earlier import: dir exists in <repo>/skills/<name>/
        // but lockfile has no entry for it. Re-running import should adopt it.
        let fx = Fixture::new();
        fx.write_skill("claude", "alpha", "fresh body");
        let dest = paths::local_skills_dir(fx.repo.path()).join("alpha");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("SKILL.md"), "stale orphan body").unwrap();

        let mut lock = Lockfile::load(fx.repo.path()).unwrap();
        assert!(lock.find("alpha").is_none());
        let outcome = bulk_import_skills(fx.repo.path(), fx.home.path(), &mut lock).unwrap();

        assert_eq!(outcome.imported, 1, "orphan should be adopted");
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        assert!(lock.find("alpha").is_some());
        // Adoption preserves the existing snapshot — does NOT clobber with fresh body.
        let body = std::fs::read_to_string(dest.join("SKILL.md")).unwrap();
        assert_eq!(body, "stale orphan body");
    }

    #[test]
    fn bulk_skips_symlinks_into_ateam_cache() {
        let fx = Fixture::new();
        // Pretend a skill is already an ateam-managed symlink.
        let cache_target = paths::cache_dir(fx.repo.path()).join("alpha");
        std::fs::create_dir_all(&cache_target).unwrap();
        std::fs::write(cache_target.join("SKILL.md"), "cached").unwrap();
        let claude_skills = fx.home.path().join(".claude/skills");
        std::fs::create_dir_all(&claude_skills).unwrap();
        std::os::unix::fs::symlink(&cache_target, claude_skills.join("alpha")).unwrap();

        let mut lock = Lockfile::load(fx.repo.path()).unwrap();
        let outcome = bulk_import_skills(fx.repo.path(), fx.home.path(), &mut lock).unwrap();
        assert_eq!(outcome.imported, 0);
        assert_eq!(outcome.skipped_managed, 1);
    }
}
