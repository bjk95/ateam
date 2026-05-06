# A-Team

**Install an AI skill once. Run it on every agent, on every machine.**

Add a skill — or edit your `CLAUDE.md` — on your laptop and it appears on
your work box. Claude Code sees it. Codex sees it. No re-installing, no
copy-pasting between `~/.claude/skills`, no maintaining `CLAUDE.md` and
`AGENTS.md` side by side, no `git push` — ateam syncs everything invisibly.

## What ateam gives you

- **One install, every tool.** `ateam skills add deploy-to-vercel` lands in
  Claude Code *and* Codex from a single command. No more installing the same
  skill twice.
- **One source for `CLAUDE.md` and `AGENTS.md`.** A single Handlebars template
  at `instructions/instructions.md.hbs` renders to both files, with
  profile-gated fragments so your work laptop and home machine read different
  instructions from the same source. Adopt your existing globals with
  `ateam skills import --instructions`.
- **One install, every machine.** Wire a git remote and every `add` /
  `update` / `remove` propagates. Open a fresh laptop, run `ateam apply`, and
  your full skill library is there in seconds.
- **Invisible git.** Pull, commit, and push happen in the background on every
  command. Soft-fails offline so you're never blocked from working locally.
- **Profiles.** Tag skills `work` or `personal`. Work laptop gets the work
  skills; home machine doesn't. One source of truth, profile-gated outputs.
- **Project scope.** Drop a skill into one repo's `.claude/skills` without
  polluting your globals. The same project lives at different paths on
  different machines — ateam handles the alias.
- **Soft-disable.** `ateam skills deactivate` unlinks a skill from your
  agents but keeps the lockfile entry. No usage tracking; reversible cleanup
  for skills you suspect you don't need.
- **Drop-in for `npx skills`.** Every flag from Vercel's CLI works as
  `ateam skills add`. Already using the Vercel one? Switch in a minute.

> **Status:** v1, stable on macOS + Linux. Tested end-to-end against the live
> [skills.sh](https://skills.sh) registry.

## Install

```bash
curl -fsSL https://github.com/bjk95/ateam/releases/latest/download/ateam-installer.sh | sh
```

Single static binary at `~/.local/bin/ateam`. macOS (Apple Silicon or Intel)
and Linux (x86_64 or aarch64, musl-static — works on glibc and musl distros
alike). After install, `ateam` self-updates: every command checks GitHub
Releases at most once every 24 hours and replaces the binary in place when a
newer version ships. To trigger explicitly: `ateam upgrade`.

To build from source, see [docs/install.md](./docs/install.md).

## 5-minute quickstart

```bash
# 1. Bootstrap a config repo at ~/.config/ateam/
ateam init --scaffold

# 2. Install a skill — it appears in Claude Code AND Codex
ateam skills add vercel-labs/agent-skills --skill deploy-to-vercel -y
ls ~/.claude/skills/ ~/.codex/skills/   # both now have it

# 3. Wire a remote so other machines can sync
ateam remote add git@github.com:you/ateam-config.git
```

On a second machine:

```bash
ateam init git@github.com:you/ateam-config.git
ateam apply
# every skill from machine A now lives on machine B
```

That's it. From now on every `ateam skills add` / `update` / `remove` syncs
invisibly.

## Docs

Full docs at <https://bjk95.github.io/ateam/>, or browse the markdown
directly:

- [Installation](./docs/install.md)
- [Quickstart](./docs/quickstart.md)
- Concepts: [Auto-sync](./docs/concepts/auto-sync.md) · [Profiles](./docs/concepts/profiles.md) · [Project scope](./docs/concepts/project-scope.md)
- Reference: [CLI](./docs/reference/cli.md) · [Lockfile format](./docs/reference/lockfile.md)
- [Troubleshooting](./docs/operate/troubleshooting.md)

## License

MIT.
