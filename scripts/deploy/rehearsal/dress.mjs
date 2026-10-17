// Rehearsal only: the calls the dress rehearsal makes on the copied coins,
// with what each did to the coin's books, written to a file for comparing
// before and after the upgrade. Refuses the main network.
//   node dress.mjs <before|after> --payer <file> --coins <mint,...> --out <file> [--payout-wallet <address>]
// With --payout-wallet (set and confirmed beforehand), "after" also runs the Liquidity upgrade's new calls.
// When the build has the shadow burn, "after" also runs its calls on $BURR's real pool.
import { Keypair, PublicKey, SystemProgram, TransactionInstruction } from "@solana/web3.js";
import { writeFileSync } from "node:fs";
import {
  addLiquidityIx, BURR, burnBurrIx, recoverIx, releaseIx, burnBurrPotIx, quoteParts, coinTransferIx, followWalletIx, poolBuyIx, poolSellIx, setShadowWatchIx, shadowBurnIx, supplyOf, burnCurveIx, burnPoolIx, buyIx, clock, coinAccount, collectCurveIx, collectPoolIx, curveOf, launchIx, migrateIx, poolOf,
  makeTokenAccountIx, payOutIx, sellIx, splitIx, tokenBalance, zecAccount, zecTransferIx,
} from "../coin.mjs";
import { address, ata, coinAddresses, connect, fail, loadIdl, loadKey, loadTable, need, options, pda, PUMP_AMM, readAccount, readOut, send, TOKEN, TOKEN_2022, ZEC } from "../lib.mjs";

const opts = options(["url", "payer", "coins", "out", "payout-wallet", "rewards-key"]);
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
  // The vault is in the coin's own quote coin (ZEC for every coin before $BURR pairs).
  const c = await readAccount(conn, idl, "Coin", a.coin);
  const q = quoteParts(Number(c?.quote ?? 0));
  const vault = await tokenBalance(conn, ata(a.authority, q.mint, q.program)).catch(() => 0n);
  return { vault: String(vault), ledger: l ? text(l) : null };
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
  // recover: a side ZEC account of the coin's authority (anyone can make one), filled, then brought home and split.
  const stray = Keypair.generate();
  const init = new TransactionInstruction({ programId: TOKEN, data: Buffer.concat([Buffer.from([18]), coinAddresses(idl, mint).authority.toBuffer()]),
    keys: [{ pubkey: stray.publicKey, isSigner: false, isWritable: true }, { pubkey: ZEC, isSigner: false, isWritable: false }] });
  await step("recover a side ZEC account", mint, [
    SystemProgram.createAccount({ fromPubkey: me, newAccountPubkey: stray.publicKey, lamports: 2_039_280, space: 165, programId: TOKEN }),
    init, zecTransferIx(zecAccount(me), stray.publicKey, me, 50_000_000n), recoverIx(idl, mint, stray.publicKey),
  ], [payer, stray]);
  // The holder pot, released by the rewards wallet (a stand-in on this copy) for a named snapshot.
  if (opts["rewards-key"] && Number(coin.shares[3]) > 0) {
    const rewards = loadKey(opts["rewards-key"], "--rewards-key");
    const l = await readAccount(conn, idl, "Ledger", coinAddresses(idl, mint).ledger);
    const slot = BigInt(await conn.getSlot("confirmed"));
    await step("release_holder_rewards (the whole pot)", mint, [releaseIx(idl, mint, rewards.publicKey, BigInt(l.pots[4]), slot, Buffer.alloc(32, 7))], [rewards]);
  }
  await waitForBurn(mint);
  await step(graduated ? "burn_pool" : "burn_curve", mint, [graduated ? burnPoolIx(idl, mint, me) : burnCurveIx(idl, mint, me)]);
}

// New coins: a plain launch with a first buy, as the site does it today.
const plan = { name: "Dress rehearsal", symbol: "DRESS", uri: "https://example.invalid/" + "x".repeat(76), fee_bps: 200,
  shares: [2500, 2500, 2500, 2500], creator_receiver: me, give_receiver: me };
const launchWithBuy = async (name, p, makeOwnAccount, tables = table, firstBuy = 1_000_000n) => {
  const mint = Keypair.generate();
  const ixs = makeOwnAccount
    ? [await launchIx(conn, idl, mint.publicKey, me, p, { makeOwnAccount: true })]
    : [await launchIx(conn, idl, mint.publicKey, me, p), makeTokenAccountIx(me, me, mint.publicKey, TOKEN_2022)];
  ixs.push(buyIx(mint.publicKey, me, firstBuy, 1n, p.quote ?? 0));
  await step(name, mint.publicKey, ixs, [payer, mint], { table: tables });
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

// The shadow burn's calls: a launch_v4 coin watching the payer, who sells $BURR on its real pool;
// the keeper (the payer here) buys back at once; then the payer sends $BURR to a fresh wallet, which the keeper follows.
if (phase === "after" && idl.instructions.some((i) => i.name === "shadow_burn")) {
  const m = await launchWithBuy("launch_v4 with a shadow share", { ...plan, shares: [2000, 2000, 2000, 2000], shadow_share: 2000 }, true);
  await send(conn, [makeTokenAccountIx(me, me, BURR, TOKEN_2022), poolBuyIx(BURR, me, 200_000_000n, 1n)], [payer], { units: 400_000 });
  await step("set_shadow_watch (the launcher)", m, [setShadowWatchIx(idl, m, me, [me], [], me, true)]);
  await step("fee in + split (launch_v4 coin)", m, [zecTransferIx(zecAccount(me), coinAddresses(idl, m).vault, me, 2_000_000_000n), splitIx(idl, m)]);
  const held = await tokenBalance(conn, coinAccount(me, BURR));
  const sold = held / 10n;
  const zec0 = await tokenBalance(conn, zecAccount(me));
  await send(conn, [poolSellIx(BURR, me, sold, 0n)], [payer], { units: 400_000 });
  const got = (await tokenBalance(conn, zecAccount(me))) - zec0;
  const supply0 = await supplyOf(conn, BURR);
  const pot0 = (await readAccount(conn, idl, "Ledger", coinAddresses(idl, m).ledger)).shadow.pot;
  await step("shadow_burn: the sale bought back at once", m, [shadowBurnIx(idl, m, me, [me], sold, got)]);
  const r = results.at(-1);
  r.sold = String(sold); r.received = String(got);
  r.burned = String(supply0 - (await supplyOf(conn, BURR)));
  r.spent = String(BigInt(pot0) - BigInt((await readAccount(conn, idl, "Ledger", coinAddresses(idl, m).ledger)).shadow.pot));
  const watch = await readAccount(conn, idl, "ShadowWatch", pda(["shadow", m], idl.programId));
  console.log(`      sold ${r.sold} $BURR units for ${r.received} ZEC units; spent ${r.spent}, burned ${r.burned}; still owed ${watch.owed}`);
  const fresh = Keypair.generate().publicKey;
  const decimals = (await conn.getAccountInfo(BURR)).data[44];
  await send(conn, [makeTokenAccountIx(me, fresh, BURR, TOKEN_2022), coinTransferIx(BURR, me, fresh, sold / 4n, decimals)], [payer], { units: 200_000 });
  await step("follow_wallet: the keeper follows where the watched wallet sent", m, [followWalletIx(idl, m, me, 0, me, fresh)]);
  const after = await readAccount(conn, idl, "ShadowWatch", pda(["shadow", m], idl.programId));
  console.log(`      watched wallets now ${after.count}; the new one recorded at ${after.held[1]} (sent ${sold / 4n})`);
}

// Coins priced in $BURR (when the build has them): launched with a first buy through the second address
// table, traded in $BURR, and every first-cut hook run on them. The buy-back receivers' $BURR accounts and
// the table were made beforehand by the owner's own scripts (burr-buyback-accounts.mjs, burr-table.mjs).
if (phase === "after" && idl.instructions.some((i) => i.name === "burn_burr_pot")) {
  const burrTable = await loadTable(conn, address(readOut(conn.outName).burr_table, "saved $BURR table"));
  const both = [table, burrTable];
  const B = (n) => BigInt(n) * 1_000_000n;
  const m = await launchWithBuy("launch_v5 priced in $BURR, with a first buy in $BURR", { ...plan, shares: [2000, 2000, 2000, 2000], burr_share: 2000, quote: 1 }, true, both, B(10_000));
  const coin = await readAccount(conn, idl, "Coin", coinAddresses(idl, m).coin);
  console.log(`      recorded: priced in ${coin.quote === 1 ? "$BURR" : coin.quote}, $BURR share ${coin.burr_share}`);
  await step("buy in $BURR", m, [buyIx(m, me, B(200_000), 1n, 1)], [payer], { table: both, units: 400_000 });
  const half = (await tokenBalance(conn, coinAccount(me, m))) / 2n;
  await step("sell half back for $BURR", m, [sellIx(m, me, half, 0n, 1)], [payer], { table: both, units: 400_000 });
  await step("collect_curve ($BURR coin)", m, [collectCurveIx(idl, m, me, 1)], [payer], { table: both });
  await step("split ($BURR coin)", m, [splitIx(idl, m, 1)]);
  for (const payee of ["Platform", "Creator", "Give"]) {
    const receiver = payee === "Platform" ? new PublicKey(coin.platform_receiver) : me;
    await step(`pay_out ${payee} in $BURR`, m, [payOutIx(idl, m, me, payee, receiver, 1)], [payer], { table: both });
  }
  await waitForBurn(m);
  await step("burn_curve with $BURR", m, [burnCurveIx(idl, m, me, 1)], [payer], { table: both, units: 900_000 });
  const supply0 = await supplyOf(conn, BURR);
  const pot = (await readAccount(conn, idl, "Ledger", coinAddresses(idl, m).ledger)).burr.pot;
  await step("burn_burr_pot: the $BURR share burned as it is", m, [burnBurrPotIx(idl, m)]);
  console.log(`      $BURR pot ${pot}; $BURR supply fell by ${supply0 - (await supplyOf(conn, BURR))}`);
}

writeFileSync(need(opts, "out"), JSON.stringify(results, null, 1));
