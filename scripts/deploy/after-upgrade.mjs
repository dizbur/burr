// Right after an upgrade lands: checks, reading only, that the program on the
// network is the release build kept here, that its upgrade key and settings are
// as they were, and that collect, split and burn still work on live coins, by
// simulating them (nothing is ever sent, and no key is needed).
//
//   Before the upgrade (keeps a copy of the settings):
//     node scripts/deploy/after-upgrade.mjs --save yes --authority <vault> --mainnet yes
//   After it lands:
//     node scripts/deploy/after-upgrade.mjs --authority <vault> --coins <mint,mint,...> --mainnet yes
//
// --admin <address> if the administrator is not the same as --authority.
import { PublicKey, TransactionMessage, VersionedTransaction, ComputeBudgetProgram } from "@solana/web3.js";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { burnCurveIx, burnPoolIx, collectCurveIx, collectPoolIx, curveOf, splitIx } from "./coin.mjs";
import { address, checker, configAddress, connect, decodeAccount, fail, loadIdl, need, options, programDataAddress, readAccount, ROOT, sha256 } from "./lib.mjs";

const opts = options(["url", "mainnet", "authority", "admin", "coins", "save"]);
const conn = await connect(opts);
const idl = loadIdl();
const programId = new PublicKey(idl.programId);
const authority = address(need(opts, "authority"), "--authority");
const admin = opts.admin ? address(opts.admin, "--admin") : authority;
const config = configAddress(idl);
const keep = path.join(ROOT, "scripts/deploy/out", `settings-before-${conn.isMainnet ? "mainnet" : "test"}.json`);
const settings = await conn.getAccountInfo(config);
if (!settings) fail("no settings account: is this the right network?");

if (opts.save === "yes") {
  writeFileSync(keep, JSON.stringify({ at: new Date().toISOString(), data: settings.data.toString("base64") }) + "\n");
  console.log(`Kept a copy of the settings (${settings.data.length} bytes) for after the upgrade. Nothing was sent.`);
  process.exit(0);
}

const t = checker();
const trim = (b) => { let n = b.length; while (n && b[n - 1] === 0) n--; return b.subarray(0, n); };

// 1. The code, against the release build kept here.
const commit = readFileSync(path.join(ROOT, "target/release-kit/commit.txt"), "utf8").trim();
const local = trim(readFileSync(path.join(ROOT, "target/release-kit/burrpad.so")));
const data = await conn.getAccountInfo(programDataAddress(programId));
if (t.check("the program is deployed", !!data)) {
  const onChain = trim(data.data.subarray(45));
  t.check(`the code on the network is the release build of ${commit.slice(0, 7)}`, onChain.equals(local),
    `sha256 ${sha256(onChain).toString("hex").slice(0, 16)}… on the network, ${sha256(local).toString("hex").slice(0, 16)}… here (trailing zeros left off both)`);
  // 2. The upgrade key.
  const key = data.data[12] === 1 ? new PublicKey(data.data.subarray(13, 45)).toBase58() : "none";
  t.eq("the upgrade key is still the multi-signature wallet's vault", key, authority.toBase58());
}

// 3. The settings: every field as before the upgrade, and the administrator. One field is left out:
// the shared $BURR wait and reserve (`burr_buy`), which the keeper's $BURR buys write once the upgrade is live.
const text = (x) => JSON.stringify(x, (_, v) => (typeof v === "bigint" ? v.toString() : v));
if (existsSync(keep)) {
  const before = decodeAccount(idl, "Config", Buffer.from(JSON.parse(readFileSync(keep, "utf8")).data, "base64"));
  const now = decodeAccount(idl, "Config", settings.data);
  const changed = Object.keys(now).filter((k) => k !== "burr_buy" && text(now[k]) !== text(before[k]));
  t.check("the settings are as they were before the upgrade (all but the shared $BURR wait)", !changed.length,
    changed.length ? "changed: " + changed.map((k) => `${k} ${text(before[k])} -> ${text(now[k])}`).join("; ") : `${Object.keys(now).length - 1} fields`);
} else {
  console.log("      (no copy of the settings from before the upgrade: run with --save yes first to compare them)");
}
const c = await readAccount(conn, idl, "Config", config);
t.eq("the administrator is still the vault", c.admin, admin.toBase58());
t.check("no administrator change is pending", c.pending_admin === PublicKey.default.toBase58(), c.pending_admin);
t.check("no rewards wallet change is pending", c.pending_rewards_wallet === PublicKey.default.toBase58(), c.pending_rewards_wallet);
console.log(`      launches ${c.launches_paused ? "PAUSED" : "on"}, holder releases ${c.holder_releases_paused ? "PAUSED" : "on"}`);

// 4. Collect, split and burn on live coins: simulated with the real accounts, never sent.
// Refusals that only mean "not now" are expected: the keeper burns every minute, and a pot may be empty.
const expected = { BurnTooSoon: "the coin's last burn was less than a minute ago", NothingToBurn: "nothing to buy now (the pot is empty, or too small)", HookNotChosen: "the coin has no burn share" };
const simulate = async (what, ix) => {
  const { blockhash } = await conn.getLatestBlockhash();
  const message = new TransactionMessage({
    payerKey: authority, recentBlockhash: blockhash,
    instructions: [ComputeBudgetProgram.setComputeUnitLimit({ units: 600_000 }), ix],
  }).compileToV0Message();
  const r = await conn.simulateTransaction(new VersionedTransaction(message), { sigVerify: false, replaceRecentBlockhash: true });
  const logs = (r.value.logs ?? []).join("\n");
  const code = (logs.match(/Error Code: (\w+)/) ?? [])[1];
  if (!r.value.err) return t.check(what, true, `would work (${r.value.unitsConsumed} compute units)`);
  if (code && expected[code]) return t.check(what, true, `refused as expected: ${code}, ${expected[code]}`);
  return t.check(what, false, `${code ?? JSON.stringify(r.value.err)}\n${logs.split("\n").slice(-6).join("\n")}`);
};
const mints = (opts.coins ?? "").split(",").filter(Boolean).map((m) => address(m, "a coin"));
if (!mints.length) console.log("      (no --coins given: the simulated calls were skipped)");
for (const mint of mints) {
  const name = mint.toBase58().slice(0, 8) + "…";
  const curve = await conn.getAccountInfo(curveOf(mint));
  const graduated = !!curve && curve.data[48] !== 0;
  await simulate(`${name}: collect from the curve`, collectCurveIx(idl, mint, authority));
  if (graduated) await simulate(`${name}: collect from the pool`, collectPoolIx(idl, mint, authority));
  await simulate(`${name}: split`, splitIx(idl, mint));
  await simulate(`${name}: buy back and burn (${graduated ? "pool" : "curve"})`, graduated ? burnPoolIx(idl, mint, authority) : burnCurveIx(idl, mint, authority));
}
t.finish();
