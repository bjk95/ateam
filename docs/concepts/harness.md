---
title: Supported harnesses
description: Which AI coding harnesses agents syncs to, and what each one gets.
---

agents syncs skills and instructions across these **harnesses** — the term we use for an AI coding tool that hosts skills (Claude Code, Codex, OpenCode, Gemini CLI). We avoid "agent" because it collides with each tool's own internal `agents/` subagent concept (e.g. Claude's `~/.claude/agents/`).

Each harness has a stable id used in `agents.toml`'s `enabled_harnesses` list and in lockfile entries' `harnesses` field.

| id | tool | skills directory | global instructions file |
|---|---|---|---|
| `claude-code` | [Claude Code](https://claude.com/claude-code) | `~/.claude/skills/<name>/SKILL.md` | `~/.claude/CLAUDE.md` |
| `codex` | [OpenAI Codex CLI](https://github.com/openai/codex) | `~/.codex/skills/<name>/SKILL.md` | `~/.codex/AGENTS.md` |
| `opencode` | [OpenCode](https://opencode.ai) | `~/.config/opencode/skills/<name>/SKILL.md` | `~/.config/opencode/AGENTS.md` |
| `gemini` | [Gemini CLI](https://github.com/google-gemini/gemini-cli) | `~/.gemini/skills/<name>/SKILL.md` | `~/.gemini/GEMINI.md` |

All four use the same `SKILL.md` format ([agentskills.io](https://agentskills.io) open standard): a directory containing a `SKILL.md` file with YAML frontmatter (`name`, `description`) and optional bundled `scripts/`, `references/`, `assets/`. agents's symlink-from-cache install logic works identically for every harness — only the destination path differs.

## Default-enabled set

By default, all four harnesses are enabled. agents's `apply` will render instruction files into the agents repo and install harness symlinks for each one — even if the harness itself isn't installed on this machine.

## Managing the enabled set

Use the `agents harness` subcommand to toggle harnesses on and off. The commands edit `enabled_harnesses` in `agents.toml` for you, then re-render instructions and reconcile harness symlinks so the filesystem matches the new state immediately.

```bash
agents harness list                  # show every harness + enabled/disabled status
agents harness add gemini opencode   # enable one or more
agents harness remove gemini         # disable one or more
```

Both `add` and `remove` are idempotent — adding an already-enabled harness or removing one that isn't enabled is a no-op with an informational message, not an error. agents refuses to remove the last enabled harness (that would disable agents itself); if you really want an empty list, edit `agents.toml` by hand.

## Per-skill harness gating

The lockfile's `harnesses` field on each skill entry restricts which harnesses that skill installs to. Use `["*"]` for all enabled harnesses, or list specific ids:

```toml
[[skill]]
name = "my-skill"
harnesses = ["claude-code", "codex"]   # skip opencode + gemini for this skill
```

## Adding support for another harness

Adding a harness is a one-row change in `src/harness.rs`. File an issue or open a PR with the harness's stable id, display name, skills subdir (relative to install root), and instructions file path.

Cursor and Copilot are not currently supported because neither has a globally-syncable file surface — Cursor User Rules live in app settings and Copilot personal instructions live in VS Code settings, not files. Project-level rules for both live in the project repo, outside agents's per-machine sync scope.
