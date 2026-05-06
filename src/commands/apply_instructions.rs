use crate::config::{MachineConfig, RepoConfig};
use crate::install::{self, CopyOutcome};
use crate::instructions::{self, Tool};
use crate::lockfile::{InstructionsEntry, Lockfile};
use crate::manifest::{self, EntryKind, Manifest, ManifestEntry};
use crate::paths;
use anyhow::{bail, Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct ApplyOutcome {
    pub written: usize,
    pub lockfile_dirty: bool,
    pub instructions_skip_set: bool,
}

/// Plan + execute the instructions render-and-write pass.
///
/// Returns the count of files written and whether the lockfile or machine.toml
/// need to be persisted by the caller.
pub fn apply(
    repo: &Path,
    home: &Path,
    repo_cfg: &RepoConfig,
    lock: &mut Lockfile,
    machine: &mut MachineConfig,
    prev_manifest: &Manifest,
    new_manifest: &mut Manifest,
    dry_run: bool,
    force: bool,
) -> Result<ApplyOutcome> {
    let mut outcome = ApplyOutcome {
        written: 0,
        lockfile_dirty: false,
        instructions_skip_set: false,
    };

    let template_path = paths::instructions_template(repo);
    let template_exists = template_path.exists();

    if lock.instructions.is_some() && !template_exists {
        bail!(
            "lockfile has [instructions] but template missing at {}",
            template_path.display()
        );
    }

    if !template_exists {
        return Ok(outcome);
    }

    if lock.instructions.is_none() {
        lock.instructions = Some(InstructionsEntry::default());
        outcome.lockfile_dirty = true;
    }

    if machine.instructions_skip {
        return Ok(outcome);
    }

    let entry = lock.instructions.as_ref().unwrap().clone();
    let tools = resolve_tools(repo_cfg, &entry);

    let template_src = instructions::read_template(repo)?;

    let prev_paths: HashSet<PathBuf> = prev_manifest
        .entries
        .iter()
        .filter(|e| matches!(e.kind, EntryKind::Copy))
        .map(|e| e.path.clone())
        .collect();

    let hostname = instructions::current_hostname();

    for tool in tools {
        let ctx = instructions::build_context(repo_cfg, machine, &hostname, tool);
        let rendered = instructions::render(&template_src, &ctx)?;
        let out = instructions::output_path(home, tool);
        let was_managed = prev_paths.contains(&out);

        if dry_run {
            println!(
                "would write {} ({} bytes) [{}]",
                out.display(),
                rendered.len(),
                tool.agent()
            );
            new_manifest.entries.push(ManifestEntry {
                path: out,
                kind: EntryKind::Copy,
                skill: "_instructions".into(),
                agent: tool.agent().into(),
                target: template_path.clone(),
                applied_at: manifest::now_unix(),
            });
            continue;
        }

        let result = install::install_copy(&out, &rendered, was_managed, force)
            .with_context(|| format!("writing {}", out.display()))?;

        match result {
            CopyOutcome::Written | CopyOutcome::MovedAside { .. } => {
                if let CopyOutcome::MovedAside { backup } = &result {
                    eprintln!(
                        "ateam: moved aside existing {} → {}",
                        out.display(),
                        backup.display()
                    );
                }
                new_manifest.entries.push(ManifestEntry {
                    path: out,
                    kind: EntryKind::Copy,
                    skill: "_instructions".into(),
                    agent: tool.agent().into(),
                    target: template_path.clone(),
                    applied_at: manifest::now_unix(),
                });
                outcome.written += 1;
            }
            CopyOutcome::Refused => {
                let choice = prompt_collision(&out)?;
                match choice {
                    CollisionChoice::Skip => {
                        machine.instructions_skip = true;
                        outcome.instructions_skip_set = true;
                        eprintln!(
                            "ateam: instructions sync disabled on this machine (recorded in machine.toml). re-enable by clearing `instructions_skip` and re-running."
                        );
                        return Ok(outcome);
                    }
                    CollisionChoice::Cancel => {
                        eprintln!(
                            "ateam: cancelled — {} left untouched. rerun with --force to overwrite, or back it up first.",
                            out.display()
                        );
                        return Ok(outcome);
                    }
                }
            }
        }
    }

    Ok(outcome)
}

fn resolve_tools(repo_cfg: &RepoConfig, entry: &InstructionsEntry) -> Vec<Tool> {
    let agents: Vec<&String> = if entry.agents.iter().any(|a| a == "*") {
        repo_cfg.enabled_agents.iter().collect()
    } else {
        entry
            .agents
            .iter()
            .filter(|a| repo_cfg.enabled_agents.contains(a))
            .collect()
    };
    agents
        .into_iter()
        .filter_map(|a| Tool::from_agent(a))
        .collect()
}

#[derive(Debug)]
enum CollisionChoice {
    Skip,
    Cancel,
}

fn prompt_collision(path: &Path) -> Result<CollisionChoice> {
    use dialoguer::{theme::ColorfulTheme, Select};
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        // Non-interactive: default to cancel so we don't accidentally skip forever.
        eprintln!(
            "ateam: refusing to overwrite existing {} (non-interactive). rerun with --force or set `instructions_skip = true` in machine.toml to skip on this machine.",
            path.display()
        );
        return Ok(CollisionChoice::Cancel);
    }
    let prompt = format!(
        "{} already exists and isn't tracked by ateam.\n  How should ateam proceed?",
        path.display()
    );
    let choice = Select::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .items(&[
            "Skip syncing instructions on this machine (record in machine.toml)",
            "Cancel — leave the file alone for this run",
        ])
        .default(1)
        .interact()?;
    Ok(match choice {
        0 => CollisionChoice::Skip,
        _ => CollisionChoice::Cancel,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::InstructionsEntry;
    use std::path::PathBuf;

    struct Fixture {
        repo: tempfile::TempDir,
        home: tempfile::TempDir,
    }

    impl Fixture {
        fn new(declared: &[&str], machine_profiles: &[&str]) -> Self {
            let repo = tempfile::tempdir().unwrap();
            let home = tempfile::tempdir().unwrap();
            let repo_cfg = RepoConfig {
                declared_profiles: declared.iter().map(|s| (*s).to_string()).collect(),
                enabled_agents: vec!["claude-code".into(), "codex".into()],
            };
            repo_cfg.write(repo.path()).unwrap();
            std::fs::create_dir_all(repo.path().join(".ateam")).unwrap();
            let mut machine = MachineConfig::default();
            machine.profiles = machine_profiles.iter().map(|s| (*s).to_string()).collect();
            machine.write(repo.path()).unwrap();
            Self { repo, home }
        }

        fn write_template(&self, body: &str) {
            std::fs::create_dir_all(self.repo.path().join("instructions")).unwrap();
            std::fs::write(
                self.repo.path().join("instructions/instructions.md.hbs"),
                body,
            )
            .unwrap();
        }

        fn lock_with_instructions(&self) -> Lockfile {
            Lockfile {
                skills: Vec::new(),
                instructions: Some(InstructionsEntry::default()),
            }
        }

        fn lock_empty(&self) -> Lockfile {
            Lockfile {
                skills: Vec::new(),
                instructions: None,
            }
        }

        fn read_output(&self, tool: Tool) -> Option<String> {
            let p = self.home.path().join(tool.output_subpath());
            std::fs::read_to_string(p).ok()
        }

        fn output_path(&self, tool: Tool) -> PathBuf {
            self.home.path().join(tool.output_subpath())
        }

        fn run(
            &self,
            lock: &mut Lockfile,
            machine: &mut MachineConfig,
            prev: &Manifest,
            new: &mut Manifest,
            force: bool,
        ) -> Result<ApplyOutcome> {
            let repo_cfg = RepoConfig::load(self.repo.path()).unwrap();
            apply(
                self.repo.path(),
                self.home.path(),
                &repo_cfg,
                lock,
                machine,
                prev,
                new,
                false,
                force,
            )
        }
    }

    #[test]
    fn no_template_no_lockfile_entry_is_noop() {
        let fx = Fixture::new(&["work"], &["work"]);
        let mut lock = fx.lock_empty();
        let mut machine = MachineConfig::load(fx.repo.path()).unwrap();
        let prev = Manifest::default();
        let mut new = Manifest::default();
        let outcome = fx.run(&mut lock, &mut machine, &prev, &mut new, false).unwrap();
        assert_eq!(outcome.written, 0);
        assert!(!outcome.lockfile_dirty);
        assert!(lock.instructions.is_none());
    }

    #[test]
    fn template_present_auto_adds_lockfile_entry_and_writes() {
        let fx = Fixture::new(&["work", "personal"], &["work"]);
        fx.write_template("hello {{#if work}}WORK{{/if}}{{#if personal}}HOME{{/if}}\n");
        let mut lock = fx.lock_empty();
        let mut machine = MachineConfig::load(fx.repo.path()).unwrap();
        let prev = Manifest::default();
        let mut new = Manifest::default();
        let outcome = fx.run(&mut lock, &mut machine, &prev, &mut new, false).unwrap();
        assert_eq!(outcome.written, 2);
        assert!(outcome.lockfile_dirty, "should mark lockfile dirty when auto-adding entry");
        assert!(lock.instructions.is_some());

        let claude = fx.read_output(Tool::Claude).unwrap();
        let codex = fx.read_output(Tool::Codex).unwrap();
        assert!(claude.contains("WORK"), "got: {}", claude);
        assert!(!claude.contains("HOME"));
        assert_eq!(claude, codex, "no tool branching → identical content");
    }

    #[test]
    fn tool_branch_diverges_per_render() {
        let fx = Fixture::new(&["work"], &["work"]);
        fx.write_template("{{#if claude}}CLAUDE{{/if}}{{#if codex}}CODEX{{/if}}\n");
        let mut lock = fx.lock_with_instructions();
        let mut machine = MachineConfig::load(fx.repo.path()).unwrap();
        let prev = Manifest::default();
        let mut new = Manifest::default();
        fx.run(&mut lock, &mut machine, &prev, &mut new, false).unwrap();
        assert_eq!(fx.read_output(Tool::Claude).unwrap().trim(), "CLAUDE");
        assert_eq!(fx.read_output(Tool::Codex).unwrap().trim(), "CODEX");
    }

    #[test]
    fn machine_skip_short_circuits() {
        let fx = Fixture::new(&["work"], &["work"]);
        fx.write_template("anything\n");
        let mut lock = fx.lock_with_instructions();
        let mut machine = MachineConfig::load(fx.repo.path()).unwrap();
        machine.instructions_skip = true;
        let prev = Manifest::default();
        let mut new = Manifest::default();
        let outcome = fx.run(&mut lock, &mut machine, &prev, &mut new, false).unwrap();
        assert_eq!(outcome.written, 0);
        assert!(fx.read_output(Tool::Claude).is_none());
        assert!(fx.read_output(Tool::Codex).is_none());
    }

    #[test]
    fn pre_existing_file_refused_without_force() {
        let fx = Fixture::new(&["work"], &["work"]);
        fx.write_template("template body\n");
        // Pre-create a colliding file outside ateam's manifest.
        let claude_out = fx.output_path(Tool::Claude);
        std::fs::create_dir_all(claude_out.parent().unwrap()).unwrap();
        std::fs::write(&claude_out, "user-managed content").unwrap();

        let mut lock = fx.lock_with_instructions();
        let mut machine = MachineConfig::load(fx.repo.path()).unwrap();
        let prev = Manifest::default();
        let mut new = Manifest::default();
        // Non-interactive (cargo test) → CollisionChoice::Cancel
        fx.run(&mut lock, &mut machine, &prev, &mut new, false).unwrap();

        // Original content untouched.
        let after = std::fs::read_to_string(&claude_out).unwrap();
        assert_eq!(after, "user-managed content");
    }

    #[test]
    fn force_backs_up_and_writes() {
        let fx = Fixture::new(&["work"], &["work"]);
        fx.write_template("template body\n");
        let claude_out = fx.output_path(Tool::Claude);
        std::fs::create_dir_all(claude_out.parent().unwrap()).unwrap();
        std::fs::write(&claude_out, "stale local").unwrap();

        let mut lock = fx.lock_with_instructions();
        let mut machine = MachineConfig::load(fx.repo.path()).unwrap();
        let prev = Manifest::default();
        let mut new = Manifest::default();
        let outcome = fx.run(&mut lock, &mut machine, &prev, &mut new, true).unwrap();
        assert!(outcome.written >= 1);
        assert_eq!(fx.read_output(Tool::Claude).unwrap(), "template body\n");

        // Backup file should exist alongside.
        let parent = claude_out.parent().unwrap();
        let backups: Vec<_> = std::fs::read_dir(parent)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("CLAUDE.md.bak.")
            })
            .collect();
        assert_eq!(backups.len(), 1, "expected one backup file");
    }

    #[test]
    fn previously_managed_file_is_overwritten_without_force() {
        let fx = Fixture::new(&["work"], &["work"]);
        fx.write_template("v1\n");
        let mut lock = fx.lock_with_instructions();
        let mut machine = MachineConfig::load(fx.repo.path()).unwrap();
        let mut prev = Manifest::default();
        let mut new = Manifest::default();
        // First apply
        fx.run(&mut lock, &mut machine, &prev, &mut new, false).unwrap();
        assert_eq!(fx.read_output(Tool::Claude).unwrap(), "v1\n");

        // Carry forward: prev = previous new.
        prev = new;
        fx.write_template("v2\n");
        let mut new2 = Manifest::default();
        fx.run(&mut lock, &mut machine, &prev, &mut new2, false).unwrap();
        assert_eq!(fx.read_output(Tool::Claude).unwrap(), "v2\n");
    }

    #[test]
    fn missing_template_with_lockfile_entry_errors() {
        let fx = Fixture::new(&["work"], &["work"]);
        let mut lock = fx.lock_with_instructions();
        let mut machine = MachineConfig::load(fx.repo.path()).unwrap();
        let prev = Manifest::default();
        let mut new = Manifest::default();
        let err = fx
            .run(&mut lock, &mut machine, &prev, &mut new, false)
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("template missing"), "got: {msg}");
    }
}

