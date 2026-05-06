---
title: CLI
description: Every ateam command and flag.
---

## `ateam init`

Bootstrap a fresh ateam-config repo or clone an existing one.

```bash
ateam init                         # interactive: clone or scaffold?
ateam init <git-url>               # clone into ~/.config/ateam/
ateam init --scaffold              # fresh empty repo at ~/.config/ateam/
ateam init --repo <path>           # use a non-default location
ateam init ... --profiles a,b      # set this machine's profile list
```

## `ateam apply`

Materialize the lockfile (active entries only).

```bash
ateam apply [--dry-run] [-a <agent>...] [--project <alias>] [--force]
```

`--force` moves any existing real directory at a target path aside to
`<name>.bak.<unix-ts>` instead of refusing.

## `ateam status`

```bash
ateam status                       # repo path, profiles, manifest health
```

## `ateam skills add` (Vercel-compatible)

Drop-in replacement for `npx skills add` — same flags, swap `npx` for `ateam`.

| Flag | Behavior |
|---|---|
| `<repo>` (positional) | `owner/repo` shorthand, full git URL, or local path |
| `--list` | Print discovered skills, don't install |
| `--skill <name>` | Specific skill names, repeatable; `*` = all |
| `--all` | Equivalent to `--skill '*'` |
| `-a` / `--agent <name>` | Target agents, repeatable; `*` = all enabled |
| `-y` / `--yes` | Non-interactive; in an unregistered git repo, auto-registers + project-scopes |
| `-g` / `--global` | Force global scope (overrides cwd auto-detect and the auto-register prompt) |
| `--profile <name>` | Tag entry with profile gates (repeatable) |
| `--project <alias>` | Install into a registered project |
| `--ref <ref>` | Pin to a specific git ref/tag/commit |
| `--no-sync` | Skip auto pull/commit/push for this run |

## `ateam skills update`

Check GitHub tree SHAs and refetch any drifted skills. Skips deactivated entries.

```bash
ateam skills update                # all active entries
ateam skills update <name>...      # specific entries
```

## `ateam skills remove`

Delete a skill from the lockfile and uninstall its symlinks.

```bash
ateam skills remove <name>
```

Local-source directories under `<repo>/skills/` are never deleted by ateam — you
remove them yourself if you want them gone.

## `ateam skills deactivate` / `ateam skills activate`

Soft-disable a skill without losing its lockfile entry. Deactivating immediately
unlinks the skill from `~/.claude/skills/` and `~/.codex/skills/`; activating
re-materializes it.

```bash
ateam skills deactivate <name>
ateam skills activate <name>
```

The `active` flag rides with the skill in the lockfile, so deactivating on one
machine deactivates everywhere after the next sync. `ateam skills list` marks
deactivated entries with `[off]`.

## `ateam skills list`

```bash
ateam skills list                  # all locked skills (active + [off])
ateam skills list --project canva  # only entries scoped to one project
```

## `ateam skills import`

Adopt an installed-locally skill into the synced lockfile.

```bash
ateam skills import <name>                            # snapshot into <repo>/skills/
ateam skills import <name> --upstream github:foo/bar  # track upstream instead
ateam skills import <name> --project canva            # tag with project alias
```

## `ateam upgrade`

Self-update: download the latest `ateam` release and replace this binary.
Bypasses the 24h TTL check that runs implicitly before every other command.

```bash
ateam upgrade
```

Prints `ateam: updated X → Y` on success or `ateam: already at latest (X)`
when no upgrade was needed. Exits non-zero on failure.

The implicit check runs at most once every 24 hours, soft-fails on any
network/filesystem error, and never blocks the command you actually ran.
There is no env-var opt-out; if you don't want updates, ignore the
occasional `ateam: updated …` line — the cache is at `~/.cache/ateam/`.

## `ateam project`

Manage this machine's alias→path map.

```bash
ateam project add <alias> <path>     # register
ateam project list                   # show
ateam project remove <alias>         # forget
```

`add` accepts `register` as a hidden alias for muscle memory.
