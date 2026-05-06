use crate::cli::ListArgs;
use crate::lockfile::Lockfile;
use crate::paths;
use anyhow::Result;

pub fn run(args: ListArgs) -> Result<()> {
    let repo = paths::resolve_repo()?;
    let lock = Lockfile::load(&repo)?;

    let entries: Vec<_> = lock
        .skills
        .iter()
        .filter(|s| match (&args.project, &s.project) {
            (None, _) => true,
            (Some(filter), Some(p)) => filter == p,
            (Some(_), None) => false,
        })
        .collect();

    if entries.is_empty() {
        println!("(no skills locked)");
        return Ok(());
    }

    for s in entries {
        let scope = match &s.project {
            Some(alias) => format!("project={}", alias),
            None => "global".into(),
        };
        let profiles = if s.profiles.is_empty() {
            "all".to_string()
        } else {
            s.profiles.join(",")
        };
        let r = s.git_ref.as_deref().unwrap_or("(default)");
        println!(
            "{:30}  {:50}  scope={:<14}  profiles={:<14}  ref={}",
            s.name, s.source, scope, profiles, r
        );
    }
    Ok(())
}
