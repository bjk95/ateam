//! Per-agent registry. Replaces the hardcoded `match agent { ... }` arms
//! that used to live in paths.rs, instructions.rs, upstream.rs, config.rs,
//! and commands/import.rs.
//!
//! Adding a new supported agent is a one-row change here. No other file
//! needs to learn about it.

use std::collections::HashMap;
use std::path::Path;

/// Function signature for agent-specific upstream-source indexers (e.g., the
/// Claude marketplace scanner that maps installed-skill names back to their
/// origin git repos). Today only `claude-code` populates this.
pub type UpstreamIndexer = fn(&Path, &mut HashMap<String, String>);

/// One row of the registry. All paths are relative to an install root
/// (typically `$HOME` for global installs or a project root for per-project
/// installs). Both `skills_subdir` and `instructions_file` are `Option` so
/// future agents that surface only one of the two can be added cleanly.
///
/// `PartialEq` is implemented by `id` only — fn pointers in `upstream_indexer`
/// would otherwise produce an `unpredictable_function_pointer_comparisons`
/// warning, and id-equality is the right semantic anyway (two rows with the
/// same id are the same agent).
#[derive(Debug, Clone, Copy)]
pub struct HarnessDef {
    /// Stable agent identifier used in `ateam.toml` `enabled_harnesses` and
    /// in lockfile `agents` lists. Examples: `"claude-code"`, `"codex"`.
    pub id: &'static str,
    /// Human-readable name for UI surfaces.
    pub display: &'static str,
    /// Skills directory under the install root, or `None` if this agent
    /// has no skills concept.
    pub skills_subdir: Option<&'static str>,
    /// Global instructions file under the install root, or `None`.
    pub instructions_file: Option<&'static str>,
    /// Handlebars context flag (e.g., `{{#if claude}}`).
    pub ctx_flag: &'static str,
    /// Optional upstream-source indexer. The indexer populates a
    /// skill-name → source-string map for skills installed via that
    /// agent's plugin/marketplace mechanism.
    pub upstream_indexer: Option<UpstreamIndexer>,
}

pub const CLAUDE_CODE: HarnessDef = HarnessDef {
    id: "claude-code",
    display: "Claude Code",
    skills_subdir: Some(".claude/skills"),
    instructions_file: Some(".claude/CLAUDE.md"),
    ctx_flag: "claude",
    upstream_indexer: Some(crate::upstream::index_claude_marketplaces),
};

pub const CODEX: HarnessDef = HarnessDef {
    id: "codex",
    display: "Codex",
    skills_subdir: Some(".codex/skills"),
    instructions_file: Some(".codex/AGENTS.md"),
    ctx_flag: "codex",
    upstream_indexer: None,
};

pub const OPENCODE: HarnessDef = HarnessDef {
    id: "opencode",
    display: "OpenCode",
    skills_subdir: Some(".config/opencode/skills"),
    instructions_file: Some(".config/opencode/AGENTS.md"),
    ctx_flag: "opencode",
    upstream_indexer: None,
};

pub const GEMINI: HarnessDef = HarnessDef {
    id: "gemini",
    display: "Gemini CLI",
    skills_subdir: Some(".gemini/skills"),
    instructions_file: Some(".gemini/GEMINI.md"),
    ctx_flag: "gemini",
    upstream_indexer: None,
};

pub const REGISTRY: &[&HarnessDef] = &[&CLAUDE_CODE, &CODEX, &OPENCODE, &GEMINI];

impl PartialEq for HarnessDef {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for HarnessDef {}

/// Look up an agent definition by its stable id.
pub fn lookup(id: &str) -> Option<&'static HarnessDef> {
    REGISTRY.iter().copied().find(|a| a.id == id)
}

/// All registered agents in registry order.
pub fn all() -> impl Iterator<Item = &'static HarnessDef> {
    REGISTRY.iter().copied()
}

/// All registered agent ids in registry order.
pub fn ids() -> impl Iterator<Item = &'static str> {
    all().map(|a| a.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_lookup_round_trip() {
        for def in REGISTRY {
            assert_eq!(lookup(def.id), Some(*def));
        }
    }

    #[test]
    fn lookup_returns_none_for_unknown() {
        assert!(lookup("not-a-real-agent").is_none());
    }

    #[test]
    fn registry_contains_claude_and_codex() {
        let names: Vec<&str> = ids().collect();
        assert!(names.contains(&"claude-code"));
        assert!(names.contains(&"codex"));
    }

    #[test]
    fn claude_code_def_has_marketplace_indexer() {
        let def = lookup("claude-code").unwrap();
        assert!(def.upstream_indexer.is_some());
    }

    #[test]
    fn codex_def_has_no_marketplace_indexer() {
        let def = lookup("codex").unwrap();
        assert!(def.upstream_indexer.is_none());
    }

    #[test]
    fn opencode_lookup() {
        let def = lookup("opencode").expect("opencode in registry");
        assert_eq!(def.id, "opencode");
        assert_eq!(def.skills_subdir, Some(".config/opencode/skills"));
        assert_eq!(def.instructions_file, Some(".config/opencode/AGENTS.md"));
        assert_eq!(def.ctx_flag, "opencode");
        assert!(def.upstream_indexer.is_none());
    }

    #[test]
    fn gemini_lookup() {
        let def = lookup("gemini").expect("gemini in registry");
        assert_eq!(def.id, "gemini");
        assert_eq!(def.skills_subdir, Some(".gemini/skills"));
        assert_eq!(def.instructions_file, Some(".gemini/GEMINI.md"));
        assert_eq!(def.ctx_flag, "gemini");
        assert!(def.upstream_indexer.is_none());
    }

    #[test]
    fn registry_has_four_agents() {
        assert_eq!(REGISTRY.len(), 4);
    }
}
