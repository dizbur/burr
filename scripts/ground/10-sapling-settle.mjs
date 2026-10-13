// Reads how the other launchpad's "Settle" instruction is authorised: who
// signs it, and where the money goes. Reading only; a handful of calls.
import { PublicKey } from "@solana/web3.js";
import { conn, ZEC, TOKEN, ata } from "./lib.mjs";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const PROGRAM = new PublicKey("A9zFnNZpVjBbcE1SAPY92Z6H7WhicysLNPYPpDz15UxF");
const creator = new PublicKey("8gM6Sp1wRsiLjQyKuCLnFRWVNmHt3Az63eYkJdeFKcc9");
const creatorZec = ata(creator, ZEC, TOKEN);

// Who can upgrade the program.
const prog = await conn.getAccountInfo(PROGRAM);
const pd = new PublicKey(prog.data.subarray(4, 36));
const pdi = await conn.getAccountInfo(pd, { dataSlice: { offset: 0, length: 45 } });
console.log("program owner:", prog.owner.toBase58(), "| upgrade authority:",
  pdi.data[12] ? new PublicKey(pdi.data.subarray(13, 45)).toBase58() : "none");

await sleep(1500);
const sigs = (await conn.getSignaturesForAddress(creatorZec, { limit: 25 })).filter((s) => !s.err);
const seen = new Map();
let shown = 0;
for (const s of sigs) {
  if (shown >= 4) break;
  await sleep(1500);
  const tx = await conn.getTransaction(s.signature, { maxSupportedTransactionVersion: 0 });
  const msg = tx.transaction.message;
  const keys = [...msg.staticAccountKeys, ...(tx.meta.loadedAddresses?.writable ?? []), ...(tx.meta.loadedAddresses?.readonly ?? [])].map((k) => k.toBase58());
  const names = tx.meta.logMessages.filter((l) => l.includes("Instruction:")).map((l) => l.split("Instruction: ")[1]);
  if (!names.includes("Settle")) continue;
  shown++;
  const nsig = msg.header.numRequiredSignatures;
  const signers = keys.slice(0, nsig);
  signers.forEach((k) => seen.set(k, (seen.get(k) ?? 0) + 1));
  console.log(`\nSettle tx ${s.signature.slice(0, 16)}… slot ${s.slot}`);
  console.log("  signers:", signers.join(", "));
  for (const ix of msg.compiledInstructions) {
    if (keys[ix.programIdIndex] !== PROGRAM.toBase58()) continue;
    console.log("  data bytes:", ix.data.length, "| first 8:", Buffer.from(ix.data.subarray(0, 8)).toString("hex"),
      "| rest:", Buffer.from(ix.data.subarray(8, 72)).toString("hex"));
    console.log("  accounts:", ix.accountKeyIndexes.length);
    ix.accountKeyIndexes.forEach((k, i) => console.log(`    ${i} ${keys[k]}${k < nsig ? "  (signer)" : ""}`));
  }
  const pre = Object.fromEntries(tx.meta.preTokenBalances.map((b) => [b.accountIndex, b]));
  for (const b of tx.meta.postTokenBalances) {
    if (b.mint !== ZEC.toBase58()) continue;
    const before = BigInt(pre[b.accountIndex]?.uiTokenAmount.amount ?? 0);
    const delta = BigInt(b.uiTokenAmount.amount) - before;
    if (delta !== 0n) console.log(`  ZEC ${delta > 0n ? "+" : ""}${delta} -> account ${keys[b.accountIndex].slice(0, 8)}… owned by ${b.owner}`);
  }
  console.log("  inner instruction names:", names.join(" | "));
}
console.log("\nsigners seen across these Settle calls:", JSON.stringify([...seen]));
