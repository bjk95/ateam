use crate::config::{MachineConfig, RepoConfig};
use anyhow::{anyhow, Context, Result};
use handlebars::template::{HelperTemplate, Parameter, Template, TemplateElement};
use handlebars::{handlebars_helper, Handlebars};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub const TEMPLATE_FILENAME: &str = "instructions.md.hbs";
pub const TEMPLATE_DIR: &str = "instructions";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Harness(pub &'static crate::harness::HarnessDef);

impl Harness {
    pub const CLAUDE: Harness = Harness(&crate::harness::CLAUDE_CODE);
    pub const CODEX: Harness = Harness(&crate::harness::CODEX);

    pub fn id(&self) -> &'static str {
        self.0.id
    }

    pub fn display(&self) -> &'static str {
        self.0.display
    }

    pub fn key(&self) -> &'static str {
        self.0.ctx_flag
    }

    /// Path of the rendered instructions file under the install root.
    /// Panics if the agent has no `instructions_file`. All current agents
    /// have one — when that changes, callers should be migrated to handle
    /// the Option.
    pub fn output_subpath(&self) -> &'static str {
        self.0
            .instructions_file
            .expect("agent has no instructions_file")
    }

    pub fn from_id(id: &str) -> Option<Self> {
        crate::harness::lookup(id).map(Harness)
    }

    pub fn all() -> Vec<Harness> {
        crate::harness::all().map(Harness).collect()
    }
}

pub fn template_path(repo: &Path) -> PathBuf {
    repo.join(TEMPLATE_DIR).join(TEMPLATE_FILENAME)
}

pub fn output_path(home: &Path, harness: Harness) -> PathBuf {
    home.join(harness.output_subpath())
}

/// Reserved identifiers always available in the render context.
/// One ctx_flag per agent in the registry, plus `"hostname"`.
pub fn reserved_identifiers() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = crate::harness::all().map(|a| a.ctx_flag).collect();
    v.push("hostname");
    v
}

/// Build the render context for a single tool render.
pub fn build_context(
    repo_cfg: &RepoConfig,
    machine: &MachineConfig,
    hostname: &str,
    harness: Harness,
) -> Value {
    let mut ctx = serde_json::Map::new();
    for p in &repo_cfg.declared_profiles {
        let on = machine.profiles.iter().any(|m| m == p);
        ctx.insert(p.clone(), Value::Bool(on));
    }
    for t in Harness::all() {
        ctx.insert(t.key().into(), Value::Bool(t == harness));
    }
    ctx.insert("hostname".into(), Value::String(hostname.to_string()));
    Value::Object(ctx)
}

handlebars_helper!(or_helper: |*args| args.iter().any(|v| is_truthy(v)));
handlebars_helper!(and_helper: |*args| args.iter().all(|v| is_truthy(v)));
handlebars_helper!(not_helper: |v: Value| !is_truthy(&v));

fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

pub fn handlebars() -> Handlebars<'static> {
    let mut hb = Handlebars::new();
    hb.set_strict_mode(true);
    hb.register_helper("or", Box::new(or_helper));
    hb.register_helper("and", Box::new(and_helper));
    hb.register_helper("not", Box::new(not_helper));
    hb
}

pub fn render(template_src: &str, ctx: &Value) -> Result<String> {
    handlebars()
        .render_template(template_src, ctx)
        .map_err(|e| anyhow!("rendering instructions template: {e}"))
}

pub fn read_template(repo: &Path) -> Result<String> {
    let path = template_path(repo);
    std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))
}

/// Walk the template's AST and return identifiers referenced as variables that
/// are not in `allowed`. Helper names and built-in identifiers (`this`, `@key`,
/// etc.) are not flagged.
pub fn unknown_identifiers(
    template_src: &str,
    allowed: &BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    let tpl = Template::compile(template_src)
        .map_err(|e| anyhow!("parsing template: {e}"))?;
    let mut found = BTreeSet::new();
    walk_template(&tpl, &mut found);
    Ok(found
        .into_iter()
        .filter(|id| !allowed.contains(id))
        .collect())
}

fn walk_template(tpl: &Template, found: &mut BTreeSet<String>) {
    for el in &tpl.elements {
        walk_element(el, found);
    }
}

fn walk_element(el: &TemplateElement, found: &mut BTreeSet<String>) {
    match el {
        TemplateElement::Expression(h)
        | TemplateElement::HtmlExpression(h)
        | TemplateElement::HelperBlock(h) => walk_helper(h, found),
        _ => {}
    }
}

fn walk_helper(h: &HelperTemplate, found: &mut BTreeSet<String>) {
    let bare = h.params.is_empty() && h.hash.is_empty() && !h.block;
    if bare {
        if let Some(top) = param_top_segment(&h.name) {
            if is_collectible(top) {
                found.insert(top.to_string());
            }
        }
    }
    for p in &h.params {
        walk_param(p, found);
    }
    for v in h.hash.values() {
        walk_param(v, found);
    }
    if let Some(t) = &h.template {
        walk_template(t, found);
    }
    if let Some(t) = &h.inverse {
        walk_template(t, found);
    }
}

fn walk_param(p: &Parameter, found: &mut BTreeSet<String>) {
    match p {
        Parameter::Path(_) => {
            if let Some(top) = param_top_segment(p) {
                if is_collectible(top) {
                    found.insert(top.to_string());
                }
            }
        }
        Parameter::Subexpression(sub) => walk_element(sub.as_element(), found),
        Parameter::Name(_) | Parameter::Literal(_) => {}
        _ => {}
    }
}

fn param_top_segment(p: &Parameter) -> Option<&str> {
    let raw = p.as_name()?;
    top_segment(raw)
}

fn top_segment(raw: &str) -> Option<&str> {
    let trimmed = raw.trim_start_matches("./");
    let end = trimmed
        .find(|c: char| c == '.' || c == '[' || c == '/')
        .unwrap_or(trimmed.len());
    if end == 0 {
        None
    } else {
        Some(&trimmed[..end])
    }
}

fn is_collectible(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('@') && name != "this"
}

pub fn current_hostname() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn render_basic_profile_gate() {
        let src = "head\n{{#if work}}WORK{{/if}}\n{{#if personal}}HOME{{/if}}\ntail";
        let ctx = serde_json::json!({"work": true, "personal": false, "claude": true, "codex": false, "hostname": "h"});
        let out = render(src, &ctx).unwrap();
        assert!(out.contains("WORK"));
        assert!(!out.contains("HOME"));
    }

    #[test]
    fn render_or_helper() {
        let src = "{{#if (or work devbox)}}OK{{/if}}";
        let ctx = serde_json::json!({"work": false, "devbox": true});
        let out = render(src, &ctx).unwrap();
        assert!(out.contains("OK"));
    }

    #[test]
    fn render_tool_branch() {
        let src = "{{#if claude}}C{{/if}}{{#if codex}}X{{/if}}";
        let ctx_c = serde_json::json!({"claude": true, "codex": false});
        let ctx_x = serde_json::json!({"claude": false, "codex": true});
        assert_eq!(render(src, &ctx_c).unwrap(), "C");
        assert_eq!(render(src, &ctx_x).unwrap(), "X");
    }

    #[test]
    fn unknown_identifiers_flags_undeclared_profile() {
        let src = "{{#if work}}A{{/if}}{{#if mystery}}B{{/if}}";
        let allowed = allowed(&["work", "claude", "codex", "hostname"]);
        let unknown = unknown_identifiers(src, &allowed).unwrap();
        assert_eq!(unknown.iter().cloned().collect::<Vec<_>>(), vec!["mystery"]);
    }

    #[test]
    fn unknown_identifiers_inside_subexpressions() {
        let src = "{{#if (or work mystery)}}A{{/if}}";
        let allowed = allowed(&["work", "claude", "codex", "hostname"]);
        let unknown = unknown_identifiers(src, &allowed).unwrap();
        assert_eq!(unknown.iter().cloned().collect::<Vec<_>>(), vec!["mystery"]);
    }

    #[test]
    fn bare_variable_collected() {
        let src = "host: {{hostname}} machine: {{mystery}}";
        let allowed = allowed(&["hostname"]);
        let unknown = unknown_identifiers(src, &allowed).unwrap();
        assert_eq!(unknown.iter().cloned().collect::<Vec<_>>(), vec!["mystery"]);
    }

    #[test]
    fn helper_names_not_flagged_as_variables() {
        // `if` and `or` are helpers; only `work` and `devbox` are variables.
        let src = "{{#if (or work devbox)}}x{{/if}}";
        let allowed = allowed(&["work", "devbox"]);
        let unknown = unknown_identifiers(src, &allowed).unwrap();
        assert!(unknown.is_empty(), "got unknown: {:?}", unknown);
    }

    #[test]
    fn build_context_sets_profile_booleans() {
        let repo_cfg = RepoConfig {
            declared_profiles: vec!["work".into(), "personal".into(), "devbox".into()],
            enabled_harnesses: vec![
                "claude-code".into(),
                "codex".into(),
                "opencode".into(),
                "gemini".into(),
            ],
        };
        let mut machine = MachineConfig::default();
        machine.profiles = vec!["work".into()];
        let ctx = build_context(&repo_cfg, &machine, "host-x", Harness::CLAUDE);
        assert_eq!(ctx["work"], Value::Bool(true));
        assert_eq!(ctx["personal"], Value::Bool(false));
        assert_eq!(ctx["devbox"], Value::Bool(false));
        assert_eq!(ctx["claude"], Value::Bool(true));
        assert_eq!(ctx["codex"], Value::Bool(false));
        assert_eq!(ctx["opencode"], Value::Bool(false));
        assert_eq!(ctx["gemini"], Value::Bool(false));
        assert_eq!(ctx["hostname"], Value::String("host-x".into()));
    }

    #[test]
    fn build_context_sets_opencode_and_gemini_flags() {
        let repo_cfg = RepoConfig::default();
        let machine = MachineConfig::default();
        let opencode_harness = Harness::from_id("opencode").expect("opencode tool");
        let ctx = build_context(&repo_cfg, &machine, "h", opencode_harness);
        assert_eq!(ctx["opencode"], Value::Bool(true));
        assert_eq!(ctx["gemini"], Value::Bool(false));
        assert_eq!(ctx["claude"], Value::Bool(false));
        assert_eq!(ctx["codex"], Value::Bool(false));

        let gemini_harness = Harness::from_id("gemini").expect("gemini tool");
        let ctx = build_context(&repo_cfg, &machine, "h", gemini_harness);
        assert_eq!(ctx["gemini"], Value::Bool(true));
        assert_eq!(ctx["opencode"], Value::Bool(false));
    }
}
