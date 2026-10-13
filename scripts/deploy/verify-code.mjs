// Reads the program's code back from the network and compares it, byte for
// byte, with the file built here. Sends nothing and needs no key.
//   node scripts/deploy/verify-code.mjs [--mainnet yes]
import { PublicKey } from "@solana/web3.js";
import { readFileSync } from "node:fs";
import path from "node:path";
import { checker, connect, loadIdl, options, programDataAddress, ROOT, sha256 } from "./lib.mjs";

const opts = options(["url", "mainnet"]);
const conn = await connect(opts);
const idl = loadIdl();
const t = checker();
const trim = (b) => { let n = b.length; while (n && b[n - 1] === 0) n--; return b.subarray(0, n); };
const data = await conn.getAccountInfo(programDataAddress(idl.programId));
if (t.check(`program ${idl.programId.toBase58()} is on the network`, !!data)) {
  const local = trim(readFileSync(path.join(ROOT, "target/deploy/burrpad.so")));
  const onChain = trim(data.data.subarray(45));
  t.check("the code on the network is byte for byte the file built here", onChain.equals(local),
    `sha256 ${sha256(onChain).toString("hex").slice(0, 16)}… on the network, ${sha256(local).toString("hex").slice(0, 16)}… here`);
  console.log("      upgrade key: " + (data.data[12] === 1 ? new PublicKey(data.data.subarray(13, 45)).toBase58() : "none"));
}
t.finish();
