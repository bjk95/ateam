---
title: Production readiness validation
description: One-by-one validation notes and resolved issues for the agents CLI production readiness behaviors.
---

# Production readiness validation

Validated against `agents` v0.2.45 on 2026-05-08. Evidence came from source
review, the existing Rust test suite, and temporary-directory CLI probes that
kept the real home/config/cache state untouched.

## Validation commands

```bash
cargo test
cargo test --test production_readiness
pnpm --dir site build
```

Targeted probes also covered:

- `agents --quiet remote list`
- `agents --no-sync apply --dry-run`
- repeated `agents --no-sync apply`
- `agents --no-sync apply --copy` followed by `skills deactivate` and `skills remove`
- `agents --no-sync skills find deploy`

Review clarification: row 49 validates import of external subagent files into
agents' internal canonical representation. The canonical multi-harness
frontmatter in the lockfile docs is an internal storage/rendering format, not
an external standard that third-party sources are expected to publish.

## Resolved issues

| ID | Behaviors | Severity | Issue | Resolution | Regression coverage |
|---|---:|---|---|---|---|
| PRI-001 | 7 | P1 | `--quiet` was not global because several commands printed directly, bypassing `ui::quiet`. | Normal command output now routes through quiet-aware UI helpers; warnings/errors remain visible. | `quiet_suppresses_remote_list_plain_output` |
| PRI-002 | 17 | P1 | `apply --dry-run` swept `.agents/tmp` and could pre-pull before planning. | Dry-run skips temp sweeping and auto-sync pre-pull. | `apply_dry_run_preserves_tmp_dirs` |
| PRI-003 | 18 | P2 | Repeated `apply` changed `.agents/manifest.toml` because `applied_at` was regenerated and the file was always rewritten. | Manifest entries preserve prior timestamps when path/kind/skill/harness/target are unchanged, and manifest writes are skipped when serialized content is unchanged. | `repeated_apply_keeps_manifest_content_stable` |
| PRI-004 | 37, 39 | P1 | `skills deactivate` and `skills remove` left copy-mode skill installs on disk. | Both paths now use manifest `EntryKind` to dispatch symlinks to `uninstall_path` and copies to `uninstall_copy`. | `deactivate_removes_copy_mode_skill_install`, `remove_removes_copy_mode_skill_install` |
| PRI-005 | 44 | P2 | `skills find` printed Vercel-style `npx skills` output instead of agents install commands. | Non-interactive search results now emit `agents skills add <source> --skill <name>` lines. | `non_interactive_result_formats_agents_install_command` |
| PRI-006 | 33 | P3 | Registry fallback was reported as logging duplicate warnings when registry lookup errored. | Rechecked the current source and confirmed there is a single warning path. | Source review; covered by single remaining warning path. |

## One-by-one validation

| # | Area | Status | Validation notes |
|---:|---|---|---|
| 1 | Fresh install | Pass | `init --scaffold` resolves a target, writes `agents.toml`, `agents.lock.toml`, `.gitignore`, `.agents/`, initializes git, and soft-fails only the initial commit if git identity is missing. |
| 2 | Fresh clone | Pass | `init <git-url>` clones into the target, refuses non-empty targets, ensures state dirs, and writes the pointer/machine config. |
| 3 | Custom repo path | Pass | `init --repo <path>` writes the XDG pointer when the target differs from the default repo path. |
| 4 | Machine profiles | Pass | `--profiles` is parsed as comma-delimited values and persisted to `.agents/machine.toml`; `apply` uses `profile_match` for skills, subagents, and instructions context. |
| 5 | Existing repo detection | Pass | `status` resolves the repo, reads lockfile/manifest/machine config, reports profile and health details, and does not write state. |
| 6 | Missing repo recovery | Pass | `paths::resolve_repo` errors with the expected pointer/default locations and `run agents init` recovery text. |
| 7 | Global quiet mode | Pass | Normal output routes through quiet-aware UI helpers; machine-readable `skills list --json` / `--names` output remains direct by design. |
| 8 | Verbose diagnosis | Pass | `status` emits repo and manifest details through `ui::detail`, which is gated by `--verbose`; list output also adds scope/profile detail in verbose mode. |
| 9 | Read-only safety | Pass | `is_mutating` leaves `status`, `skills list/show/find`, `validate`, `project list`, `remote list`, `instructions diff/show`, and `subagents list` unlocked. |
| 10 | Mutating serialization | Pass | Mutating commands acquire `RepoLock`; integration test `apply_prints_wait_message_when_repo_lock_is_held` validates lock waiting. |
| 11 | Fail-fast lock mode | Pass | `RepoLock::acquire(..., no_wait = true)` returns a clear contention error; unit coverage validates the branch. |
| 12 | Remote setup | Pass | `remote add` refuses non-git repos, adds `origin`, pushes the current branch with upstream tracking, and rolls back `origin` when push fails. |
| 13 | Remote protection | Pass | `remote add` checks for an existing `origin` and bails with the current URL before changing git config. |
| 14 | Manual sync | Pass | `sync` delegates to `git pull --rebase --autostash` and push; it does not stage, commit, or rewrite lockfile content itself. |
| 15 | Offline tolerance | Pass | Auto-sync pre-pull and push detect common offline errors, warn, and keep local state. |
| 16 | Clean apply | Pass | `apply` materializes active, profile-matching skills, subagents, and instructions across resolved harnesses. |
| 17 | Apply dry run | Pass | Dry-run previews planned writes without sweeping `.agents/tmp`, writing files, updating manifests, creating backups, or auto-syncing. |
| 18 | Idempotent apply | Pass | Repeated apply keeps managed files correct and preserves manifest content when the desired state is unchanged. |
| 19 | Harness filtering | Pass | `apply -a <harness>` builds a target harness set and skips unmatched skill/subagent outputs. |
| 20 | Project filtering | Pass | `apply --project <alias>` only processes entries whose `project` matches that alias. |
| 21 | Unregistered project | Pass | Missing project aliases are collected, warned, and skipped while other entries continue. |
| 22 | Matching directory auto-heal | Pass | `install_symlink` hashes existing real dirs/files against the canonical target and auto-heals matching content. |
| 23 | Conflicting directory refusal | Pass | `install_symlink` and `install_copy_dir` return `Refused` for foreign real paths when `force` is false. |
| 24 | Forced conflict recovery | Pass | `--force` moves foreign paths to `<name>.bak.<unix-ts>` before installing managed output. |
| 25 | Copy install mode | Pass | `apply --copy` installs directories through `install_copy_dir` and records manifest entries as `kind = "copy"`. |
| 26 | Copy-to-symlink transition | Pass | A later symlink-mode apply auto-heals byte-identical copied dirs into symlinks and records symlink manifest entries. |
| 27 | Skill add happy path | Pass | `skills add` fetches, snapshots remote sources, upserts the lockfile, installs into harnesses, and auto-commits when sync is enabled. |
| 28 | Skill add list mode | Pass with caveat | `--list` avoids lockfile/snapshot/manifest/harness changes, but still acquires the mutating command lock and may pre-pull/fetch into temp state. |
| 29 | Vercel compatibility | Pass | `normalize_all_flag` makes `--all` imply wildcard skill selection, wildcard harnesses, and `-y`. |
| 30 | Explicit harness install | Pass | Explicit `-a` values are preserved in the lockfile and used for install targeting. |
| 31 | Profile-gated skill | Pass | `--profile` is stored on the lock entry and `apply` skips machines without a matching profile. |
| 32 | Project-scoped skill | Pass | `--project` requires a registered alias, stores it in the lockfile, and installs under that project root. |
| 33 | Unknown skill fallback | Pass | Registry fallback exists and uses the skills.sh download endpoint for missing GitHub skills; registry lookup failures now produce one warning. |
| 34 | OpenClaw risk gate | Pass | `Source::parse_with` rejects `openclaw/*` sources unless the explicit risk flag is set. |
| 35 | Skill update | Pass | `skills update` compares upstream SHAs/hashes, refetches drifted snapshots, updates `tree_sha`, and leaves unchanged entries alone. |
| 36 | Deactivated update skip | Pass | Deactivated entries are excluded from bulk updates and explicitly skipped with a warning when named. |
| 37 | Skill deactivate | Pass | Deactivation removes both symlink-mode and copy-mode installs using the manifest entry kind. |
| 38 | Skill activate | Pass | `skills activate` flips `active = true`, writes the lockfile, and invokes apply to re-materialize eligible entries. |
| 39 | Skill remove | Pass | Removal deletes lockfile entries, managed snapshots, and both symlink-mode and copy-mode installs. |
| 40 | Missing skill remove | Pass | Target resolution bails before removal when a named skill is missing in the selected scope. |
| 41 | Pipe-friendly list | Pass | `skills list` switches stdout to names-only when stdout is not a TTY; `skills remove` reads whitespace-separated names from stdin. |
| 42 | JSON list contract | Pass | `skills list --json` suppresses the banner/UI output and emits the versioned JSON envelope with all documented fields. |
| 43 | Skill show | Pass | `skills show` resolves the canonical skill dir and prints only `SKILL.md`, erroring if the snapshot is missing. |
| 44 | Registry search | Pass | Non-interactive search emits pipe-friendly `agents skills add <source> --skill <name>` commands; the TTY picker still installs selected results interactively. |
| 45 | Bulk import | Pass | Bulk import adopts eligible local skills, dedupes across harness dirs, skips managed/plugin skills, and can adopt orphan snapshots. |
| 46 | Instructions import | Pass | `skills import --instructions` writes the template, adds `[instructions]`, and records existing output files in the manifest. |
| 47 | Instructions validation | Pass | `validate` checks template identifiers against declared profiles plus reserved harness/hostname identifiers. |
| 48 | Instructions conflict | Pass | Non-interactive apply refuses foreign instruction files; interactive apply offers skip/cancel/overwrite, and `--force` backs up then writes. |
| 49 | Subagent add | Pass | `subagents add` imports external Claude-format Markdown, converts it into agents' internal canonical Markdown, stores lockfile metadata, and renders native harness outputs. The internal canonical format is not treated as a required external import format. |
| 50 | Self-update | Code-validated | `upgrade` calls the updater, reports updated or already-at-latest, and refreshes the update-check cache. It was not run as a probe because it can replace the local binary. |
