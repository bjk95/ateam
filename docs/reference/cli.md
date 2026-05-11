---
title: CLI
description: Every agents command and flag.
---

## Global flags

These work on every subcommand.

| Flag | Behavior |
|---|---|
| `--no-sync` | Skip auto pull/commit/push for this invocation. Equivalent: `AGENTS_NO_SYNC=1`. |
| `--no-wait` | Fail fast if another `agents` process holds the repo lock instead of waiting. |
| `-v` / `--verbose` | Show extra detail (paths, SHAs, per-harness links). |
| `-q` / `--quiet` | Suppress non-error output: banner, success lines, progress spinners, plain text. Errors and warnings still print to stderr. |

### Concurrent invocations

Mutating commands (`apply`, `sync`, `skills add`/`update`/`remove`/`import`/`activate`/`deactivate`,
`mcp add`/`remove`/`activate`/`deactivate`, `project add`/`remove`, `remote add`, `edit`, `instructions edit`) take an exclusive
`flock` on `<repo>/.agents/lock` for the duration of the command. A second invocation
waits for the first to finish before reading and writing `agents.lock.toml` and
`.agents/manifest.toml`, so concurrent runs cannot clobber each other's edits.

Pass `--no-wait` to fail fast instead of blocking. Read-only commands (`status`,
`skills list`/`show`/`find`, `mcp list`, `instructions diff`/`show`, `validate`, `project list`,
`remote list`) take no lock.

## `agents init`

Bootstrap a fresh agents-config repo or clone an existing one.

```bash
agents init                         # interactive: clone or scaffold?
agents init <git-url>               # clone into ~/.config/agents/
agents init --scaffold              # fresh empty repo at ~/.config/agents/
agents init --repo <path>           # use a non-default location
agents init ... --profiles a,b      # set this machine's profile list
```

## `agents apply`

Materialize the lockfile (active entries only).

```bash
agents apply [--dry-run] [-a <harness>...] [--project <alias>] [--force]
```

If a real directory already sits at a target skill path (e.g. a skill installed
by hand or by `npx skills`), apply deletes that harness-local copy and replaces
it with a symlink to the canonical snapshot at `<repo>/skills/<name>/`. Stale
cross-tool aliases under `.agents/skills` are also removed for managed skills.
Instructions and subagents keep the older conflict behavior: foreign real paths
are refused unless `--force` is passed, in which case they are moved aside to
`<name>.bak.<unix-ts>`.

Harness targets are always symlinks. Skills point at their canonical snapshot
under `<repo>/skills/`; instructions and subagents point at per-harness rendered
files under the agents repo.

MCP servers are reconciled into supported harness config files instead of
symlinked. Today agents manages Codex `~/.codex/config.toml` and Claude Code
`~/.claude.json`.

## `agents status`

```bash
agents status                       # repo path, profiles, manifest health
```

## `agents sync`

Reconcile the agents-config repo with git without changing the lockfile.

```bash
agents sync                         # git pull --rebase --autostash, then git push
```

`sync` is useful when you only want to pull remote changes and push local
commits. It does not stage or commit working-tree changes; state-changing
`agents` commands still handle those auto-commits themselves.

## `agents skills add` (Vercel-compatible)

Drop-in replacement for `npx skills add` — same flags, swap `npx` for `agents`.

| Flag | Behavior |
|---|---|
| `<repo>` (positional) | `owner/repo` shorthand, full git URL, or local path |
| `--list` | Print discovered skills, don't install |
| `--skill <name>` | Specific skill names, repeatable; `*` = all |
| `--all` | Implies `--skill '*'`, `--harness "*"`, and `-y` (Vercel-compat triple-flag override) |
| `-a` / `--harness <name>` | Target harnesses, repeatable; `*` = all enabled |
| `-y` / `--yes` | Non-interactive; in an unregistered git repo, auto-registers + project-scopes |
| `-g` / `--global` | Force global scope (overrides cwd auto-detect and the auto-register prompt) |
| `--profile <name>` | Tag entry with profile gates (repeatable) |
| `--project <alias>` | Install into a registered project |
| `--ref <ref>` | Pin to a specific git ref/tag/commit |
| `--dangerously-accept-openclaw-risks` | Permit `openclaw/*` sources (which can shell out at runtime) |
| `--no-sync` | Skip auto pull/commit/push for this run |

### skills.sh registry fallback

When `--skill <name>` doesn't match anything in the cloned GitHub repo's tree,
agents falls back to the [skills.sh](https://skills.sh) blob endpoint and
installs from the registry's snapshot. Mirrors `npx skills add` — covers
skills that have been renamed, moved, or removed upstream but are still
served from the registry's cache. Only fires for github sources with an
explicit `--skill <name>` (not `--all` / `*`).

## `agents skills update`

Check GitHub tree SHAs and refetch any drifted skills. Skips deactivated entries.

```bash
agents skills update                # all active entries
agents skills update <name>...      # specific entries
agents skills update --global       # only global-scoped entries
agents skills update --project foo  # only entries tagged with project alias `foo`
```

| Flag | Behavior |
|---|---|
| `-y` / `--yes` | Non-interactive (skip confirmation prompts) |
| `-g` / `--global` | Only update entries without a project scope |
| `--project <alias>` | Only update entries tagged with this project alias |
| `--no-sync` | Skip auto pull/commit/push for this run |

`--global` and `--project` are mutually exclusive.

## `agents skills remove`

Delete one or more skills from the lockfile and uninstall their managed harness
symlinks. Legacy managed copies from older versions are also cleaned up. If any
name isn't in the lockfile (within the selected scope), nothing is removed and
the command errors.

```bash
agents skills remove <name>...                  # one or more positional names
agents skills remove --all                      # every locked skill
agents skills remove --all -g                   # every globally-scoped skill
agents skills remove --all -a claude            # every skill targeting claude
agents skills remove foo bar -y                 # skip the confirmation prompt
```

| Flag | Behavior |
|---|---|
| `<name>...` (positional) | Skill names to remove. Repeatable. |
| `--all` | Remove every locked skill (within `--harness` / `--global` scope). |
| `-y` / `--yes` | Skip the confirmation prompt (also skipped when stdin is not a TTY). |
| `-a` / `--harness <name>` | Only target entries whose harnesses list matches. Repeatable. |
| `-g` / `--global` | Only target entries with no project alias. |

A confirmation prompt lists the skills about to be removed and defaults to "no".
Pass `-y` (or pipe stdin) to skip it.

When no positional names are given, `--all` isn't set, and stdin is a pipe,
names are read from stdin (whitespace-separated). `agents skills list` auto-
switches to names-only output when its stdout is a pipe, so the obvious form
just works:

```bash
agents skills list | agents skills remove
agents skills list --project canva | agents skills remove
```

Pass `--names` explicitly if you want plain names on a TTY (e.g. into a file).

Local-source directories under `<repo>/skills/` are never deleted by agents — you
remove them yourself if you want them gone.

## `agents skills deactivate` / `agents skills activate`

Soft-disable a skill without losing its lockfile entry. Deactivating immediately
removes its managed harness symlinks and any legacy managed copies; activating
re-materializes it.

```bash
agents skills deactivate <name>
agents skills activate <name>
```

The `active` flag rides with the skill in the lockfile, so deactivating on one
machine deactivates everywhere after the next sync. `agents skills list` marks
deactivated entries with `[off]`.

## `agents skills list`

```bash
agents skills list                  # all locked skills (active + [off])
agents skills list --project canva  # only entries scoped to one project
agents skills list --json           # versioned JSON for editor integrations
agents skills list --names          # force one-name-per-line output on a TTY
```

Entries are sorted by source (remote) alphabetically, then by name within each
source. This applies to all output modes (default, `--names`, `--json`).

When stdout is not a terminal (i.e. piped or redirected), `list` auto-switches
to plain names-only output — same as passing `--names` — so it composes cleanly
with `xargs` and `agents skills remove`. `--json` overrides this and always
emits JSON.

```bash
agents skills list | agents skills remove                   # remove all (with prompt)
agents skills list --project canva | xargs agents skills remove -y
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
      "harnesses": ["*"],
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
| `skills[].harnesses` | string[] | Targeted harnesses. `["*"]` = all enabled. |
| `skills[].profiles` | string[] | Profile gates. Empty = always active. |
| `skills[].project` | string \| null | Project alias scope. |
| `skills[].active` | bool | False when the skill is deactivated (`[off]`). |
| `skills[].upstream` | string \| null | Origin source for snapshotted (`local:`) entries. |

Every field is always present, even when null/empty, so consumers can rely on
the shape without defaulting. New fields may be added without bumping the
version; renames or semantic changes will bump it.

## `agents skills show`

Print the `SKILL.md` for a locked skill to stdout. Reads the snapshot at
`<repo>/skills/<name>/SKILL.md` (or the `local:` path for user-authored skills).
Useful for piping into `less`, `grep`, or another harness.

```bash
agents skills show deploy-to-vercel
agents skills show deploy-to-vercel | less
```

If the snapshot is missing, agents tells you to run `agents apply` first.

## `agents skills find`

Search the [skills.sh](https://skills.sh) registry. Two modes:

```bash
agents skills find deploy vercel     # non-interactive: print matches and exit
agents skills find                   # interactive picker (TTY only)
```

Pipe-friendly. The non-interactive form prints complete
`agents skills add <source> --skill <name>` commands. Run from a non-TTY shell
with no query and agents prints a two-step hint instead of opening a picker.

## `agents import`

Adopt local harness state into the synced lockfile.

```bash
agents import                                   # bulk: skills + instructions + MCP servers
agents import <name>                            # snapshot a single skill into <repo>/skills/
agents import <name> --upstream github:foo/bar  # track upstream instead of snapshotting
agents import <name> --project canva            # tag with project alias
agents import --instructions                    # only adopt CLAUDE.md / AGENTS.md as the template
agents import --mcp                             # only adopt supported MCP config entries
```

Bulk mode (no name) walks every registered harness's skills directory, plus
the cross-tool `~/.agents/skills` alias. It also imports global instructions
from `CLAUDE.md` / `AGENTS.md`. When the two instruction files differ, agents
shows an interactive picker so you choose which becomes the canonical template.
Orphan snapshot directories (already in
`<repo>/skills/` but missing from the lockfile) are adopted instead of erroring.
Bulk mode also adopts MCP servers from supported harness configs: Codex
`~/.codex/config.toml` and Claude Code `~/.claude.json`. Existing lockfile MCP
entries are skipped, and same-name entries from multiple harnesses are merged
only when their server config matches.

Plugin-managed skills (those installed via `claude plugin add` from a
marketplace) are detected through `~/.claude/plugins/installed_plugins.json`
and **skipped** — both bulk and single import refuse to take ownership, since
Claude's plugin updater would race agents's symlink on the next plugin sync.
Single import errors out with the upstream source named; bulk prints a `·`
line per skipped plugin skill and continues. Manage those skills via the
`claude plugin` commands instead.

For each adopted skill, agents also auto-discovers upstream by inspecting the
on-disk skill folder for a `.git/config` or sibling git checkout — so a skill
imported from a local clone of `github.com/foo/bar` gets a `github:foo/bar`
source automatically. Pass `--upstream` to override.

## `agents subagents`

Manage subagents — one canonical Markdown file per subagent at
`<repo>/agents/<name>.md` with multi-harness frontmatter. On every `apply`
agents renders the canonical into repo-local per-harness files, then symlinks
each harness path to its native format:
`~/.claude/agents/<name>.md` (Markdown), `~/.codex/agents/<name>.toml` (TOML),
`~/.config/opencode/agents/<name>.md`, `~/.gemini/agents/<name>.md`.

```bash
agents subagents add <source> [--subagent <name>]... [--path <file>] [-a <harness>]... [--ref <ref>] [--profile <name>]... [-y]
agents subagents remove <name>... [-y]
agents subagents list
```

See [Lockfile format → Subagents](./lockfile.md#subagents) for the canonical
schema (universal `name`/`description` + per-harness `model.*`/`effort.*` +
shared `skills`/`color`).

### `subagents add`

`<source>` is the same set of forms as `skills add`: `owner/repo`,
`https://...`, `git@...`, or a local path (file or directory).
Imported subagent files use the supported external import format, currently
Claude-compatible Markdown frontmatter. Agents then stores the subagent in its
own canonical multi-harness format before rendering native harness outputs.

| Flag | Behavior |
|---|---|
| `--subagent <name>` | Subagent name (repeatable). By default looks for `agents/<name>.md` in the source. |
| `--path <file>` | Explicit file path within the source. Implies a single subagent; name comes from the file stem unless `--subagent` is also given. |
| `-a` / `--harness <name>` | Target harnesses; `*` = all enabled with subagent support. |
| `--ref <ref>` | Pin to a specific git ref/tag/commit. |
| `--profile <name>` | Annotate lockfile entry with profile gates. |
| `-y` / `--yes` | Skip confirmation prompts (non-interactive). |

Local file shortcut: `agents subagents add ./agents/foo.md` snapshots the file
verbatim and derives the name from its stem. Useful for adopting a
hand-authored subagent into the lockfile in one step.

### `subagents remove`

Removes the lockfile entry, deletes the canonical at `<repo>/agents/<name>.md`,
and uninstalls the rendered files from each harness's agents dir.

### `subagents list`

Prints every locked subagent with its source. `●` marks active entries,
`○` marks deactivated.

## `agents mcp`

Manage MCP server definitions in the shared lockfile and materialize them into
supported harness config files.

```bash
agents mcp add <name> [-a <harness>]... [--profile <name>]... [--env KEY=VALUE]... -- <command> [args...]
agents mcp add <name> --url <url> [--bearer-token-env-var <env>] [-a <harness>]... [--profile <name>]...
agents mcp remove <name>...
agents mcp deactivate <name>...
agents mcp activate <name>...
agents mcp list
```

Examples:

```bash
agents mcp add otter --harness codex --profile canva -- otter mcp serve
agents mcp add docs --url https://example.com/mcp --bearer-token-env-var DOCS_TOKEN
```

`--profile` and `--harness` use the same semantics as skills and subagents:
profiles choose which machines get the server, and harnesses choose which
enabled harness configs are written. `*` means every enabled harness with MCP
support. Unsupported MCP harness renderers are rejected at add time.

Deactivation keeps the lockfile entry with `active = false` and removes the
managed server from harness config files. Removal deletes the lockfile entry
and also removes the managed server from harness config files. Unmanaged MCP
servers in those files are preserved.

## `agents upgrade`

Self-update: download the latest `agents` release and replace this binary.
Bypasses the 24h TTL check that runs implicitly before every other command.

```bash
agents upgrade
```

Prints `agents: updated X → Y` on success or `agents: already at latest (X)`
when no upgrade was needed. Exits non-zero on failure.

The implicit check runs at most once every 24 hours, soft-fails on any
network/filesystem error, and never blocks the command you actually ran.
There is no env-var opt-out; if you don't want updates, ignore the
occasional `agents: updated …` line — the cache is at `~/.cache/agents/`.

## `agents project`

Manage this machine's alias→path map.

```bash
agents project add <alias> <path>     # register
agents project list                   # show
agents project remove <alias>         # forget
```

`add` accepts `register` as a hidden alias for muscle memory.

## `agents remote`

Manage the agents-config repo's git remote without dropping into `git -C`.

```bash
agents remote add <git-url>           # set origin and push current branch upstream
agents remote list                    # print configured remotes (`git remote -v`)
```

`remote add` refuses to clobber an existing `origin` and rolls itself back if
the initial push fails (so you don't end up half-configured).

## `agents validate`

Lint the instructions template at `<repo>/instructions/instructions.md.hbs`.
Checks that every Handlebars identifier referenced in the template is either a
declared profile or one of the reserved identifiers (`claude`, `codex`,
`opencode`, `gemini`, `hostname`).

```bash
agents validate
```

Exits zero if the template is missing (nothing to validate) or all identifiers
are declared. Exits non-zero with a list of undeclared identifiers otherwise.
