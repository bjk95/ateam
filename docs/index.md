---
title: ateam
description: Multi-machine AI skills sync — Vercel-compatible CLI with invisible git auto-sync.
template: doc
---

# ateam

`ateam` is a Rust CLI that syncs AI coding-assistant skills across your machines.
It speaks the same flags as `npx skills add`, stores everything in one git repo,
and runs `git pull` / `git commit` / `git push` invisibly so a skill installed on
your work machine appears on your personal machine after `ateam apply`.

## What you get

- A drop-in for `npx skills add` (`--list`, `--skill`, `--all`, `-a`, `-y`, `-g`).
- One lockfile, one `~/.config/ateam/` repo, zero pollution of project repos.
- Project-scoped skills via per-machine path aliases — different filesystem
  layout on every machine, same identifiers.
- Soft-disable via `skills deactivate` / `skills activate` — quarantine a skill
  team-wide before deleting it, no telemetry needed.
- Auto-sync wraps every mutating command. You never type `git`.

## Status

v1 is functional end-to-end against the live `vercel-labs/agent-skills` registry.
See the [installation guide](/install/) to get running, then read the
[quickstart](/quickstart/) for the first 5 minutes.

## Source

`ateam` is open source. [Read the code on GitHub](https://github.com/bradleykester/ateam).
