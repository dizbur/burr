#!/bin/bash
# Rehearsal only: measures a real-size launch with a first buy on a local
# validator that runs pump.fun's real programs, the way it is done today and
# with the launch making the launcher's coin account. Nothing leaves this
# machine; every key is a throwaway test key under tmp/.
#   ./scripts/deploy/rehearsal/launch-size.sh <name of this run>
set -euo pipefail
cd "$(dirname "$0")/../../.."
export PATH="$HOME/.cargo/bin:$HOME/.local/share/solana/install/active_release/bin:$PATH"
run="${1:?give this run a name}"
[[ "$run" =~ ^[A-Za-z0-9_-]+$ ]] || { echo "STOPPED: the run's name may only have letters, digits, - and _." >&2; exit 1; }
root=$(pwd)
work="tmp/rehearsal/$run"
port="${REHEARSAL_PORT:-8899}"
url="http://127.0.0.1:$port"
[ -d fixtures ] || { echo "STOPPED: fixtures/ is missing." >&2; exit 1; }
if solana cluster-version --url "$url" >/dev/null 2>&1; then echo "STOPPED: something is already running on $url." >&2; exit 1; fi

rm -rf "$work"; mkdir -p "$work/src" "$work/keys"
git ls-files -co --exclude-standard | while read -r f; do
  [ -f "$f" ] || continue
  mkdir -p "$work/src/$(dirname "$f")"; cp "$f" "$work/src/$f"
done
for k in program deployer platform rewards; do solana-keygen new --no-bip39-passphrase --silent --outfile "$work/keys/$k.json" >/dev/null; done
key() { echo "$root/$work/keys/$1.json"; }
addr() { solana-keygen pubkey "$(key "$1")"; }
( cd "$work/src/scripts/deploy" && npm ci --ignore-scripts --no-audit --no-fund >/dev/null 2>&1 )
kit="$work/src/scripts/deploy"
export BURRPAD_RPC_URL="$url"
in_copy() { ( cd "$work/src" && "$@" ); }

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
solana airdrop 20 "$(addr deployer)" --url "$url" >/dev/null
for k in platform rewards; do solana airdrop 1 "$(addr "$k")" --url "$url" >/dev/null; done

in_copy ./scripts/deploy/set-program-id.sh "$(addr program)" >/dev/null
in_copy ./scripts/deploy/build.sh --plain > "$work/build.log" 2>&1 || { tail -30 "$work/build.log"; exit 1; }
in_copy ./scripts/deploy/deploy.sh --program-key "$(key program)" --payer "$(key deployer)" > "$work/deploy.log" 2>&1 || { grep -vE "^[a-z]+( [a-z]+){11}$" "$work/deploy.log" | tail -20; exit 1; }
node "$kit/initialize.mjs" --upgrade-key "$(key deployer)" --platform-receiver "$(addr platform)" --rewards-wallet "$(addr rewards)" >/dev/null
node "$kit/address-table.mjs" --payer "$(key deployer)" >/dev/null
node "$kit/rehearsal/launch-size.mjs" --payer "$(key deployer)"
