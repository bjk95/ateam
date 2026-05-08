---
title: Production readiness validation
description: One-by-one validation notes and discovered issues for the agents CLI production readiness behaviors.
---

# Production readiness validation

Validated against `agents` v0.2.45 on 2026-05-08. Evidence came from source
review, the existing Rust test suite, and temporary-directory CLI probes that
kept the real home/config/cache state untouched.

## Validation commands

```bash
cargo test
pnpm --dir site build
```

Targeted probes also covered:

- `agents --quiet remote list`
- `agents --no-sync apply --dry-run`
- repeated `agents --no-sync apply`
- `agents --no-sync apply --copy` followed by `skills deactivate` and `skills remove`
- `agents --no-sync skills find deploy`

## Discovered issues

| ID | Behaviors | Severity | Issue | Evidence | Suggested action |
|---|---:|---|---|---|---|
| PRI-001 | 7 | P1 | `--quiet` is not global. Several commands print directly with `println!` / `eprintln!`, bypassing `ui::quiet`. | `agents --quiet remote list` printed `origin`; direct prints exist in `validate`, `remote`, `harness list`, `instructions`, `import`, `activate`, `deactivate`, and `self_update`. | Route command output through `ui::*` or explicitly gate direct prints on `ui::is_quiet()`. |
| PRI-002 | 17 | P1 | `apply --dry-run` is not side-effect free. | Probe showed `.agents/tmp/stale-dir` was removed during dry-run; source calls `install::sweep_tmp(&repo)` before checking `dry_run`, and auto-sync pre-pull can also mutate git state unless `--no-sync` is passed. | Skip temp sweeping and git pre-pull for dry-runs, or document and rename the behavior as a planning run with repo-maintenance side effects. |
| PRI-003 | 18 | P2 | Repeated `apply` rewrites `.agents/manifest.toml` even when state is already correct. | Probe showed manifest content changed after a second apply because `applied_at = now_unix()` is regenerated and the manifest is always written. | Preserve existing manifest entries when unchanged, or omit volatile timestamps from idempotent entries. |
| PRI-004 | 37, 39 | P1 | `skills deactivate` and `skills remove` do not uninstall copy-mode skill installs. | Copy-mode probe left `~/.codex/skills/alpha` as a real directory after both commands; both paths call `install::uninstall_path`, which refuses non-symlinks. | Use manifest `EntryKind` to dispatch to `uninstall_copy` for copy entries, as `apply` already does during reconciliation. |
| PRI-005 | 44 | P2 | `skills find` output is not the documented pipe-friendly agents command form. | Probe printed `Install with npx skills add <owner/repo@skill>` and rows like `vercel-labs/agent-skills@deploy-to-vercel`, not `agents skills add <owner/repo> --skill <name>`. | Emit copy-pasteable `agents skills add <source> --skill <name>` lines, or update the behavior checklist/docs to match the Vercel-style output. |
| PRI-006 | 49 | P1 | `subagents add` does not accept the canonical multi-harness subagent format described by the lockfile docs. | The implementation imports Claude-format YAML frontmatter and promotes `model` / `effort` strings into Claude slots; canonical `model: { claude: ... }` frontmatter is not accepted by this path. | Accept canonical agents subagent files directly, and keep Claude-format import as a compatibility path. |
| PRI-007 | 33 | P3 | Registry fallback logs duplicate warnings when the registry lookup errors. | `resolve_via_registry` contains two identical `ui::warn(format!("registry lookup failed ..."))` calls in the same error branch. | Remove the duplicate warning. |

## One-by-one validation

| # | Area | Status | Validation notes |
|---:|---|---|---|
| 1 | Fresh install | Pass | `init --scaffold` resolves a target, writes `agents.toml`, `agents.lock.toml`, `.gitignore`, `.agents/`, initializes git, and soft-fails only the initial commit if git identity is missing. |
| 2 | Fresh clone | Pass | `init <git-url>` clones into the target, refuses non-empty targets, ensures state dirs, and writes the pointer/machine config. |
| 3 | Custom repo path | Pass | `init --repo <path>` writes the XDG pointer when the target differs from the default repo path. |
| 4 | Machine profiles | Pass | `--profiles` is parsed as comma-delimited values and persisted to `.agents/machine.toml`; `apply` uses `profile_match` for skills, subagents, and instructions context. |
| 5 | Existing repo detection | Pass | `status` resolves the repo, reads lockfile/manifest/machine config, reports profile and health details, and does not write state. |
| 6 | Missing repo recovery | Pass | `paths::resolve_repo` errors with the expected pointer/default locations and `run agents init` recovery text. |
| 7 | Global quiet mode | Fail | See PRI-001. `ui::*` output is suppressed, but direct `println!` output is not. |
| 8 | Verbose diagnosis | Pass | `status` emits repo and manifest details through `ui::detail`, which is gated by `--verbose`; list output also adds scope/profile detail in verbose mode. |
| 9 | Read-only safety | Pass | `is_mutating` leaves `status`, `skills list/show/find`, `validate`, `project list`, `remote list`, `instructions diff/show`, and `subagents list` unlocked. |
| 10 | Mutating serialization | Pass | Mutating commands acquire `RepoLock`; integration test `apply_prints_wait_message_when_repo_lock_is_held` validates lock waiting. |
| 11 | Fail-fast lock mode | Pass | `RepoLock::acquire(..., no_wait = true)` returns a clear contention error; unit coverage validates the branch. |
| 12 | Remote setup | Pass | `remote add` refuses non-git repos, adds `origin`, pushes the current branch with upstream tracking, and rolls back `origin` when push fails. |
| 13 | Remote protection | Pass | `remote add` checks for an existing `origin` and bails with the current URL before changing git config. |
| 14 | Manual sync | Pass | `sync` delegates to `git pull --rebase --autostash` and push; it does not stage, commit, or rewrite lockfile content itself. |
| 15 | Offline tolerance | Pass | Auto-sync pre-pull and push detect common offline errors, warn, and keep local state. |
| 16 | Clean apply | Pass | `apply` materializes active, profile-matching skills, subagents, and instructions across resolved harnesses. |
| 17 | Apply dry run | Fail | See PRI-002. It previews planned writes, but still sweeps `.agents/tmp` and can pre-pull. |
| 18 | Idempotent apply | Fail | See PRI-003. Files remain correct, but manifest content is rewritten on every run. |
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
| 33 | Unknown skill fallback | Pass with issue | Registry fallback exists and uses the skills.sh download endpoint for missing GitHub skills; see PRI-007 for duplicate error warnings. |
| 34 | OpenClaw risk gate | Pass | `Source::parse_with` rejects `openclaw/*` sources unless the explicit risk flag is set. |
| 35 | Skill update | Pass | `skills update` compares upstream SHAs/hashes, refetches drifted snapshots, updates `tree_sha`, and leaves unchanged entries alone. |
| 36 | Deactivated update skip | Pass | Deactivated entries are excluded from bulk updates and explicitly skipped with a warning when named. |
| 37 | Skill deactivate | Fail for copy mode | Symlink-mode deactivation works; copy-mode installs are left on disk. See PRI-004. |
| 38 | Skill activate | Pass | `skills activate` flips `active = true`, writes the lockfile, and invokes apply to re-materialize eligible entries. |
| 39 | Skill remove | Fail for copy mode | Lockfile removal and symlink uninstall work; copy-mode installs are left on disk. See PRI-004. |
| 40 | Missing skill remove | Pass | Target resolution bails before removal when a named skill is missing in the selected scope. |
| 41 | Pipe-friendly list | Pass | `skills list` switches stdout to names-only when stdout is not a TTY; `skills remove` reads whitespace-separated names from stdin. |
| 42 | JSON list contract | Pass | `skills list --json` suppresses the banner/UI output and emits the versioned JSON envelope with all documented fields. |
| 43 | Skill show | Pass | `skills show` resolves the canonical skill dir and prints only `SKILL.md`, erroring if the snapshot is missing. |
| 44 | Registry search | Fail | See PRI-005. Search works, but output does not match the desired agents-command form. |
| 45 | Bulk import | Pass | Bulk import adopts eligible local skills, dedupes across harness dirs, skips managed/plugin skills, and can adopt orphan snapshots. |
| 46 | Instructions import | Pass | `skills import --instructions` writes the template, adds `[instructions]`, and records existing output files in the manifest. |
| 47 | Instructions validation | Pass | `validate` checks template identifiers against declared profiles plus reserved harness/hostname identifiers. |
| 48 | Instructions conflict | Pass | Non-interactive apply refuses foreign instruction files; interactive apply offers skip/cancel/overwrite, and `--force` backs up then writes. |
| 49 | Subagent add | Fail for canonical input | See PRI-006. Claude-format import works and renders native outputs, but canonical multi-harness files are not accepted by `subagents add`. |
| 50 | Self-update | Code-validated | `upgrade` calls the updater, reports updated or already-at-latest, and refreshes the update-check cache. It was not run as a probe because it can replace the local binary. |
