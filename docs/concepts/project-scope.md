---
title: Project scope
description: Install a skill into a specific project's native agent dirs, by alias.
---

A skill can be tagged with a `project` alias in the lockfile. When `ateam apply`
encounters a project-tagged entry, it installs into that project's native agent
discovery paths (`<project>/.claude/skills/<name>` and
`<project>/.codex/skills/<name>`) instead of `~/.claude/skills/<name>`.

Aliases — not paths — because the same project lives at different filesystem
locations across machines.

## Register a project on this machine

```bash
ateam project add canva ~/work/canva
ateam project list
```

This writes to `<repo>/.ateam/machine.toml`, which is gitignored. Each machine
maintains its own alias→path map, so the synced lockfile stays portable.

## Install a skill scoped to a project

```bash
ateam add canva/agent-skills --skill style-guide --project canva -y
```

Or auto-detect from cwd:

```bash
cd ~/work/canva
ateam add canva/agent-skills --skill style-guide -y
```

ateam walks up from the current directory; if a parent matches a registered
project's path, it tags the install with that alias.

## On a machine that hasn't registered the alias

`ateam apply` skips the entry silently and prints a nudge:

```
note: 1 lockfile entry references unregistered project alias:
  - canva (style-guide)
register with: ateam project add <alias> <path>
```

This is expected — your personal machine probably has no `canva` checkout.
