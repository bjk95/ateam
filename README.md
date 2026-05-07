# Agents

Sync skills, instructions, and subagents across AI coding harnesses (Claude
Code, Codex, OpenCode, Gemini CLI) and across machines.

## The problem

Each harness reads from its own config directory and uses its own file
format:

- Claude Code → `~/.claude/skills/`, `~/.claude/agents/`, `~/.claude/CLAUDE.md`
- Codex → `~/.codex/skills/`, `~/.codex/agents/`, `~/.codex/AGENTS.md`
- OpenCode → `~/.config/opencode/skills/`, `~/.config/opencode/agent/`, `~/.config/opencode/AGENTS.md`
- Gemini CLI → `~/.gemini/skills/`, `~/.gemini/agents/`, `~/.gemini/GEMINI.md`

Subagent frontmatter is YAML for Claude, OpenCode, and Gemini; TOML for
Codex.

If you use more than one harness, you install each skill multiple times,
maintain `CLAUDE.md` and `AGENTS.md` in parallel, and keep two flavors of
subagent frontmatter in sync. If you use more than one machine, you redo
all of that on every machine.

## What agents does

You keep one canonical copy of each artifact in a git repo. `agents`
renders it to each harness's native format on disk and keeps every machine
in sync through the git remote.

- **Skills.** `agents skills add <name>` installs once; the skill lands in
  every enabled harness. Drop-in for `npx skills add`; sources from
  skills.sh, GitHub, or local paths.
- **Instructions.** One Handlebars template at
  `instructions/instructions.md.hbs` renders to `~/.claude/CLAUDE.md`,
  `~/.codex/AGENTS.md`, `~/.config/opencode/AGENTS.md`, and
  `~/.gemini/GEMINI.md`. Profile-gated fragments let work and home machines
  read different rules from the same template. Adopt your existing globals
  with `agents import --instructions`.
- **Subagents.** One Markdown file at `<repo>/agents/<name>.md` with
  multi-harness frontmatter renders to YAML+Markdown for Claude, OpenCode,
  and Gemini, and to TOML for Codex.
- **Cross-machine sync.** Wire a git remote; every `add` / `update` /
  `remove` / `instructions edit` pulls before the change and pushes after,
  in the background. On a new machine, `agents apply` reproduces the full
  setup.

## Other features

- **Profiles.** Tag skills and subagents `work` or `personal` so each
  machine gets the right subset.
- **Project scope.** Install a skill into one repo's `.claude/skills`
  without touching globals. The same project lives at different paths on
  different machines — agents handles the alias.
- **Soft-disable.** `agents skills deactivate` unlinks from harnesses but
  keeps the lockfile entry, so re-enabling is one command.
- **Harness toggling.** `agents harness add|remove <name>` turns a harness
  on or off and re-applies.
- **Offline-tolerant.** Git operations soft-fail offline so local work is
  never blocked.

> **Status:** v1, stable on macOS + Linux. Tested end-to-end against the
> live [skills.sh](https://skills.sh) registry.

## Install

```bash
curl -fsSL https://github.com/bjk95/agents/releases/latest/download/agents-installer.sh | sh
```

Single static binary at `~/.local/bin/agents`. macOS (Apple Silicon or
Intel) and Linux (x86_64 or aarch64, musl-static — works on glibc and musl
distros alike). After install, `agents` self-updates: every command checks
GitHub Releases at most once every 24 hours and replaces the binary in
place when a newer version ships. Trigger explicitly with `agents upgrade`.

To build from source, see [docs/install.md](./docs/install.md).

## Quickstart

```bash
# 1. Bootstrap a config repo at ~/.config/agents/
agents init --scaffold

# 2. Install a skill — it appears in every enabled harness
agents skills add vercel-labs/agent-skills --skill deploy-to-vercel -y

# 3. Drop in a subagent — same thing, every harness
agents subagents add vercel-labs/agent-skills --subagent code-reviewer -y

# 4. Adopt your existing CLAUDE.md / AGENTS.md as the instructions template
agents import --instructions

# 5. Wire a remote so other machines can sync
agents remote add git@github.com:you/agents-config.git
```

On a second machine:

```bash
agents init git@github.com:you/agents-config.git
agents apply
# every skill, subagent, and instructions file from machine A now lives on machine B
```

From here, every `agents skills add`, `subagents add`, or
`instructions edit` pushes to the remote and reaches every other machine
the next time it runs an `agents` command.

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
