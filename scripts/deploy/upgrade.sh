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
START_DIR="$PWD"
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
key=$(full_path "$key"); payer=$(full_path "$payer")
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

# An upload already offered to this wallet for this build on this network, and interrupted before its check?
# Then only the check is left: nothing is built, uploaded or paid for again.
mkdir -p scripts/deploy/out
note="scripts/deploy/out/offered-buffer.txt"
offered=""
if [ -n "$offer" ]; then
  if [ "$build" != "none" ] && [ ! -f "$note" ] && [ ! -f "$HOLDING" ]; then
    if [ "$build" = "--plain" ]; then ./scripts/deploy/build.sh --plain; else ./scripts/deploy/build.sh; fi
  fi
  built_check
  mark="$genesis $offer $(shasum -a 256 "$BUILT" | cut -d' ' -f1)"
  if [ -f "$note" ]; then
    if [ "$(sed -n 1p "$note")" = "$mark" ]; then offered=$(sed -n 2p "$note"); else
      echo "NOTE  an earlier offer on record was for another build, wallet or network; it is ignored. (Its upload, if unused, can only be closed by the wallet it was offered to.)"
      rm -f "$note"
    fi
  fi
  # The hand-over may have landed although the tool said it failed: look before uploading again.
  if [ -z "$offered" ] && [ -f "$HOLDING" ]; then
    maybe=$(solana-keygen pubkey "$HOLDING")
    if node scripts/deploy/verify-buffer.mjs "$maybe" --authority "$offer" ${mainnet:+--mainnet "$mainnet"} >/dev/null 2>&1; then
      offered="$maybe"; printf '%s\n%s\n' "$mark" "$offered" > "$note"; holding_done
    fi
  fi
else
  case "$build" in
    none) ;;
    --plain) ./scripts/deploy/build.sh --plain;;
    *) ./scripts/deploy/build.sh;;
  esac
  built_check
fi
grep -q "\"address\": \"$id\"" target/release-kit/burrpad.json || { echo "STOPPED: the release build is for a different program address. Run build.sh again." >&2; exit 1; }

if [ -z "$offered" ]; then
  size=$(wc -c < "$BUILT" | tr -d ' ')
  if [ "$size" -gt "$room" ]; then
    # A larger program needs more room, paid for once and kept. The network adds room in steps of at least 10,240 bytes.
    grow=$(( size - room )); [ "$grow" -ge 10240 ] || grow=10240
    if [ -n "$offer" ]; then
      echo "STOPPED: the new code is $((size - room)) bytes larger than the room the program has. The multi-signature wallet must first extend the program by $grow bytes (in its app, or: solana program extend $id $grow)." >&2; exit 1
    fi
  fi
  balance_check "$signer" upgrade
  if [ "$size" -gt "$room" ]; then
    with=$(chain rent "$((room + grow + 45))") || exit 1
    without=$(chain rent "$((room + 45))") || exit 1
    extra=$(awk -v a="$with" -v b="$without" 'BEGIN { printf "%.4f", a - b }')
    echo "The new code is $((size - room)) bytes larger than the room the program has."
    echo "  added to the program for good:    $extra SOL (on top of the figures above)"
    have=$(chain balance "$(solana-keygen pubkey "$signer")") || exit 1
    awk -v h="$have" -v n="$need" -v e="$extra" 'BEGIN { exit !(h >= n + e) }' || { echo "STOPPED: not enough SOL for the larger program. Nothing was sent." >&2; exit 1; }
  fi
fi

if [ -n "$key" ]; then
  holding_key
  # The solana tool makes the extra room itself when the new code is larger.
  sol program deploy "$BUILT" --keypair "$key" --program-id "$id" --upgrade-authority "$key" --buffer "$HOLDING" \
    --use-rpc --max-sign-attempts 30 --with-compute-unit-price "$PRIORITY" || upload_stopped
  holding_done
  read_back "$id" "$mainnet"
else
  if [ -z "$offered" ]; then
    holding_key
    buffer=$(solana-keygen pubkey "$HOLDING")
    echo "The new code goes to a holding account at: $buffer"
    sol program write-buffer "$BUILT" --keypair "$payer" --buffer "$HOLDING" \
      --use-rpc --max-sign-attempts 30 --with-compute-unit-price "$PRIORITY" || upload_stopped
    sol program set-buffer-authority "$buffer" --new-buffer-authority "$offer" --keypair "$payer" \
      || { echo "STOPPED: the upload is complete but the hand-over to the wallet was not confirmed. Run the same command again: it checks whether it went through." >&2; exit 1; }
    printf '%s\n%s\n' "$mark" "$buffer" > "$note"
    holding_done
    sleep 3
  else
    buffer="$offered"
    echo "The new code was already uploaded and offered, at: $buffer"
  fi
  node scripts/deploy/verify-buffer.mjs "$buffer" --authority "$offer" ${mainnet:+--mainnet "$mainnet"} \
    || { echo "The upload is at $buffer and only the wallet $offer can use or close it." >&2
         echo "Run the same command again to check it once more. If it keeps failing and you want to upload afresh, delete $note first." >&2; exit 1; }
  rm -f "$note"
  echo "PASS  new code uploaded and offered to $offer"
  echo "      buffer address: $buffer"
  echo "Next: every signer of the multi-signature wallet runs"
  echo "        node scripts/deploy/verify-buffer.mjs $buffer --authority $offer"
  echo "      and approves only if it passes. Then, in the wallet's app, upgrade program $id from that buffer."
  echo "      The deposit goes to whichever address that upgrade names as the place for leftover SOL."
  echo "Then: node scripts/deploy/verify-code.mjs"
fi
