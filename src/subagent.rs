//! Canonical multi-harness subagent representation and rendering.
//!
//! A subagent is one file at `<repo>/agents/<name>.md` with YAML frontmatter
//! describing the universal fields (`name`, `description`), per-harness model
//! and effort selectors, plus shared `skills` / `color`. The markdown body is
//! the system prompt.
//!
//! `apply` parses each canonical file and renders it into each enabled
//! harness's native format (Claude `.md`, Codex `.toml`, OpenCode `.md`,
//! Gemini `.md`). Rendering replaces symlinks for subagents — Codex needs
//! a different file format and field names, so a single-file symlink can't
//! serve both.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Parsed canonical subagent: frontmatter + body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subagent {
    pub frontmatter: SubagentFrontmatter,
    pub body: String,
}

/// All managed frontmatter fields. Anything we don't model is dropped on
/// re-render — Phase 1 only manages the fields we explicitly support.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentFrontmatter {
    pub name: String,
    pub description: String,

    #[serde(default, skip_serializing_if = "ModelMap::is_empty")]
    pub model: ModelMap,

    #[serde(default, skip_serializing_if = "EffortMap::is_empty")]
    pub effort: EffortMap,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelMap {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opencode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gemini: Option<String>,
}

impl ModelMap {
    pub fn is_empty(&self) -> bool {
        self.claude.is_none()
            && self.codex.is_none()
            && self.opencode.is_none()
            && self.gemini.is_none()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffortMap {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex: Option<String>,
}

impl EffortMap {
    pub fn is_empty(&self) -> bool {
        self.claude.is_none() && self.codex.is_none()
    }
}

// ---------------------------------------------------------------------------
// Parse / load

impl Subagent {
    /// Parse a canonical file: YAML frontmatter (between `---` markers) +
    /// markdown body. Body is trimmed of one leading blank line if present.
    pub fn parse(raw: &str) -> Result<Self> {
        let parsed = gray_matter::Matter::<gray_matter::engine::YAML>::new().parse(raw);
        let data = parsed
            .data
            .ok_or_else(|| anyhow!("missing YAML frontmatter (--- ... --- block)"))?;
        let frontmatter: SubagentFrontmatter = data.deserialize().context(
            "parsing canonical frontmatter — expected name/description/model/effort/skills/color",
        )?;
        if frontmatter.name.is_empty() {
            bail!("`name` is required");
        }
        if frontmatter.description.is_empty() {
            bail!("`description` is required");
        }
        Ok(Self {
            frontmatter,
            // Normalize: no leading/trailing blank lines in the struct. The
            // renderer adds a trailing `\n` when writing files, so equality
            // between two structs ignores end-of-file whitespace differences.
            body: parsed.content.trim_matches('\n').to_string(),
        })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    /// Serialize back to the canonical file format. Used by `add` to write the
    /// initial file from user input, and (later) by `sync` to write back after
    /// backfilling from harness edits.
    pub fn to_canonical(&self) -> Result<String> {
        let yaml = serde_yaml::to_string(&self.frontmatter).context("serializing frontmatter")?;
        let mut out = String::with_capacity(yaml.len() + self.body.len() + 16);
        out.push_str("---\n");
        out.push_str(&yaml);
        out.push_str("---\n\n");
        out.push_str(self.body.trim_start_matches('\n'));
        if !out.ends_with('\n') {
            out.push('\n');
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Render — one function per harness, returns the bytes to write.

/// Render the canonical for Claude Code: YAML frontmatter + markdown body.
pub fn render_claude(s: &Subagent) -> Result<String> {
    let mut fm = serde_yaml::Mapping::new();
    fm.insert("name".into(), s.frontmatter.name.clone().into());
    fm.insert(
        "description".into(),
        s.frontmatter.description.clone().into(),
    );
    if let Some(m) = &s.frontmatter.model.claude {
        fm.insert("model".into(), m.clone().into());
    }
    if let Some(e) = &s.frontmatter.effort.claude {
        fm.insert("effort".into(), e.clone().into());
    }
    if !s.frontmatter.skills.is_empty() {
        fm.insert(
            "skills".into(),
            serde_yaml::Value::Sequence(
                s.frontmatter
                    .skills
                    .iter()
                    .map(|v| serde_yaml::Value::String(v.clone()))
                    .collect(),
            ),
        );
    }
    if let Some(c) = &s.frontmatter.color {
        fm.insert("color".into(), c.clone().into());
    }
    let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(fm))
        .context("rendering claude frontmatter")?;
    let mut out = String::with_capacity(yaml.len() + s.body.len() + 16);
    out.push_str("---\n");
    out.push_str(&yaml);
    out.push_str("---\n\n");
    out.push_str(s.body.trim_start_matches('\n'));
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

/// Render the canonical for Codex: TOML with `developer_instructions` holding
/// the body. Field naming follows Codex's docs (snake_case, e.g.
/// `model_reasoning_effort`).
pub fn render_codex(s: &Subagent) -> Result<String> {
    let mut t = toml::value::Table::new();
    t.insert(
        "name".into(),
        toml::Value::String(s.frontmatter.name.clone()),
    );
    t.insert(
        "description".into(),
        toml::Value::String(s.frontmatter.description.clone()),
    );
    t.insert(
        "developer_instructions".into(),
        toml::Value::String(s.body.clone()),
    );
    if let Some(m) = &s.frontmatter.model.codex {
        t.insert("model".into(), toml::Value::String(m.clone()));
    }
    if let Some(e) = &s.frontmatter.effort.codex {
        t.insert(
            "model_reasoning_effort".into(),
            toml::Value::String(e.clone()),
        );
    }
    if !s.frontmatter.skills.is_empty() {
        let mut skills = toml::value::Table::new();
        skills.insert(
            "config".into(),
            toml::Value::Array(
                s.frontmatter
                    .skills
                    .iter()
                    .map(|v| toml::Value::String(v.clone()))
                    .collect(),
            ),
        );
        t.insert("skills".into(), toml::Value::Table(skills));
    }
    // color is not understood by Codex — drop silently
    toml::to_string_pretty(&toml::Value::Table(t)).context("serializing codex toml")
}

/// Render for OpenCode: YAML frontmatter (no `name` — derived from filename)
/// plus markdown body.
pub fn render_opencode(s: &Subagent) -> Result<String> {
    let mut fm = serde_yaml::Mapping::new();
    fm.insert(
        "description".into(),
        s.frontmatter.description.clone().into(),
    );
    if let Some(m) = &s.frontmatter.model.opencode {
        fm.insert("model".into(), m.clone().into());
    }
    if let Some(c) = &s.frontmatter.color {
        fm.insert("color".into(), c.clone().into());
    }
    // skills/effort not supported by OpenCode in our managed-field set
    let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(fm))
        .context("rendering opencode frontmatter")?;
    let mut out = String::with_capacity(yaml.len() + s.body.len() + 16);
    out.push_str("---\n");
    out.push_str(&yaml);
    out.push_str("---\n\n");
    out.push_str(s.body.trim_start_matches('\n'));
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

/// Render for Gemini: YAML frontmatter (with `name`) + markdown body.
pub fn render_gemini(s: &Subagent) -> Result<String> {
    let mut fm = serde_yaml::Mapping::new();
    fm.insert("name".into(), s.frontmatter.name.clone().into());
    fm.insert(
        "description".into(),
        s.frontmatter.description.clone().into(),
    );
    if let Some(m) = &s.frontmatter.model.gemini {
        fm.insert("model".into(), m.clone().into());
    }
    let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(fm))
        .context("rendering gemini frontmatter")?;
    let mut out = String::with_capacity(yaml.len() + s.body.len() + 16);
    out.push_str("---\n");
    out.push_str(&yaml);
    out.push_str("---\n\n");
    out.push_str(s.body.trim_start_matches('\n'));
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

/// Dispatch to the per-harness renderer. Returns `Ok(None)` if the harness
/// has no subagent install path (today: any harness whose `subagents_subdir`
/// is `None`).
pub fn render_for_harness(s: &Subagent, harness_id: &str) -> Result<Option<String>> {
    if crate::harness::lookup(harness_id)
        .and_then(|d| d.subagents_subdir)
        .is_none()
    {
        return Ok(None);
    }
    let body = match harness_id {
        "claude-code" => render_claude(s)?,
        "codex" => render_codex(s)?,
        "opencode" => render_opencode(s)?,
        "gemini" => render_gemini(s)?,
        other => bail!("no subagent renderer for harness `{}`", other),
    };
    Ok(Some(body))
}

/// File extension to use when writing for a harness. Codex is `.toml`;
/// every other harness uses `.md`.
pub fn harness_file_extension(harness_id: &str) -> &'static str {
    match harness_id {
        "codex" => "toml",
        _ => "md",
    }
}

/// Compute the install path for a subagent within a harness, picking the
/// right extension for that harness (`.toml` for Codex, `.md` for the rest).
/// Returns `Ok(None)` when the harness has no `subagents_subdir`.
pub fn harness_install_path(
    install_root: &Path,
    harness_id: &str,
    name: &str,
) -> Result<Option<std::path::PathBuf>> {
    let def = crate::harness::lookup(harness_id)
        .ok_or_else(|| anyhow!("unknown harness `{}`", harness_id))?;
    Ok(def.subagents_subdir.map(|sub| {
        install_root
            .join(sub)
            .join(format!("{}.{}", name, harness_file_extension(harness_id)))
    }))
}

pub fn rendered_root(repo: &Path) -> std::path::PathBuf {
    crate::paths::local_subagents_dir(repo).join("rendered")
}

pub fn rendered_path(repo: &Path, harness_id: &str, name: &str) -> std::path::PathBuf {
    rendered_root(repo).join(harness_id).join(format!(
        "{}.{}",
        name,
        harness_file_extension(harness_id)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Subagent {
        Subagent {
            frontmatter: SubagentFrontmatter {
                name: "code-reviewer".into(),
                description: "PR reviewer.".into(),
                model: ModelMap {
                    claude: Some("sonnet".into()),
                    codex: Some("gpt-5.3-codex-spark".into()),
                    opencode: None,
                    gemini: Some("gemini-2.5-pro".into()),
                },
                effort: EffortMap {
                    claude: Some("medium".into()),
                    codex: Some("medium".into()),
                },
                skills: vec!["code-review-checklist".into()],
                color: Some("yellow".into()),
            },
            body: "You are a code reviewer.".into(),
        }
    }

    #[test]
    fn parse_minimal_canonical() {
        let raw = "---\nname: foo\ndescription: bar\n---\n\nbody\n";
        let s = Subagent::parse(raw).unwrap();
        assert_eq!(s.frontmatter.name, "foo");
        assert_eq!(s.frontmatter.description, "bar");
        assert_eq!(s.body.trim(), "body");
        assert!(s.frontmatter.model.is_empty());
        assert!(s.frontmatter.effort.is_empty());
    }

    #[test]
    fn parse_rejects_missing_name() {
        let raw = "---\ndescription: bar\n---\nbody\n";
        let err = Subagent::parse(raw).unwrap_err();
        assert!(format!("{:#}", err).contains("name"));
    }

    #[test]
    fn parse_rejects_missing_description() {
        let raw = "---\nname: foo\n---\nbody\n";
        let err = Subagent::parse(raw).unwrap_err();
        assert!(format!("{:#}", err).contains("description"));
    }

    #[test]
    fn parse_rejects_no_frontmatter() {
        let raw = "no frontmatter here\n";
        let err = Subagent::parse(raw).unwrap_err();
        assert!(format!("{:#}", err).contains("frontmatter"));
    }

    #[test]
    fn round_trip_canonical() {
        let s = fixture();
        let canonical = s.to_canonical().unwrap();
        let reparsed = Subagent::parse(&canonical).unwrap();
        assert_eq!(reparsed, s);
    }

    #[test]
    fn render_claude_includes_managed_fields_only() {
        let out = render_claude(&fixture()).unwrap();
        assert!(out.contains("name: code-reviewer"));
        assert!(out.contains("description: PR reviewer."));
        assert!(out.contains("model: sonnet"));
        assert!(out.contains("effort: medium"));
        assert!(out.contains("color: yellow"));
        assert!(out.contains("skills:"));
        assert!(out.contains("code-review-checklist"));
        assert!(out.contains("You are a code reviewer."));
        // Codex-specific fields must not leak in
        assert!(!out.contains("gpt-5.3-codex-spark"));
        assert!(!out.contains("model_reasoning_effort"));
        // gemini-specific must not leak
        assert!(!out.contains("gemini-2.5-pro"));
    }

    #[test]
    fn render_codex_uses_toml_with_developer_instructions() {
        let out = render_codex(&fixture()).unwrap();
        assert!(out.contains("name = \"code-reviewer\""));
        assert!(out.contains("description = \"PR reviewer.\""));
        assert!(out.contains("developer_instructions ="));
        assert!(out.contains("You are a code reviewer."));
        assert!(out.contains("model = \"gpt-5.3-codex-spark\""));
        assert!(out.contains("model_reasoning_effort = \"medium\""));
        // skills.config arrives via [skills] table
        assert!(out.contains("config = [\"code-review-checklist\"]"));
        // claude/gemini selectors shouldn't leak
        assert!(!out.contains("sonnet"));
        assert!(!out.contains("gemini-2.5-pro"));
        // color is silently dropped — Codex doesn't define it
        assert!(!out.contains("yellow"));
    }

    #[test]
    fn render_opencode_omits_name_uses_description_and_model() {
        // OpenCode derives the name from the filename
        let mut s = fixture();
        s.frontmatter.model.opencode = Some("anthropic/claude-sonnet".into());
        let out = render_opencode(&s).unwrap();
        assert!(!out.contains("name:")); // name is filename
        assert!(out.contains("description: PR reviewer."));
        assert!(out.contains("model: anthropic/claude-sonnet"));
        assert!(out.contains("color: yellow"));
        assert!(!out.contains("model_reasoning_effort"));
        assert!(!out.contains("skills:"));
    }

    #[test]
    fn render_gemini_uses_name_description_model_only() {
        let out = render_gemini(&fixture()).unwrap();
        assert!(out.contains("name: code-reviewer"));
        assert!(out.contains("description: PR reviewer."));
        assert!(out.contains("model: gemini-2.5-pro"));
        assert!(out.contains("You are a code reviewer."));
        // Gemini doesn't get effort/skills/color in our managed set
        assert!(!out.contains("effort:"));
        assert!(!out.contains("skills:"));
        assert!(!out.contains("color:"));
    }

    #[test]
    fn render_for_harness_returns_none_for_unsupported() {
        let s = fixture();
        // All four harnesses now have subagents_subdir set, so all should render
        assert!(render_for_harness(&s, "claude-code").unwrap().is_some());
        assert!(render_for_harness(&s, "codex").unwrap().is_some());
        assert!(render_for_harness(&s, "opencode").unwrap().is_some());
        assert!(render_for_harness(&s, "gemini").unwrap().is_some());
    }

    #[test]
    fn harness_install_path_uses_toml_for_codex_md_for_others() {
        let root = std::path::PathBuf::from("/tmp/x");
        assert_eq!(
            harness_install_path(&root, "claude-code", "foo").unwrap(),
            Some(root.join(".claude/agents/foo.md")),
        );
        assert_eq!(
            harness_install_path(&root, "codex", "foo").unwrap(),
            Some(root.join(".codex/agents/foo.toml")),
        );
        assert_eq!(
            harness_install_path(&root, "opencode", "foo").unwrap(),
            Some(root.join(".config/opencode/agents/foo.md")),
        );
        assert_eq!(
            harness_install_path(&root, "gemini", "foo").unwrap(),
            Some(root.join(".gemini/agents/foo.md")),
        );
    }

    #[test]
    fn empty_per_harness_overrides_omit_fields() {
        let s = Subagent {
            frontmatter: SubagentFrontmatter {
                name: "minimal".into(),
                description: "Bare-bones.".into(),
                model: ModelMap::default(),
                effort: EffortMap::default(),
                skills: vec![],
                color: None,
            },
            body: "Hello.".into(),
        };
        let claude = render_claude(&s).unwrap();
        assert!(!claude.contains("model:"));
        assert!(!claude.contains("effort:"));
        assert!(!claude.contains("color:"));
        assert!(!claude.contains("skills:"));
        let codex = render_codex(&s).unwrap();
        assert!(!codex.contains("model = "));
        assert!(!codex.contains("model_reasoning_effort"));
        assert!(!codex.contains("[skills]"));
    }
}
