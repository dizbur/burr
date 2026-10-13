# Shared by the shell scripts: the network's address, the main-network guard,
# and the check for a rule that would refuse this program's format.
# The address comes from BURRPAD_RPC_URL (or --url) and is never printed.
export PATH="$HOME/.cargo/bin:$HOME/.local/share/solana/install/active_release/bin:$PATH"
MAINNET_GENESIS="5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"
OLD_FORMAT_RULE="B8JJXCy5amZyWG9r7EnUYLwzXSXTxG7GZ1qZ1qggo83g"

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

# Stops unless the wallet behind key file $2 holds enough SOL for a program file of the size built. $1 = url.
balance_check() {
  size=$(wc -c < target/deploy/burrpad.so | tr -d ' ')
  need=$(solana rent "$((size + 45))" --url "$1" | head -1 | awk '{print $3}')
  have=$(solana balance "$(solana-keygen pubkey "$2")" --url "$1" | awk '{print $1}')
  echo "The program file is $size bytes. Holding it on the network takes $need SOL; this wallet has $have SOL."
  awk -v h="$have" -v n="$need" 'BEGIN { exit !(h >= n + 0.05) }' || { echo "STOPPED: not enough SOL. It needs $need plus about 0.05 for fees." >&2; exit 1; }
}
code_id() { grep -o 'declare_id!("[^"]*")' programs/burrpad/src/lib.rs | cut -d'"' -f2; }
