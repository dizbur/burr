// Finds the coin behind the known Sapling curve and a wallet that traded it,
// to name as the simulated trader. A few calls. Nothing is asked of that wallet.
import { PublicKey } from "@solana/web3.js";
import { conn, ZEC, TOKEN, ata } from "./lib.mjs";
const curve = new PublicKey("5R41L1P8EhAmikGMYKs35R7Eoyi7S4uJLa9wuZy3f4AF");
const sigs = await conn.getSignaturesForAddress(curve, { limit: 8 });
console.log(sigs.map((s) => `${s.signature.slice(0, 12)} slot ${s.slot} err ${JSON.stringify(s.err)}`).join("\n"));
const ok = sigs; // failed ones still name the coin and a trader
for (const s of [ok[0]]) {
  const tx = await conn.getTransaction(s.signature, { maxSupportedTransactionVersion: 0 });
  const keys = tx.transaction.message.staticAccountKeys.map((k) => k.toBase58());
  console.log("\ntx", s.signature, "\npayer", keys[0]);
  console.log("logs:", tx.meta.logMessages.filter((l) => l.includes("Instruction:")).join(" | "));
  console.log("accounts:", keys.join(" "), (tx.meta.loadedAddresses?.writable ?? []).concat(tx.meta.loadedAddresses?.readonly ?? []).map(String).join(" "));
  console.log("token balances:", JSON.stringify(tx.meta.postTokenBalances.map((b) => [b.mint.slice(0, 6), b.owner?.slice(0, 6), b.uiTokenAmount.uiAmountString, b.programId?.slice(0, 6)])));
  console.log("mints seen:", [...new Set(tx.meta.postTokenBalances.map((b) => b.mint))].join(" "));
  await new Promise((r) => setTimeout(r, 1500));
}
