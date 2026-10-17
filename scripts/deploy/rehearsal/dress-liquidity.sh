#!/bin/bash
# Rehearsal only: a dress rehearsal of two upgrades in a row, against the main
# network's real state, on a local validator with its own ports. Reads the
# main network, sends nothing there. Copies the live program, its settings and
# the named coins; runs the calls once on the program as it is; then upgrades
# the copy as the multi-signature wallet will, first to the build in
# target/release-kit/ (the next upgrade's, from its exact commit), then to a
# plain build of this folder's commit; opens private creator payouts with
# admin.mjs; and runs the same calls again, plus the new ones. Then compares.
#   ./scripts/deploy/rehearsal/dress-liquidity.sh <name> <mint,mint,...>
# Needs BURRPAD_MAINNET_READ_URL: a main-network address to read from.
# DRESS_SECOND_KIT=<a release-kit folder> upgrades second to that build (from the
# commit in its commit.txt) instead of a plain build of this folder's commit:
# the way to rehearse the exact Docker build that will be offered.
# DRESS_ONE_JUMP=yes skips the first upgrade: the live program goes straight to the second build.
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
# The main network's rent, read 15 Oct (lamports a byte); the local validator asks more.
MAINNET_LAMPORTS_PER_BYTE=5080

step "1. the two commits to upgrade to, the rehearsal's tools, throwaway keys"
first=$(cat target/release-kit/commit.txt)
second=$(git rev-parse HEAD)
[ -z "${DRESS_SECOND_KIT:-}" ] || second=$(cat "$DRESS_SECOND_KIT/commit.txt")
rm -rf "$work"; mkdir -p "$work/first" "$work/second" "$work/tools" "$work/keys"
git archive "$first" | tar -x -C "$work/first"
mkdir -p "$work/first/target"; cp -R target/release-kit "$work/first/target/"
git archive "$second" | tar -x -C "$work/second"
if [ -n "${DRESS_SECOND_KIT:-}" ]; then
  mkdir -p "$work/second/target"; cp -R "$DRESS_SECOND_KIT" "$work/second/target/release-kit"
else
  ( cd "$work/second" && ./scripts/deploy/build.sh --plain > "$root/$work/build-second.log" 2>&1 ) || { tail -20 "$work/build-second.log"; exit 1; }
fi
git ls-files -co --exclude-standard scripts | while read -r f; do mkdir -p "$work/tools/$(dirname "$f")"; cp "$f" "$work/tools/$f"; done
mkdir -p "$work/tools/target"; cp -R "$work/second/target/release-kit" "$work/tools/target/"
for d in first second tools; do ( cd "$work/$d/scripts/deploy" && npm ci --ignore-scripts --no-audit --no-fund >/dev/null 2>&1 ); done
for k in standin deployer payout rewards; do solana-keygen new --no-bip39-passphrase --silent --outfile "$work/keys/$k.json" >/dev/null; done
key() { echo "$root/$work/keys/$1.json"; }; addr() { solana-keygen pubkey "$(key "$1")"; }
kit="$work/tools/scripts/deploy"
size1=$(wc -c < "$work/first/target/release-kit/burrpad.so" | tr -d ' '); size2=$(wc -c < "$work/second/target/release-kit/burrpad.so" | tr -d ' ')
echo "first:  ${first:0:7} ($(cat "$work/first/target/release-kit/mode.txt") build, $size1 bytes, sha256 $(shasum -a 256 "$work/first/target/release-kit/burrpad.so" | cut -c1-16)…)"
echo "second: ${second:0:7} ($(cat "$work/second/target/release-kit/mode.txt") build, $size2 bytes, sha256 $(shasum -a 256 "$work/second/target/release-kit/burrpad.so" | cut -c1-16)…)"

step "2. the main network's state, read only (the stand-in for the vault is upgrade key and administrator)"
BURRPAD_RPC_URL="$BURRPAD_MAINNET_READ_URL" node "$kit/rehearsal/mainnet-copy.mjs" "$work/accounts" --stand-in "$(addr standin)" --trader "$(addr deployer)" --coins "$coins" --admin-stand-in yes --rewards-stand-in "$(addr rewards)" --mainnet yes | grep -v "^Network:" > "$work/validator-args.txt"
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
  solana airdrop 100 "$(addr deployer)" --url "$url" >/dev/null; solana airdrop 10 "$(addr standin)" --url "$url" >/dev/null
  solana airdrop 1 "$(addr rewards)" --url "$url" >/dev/null
  sleep 2
  node "$kit/address-table.mjs" --payer "$(key deployer)" >/dev/null
}
stop() { [ -n "$validator" ] && kill "$validator" 2>/dev/null && wait "$validator" 2>/dev/null || true; validator=""; }
trap stop EXIT
# One upgrade, done as the multi-signature wallet will, from the given commit's own copy.
upgrade() {
  local dir="$work/$1" room
  room=$(solana program show 42w1jV45WeSmfM96vr6GrqSWRcyF5rVbPWjmQ89bvdhg --url "$url" | awk '/Data Length/ {print $3}')
  # The extend is sent by the deploy wallet itself, never through the multi-signature wallet: the network
  # refuses an extend made from inside another program (Squads), which a stand-in key signing directly cannot show.
  node "$kit/extend.mjs" --authority "$(addr standin)" --release-kit "$dir/target/release-kit" --payer "$(key deployer)" --send yes > "$work/extend-$1.txt" 2>&1 || { tail -5 "$work/extend-$1.txt"; exit 1; }
  grep -E "Room now|Build|Adds|PASS|FAIL|STOPPED|No extend needed" "$work/extend-$1.txt" || true
  local grow; grow=$(awk '/^Adds:/ {print $2}' "$work/extend-$1.txt")
  [ -z "$grow" ] || echo "On the main network that room costs about $(awk -v b="$grow" -v r=$MAINNET_LAMPORTS_PER_BYTE 'BEGIN { printf "%.4f", b * r / 1e9 }') SOL (the local figure above is at the local rent)."
  sleep 2
  ( cd "$dir" && ./scripts/deploy/upgrade.sh --built --payer "$(key deployer)" --offer-to "$(addr standin)" ) > "$work/upgrade-$1.log" 2>&1 || { grep -vE "^[a-z]+( [a-z]+){11}$" "$work/upgrade-$1.log" | tail -20; exit 1; }
  grep -E "^(PASS|FAIL|STOPPED)" "$work/upgrade-$1.log"
  local buffer; buffer=$(grep "buffer address:" "$work/upgrade-$1.log" | awk '{print $NF}')
  ( cd "$dir" && node scripts/deploy/verify-buffer.mjs "$buffer" --authority "$(addr standin)" ) | grep -E "PASS|FAIL"
  node "$kit/rehearsal/stand-in.mjs" upgrade "$buffer" --spill "$(addr deployer)" --key "$(key standin)" | grep -v "^Network:"
  sleep 2
  ( cd "$dir" && node scripts/deploy/verify-code.mjs ) | grep -E "PASS|FAIL"
  echo "room was $room bytes"
}

step "3. before: the calls on the program as it is"
start
echo "-- the after-upgrade check keeps the settings as they are now, for after both upgrades"
node "$kit/after-upgrade.mjs" --save yes --release-kit "$work/second/target/release-kit" --authority "$(addr standin)" | grep -v "^Network:"
node "$kit/rehearsal/dress.mjs" before --payer "$(key deployer)" --coins "$coins" --out "$work/before.json" --rewards-key "$(key rewards)" | grep -v "^Network:"
stop

step "4. after: the upgrade(s), each as the multi-signature wallet will"
start
if [ "${DRESS_ONE_JUMP:-}" = yes ]; then echo "-- one upgrade only: from the live program straight to the second build"
else echo "-- first upgrade (${first:0:7})"; upgrade first; fi
echo "-- second upgrade (${second:0:7})"; upgrade second
echo "-- the after-upgrade check, right after the second upgrade, against the second build"
# The live coins' burn pots are often empty (the keeper burns every minute): then "at least one burn all the way
# through the buy" is shown again below, on the coins launched in this rehearsal. Any other failure stops here.
node "$kit/after-upgrade.mjs" --release-kit "$work/second/target/release-kit" --authority "$(addr standin)" --coins "$coins" > "$work/after-upgrade-1.txt" 2>&1 || true
grep -v "^Network:" "$work/after-upgrade-1.txt"
others=$(grep "^FAIL" "$work/after-upgrade-1.txt" | grep -v "at least one burn was simulated all the way through the buy" || true)
[ -z "$others" ] || { echo "STOPPED: the after-upgrade check failed."; exit 1; }
echo "-- private creator payouts opened with admin.mjs: set by the administrator, confirmed by the wallet itself"
solana airdrop 1 "$(addr payout)" --url "$url" >/dev/null
node "$kit/admin.mjs" set-creator-payout-wallet "$(addr payout)" --admin-key "$(key standin)" | grep -E "PASS|STOPPED|creator payout"
node "$kit/admin.mjs" confirm-creator-payout-wallet --admin-key "$(key payout)" | grep -E "PASS|STOPPED|creator payout"
if grep -q '"burn_burr_pot"' "$work/second/target/release-kit/burrpad.json"; then
  echo "-- before the first coin priced in \$BURR, the owner's two steps: pump.fun's buy-back receivers' \$BURR accounts, and the second address table"
  node "$kit/burr-buyback-accounts.mjs" --payer "$(key deployer)" --send yes | grep -E "PASS|FAIL|receivers|costs"
  node "$kit/burr-table.mjs" --payer "$(key deployer)" | grep -E "PASS|FAIL|TABLE"
fi
node "$kit/rehearsal/dress.mjs" after --payer "$(key deployer)" --coins "$coins" --out "$work/after.json" --payout-wallet "$(addr payout)" --rewards-key "$(key rewards)" | grep -v "^Network:"
if grep -q "at least one burn was simulated all the way through the buy" <(grep "^FAIL" "$work/after-upgrade-1.txt"); then
  echo "-- the after-upgrade check again, adding the coins launched just now (their burn pots hold ZEC)"
  launched=$(node -e 'const r = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")); console.log([...new Set(r.filter((x) => x.ok && /^launch/.test(x.name)).map((x) => x.mint))].join(","))' "$work/after.json")
  # The settings changed on purpose since the first check (the payout wallet): compare from now.
  node "$kit/after-upgrade.mjs" --save yes --release-kit "$work/second/target/release-kit" --authority "$(addr standin)" | grep -v "^Network:"
  node "$kit/after-upgrade.mjs" --release-kit "$work/second/target/release-kit" --authority "$(addr standin)" --coins "$coins,$launched" | grep -v "^Network:"
fi
stop

step "5. what differs"
node "$kit/rehearsal/dress-compare.mjs" "$work/before.json" "$work/after.json"
