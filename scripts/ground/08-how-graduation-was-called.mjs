// Reads the real transaction that graduated the sample Sapling coin, to learn
// which extra accounts pump.fun's migration takes. Two or three calls.
import { PublicKey } from "@solana/web3.js";
import { conn, PUMP, PUMP_AMM, ZEC, pda, loadIdl } from "./lib.mjs";

const mint = new PublicKey("8TUn4HHBybSR4oatWshpeM7i1keT5esWCZE6zc8EB1uC");
const pool = pda(["pool", Buffer.from([0, 0]), pda(["pool-authority", mint], PUMP), mint, ZEC], PUMP_AMM);
const lpMint = pda(["pool_lp_mint", pool], PUMP_AMM);
const sigs = await conn.getSignaturesForAddress(lpMint, { limit: 20 });
const first = sigs[sigs.length - 1];
console.log("signatures on the pool's share mint:", sigs.length, "| oldest:", first.signature, "slot", first.slot);
await new Promise((r) => setTimeout(r, 1500));
const tx = await conn.getTransaction(first.signature, { maxSupportedTransactionVersion: 0 });
const msg = tx.transaction.message;
const keys = [...msg.staticAccountKeys, ...(tx.meta.loadedAddresses?.writable ?? []), ...(tx.meta.loadedAddresses?.readonly ?? [])].map((k) => k.toBase58());
const idl = loadIdl("pump.json");
const names = Object.fromEntries(idl.instructions.map((i) => [Buffer.from(i.discriminator).toString("hex"), i]));
for (const ix of msg.compiledInstructions) {
  const program = keys[ix.programIdIndex];
  if (program !== PUMP.toBase58()) { console.log("-", program); continue; }
  const def = names[Buffer.from(ix.data.subarray(0, 8)).toString("hex")];
  console.log("- pump", def?.name, "accounts:", ix.accountKeyIndexes.length, "of which named:", def?.accounts.length);
  ix.accountKeyIndexes.forEach((k, i) => console.log(`   ${String(i + 1).padStart(2)} ${(def?.accounts[i]?.name ?? "EXTRA").padEnd(34)} ${keys[k]}`));
}
console.log(tx.meta.logMessages.filter((l) => l.includes("Instruction:")).join(" | "));
