#!/usr/bin/env sh
# ==============================================================================
# install-binary.sh — install a prebuilt l0-compressor binary (no Rust toolchain).
#
# Downloads the right prebuilt binary for your OS/arch from the latest GitHub
# Release, verifies its SHA-256, and installs it to ~/.local/bin (with the `t`
# alias). For a from-source build instead, use ./install.sh.
#
#   curl -fsSL https://raw.githubusercontent.com/fabriziosalmi/l0-compressor/master/install-binary.sh | sh
#
# Env overrides: L0_COMPRESSOR_BIN_DIR (install dir), L0_COMPRESSOR_VERSION (tag, e.g. v0.1.8).
# ==============================================================================
set -eu

REPO="fabriziosalmi/l0-compressor"
BIN_DIR="${L0_COMPRESSOR_BIN_DIR:-$HOME/.local/bin}"

err() { printf '%s\n' "$*" >&2; }

# 1. Map OS/arch to a release target triple.
os="$(uname -s)"
arch="$(uname -m)"
case "$os-$arch" in
  Darwin-arm64) target="aarch64-apple-darwin" ;;
  Darwin-x86_64) target="x86_64-apple-darwin" ;;
  Linux-x86_64 | Linux-amd64) target="x86_64-unknown-linux-musl" ;;
  *)
    err "No prebuilt binary for $os-$arch."
    err "Build from source instead:  git clone https://github.com/$REPO && cd l0-compressor && ./install.sh"
    exit 1
    ;;
esac

# 2. Resolve the release tag (latest unless pinned via L0_COMPRESSOR_VERSION).
if [ -n "${L0_COMPRESSOR_VERSION:-}" ]; then
  tag="$L0_COMPRESSOR_VERSION"
else
  tag="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | grep -m1 '"tag_name"' | cut -d'"' -f4)"
fi
[ -n "$tag" ] || { err "Could not resolve the latest release tag."; exit 1; }

art="l0-compressor-$target.tar.gz"
url="https://github.com/$REPO/releases/download/$tag/$art"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

# 3. Download + (best-effort) checksum verification.
printf 'Downloading l0-compressor %s (%s)…\n' "$tag" "$target"
curl -fsSL "$url" -o "$tmp/$art" || { err "Download failed: $url"; exit 1; }

if curl -fsSL "$url.sha256" -o "$tmp/sum" 2>/dev/null; then
  expected="$(cut -d' ' -f1 < "$tmp/sum")"
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$tmp/$art" | cut -d' ' -f1)"
  else
    actual="$(shasum -a 256 "$tmp/$art" | cut -d' ' -f1)"
  fi
  if [ "$expected" != "$actual" ]; then
    err "Checksum mismatch! expected $expected got $actual"
    exit 1
  fi
fi

# 4. Install + create the `l0-comp` and `t` aliases.
tar -xzf "$tmp/$art" -C "$tmp"
mkdir -p "$BIN_DIR"
install -m 0755 "$tmp/l0-compressor" "$BIN_DIR/l0-compressor"
ln -sf l0-compressor "$BIN_DIR/l0-comp"
ln -sf l0-compressor "$BIN_DIR/t"

printf 'Installed: %s  →  %s (aliases: l0-comp, t)\n' "$("$BIN_DIR/l0-compressor" --version)" "$BIN_DIR/l0-compressor"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) err "Note: $BIN_DIR is not on your PATH — add it to use l0-compressor." ;;
esac
