#!/bin/sh
# Everything that must pass before a commit: format, lint, advisories,
# the program build, and the whole test suite.
set -eu
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$HOME/.local/share/solana/install/active_release/bin:$PATH"

free_gb=$(df -g . | tail -1 | awk '{print $4}')
if [ "$free_gb" -lt 8 ]; then
  echo "Less than 8 GB free ($free_gb GB). Clean up before building." >&2
  exit 1
fi

echo "== format"
cargo fmt --all -- --check
echo "== lint (warnings are errors)"
cargo clippy --workspace --all-targets -- -D warnings
echo "== advisories"
cargo audit
echo "== build the program, and the misbehaving stand-in the tests use"
cargo build-sbf --manifest-path programs/burr/Cargo.toml
cargo build-sbf --manifest-path tests-standin/Cargo.toml
echo "== tests"
cargo test --workspace
echo "== all passed"
