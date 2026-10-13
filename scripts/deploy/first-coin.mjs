// The first-coin check: one small coin taken through everything, on the
// network the program was deployed to. Small amounts, your own key files.
// Every transaction is simulated first and is not sent if that fails.
//
//   node scripts/deploy/first-coin.mjs start  --payer <file> [--mainnet yes]
//        launches a coin with a first buy, buys, sells, collects, splits and pays out
//   node scripts/deploy/first-coin.mjs finish --payer <file> --rewards-key <file> [--mainnet yes]
//        thirty minutes later: buy back and burn, then the holders' release
//
// The payer needs about 0.06 SOL and 0.03 ZEC. It is named as the coin's
// creator and give receiver, so those shares come straight back to it.
// If `start` stops part way, running it again carries on with the same coin;
// it does not launch a second one. `--again yes` starts a fresh coin.
import { Keypair } from "@solana/web3.js";
import { buyIx, burnCurveIx, clock, coinAccount, collectCurveIx, curveOf, launchIx, makeTokenAccountIx, payOutIx, releaseIx, sellIx,
  snapshotHash, splitIx, supplyOf, tokenBalance, zecAccount, zecTransferIx } from "./coin.mjs";
import { address, checker, coinAddresses, configAddress, connect, fail, loadIdl, loadKey, loadTable, need, options, readAccount,
  readOut, send, sleep, TOKEN_2022, writeOut } from "./lib.mjs";

const opts = options(["url", "payer", "rewards-key", "table", "again", "mainnet"]);
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
// These trades are a few cents, so they accept any price. A real buy or sell
// must state the least it accepts, worked out from the curve.
const ANY_PRICE_COINS = 1n, ANY_PRICE_ZEC = 0n;

if (phase === "start") {
  if (cfg.launches_paused) fail("launches are paused");
  const sol = await conn.getBalance(me);
  const myZec = await tokenBalance(conn, zecAccount(me));
  if (sol < 60_000_000) fail(`the payer holds ${sol / 1e9} SOL; this needs about 0.06`);
  if (myZec < 3_000_000n) fail(`the payer holds ${zec(myZec)}; this needs about 0.03 ZEC (the token on Solana)`);
  if ((await conn.getBalance(address(cfg.rewards_wallet))) < 5_000_000) {
    fail(`the rewards wallet ${cfg.rewards_wallet} holds less than 0.005 SOL; the second half needs it to pay a fee and open its ZEC account`);
  }

  // Which coin: the one already started, if its record exists, else a new one.
  let mint = null, step = 0;
  if (saved.firstCoin && opts.again !== "yes") {
    const earlier = address(saved.firstCoin, "the saved coin");
    // A launch sent moments ago may not be visible yet: give it until it can no longer land.
    for (let i = 0; i < 100 && saved.firstCoinStep === 0 && !(await readAccount(conn, idl, "Coin", coinAddresses(idl, earlier).coin))
      && (await conn.getBlockHeight("confirmed")) <= (saved.firstCoinLastHeight ?? 0); i++) await sleep(2000);
    if (await readAccount(conn, idl, "Coin", coinAddresses(idl, earlier).coin)) {
      mint = earlier; step = Math.max(saved.firstCoinStep ?? 1, 1);
      console.log(`Carrying on with the coin started earlier: ${mint.toBase58()} (step ${step} of 5 done)`);
    }
  }
  const done = (n) => { step = n; writeOut(conn.outName, { firstCoinStep: n }); };
  let sent;

  // 1. launch, with the launcher's own first buy in the same transaction
  if (!mint) {
    const mintKey = Keypair.generate(); // a fresh address for every attempt
    mint = mintKey.publicKey;
    // Written down before sending: if the answer is lost on the way back, the next run finds the coin instead of making another.
    writeOut(conn.outName, { firstCoin: mint.toBase58(), firstCoinStep: 0, firstCoinLastHeight: (await conn.getLatestBlockhash()).lastValidBlockHeight + 20 });
    const plan = { name: "Burrpad first coin", symbol: "BFC", uri: "https://example.invalid/first-coin.json",
      fee_bps: 200, shares: [2500, 2500, 2500, 2500], creator_receiver: me, give_receiver: me };
    sent = await send(conn, [
      await launchIx(conn, idl, mint, me, plan),
      makeTokenAccountIx(me, me, mint, TOKEN_2022),
      buyIx(mint, me, 1_000_000n, ANY_PRICE_COINS),
    ], [payer, mintKey], { table, units: 900_000 });
    done(1);
    did(`launch with a first buy of 0.01 ZEC: coin ${mint.toBase58()}`, sent, `${sent.size} of 1232 bytes`);
  }
  const a = coinAddresses(idl, mint);
  const coin = await readAccount(conn, idl, "Coin", a.coin);
  t.eq("the coin's record carries the platform's share and receiver in force", [coin.platform_fraction_bps, coin.platform_receiver], [cfg.platform_fraction_bps, cfg.platform_receiver]);
  const creatorOnPump = (await conn.getAccountInfo(curveOf(mint))).data.subarray(49, 81);
  t.check("pump.fun pays this coin's fee to the program's own address", a.authority.toBuffer().equals(creatorOnPump));

  // 2. a buy and a sell, straight on pump.fun
  if (step < 2) {
    sent = await send(conn, [buyIx(mint, me, 500_000n, ANY_PRICE_COINS)], [payer], { units: 400_000 });
    did("buy 0.005 ZEC more", sent);
    const held = await tokenBalance(conn, coinAccount(me, mint));
    sent = await send(conn, [sellIx(mint, me, held / 2n, ANY_PRICE_ZEC)], [payer], { units: 400_000 });
    did("sell half of what was bought", sent);
    done(2);
  }

  // 3. collect the coin's fee into its vault; it is split on arrival
  let l;
  if (step < 3) {
    sent = await send(conn, [collectCurveIx(idl, mint, me)], [payer], { units: 600_000 });
    l = await readAccount(conn, idl, "Ledger", a.ledger);
    did("collect", sent, `fee collected so far: ${zec(l.total_split)}`);
    t.check("the fee arrived and was split", l.total_split > 0n && (await tokenBalance(conn, a.vault)) === l.accounted);
    done(3);
  }

  // 4. split, of ZEC that reaches the vault by itself
  if (step < 4) {
    const before = (await readAccount(conn, idl, "Ledger", a.ledger)).accounted;
    sent = await send(conn, [zecTransferIx(zecAccount(me), a.vault, me, 10_000n), splitIx(idl, mint)], [payer]);
    l = await readAccount(conn, idl, "Ledger", a.ledger);
    did("split", sent);
    t.check("ZEC sent straight to the vault was counted", l.accounted - before >= 10_000n);
    done(4);
  }

  // 5. pay out the three fixed pots. Anyone may; paying an empty pot is harmless.
  for (const [payee, receiver, pot] of [["Creator", me, 1], ["Give", me, 2], ["Platform", address(coin.platform_receiver), 0]]) {
    l = await readAccount(conn, idl, "Ledger", a.ledger);
    const due = l.pots[pot];
    if (due === 0n) { console.log(`      ${payee}'s pot is empty: already paid`); continue; }
    const account = zecAccount(receiver);
    const had = await tokenBalance(conn, account);
    sent = await send(conn, [payOutIx(idl, mint, me, payee, receiver)], [payer]);
    did(`pay out: ${payee}`, sent, zec(due));
    t.check(`  it arrived at ${receiver.toBase58()}`, (await tokenBalance(conn, account)) - had >= due);
  }
  done(5);
  l = await readAccount(conn, idl, "Ledger", a.ledger);
  t.eq("half of everything split went to the platform", l.credited[0], l.total_split / 2n);
  t.check("the vault covers the ledger", (await tokenBalance(conn, a.vault)) >= l.accounted);
  const ready = new Date(Number(l.holders.last_release + BigInt(coin.holders_release_interval_secs)) * 1000);
  console.log(`\nWaiting in the pots: buy back and burn ${zec(l.pots[3])}, holders ${zec(l.pots[4])}.`);
  console.log(`The holders' release is allowed from ${ready.toLocaleTimeString()} (thirty minutes after launch). Then run:  first-coin.mjs finish`);
  t.finish();
}

// ---- finish ----
const rewards = loadKey(need(opts, "rewards-key"), "--rewards-key");
if (rewards.publicKey.toBase58() !== cfg.rewards_wallet) fail(`that key is not the rewards wallet in the settings (${cfg.rewards_wallet})`);
if (cfg.holder_releases_paused) fail("holder releases are paused");
if ((await conn.getBalance(rewards.publicKey)) < 5_000_000) fail("the rewards wallet holds less than 0.005 SOL; it pays the fee for the release and opens its own ZEC account");
const mint = address(saved.firstCoin ?? fail("no first coin recorded: run  first-coin.mjs start  first"), "the first coin");
const a = coinAddresses(idl, mint);
const coin = await readAccount(conn, idl, "Coin", a.coin);
if (!coin) fail("the recorded first coin does not exist: run  first-coin.mjs start  first");
const ledger = () => readAccount(conn, idl, "Ledger", a.ledger);
let l = await ledger();
const wait = l.holders.last_release + BigInt(coin.holders_release_interval_secs) - (await clock(conn)).time;
if (wait > 0n && l.pots[4] > 0n) fail(`too early: the holders' release is allowed in ${Math.ceil(Number(wait) / 60)} minute(s). Nothing was sent.`);

// 6. buy back and burn: one call a minute until it buys. Once is enough for
// this check; each buy spends only part of the pot, by design.
if (l.burn.total_burned > 0n) {
  console.log(`      buy back and burn has already bought and destroyed ${l.burn.total_burned} units of the coin`);
} else if (l.pots[3] > 0n) {
  const supply = await supplyOf(conn, mint);
  let sent = null, calls = 0;
  for (; calls < 20 && (await ledger()).burn.total_burned === 0n; calls++) {
    if (calls > 0) console.log(`      the price has moved since launch; the buy back follows it one step a minute (${calls})`);
    while ((await clock(conn)).time < (await ledger()).burn.last_time + BigInt(coin.burn_cooldown_secs) + 1n) await sleep(5000);
    sent = await send(conn, [burnCurveIx(idl, mint, me)], [payer], { units: 800_000 }).catch((e) => {
      if (/BurnTooSoon/.test(e.message)) return null; // someone else called it first; wait and look again
      throw e;
    });
  }
  l = await ledger();
  if (l.burn.total_burned === 0n) {
    console.log("      The price has risen a long way since launch and the buy back is still following it. Nothing is wrong.");
    console.log("      Run  first-coin.mjs finish  again in a few minutes; it carries on.");
    process.exit(0);
  }
  t.check("buy back and burn bought the coin with its pot", true, `spent ${zec(l.burn.total_spent)}  ${sent?.link ?? ""}`);
  t.check("the coins it bought were destroyed", supply - (await supplyOf(conn, mint)) >= l.burn.total_burned);
} else {
  console.log("      the burn pot is empty");
}

// 7. the holders' release
l = await ledger();
const pot = l.pots[4];
if (pot > 0n) {
  const had = await tokenBalance(conn, zecAccount(rewards.publicKey));
  // A slot the whole network has settled on: later than the last release, and never ahead of the slot the release runs in.
  const slot = BigInt(await conn.getSlot("finalized"));
  if (slot <= l.holders.last_release_slot) fail("the network has not yet settled a slot after the launch; run this again in a minute");
  const hash = snapshotHash(mint, slot, [[me.toBase58(), await tokenBalance(conn, coinAccount(me, mint))]]);
  const sent = await send(conn, [releaseIx(idl, mint, rewards.publicKey, pot, slot, hash)], [rewards]);
  l = await ledger();
  t.check("holders' release", true, `${zec(pot)}  ${sent.link}`);
  t.eq("  the pot reached the rewards wallet, whole", (await tokenBalance(conn, zecAccount(rewards.publicKey))) - had, pot);
  t.eq("  the snapshot's fingerprint is kept on the chain", Buffer.from(l.holders.last_snapshot_hash).toString("hex"), hash.toString("hex"));
} else {
  console.log(l.holders.runs > 0n ? "      the holders' pot has already been released" : "      the holders' pot is empty");
}
t.check("the vault covers the ledger", (await tokenBalance(conn, a.vault)) >= l.accounted);
t.finish();
