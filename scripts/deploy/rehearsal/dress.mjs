// Rehearsal only: the calls the dress rehearsal makes on the copied coins,
// with what each did to the coin's books, written to a file for comparing
// before and after the upgrade. Refuses the main network.
//   node dress.mjs <before|after> --payer <file> --coins <mint,...> --out <file> [--payout-wallet <address>]
// With --payout-wallet (set and confirmed beforehand), "after" also runs the Liquidity upgrade's new calls.
import { Keypair, PublicKey } from "@solana/web3.js";
import { writeFileSync } from "node:fs";
import {
  addLiquidityIx, BURR, burnBurrIx, burnCurveIx, burnPoolIx, buyIx, clock, coinAccount, collectCurveIx, collectPoolIx, curveOf, launchIx, migrateIx, poolOf,
  makeTokenAccountIx, payOutIx, splitIx, tokenBalance, zecAccount, zecTransferIx,
} from "../coin.mjs";
import { address, ata, coinAddresses, connect, fail, loadIdl, loadKey, loadTable, need, options, pda, PUMP_AMM, readAccount, readOut, send, TOKEN, TOKEN_2022, ZEC } from "../lib.mjs";

const opts = options(["url", "payer", "coins", "out", "payout-wallet"]);
const phase = opts._[0];
if (!["before", "after"].includes(phase)) fail("say: before or after");
const conn = await connect(opts);
if (conn.isMainnet) fail("never on the main network");
const idl = loadIdl();
const payer = loadKey(need(opts, "payer"), "--payer");
const me = payer.publicKey;
const mints = need(opts, "coins").split(",").map((m) => address(m, "a coin"));
const table = await loadTable(conn, address(readOut(conn.outName).table, "saved table"));
const text = (x) => JSON.parse(JSON.stringify(x, (_, v) => (typeof v === "bigint" ? v.toString() : v)));
const books = async (mint) => {
  const a = coinAddresses(idl, mint);
  const l = await readAccount(conn, idl, "Ledger", a.ledger);
  return { vault: String(await tokenBalance(conn, a.vault)), ledger: l ? text(l) : null };
};
const results = [];
const step = async (name, mint, ixs, signers = [payer], sendOpts = {}) => {
  const before = await books(mint);
  let ok = true, err = "", size = 0;
  try { size = (await send(conn, ixs, signers, { units: 900_000, ...sendOpts })).size; }
  catch (e) { ok = false; err = (e.message.match(/Error Code: \w+|custom program error: 0x\w+|insufficient[^\n]*|[A-Z]\w+Error[^\n]{0,80}/) ?? [e.message.slice(0, 160)])[0]; }
  const after = await books(mint);
  results.push({ name, mint: mint.toBase58(), ok, err, size, before, after });
  console.log(`${ok ? "ok  " : "FAIL"}  ${name} ${mint.toBase58().slice(0, 8)}${err ? "  " + err : ""}`);
};

// A burn is refused for 60 seconds after the last one (the keeper burns live coins every minute):
// wait it out, so that both runs really burn.
const waitForBurn = async (mint) => {
  const l = await readAccount(conn, idl, "Ledger", coinAddresses(idl, mint).ledger);
  const wait = Number(l.burn.last_time) + 62 - Number((await clock(conn)).time);
  if (wait > 0) await new Promise((r) => setTimeout(r, wait * 1000));
};

for (const mint of mints) {
  const coin = await readAccount(conn, idl, "Coin", coinAddresses(idl, mint).coin);
  const curve = await conn.getAccountInfo(curveOf(mint));
  const graduated = !!curve && curve.data[48] !== 0;
  await step("collect_curve", mint, [collectCurveIx(idl, mint, me)]);
  if (graduated) await step("collect_pool", mint, [collectPoolIx(idl, mint, me)]);
  // Fee arriving, then split: 2 test ZEC sent straight to the vault.
  await step("fee in + split", mint, [zecTransferIx(zecAccount(me), coinAddresses(idl, mint).vault, me, 200_000_000n), splitIx(idl, mint)]);
  for (const [payee, receiver] of [["Platform", coin.platform_receiver], ["Creator", coin.creator_receiver], ["Give", coin.give_receiver]]) {
    if (receiver !== PublicKey.default.toBase58()) await step(`pay_out ${payee}`, mint, [payOutIx(idl, mint, me, payee, new PublicKey(receiver))]);
  }
  await waitForBurn(mint);
  await step(graduated ? "burn_pool" : "burn_curve", mint, [graduated ? burnPoolIx(idl, mint, me) : burnCurveIx(idl, mint, me)]);
}

// New coins: a plain launch with a first buy, as the site does it today.
const plan = { name: "Dress rehearsal", symbol: "DRESS", uri: "https://example.invalid/" + "x".repeat(76), fee_bps: 200,
  shares: [2500, 2500, 2500, 2500], creator_receiver: me, give_receiver: me };
const launchWithBuy = async (name, p, makeOwnAccount) => {
  const mint = Keypair.generate();
  const ixs = makeOwnAccount
    ? [await launchIx(conn, idl, mint.publicKey, me, p, { makeOwnAccount: true })]
    : [await launchIx(conn, idl, mint.publicKey, me, p), makeTokenAccountIx(me, me, mint.publicKey, TOKEN_2022)];
  ixs.push(buyIx(mint.publicKey, me, 1_000_000n, 1n));
  await step(name, mint.publicKey, ixs, [payer, mint], { table });
  const bought = await tokenBalance(conn, coinAccount(me, mint.publicKey)).catch(() => 0n);
  results.at(-1).bought = String(bought);
  return mint.publicKey;
};
await launchWithBuy("launch (today's way)", plan, false);
if (phase === "after") {
  await launchWithBuy("launch, the launch making the buyer's account", plan, true);
  const v2 = await launchWithBuy("launch_v2 with a $BURR share", { ...plan, shares: [2000, 2000, 2000, 2000], burr_share: 2000 }, true);
  await step("fee in + split (launch_v2 coin)", v2, [zecTransferIx(zecAccount(me), coinAddresses(idl, v2).vault, me, 200_000_000n), splitIx(idl, v2)]);
  await waitForBurn(BURR);
  await step("burn_burr on $BURR's real pool", v2, [burnBurrIx(idl, v2, me)]);
  const burr = await readAccount(conn, idl, "Ledger", coinAddresses(idl, BURR).ledger);
  console.log("      $BURR's own ledger after: burn last_time", String(burr.burn.last_time));
}
// The Liquidity upgrade's new calls, once the payout wallet has been set and confirmed (by admin.mjs, before this).
if (phase === "after" && opts["payout-wallet"]) {
  const wallet = address(opts["payout-wallet"], "--payout-wallet");
  const every = { ...plan, shares: [1500, 1500, 1500, 1500], burr_share: 2000, liquidity_share: 2000,
    payout_options: 7, long_holder_step_bps: 100, long_holder_max_bps: 10000, creator_receiver: wallet };
  const m = await launchWithBuy("launch_v3, every option on", every, true);
  const coin = await readAccount(conn, idl, "Coin", coinAddresses(idl, m).coin);
  console.log(`      recorded: choices ${coin.payout_options}, bonus ${coin.long_holder_step_bps}/${coin.long_holder_max_bps}, $BURR ${coin.burr_share}, liquidity ${coin.liquidity_share}, creator receiver ${coin.creator_receiver === wallet.toBase58() ? "the payout wallet" : coin.creator_receiver}`);
  // Graduation on pump.fun's real programs: buy until the curve completes, then pump.fun's own migration to PumpSwap.
  for (let i = 0; i < 20; i++) {
    if ((await conn.getAccountInfo(curveOf(m))).data[48] !== 0) break;
    await send(conn, [buyIx(m, me, 500_000_000n, 1n)], [payer], { units: 400_000 });
  }
  await step("pump.fun's migration to PumpSwap", m, [migrateIx(m, me)], [payer], { units: 600_000 });
  await step("fee in + split (launch_v3 coin)", m, [zecTransferIx(zecAccount(me), coinAddresses(idl, m).vault, me, 2_000_000_000n), splitIx(idl, m)]);
  await waitForBurn(m);
  await step("add_liquidity, first call (records)", m, [addLiquidityIx(idl, m, me)]);
  await waitForBurn(m);
  const lpMint = pda(["pool_lp_mint", poolOf(m)], PUMP_AMM);
  const lpSupply = async () => (await conn.getAccountInfo(lpMint)).data.readBigUInt64LE(36);
  const poolZec = async () => tokenBalance(conn, ata(poolOf(m), ZEC, TOKEN));
  const [supplyBefore, zecBefore] = [await lpSupply(), await poolZec()];
  await step("add_liquidity, adds and burns the receipts", m, [addLiquidityIx(idl, m, me)]);
  const r = results.at(-1);
  r.lpSupplyBefore = String(supplyBefore);
  r.lpSupplyAfter = String(await lpSupply());
  r.burnerReceipts = String(await tokenBalance(conn, ata(coinAddresses(idl, m).burner, lpMint, TOKEN_2022)).catch(() => 0n));
  r.poolZecRose = String((await poolZec()) - zecBefore);
  console.log(`      receipt mint supply ${r.lpSupplyBefore} -> ${r.lpSupplyAfter} (all burned if equal); the burner holds ${r.burnerReceipts} receipts; the pool's ZEC rose ${r.poolZecRose}`);
  const walletZec = async () => tokenBalance(conn, zecAccount(wallet)).catch(() => 0n);
  const w0 = await walletZec();
  await step("pay_out Creator to the payout wallet", m, [payOutIx(idl, m, me, "Creator", wallet)]);
  results.at(-1).payoutWalletRose = String((await walletZec()) - w0);
  console.log(`      the payout wallet received ${results.at(-1).payoutWalletRose}`);
  await waitForBurn(BURR);
  await step("burn_burr (launch_v3 coin)", m, [burnBurrIx(idl, m, me)]);
}

writeFileSync(need(opts, "out"), JSON.stringify(results, null, 1));
