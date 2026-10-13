#!/bin/bash
# Puts the built program on a network. The payer becomes the upgrade key.
#   ./scripts/deploy/deploy.sh --program-key <file> --payer <file> [--mainnet yes] [--url <network>]
# Both files are key files you made yourself. They are passed straight to the
# `solana` tool and are never shown.
set -euo pipefail
cd "$(dirname "$0")/../.."
. scripts/deploy/net.sh
url="${BURRPAD_RPC_URL:-}"; program_key=""; payer=""; mainnet=""
while [ $# -gt 0 ]; do
  case "$1" in
    --url) url="$2";; --program-key) program_key="$2";; --payer) payer="$2";; --mainnet) mainnet="$2";;
    *) echo "Unknown option $1" >&2; exit 1;;
  esac
  shift 2
done
[ -n "$program_key" ] && [ -n "$payer" ] || { echo "Missing --program-key or --payer." >&2; exit 1; }
[ -f "$program_key" ] && [ -f "$payer" ] || { echo "STOPPED: a key file was not found at the path given." >&2; exit 1; }
[ -f target/deploy/burrpad.so ] || { echo "STOPPED: not built yet. Run scripts/deploy/build.sh first." >&2; exit 1; }
net_check "$url" "$mainnet"

id=$(solana-keygen pubkey "$program_key")
[ "$id" = "$(code_id)" ] || { echo "STOPPED: the program key's address ($id) is not the one in the code ($(code_id)). Run set-program-id.sh and build again." >&2; exit 1; }
grep -q "\"address\": \"$id\"" target/idl/burrpad.json || { echo "STOPPED: the build is older than the address in the code. Run build.sh again." >&2; exit 1; }
if node scripts/deploy/program-info.mjs "$id" --url "$url" >/dev/null 2>&1; then
  echo "STOPPED: a program is already at $id. To replace its code use scripts/deploy/upgrade.sh." >&2; exit 1
fi
balance_check "$url" "$payer"

echo "Deploying $id ..."
solana program deploy target/deploy/burrpad.so --url "$url" --keypair "$payer" --program-id "$program_key" \
  --use-rpc --max-sign-attempts 30 --with-compute-unit-price "${BURRPAD_PRIORITY_FEE:-50000}"
echo "PASS  deployed: $id"
echo "Next, in the same sitting: node scripts/deploy/initialize.mjs"
