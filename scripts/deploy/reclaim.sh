#!/bin/bash
# Gets back the SOL held by an upload that was started and not finished, by
# closing its holding account. Use it when a deploy or upgrade stopped half
# way and you want the deposit back rather than carrying on.
#   ./scripts/deploy/reclaim.sh --payer <file> [--mainnet yes] [--url <network>]
# It only touches holding accounts made by that wallet. It cannot touch a deployed program.
set -euo pipefail
START_DIR="$PWD"
cd "$(dirname "$0")/../.."
. scripts/deploy/net.sh
url=""; payer=""; mainnet=""
while [ $# -gt 0 ]; do
  [ $# -ge 2 ] || { echo "STOPPED: $1 needs a value." >&2; exit 1; }
  case "$1" in
    --url) url="$2";; --payer) payer="$2";; --mainnet) mainnet="$2";;
    *) echo "Unknown option $1" >&2; exit 1;;
  esac
  shift 2
done
payer=$(full_path "$payer")
[ -n "$payer" ] && [ -f "$payer" ] || { echo "STOPPED: give --payer <file>." >&2; exit 1; }
net_check "$url" "$mainnet"
mkdir -p scripts/deploy/out
who=$(solana-keygen pubkey "$payer")
before=$(chain balance "$who") || exit 1
# First the one holding account this folder knows about, by its address. Then any others this wallet made.
# The note of that holding account is only ever removed after this run has itself
# seen its deposit come back, on the network the upload was made on.
if [ -f "$HOLDING" ]; then
  where=$(solana-keygen pubkey "$HOLDING")
  made_on=$(cat "$HOLDING.net" 2>/dev/null || true)
  if [ -n "$made_on" ] && [ "$made_on" != "$genesis" ]; then
    echo "STOPPED: the unfinished upload on record was made on a different network from the one this window is connected to. Nothing was done." >&2
    echo "Check BURRPAD_RPC_URL (and --mainnet yes) and run this again." >&2; exit 1
  fi
  held=$(chain balance "$where") || exit 1
  if awk -v h="$held" 'BEGIN { exit !(h > 0) }'; then
    echo "Closing the holding account $where, which holds $held SOL ..."
    sol program close "$where" --keypair "$payer" || true
    gone=no
    for attempt in 1 2 3 4 5 6; do
      sleep 3
      held=$(chain balance "$where") || exit 1
      if awk -v h="$held" 'BEGIN { exit !(h == 0) }'; then gone=yes; break; fi
    done
    if [ "$gone" = no ]; then
      echo "STOPPED: the deposit still shows as held at $where. Nothing is lost." >&2
      echo "Is --payer the same key file the upload was started with? If so, wait a minute and run this again. (An upload already offered to a multi-signature wallet can only be closed by that wallet.)" >&2; exit 1
    fi
    holding_done
  else
    echo "The holding account on record ($where) holds nothing on this network. Its note is kept."
  fi
fi
searched=yes
sol program close --buffers --keypair "$payer" > "scripts/deploy/out/reclaim.log" 2>&1 || { searched=no; echo "NOTE  could not look for other unfinished uploads on this connection (some providers do not allow that search)."; }
sleep 2
after=$(chain balance "$who") || exit 1
echo "The wallet had $before SOL and now has $after SOL."
if awk -v a="$after" -v b="$before" 'BEGIN { exit !(a > b) }'; then echo "PASS  the deposit is back"; elif [ "$searched" = yes ]; then echo "Nothing more was found being held."; else echo "No deposit came back on this run."; fi
