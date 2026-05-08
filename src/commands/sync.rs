use crate::{git_sync, paths};
use anyhow::Result;

pub fn run() -> Result<()> {
    let repo = paths::resolve_repo()?;
    git_sync::sync(&repo)
}
