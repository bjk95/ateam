# A-Team

Multi-machine AI skills sync. A Rust CLI that's a drop-in for `npx skills add`,
backed by a single git repo at `~/.config/ateam/`, with `git pull` / `commit` /
`push` happening invisibly so you never type `git` directly.

- **Vercel-compatible** — every flag from `npx skills add` works as `ateam skills add`.
- **One lockfile, one repo, zero project pollution.** Skills declared in
  `~/.config/ateam/ateam.lock.toml`; project repos gain nothing ateam-specific.
- **Project scope by alias.** Same project lives at different paths on
  different machines — register once per machine, sync everywhere.
- **Soft-disable for lean libraries.** `ateam skills deactivate <name>` unlinks
  a skill from your agents but keeps the lockfile entry — quarantine-then-delete
  for skills you suspect you don't use, no usage tracking required.
- **Auto-sync.** Every mutating command pulls before mutating, commits the
  result, and pushes. Soft-fails offline, never blocks your local change.

> **Status:** v1, macOS + Linux. Tested end-to-end against the live
> [skills.sh](https://skills.sh) registry. See [WISHLIST.md](./WISHLIST.md) for
> what's coming next (CLAUDE.md/AGENTS.md sync, subagents, settings, hooks).

## Install

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/bjk95/ateam/releases/latest/download/ateam-installer.sh | sh
```

Drops a single static binary at `~/.local/bin/ateam` on macOS (Apple Silicon
or Intel) and Linux (x86_64 or aarch64, musl-static — works on glibc and musl
distros alike). To build from source instead, see [docs/install.md](./docs/install.md).

After install, `ateam` keeps itself up to date: every command checks GitHub
Releases at most once every 24 hours and replaces the binary in place when a
newer version is found. To trigger explicitly, run `ateam upgrade`.

## Quickstart

```bash
# Bootstrap a fresh ateam-config repo at ~/.config/ateam/
ateam init --scaffold --profiles personal

# Install a skill from skills.sh
ateam skills add vercel-labs/agent-skills --skill deploy-to-vercel -y

# Verify both Claude Code and Codex see it
ls ~/.claude/skills/ ~/.codex/skills/
```

To sync to a second machine, point ateam at a git remote first:

```bash
# On machine A
git -C ~/.config/ateam remote add origin git@github.com:you/ateam-config.git
git -C ~/.config/ateam push -u origin main
ateam skills add vercel-labs/agent-skills --skill web-design-guidelines -y
# auto-pushes the lockfile change

# On machine B
ateam init git@github.com:you/ateam-config.git --profiles work
ateam apply
```

That's it. From now on every `ateam skills add` / `update` / `remove` syncs invisibly.

## Docs

Full docs at <https://bjk95.github.io/ateam/>, or browse the markdown directly:

- [Installation](./docs/install.md)
- [Quickstart](./docs/quickstart.md)
- Concepts: [Auto-sync](./docs/concepts/auto-sync.md) · [Profiles](./docs/concepts/profiles.md) · [Project scope](./docs/concepts/project-scope.md)
- Reference: [CLI](./docs/reference/cli.md) · [Lockfile format](./docs/reference/lockfile.md)
- [Troubleshooting](./docs/operate/troubleshooting.md)

The docs site is built with Astro + Starlight from this same `docs/` directory
via a symlink, so the markdown is the single source of truth — GitHub renders
it natively when you click `docs/` above, and the published site stays in sync
on every push.

## Repo layout

```
ateam/
├── src/                       Rust CLI source
│   ├── commands/              clap subcommand handlers
│   ├── source/                github / git / local source fetchers
│   ├── lockfile.rs            ateam.lock.toml read/write
│   ├── install.rs             symlink + atomic cache materialization
│   ├── manifest.rs            per-machine install tracking
│   ├── git_sync.rs            invisible auto pull/commit/push
│   └── …
├── docs/                      Markdown source for the docs site
├── site/                      Astro + Starlight build (symlinks docs/ in)
└── .github/workflows/         GitHub Pages deploy
```

## Contributing

PRs welcome on anything in [WISHLIST.md](./WISHLIST.md), or anything that
makes the v1 surface tighter. Run `cargo test` before submitting.

## License

MIT.
