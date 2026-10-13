// The first-coin check: one small coin taken through everything, on the
// network the program was deployed to. Small amounts, your own key files.
// Every transaction is simulated first and is not sent if that fails.
//
//   node scripts/deploy/first-coin.mjs start  --payer <file> [--mainnet yes]
//        launches a coin with a first buy, buys, sells, collects, splits and pays out
//   node scripts/deploy/first-coin.mjs finish --payer <file> --rewards-key <file> [--mainnet yes]
//        thirty minutes later: buy back and burn, then the holders' release
//
// The payer needs about 0.15 SOL and 0.03 ZEC. It is named as the coin's
// creator and give receiver, so those shares come straight back to it.
import { Keypair } from "@solana/web3.js";
import { buyIx, burnCurveIx, clock, coinAccount, collectCurveIx, curveOf, launchIx, makeTokenAccountIx, payOutIx, releaseIx, sellIx,
  snapshotHash, splitIx, supplyOf, tokenBalance, zecAccount, zecTransferIx } from "./coin.mjs";
import { address, checker, coinAddresses, configAddress, connect, fail, loadIdl, loadKey, loadTable, need, options, readAccount,
  readOut, send, sleep, TOKEN_2022, writeOut } from "./lib.mjs";

const opts = options(["url", "payer", "rewards-key", "table", "mainnet"]);
const phase = opts._[0];
if (phase !== "start" && phase !== "finish") fail("say: start or finish");
const conn = await connect(opts);
const idl = loadIdl();
const payer = loadKey(need(opts, "payer"), "--payer");
const me = payer.publicKey;
const saved = readOut(conn.outName);
const table = await loadTable(conn, address(opts.table ?? saved.table ?? fail("no address table known: run address-table.mjs first, or give --table"), "the table"));
const cfg = await readAccount(conn, idl, "Config", configAddress(idl));
if (!cfg) fail("the program is not initialised on this network");
const t = checker();
const did = (what, sent, detail = "") => t.check(what, true, [detail, sent.link].filter(Boolean).join("  "));
const zec = (units) => `${Number(units) / 1e8} ZEC`;

if (phase === "start") {
  if (cfg.launches_paused) fail("launches are paused");
  const sol = await conn.getBalance(me);
  const myZec = await tokenBalance(conn, zecAccount(me));
  if (sol < 150_000_000) fail(`the payer holds ${sol / 1e9} SOL; this needs about 0.15`);
  if (myZec < 3_000_000n) fail(`the payer holds ${zec(myZec)}; this needs about 0.03 ZEC (the token on Solana)`);

  // 1. launch, with the launcher's own first buy in the same transaction
  const mintKey = Keypair.generate(); // a fresh address for every attempt, never shown before it is sent
  const mint = mintKey.publicKey;
  const a = coinAddresses(idl, mint);
  const plan = { name: "Burrpad first coin", symbol: "BFC", uri: "https://example.invalid/first-coin.json",
    fee_bps: 200, shares: [2500, 2500, 2500, 2500], creator_receiver: me, give_receiver: me };
  let sent = await send(conn, [
    await launchIx(conn, idl, mint, me, plan),
    makeTokenAccountIx(me, me, mint, TOKEN_2022),
    buyIx(mint, me, 1_000_000n),
  ], [payer, mintKey], { table, units: 900_000 });
  writeOut(conn.outName, { firstCoin: mint.toBase58() });
  const coin = await readAccount(conn, idl, "Coin", a.coin);
  did(`launch with a first buy of 0.01 ZEC: coin ${mint.toBase58()}`, sent, `${sent.size} of 1232 bytes`);
  t.eq("the coin's record carries the platform's share and receiver in force", [coin.platform_fraction_bps, coin.platform_receiver], [cfg.platform_fraction_bps, cfg.platform_receiver]);
  const creatorOnPump = (await conn.getAccountInfo(curveOf(mint))).data.subarray(49, 81);
  t.check("pump.fun pays this coin's fee to the program's own address", a.authority.toBuffer().equals(creatorOnPump));

  // 2. a buy and a sell, straight on pump.fun
  sent = await send(conn, [buyIx(mint, me, 500_000n)], [payer], { units: 400_000 });
  did("buy 0.005 ZEC more", sent);
  const held = await tokenBalance(conn, coinAccount(me, mint));
  sent = await send(conn, [sellIx(mint, me, held / 2n)], [payer], { units: 400_000 });
  did("sell half of what was bought", sent);

  // 3. collect the coin's fee into its vault; it is split on arrival
  sent = await send(conn, [collectCurveIx(idl, mint, me)], [payer], { units: 600_000 });
  let l = await readAccount(conn, idl, "Ledger", a.ledger);
  did("collect", sent, `fee collected: ${zec(l.total_split)}`);
  t.check("the fee arrived and was split", l.total_split > 0n && (await tokenBalance(conn, a.vault)) === l.accounted);
  t.eq("half to the platform's pot", l.pots[0], l.total_split / 2n);
  t.eq("the rest in four equal parts", l.pots.slice(1), Array(4).fill(l.total_split / 8n));

  // 4. split, of ZEC that reaches the vault by itself
  const before = l.accounted;
  sent = await send(conn, [zecTransferIx(zecAccount(me), a.vault, me, 10_000n), splitIx(idl, mint)], [payer]);
  l = await readAccount(conn, idl, "Ledger", a.ledger);
  did("split", sent);
  t.eq("ZEC sent straight to the vault was counted", l.accounted - before, 10_000n);

  // 5. pay out the three fixed pots
  for (const [payee, receiver, pot] of [["Creator", me, 1], ["Give", me, 2], ["Platform", address(coin.platform_receiver), 0]]) {
    const account = zecAccount(receiver);
    const had = await tokenBalance(conn, account);
    const due = l.pots[pot];
    sent = await send(conn, [payOutIx(idl, mint, me, payee, receiver)], [payer]);
    did(`pay out: ${payee}`, sent, zec(due));
    t.eq(`  it arrived whole at ${receiver.toBase58()}`, (await tokenBalance(conn, account)) - had, due);
  }
  l = await readAccount(conn, idl, "Ledger", a.ledger);
  t.check("the vault still covers the ledger", (await tokenBalance(conn, a.vault)) >= l.accounted);
  const ready = new Date(Number(l.holders.last_release + 1800n) * 1000);
  console.log(`\nWaiting in the pots: buy back and burn ${zec(l.pots[3])}, holders ${zec(l.pots[4])}.`);
  console.log(`The holders' release is allowed from ${ready.toLocaleTimeString()} (thirty minutes after launch). Then run:  first-coin.mjs finish`);
  t.finish();
}

// ---- finish ----
const rewards = loadKey(need(opts, "rewards-key"), "--rewards-key");
if (rewards.publicKey.toBase58() !== cfg.rewards_wallet) fail(`that key is not the rewards wallet in the settings (${cfg.rewards_wallet})`);
if (cfg.holder_releases_paused) fail("holder releases are paused");
const mint = address(saved.firstCoin ?? fail("no first coin recorded: run  first-coin.mjs start  first"), "the first coin");
const a = coinAddresses(idl, mint);
const ledger = () => readAccount(conn, idl, "Ledger", a.ledger);
let l = await ledger();
const wait = l.holders.last_release + 1800n - (await clock(conn)).time;
if (wait > 0n) fail(`too early: the holders' release is allowed in ${Math.ceil(Number(wait) / 60)} minute(s). Nothing was sent.`);

// 6. buy back and burn: one call a minute until it buys
if (l.pots[3] > 0n) {
  const supply = await supplyOf(conn, mint);
  const pot = l.pots[3];
  let sent, calls = 0;
  for (; calls < 20 && (await ledger()).pots[3] >= pot; calls++) {
    while ((await clock(conn)).time < (await ledger()).burn.last_time + 61n) await sleep(5000);
    sent = await send(conn, [burnCurveIx(idl, mint, me)], [payer], { units: 800_000 });
  }
  l = await ledger();
  if (t.check("buy back and burn bought the coin with its pot", l.pots[3] < pot, `${calls} call(s), spent ${zec(pot - l.pots[3])}  ${sent.link}`)) {
    t.eq("every coin it bought was destroyed", supply - (await supplyOf(conn, mint)), l.burn.total_burned);
  }
} else {
  console.log("      the burn pot is empty: already burned");
}

// 7. the holders' release
const pot = l.pots[4];
if (pot > 0n) {
  const had = await tokenBalance(conn, zecAccount(rewards.publicKey));
  const slot = (await clock(conn)).slot;
  const hash = snapshotHash(mint, slot, [[me.toBase58(), await tokenBalance(conn, coinAccount(me, mint))]]);
  const sent = await send(conn, [releaseIx(idl, mint, rewards.publicKey, pot, slot, hash)], [rewards]);
  l = await ledger();
  t.check("holders' release", true, `${zec(pot)}  ${sent.link}`);
  t.eq("  the pot reached the rewards wallet, whole", (await tokenBalance(conn, zecAccount(rewards.publicKey))) - had, pot);
  t.eq("  the snapshot's fingerprint is kept on the chain", Buffer.from(l.holders.last_snapshot_hash).toString("hex"), hash.toString("hex"));
} else {
  console.log("      the holders' pot is empty: already released");
}
t.check("the vault still covers the ledger", (await tokenBalance(conn, a.vault)) >= l.accounted);
t.finish();
