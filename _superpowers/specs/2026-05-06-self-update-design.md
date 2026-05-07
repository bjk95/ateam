# Self-update for the `agents` CLI

**Date:** 2026-05-06
**Status:** Approved, awaiting implementation plan

## Goal

Add a Claude-Code-style auto-updater to `agents` so users on machines that already have the binary get new releases without re-running the installer script by hand.

Two surfaces:

1. **Implicit:** every `agents <cmd>` invocation silently checks for updates at most once per 24 hours and installs them in place.
2. **Explicit:** a new `agents upgrade` subcommand bypasses the TTL and runs the same check/install path with output.

Failures of any kind never block the underlying command. The implicit check is purely additive — if the network is down, GitHub is rate-limited, or the cache file can't be written, the user notices nothing and the command they actually ran proceeds as today.

## High-level flow

```
agents <any-cmd>
   │
   ├─► self_update::maybe_check()      [first call in main, before git_sync]
   │     ├─ read mtime of ~/.cache/agents/.update-check
   │     ├─ if (now - mtime) < 24h    → return [warm cache, ~1 syscall]
   │     └─ else (cold cache):
   │          ├─ axoupdater.run_sync()
   │          │    ├─ Ok(Some(result))  → log "agents: 0.1.0 → 0.2.0", touch cache
   │          │    ├─ Ok(None)          → up-to-date, touch cache
   │          │    └─ Err(_)            → swallow, leave cache untouched
   │          └─ outer closure swallows ALL errors
   │
   └─► dispatch(cli) [unchanged]
```

`agents upgrade` skips the TTL check and runs the same `axoupdater.run_sync()` path with output enabled. It returns a real `Result` so a manual upgrade exits non-zero on failure — the user explicitly asked us to update, they want to know if it broke.

### Key invariants

1. The check runs *before* `git_sync` and `dispatch`. A successful update means the *next* invocation uses the new binary, not the current one — the running process keeps its old text segment but the file on disk is replaced.
2. Soft-fail is the only failure mode in `maybe_check`. The check has no path to make `agents <cmd>` fail.
3. The cache file is touched *only* on a successful network round-trip. Offline machines therefore retry every invocation (cheap — fails fast on the first DNS/connect error).

## Components

Three units, each with one job.

### 1. `src/self_update.rs` — new module

**Public surface:**

```rust
pub fn maybe_check();                  // called from main, TTL-gated, soft-fails
pub fn force_upgrade() -> Result<()>;  // called from `agents upgrade`, prints output
```

**Internals (private):**

- `cache_path() -> Result<PathBuf>` → `~/.cache/agents/.update-check`. Resolved via `directories::BaseDirs::cache_dir()` (XDG cache, transient by convention) so it is **outside** the git-managed `~/.config/agents/` repo and never shows up in `git status` or auto-sync commits.
- `ensure_cache_dir(path: &Path) -> Result<()>` → `create_dir_all` on the parent. `~/.cache/agents/` may not exist yet on a fresh machine; we own creating it.
- `is_cache_fresh(path: &Path) -> bool` → mtime within 24h. Missing file or unreadable mtime → not fresh (treat as cold).
- `touch_cache(path: &Path) -> Result<()>` → `fs::write` of current unix timestamp as text (~10 bytes). The *content* is not actually read — `is_cache_fresh` uses mtime — but writing the timestamp makes the file self-describing on disk for debugging.
- `build_updater() -> AxoUpdater` → configures name/owner/source/current_version

**Dependencies:** `axoupdater`, `std::fs`, `std::time`. No new transitive crates beyond what `axoupdater` itself pulls.

### 2. `src/cli.rs` — one new variant

```rust
Command::Upgrade,    // no args, no flags
```

Plus one new dispatch arm: `Command::Upgrade => self_update::force_upgrade()`.

### 3. `src/main.rs` — three new lines

Between `Cli::parse()` and `dispatch(cli)`, add a guarded call:

```rust
if !matches!(cli.command, cli::Command::Upgrade) {
    self_update::maybe_check();
}
```

The guard prevents `agents upgrade` from running the silent TTL-gated check before the explicit upgrade — they'd both call `axoupdater.run_sync()` and the silent one would race the loud one for the cache file.

### Cargo.toml change

```toml
axoupdater = { version = "0.10", default-features = false, features = ["blocking"] }
```

The `blocking` feature gives us `run_sync()` / `is_update_needed_sync()` so we don't pull tokio into a CLI that doesn't have it. The implementation pass should verify the exact feature flags `axoupdater 0.10` exposes for GitHub Releases — they may already be in the default set, or they may need an additional flag (e.g. a `github_releases` feature). Either way, the goal is "blocking + GitHub source enabled, tokio not pulled in transitively".

## Data flow

### `build_updater()` body

```rust
fn build_updater() -> AxoUpdater {
    let mut u = AxoUpdater::new_for("agents");
    u.set_release_source(ReleaseSource {
        release_type: ReleaseSourceType::GitHub,
        owner: "bjk95".into(),
        name: "agents".into(),
        app_name: "agents".into(),
    });
    u.set_current_version(env!("CARGO_PKG_VERSION").parse().unwrap()).ok();
    u
}
```

The `.ok()` on `set_current_version` is intentional — if our own version is somehow unparseable as semver, that's a build-time bug. The whole subsystem soft-fails, so we don't panic at runtime either.

### `maybe_check()` body — the silent path

```rust
pub fn maybe_check() {
    let _ = (|| -> Result<()> {
        let cache = cache_path()?;
        if is_cache_fresh(&cache) { return Ok(()); }   // 99% of calls exit here

        let mut u = build_updater();
        u.disable_installer_output();
        match u.run_sync() {
            Ok(Some(result)) => {
                // Field names below are illustrative; verify against
                // axoupdater 0.10's UpdateResult during implementation.
                eprintln!("agents: updated {} → {}", result.old_version, result.new_version);
                touch_cache(&cache)?;
            }
            Ok(None) => { touch_cache(&cache)?; }
            Err(_)   => { /* leave cache untouched, retry next call */ }
        }
        Ok(())
    })();
    // outer closure swallows EVERYTHING.
}
```

### `force_upgrade()` body — the loud path

```rust
pub fn force_upgrade() -> Result<()> {
    let mut u = build_updater();
    u.enable_installer_output();
    match u.run_sync()? {
        Some(result) => println!("agents: updated {} → {}", result.old_version, result.new_version),
        None         => println!("agents: already at latest ({})", env!("CARGO_PKG_VERSION")),
    }
    let _ = touch_cache(&cache_path()?);
    Ok(())
}
```

## Error handling

| Failure | `maybe_check` | `force_upgrade` |
|---|---|---|
| Cache file missing / unreadable | Treat as cold → run check | Skip cache read, still upgrade |
| Cache file write fails | Swallow, retry next invocation | Swallow, print warning |
| `~/.cache/agents` missing | `ensure_cache_dir` creates it; if `create_dir_all` itself fails, swallow | Same |
| Pointer / repo not yet initialized (`agents init` never run) | Irrelevant — cache lives outside the repo, no dependency on `paths::resolve_repo()` | Same |
| Network error (offline, DNS down) | Swallow, cache untouched | Bubble up, exit 1 |
| GitHub rate limit (HTTP 403) | Swallow, cache untouched | Bubble up, exit 1 |
| Latest release missing `agents-installer.sh` | Swallow, cache untouched | Bubble up, exit 1 |
| Permission denied replacing the binary | Swallow | Bubble up, exit 1 |
| Pre-release (e.g. `0.2.0-rc.1`) is latest | axoupdater semver-compares; pre-releases are *less than* the release tag, so stable users stay on stable. | Same |
| Our own `CARGO_PKG_VERSION` unparseable | `.ok()` swallows | Same |

### Ordering subtlety

`maybe_check` runs *before* `git_sync.pull`. The reasoning: putting it after `git_sync` means a busted install could prevent a `git pull`. Putting it first means the worst case is "update succeeded but the user's command crashed" — and the next `agents` call gets the new binary, which is the right resolution.

## What this design explicitly does NOT include

- **Env-var opt-out.** No `AGENTS_NO_UPDATE_CHECK`. Soft-fail covers the offline/corporate/locked-down case. The user does not want to set environment variables for this.
- **Notify-only mode.** Auto-install is the whole point; "an update is available" without action is just noise.
- **Retry / backoff.** Soft-fail = swallow + try again on next cache miss.
- **Version pinning, rollback, downgrade.** axoupdater handles atomic replace; broken releases are a release-discipline problem, not an updater-design problem.
- **Config file for cadence or behavior.** 24h is hard-coded.
- **Pre-release opt-in channel.** Stable channel only.
- **Background / async check.** Synchronous and blocking; the cache miss happens at most once per 24 hours per machine, and the visible cost is the ~500ms install when an update is found.

## Testing

### Unit tests in `self_update.rs`

Two tests for the only pure logic worth isolating:

1. **`is_cache_fresh` boundary.** Tempfile with mtime `now - 23h59m` → fresh; `now - 24h01m` → stale; missing → stale.
2. **`build_updater` configuration.** Sanity check that name/owner/app_name/source-type are what we expect. No network.

The rest of `maybe_check` is the swallow-all wrapper — testing it would just verify Rust's `Result` semantics.

### Not tested

- `axoupdater`'s install logic — that's their crate's job.
- The binary swap itself — tests can't safely replace the test runner's binary.
- `force_upgrade` — 6 lines that all delegate to `axoupdater`.

### Manual smoke tests (PR description, not CI)

1. `cargo run -- upgrade` against a real release where `CARGO_PKG_VERSION` is older than the latest tag. Expect: prints `updated X → Y`, binary at `~/.local/bin/agents` is replaced.
2. `cargo run -- list` with wifi off. Expect: command succeeds, no error printed, no hang.

### CI

No new CI. cargo-dist's existing release pipeline already publishes `agents-installer.sh` per release; that's the artifact `axoupdater` consumes. If the installer asset ever stops being published, the failure surfaces on the first user `upgrade` — not something a unit test would catch better.

## Documentation impact

- `README.md` — add a one-paragraph "Updating" section near "Install", documenting the auto-check behavior and the explicit `agents upgrade` command.
- `docs/install.md` — same addition.
- `docs/reference/cli.md` — add `upgrade` to the subcommand table.
- `WISHLIST.md` — no entries to remove (auto-update was not previously listed).

## Out of scope (for follow-ups, not this design)

- Telemetry on update success/failure rates.
- Multi-channel (stable / beta / nightly).
- Update notifications via system notification frameworks.
- `agents doctor` — already an explicit non-goal in `WISHLIST.md`.
