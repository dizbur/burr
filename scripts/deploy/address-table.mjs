// Makes the address table a launch needs, fills it, and freezes it so nobody
// can ever change it. Safe to run again: it carries on from where it stopped.
//   node scripts/deploy/address-table.mjs --url <network> --payer <file> [--table <address>] [--mainnet yes]
import { AddressLookupTableProgram } from "@solana/web3.js";
import { address, checker, connect, fixedAccounts, loadIdl, loadKey, need, options, readOut, send, sleep, writeOut } from "./lib.mjs";

const opts = options(["url", "payer", "table", "mainnet"]);
const conn = await connect(opts);
const idl = loadIdl();
const payer = loadKey(need(opts, "payer"), "--payer");
const name = conn.outName;
const want = fixedAccounts(idl);

let table = opts.table ? address(opts.table, "--table") : readOut(name).table ? address(readOut(name).table, "saved table") : null;
if (table && !(await conn.getAddressLookupTable(table)).value) table = null;
if (!table) {
  const slot = await conn.getSlot("finalized");
  const [create, addr] = AddressLookupTableProgram.createLookupTable({ authority: payer.publicKey, payer: payer.publicKey, recentSlot: slot });
  await send(conn, [create], [payer]);
  table = addr;
  writeOut(name, { table: table.toBase58() });
  console.log("Table made: " + table.toBase58());
}
const state = async () => (await conn.getAddressLookupTable(table)).value.state;
let s = await state();
const have = new Set(s.addresses.map((a) => a.toBase58()));
const missing = want.filter((a) => !have.has(a.toBase58()));
if (missing.length) {
  if (!s.authority) throw new Error("the table is frozen but incomplete; make a new one with a different payer slot");
  await send(conn, [AddressLookupTableProgram.extendLookupTable({ lookupTable: table, authority: payer.publicKey, payer: payer.publicKey, addresses: missing })], [payer]);
  console.log(`Added ${missing.length} addresses.`);
}
s = await state();
if (s.authority) {
  await send(conn, [AddressLookupTableProgram.freezeLookupTable({ lookupTable: table, authority: payer.publicKey })], [payer]);
  console.log("Frozen.");
}
// A table's new addresses can be relied on once the change is final on the
// network. A transaction that uses them sooner can be dropped without a trace.
const changed = await conn.getSlot("confirmed");
process.stdout.write("Waiting for the table to settle ");
while ((await conn.getSlot("finalized")) <= changed) { process.stdout.write("."); await sleep(2000); }
console.log(" done.");
s = await state();
const t = checker();
t.eq("the table holds exactly the fixed accounts, in order", s.addresses.map((a) => a.toBase58()), want.map((a) => a.toBase58()));
t.check("the table is frozen: nobody can change it", !s.authority);
console.log("Address table: " + table.toBase58());
t.finish();
