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
# --built builds nothing: it uses the release build already made by build.sh.
# The wallet needs the program's rent again as a float (about 4.7 SOL); it comes back when the upgrade lands.
set -euo pipefail
cd "$(dirname "$0")/../.."
. scripts/deploy/net.sh
url="${BURRPAD_RPC_URL:-}"; key=""; payer=""; offer=""; mainnet=""; build="verified"
while [ $# -gt 0 ]; do
  case "$1" in
    --plain) build="--plain"; shift; continue;;
    --built) build="none"; shift; continue;;
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

case "$build" in
  none) [ -f target/release-kit/burrpad.so ] || { echo "STOPPED: no release build here. Run scripts/deploy/build.sh first." >&2; exit 1; }
        echo "Using the release build already here: commit $(cut -c1-7 target/release-kit/commit.txt), sha256 $(shasum -a 256 target/release-kit/burrpad.so | cut -c1-16)…";;
  --plain) ./scripts/deploy/build.sh --plain;;
  *) ./scripts/deploy/build.sh;;
esac
grep -q "\"address\": \"$id\"" target/release-kit/burrpad.json || { echo "STOPPED: the release build is for a different program address. Run build.sh again." >&2; exit 1; }
balance_check "$url" "$signer" upgrade
size=$(wc -c < target/release-kit/burrpad.so | tr -d ' ')
if [ "$size" -gt "$room" ]; then
  if [ -n "$offer" ]; then
    echo "STOPPED: the new code is $((size - room)) bytes larger than the room the program has. The multi-signature wallet must extend the program first." >&2; exit 1
  fi
  echo "The new code is larger: making room for $((size - room)) more bytes."
  solana program extend "$id" "$((size - room))" --url "$url" --keypair "$key"
fi

holding_key
if [ -n "$key" ]; then
  solana program deploy target/release-kit/burrpad.so --url "$url" --keypair "$key" --program-id "$id" --upgrade-authority "$key" --buffer "$HOLDING" \
    --use-rpc --max-sign-attempts 30 --with-compute-unit-price "${BURRPAD_PRIORITY_FEE:-50000}"
  rm -f "$HOLDING"
  sleep 2
  node scripts/deploy/verify-code.mjs --url "$url" ${mainnet:+--mainnet "$mainnet"}
else
  out=$(solana program write-buffer target/release-kit/burrpad.so --url "$url" --keypair "$payer" --buffer "$HOLDING" \
    --use-rpc --max-sign-attempts 30 --with-compute-unit-price "${BURRPAD_PRIORITY_FEE:-50000}")
  buffer=$(echo "$out" | awk '/^Buffer:/ {print $2}')
  [ -n "$buffer" ] || { echo "STOPPED: the upload did not finish."; exit 1; }
  solana program set-buffer-authority "$buffer" --new-buffer-authority "$offer" --url "$url" --keypair "$payer" >/dev/null
  rm -f "$HOLDING"
  echo "PASS  new code uploaded and offered to $offer"
  echo "      buffer address: $buffer"
  echo "      sha256 of the file: $(shasum -a 256 target/release-kit/burrpad.so | cut -d' ' -f1)"
  echo "Next: in the multi-signature wallet's app, upgrade program $id from that buffer and approve it."
  echo "Then: node scripts/deploy/verify-code.mjs"
fi
