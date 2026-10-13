// Create a ZEC coin and buy it with ZEC, in one simulated transaction, to see
// what the creator fee really is and what kind of coin comes out.
// (Sweeping and collecting as well does not fit in one transaction without an
// address table, which cannot be made without sending. Those steps are proven
// on the local copy of pump.fun's programs instead.)
import { Keypair, PublicKey } from "@solana/web3.js";
import { conn, ZEC, TOKEN, TOKEN_2022, ata, decode, show } from "./lib.mjs";
import { idl, createV2, simulate, curveOf, programControlledCreator, errName, createAta,
  buyExactQuoteInV3, sellV3, sweepCreatorFee, collectCreatorFeeV2, creatorVault, tokenAmount } from "./sim.mjs";

const trader = new PublicKey(process.argv[2]); // an existing wallet with SOL and ZEC; simulated only
const feeBps = Number(process.argv[3] ?? 200);
const spend = BigInt(process.argv[4] ?? 100_000_000); // 1 ZEC
const sellBack = BigInt(process.argv[5] ?? 0);
const mint = Keypair.generate().publicKey;
const { creator } = programControlledCreator(mint);
const curve = curveOf(mint);
const watch = [curve, ata(creator, ZEC, TOKEN), ata(creatorVault(creator), ZEC, TOKEN),
  ata(trader, mint, TOKEN_2022), ata(trader, ZEC, TOKEN), ata(curve, ZEC, TOKEN), mint];
const before = await conn.getTokenAccountBalance(ata(trader, ZEC, TOKEN));

const ixs = [
  createV2({ mint, user: trader, creator, feeBps }),
  createAta(trader, trader, mint, TOKEN_2022),
  buyExactQuoteInV3(mint, trader, spend),
  ...(sellBack ? [sellV3(mint, trader, sellBack)] : []),
];
const r = await simulate(trader, ixs, watch, false);
console.log("fee", feeBps, "spend", spend, "sellBack", sellBack, "| error:", JSON.stringify(r.err), "| units:", r.unitsConsumed);
const custom = r.err?.InstructionError?.[1]?.Custom;
if (custom !== undefined) console.log("error name:", errName(custom));
if (r.err) { console.log(r.logs.slice(-14).join("\n")); process.exit(0); }
console.log(r.logs.filter((l) => l.includes("Instruction:")).join(" | "));
const [c, creatorZec, vaultZec, traderCoin, traderZec, curveZec, mintAcct] = r.accounts;
const cur = decode(idl, "BondingCurve", Buffer.from(c.data[0], "base64"));
console.log("creator's ZEC account:", tokenAmount(creatorZec), "| pump creator vault left:", tokenAmount(vaultZec));
console.log("trader coin:", tokenAmount(traderCoin), "| trader ZEC before:", before.value.amount, "after:", tokenAmount(traderZec));
console.log("curve ZEC account:", tokenAmount(curveZec), "| curve real_quote_reserves:", cur.real_quote_reserves,
  "| creator_fee waiting:", cur.creator_fee, "| protocol_fees waiting:", cur.protocol_fees);
const md = Buffer.from(mintAcct.data[0], "base64");
console.log("mint owner:", mintAcct.owner, "len", md.length, "| mintAuthOpt", md.readUInt32LE(0), "supply", md.readBigUInt64LE(36),
  "decimals", md[44], "freezeOpt", md.readUInt32LE(46));
// Token-2022 extensions follow byte 166: type u16, length u16
let o = 166; const ext = [];
while (o + 4 <= md.length) { const t = md.readUInt16LE(o), l = md.readUInt16LE(o + 2); if (t === 0) break; ext.push(`${t}(len ${l})`); o += 4 + l; }
console.log("mint extensions (type ids):", ext.join(", "));
