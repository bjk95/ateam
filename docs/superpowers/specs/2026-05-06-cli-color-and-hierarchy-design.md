# CLI color and hierarchy refresh

**Status:** approved 2026-05-06
**Scope:** rewrite the human-facing output of every `ateam` subcommand. No
behavior changes — same exit codes, same files written, same errors raised.
Only what the user reads on stdout/stderr changes.

## Goal

Make the CLI feel modern: color-coded symbols, narrated steps, less jargon
exposed by default, and an opt-in `-v` flag for power users who want the
detail today's CLI shows unconditionally.

Style reference: pnpm/npm/Vercel CLI — narrated phases with a leading symbol
that carries the only color on the line.

## Non-goals

- No change to subcommand structure, flag surface, lockfile format, or the
  underlying install/sync behavior.
- No replacement of `tracing` / `RUST_LOG`. That stays as-is for deep
  diagnostics; `-v` is an independent layer for end-user-friendly detail.
- No interactive TTY widgets beyond a single spinner per long-running op.
- No localization, no theming, no config file for output style.

## Architecture

One new module — `src/ui.rs` — is the single seam for everything the user
reads. Every `println!` / `eprintln!` currently in `src/commands/*.rs` and in
`src/git_sync.rs` is rewritten to call a `ui::*` helper.

```rust
// src/ui.rs (interface, not the implementation)

pub fn set_verbose(v: bool);              // called once in main()
pub fn is_verbose() -> bool;

pub fn ok(msg: impl AsRef<str>);          // ✓ green, stdout
pub fn fail(msg: impl AsRef<str>);        // ✗ red, stderr
pub fn warn(msg: impl AsRef<str>);        // ⚠ yellow, stderr
pub fn detail(msg: impl AsRef<str>);      // · dim, stdout, no-op unless verbose
pub fn step(msg: impl AsRef<str>) -> Step; // → cyan + spinner; .ok()/.fail() finishes it
pub fn plain(msg: impl AsRef<str>);       // raw stdout line, no symbol (e.g. list rows)

pub struct Step { /* wraps indicatif::ProgressBar */ }
impl Step {
    pub fn ok(self, msg: impl AsRef<str>);   // replaces spinner with ✓ msg
    pub fn fail(self, msg: impl AsRef<str>); // replaces spinner with ✗ msg
    pub fn finish(self);                     // clears the spinner line entirely
}
```

`ui` wraps `console` (color + TTY detect + `NO_COLOR` env) and `indicatif`
(spinner). Both are widely-used Rust crates; `indicatif` already pulls in
`console` transitively, but we declare both as direct deps so `Cargo.toml`
documents what we use.

### main.rs and cli.rs changes

- Add `--verbose` / `-v` as a `global` flag on `Cli` next to `--no-sync`.
- In `main()`, call `ui::set_verbose(cli.verbose)` before `dispatch()`.
- After `dispatch()` returns `Err(e)`, format and print the error chain
  through `ui::fail` (using `format!("{:#}", e)` to flatten anyhow's chain),
  then `std::process::exit(1)` so the user never sees the default
  `Error: <msg>` prefix that `anyhow` produces from `main`'s `Result`.

### tracing

`tracing_subscriber` setup in `main.rs` stays in place. **Default filter
changes from `"ateam=info,warn"` to `"ateam=warn"`** — today the `info`
default means `tracing::info!("drift detected for …")` in `apply.rs` prints
unprefixed alongside our new `ui::*` lines, breaking the visual hierarchy.
Bumping the default to `warn` keeps `RUST_LOG` as the developer-debug
escape hatch without polluting normal output. `-v` is the user-debug
escape hatch; the two stay independent.

## Symbol and color contract

| Symbol | Color | Stream | Purpose | Helper |
|--------|-------|--------|---------|--------|
| `✓` | green | stdout | success / completion | `ui::ok` |
| `✗` | red | stderr | failure | `ui::fail` |
| `⚠` | yellow | stderr | warning | `ui::warn` |
| `→` | cyan | stdout | step in progress (spinner head) | `ui::step` |
| `·` | dim grey | stdout | sub-detail; only printed when verbose | `ui::detail` |

Format: `<symbol><two spaces><message>`. Message is lowercase, no trailing
period, no `ateam:` prefix (the symbol is the prefix). When color is
disabled (`NO_COLOR=1`, non-TTY, or `console` decides the terminal can't),
symbols still print — only the ANSI codes are stripped.

A spinner uses `→` while running, then `console::Term::clear_line` plus a
single `ui::ok` / `ui::fail` line. There is at most one spinner active at
a time per command.

## Per-command output

For every command below: defaults are aggressive about hiding detail; `-v`
restores roughly today's output level.

### `init`

Scaffold (default):
```
→ scaffolding repo
✓ initialized ateam
```

Clone (default):
```
→ cloning git@github.com:you/ateam-config.git
✓ initialized ateam
```

`-v` adds, after the `✓` line:
```
·  repo: ~/.config/ateam
·  profiles: personal
```
If profiles is empty (`--profiles` not passed), suppress the `· profiles:`
line — same rule as `status`.

The existing interactive `dialoguer` prompt (when neither `--scaffold` nor a
git URL is passed) is unchanged — `dialoguer::ColorfulTheme` already styles
itself. No `ui::*` calls needed there.

The current `git clone` invocation prints its own progress chatter to
stderr. Pass `--quiet` to it now that we own the user-facing narration.

### `add` (install path)

Default:
```
→ fetching vercel-labs/agent-skills      ← spinner during network fetch
✓ fetched vercel-labs/agent-skills
✓ installed deploy-to-vercel
✓ installed web-design-guidelines
```

Partial failure:
```
→ fetching vercel-labs/agent-skills
✓ fetched vercel-labs/agent-skills
✓ installed deploy-to-vercel
✗ install web-design-guidelines — not found in source
```

`-v` adds, between `→ fetching` and the install lines:
```
·  source: github:vercel-labs/agent-skills
·  cached at ~/.config/ateam/.ateam/cache/deploy-to-vercel
```
and one `·  linked <path>` line per agent symlink under each `✓ installed`.

`bail!("no skills installed (all failed)")` — left as-is; the top-level
error handler renders it as `✗ no skills installed (all failed)`.

### `add --list` (listing only)

Default (no symbols — this is a directory listing, not a status update):
```
skills in vercel-labs/agent-skills

  deploy-to-vercel       Deploy a Next.js project to Vercel
  web-design-guidelines  Brand-consistent web design
```

The skill name is in the default terminal color. The description is
rendered via `console`'s dim style. Column gap is two spaces, padded to the
longest name. When there are no skills: `(no skills found in <input>)`.

### `apply`

Default:
```
→ applying skills
✓ applied 5 skills
```

Dry-run default:
```
✓ dry run: would apply 5 skills
```

Dry-run `-v` adds one dim line per planned link:
```
·  ~/.claude/skills/deploy-to-vercel → ~/.config/ateam/.ateam/cache/deploy-to-vercel
```

Unregistered project alias (default and `-v`):
```
⚠ unregistered project: foo (used by skill1, skill2)
  run: ateam project add foo <path>
✓ applied 3 skills
```
The hint line is plain (no symbol) and indented two spaces under the
warning — matches the existing two-space hint convention in this spec.

The existing `LinkOutcome::Refused` warning ("refused to install … real dir
at … rerun with --force to move aside") is rendered as a `ui::warn`. The
silent `LinkOutcome::MovedAside` case stays silent.

### `update`

No changes available:
```
→ checking 12 skills
✓ all skills up to date
```

Updates available:
```
→ checking 12 skills
✓ updated deploy-to-vercel
✓ updated web-design-guidelines
```

`-v` adds under each `✓ updated …`:
```
·  abc1234 → def5678
```

A failed check stays a `⚠` warning on the affected skill, then continues:
```
⚠ couldn't check deploy-to-vercel: <reason>
```

### `remove`

```
✓ removed deploy-to-vercel
```
No verbose-only additions.

### `import`

Already-managed:
```
✓ deploy-to-vercel already managed by ateam
```

Newly imported / updated:
```
✓ imported deploy-to-vercel
  run: ateam apply to materialize
```

`-v` adds `·  source: local:skills/deploy-to-vercel`.

### `project add` / `project remove` / `project list`

```
✓ registered project ateam → ~/dev/ateam
✓ removed project ateam
⚠ no project ateam registered
```

List (no skills, no projects → friendly empty message):
```
(no projects registered)
```

Otherwise (plain rows, alias in default color, path dimmed via `console`):
```
ateam   ~/dev/ateam
work    ~/work/canva
```

### `list`

Empty:
```
(no skills locked)
```

Default (two columns: name, source — both in default color, two-space gap):
```
deploy-to-vercel       vercel-labs/agent-skills
web-design-guidelines  vercel-labs/agent-skills
foo                    local
bar                    canva/canva-skills @ v1.2
```

Source rendering rules:
- `github:owner/repo` → `owner/repo`
- `local:…` → `local`
- `git:<url>` → `<url>` (truncated to terminal width if needed)
- if `git_ref` is set → append ` @ <ref>`

`-v` appends a dim qualifier line under each entry, only including
fields that are non-default:
```
·  scope: project=ateam · profiles: work
```
If scope is global and profiles is empty, the dim line is suppressed.

### `status`

Healthy:
```
✓ ateam · personal
  12 skills installed
  2 projects: ateam, work
```

With dangling links:
```
⚠ ateam · personal
  12 skills installed
  2 projects: ateam, work
  ✗ 3 broken links — run: ateam apply
```

`-v` appends:
```
·  repo: ~/.config/ateam
·  manifest: 24 entries
```

The `·` after `ateam` is a literal middle-dot separator character, not the
dim sub-detail symbol — same glyph, different role. The list after it is
the active profiles joined by `, `. If profiles is empty: omit the
`· …` suffix entirely so the line reads just `✓ ateam`.

### Top-level errors

Today: anyhow prints `Error: <msg>` from `main`'s `Result`.
After: `main` catches the `Err` and routes through `ui::fail`:

```
✗ no skill named foo in lockfile
```

The error chain (`{:#}` formatting) is included on the same line, e.g.:
```
✗ refused to clone into non-empty /Users/brad/.config/ateam: …
```

### `git_sync` warnings

Every `eprintln!("ateam: warning — …")` and `eprintln!("ateam: note — …")`
in `src/git_sync.rs` is rewritten as `ui::warn(...)`. The "remote moved
during op, rebasing and retrying push…" line becomes a `ui::step` that
finishes with `ui::ok` / `ui::warn`.

## Path display

Anywhere we print a filesystem path that lives under `$HOME`, render it
with `~` instead of the absolute path. New helper in `paths.rs`:

```rust
pub fn display_path(p: &Path) -> String;  // $HOME → ~ substitution
```

Used by every `ui::*` call site that prints paths. Without `-v` most paths
are hidden anyway — this matters mostly for `-v` detail lines, the `init`
clone-URL line, and the `project list` rows.

## Spinner behavior

- TTY: `indicatif::ProgressBar::new_spinner()` with the `dots` style and a
  cyan `{spinner}` token, e.g. while running: `⠋ fetching …` cycling. The
  `→` palette symbol is *not* used during the animated state — the dots
  frames replace it.
- Non-TTY (piped to a file, redirected, `is_term() == false` per `console`):
  spinner is skipped. We print `→ fetching …` once and the `✓` / `✗`
  resolution line later. The `→` is the static fallback for the in-progress
  symbol.
- `Step::ok` and `Step::fail` clear the spinner line, then write the
  resolution line as a normal `ui::ok` / `ui::fail`.
- If a `Step` is dropped without `.ok()` / `.fail()` / `.finish()` (e.g.
  early return on error), `Drop` clears the spinner line so the terminal
  doesn't end on a half-rendered animation.

## Color and TTY detection

Delegated to `console`:
- `NO_COLOR=1` → strip all ANSI codes; symbols remain.
- Non-TTY stdout → strip ANSI codes; spinner becomes a single line.
- `CLICOLOR_FORCE=1` → keep colors even when not a TTY (for tests, scripted
  output checks).

No code in `ui.rs` reads env vars directly for color decisions. `console`
owns that.

## Testing

Two layers:

1. **Snapshot tests of pure formatting helpers.** A few unit tests in
   `src/ui.rs` that build the line that would be printed (returning a
   `String`, not actually writing) and assert on the bytes. Covers symbol
   placement, color stripping in non-TTY mode, `~` substitution.

2. **End-to-end smoke run.** Manually exercise each subcommand in a
   throwaway repo (`ateam init --scaffold`, `add` / `list` / `status` /
   `apply` / `update` / `remove`) once with default output and once with
   `-v`. Verify the visual layout matches the per-command examples above.
   This is a manual gate, not an automated test — the existing test suite
   in this repo doesn't cover stdout formatting.

## Migration

Single PR. All call sites rewritten in one pass. No feature flag, no
gradual rollout — output formatting is a global concern and split states
would be more confusing than helpful.
