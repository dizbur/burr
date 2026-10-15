// Reads a Squads v4 multisig's settings from the chain: how many approvals
// it needs, its time lock, and its members. Sends nothing and needs no key.
//   node scripts/deploy/squad.mjs <the multisig's address> [--mainnet yes]
import { PublicKey } from "@solana/web3.js";
import { address, connect, fail, options } from "./lib.mjs";

const SQUADS_V4 = new PublicKey("SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf");
const opts = options(["url", "mainnet"]);
const multisig = address(opts._[0] ?? fail("give the multisig's address"), "the multisig");
const conn = await connect(opts);
const info = await conn.getAccountInfo(multisig);
if (!info || !info.owner.equals(SQUADS_V4)) fail("no Squads v4 multisig at that address");
const d = info.data;
// After the 8-byte code: the key it was made with (32), the config authority (32),
// the threshold (u16), the time lock in seconds (u32), two transaction counters
// (u64 each), an optional rent collector, a bump, then the members.
let o = 8 + 32;
const configAuthority = new PublicKey(d.subarray(o, o + 32)); o += 32;
const threshold = d.readUInt16LE(o); o += 2;
const timeLock = d.readUInt32LE(o); o += 4;
o += 16;
o += d[o] === 1 ? 33 : 1;
o += 1;
const count = d.readUInt32LE(o); o += 4;
const members = [];
for (let i = 0; i < count; i++, o += 33) {
  const p = d[o + 32];
  members.push(`${new PublicKey(d.subarray(o, o + 32)).toBase58()}  ${["propose", "vote", "execute"].filter((_, b) => p & (1 << b)).join(", ")}`);
}
const span = (s) => (s === 0 ? "none" : s % 86400 === 0 ? `${s / 86400} day(s)` : s % 3600 === 0 ? `${s / 3600} hour(s)` : `${s} seconds`);
console.log(`Approvals needed: ${threshold} of ${count}`);
console.log(`Time lock:        ${span(timeLock)} (${timeLock} seconds)`);
console.log(`Config authority: ${configAuthority.equals(PublicKey.default) ? "none (settings change only by the members' vote)" : configAuthority.toBase58()}`);
console.log("Members:");
for (const m of members) console.log("  " + m);
