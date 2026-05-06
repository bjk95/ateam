---
title: Troubleshooting
description: Common errors and how to recover.
---

## `no ateam repo found`

You haven't run `ateam init` on this machine, or you removed the pointer file
without telling ateam where to look.

```bash
ateam init <git-url>                 # clone existing repo
# or
ateam init --scaffold                # fresh repo at ~/.config/ateam/
```

## `git pull --ff-only refused: local and remote have diverged`

Two machines committed lockfile changes without one pushing first. ateam
refuses to mutate until it's resolved.

```bash
cd ~/.config/ateam        # or wherever your pointer points
git pull --rebase
# resolve any TOML conflicts in your editor
git push
ateam apply
```

## `note: N lockfile entries reference unregistered project aliases`

A project-scoped entry refers to an alias this machine hasn't registered.
Either register it or ignore — the entries are simply skipped.

```bash
ateam project add canva ~/work/canva
ateam apply
```

## Symlink at target points elsewhere

ateam refuses to overwrite a real directory at a target install path. Two ways
to recover:

1. Delete or move the existing directory yourself and re-run `ateam apply`.
2. Run `ateam apply --force` — ateam will move the conflicting directory aside
   to `<name>.bak.<unix-ts>` rather than deleting it.

## `Author identity unknown` during `ateam init` or auto-sync

git itself isn't configured. One-time per machine:

```bash
git config --global user.email "you@example.com"
git config --global user.name "Your Name"
```

ateam soft-fails on commit/push errors — your local change is kept; you can
re-run after fixing the issue and the queued commit will go out.
