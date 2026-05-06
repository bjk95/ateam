# ateam wishlist

Post-v1 roadmap. v1 (multi-machine skills sync with auto-git) is being implemented in `/Users/brad/.claude/plans/https-developers-openai-com-codex-skills-virtual-dove.md`.

This file captures what comes next, what's parked, and what's explicitly out.

## Asset coverage (the next big chunk)

- **Instructions file sync** — `CLAUDE.md` / `AGENTS.md` from a single canonical source with profile-gated fragments and Handlebars templating. Replaces today's hand-mirroring.
- **Subagents** — Claude `~/.claude/agents/*.md` ↔ Codex `~/.codex/agents/*.toml` with format translation.
- **Claude plugin marketplace** as a source type — `ateam skills add marketplace:<url>` so marketplace-installed skills can be tracked in the lockfile.
- **`settings.json` sync** — profile-gated. Work and personal need different settings; this collapses two hand-edited files into one source.
- **Hooks sync** — `~/.claude/hooks/`.
- **MCP server config sync** — profile-gated (e.g. work has Canva-internal MCPs, personal has different ones).

> **Cross-tool requirement:** settings, hooks, and MCP configs must all sync to **both Claude and Codex**, not just one. Mirror the per-tool format where they diverge (the same translation pattern as subagents).

## TUI

- Interactive skill browser
- Install / remove flows
- Per-machine status dashboard (what's installed, what's drifted, what's pending)
- (Conflict resolver — parked, decide later)

## Discovery

- `ateam search <query>` — search installed + skills.sh registry in one place
- Skill preview before install — show frontmatter, file tree, line count, description

## Sync mechanism

- `ateam status --fetch` — show remote drift without pulling
- `ateam diff <commit>` — show what would change between lockfile versions
- (Conflict resolver in TUI — parked, decide later)

## Skill authoring

- Local skill publishing helper — push a `local:` skill to a git remote and (eventually) register it on skills.sh
- Version pinning — `ateam skills add foo/bar --version ^1.2` with semver-style updates

## Migration

- Importer from `npx skills` lockfile format — for users already using the Vercel CLI

## Multi-tool reach (later, do not pre-build for it)

Cursor, Windsurf, Cline, etc. are downstream of this. The skills.sh per-agent path conventions are already documented for ~55 agents, so adding a tool is mostly a path-mapping table entry. **Do not build pluggable-tool infrastructure now** — add tools when the need arises.

## Explicit non-goals

These were considered and rejected. Don't quietly slip them back in.

- Skills ordering (tools handle this themselves)
- Skill linter / skill testing harness
- Watch mode (`ateam watch`)
- Selective sync — per-machine exclusion outside the profile system
- `ateam doctor` health check
- Audit log (`git log` is sufficient)
- Skill usage tracking / effectiveness metrics
- Disk usage report
- Profile composition / tool-specific profiles / org-team profiles / time-based gating
- Starter-kit templates (`ateam init --template ...`)
- chezmoi migration helper (not enough users)
- Security: skill signing, sandboxing, encrypted secrets
- Power-user features: `ateam batch`, library/API mode, plugin hooks
- Windows support
