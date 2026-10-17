// For coins priced in $BURR: makes the second address table a $BURR launch
// needs (the first, frozen at deployment, has none of $BURR's addresses), fills
// it, and freezes it so nobody can ever change it. It holds $BURR's mint, its
// curve, its pool and the pool's two vaults (the four accounts a $BURR launch
// passes), and the $BURR accounts of pump.fun's buy-back receivers (curve and PumpSwap lists). Safe to run
// again: it carries on from where it stopped. Saved as `burr_table`.
//   node scripts/deploy/burr-table.mjs --payer <deploy wallet key file> [--table <address>] [--mainnet yes]
import { AddressLookupTableProgram, PublicKey } from "@solana/web3.js";
import { BURR, burrQuoteAccounts } from "./coin.mjs";
import { address, ata, checker, connect, fail, loadKey, need, options, pda, PUMP, PUMP_AMM, readOut, send, sleep, TOKEN_2022, writeOut } from "./lib.mjs";

const opts = options(["url", "payer", "table", "mainnet"]);
const conn = await connect(opts);
const payer = loadKey(need(opts, "payer"), "--payer");
const name = conn.outName;
// pump.fun's buy-back receivers: those on the curve's list and those on PumpSwap's (the same eight today),
// so that trades on a graduated $BURR coin's pool stay as small as on its curve.
const receivers = new Map();
for (const [key, offset] of [[pda(["global"], PUMP), 741], [pda(["global_config"], PUMP_AMM), 643]]) {
  const d = (await conn.getAccountInfo(key)).data;
  for (let i = 0; i < 8; i++) {
    const r = new PublicKey(d.subarray(offset + 32 * i, offset + 32 * i + 32));
    if (!r.equals(PublicKey.default)) receivers.set(r.toBase58(), r);
  }
}
const want = [BURR, ...burrQuoteAccounts(), ...[...receivers.values()].map((r) => ata(r, BURR, TOKEN_2022))];

let table = opts.table ? address(opts.table, "--table") : readOut(name).burr_table ? address(readOut(name).burr_table, "saved table") : null;
if (!table) {
  const slot = await conn.getSlot("finalized");
  const [create, addr] = AddressLookupTableProgram.createLookupTable({ authority: payer.publicKey, payer: payer.publicKey, recentSlot: slot });
  table = addr;
  writeOut(name, { burr_table: table.toBase58() }); // written down first, so a lost answer does not orphan it
  await send(conn, [create], [payer]);
  console.log("Table made: " + table.toBase58());
}
const state = async () => {
  for (let i = 0; i < 10; i++) {
    const got = (await conn.getAddressLookupTable(table)).value;
    if (got) return got.state;
    await sleep(1500);
  }
  fail("the table is not visible on the network yet. Run this again in a minute; it carries on.");
};
let s = await state();
const have = new Set(s.addresses.map((a) => a.toBase58()));
const missing = want.filter((a) => !have.has(a.toBase58()));
if (missing.length) {
  if (!s.authority) fail("the table is frozen but incomplete: remove burr_table from scripts/deploy/out/ for this network, and run again");
  await send(conn, [AddressLookupTableProgram.extendLookupTable({ lookupTable: table, authority: payer.publicKey, payer: payer.publicKey, addresses: missing })], [payer]);
  s = await state();
}
if (s.authority) {
  await send(conn, [AddressLookupTableProgram.freezeLookupTable({ lookupTable: table, authority: payer.publicKey })], [payer]);
  s = await state();
}
const t = checker();
const now = new Set(s.addresses.map((a) => a.toBase58()));
t.check("the table holds every address a $BURR launch and trade need", want.every((a) => now.has(a.toBase58())), `${want.length} addresses`);
t.check("the table is frozen: nobody can change it", !s.authority);
console.log(`BURR_ADDRESS_TABLE = ${table.toBase58()}`);
t.finish();
