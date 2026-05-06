---
title: Lockfile format
description: The one TOML file that records every skill ateam manages.
---

`<repo>/ateam.lock.toml` is the single source of truth for which skills should
be installed on a given machine. It's committed to git; every machine reads the
same file and reconciles its symlinks against it.

## Schema

```toml
[[skill]]
name = "deploy-to-vercel"             # normalized: lowercase + hyphens
source = "github:vercel-labs/agent-skills"
path = "skills/deploy-to-vercel"      # subpath within the source repo
ref = "main"                          # optional pin; default = repo default
tree_sha = "1378aa50…"                # GitHub tree SHA at install time
harnesses = ["*"]                     # which harnesses to install for; "*" = all
                                      # Valid ids: "claude-code", "codex",
                                      # "opencode", "gemini". See concepts/harness.md.
profiles = ["work"]                   # optional; absent = all machines
project = "canva"                     # optional; absent = global scope
active = false                        # optional; absent or true = install; false = soft-disabled
```

## The `active` flag

Every entry has an implicit `active = true`. `ateam skills deactivate <name>`
sets it to `false`, which causes `apply` and `update` to skip that entry and
unlinks it from every enabled harness's skills directory (e.g.,
`~/.claude/skills/`, `~/.codex/skills/`, `~/.config/opencode/skills/`,
`~/.gemini/skills/`). The lockfile entry
(and the cached source content) stays put so `ateam skills activate <name>`
re-materializes it without refetching. The flag rides with the skill across
the team — deactivating in one machine syncs everywhere.

## Source types

| Prefix | Use |
|---|---|
| `github:owner/repo` | A GitHub repo (default for `owner/repo` shorthand) |
| `git:<url>` | Any other git URL |
| `local:skills/<name>` | A user-authored skill kept in `<repo>/skills/` |

## Update detection

For `github:` entries, `ateam skills update` calls
`GET /repos/{owner}/{repo}/git/trees/{ref}?recursive=1`, walks the tree to find
the entry matching `path`, and compares its SHA with `tree_sha`. One API call
per skill. `git:` entries use `git ls-remote`; `local:` entries hash the source
directory.

## Duplicate-name validator

If two `[[skill]]` entries share a `name`, ateam refuses to load the lockfile
and prints both offending entries. Resolve the conflict in your editor, then
re-run.

## Subpath validator

For `github:` and `git:` entries, `path` must be a relative subpath inside the
source repo. Components like `..` (parent traversal) or absolute paths are
rejected at lockfile load — ateam refuses before any tarball is extracted, so
a malicious entry can't escape the package root. `local:` entries skip this
check because their `path` records the source location itself.
