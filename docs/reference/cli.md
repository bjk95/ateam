---
title: CLI
description: Every ateam command and flag.
---

## Global flags

These work on every subcommand.

| Flag | Behavior |
|---|---|
| `--no-sync` | Skip auto pull/commit/push for this invocation. Equivalent: `ATEAM_NO_SYNC=1`. |
| `-v` / `--verbose` | Show extra detail (paths, SHAs, per-agent links). |
| `-q` / `--quiet` | Suppress non-error output: banner, success lines, progress spinners, plain text. Errors and warnings still print to stderr. |

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

If a real directory already sits at a target path (e.g. a skill installed by
hand or by `npx skills`), apply auto-heals it: if its contents match the
snapshot at `<repo>/skills/<name>/` byte-for-byte, ateam removes the dir and
replaces it with a symlink. No `--force` needed and no data loss is possible
because the snapshot already has the same bytes. If the contents don't match,
apply refuses — `--force` is the escape hatch and moves the conflicting
directory aside to `<name>.bak.<unix-ts>` rather than deleting it.

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
| `--all` | Implies `--skill '*'`, `--agent '*'`, and `-y` (Vercel-compat triple-flag override) |
| `-a` / `--agent <name>` | Target agents, repeatable; `*` = all enabled |
| `-y` / `--yes` | Non-interactive; in an unregistered git repo, auto-registers + project-scopes |
| `-g` / `--global` | Force global scope (overrides cwd auto-detect and the auto-register prompt) |
| `--profile <name>` | Tag entry with profile gates (repeatable) |
| `--project <alias>` | Install into a registered project |
| `--ref <ref>` | Pin to a specific git ref/tag/commit |
| `--no-sync` | Skip auto pull/commit/push for this run |

### skills.sh registry fallback

When `--skill <name>` doesn't match anything in the cloned GitHub repo's tree,
ateam falls back to the [skills.sh](https://skills.sh) blob endpoint and
installs from the registry's snapshot. Mirrors `npx skills add` — covers
skills that have been renamed, moved, or removed upstream but are still
served from the registry's cache. Only fires for github sources with an
explicit `--skill <name>` (not `--all` / `*`).

## `ateam skills update`

Check GitHub tree SHAs and refetch any drifted skills. Skips deactivated entries.

```bash
ateam skills update                # all active entries
ateam skills update <name>...      # specific entries
ateam skills update --global       # only global-scoped entries
ateam skills update --project foo  # only entries tagged with project alias `foo`
```

| Flag | Behavior |
|---|---|
| `-y` / `--yes` | Non-interactive (skip confirmation prompts) |
| `-g` / `--global` | Only update entries without a project scope |
| `--project <alias>` | Only update entries tagged with this project alias |
| `--no-sync` | Skip auto pull/commit/push for this run |

`--global` and `--project` are mutually exclusive.

## `ateam skills remove`

Delete one or more skills from the lockfile and uninstall their symlinks. If
any name isn't in the lockfile, nothing is removed and the command errors.

```bash
ateam skills remove <name>...
```

```bash
ateam skills remove gstack-retro gstack-plan-design-review
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
ateam skills list --json           # versioned JSON for editor integrations
```

### `--json` schema

`--json` emits a single JSON document on stdout. Banner and other UI output are
suppressed automatically so the document is the only thing on stdout.

```json
{
  "version": 1,
  "skills": [
    {
      "name": "deploy-to-vercel",
      "source": "github:vercel-labs/agent-skills",
      "ref": null,
      "tree_sha": null,
      "path": null,
      "agents": ["*"],
      "profiles": [],
      "project": null,
      "active": true,
      "upstream": null
    }
  ]
}
```

| Field | Type | Notes |
|---|---|---|
| `version` | integer | Schema version. Currently `1`. Bumped on incompatible changes. |
| `skills[]` | array | One entry per locked skill. Empty array if none. |
| `skills[].name` | string | Skill name (matches lockfile `name`). |
| `skills[].source` | string | `github:owner/repo`, `git:<url>`, or `local:<path>`. |
| `skills[].ref` | string \| null | Git ref/tag/commit pin. |
| `skills[].tree_sha` | string \| null | Last fetched tree SHA (for drift detection). |
| `skills[].path` | string \| null | Subpath inside the source repo, when applicable. |
| `skills[].agents` | string[] | Targeted agents. `["*"]` = all enabled. |
| `skills[].profiles` | string[] | Profile gates. Empty = always active. |
| `skills[].project` | string \| null | Project alias scope. |
| `skills[].active` | bool | False when the skill is deactivated (`[off]`). |
| `skills[].upstream` | string \| null | Origin source for snapshotted (`local:`) entries. |

Every field is always present, even when null/empty, so consumers can rely on
the shape without defaulting. New fields may be added without bumping the
version; renames or semantic changes will bump it.

## `ateam skills show`

Print the `SKILL.md` for a locked skill to stdout. Reads the snapshot at
`<repo>/skills/<name>/SKILL.md` (or the `local:` path for user-authored skills).
Useful for piping into `less`, `grep`, or another agent.

```bash
ateam skills show deploy-to-vercel
ateam skills show deploy-to-vercel | less
```

If the snapshot is missing, ateam tells you to run `ateam apply` first.

## `ateam skills find`

Search the [skills.sh](https://skills.sh) registry. Two modes:

```bash
ateam skills find deploy vercel     # non-interactive: print matches and exit
ateam skills find                   # interactive picker (TTY only)
```

Pipe-friendly. The non-interactive form prints `owner/repo --skill <name>` lines
you can feed straight into `ateam skills add`. Run from a non-TTY shell with no
query and ateam prints a two-step hint instead of opening a picker.

## `ateam skills import`

Adopt an installed-locally skill (or your global `CLAUDE.md` / `AGENTS.md`) into
the synced lockfile.

```bash
ateam skills import                                   # bulk: every skill on disk + instructions
ateam skills import <name>                            # snapshot a single skill into <repo>/skills/
ateam skills import <name> --upstream github:foo/bar  # track upstream instead of snapshotting
ateam skills import <name> --project canva            # tag with project alias
ateam skills import --instructions                    # only adopt CLAUDE.md / AGENTS.md as the template
```

Bulk mode (no name) walks `~/.claude/skills`, `~/.codex/skills`, and
`~/.agents/skills`, plus the global `CLAUDE.md` / `AGENTS.md`. When the two
instruction files differ, ateam shows an interactive picker so you choose which
becomes the canonical template. Orphan snapshot directories (already in
`<repo>/skills/` but missing from the lockfile) are adopted instead of erroring.

For each adopted skill, ateam also auto-discovers upstream by inspecting the
on-disk skill folder for a `.git/config` or sibling git checkout — so a skill
imported from a local clone of `github.com/foo/bar` gets a `github:foo/bar`
source automatically. Pass `--upstream` to override.

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

## `ateam remote`

Manage the ateam-config repo's git remote without dropping into `git -C`.

```bash
ateam remote add <git-url>           # set origin and push current branch upstream
ateam remote list                    # print configured remotes (`git remote -v`)
```

`remote add` refuses to clobber an existing `origin` and rolls itself back if
the initial push fails (so you don't end up half-configured).

## `ateam validate`

Lint the instructions template at `<repo>/instructions/instructions.md.hbs`.
Checks that every Handlebars identifier referenced in the template is either a
declared profile or one of the reserved identifiers (`claude`, `codex`,
`hostname`).

```bash
ateam validate
```

Exits zero if the template is missing (nothing to validate) or all identifiers
are declared. Exits non-zero with a list of undeclared identifiers otherwise.
