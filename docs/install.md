---
title: Installation
description: Build and install the ateam CLI.
---

`ateam` is a single Rust binary. On macOS today; Linux is coming.

## Build from source

```bash
git clone https://github.com/bradleykester/ateam ~/dev/ateam
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
