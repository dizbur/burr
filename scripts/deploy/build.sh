#!/bin/bash
# Builds the program for deployment and writes its interface file.
#   ./scripts/deploy/build.sh            a reproducible build, inside Docker (needs Docker and solana-verify)
#   ./scripts/deploy/build.sh --plain    an ordinary build; cannot be verified by others afterwards
set -eu
cd "$(dirname "$0")/../.."
export PATH="$HOME/.cargo/bin:$HOME/.local/share/solana/install/active_release/bin:$PATH"
mode="${1:-verified}"

if [ -n "$(git status --porcelain 2>/dev/null)" ]; then
  echo "NOTE  there are uncommitted changes. A build that others can verify must come from a commit."
fi
if [ "$mode" = "--plain" ]; then
  echo "== ordinary build (not verifiable)"
  cargo build-sbf --manifest-path programs/burrpad/Cargo.toml
else
  command -v solana-verify >/dev/null 2>&1 || { echo "STOPPED: solana-verify is not installed (cargo install solana-verify --locked)." >&2; exit 1; }
  command -v docker >/dev/null 2>&1 || { echo "STOPPED: Docker is not installed or not running." >&2; exit 1; }
  echo "== reproducible build, in Docker"
  solana-verify build --library-name burrpad
fi
echo "== interface file"
mkdir -p target/idl target/types
anchor idl build -p burrpad -o target/idl/burrpad.json -t target/types/burrpad.ts
# A copy of exactly what was built, kept apart from the folder the tests
# rebuild into. deploy.sh, upgrade.sh and the read-back scripts use this copy,
# so later work on the code cannot change what gets deployed by accident.
mkdir -p target/release-kit
cp target/deploy/burrpad.so target/release-kit/burrpad.so
cp target/idl/burrpad.json target/release-kit/burrpad.json
git rev-parse HEAD > target/release-kit/commit.txt 2>/dev/null || echo none > target/release-kit/commit.txt
if [ "$mode" = "--plain" ]; then echo plain > target/release-kit/mode.txt; else echo reproducible > target/release-kit/mode.txt; fi
id=$(grep -o 'declare_id!("[^"]*")' programs/burrpad/src/lib.rs | cut -d'"' -f2)
echo "PASS  built, and kept as target/release-kit/burrpad.so"
echo "      program address in the code: $id"
echo "      commit: $(git rev-parse --short HEAD 2>/dev/null || echo none)"
echo "      sha256 of the file: $(shasum -a 256 target/release-kit/burrpad.so | cut -d' ' -f1)"
