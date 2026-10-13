#!/bin/bash
# Writes the program's real address into the code, in the two places it lives.
# Run once, before building, with the ADDRESS (not the key file) of the
# program key you made. Commit the change afterwards.
#   ./scripts/deploy/set-program-id.sh <program address>
set -eu
cd "$(dirname "$0")/../.."
[ $# -eq 1 ] || { echo "Give the program's address." >&2; exit 1; }
id="$1"
echo "$id" | grep -Eq '^[1-9A-HJ-NP-Za-km-z]{32,44}$' || { echo "That is not an address." >&2; exit 1; }
perl -pi -e "s/declare_id!\\(\"[^\"]+\"\\)/declare_id!(\"$id\")/" programs/burrpad/src/lib.rs
perl -pi -e "s/^burrpad = \"[^\"]+\"/burrpad = \"$id\"/" Anchor.toml
grep -q "declare_id!(\"$id\")" programs/burrpad/src/lib.rs && grep -q "^burrpad = \"$id\"" Anchor.toml \
  || { echo "STOPPED: could not write the address." >&2; exit 1; }
echo "PASS  program address written: $id"
echo "Next: commit this change, then run scripts/deploy/build.sh"
