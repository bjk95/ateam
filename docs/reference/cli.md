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

## `ateam add` (Vercel-compatible)

Drop-in replacement for `npx skills add`.

| Flag | Behavior |
|---|---|
| `<repo>` (positional) | `owner/repo` shorthand, full git URL, or local path |
| `--list` | Print discovered skills, don't install |
| `--skill <name>` | Specific skill names, repeatable; `*` = all |
| `--all` | Equivalent to `--skill '*'` |
| `-a` / `--agent <name>` | Target agents, repeatable; `*` = all enabled |
| `-y` / `--yes` | Non-interactive |
| `-g` / `--global` | Force global scope (overrides cwd auto-detect) |
| `--profile <name>` | Tag entry with profile gates (repeatable) |
| `--project <alias>` | Install into a registered project |
| `--ref <ref>` | Pin to a specific git ref/tag/commit |
| `--no-sync` | Skip auto pull/commit/push for this run |

## `ateam apply`

Materialize the lockfile.

```bash
ateam apply [--dry-run] [-a <agent>...] [--project <alias>] [--force]
```

`--force` moves any existing real directory at a target path aside to
`<name>.bak.<unix-ts>` instead of refusing.

## `ateam update`

Check GitHub tree SHAs and refetch any drifted skills.

```bash
ateam update                       # all entries
ateam update <name>...             # specific entries
```

## `ateam remove`

Delete a skill from the lockfile and uninstall its symlinks.

```bash
ateam remove <name>
```

Local-source directories under `<repo>/skills/` are never deleted by ateam — you
remove them yourself if you want them gone.

## `ateam list` / `ateam status`

```bash
ateam list                         # all locked skills
ateam list --project canva         # only entries scoped to one project
ateam status                       # repo path, profiles, manifest health
```

## `ateam import`

Adopt an installed-locally skill into the synced lockfile.

```bash
ateam import <name>                            # snapshot into <repo>/skills/
ateam import <name> --upstream github:foo/bar  # track upstream instead
ateam import <name> --project canva            # tag with project alias
```

## `ateam project`

Manage this machine's alias→path map.

```bash
ateam project add <alias> <path>     # register
ateam project list                   # show
ateam project remove <alias>         # forget
```

`add` accepts `register` as a hidden alias for muscle memory.
