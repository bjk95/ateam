---
title: Quickstart
description: First 5 minutes — bootstrap, install a skill, sync to another machine.
---

This walks through the five minutes from a fresh shell to having a synced skill
visible in both Claude Code and Codex.

## 1. Bootstrap a fresh repo

```bash
ateam init --scaffold --profiles personal
```

This creates `~/.config/ateam/` with the lockfile, `.ateam/` state dir, and an
initial git commit. No remote yet — that's optional.

## 2. Install a skill from skills.sh

```bash
ateam skills add vercel-labs/agent-skills --skill deploy-to-vercel -y
```

Behind the scenes ateam:

1. Fetches the skill folder from the GitHub repo.
2. Records a lockfile entry with the GitHub tree SHA so updates are detectable.
3. Symlinks `~/.claude/skills/deploy-to-vercel` and `~/.codex/skills/deploy-to-vercel`
   into the canonical copy inside the ateam repo.
4. Auto-commits the lockfile change. (Push happens once you wire a remote.)

## 3. Wire a remote — auto-sync activates

```bash
git -C ~/.config/ateam remote add origin git@github.com:you/ateam-config.git
git -C ~/.config/ateam push -u origin main
```

From now on every `ateam skills add` / `update` / `remove` pulls, commits, and
pushes without you ever typing `git`.

## 4. On a second machine

```bash
ateam init git@github.com:you/ateam-config.git --profiles work
ateam apply
```

`init` clones the config repo. `apply` reads the lockfile, refetches every
remote skill into a cold cache, and creates the same symlinks. Both Claude Code
and Codex now see the same skill.

## Next

- [Project-scoped skills](/concepts/project-scope/) — install a skill into a
  specific project's `.claude/skills/` instead of user-global.
- [Profiles](/concepts/profiles/) — gate skills by machine (work / personal /
  devbox).
- [`ateam` CLI reference](/reference/cli/) — every flag.
