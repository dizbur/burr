// Rehearsal only: does on a local validator what the multi-signature wallet
// does on the main network, with a throwaway key standing in for its vault.
// Refuses the main network.
//   node stand-in.mjs sign <file with a base58 transaction> --key <file>   signs as the vault and sends
//   node stand-in.mjs upgrade <buffer> --spill <address> --key <file>      the program upgrade from a buffer
import { PublicKey, Transaction, TransactionInstruction } from "@solana/web3.js";
import bs58 from "bs58";
import { readFileSync } from "node:fs";
import { address, connect, fail, loadIdl, loadKey, LOADER, need, options, programDataAddress, send } from "../lib.mjs";

const opts = options(["url", "key", "spill"]);
const conn = await connect(opts);
if (conn.isMainnet) fail("never on the main network");
const key = loadKey(need(opts, "key"), "--key");
const [what, arg] = opts._;
if (what === "sign") {
  const tx = Transaction.from(bs58.decode(readFileSync(arg ?? fail("give the file"), "utf8").trim()));
  tx.recentBlockhash = (await conn.getLatestBlockhash()).blockhash;
  tx.sign(key);
  const sig = await conn.sendRawTransaction(tx.serialize());
  const r = await conn.confirmTransaction(sig, "confirmed");
  if (r.value.err) fail("failed: " + JSON.stringify(r.value.err));
  console.log("PASS  signed as the stand-in vault and sent");
} else if (what === "upgrade") {
  const programId = new PublicKey(loadIdl().programId);
  const buffer = address(arg ?? fail("give the buffer"), "the buffer");
  const spill = address(need(opts, "spill"), "--spill");
  // The loader's "upgrade" (number 3): program data, program, buffer, spill, rent, clock, upgrade key.
  const data = Buffer.alloc(4);
  data.writeUInt32LE(3, 0);
  const ix = new TransactionInstruction({
    programId: LOADER,
    keys: [
      { pubkey: programDataAddress(programId), isSigner: false, isWritable: true },
      { pubkey: programId, isSigner: false, isWritable: true },
      { pubkey: buffer, isSigner: false, isWritable: true },
      { pubkey: spill, isSigner: false, isWritable: true },
      { pubkey: new PublicKey("SysvarRent111111111111111111111111111111111"), isSigner: false, isWritable: false },
      { pubkey: new PublicKey("SysvarC1ock11111111111111111111111111111111"), isSigner: false, isWritable: false },
      { pubkey: key.publicKey, isSigner: true, isWritable: false },
    ],
    data,
  });
  await send(conn, [ix], [key], { units: 200_000 });
  console.log("PASS  upgraded from the buffer, leftover SOL to " + spill.toBase58());
} else fail("say: sign or upgrade");
