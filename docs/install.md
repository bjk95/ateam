---
title: Installation
description: Install the ateam CLI.
---

`ateam` is a single static binary. Prebuilt releases cover macOS (Apple Silicon
and Intel) and Linux (x86_64 and aarch64, musl-static).

## One-line install

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/bjk95/ateam/releases/latest/download/ateam-installer.sh | sh
```

The installer detects your OS/arch, downloads the matching binary, and drops
it at `~/.local/bin/ateam` (override with `ATEAM_INSTALL_DIR`). It also
appends `~/.local/bin` to your shell `PATH` if it isn't already on it.

The Linux binaries are statically linked against musl libc, so they run on
any glibc *or* musl distro (Ubuntu, Debian, Alpine, etc.) with no extra
runtime dependencies.

## Build from source

If you'd rather build locally — for example on a platform without a prebuilt
release, or to hack on `ateam` itself:

```bash
git clone https://github.com/bjk95/ateam ~/dev/ateam
cd ~/dev/ateam
cargo build --release
cp target/release/ateam /usr/local/bin/
```

`cargo build --release` requires Rust 1.90 or newer. If `cargo` is missing,
install it via [rustup](https://rustup.rs).

## Verify

```bash
ateam --version
```

If that prints a version, you're done. Move on to the [quickstart](/quickstart/).
