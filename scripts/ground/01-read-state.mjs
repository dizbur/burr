// Reads pump.fun's settings and one existing ZEC coin. One call.
import { PublicKey } from "@solana/web3.js";
import { conn, PUMP, ZEC, pda, loadIdl, decode, show } from "./lib.mjs";

const idl = loadIdl("pump.json");
const global = pda(["global"], PUMP);
const quoteControl = pda(["quote-control"], PUMP);
const saplingCurve = new PublicKey("5R41L1P8EhAmikGMYKs35R7Eoyi7S4uJLa9wuZy3f4AF");

const [g, qc, bc, zec] = await conn.getMultipleAccountsInfo([global, quoteControl, saplingCurve, ZEC]);
console.log("slot", await conn.getSlot());
console.log("global", global.toBase58(), "owner", g.owner.toBase58(), "len", g.data.length);
console.log(show(decode(idl, "Global", g.data)));
console.log("quote-control", quoteControl.toBase58(), qc ? `owner ${qc.owner.toBase58()} len ${qc.data.length}` : "MISSING");
if (qc) console.log(show(decode(idl, "QuoteControl", qc.data)));
console.log("sapling curve owner", bc.owner.toBase58(), "len", bc.data.length);
console.log(show(decode(idl, "BondingCurve", bc.data)));
console.log("ZEC mint owner", zec.owner.toBase58(), "len", zec.data.length,
  "decimals", zec.data[44], "mintAuthOpt", zec.data.readUInt32LE(0), "freezeAuthOpt", zec.data.readUInt32LE(46),
  "freezeAuth", new PublicKey(zec.data.subarray(50, 82)).toBase58(),
  "mintAuth", new PublicKey(zec.data.subarray(4, 36)).toBase58());
