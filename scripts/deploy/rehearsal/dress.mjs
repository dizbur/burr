// Rehearsal only: the calls the dress rehearsal makes on the copied coins,
// with what each did to the coin's books, written to a file for comparing
// before and after the upgrade. Refuses the main network.
//   node dress.mjs <before|after> --payer <file> --coins <mint,...> --out <file>
import { Keypair, PublicKey } from "@solana/web3.js";
import { writeFileSync } from "node:fs";
import {
  BURR, burnBurrIx, burnCurveIx, burnPoolIx, buyIx, clock, coinAccount, collectCurveIx, collectPoolIx, curveOf, launchIx,
  makeTokenAccountIx, payOutIx, splitIx, tokenBalance, zecAccount, zecTransferIx,
} from "../coin.mjs";
import { address, coinAddresses, connect, fail, loadIdl, loadKey, loadTable, need, options, readAccount, readOut, send, TOKEN_2022 } from "../lib.mjs";

const opts = options(["url", "payer", "coins", "out"]);
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
writeFileSync(need(opts, "out"), JSON.stringify(results, null, 1));
