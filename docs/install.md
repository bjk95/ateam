---
title: Installation
description: Install the agents CLI.
---

`agents` is a single static binary. Prebuilt releases cover macOS (Apple Silicon
and Intel) and Linux (x86_64 and aarch64, musl-static).

## One-line install

```bash
curl -fsSL https://github.com/bjk95/agents/releases/latest/download/agents-installer.sh | sh
```

The installer detects your OS/arch, downloads the matching binary, and drops
it at `~/.local/bin/agents` (override with `AGENTS_INSTALL_DIR`). It also
appends `~/.local/bin` to your shell `PATH` if it isn't already on it.

The Linux binaries are statically linked against musl libc, so they run on
any glibc *or* musl distro (Ubuntu, Debian, Alpine, etc.) with no extra
runtime dependencies.

## Build from source

If you'd rather build locally — for example on a platform without a prebuilt
release, or to hack on `agents` itself:

```bash
git clone https://github.com/bjk95/agents ~/dev/agents
cd ~/dev/agents
cargo build --release
cp target/release/agents /usr/local/bin/
```

`cargo build --release` requires Rust 1.90 or newer. If `cargo` is missing,
install it via [rustup](https://rustup.rs).

## Updating

`agents` checks for new releases at most once every 24 hours and installs
them in place when found. The check is silent on success, soft-fails on any
network or filesystem error, and never blocks your command. To trigger an
update explicitly:

```bash
agents upgrade
```

The update path uses the same installer asset as the initial install, so
the binary at `~/.local/bin/agents` (or `$AGENTS_INSTALL_DIR`) is replaced
atomically.

## Verify

```bash
agents --version
```

If that prints a version, you're done. Move on to the [quickstart](/quickstart/).
