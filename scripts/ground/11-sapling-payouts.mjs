// Follows the largest share of the other launchpad's Settle (40%) to see who
// controls it and how it is paid onward. Reading only; a handful of calls.
import { PublicKey } from "@solana/web3.js";
import { conn, ZEC } from "./lib.mjs";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const PROGRAM = "A9zFnNZpVjBbcE1SAPY92Z6H7WhicysLNPYPpDz15UxF";
const holders = ["FauKh2enCxjqMDaEEmo3qMmA9toxyZPFvPhGEZNMzChr", "3G6yQGLssh4VTNDaGpLn9CgARSZkXk2xRvfeu4f4SLA9",
  "7AE1g5Y7BXeHdvSkD4R53UFg9E2EcXdivMZZV7Av4Qar", "A761EcsjGa5zwoZ2ZQwC5hm8pcfeCosxwEs2YS3raW42"].map((k) => new PublicKey(k));
const infos = await conn.getMultipleAccountsInfo(holders);
holders.forEach((k, i) => console.log(["10%", "25%", "40%", "25%"][i], k.toBase58(), "| on curve (a wallet key):", PublicKey.isOnCurve(k),
  "|", infos[i] ? `owner ${infos[i].owner.toBase58()} data ${infos[i].data.length} bytes` : "no account"));

const pot = new PublicKey("2nv5wcB89CX4YW6VqdXoiZi5276Eqrt2i9inTSvgYXVD"); // the 40% ZEC account
await sleep(1500);
const sigs = (await conn.getSignaturesForAddress(pot, { limit: 40 })).filter((s) => !s.err);
let shown = 0;
for (const s of sigs) {
  if (shown >= 3) break;
  await sleep(1500);
  const tx = await conn.getTransaction(s.signature, { maxSupportedTransactionVersion: 0 });
  const msg = tx.transaction.message;
  const keys = [...msg.staticAccountKeys, ...(tx.meta.loadedAddresses?.writable ?? []), ...(tx.meta.loadedAddresses?.readonly ?? [])].map((k) => k.toBase58());
  const names = tx.meta.logMessages.filter((l) => l.includes("Instruction:")).map((l) => l.split("Instruction: ")[1]);
  const pre = Object.fromEntries(tx.meta.preTokenBalances.map((b) => [b.accountIndex, b]));
  const moves = tx.meta.postTokenBalances.filter((b) => b.mint === ZEC.toBase58())
    .map((b) => [keys[b.accountIndex], b.owner, BigInt(b.uiTokenAmount.amount) - BigInt(pre[b.accountIndex]?.uiTokenAmount.amount ?? 0)])
    .filter((m) => m[2] !== 0n);
  const out = moves.find((m) => m[0] === pot.toBase58());
  if (!out || out[2] >= 0n) continue; // only transactions that pay out of the pot
  shown++;
  const nsig = msg.header.numRequiredSignatures;
  console.log(`\npayout tx ${s.signature.slice(0, 16)}… slot ${s.slot}`);
  console.log("  signers:", keys.slice(0, nsig).join(", "));
  console.log("  programs called:", [...new Set(msg.compiledInstructions.map((i) => keys[i.programIdIndex]))].join(", "));
  console.log("  instruction names:", [...new Set(names)].join(" | "));
  for (const ix of msg.compiledInstructions) {
    if (keys[ix.programIdIndex] !== PROGRAM) continue;
    console.log("  our-kind instruction: data", ix.data.length, "bytes, first 8:", Buffer.from(ix.data.subarray(0, 8)).toString("hex"),
      "| signer among its accounts:", ix.accountKeyIndexes.some((k) => k < nsig));
  }
  console.log("  ZEC moves:", moves.length, "| out of the pot:", out[2].toString(), "| receivers:", moves.filter((m) => m[2] > 0n).length);
  moves.filter((m) => m[2] > 0n).slice(0, 4).forEach((m) => console.log("    +" + m[2], "to an account owned by", m[1]));
}
if (!shown) console.log("\nno payout out of the 40% account in its last", sigs.length, "transactions");
