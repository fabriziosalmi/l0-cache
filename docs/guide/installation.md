# Installation

## Prebuilt binary (no Rust needed)

Download a prebuilt binary for your platform (macOS arm64/x64, Linux x64) from the
latest GitHub Release and install it to `~/.local/bin/` (with the `l0-comp` and
`t` aliases). The script verifies the SHA-256 checksum:

```sh
curl -fsSL https://raw.githubusercontent.com/fabriziosalmi/l0-compressor/master/install-binary.sh | sh
```

Override the install dir with `L0_COMPRESSOR_BIN_DIR`, or pin a version with
`L0_COMPRESSOR_VERSION=v0.2.0`.

::: info Upgrading from l0-cache
The project was renamed from `l0-cache` in 0.2.0. On first run the new binary
performs a one-time, non-destructive migration of the data and config
directories (`…/l0-cache/` → `…/l0-compressor/`), so accumulated metrics,
adaptive tunes, and config files carry over. The `L0_CACHE_GUARD` and
`L0_CACHE_NO_TELEMETRY` environment variables keep working as deprecated
fallbacks until the next major version.
:::

## Homebrew (macOS / Linux)

```sh
brew tap fabriziosalmi/l0-compressor https://github.com/fabriziosalmi/l0-compressor
brew install l0-compressor
```

This installs the `l0-compressor` binary (and the `t` alias) plus the integration
helpers — `l0-compressor-claude-hook`, `l0-compressor-agent-hook`, `l0-compressor-agent-rules` —
so the transparent Claude Code / Gemini CLI hook works without cloning the repo:

```sh
l0-compressor-claude-hook install && l0-compressor-claude-hook enable
```

## From source (non-interactive)

Builds with Rust/Cargo and installs to `~/.local/bin/` (requires Git and a Rust
toolchain):

```sh
curl -fsSL https://raw.githubusercontent.com/fabriziosalmi/l0-compressor/master/install.sh | bash
```

## Interactive Installer

Clone the repository and run the setup script for a guided interactive install:

```sh
git clone https://github.com/fabriziosalmi/l0-compressor.git
cd l0-compressor
./install.sh
```

## Manual Build

Requirements: Rust 1.85+ (see `rust-version` in `Cargo.toml`).

```sh
git clone https://github.com/fabriziosalmi/l0-compressor.git
cd l0-compressor
cargo build --release
sudo cp target/release/l0-compressor /usr/local/bin/
```

## Verify

```sh
l0-compressor --version
# l0-compressor 0.2.0 (abc1234)
```

## Shell Completions

Generate and install completions for your shell:

```sh
# Bash
l0-compressor --completions bash > /etc/bash_completion.d/l0-compressor

# Zsh
mkdir -p ~/.zsh/completions
l0-compressor --completions zsh > ~/.zsh/completions/_l0-compressor

# Fish
l0-compressor --completions fish > ~/.config/fish/completions/l0-compressor.fish
```

## Cross-Platform Builds

Build on macOS for deployment to Linux servers:

```sh
# Prerequisites (macOS)
brew install filosottile/musl-cross/musl-cross
rustup target add x86_64-unknown-linux-gnu
rustup target add x86_64-unknown-linux-musl

# Ubuntu / Debian (glibc, dynamic linking)
make build-linux

# Alpine / container (musl, fully static)
make build-alpine
```

## Deploy to Remote Server

```sh
# Ubuntu server
make deploy-linux HOST=user@myserver

# Alpine or LXC container
make deploy-alpine HOST=user@container
```

The deploy target copies the binary via `scp` and verifies with `l0-compressor --version`.
