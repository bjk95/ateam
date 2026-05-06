# Surface unmanaged skills in `ateam status`

**Date:** 2026-05-06
**Status:** Awaiting user approval

## Goal

When Claude, Codex, or another agent authors a skill via its native path (e.g. `anthropic-skills:skill-creator` writing into `~/.claude/skills/<name>/`), the resulting directory is invisible to ateam until the user remembers to run `ateam skills import`. The discoverability gap is the actual friction — `import` already does the right thing once invoked.

`ateam status` is extended to surface this drift. When unmanaged skill directories exist in any of the watched agent dirs, status prints one body line counting them and pointing at `ateam skills import`. Verbose mode lists names and the dirs each one was seen in.

The headline (`✓ ateam` / `⚠ ateam`) is unchanged — unmanaged skills are not a broken state, just an "adoption pending" state.

## Out of scope

- A new `ateam skills create` command (rejected during brainstorming: AI agents already author skills well; ateam scaffolding would duplicate worse).
- Auto-importing on detection (rejected: removes user review before content lands in the synced repo).
- A `--ignore` list for skills the user deliberately keeps unmanaged (deferred: no observed need; revisit when one lands).
- Drift surfacing for instructions, hooks, agents, or MCP configs (this change is scoped to skills only).

## High-level flow

```
ateam status
  │
  ├─► load lockfile, manifest, machine config
  ├─► count_dangling          [existing]
  ├─► unpushed_count          [existing]
  ├─► discover_unmanaged      [NEW]
  │     ├─ scan ~/.claude/skills, ~/.codex/skills, ~/.agents/skills
  │     ├─ for each entry: skip if hidden, symlink-into-ateam, or in lockfile
  │     ├─ dedup by name; aggregate dirs per name
  │     └─ return Vec<UnmanagedSkill>, sorted by name
  │
  └─► render
        ├─ headline (unchanged: ⚠ iff dangling > 0 or unpushed > 0)
        ├─ "N skills installed"
        ├─ "N projects: …"
        ├─ broken-links line (existing)
        ├─ unpushed line (existing)
        └─ "N unmanaged skills in <dirs> — run: ateam skills import"   [NEW; default]
              under -v: per-skill list with origin dirs
```

### Key invariants

1. The headline never flips because of unmanaged skills. Only `dangling > 0 || unpushed > 0` triggers ⚠.
2. `discover_unmanaged` is read-only. It never mutates the lockfile, manifest, or filesystem.
3. The same definition of "unmanaged" is used by both `status` and the bulk-import path, via a shared helper. There is one source of truth for the rule.

## Components

### 1. `src/discover.rs` — one new public function + struct

The module already owns `walk_package` for source-dir discovery. It's the natural home for "scan agent dirs."

**Public surface:**

```rust
pub struct UnmanagedSkill {
    pub name: String,
    pub dirs: Vec<PathBuf>,  // each agent dir it was found in
}

pub fn discover_unmanaged(
    repo: &Path,
    home: &Path,
    lock: &Lockfile,
) -> Vec<UnmanagedSkill>;
```

**Internals (private):**

- `agent_skill_dirs(home: &Path) -> Vec<PathBuf>` — moved here from `commands/import.rs`. Returns `[~/.claude/skills, ~/.codex/skills, ~/.agents/skills]`. Both modules now call it.
- Skip rules per directory entry:
  - `name.starts_with('.')` → skip (hidden)
  - Not a directory or symlink → skip
  - Symlink whose target starts with `paths::cache_dir(repo)` or `paths::local_skills_dir(repo)` → skip (already managed)
  - `lock.find(&name).is_some()` → skip (lockfile-listed counts as managed, even if `active: false`)
- Cross-tool dedup by `name`. Aggregate `dirs` per name.
- A missing agent dir (`NotFound`) is silently skipped — matches the existing `bulk_import_skills` behavior.
- Returns sorted by name for stable output ordering.

### 2. `src/commands/status.rs` — render the new line

`status::run` currently takes no arguments. Two changes:

1. Add a `verbose: bool` parameter, plumbed from the global `Cli::verbose` in `cli.rs::dispatch`.
2. After the existing unpushed-commits block, append:

```rust
let unmanaged = discover::discover_unmanaged(&repo, &home, &lock);
if !unmanaged.is_empty() {
    let dirs_summary = summarize_dirs(&unmanaged); // e.g. "~/.claude, ~/.codex"
    ui::plain(format!(
        "  {} unmanaged skill{} in {} — run: ateam skills import",
        unmanaged.len(),
        if unmanaged.len() == 1 { "" } else { "s" },
        dirs_summary,
    ));
    if verbose {
        for u in &unmanaged {
            let where_ = u.dirs.iter()
                .map(|p| paths::display_path(p))
                .collect::<Vec<_>>()
                .join(", ");
            ui::detail(format!("  - {} (in {})", u.name, where_));
        }
    }
}
```

`summarize_dirs` is a small private helper that takes the union of dirs across all unmanaged skills, shortens each via `paths::display_path` (existing `~`-collapsing helper used elsewhere in status), and joins them. If three dirs all appear, the line says "in ~/.claude, ~/.codex, ~/.agents."

### 3. `src/commands/import.rs` — refactor only

Replace the inline directory walk in `bulk_import_skills` with a call into `discover::discover_unmanaged`, then for each result run the existing snapshot + upsert + error-collection flow. Keeps a single source of truth for what "unmanaged" means.

This is the main risk area. `bulk_import_skills` does several things during iteration: canonicalize symlinks before snapshotting, per-skill `Result` capture into `outcome.errors`, the "already snapshotted" branch when the dest dir exists. The refactor must preserve all of them. Plan:

- `discover_unmanaged` returns *what* needs adopting.
- The per-skill snapshot/upsert work stays in `bulk_import_skills`, now driven by the returned list.

If preserving the error semantics turns out to add awkwardness, fall back to leaving `bulk_import_skills` untouched and having `discover_unmanaged` duplicate the iteration. File a follow-up bead for the cleanup. The status feature itself does not depend on the refactor.

### 4. `src/cli.rs` — one dispatch change

The `Skills` arm already plumbs `no_sync`. The `Status` arm currently calls `crate::commands::status::run()`. Change to `crate::commands::status::run(cli.verbose)`. No new flag, no help-surface change.

## Edge cases

| Case | Behavior |
|---|---|
| Empty agent dir | not counted |
| Agent dir does not exist on disk | silently skipped |
| Symlink whose target is inside the ateam repo cache or local-skills dir | skipped (managed) |
| Symlink to a path outside the ateam repo (user manually symlinked) | reported as unmanaged. User can ignore the message; revisit if anyone complains |
| Same skill name present in both `~/.claude/skills` and `~/.codex/skills` | counted once, both dirs listed under `-v` |
| Skill name in lockfile but no directory exists on disk | not counted by `discover_unmanaged`; surfaced via the existing `dangling` channel |
| Deactivated skill (lockfile entry, `active: false`, directory absent) | not counted (no directory to flag) |
| Hidden directory like `.foo/` | skipped |

## Tests

The codebase uses `#[cfg(test)] mod tests` inline in each module — there is no top-level `tests/` directory. Add unit tests in `src/discover.rs` for the new `discover_unmanaged` function, since that's where the logic lives. Render-side concerns (the formatted line, verbose vs. default) are exercised through `discover_unmanaged`'s return shape, so they don't need a separate `status::run` test harness for this change.

Cases (all in `discover.rs`):

1. **unmanaged_skill_detected** — tmp `repo` + tmp `home`. Drop a non-symlink dir at `$HOME/.claude/skills/foo/`. Empty lockfile. Assert `discover_unmanaged` returns one entry, `name = "foo"`, `dirs = [~/.claude/skills]`.
2. **symlink_into_repo_is_managed** — create the same dir as a symlink whose target is under `paths::local_skills_dir(repo)`. Assert empty result.
3. **lockfile_match_is_managed** — directory `$HOME/.claude/skills/foo/` exists as a real dir, but `lock` has an entry for `foo`. Assert empty result (lockfile match wins regardless of symlink status).
4. **cross_tool_dedup** — same name in both `~/.claude/skills/foo/` and `~/.codex/skills/foo/`. Assert one returned entry, `dirs.len() == 2`, both dirs present.
5. **hidden_dir_skipped** — `.foo/` in `~/.claude/skills/`. Assert empty result.
6. **missing_agent_dir_silent** — `~/.codex/skills` does not exist on disk. Assert no error, no entries.
7. **sorted_by_name** — drop dirs `c/`, `a/`, `b/`. Assert returned names are `["a", "b", "c"]`.

Render-side coverage (the headline-stays-✓ invariant) is implicit — the new line is appended after the existing dangling/unpushed checks without touching them, and the existing status tests, if any, will catch a regression there. If existing status coverage is thin, leave that as a follow-up; do not block this change on backfilling unrelated tests.

## Risks & mitigations

| Risk | Mitigation |
|---|---|
| Refactoring `bulk_import_skills` introduces a regression in import behavior | Run the full existing import test suite after the refactor. If any test breaks and the fix is non-trivial, drop the refactor and file follow-up. The status feature lands either way. |
| `discover_unmanaged` runs on every status call. Cost = 3 × `read_dir` + a stat per entry. | Trivial. Status is interactive, not hot-path. No caching needed. |
| `verbose` parameter ripples through other commands' dispatch | Only `status` needs it for this change. Don't pre-emptively plumb it elsewhere. |
