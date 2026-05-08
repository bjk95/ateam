---
title: Project scope
description: Install a skill into a specific project's native harness dirs, by alias.
---

A skill can be tagged with a `project` alias in the lockfile. When `agents apply`
encounters a project-tagged entry, it installs into that project's native harness
discovery paths, such as `<project>/.claude/skills/<name>`,
`<project>/.codex/skills/<name>`,
`<project>/.config/opencode/skills/<name>`, and
`<project>/.gemini/skills/<name>`, instead of user-global harness paths.

Aliases — not paths — because the same project lives at different filesystem
locations across machines.

## Register a project on this machine

```bash
agents project add canva ~/work/canva
agents project list
```

This writes to `<repo>/.agents/machine.toml`, which is gitignored. Each machine
maintains its own alias→path map, so the synced lockfile stays portable.

## Install a skill scoped to a project

```bash
agents skills add canva/agent-skills --skill style-guide --project canva -y
```

Or auto-detect from cwd:

```bash
cd ~/work/canva
agents skills add canva/agent-skills --skill style-guide -y
```

agents walks up from the current directory; if a parent matches a registered
project's path, it tags the install with that alias.

If cwd is inside a git repo that has *not* been registered, `agents skills add`
prompts to install project-scoped (auto-registering the repo's directory name
as the alias) or globally — defaulting to project. Pass `-g`/`--global` to
skip the prompt and force global. With `-y` and no TTY, the prompt
auto-resolves to project; without `-y` and no TTY, it falls through to global.

## On a machine that hasn't registered the alias

`agents apply` skips the entry silently and prints a nudge:

```
note: 1 lockfile entry references unregistered project alias:
  - canva (style-guide)
register with: agents project add <alias> <path>
```

This is expected — your personal machine probably has no `canva` checkout.
