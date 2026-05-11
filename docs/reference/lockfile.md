---
title: Lockfile format
description: The one TOML file that records every skill agents manages.
---

`<repo>/agents.lock.toml` is the single source of truth for which skills should
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

Every entry has an implicit `active = true`. `agents skills deactivate <name>`
sets it to `false`, which causes `apply` and `update` to skip that entry and
unlinks it from every enabled harness's skills directory (e.g.,
`~/.claude/skills/`, `~/.codex/skills/`, `~/.config/opencode/skills/`,
`~/.gemini/skills/`). The lockfile entry
(and the cached source content) stays put so `agents skills activate <name>`
re-materializes it without refetching. The flag rides with the skill across
the team — deactivating in one machine syncs everywhere.

## Subagents

Subagents are stored as a single canonical Markdown file at
`<repo>/agents/<name>.md`. On every `apply`, agents **renders** the canonical
into each enabled harness's native format under `<repo>/agents/rendered/`, then
symlinks the harness path to that rendered file:

| Harness | Output path | Format |
|---|---|---|
| Claude Code | `~/.claude/agents/<name>.md` | YAML frontmatter + Markdown body |
| Codex | `~/.codex/agents/<name>.toml` | TOML with `developer_instructions` holding the body |
| OpenCode | `~/.config/opencode/agents/<name>.md` | YAML frontmatter (no `name`, derived from filename) + body |
| Gemini | `~/.gemini/agents/<name>.md` | YAML frontmatter + body |

A single canonical symlink can't serve all four harnesses because Codex needs
`.toml` with different field names, so each harness receives a symlink to its
own rendered file in the agents repo.

### Canonical file shape

```markdown
---
name: code-reviewer
description: PR reviewer focused on correctness, security, and missing tests.

# Per-harness model selection — only the keys you set get rendered
model:
  claude: sonnet                  # sonnet | opus | haiku | full-id | inherit
  codex: gpt-5.3-codex-spark
  opencode: anthropic/claude-sonnet
  gemini: gemini-2.5-pro

# Effort: only Claude + Codex understand it
effort:
  claude: medium                  # low | medium | high | xhigh | max
  codex: medium                   # rendered as model_reasoning_effort

# Shared
skills: [code-review-checklist]
color: yellow                     # red | blue | green | yellow | purple | orange | pink | cyan
---

You are a senior code reviewer focused on correctness, security,
and missing tests. Prioritize behavior regressions and missing
test coverage.
```

Required: `name`, `description`, body. Everything else is optional and only
emitted into a harness's rendered output if set. No translation across naming
conventions — agents doesn't map `Read` ↔ `read_file` or `sonnet` ↔ a Codex
model id. Set the right value per harness.

### Lockfile entry

```toml
[[subagent]]
name = "code-reviewer"
source = "github:vercel-labs/agent-skills"
path = "agents/code-reviewer.md"      # path within the source repo
ref = "main"                          # optional pin
file_sha = "ba7816bf…"                # sha256 of the canonical file at install time
harnesses = ["*"]                     # filter; "*" = every harness with subagent support
profiles = ["work"]                   # optional
active = true                         # absent or true = install
```

Same `active`, profile, and project semantics as `[[skill]]`. Harnesses
without a subagent install path are silently skipped (today every harness has
one — claude-code, codex, opencode, gemini).

## Source types

| Prefix | Use |
|---|---|
| `github:owner/repo` | A GitHub repo (default for `owner/repo` shorthand) |
| `git:<url>` | Any other git URL |
| `local:skills/<name>` | A user-authored skill kept in `<repo>/skills/` |
| `local:agents/<name>.md` | A user-authored subagent kept in `<repo>/agents/` |

## Update detection

For `github:` entries, `agents skills update` calls
`GET /repos/{owner}/{repo}/git/trees/{ref}?recursive=1`, walks the tree to find
the entry matching `path`, and compares its SHA with `tree_sha`. One API call
per skill. `git:` entries use `git ls-remote`; `local:` entries hash the source
directory.

## Duplicate-name validator

If two `[[skill]]` entries share a `name`, agents refuses to load the lockfile
and prints both offending entries. Resolve the conflict in your editor, then
re-run.

## Subpath validator

For `github:` and `git:` entries, `path` must be a relative subpath inside the
source repo. Components like `..` (parent traversal) or absolute paths are
rejected at lockfile load — agents refuses before any tarball is extracted, so
a malicious entry can't escape the package root. `local:` entries skip this
check because their `path` records the source location itself.
