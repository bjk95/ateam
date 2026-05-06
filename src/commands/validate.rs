use crate::config::RepoConfig;
use crate::instructions;
use crate::paths;
use anyhow::{bail, Result};
use std::collections::BTreeSet;

pub fn run() -> Result<()> {
    let repo = paths::resolve_repo()?;
    let repo_cfg = RepoConfig::load(&repo)?;
    let template = paths::instructions_template(&repo);

    if !template.exists() {
        println!(
            "no instructions template at {} — nothing to validate",
            template.display()
        );
        return Ok(());
    }

    let src = instructions::read_template(&repo)?;

    let mut allowed: BTreeSet<String> = repo_cfg.declared_profiles.iter().cloned().collect();
    for r in instructions::reserved_identifiers() {
        allowed.insert((*r).into());
    }

    let unknown = instructions::unknown_identifiers(&src, &allowed)?;

    if unknown.is_empty() {
        println!(
            "ok: instructions template references {} identifier(s), all declared",
            allowed.len()
        );
        return Ok(());
    }

    eprintln!(
        "instructions template references undeclared identifier(s): {}",
        unknown
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    );
    eprintln!(
        "declared profiles: [{}]",
        repo_cfg.declared_profiles.join(", ")
    );
    eprintln!(
        "reserved identifiers: [{}]",
        instructions::reserved_identifiers().join(", ")
    );
    bail!("validate failed");
}
