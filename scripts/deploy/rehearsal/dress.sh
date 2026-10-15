#!/bin/bash
# Rehearsal only: a dress rehearsal of an upgrade against the main network's
# real state, on a local validator with its own ports. Reads the main network,
# sends nothing there. Copies the live program and settings and the named
# coins, then runs the same calls twice from that same copy: once on the
# program as it is, once after the extend and the upgrade done the way the
# multi-signature wallet will (a throwaway key stands in for its vault), to
# the release build in target/release-kit/. Then compares the two.
#   ./scripts/deploy/rehearsal/dress.sh <name> <mint,mint,...>
# Needs BURRPAD_MAINNET_READ_URL: a main-network address to read from.
set -euo pipefail
cd "$(dirname "$0")/../../.."
export PATH="$HOME/.cargo/bin:$HOME/.local/share/solana/install/active_release/bin:$PATH"
run="${1:?give this run a name}"; coins="${2:?give the coins}"
[[ "$run" =~ ^[A-Za-z0-9_-]+$ ]] || { echo "STOPPED: the run's name may only have letters, digits, - and _." >&2; exit 1; }
: "${BURRPAD_MAINNET_READ_URL:?set BURRPAD_MAINNET_READ_URL}"
root=$(pwd); work="tmp/rehearsal/$run"
port="${REHEARSAL_PORT:-18899}"; url="http://127.0.0.1:$port"
if solana cluster-version --url "$url" >/dev/null 2>&1; then echo "STOPPED: something is already running on $url." >&2; exit 1; fi
step() { echo; echo "===== $* ====="; }

step "1. the owner's exact commit, the rehearsal's tools, throwaway keys"
commit=$(cat target/release-kit/commit.txt)
rm -rf "$work"; mkdir -p "$work/owner" "$work/tools" "$work/keys"
# The upgrade steps run from the very commit the owner builds; the rehearsal's own tools from this folder.
git archive "$commit" | tar -x -C "$work/owner"
git ls-files -co --exclude-standard scripts | while read -r f; do mkdir -p "$work/tools/$(dirname "$f")"; cp "$f" "$work/tools/$f"; done
for d in owner tools; do mkdir -p "$work/$d/target"; cp -R target/release-kit "$work/$d/target/"; ( cd "$work/$d/scripts/deploy" && npm ci --ignore-scripts --no-audit --no-fund >/dev/null 2>&1 ); done
for k in standin deployer; do solana-keygen new --no-bip39-passphrase --silent --outfile "$work/keys/$k.json" >/dev/null; done
key() { echo "$root/$work/keys/$1.json"; }; addr() { solana-keygen pubkey "$(key "$1")"; }
kit="$work/tools/scripts/deploy"
echo "owner's commit: ${commit:0:7}; release build sha256 $(shasum -a 256 target/release-kit/burrpad.so | cut -c1-16)…"

step "2. the main network's state, read only"
BURRPAD_RPC_URL="$BURRPAD_MAINNET_READ_URL" node "$kit/rehearsal/mainnet-copy.mjs" "$work/accounts" --stand-in "$(addr standin)" --trader "$(addr deployer)" --coins "$coins" --mainnet yes | grep -v "^Network:" > "$work/validator-args.txt"
export BURRPAD_RPC_URL="$url"

validator=""
start() {
  rm -rf "$work/ledger"
  # shellcheck disable=SC2046
  solana-test-validator --reset --quiet --ledger "$work/ledger" --bind-address 127.0.0.1 --rpc-port "$port" --faucet-port "$((port + 1001))" \
    --gossip-port "$((port + 1002))" --dynamic-port-range "$((port + 1003))-$((port + 1040))" \
    --deactivate-feature B8JJXCy5amZyWG9r7EnUYLwzXSXTxG7GZ1qZ1qggo83g --deactivate-feature ExtendProgCheckedWi11BeDe1eted11111111111111 \
    $(cat "$work/validator-args.txt") > "$work/validator.log" 2>&1 &
  validator=$!
  n=0; until solana cluster-version --url "$url" >/dev/null 2>&1; do n=$((n + 1)); [ $n -lt 90 ] || { echo "STOPPED: the validator did not start."; tail -5 "$work/validator.log"; exit 1; }; sleep 1; done
  solana airdrop 50 "$(addr deployer)" --url "$url" >/dev/null; solana airdrop 5 "$(addr standin)" --url "$url" >/dev/null
  sleep 2
  node "$kit/address-table.mjs" --payer "$(key deployer)" >/dev/null
}
stop() { [ -n "$validator" ] && kill "$validator" 2>/dev/null && wait "$validator" 2>/dev/null || true; validator=""; }
trap stop EXIT

step "3. before: the calls on the program as it is"
start
echo "program room: $(solana program show 42w1jV45WeSmfM96vr6GrqSWRcyF5rVbPWjmQ89bvdhg --url "$url" | grep 'Data Length')"
node "$kit/rehearsal/dress.mjs" before --payer "$(key deployer)" --coins "$coins" --out "$work/before.json"
stop

step "4. after: extend and upgrade as the multi-signature wallet will, then the same calls"
start
( cd "$work/owner" && node scripts/deploy/extend.mjs --authority "$(addr standin)" ) > "$work/extend.txt"
grep -E "Room now|Build|Adds|Rent|Vault has" "$work/extend.txt"
tail -1 "$work/extend.txt" > "$work/extend-tx.txt"
node "$kit/rehearsal/stand-in.mjs" sign "$work/extend-tx.txt" --key "$(key standin)"
sleep 2
( cd "$work/owner" && ./scripts/deploy/upgrade.sh --built --payer "$(key deployer)" --offer-to "$(addr standin)" ) > "$work/upgrade.log" 2>&1 || { grep -vE "^[a-z]+( [a-z]+){11}$" "$work/upgrade.log" | tail -20; exit 1; }
grep -E "^(PASS|FAIL|STOPPED)|reproducible|buffer address" "$work/upgrade.log"
buffer=$(grep "buffer address:" "$work/upgrade.log" | awk '{print $NF}')
( cd "$work/owner" && node scripts/deploy/verify-buffer.mjs "$buffer" --authority "$(addr standin)" ) | grep -E "PASS|FAIL"
node "$kit/rehearsal/stand-in.mjs" upgrade "$buffer" --spill "$(addr deployer)" --key "$(key standin)"
sleep 2
( cd "$work/owner" && node scripts/deploy/verify-code.mjs ) | grep -E "PASS|FAIL"
node "$kit/rehearsal/dress.mjs" after --payer "$(key deployer)" --coins "$coins" --out "$work/after.json"
stop

step "5. what differs"
node "$kit/rehearsal/dress-compare.mjs" "$work/before.json" "$work/after.json"
