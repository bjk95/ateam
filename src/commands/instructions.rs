use crate::cli::InstructionsCommand;
use crate::commands::apply_instructions::resolve_tools;
use crate::commands::edit::{pick_editor, spawn_editor};
use crate::config::{MachineConfig, RepoConfig};
use crate::git_sync;
use crate::instructions;
use crate::lockfile::{InstructionsEntry, Lockfile};
use crate::paths;
use crate::ui;
use anyhow::{bail, Result};
use console::style;
use similar::{ChangeTag, TextDiff};

pub fn run(cmd: InstructionsCommand, no_sync: bool) -> Result<()> {
    match cmd {
        InstructionsCommand::Edit => edit(no_sync),
        InstructionsCommand::Diff => diff(),
        InstructionsCommand::Show => show(),
    }
}

fn edit(no_sync: bool) -> Result<()> {
    let repo = paths::resolve_repo()?;
    let template = paths::instructions_template(&repo);
    if !template.exists() {
        bail!(
            "instructions template not found at {} — run `agents import --instructions` to bootstrap",
            template.display()
        );
    }

    let editor = pick_editor();

    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    spawn_editor(&editor, &template)?;

    if git_sync::enabled(no_sync) {
        let msg = git_sync::msg_edit("instructions");
        if let Err(e) = git_sync::commit_and_push(&repo, &msg) {
            ui::warn(format!("auto-sync failed: {:#}", e));
            ui::detail("local change saved; rerun a mutating command to retry");
        }
    }

    Ok(())
}

fn diff() -> Result<()> {
    let repo = paths::resolve_repo()?;
    let repo_cfg = RepoConfig::load(&repo)?;
    let machine = MachineConfig::load(&repo)?;
    let lock = Lockfile::load(&repo)?;
    let home = paths::home_dir()?;

    let template = paths::instructions_template(&repo);
    if !template.exists() {
        ui::plain("no instructions template");
        return Ok(());
    }
    if machine.instructions_skip {
        ui::plain(
            "instructions sync disabled on this machine (machine.toml: instructions_skip = true)",
        );
        return Ok(());
    }

    let entry = lock
        .instructions
        .clone()
        .unwrap_or_else(InstructionsEntry::default);
    let tools = resolve_tools(&repo_cfg, &entry);
    let template_src = instructions::read_template(&repo)?;
    let hostname = instructions::current_hostname();

    let mut printed = false;
    for harness in tools {
        let ctx = instructions::build_context(&repo_cfg, &machine, &hostname, harness);
        let rendered = instructions::render(&template_src, &ctx)?;
        let out = instructions::output_path(&home, harness);
        let current = std::fs::read_to_string(&out).unwrap_or_default();
        if current == rendered {
            continue;
        }
        printed = true;
        let label = paths::display_path(&out);
        ui::plain(format!("{}", style(format!("--- a/{}", label)).bold()));
        ui::plain(format!("{}", style(format!("+++ b/{}", label)).bold()));
        print_unified_diff(&current, &rendered);
        ui::plain("");
    }
    if !printed {
        ui::plain("no changes");
    }
    Ok(())
}

fn show() -> Result<()> {
    let repo = paths::resolve_repo()?;
    let repo_cfg = RepoConfig::load(&repo)?;
    let machine = MachineConfig::load(&repo)?;
    let lock = Lockfile::load(&repo)?;

    let template = paths::instructions_template(&repo);
    if !template.exists() {
        bail!(
            "instructions template not found at {} — run `agents import --instructions` to bootstrap",
            template.display()
        );
    }

    let entry = lock
        .instructions
        .clone()
        .unwrap_or_else(InstructionsEntry::default);
    let tools = resolve_tools(&repo_cfg, &entry);
    let template_src = instructions::read_template(&repo)?;
    let hostname = instructions::current_hostname();

    for (i, harness) in tools.iter().enumerate() {
        if i > 0 {
            ui::plain("");
        }
        let ctx = instructions::build_context(&repo_cfg, &machine, &hostname, *harness);
        let rendered = instructions::render(&template_src, &ctx)?;
        ui::plain(format!(
            "{}",
            style(format!(
                "# {} ({})",
                harness.output_subpath(),
                harness.display()
            ))
            .bold()
        ));
        ui::write(rendered);
    }
    Ok(())
}

fn print_unified_diff(old: &str, new: &str) {
    let diff = TextDiff::from_lines(old, new);
    for group in diff.grouped_ops(3) {
        let (old_start, old_len, new_start, new_len) = hunk_header(&group);
        ui::plain(format!(
            "{}",
            style(format!(
                "@@ -{},{} +{},{} @@",
                old_start + 1,
                old_len,
                new_start + 1,
                new_len
            ))
            .cyan()
        ));
        for op in group {
            for change in diff.iter_changes(&op) {
                let line = change.to_string();
                let styled = match change.tag() {
                    ChangeTag::Delete => style(format!("-{}", line)).red().to_string(),
                    ChangeTag::Insert => style(format!("+{}", line)).green().to_string(),
                    ChangeTag::Equal => style(format!(" {}", line)).dim().to_string(),
                };
                ui::write(styled);
            }
        }
    }
}

fn hunk_header(group: &[similar::DiffOp]) -> (usize, usize, usize, usize) {
    let first = group.first().expect("non-empty group");
    let last = group.last().expect("non-empty group");
    let old_start = first.as_tag_tuple().1.start;
    let new_start = first.as_tag_tuple().2.start;
    let old_end = last.as_tag_tuple().1.end;
    let new_end = last.as_tag_tuple().2.end;
    (
        old_start,
        old_end - old_start,
        new_start,
        new_end - new_start,
    )
}
