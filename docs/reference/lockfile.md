---
title: Lockfile format
description: The one TOML file that records every synced artifact agents manages.
---

`<repo>/agents.lock.toml` is the single source of truth for which skills,
subagents, and MCP servers should be installed or configured on a given machine.
It's committed to git; every machine reads the same file and reconciles local
harness state against it.

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

Every skill, subagent, and MCP entry has an implicit `active = true`. Deactivate
commands set it to `false`, which causes `apply` to skip that entry and remove
its managed harness state. For skills, this unlinks from every enabled harness's
skills directory (e.g.,
`~/.claude/skills/`, `~/.codex/skills/`, `~/.config/opencode/skills/`,
`~/.gemini/skills/`). The lockfile entry
(and any cached source content) stays put so `activate` re-materializes it
without refetching. The flag rides with the entry across the team —
deactivating in one machine syncs everywhere.

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

## MCP servers

MCP servers are lockfile entries too. They do not have source snapshots because
agents is managing a harness config stanza, not executable code.

```toml
[[mcp]]
name = "otter"
transport = "stdio"                  # default when omitted
command = "otter"
args = ["mcp", "serve"]
env = { OTTER_HOST = "https://example.internal" }
harnesses = ["codex", "claude-code"] # "*" = every enabled harness with MCP support
profiles = ["work"]                  # optional
active = true                        # absent or true = configure
```

HTTP servers use `url` instead of `command`:

```toml
[[mcp]]
name = "docs"
transport = "http"
url = "https://example.com/mcp"
bearer_token_env_var = "DOCS_TOKEN"  # optional
harnesses = ["*"]
```

Same `active`, profile, and harness semantics as `[[skill]]`. `agents apply`
only writes entries whose profiles match the current machine. It removes stale
managed entries from a harness config when the profile no longer matches, when
the entry is deactivated, or when the entry is removed from the lockfile.

Supported MCP config renderers:

| Harness | Config written |
|---|---|
| `codex` | `~/.codex/config.toml` under `[mcp_servers.<name>]` |
| `claude-code` | `~/.claude.json` under top-level `mcpServers.<name>` |

Unmanaged MCP entries already present in those files are preserved. Harnesses
without a supported MCP config renderer are rejected by `agents mcp add`.

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

If two `[[skill]]`, `[[subagent]]`, or `[[mcp]]` entries share a `name` within
that table type, agents refuses to load the lockfile. Resolve the conflict in
your editor, then re-run.

## Subpath validator

For `github:` and `git:` entries, `path` must be a relative subpath inside the
source repo. Components like `..` (parent traversal) or absolute paths are
rejected at lockfile load — agents refuses before any tarball is extracted, so
a malicious entry can't escape the package root. `local:` entries skip this
check because their `path` records the source location itself.
