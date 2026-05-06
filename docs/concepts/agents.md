---
title: Supported agents
description: Which AI coding agents ateam syncs to, and what each one gets.
---

ateam syncs skills and instructions across these agents. Each agent has a stable id used in `ateam.toml`'s `enabled_agents` list and in lockfile entries' `agents` field.

| id | tool | skills directory | global instructions file |
|---|---|---|---|
| `claude-code` | [Claude Code](https://claude.com/claude-code) | `~/.claude/skills/<name>/SKILL.md` | `~/.claude/CLAUDE.md` |
| `codex` | [OpenAI Codex CLI](https://github.com/openai/codex) | `~/.codex/skills/<name>/SKILL.md` | `~/.codex/AGENTS.md` |
| `opencode` | [OpenCode](https://opencode.ai) | `~/.config/opencode/skills/<name>/SKILL.md` | `~/.config/opencode/AGENTS.md` |
| `gemini` | [Gemini CLI](https://github.com/google-gemini/gemini-cli) | `~/.gemini/skills/<name>/SKILL.md` | `~/.gemini/GEMINI.md` |

All four use the same `SKILL.md` format ([agentskills.io](https://agentskills.io) open standard): a directory containing a `SKILL.md` file with YAML frontmatter (`name`, `description`) and optional bundled `scripts/`, `references/`, `assets/`. ateam's symlink-from-cache install logic works identically for every agent — only the destination path differs.

## Default-enabled set

By default, all four agents are enabled. ateam's `apply` will write instructions files and install skill symlinks for each one — even if the agent itself isn't installed on this machine. To opt out, edit `ateam.toml`:

```toml
enabled_agents = ["claude-code", "codex"]  # only these two
```

Existing users upgrading from v0.2.x with an explicit `enabled_agents` line keep their list unchanged. Only users on the default (no line, or fresh `ateam init`) get the four-agent default.

## Per-skill agent gating

The lockfile's `agents` field on each skill entry restricts which agents that skill installs to. Use `["*"]` for all enabled agents, or list specific ids:

```toml
[[skill]]
name = "my-skill"
agents = ["claude-code", "codex"]   # skip opencode + gemini for this skill
```

## Adding support for another agent

Adding an agent is a one-row change in `src/agents.rs`. File an issue or open a PR with the agent's stable id, display name, skills subdir (relative to install root), and instructions file path.

Cursor and Copilot are not currently supported because neither has a globally-syncable file surface — Cursor User Rules live in app settings and Copilot personal instructions live in VS Code settings, not files. Project-level rules for both live in the project repo, outside ateam's per-machine sync scope.
