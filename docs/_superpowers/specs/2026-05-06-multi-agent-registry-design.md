# Multi-agent registry: extend agents to OpenCode and Gemini CLI

**Date:** 2026-05-06
**Status:** Approved, awaiting implementation plan

## Goal

Replace the four scattered `match agent { "claude-code" | "codex" }` sites in the codebase with a single in-process registry, then add OpenCode and Gemini CLI as the first two agents through it.

The motivating shape is from `WISHLIST.md`:

> Cursor, Windsurf, Cline, etc. are downstream of this. The skills.sh per-agent path conventions are already documented for ~55 agents, so adding a tool is mostly a path-mapping table entry. **Do not build pluggable-tool infrastructure now** — add tools when the need arises.

The need has arisen. Live verification of the four candidate agents (OpenCode, Gemini CLI, Cursor, Copilot) against current docs surfaced two findings that shaped the scope:

1. **OpenCode and Gemini CLI both implement the same `SKILL.md` open standard as Claude Code and Codex.** Frontmatter (`name`, `description`), directory structure, name validation regex are identical. They are pure path-mapping additions — no new artifact type, no translation, no behavior change to agents's install/symlink logic.
2. **Cursor and Copilot are not file-syncable globally today.** Cursor User Rules live in app settings, not a file. Copilot personal instructions live in VS Code settings. Project-level rules live inside the project repo, outside agents's sync scope. They are out of scope for this spec — see "Out of scope" below.

## Scope

**In scope:**

- New `src/agents.rs` module exposing a `&'static [AgentDef]` registry.
- Refactor of four call sites (`paths.rs`, `instructions.rs`, `upstream.rs`, `config.rs`, `commands/import.rs`) to read the registry instead of hardcoded match arms. No behavior change for existing users on the existing two agents.
- Two new registry rows: OpenCode and Gemini CLI.
- All four agents (Claude Code, Codex, OpenCode, Gemini CLI) enabled by default.
- Docs page and one WISHLIST.md update.

**Out of scope (do not slip back in):**

- **Cursor and Copilot.** No globally-syncable file surface today. Revisit if/when those tools add one, or as a separate "project-level install" feature with its own spec.
- **Single-file rules / `.mdc` / `.instructions.md` artifact types.** agents's skill model is "directory containing SKILL.md". Single-file rules are a different artifact and would need their own design.
- **Translation between formats.** No conversion from `SKILL.md` to Cursor `.mdc` or to Copilot `.instructions.md`.
- **Runtime-extensible registry.** No TOML config defining custom agents, no plugin loading, no `Box<dyn Agent>`. The registry is a `&'static [AgentDef]` compiled into the binary. Users wanting another agent file an issue.
- **Per-agent disable command.** If `agents.toml` does not already support `enabled_agents` editing through a CLI command today, the implementation plan can either add one or document the manual edit. Either is acceptable.
- **Auto-discovery.** agents does not check whether `~/.gemini/` or `~/.config/opencode/` exists before enabling. Files materialize on `apply` regardless of whether the user has those tools installed. This is a deliberate consequence of "all enabled by default" — see Migration notes.

## High-level flow

The change happens in three independently-shippable phases.

```
Phase 1: Refactor (no behavior change, fully test-covered)
  ├─ src/agents.rs           NEW   AgentDef struct + REGISTRY const + lookup()
  ├─ src/paths.rs            CHG   agent_skill_path() routes through registry
  ├─ src/instructions.rs     CHG   Tool becomes a newtype around &'static AgentDef
  ├─ src/upstream.rs         CHG   Claude indexer becomes a registry callback
  ├─ src/config.rs           CHG   default_agents() reads from registry
  └─ src/commands/import.rs  CHG   bulk-scan iterates registry instead of hardcoded list

Phase 2: Add 2 rows to REGISTRY
  ├─ opencode    .config/opencode/skills/  +  .config/opencode/AGENTS.md
  └─ gemini      .gemini/skills/           +  .gemini/GEMINI.md

Phase 3: Docs
  ├─ docs/concepts/agents.md            NEW   table of supported agents and what each syncs
  ├─ docs/reference/lockfile.md         CHG   list new valid agent ids
  └─ WISHLIST.md                        CHG   strike "Multi-tool reach (later)" line
```

After Phase 1 every existing test should pass with zero modifications. After Phase 2 the four-agent set ships. Phase 3 is paperwork.

## Components

### 1. `src/agents.rs` — new module

**Public surface:**

```rust
pub struct AgentDef {
    pub id: &'static str,                        // "claude-code", "codex", "opencode", "gemini"
    pub display: &'static str,                   // "Claude Code", "Codex", "OpenCode", "Gemini CLI"
    pub skills_subdir: Option<&'static str>,     // path under install root; None if no skills concept
    pub instructions_file: Option<&'static str>, // path under install root; None if no global file
    pub ctx_flag: &'static str,                  // Handlebars flag, e.g. {{#if claude}} {{#if gemini}}
    pub upstream_indexer: Option<UpstreamIndexer>, // currently only claude-code populates this
}

pub type UpstreamIndexer = fn(&Path, &mut HashMap<String, String>);

pub const REGISTRY: &[AgentDef] = &[
    AgentDef {
        id: "claude-code",
        display: "Claude Code",
        skills_subdir: Some(".claude/skills"),
        instructions_file: Some(".claude/CLAUDE.md"),
        ctx_flag: "claude",
        upstream_indexer: Some(crate::upstream::index_claude_marketplaces),
    },
    AgentDef {
        id: "codex",
        display: "Codex",
        skills_subdir: Some(".codex/skills"),
        instructions_file: Some(".codex/AGENTS.md"),
        ctx_flag: "codex",
        upstream_indexer: None,
    },
    AgentDef {
        id: "opencode",
        display: "OpenCode",
        skills_subdir: Some(".config/opencode/skills"),
        instructions_file: Some(".config/opencode/AGENTS.md"),
        ctx_flag: "opencode",
        upstream_indexer: None,
    },
    AgentDef {
        id: "gemini",
        display: "Gemini CLI",
        skills_subdir: Some(".gemini/skills"),
        instructions_file: Some(".gemini/GEMINI.md"),
        ctx_flag: "gemini",
        upstream_indexer: None,
    },
];

pub fn lookup(id: &str) -> Option<&'static AgentDef>;
pub fn all() -> &'static [AgentDef];
pub fn ids() -> impl Iterator<Item = &'static str>;
```

Both `skills_subdir` and `instructions_file` are `Option` so future agents that have only one of the two surfaces (e.g., a hypothetical instructions-only agent) can be added without forcing them to invent a phantom directory.

### 2. `src/paths.rs` — single function

**Before** (lines 139–146):

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

**After:**

```rust
pub fn agent_skill_path(install_root: &Path, agent: &str, skill_name: &str) -> Result<PathBuf> {
    let def = crate::agents::lookup(agent)
        .ok_or_else(|| anyhow!("unknown agent `{}`", agent))?;
    let subdir = def.skills_subdir
        .ok_or_else(|| anyhow!("agent `{}` has no skills directory", agent))?;
    Ok(install_root.join(subdir).join(skill_name))
}
```

Behavior identical for `claude-code`/`codex`. The new `"agent has no skills directory"` error path is dead code today (all four agents have a skills dir) but exists as the contract for future instructions-only entries.

### 3. `src/instructions.rs` — `Tool` becomes a newtype

The current `Tool` enum has variants `Claude`/`Codex` and four methods (`id`, `name`, `output_path`, `from_id`). It becomes a thin newtype around `&'static AgentDef` so callers' type signatures stay intact while the underlying knowledge moves to the registry:

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Tool(pub &'static crate::agents::AgentDef);

impl Tool {
    pub fn id(&self) -> &'static str { self.0.id }
    pub fn name(&self) -> &'static str { self.0.display }
    pub fn ctx_flag(&self) -> &'static str { self.0.ctx_flag }
    pub fn output_path(&self, root: &Path) -> Option<PathBuf> {
        self.0.instructions_file.map(|f| root.join(f))
    }
    pub fn from_id(id: &str) -> Option<Self> {
        crate::agents::lookup(id).map(Tool)
    }
    pub fn all() -> impl Iterator<Item = Tool> {
        crate::agents::all().iter().map(Tool)
    }
}
```

The Handlebars context-flag insertion (`ctx.insert("claude".into(), Value::Bool(matches!(tool, Tool::Claude)))` and the equivalent for codex) becomes a loop over `Tool::all()` setting `ctx.insert(t.ctx_flag().into(), Value::Bool(t == active_tool))`. This automatically gives `{{#if opencode}}` and `{{#if gemini}}` once their rows exist.

The `allowed()` whitelist of context keys (`["claude", "codex", "hostname", ...]` at `instructions.rs:63`) becomes derived: `Tool::all().map(|t| t.ctx_flag()).chain(["hostname", ...])`.

### 4. `src/upstream.rs` — registry-driven indexer iteration

The existing `index_claude_marketplaces` function stays put but loses its hardcoded invocation. The top-level becomes:

```rust
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

Today only `claude-code` populates `upstream_indexer`; the loop has one iteration that does work and three that no-op. This is fine — the moment OpenCode or Gemini ships a marketplace-equivalent, they get a function pointer in their registry row.

### 5. `src/config.rs` — default-enabled list

`default_agents()` becomes:

```rust
fn default_agents() -> Vec<String> {
    crate::agents::all().iter().map(|a| a.id.into()).collect()
}
```

This is the explicit user decision: all four agents are in the default-enabled set. The behavioral consequences for existing users depend on whether they have an explicit `enabled_agents` line in `agents.toml` — see Migration notes.

### 6. `src/commands/import.rs` — bulk-scan paths

The hardcoded `~/.claude/skills`, `~/.codex/skills`, `~/.agents/skills` scan list becomes:

```rust
let mut scan_dirs: Vec<PathBuf> = crate::agents::all()
    .iter()
    .filter_map(|a| a.skills_subdir.map(|s| home.join(s)))
    .collect();
scan_dirs.push(home.join(".agents/skills")); // cross-tool interop alias
```

The `.agents/skills/` fallback is not bound to any one agent — it's the cross-tool standard alias honored by Gemini and OpenCode. Keeping it as a separate constant rather than a synthetic registry row matches its semantics.

The user-facing string at `import.rs:61` and `import.rs:102` (`"... in ~/.claude/skills/, ~/.codex/skills/, or ~/.agents/skills/"`) becomes derived from the `scan_dirs` list so the message stays accurate as agents are added.

## Phase 2: the new registry rows

Both rows verified live on 2026-05-06 against:

- OpenCode skills format: <https://opencode.ai/docs/skills/> ("Skills are directories containing a single `SKILL.md` file" — same `name`+`description` frontmatter, plus optional `license`/`compatibility`/`metadata` which agents ignores)
- OpenCode global config: <https://opencode.ai/docs/rules/> (`~/.config/opencode/AGENTS.md`)
- Gemini CLI skills format: <https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/skills.md> (same SKILL.md standard, cites [agentskills.io](https://agentskills.io))
- Gemini CLI GEMINI.md: <https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/gemini-md.md> (`~/.gemini/GEMINI.md` global, plain markdown)

No translation needed. agents's existing symlink-from-cache install logic (`src/install.rs`) works unchanged for both — it materializes a skill directory at the agent's `skills_subdir`/`<name>` path, which is exactly what OpenCode and Gemini scan for.

## Migration notes (v0.3 release)

Two user-visible changes:

1. **New files materialize on `agents apply`.** Existing users on v0.2.x have `enabled_agents = ["claude-code", "codex"]` in their `agents.toml` (either explicitly or via the default). When they upgrade to v0.3 and run `agents apply`, two new files appear:
   - `~/.gemini/GEMINI.md`
   - `~/.config/opencode/AGENTS.md`

   Each contains the same baseline instructions as their existing `~/.claude/CLAUDE.md` / `~/.codex/AGENTS.md`, with the relevant `{{#if gemini}}` / `{{#if opencode}}` Handlebars sections active.

   This is the explicit consequence of "all enabled by default". To opt out, the user edits `agents.toml` and removes the unwanted agents from `enabled_agents`.

2. **No automatic backfill of `enabled_agents`.** Users with an explicit `enabled_agents = ["claude-code", "codex"]` in their `agents.toml` keep that exact list — they do not get OpenCode and Gemini added behind their back. Only users relying on the default (no `enabled_agents` line, or running `agents init` fresh on v0.3) get the four-agent default.

   This means existing `agents.toml` files written by `agents init` on v0.2 have a hardcoded two-agent list and will NOT pick up the new agents. The release note should call out the one-line edit (`enabled_agents = ["claude-code", "codex", "opencode", "gemini"]`) for users who want the new default.

The release note (CHANGELOG entry) should state both points plainly. No silent behavior change for users with explicit config; new files appear for users who relied on the default.

## Testing

The Phase 1 refactor is held to a strict contract: **every existing test passes unchanged.** This is the no-behavior-change guarantee. Add:

- A registry round-trip test: for every `AgentDef` in `REGISTRY`, `lookup(def.id)` returns the same pointer.
- A test that `Tool::all().count()` equals `REGISTRY.len()`.
- A test that the Handlebars context inserts a flag for every agent (using a registry of mock entries to avoid coupling to the production count).
- A test that `agent_skill_path("claude-code", ...)` and `agent_skill_path("codex", ...)` produce byte-identical paths to the pre-refactor implementation. (Snapshot.)

Phase 2 additions get matching tests:

- `agent_skill_path("opencode", "foo")` → `<root>/.config/opencode/skills/foo`
- `agent_skill_path("gemini", "foo")` → `<root>/.gemini/skills/foo`
- Render the instructions template with `Tool::from_id("opencode")` and `Tool::from_id("gemini")` — assert the file goes to the correct path and `{{#if opencode}}` / `{{#if gemini}}` resolve correctly.

End-to-end (manual): on a fresh `agents init` with all four agents enabled, `agents apply` produces the four files at the expected paths and a fixture skill installs into all four `skills/` directories.

## Open questions for the implementation plan

These are deferred from the spec to the implementation plan — they are HOW questions, not WHAT questions:

1. ~~Does today's `agents.toml` editing flow already support enabling/disabling agents through a CLI command, or do users edit the file by hand?~~ **Resolved by addendum below — three new commands replace the manual edit.**
2. Whether to provide a one-shot `agents config migrate` command that adds OpenCode and Gemini to existing users' explicit `enabled_agents` lists with confirmation. Likely YAGNI for v0.3 — the manual edit is one line — but worth flagging.
3. Test fixture organization: should tests reference the production `REGISTRY` or use a separate `TEST_REGISTRY`? The latter avoids brittle test changes when we add agent #5.

---

## Addendum: `agents agents` subcommand

**Date:** 2026-05-06 (same day, follow-on)

The migration story above told users to edit `agents.toml` by hand to opt in/out of agents. That's friction. This addendum adds three subcommands so the TOML edit becomes the unusual path, not the default.

### Commands

```
agents agents list              # show all registry agents with [enabled]/[disabled] status
agents agents add <id>...       # enable one or more agents (variadic)
agents agents remove <id>...    # disable one or more agents (variadic)
```

### Behavior (mirrors `skills activate`/`deactivate`)

For `add` and `remove`:

1. `pre_pull` if auto-sync enabled (matches every other mutating command in agents)
2. Validate each id against the registry; reject unknown ids with `error: unknown agent 'foo'. valid: claude-code, codex, opencode, gemini`
3. Load `RepoConfig` (which materializes the four-agent default if `enabled_agents` is absent), mutate the list, write back to `agents.toml`
4. Auto-run `apply` so files materialize on `add` / disappear on `remove` — matches the skill activate/deactivate convention; without it `add gemini` is a half-action because the user expects `~/.gemini/GEMINI.md` to appear immediately
5. `commit_and_push` if auto-sync enabled

For `list`: read-only, prints a table.

### Decisions

- **Idempotent**: `add gemini` when already enabled prints `agents: gemini already enabled` and exits 0 (not an error). Same for `remove` of a not-enabled agent. Treats user intent as "make it so", not "perform exact diff".
- **Refuse to remove the last enabled agent**: empty `enabled_agents` would disable agents itself, almost certainly accidental. Error with hint: `cannot remove last enabled agent (would disable agents). use 'agents agents add <id>' first or remove the line manually.`
- **Variadic positionals**: `agents agents add gemini opencode` works in one call, matching the `bd close <id>...` pattern.
- **List output format**: shows ALL registry agents with `[enabled]/[disabled]` markers — more useful than only listing enabled, because it shows users what they could enable.

### `agents list` output

```
ID            STATUS    SKILLS DIR                       INSTRUCTIONS FILE
claude-code   enabled   ~/.claude/skills                 ~/.claude/CLAUDE.md
codex         enabled   ~/.codex/skills                  ~/.codex/AGENTS.md
opencode      disabled  ~/.config/opencode/skills        ~/.config/opencode/AGENTS.md
gemini        enabled   ~/.gemini/skills                 ~/.gemini/GEMINI.md
```

### Files

- New: `src/commands/agents.rs` (~120 lines including tests)
- Modify: `src/cli.rs` — add `Agents(AgentsCommand)` variant + `AgentsCommand` enum (`List`, `Add { ids: Vec<String> }`, `Remove { ids: Vec<String> }`)
- Modify: `src/commands/mod.rs` — register module
- Modify: `src/main.rs` — dispatch arm
- Modify: `src/git_sync.rs` — `msg_agents_add(&[String])` and `msg_agents_remove(&[String])` helpers matching existing `msg_activate`
- Modify: `docs/concepts/agents.md` — replace "edit `agents.toml`" guidance with `agents agents add/remove`

### Tests

- `agents list` against the production registry returns expected status (when no `agents.toml`, all four `enabled`; when `enabled_agents = ["claude-code"]` only that one shows enabled)
- `agents add gemini` to a config with explicit `["claude-code", "codex"]` produces `["claude-code", "codex", "gemini"]`
- `agents add gemini` to a config that already has gemini reports already-enabled and leaves the file untouched
- `agents add no-such-agent` fails with the "valid agents" message and writes nothing
- `agents remove claude-code` from a config with `["claude-code"]` errors with the last-agent hint
- `agents remove gemini` from a config without an explicit `enabled_agents` line first materializes the four-agent default, then removes gemini, ending at `["claude-code", "codex", "opencode"]`
