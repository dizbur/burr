#!/bin/bash
# Rehearses the whole deploy kit from a clean start, on a local validator
# that runs pump.fun's real programs. Nothing leaves this machine.
#   ./scripts/deploy/rehearsal/rehearse.sh <name of this run>
# Needs fixtures/ (scripts/ground/07-fetch-fixtures.mjs). Takes about 40
# minutes, most of it waiting for a coin's first holder release to be allowed.
# It follows the order of a real first deploy: one wallet deploys and
# administers, the first coin is checked, the program is upgraded, and only
# then is everything handed to a multi-signature wallet (here a stand-in key).
# Every key it makes is a throwaway test key under tmp/, which is not committed.
set -euo pipefail
cd "$(dirname "$0")/../../.."
export PATH="$HOME/.cargo/bin:$HOME/.local/share/solana/install/active_release/bin:$PATH"
run="${1:?give this run a name}"
root=$(pwd)
work="tmp/rehearsal/$run"
port="${REHEARSAL_PORT:-8899}"   # set REHEARSAL_PORT if another validator already uses 8899
url="http://127.0.0.1:$port"
[ -d fixtures ] || { echo "STOPPED: fixtures/ is missing." >&2; exit 1; }
if solana cluster-version --url "$url" >/dev/null 2>&1; then echo "STOPPED: something is already running on $url. Set REHEARSAL_PORT to another port." >&2; exit 1; fi

step() { echo; echo "===== $* ====="; }

step "1. a clean copy of the code, and throwaway test keys"
rm -rf "$work"; mkdir -p "$work/src" "$work/keys"
git ls-files -co --exclude-standard | while read -r f; do
  [ -f "$f" ] || continue
  mkdir -p "$work/src/$(dirname "$f")"; cp "$f" "$work/src/$f"
done
for k in program deployer wallet platform rewards; do
  solana-keygen new --no-bip39-passphrase --silent --outfile "$work/keys/$k.json" >/dev/null
done
key() { echo "$root/$work/keys/$1.json"; }
addr() { solana-keygen pubkey "$(key "$1")"; }
echo "program $(addr program)"
echo "deploy wallet $(addr deployer)"
echo "stand-in for the multi-signature wallet $(addr wallet)"
( cd "$work/src/scripts/deploy" && npm ci --no-audit --no-fund >/dev/null 2>&1 )
kit="$work/src/scripts/deploy"
export BURRPAD_RPC_URL="$url"
in_copy() { ( cd "$work/src" && "$@" ); }

step "2. a local validator with pump.fun's real programs"
# A local validator switches on every planned rule change. One of them, not
# in force on the main network (checked 13 Oct 2026), refuses to deploy
# programs in the format this one and pump.fun's own are built in. It is
# switched off here so the rehearsal follows the main network's rules.
node "$kit/rehearsal/local-accounts.mjs" fixtures "$work/accounts" "$(addr deployer)" > "$work/validator-args.txt"
# shellcheck disable=SC2046
solana-test-validator --reset --quiet --ledger "$work/ledger" --bind-address 127.0.0.1 --rpc-port "$port" --faucet-port "$((port + 1001))" \
  --gossip-port "$((port + 1002))" --dynamic-port-range "$((port + 1003))-$((port + 1040))" \
  --deactivate-feature B8JJXCy5amZyWG9r7EnUYLwzXSXTxG7GZ1qZ1qggo83g \
  $(cat "$work/validator-args.txt") > "$work/validator.log" 2>&1 &
validator=$!
trap 'kill $validator 2>/dev/null || true; wait $validator 2>/dev/null || true' EXIT
n=0
until solana cluster-version --url "$url" >/dev/null 2>&1; do
  n=$((n + 1)); [ $n -lt 60 ] || { echo "STOPPED: the validator did not start."; tail -5 "$work/validator.log"; exit 1; }
  sleep 1
done
echo "validator up: $(solana cluster-version --url "$url")"
solana airdrop 12 "$(addr deployer)" --url "$url" >/dev/null
for k in wallet rewards; do solana airdrop 6 "$(addr "$k")" --url "$url" >/dev/null; done
sol() { solana balance "$(addr "$1")" --url "$url"; }
echo "deploy wallet starts with $(sol deployer)"

# The solana tool prints twelve recovery words for a temporary account each time it uploads code. They are left out of this output.
quiet_upload() { "$@" > "$work/upload.log" 2>&1 || { grep -vE "^[a-z]+( [a-z]+){11}$" "$work/upload.log" | tail -25; exit 1; }
  grep -E "^(Network|Program |The program file|The new program file|The new code|Carrying on|  |PASS|FAIL|Next|Then|All items)" "$work/upload.log" || true; }

step "3. the program's address into the code, and the build"
in_copy ./scripts/deploy/set-program-id.sh "$(addr program)"
in_copy ./scripts/deploy/build.sh --plain > "$work/build.log" 2>&1 || { tail -30 "$work/build.log"; exit 1; }
grep -E "^(PASS|      )" "$work/build.log"

step "4. deploy"
echo "-- with too little SOL it refuses before sending anything, and says by how much:"
solana-keygen new --no-bip39-passphrase --silent --outfile "$work/keys/poor.json" >/dev/null
solana airdrop 1 "$(addr poor)" --url "$url" >/dev/null
if in_copy ./scripts/deploy/deploy.sh --program-key "$(key program)" --payer "$(key poor)" > "$work/again.log" 2>&1; then echo "FAIL  it started without enough SOL"; exit 1; else grep -E "needed to start|this wallet has|STOPPED" "$work/again.log"; echo "PASS  refused"; fi
half_upload() { ( cd "$work/src" && mkdir -p scripts/deploy/out && solana-keygen new --no-bip39-passphrase --silent --outfile scripts/deploy/out/upload-holding-keypair.json >/dev/null \
  && solana program write-buffer target/deploy/burrpad.so --url "$url" --keypair "$(key deployer)" --buffer scripts/deploy/out/upload-holding-keypair.json >/dev/null 2>&1 ); }
echo "-- an upload left unfinished: the deposit can be taken back"
half_upload
echo "deploy wallet with an unfinished upload: $(sol deployer)"
in_copy ./scripts/deploy/reclaim.sh --payer "$(key deployer)"
echo "-- an upload left unfinished: or the same command carries on from it"
half_upload
echo "deploy wallet with an unfinished upload: $(sol deployer)"
quiet_upload in_copy ./scripts/deploy/deploy.sh --program-key "$(key program)" --payer "$(key deployer)"
echo "deploy wallet now has $(sol deployer)"
echo "-- a second deploy to the same address is refused:"
if in_copy ./scripts/deploy/deploy.sh --program-key "$(key program)" --payer "$(key deployer)" > "$work/again.log" 2>&1; then echo "FAIL  it deployed twice"; exit 1; else grep STOPPED "$work/again.log"; echo "PASS  refused"; fi

step "5. initialize, in the same sitting"
node "$kit/initialize.mjs" --upgrade-key "$(key deployer)" --platform-receiver "$(addr platform)" --rewards-wallet "$(addr rewards)"
echo "-- a second time changes nothing:"
node "$kit/initialize.mjs" --upgrade-key "$(key deployer)" --platform-receiver "$(addr platform)" --rewards-wallet "$(addr rewards)"

step "6. the address table"
node "$kit/address-table.mjs" --payer "$(key deployer)"

verify() { node "$kit/verify.mjs" --wallet "$(addr "$1")" --platform-receiver "$(addr platform)" --rewards-wallet "$(addr rewards)"; }
step "7. read everything back (one wallet is upgrade key and administrator, as in the first tests)"
verify deployer
echo "set-up cost the deploy wallet: started with 12 SOL, now $(sol deployer)"

step "8. the first-coin check, first half"
before=$(sol deployer)
node "$kit/first-coin.mjs" start --payer "$(key deployer)"
echo "the first half cost: from $before to $(sol deployer)"

step "9. upgrade the program in place, and read it back"
quiet_upload in_copy ./scripts/deploy/upgrade.sh --plain --upgrade-key "$(key deployer)"
echo "deploy wallet after the upgrade: $(sol deployer)"

step "10. stop everything: the two pause switches"
node "$kit/admin.mjs" pause-launches --admin-key "$(key deployer)"
node "$kit/admin.mjs" pause-holder-releases --admin-key "$(key deployer)"
node "$kit/rehearsal/exercise.mjs" paused --payer "$(key deployer)" --rewards-key "$(key rewards)"
node "$kit/admin.mjs" resume-launches --admin-key "$(key deployer)"
node "$kit/admin.mjs" resume-holder-releases --admin-key "$(key deployer)"
node "$kit/rehearsal/exercise.mjs" resumed --payer "$(key deployer)" --rewards-key "$(key rewards)"

step "11. the first-coin check, second half (after the thirty minutes)"
echo "-- too early, it sends nothing:"
if node "$kit/first-coin.mjs" finish --payer "$(key deployer)" --rewards-key "$(key rewards)"; then echo "FAIL  it ran early"; exit 1; else echo "PASS  refused"; fi
[ -z "${REHEARSAL_QUICK:-}" ] || echo "(quick run: the thirty-minute wait and the second half are skipped)"
echo "waiting ..."
until [ -n "${REHEARSAL_QUICK:-}" ] || node "$kit/first-coin.mjs" finish --payer "$(key deployer)" --rewards-key "$(key rewards)" > "$work/finish.log" 2>&1; do
  grep -q "too early" "$work/finish.log" || { cat "$work/finish.log"; exit 1; }
  sleep 30
done
[ -n "${REHEARSAL_QUICK:-}" ] || cat "$work/finish.log"

step "12. before the public: hand the administrator role to the wallet (two steps), then the upgrade key"
echo "-- the upgrade key cannot be handed over before the wallet has proved itself:"
if node "$kit/hand-over-upgrade.mjs" --upgrade-key "$(key deployer)" --new-authority "$(addr wallet)"; then
  echo "FAIL  the hand-over went through too early"; exit 1
else echo "PASS  refused, as it should be"; fi
node "$kit/admin.mjs" propose-admin "$(addr wallet)" --admin-key "$(key deployer)"
echo "-- what a multi-signature wallet would be given to approve (not sent):"
node "$kit/admin.mjs" accept-admin --admin "$(addr wallet)" | head -4 | cut -c1-90
node "$kit/admin.mjs" accept-admin --admin-key "$(key wallet)"
node "$kit/hand-over-upgrade.mjs" --upgrade-key "$(key deployer)" --new-authority "$(addr wallet)"
echo "-- the old key can no longer administer or upgrade:"
if node "$kit/admin.mjs" pause-launches --admin-key "$(key deployer)" >/dev/null 2>&1; then echo "FAIL  the old administrator still has the role"; exit 1; else echo "PASS  administering refused"; fi
if in_copy ./scripts/deploy/upgrade.sh --plain --upgrade-key "$(key deployer)" > "$work/again.log" 2>&1; then echo "FAIL  the old key upgraded"; exit 1; else grep STOPPED "$work/again.log"; echo "PASS  upgrading refused"; fi

step "13. an upgrade once the wallet holds the key: upload, offer, and the wallet approves"
quiet_upload in_copy ./scripts/deploy/upgrade.sh --plain --payer "$(key deployer)" --offer-to "$(addr wallet)"
buffer=$(awk '/buffer address:/ {print $3}' "$work/upload.log")
echo "-- the wallet's part (done in its own app on the real network; here the stand-in key does it):"
solana program deploy --buffer "$buffer" --program-id "$(addr program)" --upgrade-authority "$(key wallet)" --keypair "$(key wallet)" --url "$url" > "$work/wallet-upgrade.log" 2>&1 \
  || { tail -5 "$work/wallet-upgrade.log"; exit 1; }
sleep 2
node "$kit/verify-code.mjs"

step "14. read everything back once more"
verify wallet

step "REHEARSAL PASSED"
rm -rf "$work/src/target" "$work/ledger"
