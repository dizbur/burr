// Before a multi-signature wallet approves an upgrade: checks that the
// uploaded code waiting at a buffer address is byte for byte the release
// build in this folder, and that only the wallet can use it. Anyone can make
// a buffer and offer it to a wallet, so every signer should run this.
// Sends nothing and needs no key.
//   node scripts/deploy/verify-buffer.mjs <buffer address> --authority <the wallet's vault address> [--mainnet yes]
import { PublicKey } from "@solana/web3.js";
import { readFileSync } from "node:fs";
import path from "node:path";
import { address, checker, connect, fail, LOADER, need, options, ROOT, sha256 } from "./lib.mjs";

const opts = options(["url", "authority", "mainnet"]);
const buffer = address(opts._[0] ?? fail("give the buffer's address"), "the buffer");
const conn = await connect(opts);
const want = address(need(opts, "authority"), "--authority").toBase58();
const t = checker();
const trim = (b) => { let n = b.length; while (n && b[n - 1] === 0) n--; return b.subarray(0, n); };
const info = await conn.getAccountInfo(buffer);
// A buffer: owned by the loader, kind 1, then an optional authority, then the code from byte 37.
if (t.check("an upload is waiting at that address", !!info && info.owner.equals(LOADER) && info.data.readUInt32LE(0) === 1)) {
  const local = trim(readFileSync(path.join(ROOT, "target/release-kit/burrpad.so")));
  const waiting = trim(info.data.subarray(37));
  t.check("it is byte for byte the release build here", waiting.equals(local),
    `sha256 ${sha256(waiting).toString("hex").slice(0, 16)}… waiting, ${sha256(local).toString("hex").slice(0, 16)}… here`);
  t.eq("only the wallet named can use it", info.data[4] === 1 ? new PublicKey(info.data.subarray(5, 37)).toBase58() : "none", want);
}
t.finish();
