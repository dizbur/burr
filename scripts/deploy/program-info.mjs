// Prints "<upgrade key> <bytes of room for code>" for the program at the
// address in the code, or exits 1 if no program is there. Used by the shell scripts.
//   node scripts/deploy/program-info.mjs <program address> [--url <network>]
import { Connection, PublicKey } from "@solana/web3.js";
import { LOADER, pda } from "./lib.mjs";

const id = new PublicKey(process.argv[2]);
const at = process.argv.indexOf("--url");
const conn = new Connection(at > 0 ? process.argv[at + 1] : process.env.BURRPAD_RPC_URL, "confirmed");
const data = await conn.getAccountInfo(pda([id], LOADER));
if (!data || !(await conn.getAccountInfo(id))) process.exit(1);
const authority = data.data[12] === 1 ? new PublicKey(data.data.subarray(13, 45)).toBase58() : "none";
console.log(`${authority} ${data.data.length - 45}`);
