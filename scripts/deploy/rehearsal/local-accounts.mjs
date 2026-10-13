// Rehearsal only. Turns the copies in fixtures/ (pump.fun's programs and
// accounts, fetched by scripts/ground/07-fetch-fixtures.mjs) into what a
// local validator loads at start, and makes a ZEC balance for the test trader.
//   node local-accounts.mjs <fixtures dir> <output dir> <trader address>
// Prints the validator's arguments, one per line.
import { PublicKey } from "@solana/web3.js";
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { ata, TOKEN, ZEC } from "../lib.mjs";

const [fixtures, out, traderText] = process.argv.slice(2);
mkdirSync(out, { recursive: true });
const all = Object.fromEntries(readdirSync(fixtures).filter((f) => f.endsWith(".json"))
  .map((f) => [f.slice(0, -5), JSON.parse(readFileSync(path.join(fixtures, f), "utf8"))]));
const args = [];
const writeAccount = (name, pubkey, owner, lamports, data) => {
  const file = path.join(out, name + ".json");
  // rentEpoch is the largest 64-bit number; written as text because JSON numbers in JS cannot hold it.
  writeFileSync(file, `{"pubkey":"${pubkey}","account":{"lamports":${lamports},"data":["${data.toString("base64")}","base64"],` +
    `"owner":"${owner}","executable":false,"rentEpoch":18446744073709551615,"space":${data.length}}}\n`);
  args.push("--account", pubkey, file);
};
for (const [name, f] of Object.entries(all)) {
  if (name.startsWith("programdata_")) continue;
  if (name.startsWith("program_")) {
    // An upgradeable program's code sits in its data account after a 45-byte header.
    const code = Buffer.from(all["programdata_" + name.slice(8)].data, "base64").subarray(45);
    const file = path.join(out, name + ".so");
    writeFileSync(file, code);
    args.push("--upgradeable-program", f.pubkey, file, "none");
    continue;
  }
  writeAccount(name, f.pubkey, f.owner, f.lamports, Buffer.from(f.data, "base64"));
}
// 1,000 test ZEC for the trader: a token account written directly. It exists only on the local validator.
const trader = new PublicKey(traderText);
const token = Buffer.alloc(165);
ZEC.toBuffer().copy(token, 0);
trader.toBuffer().copy(token, 32);
token.writeBigUInt64LE(1000n * 100_000_000n, 64);
token[108] = 1; // initialised
writeAccount("trader_zec", ata(trader, ZEC, TOKEN).toBase58(), TOKEN.toBase58(), 2039280, token);
console.log(args.join("\n"));
