// Question 1: can anyone create a ZEC-priced coin with a 2.00% creator fee
// and a program-controlled creator, with no approval? One simulation.
import { Keypair, PublicKey } from "@solana/web3.js";
import { conn, decode, show } from "./lib.mjs";
import { idl, createV2, simulate, curveOf, programControlledCreator, errName } from "./sim.mjs";

// An existing wallet with SOL, named only as the simulated payer. See decisions.md, 4.
const payer = new PublicKey(process.argv[2]);
const feeBps = process.argv[3] === "none" ? null : Number(process.argv[3] ?? 200);
const mint = Keypair.generate().publicKey; // throwaway, never funded, never saved
const { fakeProgram, creator } = programControlledCreator(mint);
const curve = curveOf(mint);

const r = await simulate(payer, [createV2({ mint, user: payer, creator, feeBps })], [curve]);
console.log("fee asked:", feeBps, "| creator (program-controlled):", creator.toBase58(), "| on curve:", PublicKey.isOnCurve(creator));
console.log("error:", JSON.stringify(r.err), "| units:", r.unitsConsumed);
const custom = r.err?.InstructionError?.[1]?.Custom;
if (custom !== undefined) console.log("error name:", errName(custom));
if (r.err) console.log(r.logs.slice(-8).join("\n"));
if (!r.err) {
  const data = Buffer.from(r.accounts[0].data[0], "base64");
  const c = decode(idl, "BondingCurve", data);
  console.log("curve owner after:", r.accounts[0].owner, "len", data.length);
  console.log(show(c));
}
