# Agents

**One source. Every harness. Every machine.**

Add a skill, edit your `CLAUDE.md`, or drop in a code-reviewer subagent on
your laptop. Claude Code, Codex, OpenCode, and Gemini CLI all see it. Your
work box sees it tomorrow. No re-installing, no copy-pasting between
`~/.claude/skills` and `~/.codex/skills`, no maintaining `CLAUDE.md` and
`AGENTS.md` side by side, no juggling four flavors of subagent frontmatter,
no `git push` — agents syncs everything invisibly.

## Three artifacts, all first-class

agents tracks the three filesystem-rooted things every AI coding harness
cares about, and re-shapes each one for whichever tools you use:

- **Skills.** SKILL.md packages from skills.sh, GitHub, or local paths.
  `agents skills add deploy-to-vercel` lands in Claude Code, Codex, OpenCode,
  *and* Gemini CLI from a single command. Drop-in for `npx skills add` —
  every flag from Vercel's CLI works the same way.
- **Instructions.** A single Handlebars template at
  `instructions/instructions.md.hbs` renders to `~/.claude/CLAUDE.md`,
  `~/.codex/AGENTS.md`, `~/.config/opencode/AGENTS.md`, and
  `~/.gemini/GEMINI.md`. Profile-gated fragments let your work laptop and
  home machine read different rules from the same source. Adopt your
  existing globals with `agents skills import --instructions`.
- **Subagents.** A single canonical Markdown file at `<repo>/agents/<name>.md`
  with multi-harness frontmatter renders to each harness's native format —
  YAML+Markdown for Claude/OpenCode/Gemini, TOML for Codex. Set the right
  model id per harness in one file; never maintain four flavors by hand.

## What ties them together

- **One install, every machine.** Wire a git remote and every `add` /
  `update` / `remove` / `instructions edit` propagates. Open a fresh laptop,
  run `agents apply`, and your full setup is there in seconds.
- **Invisible git.** Pull, commit, and push happen in the background on
  every command. Soft-fails offline so you're never blocked from working
  locally.
- **Profiles.** Tag skills and subagents `work` or `personal`. Work laptop
  gets the work ones; home machine doesn't. One source of truth,
  profile-gated outputs.
- **Project scope.** Drop a skill into one repo's `.claude/skills` without
  polluting your globals. The same project lives at different paths on
  different machines — agents handles the alias.
- **Soft-disable.** `agents skills deactivate` unlinks a skill from your
  harnesses but keeps the lockfile entry. No usage tracking; reversible
  cleanup for skills you suspect you don't need.
- **Toggleable harnesses.** `agents harness add gemini` /
  `agents harness remove gemini` flips a harness on or off and re-applies
  in one step.

> **Status:** v1, stable on macOS + Linux. Tested end-to-end against the live
> [skills.sh](https://skills.sh) registry.

## Install

```bash
curl -fsSL https://github.com/bjk95/agents/releases/latest/download/agents-installer.sh | sh
```

Single static binary at `~/.local/bin/agents`. macOS (Apple Silicon or Intel)
and Linux (x86_64 or aarch64, musl-static — works on glibc and musl distros
alike). After install, `agents` self-updates: every command checks GitHub
Releases at most once every 24 hours and replaces the binary in place when a
newer version ships. To trigger explicitly: `agents upgrade`.

To build from source, see [docs/install.md](./docs/install.md).

## 5-minute quickstart

```bash
# 1. Bootstrap a config repo at ~/.config/agents/
agents init --scaffold

# 2. Install a skill — it appears in every enabled harness
agents skills add vercel-labs/agent-skills --skill deploy-to-vercel -y

# 3. Drop in a subagent — same thing, every harness
agents subagents add vercel-labs/agent-skills --subagent code-reviewer -y

# 4. Adopt your existing CLAUDE.md / AGENTS.md as the instructions template
agents skills import --instructions

# 5. Wire a remote so other machines can sync
agents remote add git@github.com:you/agents-config.git
```

On a second machine:

```bash
agents init git@github.com:you/agents-config.git
agents apply
# every skill, subagent, and instructions file from machine A now lives on machine B
```

That's it. From now on every `agents skills add` / `subagents add` /
`instructions edit` syncs invisibly.

## Docs

Full docs at <https://bjk95.github.io/agents/>, or browse the markdown
directly:

- [Installation](./docs/install.md)
- [Quickstart](./docs/quickstart.md)
- Concepts: [Harnesses](./docs/concepts/harness.md) · [Auto-sync](./docs/concepts/auto-sync.md) · [Profiles](./docs/concepts/profiles.md) · [Project scope](./docs/concepts/project-scope.md)
- Reference: [CLI](./docs/reference/cli.md) · [Lockfile format](./docs/reference/lockfile.md)
- [Troubleshooting](./docs/operate/troubleshooting.md)

## License

MIT.
