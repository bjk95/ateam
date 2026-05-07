---
title: Troubleshooting
description: Common errors and how to recover.
---

## `no agents repo found`

You haven't run `agents init` on this machine, or you removed the pointer file
without telling agents where to look.

```bash
agents init <git-url>                 # clone existing repo
# or
agents init --scaffold                # fresh repo at ~/.config/agents/
```

## `git pull --ff-only refused: local and remote have diverged`

Two machines committed lockfile changes without one pushing first. agents
refuses to mutate until it's resolved.

```bash
cd ~/.config/agents        # or wherever your pointer points
git pull --rebase
# resolve any TOML conflicts in your editor
git push
agents apply
```

## `note: N lockfile entries reference unregistered project aliases`

A project-scoped entry refers to an alias this machine hasn't registered.
Either register it or ignore — the entries are simply skipped.

```bash
agents project add canva ~/work/canva
agents apply
```

## Real directory at a target install path

`agents apply` first checks whether the existing directory matches the snapshot
at `<repo>/skills/<name>/` byte-for-byte. If it does, agents auto-heals: removes
the dir and replaces it with a symlink. No flag needed — the data is already
in the snapshot, so nothing is lost.

If the contents differ, apply refuses. Two ways to recover:

1. Delete or move the existing directory yourself and re-run `agents apply`.
2. Run `agents apply --force` — agents moves the conflicting directory aside
   to `<name>.bak.<unix-ts>` rather than deleting it.

## `Author identity unknown` during `agents init` or auto-sync

git itself isn't configured. One-time per machine:

```bash
git config --global user.email "you@example.com"
git config --global user.name "Your Name"
```

agents soft-fails on commit/push errors — your local change is kept; you can
re-run after fixing the issue and the queued commit will go out.
