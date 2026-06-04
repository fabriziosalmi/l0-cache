# l0-cache — Cross-platform build targets
# Usage:
#   make build              # Build for current platform (release)
#   make build-linux        # Cross-compile for Linux x86_64 (glibc)
#   make build-alpine       # Cross-compile for Alpine/musl (fully static)
#   make build-all          # Build all targets
#   make install            # Install to /usr/local/bin
#   make test               # Run all tests
#   make lint               # Clippy + format check

.PHONY: build build-linux build-alpine build-all install test lint clean

# ── Current platform ────────────────────────────────────────────────

build:
	cargo build --release
	@echo "Built: target/release/l0-cache ($$(du -h target/release/l0-cache | cut -f1))"

test:
	cargo test

lint:
	cargo clippy -- -D warnings
	cargo fmt -- --check

# ── Cross-compilation ───────────────────────────────────────────────
# Prerequisites:
#   brew install filosottile/musl-cross/musl-cross  # for musl target
#   rustup target add x86_64-unknown-linux-gnu
#   rustup target add x86_64-unknown-linux-musl

build-linux:
	@echo "Building for Linux x86_64 (glibc, dynamic)..."
	cargo build --release --target x86_64-unknown-linux-gnu
	@echo "Built: target/x86_64-unknown-linux-gnu/release/l0-cache"

build-alpine:
	@echo "Building for Alpine/musl (fully static)..."
	CC_x86_64_unknown_linux_musl=x86_64-linux-musl-gcc \
	cargo build --release --target x86_64-unknown-linux-musl
	@echo "Built: target/x86_64-unknown-linux-musl/release/l0-cache"

build-all: build build-linux build-alpine
	@echo ""
	@echo "All targets built:"
	@ls -la target/release/l0-cache 2>/dev/null || true
	@ls -la target/x86_64-unknown-linux-gnu/release/l0-cache 2>/dev/null || true
	@ls -la target/x86_64-unknown-linux-musl/release/l0-cache 2>/dev/null || true

# ── Install ─────────────────────────────────────────────────────────

install: build
	cp target/release/l0-cache /usr/local/bin/l0-cache
	@echo "Installed to /usr/local/bin/l0-cache"
	@l0-cache --version

# ── Deploy to remote (via scp) ──────────────────────────────────────
# Usage: make deploy HOST=myserver
#   Assumes the binary is already built for the target architecture.

HOST ?= ""
REMOTE_BIN ?= /usr/local/bin/l0-cache

deploy-linux: build-linux
	@test -n "$(HOST)" || (echo "Usage: make deploy-linux HOST=user@host" && exit 1)
	scp target/x86_64-unknown-linux-gnu/release/l0-cache $(HOST):$(REMOTE_BIN)
	ssh $(HOST) "chmod +x $(REMOTE_BIN) && l0-cache --version"

deploy-alpine: build-alpine
	@test -n "$(HOST)" || (echo "Usage: make deploy-alpine HOST=user@host" && exit 1)
	scp target/x86_64-unknown-linux-musl/release/l0-cache $(HOST):$(REMOTE_BIN)
	ssh $(HOST) "chmod +x $(REMOTE_BIN) && l0-cache --version"

# ── Clean ───────────────────────────────────────────────────────────

clean:
	cargo clean
