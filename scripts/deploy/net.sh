# Shared by the shell scripts: the network's address, the main-network guard,
# and the check for a rule that would refuse this program's format.
# The address comes from BURRPAD_RPC_URL (or --url) and is never printed.
export PATH="$HOME/.cargo/bin:$HOME/.local/share/solana/install/active_release/bin:$PATH"
MAINNET_GENESIS="5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"
OLD_FORMAT_RULE="B8JJXCy5amZyWG9r7EnUYLwzXSXTxG7GZ1qZ1qggo83g"
HOLDING="scripts/deploy/out/upload-holding-keypair.json"

net_check() { # $1 = url, $2 = "yes" if --mainnet yes was given
  [ -n "$1" ] || { echo "STOPPED: no network given. Set BURRPAD_RPC_URL in this terminal (export BURRPAD_RPC_URL=...), or pass --url." >&2; exit 1; }
  genesis=$(solana genesis-hash --url "$1" 2>/dev/null) || { echo "STOPPED: the network did not answer." >&2; exit 1; }
  if [ "$genesis" = "$MAINNET_GENESIS" ]; then
    [ "$2" = "yes" ] || { echo "STOPPED: this is the main network. Add  --mainnet yes  if that is what you mean." >&2; exit 1; }
    echo "Network: MAIN NETWORK"
  else
    echo "Network: a test network"
  fi
  # The network plans a rule (SIMD-0500) that will refuse to deploy programs
  # in the format this one is built in. Stop clearly rather than half way.
  status=$(solana feature status "$OLD_FORMAT_RULE" --url "$1" 2>/dev/null | tail -1 || true)
  case "$status" in
    *inactive*|*pending*|"") ;;
    *active*)
      echo "STOPPED: this network no longer accepts programs in the format this one is built in." >&2
      echo "It must be rebuilt in the newer format (cargo build-sbf --arch v3) and the whole test suite run on that build first." >&2
      exit 1;;
  esac
}

# Works out what the upload needs from the network's own rent figures for the
# file just built, prints it, and stops if the wallet behind key file $2 is
# short, saying by how much. Nothing has been sent when this runs.
#   $1 = url, $2 = key file, $3 = "deploy" or "upgrade"
balance_check() {
  size=$(wc -c < target/release-kit/burrpad.so | tr -d ' ')
  rent_of() { solana rent "$1" --url "$2" | head -1 | awk '{print $3}'; }
  data=$(rent_of "$((size + 45))" "$1")      # the program's code, with its header
  account=$(rent_of 36 "$1")                 # the small account at the program's address
  uploads=$(( size / 1000 + 5 ))             # one transaction per thousand bytes or so
  fees=$(awk -v n="$uploads" 'BEGIN { printf "%.4f", n * 0.00001 }')
  margin=0.02
  have=$(solana balance "$(solana-keygen pubkey "$2")" --url "$1" | awk '{print $1}')
  held=0
  if [ -f "$HOLDING" ]; then held=$(solana balance "$(solana-keygen pubkey "$HOLDING")" --url "$1" 2>/dev/null | awk '{print $1}'); fi
  if awk -v h="${held:-0}" 'BEGIN { exit !(h > 0) }'; then
    # An earlier upload already paid its deposit; only fees are needed to carry on.
    need=$(awk -v b="$account" -v c="$fees" -v d="$margin" 'BEGIN { printf "%.4f", b + c + d }')
    echo "An upload started earlier already holds $held SOL of deposit."
    echo "  needed to carry on:               $need SOL"
  elif [ "$3" = "deploy" ]; then
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
    echo "  held while the new code is uploaded, returned when the upgrade lands: $data SOL"
    echo "  upload fees, about:               $fees SOL"
    echo "  spare:                            $margin SOL"
    echo "  needed to start:                  $need SOL"
  fi
  echo "  this wallet has:                  $have SOL"
  awk -v h="$have" -v n="$need" 'BEGIN { exit !(h >= n) }' || {
    echo "STOPPED: not enough SOL. Short by $(awk -v h="$have" -v n="$need" 'BEGIN { printf "%.4f", n - h }') SOL. Nothing was sent." >&2; exit 1; }
}

# The upload goes through a holding account. Its key file is kept until the
# upload is done, so that running the same command again carries on from
# where it stopped instead of paying a second deposit. It is not one of your
# keys and holds nothing once the upload has landed or been reclaimed.
holding_key() {
  mkdir -p scripts/deploy/out
  if [ -f "$HOLDING" ]; then echo "Carrying on with the upload started earlier."; else
    solana-keygen new --no-bip39-passphrase --silent --outfile "$HOLDING" >/dev/null; fi
}

code_id() { grep -o 'declare_id!("[^"]*")' programs/burrpad/src/lib.rs | cut -d'"' -f2; }
