use crate::cli::ShowArgs;
use crate::lockfile::Lockfile;
use crate::paths;
use anyhow::{anyhow, bail, Context, Result};
use std::path::PathBuf;

pub fn run(args: ShowArgs) -> Result<()> {
    let repo = paths::resolve_repo()?;
    let lock = Lockfile::load(&repo)?;
    let entry = lock
        .find(&args.name)
        .ok_or_else(|| anyhow!("no skill named `{}` in lockfile", args.name))?;

    // For local: sources, the path field is canonical; for everything else, the
    // snapshot lives at <repo>/skills/<name>/.
    let skill_dir: PathBuf = if let Some(rest) = entry.source.strip_prefix("local:") {
        repo.join(rest)
    } else {
        paths::local_skills_dir(&repo).join(&entry.name)
    };
    let skill_md = skill_dir.join("SKILL.md");

    if !skill_md.exists() {
        bail!(
            "SKILL.md not found at {} — run `ateam apply` to materialize the snapshot",
            skill_md.display()
        );
    }

    let content = std::fs::read_to_string(&skill_md)
        .with_context(|| format!("reading {}", skill_md.display()))?;
    print!("{}", content);
    Ok(())
}
