// Makes room on the network for a larger program, before an upgrade that
// needs it. This script sends nothing: it prints the transaction for the
// multi-signature wallet to import and approve. The wallet's vault pays the
// rent for the new room (kept, not spent), so the vault must hold that much
// SOL. Needs no key.
//   node scripts/deploy/extend.mjs --authority <the wallet's vault address> [--mainnet yes]
// The size wanted is that of the release build in target/release-kit/.
import { PublicKey, TransactionInstruction } from "@solana/web3.js";
import { readFileSync } from "node:fs";
import path from "node:path";
import { address, connect, fail, loadIdl, LOADER, need, options, programDataAddress, ROOT, SYSTEM, unsignedForWallet } from "./lib.mjs";

const opts = options(["url", "authority", "mainnet"]);
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
const size = readFileSync(path.join(ROOT, "target/release-kit/burrpad.so")).length;
if (size <= room) {
  console.log(`The build (${size} bytes) fits the room (${room} bytes). No extend needed. Nothing printed.`);
  process.exit(0);
}
// The network adds room in steps of at least 10,240 bytes.
const extra = Math.max(size - room, 10_240);
const rent = (await conn.getMinimumBalanceForRentExemption(info.data.length + extra)) - info.lamports;

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
    { pubkey: vault, isSigner: true, isWritable: true },
  ],
  data,
});
const vaultSol = (await conn.getBalance(vault)) / 1e9;

console.log(`Room now:   ${room} bytes`);
console.log(`Build:      ${size} bytes`);
console.log(`Adds:       ${extra} bytes`);
console.log(`Rent:       ${(rent / 1e9).toFixed(6)} SOL, paid by the vault and kept in the program's record`);
console.log(`Vault has:  ${vaultSol.toFixed(6)} SOL${vaultSol * 1e9 < rent + 10_000_000 ? "  <- not enough: fund the vault first" : ""}`);
console.log("\nImport this in the wallet's app (Tx Builder, Import base58), check it, and approve:\n");
console.log(await unsignedForWallet(conn, [ix], vault));
