// Passes the upgrade key (the power to replace the program) to the
// multi-signature wallet. THIS CANNOT BE UNDONE from this side: afterwards
// only that wallet can change the program.
//   node scripts/deploy/hand-over-upgrade.mjs --url <network> --upgrade-key <file> --new-authority <address> [--mainnet yes]
// It refuses unless that same wallet is already the administrator. Accepting
// the administrator role needed the wallet's signature, so that proves the
// address is one the wallet really controls.
import { PublicKey, TransactionInstruction } from "@solana/web3.js";
import { address, checker, configAddress, connect, fail, loadIdl, loadKey, LOADER, need, options, programDataAddress, readAccount, send } from "./lib.mjs";

const opts = options(["url", "upgrade-key", "new-authority", "mainnet"]);
const conn = await connect(opts);
const idl = loadIdl();
const key = loadKey(need(opts, "upgrade-key"), "--upgrade-key");
const next = address(need(opts, "new-authority"), "--new-authority");
const programData = programDataAddress(idl.programId);

const upgradeAuthority = async () => {
  const info = await conn.getAccountInfo(programData);
  if (!info) fail("the program is not deployed here");
  return info.data[12] === 1 ? new PublicKey(info.data.subarray(13, 45)).toBase58() : "none";
};

const c = await readAccount(conn, idl, "Config", configAddress(idl));
if (!c) fail("not initialised yet");
if (c.admin !== next.toBase58()) {
  fail(`the administrator is ${c.admin}, not ${next.toBase58()}. Hand the administrator role over first (admin.mjs propose-admin, then accept-admin from the wallet).`);
}
if ((await upgradeAuthority()) === next.toBase58()) {
  console.log("Already handed over. Nothing sent.");
  process.exit(0);
}
const data = Buffer.alloc(4);
data.writeUInt32LE(4); // the loader's "set authority"
await send(conn, [new TransactionInstruction({
  programId: LOADER,
  keys: [
    { pubkey: programData, isSigner: false, isWritable: true },
    { pubkey: key.publicKey, isSigner: true, isWritable: false },
    { pubkey: next, isSigner: false, isWritable: false },
  ],
  data,
})], [key]);
const t = checker();
t.eq("the upgrade key is now the multi-signature wallet", await upgradeAuthority(), next.toBase58());
t.finish();
