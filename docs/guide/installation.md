# Installation

## Prebuilt binary (no Rust needed)

Download a prebuilt binary for your platform (macOS arm64/x64, Linux x64) from the
latest GitHub Release and install it to `~/.local/bin/` (with the `t` alias). The
script verifies the SHA-256 checksum:

```sh
curl -fsSL https://raw.githubusercontent.com/fabriziosalmi/l0-cache/master/install-binary.sh | sh
```

Override the install dir with `L0_CACHE_BIN_DIR`, or pin a version with
`L0_CACHE_VERSION=v0.1.8`.

## Homebrew (macOS / Linux)

```sh
brew tap fabriziosalmi/l0-cache https://github.com/fabriziosalmi/l0-cache
brew install l0-cache
```

## From source (non-interactive)

Builds with Rust/Cargo and installs to `~/.local/bin/` (requires Git and a Rust
toolchain):

```sh
curl -fsSL https://raw.githubusercontent.com/fabriziosalmi/l0-cache/master/install.sh | bash
```

## Interactive Installer

Clone the repository and run the setup script for a guided interactive install:

```sh
git clone https://github.com/fabriziosalmi/l0-cache.git
cd l0-cache
./install.sh
```

## Manual Build

Requirements: Rust 1.70+ (for edition 2021 features).

```sh
git clone https://github.com/fabriziosalmi/l0-cache.git
cd l0-cache
cargo build --release
sudo cp target/release/l0-cache /usr/local/bin/
```

## Verify

```sh
l0-cache --version
# l0-cache 0.1.0 (abc1234)
```

## Shell Completions

Generate and install completions for your shell:

```sh
# Bash
l0-cache --completions bash > /etc/bash_completion.d/l0-cache

# Zsh
mkdir -p ~/.zsh/completions
l0-cache --completions zsh > ~/.zsh/completions/_l0-cache

# Fish
l0-cache --completions fish > ~/.config/fish/completions/l0-cache.fish
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

The deploy target copies the binary via `scp` and verifies with `l0-cache --version`.
