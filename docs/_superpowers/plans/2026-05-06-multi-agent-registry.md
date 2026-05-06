# Multi-agent registry implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace four scattered hardcoded `match agent { ... }` sites with a single in-process registry, then add OpenCode and Gemini CLI as new agents through it. All four agents end up enabled by default.

**Architecture:** Three independent phases. Phase 1 is a pure refactor — no behavior change, every existing test must pass unchanged. Phase 2 adds two registry rows (no other code changes). Phase 3 is docs.

**Tech Stack:** Rust 2021 edition, `cargo test` (no external test runner), `handlebars` for instructions templating, `directories` for path resolution.

**Spec:** [docs/_superpowers/specs/2026-05-06-multi-agent-registry-design.md](../specs/2026-05-06-multi-agent-registry-design.md)

---

## File structure

**New file:**
- `src/agents.rs` — `AgentDef` struct, per-agent `pub const` items, `REGISTRY` slice, `lookup`/`all`/`ids` helpers.

**Modified files (Phase 1, no behavior change):**
- `src/main.rs` — add `mod agents;`
- `src/paths.rs` — `agent_skill_path()` reads from registry
- `src/instructions.rs` — `Tool` becomes a newtype around `&'static AgentDef`; `build_context` and `reserved_identifiers` derive from registry
- `src/upstream.rs` — `build_index()` iterates registry's `upstream_indexer` callbacks
- `src/config.rs` — `default_agents()` derives from registry
- `src/commands/import.rs` — `agent_skill_dirs()` derives from registry; `Tool::Claude`/`Codex` callers updated to `Tool::CLAUDE`/`CODEX` consts
- `src/commands/apply_instructions.rs` — `Tool::Claude`/`Codex` test callers updated to `Tool::CLAUDE`/`CODEX`
- `src/commands/validate.rs` — accepts `Vec<&'static str>` from `reserved_identifiers()` instead of `&'static [&'static str]`

**Modified files (Phase 2, behavior change):**
- `src/agents.rs` — add `OPENCODE` and `GEMINI` const items, append to `REGISTRY`

**Modified files (Phase 3, docs only):**
- `docs/concepts/agents.md` — new page
- `docs/reference/lockfile.md` — list valid agent ids
- `WISHLIST.md` — strike "Multi-tool reach" line

---

## Phase 1: Refactor (no behavior change)

After every task in this phase, **all existing tests must pass with zero modifications.** That is the no-behavior-change contract.

### Task 1: Create `src/agents.rs` with the registry

**Files:**
- Create: `src/agents.rs`
- Modify: `src/main.rs` (add `mod agents;`)

- [ ] **Step 1: Find the right place in `main.rs` for the new module declaration**

Run: `head -40 src/main.rs`

Look for the existing `mod` declarations (e.g., `mod cli;`, `mod paths;`). The new line goes alphabetically with them.

- [ ] **Step 2: Write `src/agents.rs` with two registry rows (claude-code, codex)**

Create `src/agents.rs`:

```rust
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentDef {
    /// Stable agent identifier used in `ateam.toml` `enabled_agents` and
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

pub const CLAUDE_CODE: AgentDef = AgentDef {
    id: "claude-code",
    display: "Claude Code",
    skills_subdir: Some(".claude/skills"),
    instructions_file: Some(".claude/CLAUDE.md"),
    ctx_flag: "claude",
    upstream_indexer: Some(crate::upstream::index_claude_marketplaces),
};

pub const CODEX: AgentDef = AgentDef {
    id: "codex",
    display: "Codex",
    skills_subdir: Some(".codex/skills"),
    instructions_file: Some(".codex/AGENTS.md"),
    ctx_flag: "codex",
    upstream_indexer: None,
};

pub const REGISTRY: &[&AgentDef] = &[&CLAUDE_CODE, &CODEX];

/// Look up an agent definition by its stable id.
pub fn lookup(id: &str) -> Option<&'static AgentDef> {
    REGISTRY.iter().copied().find(|a| a.id == id)
}

/// All registered agents in registry order.
pub fn all() -> impl Iterator<Item = &'static AgentDef> {
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
}
```

- [ ] **Step 3: Add the module declaration to `src/main.rs`**

Find the `mod` block in `src/main.rs` and add `mod agents;` in alphabetical order. For example, between `mod cli;` and `mod commands;` if those exist.

Run: `grep -n "^mod " src/main.rs` to locate.

- [ ] **Step 4: Build and run the new tests**

Run: `cargo test --lib agents::tests`
Expected: 5 tests pass. Compilation succeeds.

- [ ] **Step 5: Run the full test suite to confirm no regressions**

Run: `cargo test`
Expected: all existing tests pass plus the 5 new ones.

- [ ] **Step 6: Commit**

```bash
git add src/agents.rs src/main.rs
git commit -m "feat(agents): introduce AgentDef registry module

Adds src/agents.rs with the AgentDef struct and a static REGISTRY
slice containing two rows: claude-code and codex. No callers yet —
subsequent tasks wire each match-arm site through the registry.

Refs spec: docs/_superpowers/specs/2026-05-06-multi-agent-registry-design.md"
```

---

### Task 2: Wire `paths::agent_skill_path` through the registry

**Files:**
- Modify: `src/paths.rs:139-146`

- [ ] **Step 1: Read the current implementation to confirm the line range**

Run: `sed -n '136,150p' src/paths.rs`
Expected: shows the `agent_skill_path` function with the `match agent { ... }` arms.

- [ ] **Step 2: Add a snapshot test asserting current paths**

Add this test inside the existing `#[cfg(test)] mod tests` block in `src/paths.rs` (or create one if absent):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn agent_skill_path_matches_known_layout() {
        let root = PathBuf::from("/tmp/install-root");
        assert_eq!(
            agent_skill_path(&root, "claude-code", "foo").unwrap(),
            PathBuf::from("/tmp/install-root/.claude/skills/foo"),
        );
        assert_eq!(
            agent_skill_path(&root, "codex", "foo").unwrap(),
            PathBuf::from("/tmp/install-root/.codex/skills/foo"),
        );
    }

    #[test]
    fn agent_skill_path_rejects_unknown_agent() {
        let root = PathBuf::from("/tmp/install-root");
        let err = agent_skill_path(&root, "no-such-agent", "foo").unwrap_err();
        assert!(format!("{err}").contains("unknown agent"));
    }
}
```

- [ ] **Step 3: Run the new tests against the current implementation**

Run: `cargo test --lib paths::tests::agent_skill_path`
Expected: PASS (the current match-arm implementation already produces these paths).

- [ ] **Step 4: Replace the function body to read from the registry**

Edit `src/paths.rs`. Find:

```rust
pub fn agent_skill_path(install_root: &Path, agent: &str, skill_name: &str) -> Result<PathBuf> {
    let agent_dir = match agent {
        "claude-code" => install_root.join(".claude").join("skills"),
        "codex" => install_root.join(".codex").join("skills"),
        other => return Err(anyhow!("unknown agent `{}`", other)),
    };
    Ok(agent_dir.join(skill_name))
}
```

Replace with:

```rust
pub fn agent_skill_path(install_root: &Path, agent: &str, skill_name: &str) -> Result<PathBuf> {
    let def = crate::agents::lookup(agent)
        .ok_or_else(|| anyhow!("unknown agent `{}`", agent))?;
    let subdir = def
        .skills_subdir
        .ok_or_else(|| anyhow!("agent `{}` has no skills directory", agent))?;
    Ok(install_root.join(subdir).join(skill_name))
}
```

- [ ] **Step 5: Run the snapshot tests against the new implementation**

Run: `cargo test --lib paths::tests::agent_skill_path`
Expected: PASS — paths are byte-identical.

- [ ] **Step 6: Run the full test suite**

Run: `cargo test`
Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/paths.rs
git commit -m "refactor(paths): route agent_skill_path through registry

No behavior change — paths for claude-code and codex are byte-identical.
Snapshot test asserts equivalence."
```

---

### Task 3: Refactor `Tool` enum to a newtype around `&'static AgentDef`

**Files:**
- Modify: `src/instructions.rs:12-51`

This is the largest file change in the plan. The existing call sites use `Tool::Claude` / `Tool::Codex` as variant names — those become `Tool::CLAUDE` / `Tool::CODEX` `pub const` items. Method signatures (`agent()`, `key()`, `output_subpath()`, `from_agent()`) stay byte-identical so callers compile unchanged. Only `Tool::all()` changes return type from `[Tool; 2]` to `Vec<Tool>` (callers iterate, so this is forward-compatible).

- [ ] **Step 1: Replace the enum definition and impl block**

In `src/instructions.rs`, find lines 12-51:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Claude,
    Codex,
}

impl Tool {
    pub fn agent(&self) -> &'static str {
        match self {
            Tool::Claude => "claude-code",
            Tool::Codex => "codex",
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            Tool::Claude => "claude",
            Tool::Codex => "codex",
        }
    }

    pub fn output_subpath(&self) -> &'static str {
        match self {
            Tool::Claude => ".claude/CLAUDE.md",
            Tool::Codex => ".codex/AGENTS.md",
        }
    }

    pub fn from_agent(agent: &str) -> Option<Self> {
        match agent {
            "claude-code" => Some(Tool::Claude),
            "codex" => Some(Tool::Codex),
            _ => None,
        }
    }

    pub fn all() -> [Tool; 2] {
        [Tool::Claude, Tool::Codex]
    }
}
```

Replace with:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tool(pub &'static crate::agents::AgentDef);

impl Tool {
    pub const CLAUDE: Tool = Tool(&crate::agents::CLAUDE_CODE);
    pub const CODEX: Tool = Tool(&crate::agents::CODEX);

    pub fn agent(&self) -> &'static str {
        self.0.id
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

    pub fn from_agent(agent: &str) -> Option<Self> {
        crate::agents::lookup(agent).map(Tool)
    }

    pub fn all() -> Vec<Tool> {
        crate::agents::all().map(Tool).collect()
    }
}
```

- [ ] **Step 2: Build to surface caller compile errors**

Run: `cargo build`
Expected: compile errors at sites referencing `Tool::Claude` / `Tool::Codex` (the enum variants no longer exist). The errors enumerate every call site that needs updating in Task 4.

Example error:
```
error[E0599]: no variant or associated item named `Claude` found for struct `Tool`
   --> src/commands/apply_instructions.rs:307:38
```

Note the file and line numbers — these are the sites Task 4 will fix. Do not fix them in this task; the test gate for this task is just that `src/instructions.rs` itself compiles in isolation.

- [ ] **Step 3: Build the library only**

Run: `cargo build --lib 2>&1 | grep "^error" | head -20`
Expected: errors are confined to caller sites, not within `src/instructions.rs`. The `Tool` newtype itself, `Tool::CLAUDE`, `Tool::CODEX`, and the methods all compile.

- [ ] **Step 4: Do not commit yet — proceed to Task 4**

The build is broken until callers are updated. Task 4 fixes them and the combined commit is one atomic refactor.

---

### Task 4: Update all `Tool::Claude` / `Tool::Codex` callers

**Files:**
- Modify: `src/commands/apply_instructions.rs:307,308,323,324,338,339,347,367,377,403,410`
- Modify: `src/commands/import.rs:312,313`
- Modify: `src/instructions.rs:293`

- [ ] **Step 1: Fix `src/commands/import.rs` production callers**

Find lines 312-313:

```rust
    let claude_path = instructions::output_path(home, Tool::Claude);
    let codex_path = instructions::output_path(home, Tool::Codex);
```

Replace with:

```rust
    let claude_path = instructions::output_path(home, Tool::CLAUDE);
    let codex_path = instructions::output_path(home, Tool::CODEX);
```

- [ ] **Step 2: Fix `src/commands/apply_instructions.rs` test callers**

Find every `Tool::Claude` and `Tool::Codex` in the file (lines 307, 308, 323, 324, 338, 339, 347, 367, 377, 403, 410). Replace `Tool::Claude` → `Tool::CLAUDE` and `Tool::Codex` → `Tool::CODEX`.

Run to verify all are caught:

```bash
grep -n "Tool::\(Claude\|Codex\)" src/commands/apply_instructions.rs
```

Expected before edit: 11 matches. Expected after edit: 0 matches.

You can do all replacements in one pass with `sed`:

```bash
sed -i.bak 's/Tool::Claude\b/Tool::CLAUDE/g; s/Tool::Codex\b/Tool::CODEX/g' src/commands/apply_instructions.rs && rm src/commands/apply_instructions.rs.bak
```

- [ ] **Step 3: Fix the `src/instructions.rs` test caller**

Find line 293:

```rust
        let ctx = build_context(&repo_cfg, &machine, "host-x", Tool::Claude);
```

Replace with:

```rust
        let ctx = build_context(&repo_cfg, &machine, "host-x", Tool::CLAUDE);
```

- [ ] **Step 4: Verify no more Tool::Claude / Tool::Codex remain anywhere**

Run: `grep -rn "Tool::\(Claude\|Codex\)\b" src/`
Expected: zero matches.

- [ ] **Step 5: Build the entire crate**

Run: `cargo build`
Expected: clean build, zero errors.

- [ ] **Step 6: Run the full test suite**

Run: `cargo test`
Expected: all tests pass. The newtype migration is behavior-preserving.

- [ ] **Step 7: Commit Tasks 3 + 4 together**

```bash
git add src/instructions.rs src/commands/apply_instructions.rs src/commands/import.rs
git commit -m "refactor(instructions): Tool becomes a newtype over &'static AgentDef

Replaces the Tool enum with a thin newtype wrapping a registry pointer.
Methods (agent, key, output_subpath, from_agent, all) keep their existing
signatures. Variant access Tool::Claude/Tool::Codex becomes pub const
items Tool::CLAUDE/Tool::CODEX. All 14 caller sites updated.

Tool::all() return type changes from [Tool; 2] to Vec<Tool> so the count
can grow without breaking signatures."
```

---

### Task 5: Refactor `build_context` to derive context flags from the registry

**Files:**
- Modify: `src/instructions.rs:67-85` (the `build_context` function)

- [ ] **Step 1: Read the existing function**

Run: `sed -n '66,86p' src/instructions.rs`

You should see the hardcoded inserts for `"claude"` and `"codex"`.

- [ ] **Step 2: Replace the hardcoded inserts with a registry loop**

Find:

```rust
pub fn build_context(
    repo_cfg: &RepoConfig,
    machine: &MachineConfig,
    hostname: &str,
    tool: Tool,
) -> Value {
    let mut ctx = serde_json::Map::new();
    for p in &repo_cfg.declared_profiles {
        let on = machine.profiles.iter().any(|m| m == p);
        ctx.insert(p.clone(), Value::Bool(on));
    }
    ctx.insert(
        "claude".into(),
        Value::Bool(matches!(tool, Tool::Claude)),
    );
    ctx.insert("codex".into(), Value::Bool(matches!(tool, Tool::Codex)));
    ctx.insert("hostname".into(), Value::String(hostname.to_string()));
    Value::Object(ctx)
}
```

Replace with:

```rust
pub fn build_context(
    repo_cfg: &RepoConfig,
    machine: &MachineConfig,
    hostname: &str,
    tool: Tool,
) -> Value {
    let mut ctx = serde_json::Map::new();
    for p in &repo_cfg.declared_profiles {
        let on = machine.profiles.iter().any(|m| m == p);
        ctx.insert(p.clone(), Value::Bool(on));
    }
    for t in Tool::all() {
        ctx.insert(t.key().into(), Value::Bool(t == tool));
    }
    ctx.insert("hostname".into(), Value::String(hostname.to_string()));
    Value::Object(ctx)
}
```

- [ ] **Step 3: Run the existing tests to confirm no regression**

Run: `cargo test --lib instructions::tests`
Expected: all instructions tests pass, including `build_context_sets_profile_booleans` and `render_tool_branch`.

- [ ] **Step 4: Commit**

```bash
git add src/instructions.rs
git commit -m "refactor(instructions): build_context derives ctx flags from registry

Replaces hardcoded inserts for 'claude' and 'codex' with a Tool::all()
loop using each tool's ctx_flag. Adding an agent now means a registry
row, not editing build_context."
```

---

### Task 6: Make `reserved_identifiers()` derive from the registry

**Files:**
- Modify: `src/instructions.rs:62-64`
- Modify: `src/commands/validate.rs:23,51`

`reserved_identifiers()` currently returns `&'static [&'static str]` from a hand-maintained literal. We change the return type to `Vec<&'static str>` (still `'static` lifetime on each element) so the contents can be derived from the registry at call time.

- [ ] **Step 1: Update `reserved_identifiers()` to derive from the registry**

In `src/instructions.rs`, find:

```rust
/// Reserved identifiers always available in the render context.
pub fn reserved_identifiers() -> &'static [&'static str] {
    &["claude", "codex", "hostname"]
}
```

Replace with:

```rust
/// Reserved identifiers always available in the render context.
/// One ctx_flag per agent in the registry, plus `"hostname"`.
pub fn reserved_identifiers() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = crate::agents::all().map(|a| a.ctx_flag).collect();
    v.push("hostname");
    v
}
```

- [ ] **Step 2: Update the validate.rs caller at line 23**

The current code:

```rust
    for r in instructions::reserved_identifiers() {
        allowed.insert((*r).into());
    }
```

Becomes (the dereference `(*r)` is no longer needed because `r` is now `&'static str` directly, not `&&'static str`):

```rust
    for r in instructions::reserved_identifiers() {
        allowed.insert(r.into());
    }
```

- [ ] **Step 3: Update the validate.rs caller at line 51**

The current code:

```rust
        instructions::reserved_identifiers().join(", ")
```

That call works on both `&[&str]` and `Vec<&str>`, so it should compile unchanged. Verify by building.

- [ ] **Step 4: Build and run all tests**

Run: `cargo test`
Expected: all tests pass. The validate.rs runtime behavior is unchanged because the registry today produces the same strings as the old literal: `["claude", "codex", "hostname"]`.

- [ ] **Step 5: Commit**

```bash
git add src/instructions.rs src/commands/validate.rs
git commit -m "refactor(instructions): reserved_identifiers derives from registry

Returns Vec<&'static str> instead of &'static [&'static str] so the
contents can be computed from the registry at call time. Validate.rs
caller adjusted for the type change."
```

---

### Task 7: Wire `upstream::build_index` through registry callbacks

**Files:**
- Modify: `src/upstream.rs:21-25` (the `build_index` function)

- [ ] **Step 1: Read the existing function**

Run: `sed -n '17,30p' src/upstream.rs`

You should see `build_index` calling `index_claude_marketplaces` directly.

- [ ] **Step 2: Replace the direct call with a registry loop**

Find:

```rust
/// One pass over the user's home directory; returns a map of skill-name →
/// source string (e.g., `github:owner/repo`). Cheap to call per import,
/// expensive to call per skill — call once, look up many.
pub fn build_index(home: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    index_claude_marketplaces(&mut map, home);
    map
}
```

Replace with:

```rust
/// One pass over the user's home directory; returns a map of skill-name →
/// source string (e.g., `github:owner/repo`). Cheap to call per import,
/// expensive to call per skill — call once, look up many.
///
/// Iterates the agent registry and invokes each agent's `upstream_indexer`
/// if it has one. Today only `claude-code` populates this — the loop has
/// one productive iteration and the rest no-op.
pub fn build_index(home: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for def in crate::agents::all() {
        if let Some(indexer) = def.upstream_indexer {
            indexer(home, &mut map);
        }
    }
    map
}
```

Note the parameter order in the signature stays (`home: &Path`, `map: &mut HashMap<...>`) for `index_claude_marketplaces` — but the registry stores `UpstreamIndexer = fn(&Path, &mut HashMap<String, String>)`, which means `(home, map)`. The existing function has signature `fn index_claude_marketplaces(map: &mut HashMap<String, String>, home: &Path)`. **The argument order is reversed.** Fix that next.

- [ ] **Step 3: Reorder the `index_claude_marketplaces` parameters**

Find the function definition (around line 53):

```rust
fn index_claude_marketplaces(map: &mut HashMap<String, String>, home: &Path) {
```

Change to:

```rust
fn index_claude_marketplaces(home: &Path, map: &mut HashMap<String, String>) {
```

The function visibility also needs to be `pub` so `agents.rs` can take its address:

```rust
pub fn index_claude_marketplaces(home: &Path, map: &mut HashMap<String, String>) {
```

- [ ] **Step 4: Build to surface any other callers**

Run: `cargo build`
Expected: clean build. There should be no other direct callers of `index_claude_marketplaces` — `build_index` was the only one.

If a caller surfaces, update its argument order too.

- [ ] **Step 5: Run the existing upstream tests**

Run: `cargo test --lib upstream::tests`
Expected: all 3 tests pass (`discovers_github_marketplace_plugin`, `empty_when_no_claude_dir`, `ignores_files_outside_claude_dir`).

- [ ] **Step 6: Run the full test suite**

Run: `cargo test`
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add src/upstream.rs
git commit -m "refactor(upstream): build_index iterates registry callbacks

Swaps the direct call to index_claude_marketplaces for a loop over the
agent registry, calling each agent's upstream_indexer if Some. Today
only claude-code has one — adding more is a registry-row change.

Also reorders index_claude_marketplaces parameters to (home, map) to
match the UpstreamIndexer fn pointer signature, and makes it pub."
```

---

### Task 8: Wire `config::default_agents` through the registry

**Files:**
- Modify: `src/config.rs:14-16`

- [ ] **Step 1: Replace the hardcoded vec**

In `src/config.rs`, find:

```rust
fn default_agents() -> Vec<String> {
    vec!["claude-code".into(), "codex".into()]
}
```

Replace with:

```rust
fn default_agents() -> Vec<String> {
    crate::agents::ids().map(String::from).collect()
}
```

- [ ] **Step 2: Add a test asserting current behavior**

Add inside the existing `#[cfg(test)] mod tests` block in `src/config.rs` (or create one if absent):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_agents_includes_claude_and_codex() {
        let defaults = default_agents();
        assert!(defaults.contains(&"claude-code".to_string()));
        assert!(defaults.contains(&"codex".to_string()));
    }

    #[test]
    fn default_agents_count_matches_registry() {
        assert_eq!(default_agents().len(), crate::agents::REGISTRY.len());
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib config::tests`
Expected: 2 new tests pass. The first proves we still include the existing two; the second pins the count to the registry so when Phase 2 lands it auto-grows.

- [ ] **Step 4: Run the full test suite**

Run: `cargo test`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "refactor(config): default_agents derives from registry

Returns the registry's ids in registry order. Today this is
['claude-code', 'codex'] — identical to the previous literal.
When Phase 2 adds OpenCode and Gemini, this grows automatically."
```

---

### Task 9: Wire `commands/import::agent_skill_dirs` through the registry

**Files:**
- Modify: `src/commands/import.rs:265-271` (the `agent_skill_dirs` helper)
- Modify: `src/commands/import.rs:61` and `:102` (user-facing scan path strings)

- [ ] **Step 1: Replace `agent_skill_dirs` with a registry-derived list**

Find:

```rust
fn agent_skill_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".claude").join("skills"),
        home.join(".codex").join("skills"),
        home.join(".agents").join("skills"),
    ]
}
```

Replace with:

```rust
fn agent_skill_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = crate::agents::all()
        .filter_map(|a| a.skills_subdir.map(|s| home.join(s)))
        .collect();
    // Cross-tool interop alias honored by Gemini and OpenCode but not
    // bound to any single agent in the registry.
    dirs.push(home.join(".agents").join("skills"));
    dirs
}
```

- [ ] **Step 2: Make the user-facing scan-path strings derive from the same source**

The current line 61 contains a hand-written list of paths in an error message:

```rust
            "no installed skill found named `{}` in ~/.claude/skills/, ~/.codex/skills/, or ~/.agents/skills/",
```

And line 102 has another:

```rust
    println!("ateam: scanning ~/.claude/skills, ~/.codex/skills, ~/.agents/skills...");
```

Replace line 61 (the `find_installed` error message) with:

```rust
            "no installed skill found named `{}` in {}",
            normalized,
            agent_skill_dirs(home)
                .iter()
                .map(|p| crate::paths::display_path(p))
                .collect::<Vec<_>>()
                .join(", "),
```

The `anyhow!` macro takes positional args, so the call site becomes:

```rust
    let installed = find_installed(home, &normalized).ok_or_else(|| {
        anyhow!(
            "no installed skill found named `{}` in {}",
            normalized,
            agent_skill_dirs(home)
                .iter()
                .map(|p| crate::paths::display_path(p))
                .collect::<Vec<_>>()
                .join(", "),
        )
    })?;
```

Replace line 102 with:

```rust
    println!(
        "ateam: scanning {}...",
        agent_skill_dirs(home)
            .iter()
            .map(|p| crate::paths::display_path(p))
            .collect::<Vec<_>>()
            .join(", "),
    );
```

- [ ] **Step 3: Build to surface any compile errors**

Run: `cargo build`
Expected: clean build. `crate::paths::display_path` already exists (per `src/paths.rs:156`).

- [ ] **Step 4: Run all import tests**

Run: `cargo test --lib commands::import::tests`
Expected: all 7 tests pass. The `bulk_imports_skills_from_both_dirs` test still finds skills under `.claude/skills/` and `.codex/skills/` because they're still in the registry-derived list.

- [ ] **Step 5: Run the full test suite**

Run: `cargo test`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src/commands/import.rs
git commit -m "refactor(import): agent_skill_dirs derives from registry

The bulk-scan and find-installed paths come from registry rows
(filtered to those with a skills_subdir) plus the cross-tool
.agents/skills/ alias. User-facing scan path strings derive from
the same source so they stay accurate as agents are added."
```

---

**Phase 1 complete.** At this point the codebase has identical behavior to before, but every per-agent decision routes through `src/agents.rs`. Next phase adds the new agent rows.

---

## Phase 2: Add OpenCode and Gemini CLI

### Task 10: Add `OPENCODE` and `GEMINI` registry rows

**Files:**
- Modify: `src/agents.rs` (add two `pub const` items + extend `REGISTRY`)

This is the moment behavior changes. Default-agent count grows from 2 to 4, and `ateam apply` on a default-config repo will materialize new files at `~/.config/opencode/AGENTS.md` and `~/.gemini/GEMINI.md`.

- [ ] **Step 1: Add the two new `pub const` items**

In `src/agents.rs`, after the `CODEX` const, add:

```rust
pub const OPENCODE: AgentDef = AgentDef {
    id: "opencode",
    display: "OpenCode",
    skills_subdir: Some(".config/opencode/skills"),
    instructions_file: Some(".config/opencode/AGENTS.md"),
    ctx_flag: "opencode",
    upstream_indexer: None,
};

pub const GEMINI: AgentDef = AgentDef {
    id: "gemini",
    display: "Gemini CLI",
    skills_subdir: Some(".gemini/skills"),
    instructions_file: Some(".gemini/GEMINI.md"),
    ctx_flag: "gemini",
    upstream_indexer: None,
};
```

- [ ] **Step 2: Extend the `REGISTRY` slice**

Find:

```rust
pub const REGISTRY: &[&AgentDef] = &[&CLAUDE_CODE, &CODEX];
```

Replace with:

```rust
pub const REGISTRY: &[&AgentDef] = &[&CLAUDE_CODE, &CODEX, &OPENCODE, &GEMINI];
```

- [ ] **Step 3: Add tests covering the new agents**

Add to the `#[cfg(test)] mod tests` block at the bottom of `src/agents.rs`:

```rust
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
```

- [ ] **Step 4: Add path tests for the new agents in `src/paths.rs`**

In the `#[cfg(test)] mod tests` block in `src/paths.rs`, extend `agent_skill_path_matches_known_layout`:

```rust
    #[test]
    fn agent_skill_path_matches_known_layout() {
        let root = PathBuf::from("/tmp/install-root");
        assert_eq!(
            agent_skill_path(&root, "claude-code", "foo").unwrap(),
            PathBuf::from("/tmp/install-root/.claude/skills/foo"),
        );
        assert_eq!(
            agent_skill_path(&root, "codex", "foo").unwrap(),
            PathBuf::from("/tmp/install-root/.codex/skills/foo"),
        );
        assert_eq!(
            agent_skill_path(&root, "opencode", "foo").unwrap(),
            PathBuf::from("/tmp/install-root/.config/opencode/skills/foo"),
        );
        assert_eq!(
            agent_skill_path(&root, "gemini", "foo").unwrap(),
            PathBuf::from("/tmp/install-root/.gemini/skills/foo"),
        );
    }
```

- [ ] **Step 5: Add a `build_context` test for OpenCode and Gemini in `src/instructions.rs`**

Add inside the existing test module:

```rust
    #[test]
    fn build_context_sets_opencode_and_gemini_flags() {
        let repo_cfg = RepoConfig::default();
        let machine = MachineConfig::default();
        let opencode_tool = Tool::from_agent("opencode").expect("opencode tool");
        let ctx = build_context(&repo_cfg, &machine, "h", opencode_tool);
        assert_eq!(ctx["opencode"], Value::Bool(true));
        assert_eq!(ctx["gemini"], Value::Bool(false));
        assert_eq!(ctx["claude"], Value::Bool(false));
        assert_eq!(ctx["codex"], Value::Bool(false));

        let gemini_tool = Tool::from_agent("gemini").expect("gemini tool");
        let ctx = build_context(&repo_cfg, &machine, "h", gemini_tool);
        assert_eq!(ctx["gemini"], Value::Bool(true));
        assert_eq!(ctx["opencode"], Value::Bool(false));
    }
```

- [ ] **Step 6: Update the existing config test to expect 4 agents in the default**

The test from Task 8 (`default_agents_includes_claude_and_codex`) still passes. Add an assertion that all four are present:

```rust
    #[test]
    fn default_agents_includes_all_four() {
        let defaults = default_agents();
        assert_eq!(defaults.len(), 4);
        assert!(defaults.contains(&"claude-code".to_string()));
        assert!(defaults.contains(&"codex".to_string()));
        assert!(defaults.contains(&"opencode".to_string()));
        assert!(defaults.contains(&"gemini".to_string()));
    }
```

- [ ] **Step 7: Update the existing `build_context_sets_profile_booleans` test**

The test asserts on a 2-agent context (`"claude-code"`, `"codex"`). With four agents in the registry, the new opencode/gemini keys are also inserted. Update the test to match — at line ~289 in `src/instructions.rs`:

Find:

```rust
        let repo_cfg = RepoConfig {
            declared_profiles: vec!["work".into(), "personal".into(), "devbox".into()],
            enabled_agents: vec!["claude-code".into(), "codex".into()],
        };
```

The `enabled_agents` here is just an arbitrary fixture for the test — the test asserts on context flags, not enabled_agents. Update to:

```rust
        let repo_cfg = RepoConfig {
            declared_profiles: vec!["work".into(), "personal".into(), "devbox".into()],
            enabled_agents: vec!["claude-code".into(), "codex".into(), "opencode".into(), "gemini".into()],
        };
```

The asserts on `ctx["claude"]` and `ctx["codex"]` still pass (still in the registry). Add asserts for the new flags:

```rust
        assert_eq!(ctx["opencode"], Value::Bool(false));
        assert_eq!(ctx["gemini"], Value::Bool(false));
```

- [ ] **Step 8: Run all tests**

Run: `cargo test`
Expected: all pass — old tests + the new opencode/gemini lookups + path tests + build_context tests + the updated default_agents test.

- [ ] **Step 9: Manual smoke test**

Build a release binary and run an end-to-end sanity check on the four-agent default. (Skip if you don't have an `~/.config/ateam/` repo — the unit tests cover the same paths.)

```bash
cargo build --release
ATEAM=$(pwd)/target/release/ateam
mkdir -p /tmp/ateam-smoke && cd /tmp/ateam-smoke
HOME=/tmp/ateam-smoke "$ATEAM" init --scaffold --profiles personal
HOME=/tmp/ateam-smoke "$ATEAM" apply
ls -la .claude .codex .config/opencode .gemini
```

Expected: instructions files materialize at all four paths.

- [ ] **Step 10: Commit**

```bash
git add src/agents.rs src/paths.rs src/instructions.rs src/config.rs
git commit -m "feat(agents): add OpenCode and Gemini CLI to the registry

Two new rows: opencode (.config/opencode/skills + AGENTS.md) and
gemini (.gemini/skills + GEMINI.md). Both use the same SKILL.md
format as Claude Code and Codex per the agentskills.io standard.

default_agents() now returns all four. Existing users with explicit
enabled_agents in ateam.toml keep their list; users on the default
will see new files at ~/.config/opencode/AGENTS.md and
~/.gemini/GEMINI.md after the next 'ateam apply'."
```

---

## Phase 3: Documentation

### Task 11: Add docs page, update lockfile reference, prune WISHLIST

**Files:**
- Create: `docs/concepts/agents.md`
- Modify: `docs/reference/lockfile.md`
- Modify: `WISHLIST.md`

- [ ] **Step 1: Create the agents concept page**

Create `docs/concepts/agents.md`:

```markdown
# Supported agents

ateam syncs skills and instructions across these agents. Each agent has a stable id used in `ateam.toml`'s `enabled_agents` list and in lockfile entries' `agents` field.

| id | tool | skills directory | global instructions file |
|---|---|---|---|
| `claude-code` | [Claude Code](https://claude.com/claude-code) | `~/.claude/skills/<name>/SKILL.md` | `~/.claude/CLAUDE.md` |
| `codex` | [OpenAI Codex CLI](https://github.com/openai/codex) | `~/.codex/skills/<name>/SKILL.md` | `~/.codex/AGENTS.md` |
| `opencode` | [OpenCode](https://opencode.ai) | `~/.config/opencode/skills/<name>/SKILL.md` | `~/.config/opencode/AGENTS.md` |
| `gemini` | [Gemini CLI](https://github.com/google-gemini/gemini-cli) | `~/.gemini/skills/<name>/SKILL.md` | `~/.gemini/GEMINI.md` |

All four use the same `SKILL.md` format ([agentskills.io](https://agentskills.io) open standard): a directory containing a `SKILL.md` file with YAML frontmatter (`name`, `description`) and optional bundled `scripts/`, `references/`, `assets/`.

## Default-enabled set

By default, all four agents are enabled. ateam's `apply` will write instructions files and install skill symlinks for each one — even if the agent itself isn't installed on this machine. To opt out, edit `ateam.toml`:

```toml
enabled_agents = ["claude-code", "codex"]  # only these two
```

## Per-skill agent gating

The lockfile's `agents` field on each skill entry restricts which agents that skill installs to. Use `["*"]` for all enabled agents, or list specific ids:

```toml
[[skills]]
name = "my-skill"
agents = ["claude-code", "codex"]   # skip opencode + gemini for this skill
```

## Adding support for another agent

Adding an agent is a one-row change in `src/agents.rs`. File an issue or open a PR with the agent's stable id, display name, skills subdir (relative to install root), and instructions file path. Cursor and Copilot are not currently supported because neither has a globally-syncable file surface (their global rules live in app settings).
```

- [ ] **Step 2: Update `docs/reference/lockfile.md`**

Find the section that describes valid values for the `agents` field on a skill entry. Add `opencode` and `gemini` to the enumerated list. (The exact phrasing depends on the existing doc — open the file, find the agent-id list, append the two new ids.)

Run: `grep -n "claude-code\|codex" docs/reference/lockfile.md` to locate.

If the doc currently says something like "Valid agent ids: `claude-code`, `codex`", change it to "Valid agent ids: `claude-code`, `codex`, `opencode`, `gemini`. See [agents](../concepts/agents.md) for the full table."

- [ ] **Step 3: Strike the "Multi-tool reach" line from `WISHLIST.md`**

In `WISHLIST.md`, find lines 46-47:

```markdown
## Multi-tool reach (later, do not pre-build for it)

Cursor, Windsurf, Cline, etc. are downstream of this. The skills.sh per-agent path conventions are already documented for ~55 agents, so adding a tool is mostly a path-mapping table entry. **Do not build pluggable-tool infrastructure now** — add tools when the need arises.
```

Replace with:

```markdown
## Multi-tool reach

Shipped in v0.3: OpenCode and Gemini CLI via the agent registry in `src/agents.rs`. Adding more is a one-row change. See [docs/concepts/agents.md](./docs/concepts/agents.md). Cursor and Copilot are intentionally out of scope — neither has a globally-syncable file surface today.
```

- [ ] **Step 4: Run the full test suite to confirm nothing else broke**

Run: `cargo test`
Expected: all pass (this task is docs only, but verify).

- [ ] **Step 5: Commit**

```bash
git add docs/concepts/agents.md docs/reference/lockfile.md WISHLIST.md
git commit -m "docs: document multi-agent support (OpenCode + Gemini CLI)

Adds a new concepts/agents.md page with the full table of supported
agents and their per-tool paths. Updates the lockfile reference to
list valid agent ids. Prunes the 'Multi-tool reach' WISHLIST entry
that this work resolves."
```

---

## Final verification

- [ ] **Step 1: Run the full test suite from a clean state**

```bash
cargo clean
cargo test
```

Expected: all tests pass on a cold build.

- [ ] **Step 2: Build the release binary**

```bash
cargo build --release
```

Expected: clean build with no warnings about the new code (existing warnings unchanged).

- [ ] **Step 3: Sanity-check the binary's behavior**

```bash
./target/release/ateam --help
```

Expected: command help renders correctly. The CLI surface itself didn't change — only the agent registry did.

- [ ] **Step 4: Diff against main to review the full set of changes**

```bash
git log main..HEAD --oneline
git diff main..HEAD --stat
```

Expected: ~11 commits (one per task), changes confined to:
- `src/agents.rs` (new)
- `src/main.rs` (one `mod` line)
- `src/paths.rs`, `src/instructions.rs`, `src/upstream.rs`, `src/config.rs`
- `src/commands/{apply_instructions,import,validate}.rs`
- `docs/concepts/agents.md` (new)
- `docs/reference/lockfile.md`, `WISHLIST.md`

If unrelated files appear in the diff, investigate before pushing.

---

## Self-review checklist for the implementing agent

Before declaring the work done:

1. Every new agent (opencode, gemini) has tests in at least three files: `agents.rs` (registry lookup), `paths.rs` (skill-path), `instructions.rs` (build_context flag).
2. `grep -rn "Tool::Claude\b\|Tool::Codex\b" src/` returns zero matches (all callers migrated).
3. `grep -rn "claude-code\|codex" src/` returns only registry-related matches in `src/agents.rs` and tests — no stragglers in production code outside the registry.
4. The CHANGELOG entry calls out the migration consequence: users on default config get new files at `~/.config/opencode/` and `~/.gemini/` after the next `apply`.
5. `cargo test` and `cargo build --release` both succeed on a clean checkout.
