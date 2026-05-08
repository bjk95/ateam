---
title: Production readiness behaviors
description: Desired state, action, and post-action state checks for validating the agents CLI before release.
---

# Production readiness behaviors

Use this as a release readiness audit for `agents`. Each row describes the
state a user starts in, the action they take, and the state that must be true
after the action completes.

| # | Area | Desired state | Action | Post-action state |
|---:|---|---|---|---|
| 1 | Fresh install | No agents repo exists and no pointer file is configured. | Run `agents init --scaffold`. | A new agents repo exists, initial config files are present, and the command exits successfully without requiring a remote. |
| 2 | Fresh clone | A remote agents-config repo exists and the local machine has no repo. | Run `agents init <git-url>`. | The repo is cloned, local machine state is initialized, and `agents status` points at the cloned repo. |
| 3 | Custom repo path | A user wants the agents repo outside `~/.config/agents`. | Run `agents init --repo <path> --scaffold`. | The pointer targets the custom path and later commands resolve that repo without extra flags. |
| 4 | Machine profiles | A machine should only materialize profile-matching entries. | Run `agents init --scaffold --profiles work,devbox`. | Machine config records both profiles and profile-gated skills or subagents apply only when matched. |
| 5 | Existing repo detection | A valid agents repo is already configured. | Run `agents status`. | The CLI prints repo path, profiles, manifest health, and exits zero without mutating files. |
| 6 | Missing repo recovery | The pointer is missing or references a deleted repo. | Run `agents status`. | The CLI gives a clear recovery path to `agents init` and exits non-zero without creating partial state. |
| 7 | Global quiet mode | Automation needs machine-readable silence except failures. | Run a successful read-only command with `--quiet`. | Non-error output is suppressed while warnings and errors still go to stderr. |
| 8 | Verbose diagnosis | A user needs enough detail to debug path or SHA mismatches. | Run `agents status --verbose` or another command with `--verbose`. | Output includes diagnostic paths, SHAs, and per-harness details without changing command semantics. |
| 9 | Read-only safety | A user wants to inspect configuration without taking a lock. | Run `agents skills list`, `agents validate`, or `agents remote list`. | The command reads current state, does not acquire the repo write lock, and leaves the worktree untouched. |
| 10 | Mutating serialization | Two shells start mutating commands against the same repo. | Run concurrent `agents skills add` or `agents apply` invocations. | One process holds `.agents/lock`, the other waits, and lockfile or manifest writes are not interleaved. |
| 11 | Fail-fast lock mode | CI should not wait behind another mutating process. | Run a mutating command with `--no-wait` while the lock is held. | The command exits non-zero with a lock message and makes no changes. |
| 12 | Remote setup | A local agents repo has no `origin`. | Run `agents remote add <git-url>`. | `origin` is configured and the current branch is pushed upstream, or the command rolls back on push failure. |
| 13 | Remote protection | A repo already has an `origin`. | Run `agents remote add <new-url>`. | The command refuses to clobber the existing remote and leaves git config unchanged. |
| 14 | Manual sync | Local commits should be reconciled without changing the lockfile. | Run `agents sync`. | The repo runs pull/rebase/autostash then push; no lockfile entries are added, removed, or rewritten by sync itself. |
| 15 | Offline tolerance | Network access is unavailable during an auto-syncing command. | Run a mutating command that succeeds locally but cannot push. | Local state remains written, the sync failure is reported as a warning, and the user can rerun sync after reconnecting. |
| 16 | Clean apply | Lockfile entries exist and no harness files are installed. | Run `agents apply`. | Active matching skills, subagents, and instructions are materialized into every enabled target harness. |
| 17 | Apply dry run | A user wants to preview materialization. | Run `agents apply --dry-run`. | Planned creations, removals, or conflicts are shown and no files, manifest entries, or backups are written. |
| 18 | Idempotent apply | The manifest already matches the lockfile and filesystem. | Run `agents apply` again. | The command exits successfully without duplicate symlinks, duplicate manifest entries, or unnecessary rewrites. |
| 19 | Harness filtering | A user wants to apply only one harness. | Run `agents apply -a codex`. | Only Codex target paths are reconciled and other harness files remain unchanged. |
| 20 | Project filtering | A project-scoped entry exists for a registered alias. | Run `agents apply --project <alias>`. | Only entries scoped to that project alias are materialized into that project's harness directories. |
| 21 | Unregistered project | The lockfile references a project alias missing on this machine. | Run `agents apply`. | The CLI warns or notes the missing alias, skips those entries, and applies all other eligible entries. |
| 22 | Matching directory auto-heal | A real directory exists at a target skill path and matches the canonical snapshot byte-for-byte. | Run `agents apply`. | The directory is replaced with the expected symlink or copy mode output without needing `--force`. |
| 23 | Conflicting directory refusal | A real directory exists at a target skill path with different contents. | Run `agents apply`. | The command refuses to overwrite, explains the conflict, and leaves the directory intact. |
| 24 | Forced conflict recovery | A user explicitly accepts replacing a conflicting target directory. | Run `agents apply --force`. | The existing directory is moved to a timestamped backup and the managed target is installed. |
| 25 | Copy install mode | Symlinks are unsuitable for the target filesystem. | Run `agents apply --copy`. | Managed artifacts are installed by recursive copy and the manifest records copy mode for future reconciliation. |
| 26 | Copy-to-symlink transition | A previous apply used `--copy` and symlinks are now acceptable. | Run `agents apply` without `--copy`. | Managed copied artifacts are safely replaced with symlinks and manifest mode returns to the default. |
| 27 | Skill add happy path | A GitHub or registry skill exists and the user accepts install. | Run `agents skills add <source> --skill <name> -y`. | The skill snapshot is fetched, the lockfile is updated, harness targets are installed, and the change is auto-committed when sync is enabled. |
| 28 | Skill add list mode | A user wants discovery before installing. | Run `agents skills add <source> --list`. | Available skills are printed and no lockfile, snapshot, manifest, or harness files are changed. |
| 29 | Vercel compatibility | A user expects `npx skills add` style flags. | Run `agents skills add <source> --all`. | The command treats this as all skills, all enabled harnesses, and yes-to-prompts behavior. |
| 30 | Explicit harness install | A skill should only target selected harnesses. | Run `agents skills add <source> --skill <name> -a codex -a claude-code -y`. | The lockfile records only those harnesses and apply installs the skill only for those harnesses. |
| 31 | Profile-gated skill | A skill should only apply on some machines. | Run `agents skills add <source> --skill <name> --profile work -y`. | The lockfile records the profile gate and machines without `work` skip the skill. |
| 32 | Project-scoped skill | A skill should live in a specific project, not globally. | Run `agents skills add <source> --skill <name> --project <alias> -y`. | The lockfile records the alias and the skill installs into that project's harness directories. |
| 33 | Unknown skill fallback | A named GitHub skill is not found in the source tree but exists in the registry cache. | Run `agents skills add github-owner/repo --skill <name> -y`. | The CLI falls back to the registry blob endpoint and installs the cached snapshot. |
| 34 | OpenClaw risk gate | A source can run runtime shell commands and requires explicit user consent. | Run `agents skills add openclaw/<repo> --skill <name>` without the risk flag. | The command refuses with an explanation and no lockfile or filesystem state is changed. |
| 35 | Skill update | A locked remote skill has a newer upstream tree SHA. | Run `agents skills update <name>`. | The snapshot and `tree_sha` are updated, active harness targets are refreshed, and unchanged skills are left alone. |
| 36 | Deactivated update skip | A locked skill is soft-disabled. | Run `agents skills update`. | The deactivated entry is skipped and remains uninstalled until activated. |
| 37 | Skill deactivate | A skill must be quarantined without losing history. | Run `agents skills deactivate <name>`. | The lockfile keeps the entry with `active = false` and all installed harness targets for that skill are removed. |
| 38 | Skill activate | A quarantined skill is ready to return. | Run `agents skills activate <name>`. | The lockfile sets the entry active again and the next apply materializes it into eligible harnesses. |
| 39 | Skill remove | A user removes one or more known skills. | Run `agents skills remove <name> -y`. | The lockfile entries and managed harness targets are removed, while user-authored local source directories are preserved. |
| 40 | Missing skill remove | A user tries to remove a skill not present in the selected scope. | Run `agents skills remove <name> -y`. | The command exits non-zero, explains the missing entry, and removes nothing else. |
| 41 | Pipe-friendly list | A user composes list output with shell tools. | Run `agents skills list | agents skills remove -y`. | `skills list` emits names only on stdout and downstream commands can consume it without parsing decorative output. |
| 42 | JSON list contract | An editor integration needs stable structured output. | Run `agents skills list --json`. | Stdout contains only the versioned JSON object with every documented field present. |
| 43 | Skill show | A user wants to inspect the installed skill source. | Run `agents skills show <name>`. | The command prints the canonical `SKILL.md` from the snapshot or local source without mutating state. |
| 44 | Registry search | A user searches for available skills. | Run `agents skills find <query>`. | Matching install commands are printed in a pipe-friendly form, or the TTY picker opens when no query is provided interactively. |
| 45 | Bulk import | A user has existing harness-local skills and instructions. | Run `agents skills import`. | Eligible local skills and instructions are adopted into the agents repo, already locked entries are skipped, and plugin-managed skills are not taken over. |
| 46 | Instructions import | A user wants shared global instructions managed by agents. | Run `agents skills import --instructions`. | The chosen global instructions file becomes `instructions/instructions.md.hbs`, `[instructions]` is recorded, and rendered harness files are tracked in the manifest. |
| 47 | Instructions validation | A template may reference undeclared Handlebars identifiers. | Run `agents validate`. | The command exits zero when all identifiers are declared or reserved, and exits non-zero with undeclared identifiers listed otherwise. |
| 48 | Instructions conflict | A foreign `CLAUDE.md`, `AGENTS.md`, or equivalent already exists at an output path. | Run `agents apply` interactively. | The user can skip on this machine, cancel, or overwrite with backup; non-interactive mode refuses unless `--force` is provided. |
| 49 | Subagent add | An external subagent file exists in a supported import format. | Run `agents subagents add <source> --subagent <name> -y`. | The subagent is converted into agents' internal canonical Markdown, lockfile metadata is recorded, and every eligible harness receives its native rendered format. |
| 50 | Self-update | A newer released `agents` binary exists. | Run `agents upgrade`. | The installed binary is replaced in place and reports old and new versions, or reports already-at-latest when no update is needed. |
