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
# The wallet needs the program's rent again as a float. With --upgrade-key it comes back to the same
# wallet when the upgrade lands; with --offer-to it goes wherever the multi-signature wallet's upgrade sends it.
set -euo pipefail
cd "$(dirname "$0")/../.."
. scripts/deploy/net.sh
url=""; key=""; payer=""; offer=""; mainnet=""; build="verified"
while [ $# -gt 0 ]; do
  case "$1" in
    --plain) build="--plain"; shift; continue;;
    --built) build="none"; shift; continue;;
  esac
  [ $# -ge 2 ] || { echo "STOPPED: $1 needs a value." >&2; exit 1; }
  case "$1" in
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
program_state "$id"
[ "$state" = present ] || { echo "STOPPED: no program at $id on this network. Use deploy.sh for a first deploy." >&2; exit 1; }
authority=${shown% *}
room=${shown#* }
echo "Program $id, upgrade key $authority"
want="${offer:-$(solana-keygen pubkey "$key")}"
[ "$authority" = "$want" ] || { echo "STOPPED: the upgrade key on the network is $authority, not $want." >&2; exit 1; }

case "$build" in
  none) ;;
  --plain) ./scripts/deploy/build.sh --plain;;
  *) ./scripts/deploy/build.sh;;
esac
built_check
grep -q "\"address\": \"$id\"" target/release-kit/burrpad.json || { echo "STOPPED: the release build is for a different program address. Run build.sh again." >&2; exit 1; }
balance_check "$signer" upgrade
size=$(wc -c < "$BUILT" | tr -d ' ')
if [ "$size" -gt "$room" ]; then
  if [ -n "$offer" ]; then
    echo "STOPPED: the new code is $((size - room)) bytes larger than the room the program has. The multi-signature wallet must extend the program first." >&2; exit 1
  fi
  echo "The new code is larger: making room for $((size - room)) more bytes."
  sol program extend "$id" "$((size - room))" --keypair "$key"
fi

holding_key
if [ -n "$key" ]; then
  sol program deploy "$BUILT" --keypair "$key" --program-id "$id" --upgrade-authority "$key" --buffer "$HOLDING" \
    --use-rpc --max-sign-attempts 30 --with-compute-unit-price "$PRIORITY" || upload_stopped
  rm -f "$HOLDING"
  sleep 2
  node scripts/deploy/verify-code.mjs ${mainnet:+--mainnet "$mainnet"}
else
  sol program write-buffer "$BUILT" --keypair "$payer" --buffer "$HOLDING" \
    --use-rpc --max-sign-attempts 30 --with-compute-unit-price "$PRIORITY" || upload_stopped
  buffer=$(solana-keygen pubkey "$HOLDING")
  sol program set-buffer-authority "$buffer" --new-buffer-authority "$offer" --keypair "$payer" >/dev/null
  rm -f "$HOLDING"
  sleep 2
  node scripts/deploy/verify-buffer.mjs "$buffer" --authority "$offer" ${mainnet:+--mainnet "$mainnet"}
  echo "PASS  new code uploaded and offered to $offer"
  echo "      buffer address: $buffer"
  echo "Next: every signer of the multi-signature wallet runs"
  echo "        node scripts/deploy/verify-buffer.mjs $buffer --authority $offer"
  echo "      and approves only if it passes. Then, in the wallet's app, upgrade program $id from that buffer."
  echo "      The deposit goes to whichever address that upgrade names as the place for leftover SOL."
  echo "Then: node scripts/deploy/verify-code.mjs"
fi
