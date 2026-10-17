// Before the first coin priced in $BURR: makes the $BURR account of each of
// pump.fun's buy-back receivers (the eight on the curve's list, and those on
// PumpSwap's list). pump.fun refuses every $BURR-priced trade, ours included,
// until the receiver named in it has a $BURR account. Anyone may make an
// account for anyone; it then belongs to that receiver for good (only the
// receiver could close it). Running it again makes only what is missing.
//   node scripts/deploy/burr-buyback-accounts.mjs [--mainnet yes]
//       lists them, and what the missing ones cost; sends nothing
//   node scripts/deploy/burr-buyback-accounts.mjs --payer <deploy wallet key file> --send yes [--mainnet yes]
//       makes the missing ones, paid by that wallet
import { PublicKey, TransactionInstruction } from "@solana/web3.js";
import { ata, ATA_PROGRAM, connect, fail, loadKey, options, pda, PUMP, PUMP_AMM, send, SYSTEM, TOKEN_2022 } from "./lib.mjs";

const BURR = new PublicKey("GiwMfYS4jHi256G8K41snZhKUovrMbyQL2hqv3fqr7WU");
// Where each settings account lists its eight buy-back receivers (as the program reads them).
const LISTS = [
  ["pump.fun's curve settings", pda(["global"], PUMP), 741],
  ["PumpSwap's settings", pda(["global_config"], PUMP_AMM), 643],
];

const opts = options(["url", "mainnet", "payer", "send"]);
const conn = await connect(opts);
const receivers = new Map();
for (const [name, key, offset] of LISTS) {
  const info = await conn.getAccountInfo(key);
  if (!info || info.data.length < offset + 32 * 8) fail(`cannot read ${name}`);
  for (let i = 0; i < 8; i++) {
    const r = new PublicKey(info.data.subarray(offset + 32 * i, offset + 32 * i + 32));
    if (!r.equals(PublicKey.default)) receivers.set(r.toBase58(), r);
  }
}
const list = [...receivers.values()];
const accounts = list.map((r) => ata(r, BURR, TOKEN_2022));
const infos = await conn.getMultipleAccountsInfo(accounts);
const missing = list.filter((_, i) => !infos[i]);
// A $BURR account (Token-2022, with the immutable-owner mark the standard account carries): 170 bytes.
const each = await conn.getMinimumBalanceForRentExemption(170);
for (const [i, r] of list.entries()) console.log(`${infos[i] ? "has     " : "MISSING "} ${r.toBase58()}  ${accounts[i].toBase58()}`);
console.log(`\n${list.length} receivers, ${missing.length} without a $BURR account.`);
console.log(`Making them costs ${(missing.length * each / 1e9).toFixed(6)} SOL in rent (${each} lamports each), plus network fees; the rent stays with the accounts.`);
if (!missing.length) process.exit(0);
if (opts.send !== "yes" || !opts.payer) {
  console.log("Nothing sent. To make them: add --payer <deploy wallet key file> --send yes");
  process.exit(0);
}
const payer = loadKey(opts.payer, "--payer");
const ix = (owner) => new TransactionInstruction({
  programId: ATA_PROGRAM,
  data: Buffer.from([1]), // make it, or do nothing if it exists
  keys: [
    { pubkey: payer.publicKey, isSigner: true, isWritable: true },
    { pubkey: ata(owner, BURR, TOKEN_2022), isSigner: false, isWritable: true },
    { pubkey: owner, isSigner: false, isWritable: false },
    { pubkey: BURR, isSigner: false, isWritable: false },
    { pubkey: SYSTEM, isSigner: false, isWritable: false },
    { pubkey: TOKEN_2022, isSigner: false, isWritable: false },
  ],
});
for (let i = 0; i < missing.length; i += 4) {
  const { signature } = await send(conn, missing.slice(i, i + 4).map(ix), [payer], { units: 200_000 });
  console.log(`PASS  made ${missing.slice(i, i + 4).length}  (${signature})`);
}
const after = await conn.getMultipleAccountsInfo(accounts);
const still = after.filter((a) => !a).length;
console.log(still ? `FAIL  ${still} still missing` : "PASS  every buy-back receiver has a $BURR account");
if (still) process.exit(1);
