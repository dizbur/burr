// Small reads from the network for the shell scripts, so they never have to
// pick numbers out of another tool's printed text. Sends nothing.
// The network's address comes from BURRPAD_RPC_URL and is never printed.
//   node chain.mjs genesis                 the network's genesis hash
//   node chain.mjs rent <bytes>            SOL the network holds for an account of that size
//   node chain.mjs balance <address>       SOL at an address
//   node chain.mjs program <address>       "<upgrade key> <bytes of room>"; exit 3 if no program is there
//   node chain.mjs rule <address>          "active" or "inactive" for a network rule (feature)
// Exit 2 means the network could not be read; the caller must stop, not guess.
import { Connection, PublicKey } from "@solana/web3.js";
import { LOADER, pda } from "./lib.mjs";

const [what, arg] = process.argv.slice(2);
const sol = (lamports) => (lamports / 1e9).toFixed(9);
try {
  if (!process.env.BURRPAD_RPC_URL) throw new Error("no network");
  const conn = new Connection(process.env.BURRPAD_RPC_URL, "confirmed");
  if (what === "genesis") console.log(await conn.getGenesisHash());
  else if (what === "rent") console.log(sol(await conn.getMinimumBalanceForRentExemption(Number(arg))));
  else if (what === "balance") console.log(sol(await conn.getBalance(new PublicKey(arg))));
  else if (what === "rule") {
    const info = await conn.getAccountInfo(new PublicKey(arg));
    // A feature account holds one byte saying whether it has been switched on, then the slot.
    console.log(info && info.data.length >= 9 && info.data[0] === 1 ? "active" : "inactive");
  } else if (what === "program") {
    const id = new PublicKey(arg);
    const [program, data] = await Promise.all([conn.getAccountInfo(id), conn.getAccountInfo(pda([id], LOADER))]);
    if (!program && !data) process.exit(3);
    if (!program?.executable || !program.owner.equals(LOADER) || !data) {
      console.error("STOPPED: something other than a deployed program is at that address.");
      process.exit(2);
    }
    const authority = data.data[12] === 1 ? new PublicKey(data.data.subarray(13, 45)).toBase58() : "none";
    console.log(`${authority} ${data.data.length - 45}`);
  } else throw new Error("unknown request");
} catch {
  console.error("STOPPED: the network could not be read.");
  process.exit(2);
}
