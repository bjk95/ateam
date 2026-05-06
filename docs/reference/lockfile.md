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
agents = ["*"]                        # which agents to install for; "*" = all
profiles = ["work"]                   # optional; absent = all machines
project = "canva"                     # optional; absent = global scope
```

## Source types

| Prefix | Use |
|---|---|
| `github:owner/repo` | A GitHub repo (default for `owner/repo` shorthand) |
| `git:<url>` | Any other git URL |
| `local:skills/<name>` | A user-authored skill kept in `<repo>/skills/` |

## Update detection

For `github:` entries, `ateam update` calls
`GET /repos/{owner}/{repo}/git/trees/{ref}?recursive=1`, walks the tree to find
the entry matching `path`, and compares its SHA with `tree_sha`. One API call
per skill. `git:` entries use `git ls-remote`; `local:` entries hash the source
directory.

## Duplicate-name validator

If two `[[skill]]` entries share a `name`, ateam refuses to load the lockfile
and prints both offending entries. Resolve the conflict in your editor, then
re-run.
