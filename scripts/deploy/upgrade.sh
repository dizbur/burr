#!/bin/bash
# Replaces the deployed program's code with the one built from this folder,
# then reads it back.
#
# While an ordinary wallet holds the upgrade key:
#   ./scripts/deploy/upgrade.sh --upgrade-key <file> [--plain] [--mainnet yes]
#
# Once a multi-signature wallet holds it, the new code is uploaded and offered to that wallet,
# which then approves the upgrade in its own app:
#   ./scripts/deploy/upgrade.sh --payer <file> --offer-to <the wallet's vault address> [--plain] [--mainnet yes]
#
# --plain builds without Docker (nobody can verify that build afterwards).
# The wallet needs the program's rent again as a float (about 4.7 SOL); it comes back when the upgrade lands.
set -euo pipefail
cd "$(dirname "$0")/../.."
. scripts/deploy/net.sh
url="${BURRPAD_RPC_URL:-}"; key=""; payer=""; offer=""; mainnet=""; build="verified"
while [ $# -gt 0 ]; do
  case "$1" in
    --plain) build="--plain"; shift; continue;;
    --url) url="$2";; --upgrade-key) key="$2";; --payer) payer="$2";; --offer-to) offer="$2";; --mainnet) mainnet="$2";;
    *) echo "Unknown option $1" >&2; exit 1;;
  esac
  shift 2
done
signer="${key:-$payer}"
[ -n "$signer" ] && [ -f "$signer" ] || { echo "STOPPED: give --upgrade-key <file>, or --payer <file> with --offer-to <address>." >&2; exit 1; }
[ -z "$key" ] || [ -z "$offer" ] || { echo "STOPPED: use either --upgrade-key, or --payer with --offer-to." >&2; exit 1; }
[ -n "$key" ] || [ -n "$offer" ] || { echo "STOPPED: --payer needs --offer-to <the wallet's vault address>." >&2; exit 1; }
net_check "$url" "$mainnet"

id=$(code_id)
shown=$(node scripts/deploy/program-info.mjs "$id" --url "$url" 2>/dev/null) || { echo "STOPPED: no program at $id on this network. Use deploy.sh for a first deploy." >&2; exit 1; }
authority=${shown% *}
room=${shown#* }
echo "Program $id, upgrade key $authority"
want="${offer:-$(solana-keygen pubkey "$key")}"
[ "$authority" = "$want" ] || { echo "STOPPED: the upgrade key on the network is $authority, not $want." >&2; exit 1; }

if [ "$build" = "--plain" ]; then ./scripts/deploy/build.sh --plain; else ./scripts/deploy/build.sh; fi
balance_check "$url" "$signer"
size=$(wc -c < target/deploy/burrpad.so | tr -d ' ')
if [ "$size" -gt "$room" ]; then
  if [ -n "$offer" ]; then
    echo "STOPPED: the new code is $((size - room)) bytes larger than the room the program has. The multi-signature wallet must extend the program first." >&2; exit 1
  fi
  echo "The new code is larger: making room for $((size - room)) more bytes."
  solana program extend "$id" "$((size - room))" --url "$url" --keypair "$key"
fi

if [ -n "$key" ]; then
  solana program deploy target/deploy/burrpad.so --url "$url" --keypair "$key" --program-id "$id" --upgrade-authority "$key" \
    --use-rpc --max-sign-attempts 30 --with-compute-unit-price "${BURRPAD_PRIORITY_FEE:-50000}"
  sleep 2
  node scripts/deploy/verify-code.mjs --url "$url" ${mainnet:+--mainnet "$mainnet"}
else
  out=$(solana program write-buffer target/deploy/burrpad.so --url "$url" --keypair "$payer" \
    --use-rpc --max-sign-attempts 30 --with-compute-unit-price "${BURRPAD_PRIORITY_FEE:-50000}")
  buffer=$(echo "$out" | awk '/^Buffer:/ {print $2}')
  [ -n "$buffer" ] || { echo "STOPPED: the upload did not finish."; exit 1; }
  solana program set-buffer-authority "$buffer" --new-buffer-authority "$offer" --url "$url" --keypair "$payer" >/dev/null
  echo "PASS  new code uploaded and offered to $offer"
  echo "      buffer address: $buffer"
  echo "      sha256 of the file: $(shasum -a 256 target/deploy/burrpad.so | cut -d' ' -f1)"
  echo "Next: in the multi-signature wallet's app, upgrade program $id from that buffer and approve it."
  echo "Then: node scripts/deploy/verify-code.mjs"
fi
