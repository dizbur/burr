// Makes room on the network for a larger program, before an upgrade that
// needs it. The deploy wallet sends it directly and pays the rent for the new
// room (kept in the program's record, not spent). It never goes through the
// multi-signature wallet: the network refuses an extend made from inside
// another program, as Squads makes every call (seen on 16 Oct 2026). The plain
// extend needs no upgrade key on the main network today, and only adds empty
// room: it cannot change the code or the upgrade key.
//   node scripts/deploy/extend.mjs --authority <the wallet's vault address> [--mainnet yes]
//       shows what would be added and what it costs; sends nothing
//   node scripts/deploy/extend.mjs --authority <vault> --payer <deploy wallet key file> --send yes [--mainnet yes]
//       sends it, paid by the deploy wallet
// The size wanted is that of the release build in target/release-kit/ (or --release-kit <folder>).
import { PublicKey, TransactionInstruction } from "@solana/web3.js";
import { readFileSync } from "node:fs";
import path from "node:path";
import { address, connect, fail, loadIdl, loadKey, LOADER, need, options, programDataAddress, ROOT, send, SYSTEM } from "./lib.mjs";

const opts = options(["url", "authority", "mainnet", "payer", "send", "release-kit"]);
const conn = await connect(opts);
const idl = loadIdl();
const vault = address(need(opts, "authority"), "--authority");
const programId = new PublicKey(idl.programId);
const programData = programDataAddress(programId);
// A program's record: 45 bytes of header (kind, slot, upgrade key), then its room.
const HEADER = 45;

const info = await conn.getAccountInfo(programData);
if (!info || !info.owner.equals(LOADER)) fail("the program is not deployed here");
const key = info.data[12] === 1 ? new PublicKey(info.data.subarray(13, 45)) : null;
if (!key) fail("the program can no longer be changed by anyone");
if (!key.equals(vault)) fail(`the upgrade key is ${key.toBase58()}, not ${vault.toBase58()}`);

const room = info.data.length - HEADER;
const size = readFileSync(path.join(opts["release-kit"] ?? path.join(ROOT, "target/release-kit"), "burrpad.so")).length;
if (size <= room) {
  console.log(`The build (${size} bytes) fits the room (${room} bytes). No extend needed. Nothing sent.`);
  process.exit(0);
}
// The network adds room in steps of at least 10,240 bytes.
const extra = Math.max(size - room, 10_240);
const rent = (await conn.getMinimumBalanceForRentExemption(info.data.length + extra)) - info.lamports;

console.log(`Room now:   ${room} bytes`);
console.log(`Build:      ${size} bytes`);
console.log(`Adds:       ${extra} bytes`);
console.log(`Rent:       ${(rent / 1e9).toFixed(6)} SOL, paid by the deploy wallet and kept in the program's record`);
if (opts.send !== "yes" || !opts.payer) {
  console.log("\nNothing sent. To send it, paid by the deploy wallet, add: --payer <deploy wallet key file> --send yes");
  process.exit(0);
}
const payer = loadKey(opts.payer, "--payer");
const payerSol = (await conn.getBalance(payer.publicKey)) / 1e9;
console.log(`Payer:      ${payer.publicKey.toBase58()}, has ${payerSol.toFixed(6)} SOL`);
if (payerSol * 1e9 < rent + 10_000_000) fail("not enough SOL in the deploy wallet for the rent: fund it first");

// The loader's "extend program" (number 6): the program's record, the
// program, the system program, the payer. (The "checked" form, which also
// asks the upgrade key, is not active on the main network.)
const data = Buffer.alloc(8);
data.writeUInt32LE(6, 0);
data.writeUInt32LE(extra, 4);
const ix = new TransactionInstruction({
  programId: LOADER,
  keys: [
    { pubkey: programData, isSigner: false, isWritable: true },
    { pubkey: programId, isSigner: false, isWritable: true },
    { pubkey: SYSTEM, isSigner: false, isWritable: false },
    { pubkey: payer.publicKey, isSigner: true, isWritable: true },
  ],
  data,
});
const { signature } = await send(conn, [ix], [payer], { units: 50_000 });
const after = (await conn.getAccountInfo(programData)).data.length - HEADER;
console.log(`PASS  extended: the room is now ${after} bytes  (${signature})`);
if (after < size) fail(`the room (${after}) is still smaller than the build (${size})`);
