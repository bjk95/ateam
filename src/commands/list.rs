use crate::cli::ListArgs;
use crate::lockfile::{Lockfile, SkillEntry};
use crate::paths;
use crate::ui;
use anyhow::Result;
use console::style;

pub fn run(args: ListArgs) -> Result<()> {
    let repo = paths::resolve_repo()?;
    let lock = Lockfile::load(&repo)?;

    let entries: Vec<&SkillEntry> = lock
        .skills
        .iter()
        .filter(|s| match (&args.project, &s.project) {
            (None, _) => true,
            (Some(filter), Some(p)) => filter == p,
            (Some(_), None) => false,
        })
        .collect();

    if entries.is_empty() {
        ui::plain("(no skills locked)");
        return Ok(());
    }

    // `:<width$` is byte-width, not display-width — fine for ASCII skill names.
    // Pad the raw name first, then style the padded version so ANSI codes don't
    // throw off alignment.
    let width = entries.iter().map(|s| s.name.len()).max().unwrap_or(0);
    for s in &entries {
        let padded_name = format!("{:<width$}", s.name, width = width);
        ui::plain(format!("{}  {}", style(padded_name).bold(), render_source(s)));

        // Verbose: append a dim qualifier line if scope or profiles is non-default.
        let scope_part = s.project.as_ref().map(|p| format!("scope: project={}", p));
        let profiles_part = if s.profiles.is_empty() {
            None
        } else {
            Some(format!("profiles: {}", s.profiles.join(",")))
        };
        let parts: Vec<String> = [scope_part, profiles_part]
            .into_iter()
            .flatten()
            .collect();
        if !parts.is_empty() {
            ui::detail(parts.join(" · "));
        }
    }
    Ok(())
}

fn render_source(s: &SkillEntry) -> String {
    let base: String = if let Some(rest) = s.source.strip_prefix("github:") {
        format!("{}", style(rest).cyan())
    } else if s.source.starts_with("local:") {
        format!("{}", style("local").dim())
    } else if let Some(url) = s.source.strip_prefix("git:") {
        format!("{}", style(url).cyan())
    } else {
        format!("{}", style(&s.source).cyan())
    };
    match &s.git_ref {
        Some(r) => format!("{} {}", base, style(format!("@ {}", r)).dim()),
        None => base,
    }
}
