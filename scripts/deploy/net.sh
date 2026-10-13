# Shared by the shell scripts: the network's address, the main-network guard,
# the check for a rule that would refuse this program's format, and the SOL check.
# The address comes from BURRPAD_RPC_URL (or --url). It is never printed: the
# solana tool is only ever run through `sol`, which removes it from anything shown.
export PATH="$HOME/.cargo/bin:$HOME/.local/share/solana/install/active_release/bin:$PATH"
START_DIR="${START_DIR:-$PWD}"
MAINNET_GENESIS="5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"
OLD_FORMAT_RULE="B8JJXCy5amZyWG9r7EnUYLwzXSXTxG7GZ1qZ1qggo83g"
HOLDING="scripts/deploy/out/upload-holding-keypair.json"
BUILT="target/release-kit/burrpad.so"

# Runs the solana tool against the network, hiding the network's address in
# whatever it prints (it puts the address into its error messages).
sol() {
  local rest="${BURRPAD_RPC_URL#*://}"
  local host="${rest%%/*}"; host="${host%%\?*}"
  local lower; lower=$(printf '%s' "$host" | tr 'A-Z' 'a-z')
  local path=""; case "$rest" in */*) path="${rest#*/}"; path="${path%%\?*}";; esac
  local query=""; case "$BURRPAD_RPC_URL" in *\?*) query="${BURRPAD_RPC_URL#*\?}";; esac
  solana "$@" --url "$BURRPAD_RPC_URL" 2>&1 | while IFS= read -r line || [ -n "$line" ]; do
    line="${line//"$BURRPAD_RPC_URL"/<the network address>}"
    [ ${#path} -lt 8 ] || line="${line//"$path"/<hidden>}"
    [ -z "$query" ] || line="${line//"$query"/<hidden>}"
    line="${line//"$host"/<the network>}"
    line="${line//"$lower"/<the network>}"
    printf '%s\n' "$line"
  done
  return "${PIPESTATUS[0]}"
}
# Turns a key file's path into a full one, so it means the same wherever the script was started from.
full_path() { case "$1" in /*) printf '%s' "$1";; "") ;; *) printf '%s/%s' "$START_DIR" "$1";; esac; }
chain() { node scripts/deploy/chain.mjs "$@"; }

# $1 = the address given with --url, if any; $2 = "yes" if --mainnet yes was given
net_check() {
  [ -z "$1" ] || BURRPAD_RPC_URL="$1"
  [ -n "${BURRPAD_RPC_URL:-}" ] || { echo "STOPPED: no network given. Set BURRPAD_RPC_URL in this terminal (export BURRPAD_RPC_URL=...), or pass --url." >&2; exit 1; }
  export BURRPAD_RPC_URL
  command -v solana >/dev/null 2>&1 || { echo "STOPPED: the solana tool is not installed." >&2; exit 1; }
  [ -d scripts/deploy/node_modules ] || { echo "STOPPED: run  (cd scripts/deploy && npm ci --ignore-scripts)  once first." >&2; exit 1; }
  genesis=$(chain genesis) || { echo "Check BURRPAD_RPC_URL and your internet connection." >&2; exit 1; }
  if [ "$genesis" = "$MAINNET_GENESIS" ]; then
    [ "$2" = "yes" ] || { echo "STOPPED: this is the main network. Add  --mainnet yes  if that is what you mean." >&2; exit 1; }
    echo "Network: MAIN NETWORK"
  else
    # Asked for the main network, got another: a wrong connection address. Stop before anything is done.
    [ "$2" != "yes" ] || { echo "STOPPED: --mainnet yes was given, but this connection is NOT the main network. Check BURRPAD_RPC_URL in this window. Nothing was done." >&2; exit 1; }
    echo "Network: a test network"
  fi
  # The network plans a rule (SIMD-0500) that will refuse to deploy programs
  # in the format this one is built in. Stop clearly rather than half way.
  rule=$(chain rule "$OLD_FORMAT_RULE") || exit 1
  if [ "$rule" = "active" ]; then
    echo "STOPPED: this network no longer accepts programs in the format this one is built in." >&2
    echo "It must be rebuilt in the newer format (cargo build-sbf --arch v3) and the whole test suite run on that build first." >&2
    exit 1
  fi
  # What is paid on top of the base fee, in millionths of a lamport per compute unit.
  PRIORITY="${BURRPAD_PRIORITY_FEE:-50000}"
  case "$PRIORITY" in ''|*[!0-9]*) echo "STOPPED: BURRPAD_PRIORITY_FEE must be a whole number." >&2; exit 1;; esac
  [ "$PRIORITY" -le 2000000 ] || { echo "STOPPED: BURRPAD_PRIORITY_FEE is $PRIORITY, far above anything sensible (the usual is 50000). It is in millionths of a lamport." >&2; exit 1; }
}

# Says which build is about to go out.
built_check() {
  [ -f "$BUILT" ] || { echo "STOPPED: no release build here. Run scripts/deploy/build.sh first." >&2; exit 1; }
  echo "Release build: commit $(cut -c1-7 target/release-kit/commit.txt), $(cat target/release-kit/mode.txt 2>/dev/null || echo unknown) build, sha256 $(shasum -a 256 "$BUILT" | cut -c1-16)…"
}

# Works out what the upload needs from the network's own rent figures for the
# release build, prints it, and stops if the wallet behind key file $1 is
# short, saying by how much. Nothing has been sent when this runs.
#   $1 = key file, $2 = "deploy" or "upgrade"
balance_check() {
  size=$(wc -c < "$BUILT" | tr -d ' ')
  data=$(chain rent "$((size + 45))") || exit 1      # the program's code, with its header
  account=$(chain rent 36) || exit 1                 # the small account at the program's address
  uploads=$(( size / 1000 + 5 ))                     # one transaction per thousand bytes or so
  fees=$(awk -v n="$uploads" 'BEGIN { printf "%.4f", n * 0.00001 }')
  margin=0.02
  have=$(chain balance "$(solana-keygen pubkey "$1")") || exit 1
  held=0
  if [ -f "$HOLDING" ]; then held=$(chain balance "$(solana-keygen pubkey "$HOLDING")") || exit 1; fi
  if awk -v h="$held" 'BEGIN { exit !(h > 0) }'; then
    # An earlier upload already paid its deposit; only fees are needed to carry on.
    # (Whatever of the upload is already there is not sent again, so the fees left are small.)
    need=$(awk -v b="$account" -v c="$fees" 'BEGIN { printf "%.4f", b + c }')
    echo "An upload started earlier already holds $held SOL of deposit."
    echo "  needed to carry on:               $need SOL"
  elif [ "$2" = "deploy" ]; then
    need=$(awk -v a="$data" -v b="$account" -v c="$fees" -v d="$margin" 'BEGIN { printf "%.4f", a + b + c + d }')
    echo "The program file is $size bytes."
    echo "  locked in the program for good:   $data SOL"
    echo "  for the program's address:        $account SOL"
    echo "  upload fees, about:               $fees SOL"
    echo "  spare, in case fees run higher:   $margin SOL"
    echo "  needed to start:                  $need SOL"
  else
    need=$(awk -v a="$data" -v c="$fees" -v d="$margin" 'BEGIN { printf "%.4f", a + c + d }')
    echo "The new program file is $size bytes."
    echo "  held while the new code is uploaded: $data SOL"
    echo "  upload fees, about:               $fees SOL"
    echo "  spare:                            $margin SOL"
    echo "  needed to start:                  $need SOL"
  fi
  echo "  this wallet has:                  $have SOL"
  awk -v h="$have" -v n="$need" 'BEGIN { exit !(h >= n) }' || {
    echo "STOPPED: not enough SOL. Short by $(awk -v h="$have" -v n="$need" 'BEGIN { printf "%.4f", n - h }') SOL. Nothing was sent." >&2
    echo "If an earlier upload was interrupted, its deposit may still be held: ./scripts/deploy/reclaim.sh --payer <the same key file> gets it back." >&2
    exit 1; }
}

# The upload goes through a holding account. Its key file is kept until the
# upload is done, so that running the same command again carries on from
# where it stopped instead of paying a second deposit. It is not one of your
# keys and holds nothing once the upload has landed or been reclaimed.
holding_key() {
  mkdir -p scripts/deploy/out
  local mark; mark="$(shasum -a 256 "$BUILT" | cut -d' ' -f1)"
  if [ -f "$HOLDING" ]; then
    # An unfinished upload can only be carried on with the very same build.
    if awk -v h="${held:-0}" 'BEGIN { exit !(h > 0) }' && [ "$(cat "$HOLDING.build" 2>/dev/null)" != "$mark" ]; then
      echo "STOPPED: an earlier upload of a different build still holds its deposit." >&2
      echo "Get it back first: ./scripts/deploy/reclaim.sh --payer <the same key file>. Then run this again." >&2
      exit 1
    fi
    if awk -v h="${held:-0}" 'BEGIN { exit !(h > 0) }'; then echo "Carrying on with the upload started earlier."; fi
  else
    solana-keygen new --no-bip39-passphrase --silent --outfile "$HOLDING" >/dev/null
  fi
  printf '%s' "$mark" > "$HOLDING.build"
  printf '%s' "$genesis" > "$HOLDING.net"   # which network this upload is on
  echo "Uploading. This takes a few minutes and shows nothing while it works. Do not close the window."
}
holding_done() { rm -f "$HOLDING" "$HOLDING.build" "$HOLDING.net"; }

# Reads the program's code back after a deploy or upgrade that has already
# succeeded. A slow server can answer with what was there a moment ago, so it
# tries a few times; and if it still cannot confirm, it says plainly that the
# deploy itself is done.   $1 = the program's address, $2 = "yes" for the main network
read_back() {
  echo "The program is on the network at $1. Reading it back ..."
  for attempt in 1 2 3 4 5 6; do
    sleep 3
    if node scripts/deploy/verify-code.mjs ${2:+--mainnet "$2"} > scripts/deploy/out/read-back.log 2>&1; then cat scripts/deploy/out/read-back.log; return 0; fi
  done
  cat scripts/deploy/out/read-back.log
  echo "" >&2
  echo "The upload itself finished and the program is in place; only the read-back could not confirm it just now." >&2
  echo "Do NOT run this command again. Wait a minute, then run:  node scripts/deploy/verify-code.mjs${2:+ --mainnet $2}" >&2
  exit 1
}

# Says what to do when an upload stops half way.
upload_stopped() {
  echo "" >&2
  echo "STOPPED: it did not finish. Nothing is lost." >&2
  echo "  To carry on: run the same command again. It picks up where it stopped." >&2
  echo "  To give up and get the deposit back: ./scripts/deploy/reclaim.sh --payer <the same key file>" >&2
  exit 1
}

# Prints "<upgrade key> <room>" for the program in the code; sets program_state to present, absent, or stops.
program_state() {
  set +e; shown=$(chain program "$1"); code=$?; set -e
  case "$code" in 0) state=present;; 3) state=absent;; *) echo "STOPPED: could not check whether a program is already at $1." >&2; exit 1;; esac
}
code_id() { grep -o 'declare_id!("[^"]*")' programs/burrpad/src/lib.rs | cut -d'"' -f2; }
