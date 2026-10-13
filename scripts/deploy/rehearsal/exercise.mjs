// Rehearsal only: checks the two pause switches against a local validator.
// Refuses the main network.
//   node exercise.mjs paused   with both switches thrown: a launch and a holder release must be refused
//   node exercise.mjs resumed  with both switches back: a launch must work
// Options: --url --payer <file> --rewards-key <file>
import { Keypair } from "@solana/web3.js";
import { clock, launchIx, releaseIx } from "../coin.mjs";
import { address, checker, coinAddresses, connect, fail, loadIdl, loadKey, loadTable, need, options, readAccount, readOut, send } from "../lib.mjs";

const opts = options(["url", "payer", "rewards-key"]);
const phase = opts._[0];
const conn = await connect(opts);
if (conn.isMainnet) fail("never on the main network");
const idl = loadIdl();
const payer = loadKey(need(opts, "payer"), "--payer");
const rewards = loadKey(need(opts, "rewards-key"), "--rewards-key");
const table = await loadTable(conn, address(readOut(conn.outName).table, "saved table"));
const t = checker();

const launch = async () => {
  const mint = Keypair.generate();
  const plan = { name: "Rehearsal", symbol: "RHS", uri: "https://example.invalid/r.json", fee_bps: 200, shares: [2500, 2500, 2500, 2500],
    creator_receiver: payer.publicKey, give_receiver: payer.publicKey };
  await send(conn, [await launchIx(conn, idl, mint.publicKey, payer.publicKey, plan)], [payer, mint], { table, units: 600_000 });
  return mint.publicKey;
};
const refusedWith = async (what, name, attempt) => {
  try { await attempt(); t.check(what, false, "it went through"); }
  catch (e) { t.check(what, e.message.includes(name), e.message.includes(name) ? "refused: " + name : e.message.slice(0, 300)); }
};

if (phase === "paused") {
  const mint = address(readOut(conn.outName).firstCoin, "the first coin");
  await refusedWith("a launch is refused while launches are paused", "LaunchesPaused", launch);
  await refusedWith("a holder release is refused while releases are paused", "HolderReleasesPaused",
    async () => send(conn, [releaseIx(idl, mint, rewards.publicKey, 1n, (await clock(conn)).slot, Buffer.alloc(32, 1))], [rewards]));
} else if (phase === "resumed") {
  const mint = await launch();
  t.check("a launch works again once launches are resumed", !!(await readAccount(conn, idl, "Coin", coinAddresses(idl, mint).coin)));
} else fail("say: paused or resumed");
t.finish();
