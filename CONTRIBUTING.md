# Contributing to l0-cache

Thank you for your interest in contributing to `l0-cache`! We appreciate your help in making this project better.

Please follow these guidelines to set up your environment, run tests, and submit contributions.

## Development Environment Setup

### Prerequisites

- **Rust**: Version 1.70 or newer is required.
- **Node.js & npm**: Version 20 or newer (only needed for editing and building the documentation).

### Cloning the Repository

```bash
git clone https://github.com/fabriziosalmi/l0-cache.git
cd l0-cache
```

### Building the Project

Build the release binary locally:

```bash
make build
```

## Running Tests

We maintain a rigorous test suite consisting of unit tests, fuzzing tests, and E2E integration tests.

### Run All Tests

To run the entire test suite:

```bash
make test
```

Or directly via Cargo:

```bash
cargo test
```

## Code Quality and Formatting

We enforce strict formatting and clippy rules to ensure the codebase remains clean and maintainable.

### Linting and Formatting Checks

Before submitting a PR, make sure to run:

```bash
make lint
```

This runs both:
- `cargo clippy -- -D warnings` (ensures zero compiler warnings)
- `cargo fmt -- --check` (ensures consistent code styling)

## Editing Documentation

The documentation is built with VitePress.

### Setup and Build Docs

To install dependencies and build documentation locally:

```bash
npm install
npm run docs:build
```

## Pull Request Guidelines

1. **Keep commits squashed/clean**: Keep the commit history descriptive.
2. **Descriptive Commit Messages**: Do not use "WIP" or "fix". Write a clear summary of what changed.
3. **Verify locally**: Make sure `make lint` and `make test` pass successfully before opening a PR.
