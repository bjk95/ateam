use crate::cli::ImportArgs;
use crate::config::RepoConfig;
use crate::git_sync;
use crate::instructions::{self, Harness};
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
            if let Err(e) = git_sync::commit_and_push(&repo, &msg) {
                ui::warn(format!("auto-sync failed: {:#}", e));
                ui::detail("local change saved; rerun a mutating command to retry");
            }
        }
        ui::plain(format!(
            "agents: imported instructions template → {}",
            template.display()
        ));
        ui::plain(format!(
            "edit the template to add Handlebars gates ({}), then `agents apply` to re-render.",
            "{{#if work}}"
        ));
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
    let normalized =
        crate::discover::standard_skill_name(&crate::lockfile::normalize_skill_name(name)?);

    let installed = find_installed(home, &normalized).ok_or_else(|| {
        anyhow!(
            "no installed skill found named `{}` in {}",
            normalized,
            crate::discover::harness_skill_dirs(home)
                .iter()
                .map(|p| crate::paths::display_path(p))
                .collect::<Vec<_>>()
                .join(", "),
        )
    })?;

    if is_managed_by_agents(repo, &installed)? {
        ui::ok(format!("{} already managed by agents", normalized));
        return Ok(());
    }

    if args.upstream.is_none() {
        let upstream_index = crate::upstream::build_index(home);
        if let Some(plugin_source) = upstream_index.get(&normalized) {
            bail!(
                "{} is plugin-managed by {} — agents won't take ownership. Manage it via `claude plugin` commands.",
                normalized,
                plugin_source
            );
        }
    }

    validate_installed_skill(&normalized, &installed)?;

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
        if let Err(e) = git_sync::commit_and_push(repo, &msg) {
            ui::warn(format!("auto-sync failed: {:#}", e));
            ui::detail("local change saved; rerun a mutating command to retry");
        }
    }

    ui::ok(format!(
        "{} {}",
        if replaced { "updated" } else { "imported" },
        normalized
    ));
    ui::plain("  run: agents apply to materialize");
    if let Some(entry) = lock.find(&normalized) {
        ui::detail(format!("source: {}", entry.source));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Bulk import: scoop everything in ~/.claude/skills, ~/.codex/skills, ~/.agents/skills,
// plus the global instructions, into the lockfile.

fn run_bulk(repo: &Path, home: &Path, no_sync: bool) -> Result<()> {
    ui::plain(format!(
        "agents: scanning {}...",
        crate::discover::harness_skill_dirs(home)
            .iter()
            .map(|p| crate::paths::display_path(p))
            .collect::<Vec<_>>()
            .join(", "),
    ));

    let mut lock = Lockfile::load(repo)?;
    let outcome = bulk_import_skills(repo, home, &mut lock)?;

    if outcome.imported > 0 || !outcome.errors.is_empty() {
        lock.write(repo)?;
    }

    let instructions_template = match import_instructions(repo, home) {
        Ok(p) => Some(p),
        Err(e) => {
            ui::warn(format!("instructions skipped — {e:#}"));
            None
        }
    };

    ui::plain("");
    ui::plain(format!(
        "agents: imported {} skill(s); skipped {} already managed",
        outcome.imported, outcome.skipped_managed
    ));
    if outcome.skipped_plugin > 0 {
        ui::plain(format!(
            "  + skipped {} plugin-managed (manage via `claude plugin`)",
            outcome.skipped_plugin
        ));
    }
    if outcome.discovered_upstream > 0 {
        ui::plain(format!(
            "  + discovered upstream for {} existing entr{}",
            outcome.discovered_upstream,
            if outcome.discovered_upstream == 1 {
                "y"
            } else {
                "ies"
            }
        ));
    }
    if !outcome.errors.is_empty() {
        ui::plain("  errors:");
        for (name, err) in &outcome.errors {
            ui::plain(format!("    - {name}: {err}"));
        }
    }
    if let Some(p) = &instructions_template {
        ui::plain(format!("  instructions template → {}", p.display()));
    }
    if outcome.imported > 0 || instructions_template.is_some() {
        ui::plain("");
        ui::plain("run `agents apply` to materialize symlinks for the new entries.");
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
        if let Err(e) = git_sync::commit_and_push(repo, &msg) {
            ui::warn(format!("auto-sync failed: {:#}", e));
            ui::detail("local change saved; rerun a mutating command to retry");
        }
    }

    Ok(())
}

#[derive(Debug, Default)]
pub(crate) struct BulkOutcome {
    pub imported: usize,
    pub skipped_managed: usize,
    pub skipped_plugin: usize,
    pub discovered_upstream: usize,
    pub errors: Vec<(String, String)>,
}

pub(crate) fn bulk_import_skills(
    repo: &Path,
    home: &Path,
    lock: &mut Lockfile,
) -> Result<BulkOutcome> {
    let mut outcome = BulkOutcome::default();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let upstream_index = crate::upstream::build_index(home);

    for dir in crate::discover::harness_skill_dirs(home) {
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
            let raw_name = entry.file_name().to_string_lossy().into_owned();
            if raw_name.starts_with('.') {
                continue;
            }
            if !entry.path().join("SKILL.md").is_file() {
                continue;
            }
            let name = match crate::lockfile::normalize_skill_name(&raw_name) {
                Ok(name) => crate::discover::standard_skill_name(&name),
                Err(e) => {
                    outcome.errors.push((raw_name, format!("{e:#}")));
                    continue;
                }
            };
            if !seen.insert(name.clone()) {
                continue;
            }
            let installed = entry.path();

            if is_managed_by_agents(repo, &installed)? {
                outcome.skipped_managed += 1;
                continue;
            }
            if lock.find(&name).is_some() {
                outcome.skipped_managed += 1;
                continue;
            }
            if let Some(plugin_source) = upstream_index
                .get(&name)
                .or_else(|| upstream_index.get(&raw_name))
            {
                outcome.skipped_plugin += 1;
                ui::plain(format!("  · {name} (plugin-managed by {plugin_source})"));
                continue;
            }
            if crate::discover::parse_skill_dir(&installed)
                .with_context(|| format!("validating {}", installed.join("SKILL.md").display()))?
                .is_none()
            {
                outcome.errors.push((
                    name.clone(),
                    format!(
                        "missing a valid SKILL.md at {} (Agent Skills standard requires YAML frontmatter with a non-empty `description`; add a description and retry)",
                        installed.join("SKILL.md").display()
                    ),
                ));
                continue;
            }

            let dest = paths::local_skills_dir(repo).join(&name);
            let already_snapshotted = dest.exists();

            if !already_snapshotted {
                let step = ui::step(format!("snapshotting {}", name));
                if let Err(e) = std::fs::create_dir_all(paths::local_skills_dir(repo)) {
                    step.fail(format!("import {} failed", name));
                    outcome.errors.push((name.clone(), format!("{e:#}")));
                    continue;
                }
                // Resolve symlinks before snapshotting so we copy real content.
                let src = std::fs::canonicalize(&installed).unwrap_or_else(|_| installed.clone());
                if !src.is_dir() {
                    step.fail(format!("import {} failed", name));
                    outcome.errors.push((
                        name.clone(),
                        format!("{} is not a directory", src.display()),
                    ));
                    continue;
                }
                if let Err(e) = crate::install::copy_dir_recursive(&src, &dest) {
                    step.fail(format!("import {} failed", name));
                    outcome.errors.push((name.clone(), format!("{e:#}")));
                    continue;
                }
                step.ok(format!("snapshotted {}", name));
            }
            match crate::discover::canonicalize_skill_dir(&dest, &name) {
                Ok(Some(repair)) => {
                    for diagnostic in repair.diagnostics {
                        ui::warn(format!("repaired {}: {}", name, diagnostic));
                    }
                }
                Ok(None) => {
                    outcome.errors.push((
                        name.clone(),
                        format!(
                            "missing a valid SKILL.md at {} (Agent Skills standard requires YAML frontmatter with a non-empty `description`; add a description and retry)",
                            dest.join("SKILL.md").display()
                        ),
                    ));
                    continue;
                }
                Err(e) => {
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
                harnesses: vec!["*".into()],
                profiles: vec![],
                project: None,
                active: true,
                upstream: upstream_index
                    .get(&name)
                    .or_else(|| upstream_index.get(&raw_name))
                    .cloned(),
            });
            outcome.imported += 1;
            if already_snapshotted {
                ui::plain(format!("  + {name} (adopted existing snapshot)"));
            } else {
                ui::plain(format!("  + {name}"));
            }
        }
    }

    // Backfill: re-discover upstream for any local entry that doesn't have one.
    // Lets the user re-run `agents import` to pick up upstream info that
    // wasn't being recorded when they first imported.
    for entry in lock.skills.iter_mut() {
        if entry.upstream.is_none() && entry.source.starts_with("local:") {
            if let Some(up) = upstream_index.get(&entry.name) {
                entry.upstream = Some(up.clone());
                outcome.discovered_upstream += 1;
            }
        }
    }

    Ok(outcome)
}

fn find_installed(home: &Path, normalized: &str) -> Option<PathBuf> {
    for dir in crate::discover::harness_skill_dirs(home) {
        let candidate = dir.join(normalized);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn validate_installed_skill(name: &str, installed: &Path) -> Result<()> {
    let skill_md = installed.join("SKILL.md");
    if crate::discover::parse_skill_dir(installed)
        .with_context(|| format!("validating {}", skill_md.display()))?
        .is_none()
    {
        bail!(
            "installed skill `{}` is missing a valid SKILL.md at {} (Agent Skills standard requires YAML frontmatter with a non-empty `description`; add a description and retry)",
            name,
            skill_md.display()
        );
    }
    Ok(())
}

fn is_managed_by_agents(repo: &Path, installed: &Path) -> Result<bool> {
    let Ok(meta) = std::fs::symlink_metadata(installed) else {
        return Ok(false);
    };
    if !meta.file_type().is_symlink() {
        return Ok(false);
    }
    let target = std::fs::read_link(installed)
        .with_context(|| format!("reading symlink {}", installed.display()))?;
    Ok(target.starts_with(paths::local_skills_dir(repo)))
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

    let claude_path = instructions::output_path(home, Harness::CLAUDE);
    let codex_path = instructions::output_path(home, Harness::CODEX);
    let claude = read_optional(&claude_path)?;
    let codex = read_optional(&codex_path)?;

    let canonical = pick_canonical(
        &claude_path,
        claude.as_deref(),
        &codex_path,
        codex.as_deref(),
    )?;

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
    for harness in Harness::all() {
        let path = instructions::output_path(home, harness);
        if !path.exists() {
            continue;
        }
        mf.entries.retain(|e| e.path != path);
        mf.entries.push(ManifestEntry {
            path,
            kind: EntryKind::Copy,
            skill: "_instructions".into(),
            harness: harness.id().into(),
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

fn prompt_pick(claude_path: &Path, claude: &str, codex_path: &Path, codex: &str) -> Result<String> {
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
        format!(
            "Claude  — {} ({} bytes)",
            claude_path.display(),
            claude.len()
        ),
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

fn build_entry(repo: &Path, name: &str, installed: &Path, args: &ImportArgs) -> Result<SkillEntry> {
    if let Some(upstream) = &args.upstream {
        let source = Source::parse(upstream)?;
        return Ok(SkillEntry {
            name: name.to_string(),
            source: source.lockfile_string(),
            path: None,
            git_ref: None,
            tree_sha: None,
            harnesses: vec!["*".into()],
            profiles: vec![],
            project: args.project.clone(),
            active: true,
            upstream: None,
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
    if let Some(repair) = crate::discover::canonicalize_skill_dir(&dest, name)? {
        for diagnostic in repair.diagnostics {
            ui::warn(format!("repaired {}: {}", name, diagnostic));
        }
    }
    Ok(SkillEntry {
        name: name.to_string(),
        source: format!("local:skills/{}", name),
        path: Some(format!("skills/{}", name)),
        git_ref: None,
        tree_sha: None,
        harnesses: vec!["*".into()],
        profiles: vec![],
        project: args.project.clone(),
        active: true,
        upstream: None,
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
            std::fs::create_dir_all(repo.path().join(".agents")).unwrap();
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
        fn write_valid_skill(&self, agent: &str, name: &str, body: &str) {
            self.write_skill(
                agent,
                name,
                &format!("---\nname: {name}\ndescription: {name} skill.\n---\n{body}"),
            );
        }
        /// Stage a Claude marketplace plugin so `upstream::build_index` will
        /// classify `skill_name` as plugin-managed.
        fn write_plugin_skill(
            &self,
            plugin: &str,
            marketplace: &str,
            repo: &str,
            skill_name: &str,
        ) {
            let plugins = self.home.path().join(".claude/plugins");
            let install_path = plugins
                .join("cache")
                .join(marketplace)
                .join(plugin)
                .join("1.0.0");
            std::fs::create_dir_all(install_path.join("skills").join(skill_name)).unwrap();
            std::fs::create_dir_all(&plugins).unwrap();
            std::fs::write(
                plugins.join("installed_plugins.json"),
                format!(
                    r#"{{"version":2,"plugins":{{"{plugin}@{marketplace}":[{{"installPath":"{}"}}]}}}}"#,
                    install_path.display()
                ),
            )
            .unwrap();
            std::fs::write(
                plugins.join("known_marketplaces.json"),
                format!(
                    r#"{{"{marketplace}":{{"source":{{"source":"github","repo":"{repo}"}}}}}}"#
                ),
            )
            .unwrap();
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
        assert_eq!(mf.entries[0].harness, "claude-code");
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
        fx.write_valid_skill("claude", "alpha", "alpha body");
        fx.write_valid_skill("codex", "beta", "beta body");
        // Same skill name in both — should dedupe (claude wins, scanned first).
        fx.write_valid_skill("claude", "shared", "claude version");
        fx.write_valid_skill("codex", "shared", "codex version");

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
        let shared_body =
            std::fs::read_to_string(fx.repo.path().join("skills/shared/SKILL.md")).unwrap();
        assert!(shared_body.contains("claude version"), "{shared_body}");
    }

    #[test]
    fn bulk_skips_already_in_lockfile() {
        let fx = Fixture::new();
        fx.write_valid_skill("claude", "alpha", "body");
        let mut lock = Lockfile::load(fx.repo.path()).unwrap();
        lock.skills.push(SkillEntry {
            name: "alpha".into(),
            source: "github:foo/bar".into(),
            path: Some("skills/alpha".into()),
            git_ref: None,
            tree_sha: None,
            harnesses: vec!["*".into()],
            profiles: vec![],
            project: None,
            active: true,
            upstream: None,
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
        fx.write_valid_skill("claude", "alpha", "fresh body");
        let dest = paths::local_skills_dir(fx.repo.path()).join("alpha");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(
            dest.join("SKILL.md"),
            "---\nname: alpha\ndescription: Alpha stale skill.\n---\nstale orphan body",
        )
        .unwrap();

        let mut lock = Lockfile::load(fx.repo.path()).unwrap();
        assert!(lock.find("alpha").is_none());
        let outcome = bulk_import_skills(fx.repo.path(), fx.home.path(), &mut lock).unwrap();

        assert_eq!(outcome.imported, 1, "orphan should be adopted");
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        assert!(lock.find("alpha").is_some());
        // Adoption preserves the existing snapshot — does NOT clobber with fresh body.
        let body = std::fs::read_to_string(dest.join("SKILL.md")).unwrap();
        assert!(body.contains("stale orphan body"), "{body}");
    }

    #[test]
    fn bulk_skips_container_dirs_without_top_level_skill_md() {
        let fx = Fixture::new();
        let container = fx.home.path().join(".codex/skills/superpowers");
        let nested = container.join("brainstorming");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("SKILL.md"), "nested body").unwrap();

        let mut lock = Lockfile::load(fx.repo.path()).unwrap();
        let outcome = bulk_import_skills(fx.repo.path(), fx.home.path(), &mut lock).unwrap();

        assert_eq!(outcome.imported, 0);
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        assert!(lock.find("superpowers").is_none());
    }

    #[test]
    fn bulk_skips_symlinks_into_agents_local() {
        let fx = Fixture::new();
        // Pretend a skill is already an agents-managed symlink.
        let local_target = paths::local_skills_dir(fx.repo.path()).join("alpha");
        std::fs::create_dir_all(&local_target).unwrap();
        std::fs::write(local_target.join("SKILL.md"), "snapshot").unwrap();
        let claude_skills = fx.home.path().join(".claude/skills");
        std::fs::create_dir_all(&claude_skills).unwrap();
        std::os::unix::fs::symlink(&local_target, claude_skills.join("alpha")).unwrap();

        let mut lock = Lockfile::load(fx.repo.path()).unwrap();
        let outcome = bulk_import_skills(fx.repo.path(), fx.home.path(), &mut lock).unwrap();
        assert_eq!(outcome.imported, 0);
        assert_eq!(outcome.skipped_managed, 1);
    }

    #[test]
    fn bulk_skips_plugin_managed_skills() {
        let fx = Fixture::new();
        // alpha is plugin-managed; agents must not snapshot it.
        fx.write_valid_skill("claude", "alpha", "alpha body");
        fx.write_plugin_skill(
            "frontend-design",
            "claude-plugins-official",
            "anthropics/claude-plugins-official",
            "alpha",
        );
        // beta is a plain unmanaged skill that should still get imported.
        fx.write_valid_skill("claude", "beta", "beta body");

        let mut lock = Lockfile::load(fx.repo.path()).unwrap();
        let outcome = bulk_import_skills(fx.repo.path(), fx.home.path(), &mut lock).unwrap();

        assert_eq!(outcome.imported, 1, "only beta should import");
        assert_eq!(outcome.skipped_plugin, 1, "alpha is plugin-managed");
        assert!(lock.find("beta").is_some());
        assert!(lock.find("alpha").is_none());
        // Snapshot dir for alpha must NOT be created.
        assert!(!paths::local_skills_dir(fx.repo.path())
            .join("alpha")
            .exists());
    }

    #[test]
    fn single_import_refuses_plugin_managed_skill() {
        let fx = Fixture::new();
        fx.write_skill("claude", "alpha", "body");
        fx.write_plugin_skill(
            "frontend-design",
            "claude-plugins-official",
            "anthropics/claude-plugins-official",
            "alpha",
        );

        let args = ImportArgs {
            name: Some("alpha".into()),
            instructions: false,
            upstream: None,
            project: None,
        };
        let err = run_single(fx.repo.path(), fx.home.path(), &args, true).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("plugin-managed"), "got: {msg}");
        assert!(
            msg.contains("anthropics/claude-plugins-official"),
            "got: {msg}"
        );

        // Lockfile untouched.
        let lock = Lockfile::load(fx.repo.path()).unwrap();
        assert!(lock.find("alpha").is_none());
    }

    #[test]
    fn single_import_refuses_invalid_skill_md() {
        let fx = Fixture::new();
        fx.write_skill("claude", "alpha", "body");

        let args = ImportArgs {
            name: Some("alpha".into()),
            instructions: false,
            upstream: None,
            project: None,
        };
        let err = run_single(fx.repo.path(), fx.home.path(), &args, true).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("missing a valid SKILL.md"), "got: {msg}");

        let lock = Lockfile::load(fx.repo.path()).unwrap();
        assert!(lock.find("alpha").is_none());
        assert!(!paths::local_skills_dir(fx.repo.path())
            .join("alpha")
            .exists());
    }

    #[test]
    fn single_import_accepts_valid_skill_md() {
        let fx = Fixture::new();
        fx.write_skill(
            "claude",
            "alpha",
            "---\nname: alpha\ndescription: Alpha skill.\n---\nbody\n",
        );

        let args = ImportArgs {
            name: Some("alpha".into()),
            instructions: false,
            upstream: None,
            project: None,
        };
        run_single(fx.repo.path(), fx.home.path(), &args, true).unwrap();

        let lock = Lockfile::load(fx.repo.path()).unwrap();
        assert!(lock.find("alpha").is_some());
        assert!(paths::local_skills_dir(fx.repo.path())
            .join("alpha")
            .join("SKILL.md")
            .is_file());
    }
}
