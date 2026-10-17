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
// --release-kit <folder> to check against a build kept elsewhere (default: target/release-kit here),
// for example the Liquidity build in ~/burr-liquidity/target/release-kit.
import { PublicKey, TransactionMessage, VersionedTransaction, ComputeBudgetProgram } from "@solana/web3.js";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { addLiquidityIx, BURR, burnBurrIx, burnBurrPotIx, shadowBurnIx, burnCurveIx, burnPoolIx, clock, collectCurveIx, collectPoolIx, curveOf, splitIx } from "./coin.mjs";
import { address, checker, coinAddresses, pda, configAddress, connect, decodeAccount, fail, loadIdl, need, options, programDataAddress, readAccount, ROOT, sha256 } from "./lib.mjs";

const opts = options(["url", "mainnet", "authority", "admin", "coins", "save", "release-kit"]);
const conn = await connect(opts);
const kit = opts["release-kit"] ? path.resolve(opts["release-kit"]) : path.join(ROOT, "target/release-kit");
if (!existsSync(path.join(kit, "burrpad.so"))) fail(`no release build in ${kit}`);
// The interface file kept with that build.
const idl = opts["release-kit"] ? JSON.parse(readFileSync(path.join(kit, "burrpad.json"), "utf8")) : loadIdl();
idl.programId = new PublicKey(idl.address);
const programId = new PublicKey(idl.programId);
const authority = address(need(opts, "authority"), "--authority");
const admin = opts.admin ? address(opts.admin, "--admin") : authority;
const config = configAddress(idl);
const keep = path.join(ROOT, "scripts/deploy/out", `settings-before-${conn.isMainnet ? "mainnet" : "test"}.json`);
const settings = await conn.getAccountInfo(config);
if (!settings) fail("no settings account: is this the right network?");

if (opts.save === "yes") {
  mkdirSync(path.dirname(keep), { recursive: true }); // a fresh checkout has no notes folder yet
  writeFileSync(keep, JSON.stringify({ at: new Date().toISOString(), data: settings.data.toString("base64") }) + "\n");
  console.log(`Kept a copy of the settings (${settings.data.length} bytes) for after the upgrade. Nothing was sent.`);
  process.exit(0);
}

const t = checker();
const trim = (b) => { let n = b.length; while (n && b[n - 1] === 0) n--; return b.subarray(0, n); };

// 1. The code, against the release build kept here.
const commit = readFileSync(path.join(kit, "commit.txt"), "utf8").trim();
const local = trim(readFileSync(path.join(kit, "burrpad.so")));
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
const expected = { BurnTooSoon: "the last burn was less than a minute ago", NothingToBurn: "nothing to buy now (the pot is empty, or too small)", HookNotChosen: "the coin did not choose this hook",
  AccountNotInitialized: "the coin has no shadow watch: it did not choose this hook, or its launcher has not set the watch yet" };
let throughTheBuy = 0, burnsTried = 0;
const run = async (ix, payer = authority) => {
  const { blockhash } = await conn.getLatestBlockhash();
  const message = new TransactionMessage({
    payerKey: payer, recentBlockhash: blockhash,
    instructions: [ComputeBudgetProgram.setComputeUnitLimit({ units: 600_000 }), ix],
  }).compileToV0Message();
  const r = await conn.simulateTransaction(new VersionedTransaction(message), { sigVerify: false, replaceRecentBlockhash: true });
  const logs = (r.value.logs ?? []).join("\n");
  return { ok: !r.value.err, err: r.value.err, code: (logs.match(/Error Code: (\w+)/) ?? [])[1], logs, units: r.value.unitsConsumed };
};
const report = (what, r) => {
  if (r.ok) return t.check(what, true, `would work (${r.units} compute units)`);
  if (r.code && expected[r.code]) return t.check(what, true, `refused as expected: ${r.code}, ${expected[r.code]}`);
  return t.check(what, false, `${r.code ?? JSON.stringify(r.err)}\n${r.logs.split("\n").slice(-6).join("\n")}`);
};
const simulate = async (what, ix, payer) => report(what, await run(ix, payer));
// A burn is simulated again once its one-minute wait is over (up to about 70 seconds), so that it is
// tried all the way through the buy, not only up to "too soon". `readyAt` gives the moment it may run.
const simulateBurn = async (what, build, readyAt) => {
  burnsTried++;
  let r = await run(build());
  for (let tries = 0; r.code === "BurnTooSoon" && tries < 2; tries++) {
    const wait = Number(await readyAt()) - Number((await clock(conn)).time) + 2;
    if (wait > 75) break;
    console.log(`      ${what}: waiting ${Math.max(wait, 1)} seconds for its wait to end, then simulating again`);
    await new Promise((res) => setTimeout(res, Math.max(wait, 1) * 1000));
    r = await run(build());
  }
  if (r.ok) throughTheBuy++;
  return report(what, r);
};
const lastBurn = (mint) => async () => {
  const l = await readAccount(conn, idl, "Ledger", coinAddresses(idl, mint).ledger);
  return BigInt(l.burn.last_time) + 60n;
};
const hasBurnBurr = idl.instructions.some((i) => i.name === "burn_burr");
const hasAddLiquidity = idl.instructions.some((i) => i.name === "add_liquidity");
const hasShadow = idl.instructions.some((i) => i.name === "shadow_burn");
const mints = (opts.coins ?? "").split(",").filter(Boolean).map((m) => address(m, "a coin"));
if (!mints.length) console.log("      (no --coins given: the simulated calls were skipped)");
for (const mint of mints) {
  const name = mint.toBase58().slice(0, 8) + "…";
  const curve = await conn.getAccountInfo(curveOf(mint));
  const graduated = !!curve && curve.data[48] !== 0;
  // What the coin is priced in: ZEC for every coin before $BURR pairs (the record reads 0), or $BURR.
  const record = await readAccount(conn, idl, "Coin", coinAddresses(idl, mint).coin);
  const quote = Number(record?.quote ?? 0);
  if (quote) console.log(`      ${name}: priced in $BURR`);
  await simulate(`${name}: collect from the curve`, collectCurveIx(idl, mint, authority, quote));
  if (graduated) await simulate(`${name}: collect from the pool`, collectPoolIx(idl, mint, authority, quote));
  await simulate(`${name}: split`, splitIx(idl, mint, quote));
  await simulateBurn(`${name}: buy back and burn (${graduated ? "pool" : "curve"})`,
    () => (graduated ? burnPoolIx(idl, mint, authority, quote) : burnCurveIx(idl, mint, authority, quote)), lastBurn(mint));
  if (quote === 1) {
    // A coin priced in $BURR burns its $BURR pot as it is; Shadow Burn and Liquidity are not for it.
    await simulate(`${name}: burn its $BURR pot`, burnBurrPotIx(idl, mint));
    continue;
  }
  if (hasBurnBurr && mint.equals(BURR)) {
    console.log(`      ${name}: buy and burn $BURR: not simulated ($BURR does not buy itself; the program refuses its own ledger twice)`);
  } else if (hasBurnBurr) {
    // No coin launched before the upgrade chose the $BURR hook, so for them the expected answer is "hook not chosen":
    // the new instruction is reached with its real accounts, up to the coin's own choice.
    const coin = await readAccount(conn, idl, "Coin", coinAddresses(idl, mint).coin);
    await simulateBurn(`${name}: buy and burn $BURR`, () => burnBurrIx(idl, mint, authority), async () => {
      const c = await readAccount(conn, idl, "Config", config);
      const own = await lastBurn(BURR)();
      return [BigInt(c.burr_buy.ready_at), own].reduce((a, b) => (a > b ? a : b));
    });
    if (!Number(coin.burr_share ?? 0)) burnsTried--;
  }
  if (hasShadow && !mint.equals(BURR)) {
    // A coin without a watch (every coin launched before the upgrade): the call reaches the new instruction with
    // a $BURR buy's real accounts, up to the missing watch. A coin with a watch: a record-only call as its keeper.
    const watch = await readAccount(conn, idl, "ShadowWatch", pda(["shadow", mint], idl.programId)).catch(() => null);
    if (watch) {
      const keeper = new PublicKey(watch.keeper);
      const wallets = watch.wallets.slice(0, watch.count).map((w) => new PublicKey(w));
      await simulate(`${name}: shadow burn (record only, as its keeper)`, shadowBurnIx(idl, mint, keeper, wallets), keeper);
    } else {
      await simulate(`${name}: shadow burn (record only)`, shadowBurnIx(idl, mint, authority));
    }
  }
  if (hasAddLiquidity && graduated) {
    // Only a graduated coin has a pool to add to. No coin launched before the upgrade chose the liquidity
    // hook, so for them the expected answer is "hook not chosen", with the pool's real accounts.
    await simulate(`${name}: add liquidity to its pool`, addLiquidityIx(idl, mint, authority));
  } else if (hasAddLiquidity) {
    console.log(`      ${name}: add liquidity: not simulated (still on its curve: it has no pool yet)`);
  }
}
if (mints.length) {
  console.log(`      burns simulated all the way through the buy: ${throughTheBuy} of ${burnsTried} tried`);
  t.check("at least one burn was simulated all the way through the buy", throughTheBuy > 0,
    throughTheBuy ? `${throughTheBuy}` : "none: every burn was waiting or had an empty pot. Run again, or add a coin whose burn pot holds ZEC");
}
t.finish();
