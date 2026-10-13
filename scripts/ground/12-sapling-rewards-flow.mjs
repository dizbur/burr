// Who receives the other launchpad's "rewards" pot, how often, how much, and
// what the receiver then does with it. Reading only; a handful of calls.
import { PublicKey } from "@solana/web3.js";
import { conn, ZEC, TOKEN, ata } from "./lib.mjs";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const pot = new PublicKey("2nv5wcB89CX4YW6VqdXoiZi5276Eqrt2i9inTSvgYXVD");
const sigs = (await conn.getSignaturesForAddress(pot, { limit: 200 })).filter((s) => !s.err);
console.log("transactions on the rewards pot fetched:", sigs.length, "| from slot", sigs.at(-1).slot, "to", sigs[0].slot);
const releases = [];
let settles = 0, checked = 0;
for (const s of sigs) {
  if (checked >= 45 || releases.length >= 8) break;
  await sleep(1200);
  checked++;
  const tx = await conn.getTransaction(s.signature, { maxSupportedTransactionVersion: 0 });
  const names = tx.meta.logMessages.filter((l) => l.includes("Instruction:")).map((l) => l.split("Instruction: ")[1]);
  if (names.includes("Settle")) { settles++; continue; }
  if (!names.includes("ReleaseRewards")) continue;
  const msg = tx.transaction.message;
  const keys = [...msg.staticAccountKeys, ...(tx.meta.loadedAddresses?.writable ?? []), ...(tx.meta.loadedAddresses?.readonly ?? [])].map((k) => k.toBase58());
  const pre = Object.fromEntries(tx.meta.preTokenBalances.map((b) => [b.accountIndex, b]));
  const got = tx.meta.postTokenBalances.filter((b) => b.mint === ZEC.toBase58())
    .map((b) => [b.owner, BigInt(b.uiTokenAmount.amount) - BigInt(pre[b.accountIndex]?.uiTokenAmount.amount ?? 0), BigInt(b.uiTokenAmount.amount)])
    .filter((m) => m[1] > 0n)[0];
  const potAfter = tx.meta.postTokenBalances.find((b) => keys[b.accountIndex] === pot.toBase58())?.uiTokenAmount.amount;
  const ix = msg.compiledInstructions.find((i) => i.data.length === 48);
  releases.push({ slot: s.slot, time: tx.blockTime, to: got?.[0], amount: got?.[1], potAfter, signers: keys.slice(0, msg.header.numRequiredSignatures),
    args: ix ? Buffer.from(ix.data.subarray(8)).toString("hex") : "" });
}
console.log("looked at", checked, "| Settle:", settles, "| ReleaseRewards:", releases.length);
for (const r of releases) console.log(new Date(r.time * 1000).toISOString().slice(0, 16), "| to", r.to, "| amount", String(r.amount), "| pot left", r.potAfter,
  "| second signer", r.signers[1] ?? "-", "| args", r.args);
const receivers = [...new Set(releases.map((r) => r.to))];
console.log("\ndistinct receivers:", receivers.length);
// What does one receiver do next with the ZEC?
if (receivers[0]) {
  const w = new PublicKey(receivers[0]);
  await sleep(1500);
  const info = await conn.getAccountInfo(w);
  console.log("receiver", w.toBase58(), "| is a wallet:", info?.owner.toBase58() === "11111111111111111111111111111111", "| SOL", (info?.lamports ?? 0) / 1e9);
  await sleep(1500);
  const zec = ata(w, ZEC, TOKEN);
  const bal = await conn.getTokenAccountBalance(zec).catch(() => null);
  console.log("its ZEC account balance now:", bal?.value.amount);
  await sleep(1500);
  const ws = (await conn.getSignaturesForAddress(zec, { limit: 12 })).filter((s) => !s.err);
  for (const s of ws.slice(0, 6)) {
    await sleep(1200);
    const tx = await conn.getTransaction(s.signature, { maxSupportedTransactionVersion: 0 });
    const names = [...new Set(tx.meta.logMessages.filter((l) => l.includes("Instruction:")).map((l) => l.split("Instruction: ")[1]))];
    const pre = Object.fromEntries(tx.meta.preTokenBalances.map((b) => [b.accountIndex, b]));
    const moves = tx.meta.postTokenBalances.filter((b) => b.mint === ZEC.toBase58())
      .map((b) => [b.owner.slice(0, 6), BigInt(b.uiTokenAmount.amount) - BigInt(pre[b.accountIndex]?.uiTokenAmount.amount ?? 0)]).filter((m) => m[1] !== 0n);
    console.log("  ", new Date(tx.blockTime * 1000).toISOString().slice(0, 16), names.slice(0, 5).join(","), "| ZEC moves:", moves.map((m) => `${m[0]}:${m[1]}`).join(" "));
  }
}
