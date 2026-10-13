#!/bin/bash
# Puts the release build on a network. The payer becomes the upgrade key.
#   ./scripts/deploy/deploy.sh --program-key <file> --payer <file> [--mainnet yes] [--url <network>]
# Both files are key files you made yourself. They are passed straight to the
# `solana` tool and are never shown.
set -euo pipefail
START_DIR="$PWD"
cd "$(dirname "$0")/../.."
. scripts/deploy/net.sh
url=""; program_key=""; payer=""; mainnet=""
while [ $# -gt 0 ]; do
  [ $# -ge 2 ] || { echo "STOPPED: $1 needs a value." >&2; exit 1; }
  case "$1" in
    --url) url="$2";; --program-key) program_key="$2";; --payer) payer="$2";; --mainnet) mainnet="$2";;
    *) echo "Unknown option $1" >&2; exit 1;;
  esac
  shift 2
done
program_key=$(full_path "$program_key"); payer=$(full_path "$payer")
[ -n "$program_key" ] && [ -n "$payer" ] || { echo "Missing --program-key or --payer." >&2; exit 1; }
[ -f "$program_key" ] && [ -f "$payer" ] || { echo "STOPPED: a key file was not found at the path given." >&2; exit 1; }
net_check "$url" "$mainnet"
built_check

id=$(solana-keygen pubkey "$program_key")
[ "$id" = "$(code_id)" ] || { echo "STOPPED: the program key's address ($id) is not the one in the code ($(code_id)). Run set-program-id.sh and build again." >&2; exit 1; }
grep -q "\"address\": \"$id\"" target/release-kit/burrpad.json || { echo "STOPPED: the release build is for a different program address. Run build.sh again." >&2; exit 1; }
program_state "$id"
if [ "$state" = present ]; then
  echo "STOPPED: a program is already at $id. If you have just deployed, go on to initialize.mjs. To replace its code use scripts/deploy/upgrade.sh." >&2; exit 1
fi
balance_check "$payer" deploy
holding_key

echo "Deploying $id ..."
sol program deploy "$BUILT" --keypair "$payer" --program-id "$program_key" --buffer "$HOLDING" \
  --use-rpc --max-sign-attempts 30 --with-compute-unit-price "$PRIORITY" || upload_stopped
holding_done
sleep 2
node scripts/deploy/verify-code.mjs ${mainnet:+--mainnet "$mainnet"}
echo "PASS  deployed: $id"
echo "Next, in the same sitting: node scripts/deploy/initialize.mjs"
