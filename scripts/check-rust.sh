#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

export PATH="$HOME/.cargo/bin:$PATH"

cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
