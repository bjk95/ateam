use crate::cli::ListArgs;
use crate::lockfile::{Lockfile, SkillEntry};
use crate::paths;
use crate::ui;
use anyhow::Result;

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

    let width = entries.iter().map(|s| s.name.len()).max().unwrap_or(0);
    for s in &entries {
        let source = render_source(s);
        ui::plain(format!("{:<width$}  {}", s.name, source, width = width));

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
    let base = if let Some(rest) = s.source.strip_prefix("github:") {
        rest.to_string()
    } else if s.source.starts_with("local:") {
        "local".to_string()
    } else if let Some(url) = s.source.strip_prefix("git:") {
        url.to_string()
    } else {
        s.source.clone()
    };
    match &s.git_ref {
        Some(r) => format!("{} @ {}", base, r),
        None => base,
    }
}
