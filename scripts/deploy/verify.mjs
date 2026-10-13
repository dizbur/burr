// Reads everything back from the network and prints PASS or FAIL per item.
// Sends nothing and needs no key.
//   node scripts/deploy/verify.mjs --wallet <the address that should hold the upgrade key and the administrator role> \
//        --platform-receiver <address> --rewards-wallet <address> [--table <address>] [--fee-range 200,300,200] [--mainnet yes]
import { PublicKey } from "@solana/web3.js";
import { readFileSync } from "node:fs";
import path from "node:path";
import { address, ata, BUYBACK, checker, configAddress, connect, fixedAccounts, loadIdl, need, options, pda, programDataAddress,
  PUMP, PUMP_AMM, PUMP_FEES, readAccount, readOut, ROOT, sha256, TOKEN, ZEC } from "./lib.mjs";

const opts = options(["url", "wallet", "platform-receiver", "rewards-wallet", "table", "fee-range", "mainnet"]);
// The fees a launcher may pick: as the program starts with, unless you changed them and say so (--fee-range 200,300,200).
const feeRange = (opts["fee-range"] ?? "50,300,200").split(",").map(Number);
const pct = (bps) => `${(bps / 100).toFixed(2)}%`;
const conn = await connect(opts);
const idl = loadIdl();
const wallet = address(need(opts, "wallet"), "--wallet").toBase58();
const platformReceiver = address(need(opts, "platform-receiver"), "--platform-receiver").toBase58();
const rewardsWallet = address(need(opts, "rewards-wallet"), "--rewards-wallet").toBase58();
const saved = readOut(conn.outName);
const tableText = opts.table ?? saved.table;
const NONE = "11111111111111111111111111111111";
const t = checker();
const trim = (b) => { let n = b.length; while (n && b[n - 1] === 0) n--; return b.subarray(0, n); };

console.log(`Program: ${idl.programId.toBase58()}\n`);
console.log("-- the program");
const program = await conn.getAccountInfo(idl.programId);
t.check("the program is on the network and runnable", !!program?.executable);
const data = await conn.getAccountInfo(programDataAddress(idl.programId));
if (t.check("its code is stored", !!data)) {
  const local = trim(readFileSync(path.join(ROOT, "target/release-kit/burrpad.so")));
  const onChain = trim(data.data.subarray(45));
  t.check("the code on the network is byte for byte the file built here", onChain.equals(local),
    `sha256 ${sha256(onChain).toString("hex").slice(0, 16)}… on the network, ${sha256(local).toString("hex").slice(0, 16)}… here`);
  const auth = data.data[12] === 1 ? new PublicKey(data.data.subarray(13, 45)).toBase58() : "none";
  t.eq("the upgrade key is the wallet named", auth, wallet);
}

console.log("\n-- the settings");
const c = await readAccount(conn, idl, "Config", configAddress(idl)).catch(() => null);
if (t.check("initialised", !!c)) {
  t.eq("layout version", c.version, 1);
  t.eq("the administrator is the wallet named", c.admin, wallet);
  t.eq("no administrator hand-over left pending", c.pending_admin, NONE);
  t.eq("platform receiver", c.platform_receiver, platformReceiver);
  t.eq("rewards wallet", c.rewards_wallet, rewardsWallet);
  t.eq("no rewards wallet change pending", c.pending_rewards_wallet, NONE);
  t.eq("the platform's share is one half", c.platform_fraction_bps, 5000);
  t.eq(`fees a launcher may pick: ${pct(feeRange[0])} to ${pct(feeRange[1])}, first offered ${pct(feeRange[2])}`, [c.min_fee_bps, c.max_fee_bps, c.default_fee_bps], feeRange);
  t.eq("launches are on", c.launches_paused, false);
  t.eq("holder releases are on", c.holder_releases_paused, false);
  t.check("the three wallets are three different addresses", new Set([wallet, platformReceiver, rewardsWallet]).size === 3);
}

console.log("\n-- the address table for launches");
if (t.check("a table address is known", !!tableText, tableText ? tableText : "give --table")) {
  const table = (await conn.getAddressLookupTable(address(tableText, "--table"))).value;
  if (t.check("the table exists", !!table)) {
    t.eq("it holds exactly the fixed accounts", table.state.addresses.map((a) => a.toBase58()), fixedAccounts(idl).map((a) => a.toBase58()));
    t.check("it is frozen", !table.state.authority);
  }
}

console.log("\n-- what the program stands on, as found on this network");
const exists = async (what, addr, owner) => {
  const info = await conn.getAccountInfo(addr);
  t.check(what, !!info && (!owner || info.owner.equals(owner)));
};
for (const [what, addr] of [["pump.fun's program", PUMP], ["PumpSwap's program", PUMP_AMM], ["pump.fun's fee program", PUMP_FEES]]) {
  const info = await conn.getAccountInfo(addr);
  t.check(`${what} is there and runnable`, !!info?.executable);
}
await exists("pump.fun's settings", pda(["global"], PUMP), PUMP);
await exists("pump.fun's permission for ZEC-priced coins", pda(["quote-control"], PUMP), PUMP);
await exists("PumpSwap's settings", pda(["global_config"], PUMP_AMM), PUMP_AMM);
await exists("ZEC", ZEC, TOKEN);
await exists("the buy-back receiver's ZEC account that burns pass", ata(BUYBACK, ZEC, TOKEN), TOKEN);
t.finish();
