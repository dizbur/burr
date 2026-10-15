// Rehearsal only: launches coins of the largest allowed size, each with a
// first buy, on a local validator. The first the way it is done today (its
// own instruction makes the launcher's coin account); the second with the
// launch making that account; the third the same with `launch_v3` (a $BURR
// share and a liquidity share). Prints each transaction's size. Refuses the
// main network.
//   node launch-size.mjs --payer <file>
import { Keypair } from "@solana/web3.js";
import { buyIx, coinAccount, launchIx, makeTokenAccountIx } from "../coin.mjs";
import { address, checker, connect, fail, loadIdl, loadKey, loadTable, need, options, readOut, send, TOKEN_2022 } from "../lib.mjs";

const opts = options(["url", "payer"]);
const conn = await connect(opts);
if (conn.isMainnet) fail("never on the main network");
const idl = loadIdl();
const payer = loadKey(need(opts, "payer"), "--payer");
const me = payer.publicKey;
const table = await loadTable(conn, address(readOut(conn.outName).table, "saved table"));
const t = checker();
// The longest name, ticker and link a launch takes.
const plan = { name: "N".repeat(32), symbol: "S".repeat(13), uri: "https://example.invalid/" + "x".repeat(76), fee_bps: 200,
  shares: [2500, 2500, 2500, 2500], creator_receiver: me, give_receiver: me };

const launch = async (makeOwnAccount, p = plan) => {
  const mint = Keypair.generate();
  const ixs = makeOwnAccount
    ? [await launchIx(conn, idl, mint.publicKey, me, p, { makeOwnAccount: true })]
    : [await launchIx(conn, idl, mint.publicKey, me, p), makeTokenAccountIx(me, me, mint.publicKey, TOKEN_2022)];
  ixs.push(buyIx(mint.publicKey, me, 1_000_000n, 1n));
  const sent = await send(conn, ixs, [payer, mint], { table, units: 900_000 });
  const got = await conn.getTokenAccountBalance(coinAccount(me, mint.publicKey));
  t.check(`${makeOwnAccount ? "the launch makes the account" : "its own instruction makes it"}: the first buy delivered`, BigInt(got.value.amount) > 0n);
  return sent.size;
};

const before = await launch(false);
const after = await launch(true);
const withHooks = idl.instructions.some((i) => i.name === "launch_v3")
  ? await launch(true, { ...plan, shares: [1500, 1500, 1500, 1500], burr_share: 2000, liquidity_share: 2000 })
  : null;
console.log(`launch with first buy, longest name, ticker and link, with the priority-fee instructions:`);
console.log(`  today:                          ${before} of 1232 bytes`);
console.log(`  the launch makes the account:   ${after} of 1232 bytes`);
console.log(`  saved:                          ${before - after} bytes; room left ${1232 - after}`);
if (withHooks) console.log(`  launch_v3, both extra shares:   ${withHooks} of 1232 bytes; room left ${1232 - withHooks}`);
t.check("smaller", after < before);
t.finish();
