---
title: Quickstart
description: First 5 minutes — bootstrap, install shared entries, sync to another machine.
---

This walks through the five minutes from a fresh shell to having synced entries
visible in every enabled harness.

## 1. Bootstrap a fresh repo

```bash
agents init --scaffold --profiles personal
```

This creates `~/.config/agents/` with the lockfile, `.agents/` state dir, and an
initial git commit. No remote yet — that's optional.

## 2. Install a skill from skills.sh

```bash
agents skills add vercel-labs/agent-skills --skill deploy-to-vercel -y
```

Behind the scenes agents:

1. Fetches the skill folder from the GitHub repo.
2. Records a lockfile entry with the GitHub tree SHA so updates are detectable.
3. Symlinks each enabled harness's skills path into the canonical copy inside
   the agents repo.
4. Auto-commits the lockfile change. (Push happens once you wire a remote.)

## 3. Wire a remote — auto-sync activates

```bash
agents remote add git@github.com:you/agents-config.git
```

`remote add` sets `origin` and pushes the current branch upstream in one step.
From now on every `agents skills add` / `update` / `remove` pulls, commits, and
pushes without you ever typing `git`.

MCP servers use the same sync path:

```bash
agents mcp add context7 --profile work -- npx -y @upstash/context7-mcp
```

## 4. On a second machine

```bash
agents init git@github.com:you/agents-config.git --profiles work
agents apply
```

`init` clones the config repo. `apply` reads the lockfile, refetches every
remote skill into a cold cache, creates the same symlinks, and writes matching
MCP config. Every enabled harness now sees the same matching setup.

## Next

- [Project-scoped skills](/concepts/project-scope/) — install a skill into a
  specific project's harness-local skills dirs instead of user-global.
- [Profiles](/concepts/profiles/) — gate skills, subagents, and MCP servers by
  machine (work / personal / devbox).
- [`agents` CLI reference](/reference/cli/) — every flag.
