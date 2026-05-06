use crate::cli::ListArgs;
use crate::lockfile::{Lockfile, SkillEntry};
use crate::paths;
use crate::ui;
use anyhow::Result;
use console::style;
use serde::Serialize;

pub fn run(args: ListArgs) -> Result<()> {
    let repo = paths::resolve_repo()?;
    let lock = Lockfile::load(&repo)?;

    let mut entries: Vec<&SkillEntry> = lock
        .skills
        .iter()
        .filter(|s| match (&args.project, &s.project) {
            (None, _) => true,
            (Some(filter), Some(p)) => filter == p,
            (Some(_), None) => false,
        })
        .collect();
    sort_entries(&mut entries);

    if args.json {
        return print_json(&entries);
    }

    if args.names {
        for s in &entries {
            println!("{}", s.name);
        }
        return Ok(());
    }

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
        let head = format!("{}  {}", style(padded_name).bold(), render_source(s));
        let line = if s.active {
            head
        } else {
            format!("{}  {}", head, style("[off]").dim())
        };
        ui::plain(line);

        // Verbose: append a dim qualifier line if scope or profiles is non-default.
        let scope_part = s.project.as_ref().map(|p| format!("scope: project={}", p));
        let profiles_part = if s.profiles.is_empty() {
            None
        } else {
            Some(format!("profiles: {}", s.profiles.join(",")))
        };
        let parts: Vec<String> = [scope_part, profiles_part].into_iter().flatten().collect();
        if !parts.is_empty() {
            ui::detail(parts.join(" · "));
        }
    }

    Ok(())
}

fn sort_entries(entries: &mut [&SkillEntry]) {
    entries.sort_by(|a, b| a.source.cmp(&b.source).then_with(|| a.name.cmp(&b.name)));
}

fn print_json(entries: &[&SkillEntry]) -> Result<()> {
    let out = JsonListOutput {
        version: 1,
        skills: entries.iter().map(|s| JsonSkill::from(*s)).collect(),
    };
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Stable JSON schema for `skills list --json`.
///
/// Bump `version` and add a migration note before changing field semantics
/// or removing fields. Adding new optional fields is backwards-compatible.
#[derive(Serialize)]
struct JsonListOutput<'a> {
    version: u32,
    skills: Vec<JsonSkill<'a>>,
}

/// Per-skill view. Every field is always emitted (no skip_serializing_if) so
/// consumers can rely on a fixed schema. `Option`s serialize to `null` and
/// empty `Vec`s serialize to `[]`.
#[derive(Serialize)]
struct JsonSkill<'a> {
    name: &'a str,
    source: &'a str,
    #[serde(rename = "ref")]
    git_ref: Option<&'a str>,
    tree_sha: Option<&'a str>,
    path: Option<&'a str>,
    agents: &'a [String],
    profiles: &'a [String],
    project: Option<&'a str>,
    active: bool,
    upstream: Option<&'a str>,
}

impl<'a> From<&'a SkillEntry> for JsonSkill<'a> {
    fn from(s: &'a SkillEntry) -> Self {
        Self {
            name: &s.name,
            source: &s.source,
            git_ref: s.git_ref.as_deref(),
            tree_sha: s.tree_sha.as_deref(),
            path: s.path.as_deref(),
            agents: &s.agents,
            profiles: &s.profiles,
            project: s.project.as_deref(),
            active: s.active,
            upstream: s.upstream.as_deref(),
        }
    }
}

fn render_source(s: &SkillEntry) -> String {
    let base: String = if let Some(rest) = s.source.strip_prefix("github:") {
        format!("{}", style(rest).cyan())
    } else if s.source.starts_with("local:") {
        let local = format!("{}", style("local").dim());
        match &s.upstream {
            Some(up) => format!("{}  {}  {}", local, style("←").dim(), render_upstream(up)),
            None => local,
        }
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

fn render_upstream(up: &str) -> String {
    if let Some(rest) = up.strip_prefix("github:") {
        format!("{}", style(rest).cyan())
    } else if let Some(url) = up.strip_prefix("git:") {
        format!("{}", style(url).cyan())
    } else {
        format!("{}", style(up).cyan())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn entry(name: &str) -> SkillEntry {
        SkillEntry {
            name: name.into(),
            source: "github:foo/bar".into(),
            path: None,
            git_ref: None,
            tree_sha: None,
            agents: vec!["*".into()],
            profiles: vec![],
            project: None,
            active: true,
            upstream: None,
        }
    }

    #[test]
    fn json_empty_list_has_versioned_envelope() {
        let entries: Vec<&SkillEntry> = Vec::new();
        let out = JsonListOutput {
            version: 1,
            skills: entries.iter().map(|s| JsonSkill::from(*s)).collect(),
        };
        let s = serde_json::to_string(&out).unwrap();
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["version"], 1);
        assert_eq!(v["skills"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn json_skill_includes_all_fields_even_when_default() {
        let e = entry("alpha");
        let view = JsonSkill::from(&e);
        let v = serde_json::to_value(&view).unwrap();
        // Required fields always present (no skip_serializing_if), nulls explicit.
        for key in [
            "name", "source", "ref", "tree_sha", "path", "agents", "profiles", "project", "active",
            "upstream",
        ] {
            assert!(v.get(key).is_some(), "missing key: {}", key);
        }
        assert_eq!(v["name"], "alpha");
        assert_eq!(v["source"], "github:foo/bar");
        assert!(v["ref"].is_null());
        assert!(v["tree_sha"].is_null());
        assert!(v["path"].is_null());
        assert_eq!(v["agents"], serde_json::json!(["*"]));
        assert_eq!(v["profiles"], serde_json::json!([]));
        assert!(v["project"].is_null());
        assert_eq!(v["active"], true);
        assert!(v["upstream"].is_null());
    }

    #[test]
    fn json_skill_renames_git_ref_to_ref() {
        let mut e = entry("alpha");
        e.git_ref = Some("v1.2.3".into());
        let view = JsonSkill::from(&e);
        let v = serde_json::to_value(&view).unwrap();
        assert_eq!(v["ref"], "v1.2.3");
    }

    #[test]
    fn sort_entries_orders_by_source_then_name() {
        let mut a = entry("zebra");
        a.source = "github:aa/aa".into();
        let mut b = entry("alpha");
        b.source = "github:bb/bb".into();
        let mut c = entry("beta");
        c.source = "github:aa/aa".into();
        let mut d = entry("alpha");
        d.source = "github:aa/aa".into();
        let mut entries: Vec<&SkillEntry> = vec![&a, &b, &c, &d];
        sort_entries(&mut entries);
        let got: Vec<(&str, &str)> = entries
            .iter()
            .map(|e| (e.source.as_str(), e.name.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("github:aa/aa", "alpha"),
                ("github:aa/aa", "beta"),
                ("github:aa/aa", "zebra"),
                ("github:bb/bb", "alpha"),
            ]
        );
    }

    #[test]
    fn json_skill_inactive_round_trips() {
        let mut e = entry("alpha");
        e.active = false;
        let view = JsonSkill::from(&e);
        let v = serde_json::to_value(&view).unwrap();
        assert_eq!(v["active"], false);
    }
}
