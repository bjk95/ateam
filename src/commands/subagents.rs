//! Subagent management — canonical multi-harness `.md` files at
//! `<repo>/agents/<name>.md` get rendered into each harness's native format
//! by `apply`. Codex needs `.toml`; Claude/OpenCode/Gemini take Markdown +
//! YAML frontmatter with different field name spellings. A single symlink
//! can't serve all four, so subagent install renders per-harness files into
//! the agents repo and symlinks harness paths to those rendered files.

use crate::cli::{SubagentAddArgs, SubagentRemoveArgs};
use crate::config::RepoConfig;
use crate::git_sync;
use crate::install;
use crate::lockfile::{Lockfile, SubagentEntry};
use crate::manifest::{self, EntryKind, Manifest, ManifestEntry};
use crate::paths;
use crate::source::{github, Source};
use crate::subagent::{self, Subagent, SubagentFrontmatter};
use crate::ui;
use anyhow::{anyhow, bail, Context, Result};
use console::style;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// add

pub fn add(args: SubagentAddArgs, no_sync: bool) -> Result<()> {
    let repo = paths::resolve_repo()?;
    let repo_cfg = RepoConfig::load(&repo)?;

    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    let source = Source::parse_with(&args.source, args.dangerously_accept_openclaw_risks)?;
    ui::diamond(format!("Source: {}", args.source));

    let targets = resolve_add_targets(&args, &source)?;
    if targets.is_empty() {
        bail!("specify --subagent <name> (repeatable) or --path <file>");
    }

    let harnesses = resolve_harnesses(&args.harnesses, &repo_cfg);
    if harnesses.is_empty() {
        bail!("no enabled harnesses support subagents");
    }

    let mut lock = Lockfile::load(&repo)?;
    let mut manifest = Manifest::load(&repo)?;
    let mut installed: Vec<String> = Vec::new();
    let mut had_error = false;

    for target in &targets {
        let step = ui::step(format!("installing subagent {}", target.name));
        match install_one(&repo, &source, target, &args, &harnesses, &mut manifest) {
            Ok(entry) => {
                lock.upsert_subagent(entry);
                lock.write(&repo).context("writing lockfile after upsert")?;
                installed.push(target.name.clone());
                step.ok(format!("installed subagent {}", target.name));
            }
            Err(e) => {
                had_error = true;
                step.fail(format!("install {} — {:#}", target.name, e));
            }
        }
    }

    if installed.is_empty() {
        if had_error {
            bail!("no subagents installed (all failed)");
        }
        return Ok(());
    }

    manifest.write(&repo).context("writing manifest")?;

    if git_sync::enabled(no_sync) {
        let msg = msg_subagent_add(&source.lockfile_string(), &installed);
        if let Err(e) = git_sync::commit_and_push(&repo, &msg) {
            ui::warn(format!("auto-sync failed: {:#}", e));
            ui::detail("local change saved; rerun a mutating command to retry");
        }
    }

    Ok(())
}

/// One unit of work for `add`: resolves to `(name, path-within-source)`.
struct AddTarget {
    name: String,
    path_in_source: String,
}

fn resolve_add_targets(args: &SubagentAddArgs, source: &Source) -> Result<Vec<AddTarget>> {
    if let Some(p) = &args.path {
        let stem = Path::new(p)
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("--path `{}` has no file stem", p))?;
        let name = match args.subagent.first() {
            Some(n) => n.clone(),
            None => stem.to_string(),
        };
        return Ok(vec![AddTarget {
            name,
            path_in_source: p.clone(),
        }]);
    }

    if let Source::Local { path } = source {
        if path.extension().and_then(|s| s.to_str()) == Some("md") {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| anyhow!("local file `{}` has no stem", path.display()))?;
            let name = match args.subagent.first() {
                Some(n) => n.clone(),
                None => stem.to_string(),
            };
            return Ok(vec![AddTarget {
                name,
                path_in_source: String::new(),
            }]);
        }
    }

    if args.subagent.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(args.subagent.len());
    for raw in &args.subagent {
        let name = raw.trim().to_string();
        if name.is_empty() {
            continue;
        }
        out.push(AddTarget {
            name: name.clone(),
            path_in_source: format!("agents/{}.md", name),
        });
    }
    Ok(out)
}

fn install_one(
    repo: &Path,
    source: &Source,
    target: &AddTarget,
    args: &SubagentAddArgs,
    harnesses: &[String],
    manifest: &mut Manifest,
) -> Result<SubagentEntry> {
    let (raw, resolved_ref) = fetch_file(source, args.r#ref.as_deref(), &target.path_in_source)?;

    // Convert imported source format → canonical. Most subagents in the wild
    // are Claude-format (.md + YAML frontmatter), so that's what we accept.
    // Codex-native .toml import is a future enhancement — flagged as such
    // and left unhandled here.
    let canonical = into_canonical_from_claude(&target.name, &raw).context(
        "parsing imported subagent — expected Claude-format Markdown with YAML frontmatter",
    )?;

    let canonical_text = canonical.to_canonical()?;
    let snapshot = paths::local_subagent_path(repo, &target.name);
    if let Some(parent) = snapshot.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    write_atomically(&snapshot, &canonical_text)?;
    let file_sha = sha256_hex(canonical_text.as_bytes());

    // Render and install for every harness with subagent support.
    let install_root = paths::home_dir()?;
    let prev = Manifest::load(repo)?;
    for harness in harnesses {
        let Some(out_path) = subagent::harness_install_path(&install_root, harness, &target.name)?
        else {
            continue;
        };
        let Some(rendered) = subagent::render_for_harness(&canonical, harness)? else {
            continue;
        };
        let rendered_path = subagent::rendered_path(repo, harness, &target.name);
        install::write_atomically(&rendered_path, &rendered)
            .with_context(|| format!("writing {}", rendered_path.display()))?;
        remove_legacy_copy(&prev, &out_path)?;
        match install::install_symlink(&out_path, &rendered_path, false)? {
            install::LinkOutcome::Created
            | install::LinkOutcome::Replaced
            | install::LinkOutcome::AlreadyCorrect
            | install::LinkOutcome::MovedAside
            | install::LinkOutcome::AutoHealed => {
                manifest.entries.push(ManifestEntry {
                    path: out_path.clone(),
                    kind: EntryKind::Symlink,
                    skill: target.name.clone(),
                    harness: harness.clone(),
                    target: rendered_path,
                    applied_at: manifest::now_unix(),
                });
                ui::detail(format!("linked {}", paths::display_path(&out_path)));
            }
            install::LinkOutcome::Refused => {
                ui::warn(format!(
                    "refused: real file at {} (use `agents apply --force` to overwrite)",
                    paths::display_path(&out_path)
                ));
            }
        }
    }

    Ok(SubagentEntry {
        name: target.name.clone(),
        source: source.lockfile_string(),
        path: if target.path_in_source.is_empty() {
            None
        } else {
            Some(target.path_in_source.clone())
        },
        git_ref: resolved_ref,
        file_sha: Some(file_sha),
        harnesses: if args.harnesses.is_empty() {
            vec!["*".into()]
        } else {
            args.harnesses.clone()
        },
        profiles: args.profile.clone(),
        project: None,
        active: true,
        upstream: None,
    })
}

fn remove_legacy_copy(prev_manifest: &Manifest, path: &Path) -> Result<()> {
    let was_copy = prev_manifest
        .entries
        .iter()
        .any(|e| e.path == path && matches!(e.kind, EntryKind::Copy));
    if was_copy {
        if std::fs::symlink_metadata(path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Ok(());
        }
        install::uninstall_copy(path)?;
    }
    Ok(())
}

/// Parse a Claude-format subagent file (YAML frontmatter + body) into the
/// canonical shape. Frontmatter `model`/`effort` go into the `claude:` slots
/// since they're Claude-specific values; `skills`/`color` are shared.
fn into_canonical_from_claude(name: &str, raw: &str) -> Result<Subagent> {
    let parsed = gray_matter::Matter::<gray_matter::engine::YAML>::new().parse(raw);
    let body = parsed.content;

    #[derive(serde::Deserialize)]
    struct ClaudeFrontmatter {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        effort: Option<String>,
        #[serde(default)]
        skills: Vec<String>,
        #[serde(default)]
        color: Option<String>,
    }

    let fm: ClaudeFrontmatter = match parsed.data {
        Some(data) => data
            .deserialize()
            .context("imported subagent has unrecognized frontmatter shape")?,
        None => bail!("imported subagent has no YAML frontmatter"),
    };

    let resolved_name = fm.name.unwrap_or_else(|| name.to_string());
    let description = fm
        .description
        .ok_or_else(|| anyhow!("imported subagent missing `description`"))?;

    let mut canonical = SubagentFrontmatter {
        name: resolved_name,
        description,
        skills: fm.skills,
        color: fm.color,
        ..Default::default()
    };
    if let Some(m) = fm.model {
        canonical.model.claude = Some(m);
    }
    if let Some(e) = fm.effort {
        canonical.effort.claude = Some(e);
    }

    Ok(Subagent {
        frontmatter: canonical,
        body,
    })
}

fn fetch_file(
    source: &Source,
    git_ref: Option<&str>,
    path_in_source: &str,
) -> Result<(String, Option<String>)> {
    match source {
        Source::Github { owner, repo } => {
            let r = match git_ref {
                Some(r) => r.to_string(),
                None => github::default_branch(owner, repo),
            };
            let body =
                github::read_file_at_ref(owner, repo, &r, path_in_source).with_context(|| {
                    format!("fetching {}/{}@{}: {}", owner, repo, r, path_in_source)
                })?;
            Ok((body, git_ref.map(|s| s.to_string())))
        }
        Source::Git { url } => {
            let suffix: u64 = rand::random();
            let tmp = std::env::temp_dir().join(format!("agents-subagent-{:016x}", suffix));
            std::fs::create_dir_all(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
            let result = (|| {
                crate::source::git::clone(url, git_ref, &tmp)?;
                let file = tmp.join(path_in_source);
                std::fs::read_to_string(&file)
                    .with_context(|| format!("reading {}", file.display()))
            })();
            let _ = std::fs::remove_dir_all(&tmp);
            let body = result?;
            Ok((body, git_ref.map(|s| s.to_string())))
        }
        Source::Local { path } => {
            let abs = if path.is_absolute() {
                path.clone()
            } else {
                std::env::current_dir()?.join(path)
            };
            let file = if path_in_source.is_empty() {
                abs
            } else {
                abs.join(path_in_source)
            };
            if !file.exists() {
                bail!("local file not found: {}", file.display());
            }
            let body = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            Ok((body, None))
        }
    }
}

fn resolve_harnesses(requested: &[String], repo_cfg: &RepoConfig) -> Vec<String> {
    let want_all = requested.is_empty() || requested.iter().any(|r| r == "*");
    let candidates: Vec<String> = if want_all {
        repo_cfg.enabled_harnesses.clone()
    } else {
        requested.to_vec()
    };
    candidates
        .into_iter()
        .filter(|id| {
            crate::harness::lookup(id)
                .and_then(|d| d.subagents_subdir)
                .is_some()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// remove

pub fn remove(args: SubagentRemoveArgs, no_sync: bool) -> Result<()> {
    let repo = paths::resolve_repo()?;

    if git_sync::enabled(no_sync) {
        git_sync::pre_pull(&repo)?;
    }

    let mut lock = Lockfile::load(&repo)?;

    if !confirm_remove(&args)? {
        ui::warn("aborted");
        return Ok(());
    }

    let mut manifest = Manifest::load(&repo)?;
    let mut removed: Vec<String> = Vec::new();
    let mut had_error = false;

    for name in &args.names {
        if lock.find_subagent(name).is_none() {
            ui::warn(format!("subagent `{}` not in lockfile", name));
            continue;
        }
        match remove_one(&repo, name, &mut manifest) {
            Ok(()) => {
                lock.remove_subagent(name);
                removed.push(name.clone());
                ui::ok(format!("removed {}", name));
            }
            Err(e) => {
                had_error = true;
                ui::fail(format!("remove {} — {:#}", name, e));
            }
        }
    }

    if removed.is_empty() {
        if had_error {
            bail!("no subagents removed (all failed)");
        }
        return Ok(());
    }

    lock.write(&repo)?;
    manifest.write(&repo).context("writing manifest")?;

    if git_sync::enabled(no_sync) {
        let msg = msg_subagent_remove(&removed);
        if let Err(e) = git_sync::commit_and_push(&repo, &msg) {
            ui::warn(format!("auto-sync failed: {:#}", e));
            ui::detail("local change saved; rerun a mutating command to retry");
        }
    }

    Ok(())
}

fn remove_one(repo: &Path, name: &str, manifest: &mut Manifest) -> Result<()> {
    let snapshot = paths::local_subagent_path(repo, name);
    let rendered_root = subagent::rendered_root(repo);

    // Uninstall every rendered file agents wrote for this subagent.
    let to_remove: Vec<PathBuf> = manifest
        .entries
        .iter()
        .filter(|e| {
            e.skill == name && (e.target == snapshot || e.target.starts_with(&rendered_root))
        })
        .map(|e| e.path.clone())
        .collect();
    for path in &to_remove {
        let result = match manifest
            .entries
            .iter()
            .find(|e| e.path == *path)
            .map(|e| e.kind)
        {
            Some(EntryKind::Copy) => install::uninstall_copy(path),
            _ => install::uninstall_path(path),
        };
        let _ = result;
    }
    for entry in manifest
        .entries
        .iter()
        .filter(|e| e.skill == name && e.target.starts_with(&rendered_root) && e.target.exists())
    {
        let _ = std::fs::remove_file(&entry.target);
    }
    manifest.entries.retain(|e| !to_remove.contains(&e.path));

    if snapshot.exists() {
        std::fs::remove_file(&snapshot)
            .with_context(|| format!("removing snapshot {}", snapshot.display()))?;
    }
    Ok(())
}

fn confirm_remove(args: &SubagentRemoveArgs) -> Result<bool> {
    if args.yes {
        return Ok(true);
    }
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        return Ok(true);
    }
    ui::plain(format!(
        "remove {} subagent{}?",
        args.names.len(),
        if args.names.len() == 1 { "" } else { "s" }
    ));
    for n in &args.names {
        ui::detail(format!("  {}", n));
    }
    let mut buf = String::new();
    eprint!("[y/N] ");
    std::io::stdin()
        .read_line(&mut buf)
        .context("reading confirmation")?;
    Ok(matches!(
        buf.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

// ---------------------------------------------------------------------------
// list

pub fn list() -> Result<()> {
    let repo = paths::resolve_repo()?;
    let lock = Lockfile::load(&repo)?;
    if lock.subagents.is_empty() {
        ui::plain("no subagents locked");
        return Ok(());
    }
    for entry in &lock.subagents {
        let dot = if entry.active { "●" } else { "○" };
        let line = format!(
            "{} {}  {}",
            style(dot).cyan(),
            style(&entry.name).bold(),
            entry.source
        );
        ui::plain(line);
        if let Some(p) = &entry.path {
            ui::detail(format!("path: {}", p));
        }
        if !entry.profiles.is_empty() {
            ui::detail(format!("profiles: {}", entry.profiles.join(", ")));
        }
        if !entry.harnesses.iter().any(|h| h == "*") {
            ui::detail(format!("harnesses: {}", entry.harnesses.join(", ")));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// helpers

fn write_atomically(path: &Path, content: &str) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let suffix: u64 = rand::random();
    let stem = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = parent.join(format!(".{}.tmp.{:016x}", stem, suffix));
    std::fs::write(&tmp, content).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} → {}", tmp.display(), path.display()))?;
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().fold(String::with_capacity(64), |mut acc, b| {
        use std::fmt::Write;
        let _ = write!(acc, "{:02x}", b);
        acc
    })
}

fn msg_subagent_add(source: &str, names: &[String]) -> String {
    if names.len() == 1 {
        format!("subagent add: {} ({})", names[0], source)
    } else {
        format!("subagent add: {} ({})", names.join(", "), source)
    }
}

fn msg_subagent_remove(names: &[String]) -> String {
    if names.len() == 1 {
        format!("subagent remove: {}", names[0])
    } else {
        format!("subagent remove: {}", names.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn resolve_targets_with_explicit_path_uses_stem_for_name() {
        let args = SubagentAddArgs {
            source: "foo/bar".into(),
            subagent: vec![],
            path: Some("agents/some/code-reviewer.md".into()),
            harnesses: vec![],
            yes: false,
            profile: vec![],
            r#ref: None,
            dangerously_accept_openclaw_risks: false,
        };
        let src = Source::Github {
            owner: "foo".into(),
            repo: "bar".into(),
        };
        let targets = resolve_add_targets(&args, &src).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "code-reviewer");
        assert_eq!(targets[0].path_in_source, "agents/some/code-reviewer.md");
    }

    #[test]
    fn resolve_targets_path_with_subagent_override_uses_override() {
        let args = SubagentAddArgs {
            source: "foo/bar".into(),
            subagent: vec!["renamed".into()],
            path: Some("agents/code-reviewer.md".into()),
            harnesses: vec![],
            yes: false,
            profile: vec![],
            r#ref: None,
            dangerously_accept_openclaw_risks: false,
        };
        let src = Source::Github {
            owner: "foo".into(),
            repo: "bar".into(),
        };
        let targets = resolve_add_targets(&args, &src).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "renamed");
    }

    #[test]
    fn resolve_targets_default_path_under_agents_dir() {
        let args = SubagentAddArgs {
            source: "foo/bar".into(),
            subagent: vec!["a".into(), "b".into()],
            path: None,
            harnesses: vec![],
            yes: false,
            profile: vec![],
            r#ref: None,
            dangerously_accept_openclaw_risks: false,
        };
        let src = Source::Github {
            owner: "foo".into(),
            repo: "bar".into(),
        };
        let targets = resolve_add_targets(&args, &src).unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].path_in_source, "agents/a.md");
        assert_eq!(targets[1].path_in_source, "agents/b.md");
    }

    #[test]
    fn resolve_targets_local_md_file_uses_stem() {
        let args = SubagentAddArgs {
            source: "/tmp/foo.md".into(),
            subagent: vec![],
            path: None,
            harnesses: vec![],
            yes: false,
            profile: vec![],
            r#ref: None,
            dangerously_accept_openclaw_risks: false,
        };
        let src = Source::Local {
            path: PathBuf::from("/tmp/foo.md"),
        };
        let targets = resolve_add_targets(&args, &src).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "foo");
        assert_eq!(targets[0].path_in_source, "");
    }

    #[test]
    fn into_canonical_from_claude_promotes_model_and_effort_to_claude_slot() {
        let raw = "---\nname: r\ndescription: d\nmodel: sonnet\neffort: medium\ncolor: yellow\nskills:\n  - one\n  - two\n---\n\nbody\n";
        let s = into_canonical_from_claude("r", raw).unwrap();
        assert_eq!(s.frontmatter.name, "r");
        assert_eq!(s.frontmatter.description, "d");
        assert_eq!(s.frontmatter.model.claude.as_deref(), Some("sonnet"));
        assert_eq!(s.frontmatter.effort.claude.as_deref(), Some("medium"));
        assert_eq!(s.frontmatter.color.as_deref(), Some("yellow"));
        assert_eq!(s.frontmatter.skills, vec!["one", "two"]);
        assert_eq!(s.body.trim(), "body");
    }

    #[test]
    fn into_canonical_falls_back_to_target_name_when_frontmatter_missing_name() {
        let raw = "---\ndescription: d\n---\nbody\n";
        let s = into_canonical_from_claude("fallback", raw).unwrap();
        assert_eq!(s.frontmatter.name, "fallback");
    }

    #[test]
    fn into_canonical_rejects_missing_description() {
        let raw = "---\nname: r\n---\nbody\n";
        let err = into_canonical_from_claude("r", raw).unwrap_err();
        assert!(format!("{:#}", err).contains("description"));
    }

    #[test]
    fn sha256_hex_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
