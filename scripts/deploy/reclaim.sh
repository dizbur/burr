#!/bin/bash
# Gets back the SOL held by an upload that was started and not finished, by
# closing its holding account. Use it when a deploy or upgrade stopped half
# way and you want the deposit back rather than carrying on.
#   ./scripts/deploy/reclaim.sh --payer <file> [--mainnet yes] [--url <network>]
# It only touches holding accounts made by that wallet. It cannot touch a deployed program.
set -euo pipefail
cd "$(dirname "$0")/../.."
. scripts/deploy/net.sh
url="${BURRPAD_RPC_URL:-}"; payer=""; mainnet=""
while [ $# -gt 0 ]; do
  case "$1" in
    --url) url="$2";; --payer) payer="$2";; --mainnet) mainnet="$2";;
    *) echo "Unknown option $1" >&2; exit 1;;
  esac
  shift 2
done
[ -n "$payer" ] && [ -f "$payer" ] || { echo "STOPPED: give --payer <file>." >&2; exit 1; }
net_check "$url" "$mainnet"
who=$(solana-keygen pubkey "$payer")
before=$(solana balance "$who" --url "$url" | awk '{print $1}')
solana program close --buffers --url "$url" --keypair "$payer" > /dev/null 2>&1 || true
rm -f "$HOLDING"
sleep 2
after=$(solana balance "$who" --url "$url" | awk '{print $1}')
echo "The wallet had $before SOL and now has $after SOL."
if awk -v a="$after" -v b="$before" 'BEGIN { exit !(a > b) }'; then echo "PASS  the deposit is back"; else echo "Nothing was being held."; fi
