// Can a stranger call pump.fun's `set_metaplex_creator`, which takes no
// signer, on a ZEC coin made by create_v2? Simulation only. Nothing is sent.
import { PublicKey, TransactionInstruction } from "@solana/web3.js";
import { PUMP, pda } from "./lib.mjs";
import { idl, disc, simulate, errName } from "./sim.mjs";

const payer = new PublicKey(process.argv[2]); // an existing funded wallet, named only as simulated payer
const mint = new PublicKey("8TUn4HHBybSR4oatWshpeM7i1keT5esWCZE6zc8EB1uC"); // the sample ZEC coin
const METAPLEX = new PublicKey("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s");
const curve = pda(["bonding-curve", mint], PUMP);
const m = (pubkey, isWritable = false) => ({ pubkey, isWritable, isSigner: false });
const ix = new TransactionInstruction({
  programId: PUMP,
  data: disc("set_metaplex_creator"),
  keys: [m(mint), m(pda(["metadata", METAPLEX, mint], METAPLEX)), m(curve, true), m(pda(["__event_authority"], PUMP)), m(PUMP)],
});
const r = await simulate(payer, [ix], [curve]);
console.log("error:", JSON.stringify(r.err));
const code = r.err?.InstructionError?.[1]?.Custom;
if (code !== undefined) console.log("error name:", errName(code) ?? "(not one of pump.fun's own; an Anchor account error)");
console.log(r.logs.slice(-6).join("\n"));
if (!r.err) console.log("creator after:", new PublicKey(Buffer.from(r.accounts[0].data[0], "base64").subarray(49, 81)).toBase58());
