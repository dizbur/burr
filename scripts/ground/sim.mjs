// Builds pump.fun instructions and simulates them. Simulation only:
// signature checking is off, nothing is signed, nothing is sent.
import {
  PublicKey, Keypair, TransactionInstruction, TransactionMessage,
  VersionedTransaction, ComputeBudgetProgram, SystemProgram,
} from "@solana/web3.js";
import { conn, PUMP, MAYHEM, ZEC, TOKEN, TOKEN_2022, ATA_PROGRAM, pda, ata, loadIdl } from "./lib.mjs";

export const idl = loadIdl("pump.json");
export const disc = (name, from = idl) =>
  Buffer.from(from.instructions.find((i) => i.name === name).discriminator);
export const errName = (code, from = idl) =>
  from.errors.find((e) => e.code === code)?.name ?? null;

const str = (s) => { const b = Buffer.from(s); const n = Buffer.alloc(4); n.writeUInt32LE(b.length); return Buffer.concat([n, b]); };
export const u64 = (v) => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(v)); return b; };
const m = (pubkey, isWritable = false, isSigner = false) => ({ pubkey, isWritable, isSigner });

export const curveOf = (mint) => pda(["bonding-curve", mint], PUMP);

// create_v2 for a ZEC-priced coin. `feeBps` null leaves the argument off.
export function createV2({ mint, user, creator, feeBps, quoteMint = ZEC, quoteProgram = TOKEN, withQuoteControl = true }) {
  const curve = curveOf(mint);
  const solVault = pda(["sol-vault"], MAYHEM);
  const keys = [
    m(mint, true, true),
    m(pda(["mint-authority"], PUMP)),
    m(curve, true),
    m(ata(curve, mint, TOKEN_2022), true),
    m(pda(["global"], PUMP)),
    m(user, true, true),
    m(SystemProgram.programId),
    m(TOKEN_2022),
    m(ATA_PROGRAM),
    m(MAYHEM, true),
    m(pda(["global-params"], MAYHEM)),
    m(solVault, true),
    m(pda(["mayhem-state", mint], MAYHEM), true),
    m(ata(solVault, mint, TOKEN_2022), true),
    m(pda(["__event_authority"], PUMP)),
    m(PUMP),
    // optional accounts that choose the quote coin
    m(quoteMint),
    m(ata(curve, quoteMint, quoteProgram), true),
    m(quoteProgram),
  ];
  if (withQuoteControl) keys.push(m(pda(["quote-control"], PUMP)));
  const parts = [disc("create_v2"), str("T"), str("T"), str("u"),
    creator.toBuffer(), Buffer.from([0]), Buffer.from([0])];
  if (feeBps !== null) parts.push(u64(feeBps), Buffer.from([0]));
  return new TransactionInstruction({ programId: PUMP, keys, data: Buffer.concat(parts) });
}

export async function simulate(payer, ixs, watch = [], budget = true) {
  const msg = new TransactionMessage({
    payerKey: payer,
    recentBlockhash: "11111111111111111111111111111111",
    instructions: budget ? [ComputeBudgetProgram.setComputeUnitLimit({ units: 1_000_000 }), ...ixs] : ixs,
  }).compileToV0Message();
  const tx = new VersionedTransaction(msg);
  const res = await conn.simulateTransaction(tx, {
    sigVerify: false, replaceRecentBlockhash: true, commitment: "confirmed",
    accounts: { encoding: "base64", addresses: watch.map((k) => k.toBase58()) },
  });
  return res.value;
}

// An address a program of ours would control: derived from a program id, no key behind it.
export function programControlledCreator(mint) {
  const fakeProgram = Keypair.generate().publicKey;
  return { fakeProgram, creator: pda(["coin", mint], fakeProgram) };
}

// ---- trade, sweep and collect on the curve ----
import { PUMP_FEES } from "./lib.mjs";
export const BUYBACK_WALLET = new PublicKey("9M4giFFMxmFGXtc3feFzRai56WbBqehoSeRE5GK7gf7");
const optBool = (v) => Buffer.from([v ? 1 : 0]);

export const createAta = (payer, owner, mint, tokenProgram) =>
  new TransactionInstruction({
    programId: ATA_PROGRAM,
    keys: [m(payer, true, true), m(ata(owner, mint, tokenProgram), true), m(owner), m(mint),
      m(SystemProgram.programId), m(tokenProgram)],
    data: Buffer.from([1]), // create, or do nothing if it exists
  });

function tradeV3Keys(mint, user) {
  const curve = curveOf(mint);
  return [
    m(pda(["global"], PUMP)), m(mint), m(ZEC), m(TOKEN_2022), m(TOKEN),
    m(curve, true), m(ata(curve, mint, TOKEN_2022), true), m(ata(curve, ZEC, TOKEN), true),
    m(user, true, true), m(ata(user, mint, TOKEN_2022), true), m(ata(user, ZEC, TOKEN), true),
    m(pda(["user_volume_accumulator", user], PUMP), true),
    m(pda(["fee_config", PUMP], PUMP_FEES)),
    m(ata(BUYBACK_WALLET, ZEC, TOKEN), true),
    m(SystemProgram.programId), m(pda(["__event_authority"], PUMP)), m(PUMP),
  ];
}
export const buyExactQuoteInV3 = (mint, user, quoteIn, minOut = 1) =>
  new TransactionInstruction({ programId: PUMP, keys: tradeV3Keys(mint, user),
    data: Buffer.concat([disc("buy_exact_quote_in_v3"), u64(quoteIn), u64(minOut), optBool(false)]) });
export const sellV3 = (mint, user, amount, minOut = 0) =>
  new TransactionInstruction({ programId: PUMP, keys: tradeV3Keys(mint, user),
    data: Buffer.concat([disc("sell_v3"), u64(amount), u64(minOut)]) });

export const creatorVault = (creator) => pda(["creator-vault", creator], PUMP);

export function sweepCreatorFee(mint, payer, creator) {
  const curve = curveOf(mint), vault = creatorVault(creator);
  return new TransactionInstruction({ programId: PUMP, data: disc("sweep_creator_fee"), keys: [
    m(payer, true, true), m(pda(["global"], PUMP)), m(mint), m(ZEC), m(TOKEN), m(ATA_PROGRAM),
    m(SystemProgram.programId), m(curve, true), m(ata(curve, ZEC, TOKEN), true),
    m(vault, true), m(ata(vault, ZEC, TOKEN), true), m(pda(["__event_authority"], PUMP)), m(PUMP)] });
}
export function collectCreatorFeeV2(creator) {
  const vault = creatorVault(creator);
  return new TransactionInstruction({ programId: PUMP, data: disc("collect_creator_fee_v2"), keys: [
    m(creator, true), m(ata(creator, ZEC, TOKEN), true), m(vault, true), m(ata(vault, ZEC, TOKEN), true),
    m(ZEC), m(TOKEN), m(ATA_PROGRAM), m(SystemProgram.programId), m(pda(["__event_authority"], PUMP)), m(PUMP)] });
}
export const tokenAmount = (acct) =>
  acct ? Buffer.from(acct.data[0], "base64").readBigUInt64LE(64) : null;
