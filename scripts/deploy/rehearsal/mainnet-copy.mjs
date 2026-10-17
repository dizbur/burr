// Rehearsal only. Copies from the main network, reading only, everything the
// dress rehearsal of an upgrade touches, as files a local validator loads at
// start: the live program (its upgrade key replaced by a stand-in for the
// multi-signature wallet; its room unchanged), pump.fun's programs, the
// settings, and the named coins with their pump.fun / PumpSwap accounts.
// Sends nothing.
//   node mainnet-copy.mjs <output dir> --stand-in <address> --trader <address> --coins <mint,mint,...> --mainnet yes
// Prints the validator's arguments, one per line.
import { Keypair, PublicKey } from "@solana/web3.js";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { BURR, WITHDRAW_AUTHORITY, burnBurrIx, burnCurveIx, burnPoolIx, collectCurveIx, collectPoolIx, curveOf, launchIx, payOutIx, poolOf, splitIx } from "../coin.mjs";
import {
  address, ata, ATA_PROGRAM, COMPUTE_BUDGET, configAddress, connect, decodeAccount, fail, fixedAccounts, loadIdl, LOADER, MAYHEM, need,
  options, pda, PUMP, PUMP_AMM, PUMP_FEES, readAccount, SYSTEM, TOKEN, TOKEN_2022, ZEC,
} from "../lib.mjs";

const opts = options(["url", "mainnet", "stand-in", "trader", "coins", "admin-stand-in", "rewards-stand-in"]);
const out = opts._[0] ?? fail("give the output folder");
const conn = await connect(opts);
if (!conn.isMainnet) fail("this copies from the main network; add --mainnet yes with a main-network connection");
const idl = loadIdl();
const standIn = address(need(opts, "stand-in"), "--stand-in");
const trader = address(need(opts, "trader"), "--trader");
const mints = need(opts, "coins").split(",").map((m) => address(m, "a coin"));
mkdirSync(out, { recursive: true });

// Every account the rehearsal's calls name, worked out with the kit's own builders.
const keys = new Set(fixedAccounts(idl).map((k) => k.toBase58()));
const add = (ix) => ix.keys.forEach((k) => keys.add(k.pubkey.toBase58()));
for (const mint of mints) {
  const coin = await readAccount(conn, idl, "Coin", pda(["coin", mint], idl.programId));
  if (!coin) fail(`${mint.toBase58()} is not a Burrpad coin`);
  const curve = await conn.getAccountInfo(curveOf(mint));
  const graduated = !!curve && curve.data[48] !== 0;
  add(collectCurveIx(idl, mint, trader));
  add(splitIx(idl, mint));
  for (const [payee, receiver] of [["Platform", coin.platform_receiver], ["Creator", coin.creator_receiver], ["Give", coin.give_receiver]]) {
    if (receiver !== PublicKey.default.toBase58()) add(payOutIx(idl, mint, trader, payee, new PublicKey(receiver)));
  }
  if (graduated) {
    add(collectPoolIx(idl, mint, trader));
    add(burnPoolIx(idl, mint, trader));
    keys.add(pda(["pool_lp_mint", poolOf(mint)], PUMP_AMM).toBase58());
  } else {
    add(burnCurveIx(idl, mint, trader));
  }
}
add(await launchIx(conn, idl, Keypair.generate().publicKey, trader, {
  name: "x", symbol: "x", uri: "x", fee_bps: 200, shares: [2500, 2500, 2500, 2500], creator_receiver: trader, give_receiver: trader,
}));
add(burnBurrIx(idl, BURR, trader));
for (const [seeds, program] of [[["global_volume_accumulator"], PUMP], [["global_volume_accumulator"], PUMP_AMM]]) keys.add(pda(seeds, program).toBase58());
keys.add(WITHDRAW_AUTHORITY.toBase58());

const programs = [idl.programId, PUMP, PUMP_AMM, PUMP_FEES, MAYHEM];
const skip = new Set([...programs, TOKEN, TOKEN_2022, ATA_PROGRAM, SYSTEM, COMPUTE_BUDGET, LOADER].map((k) => new PublicKey(k).toBase58()));
const wanted = [...keys].filter((k) => !skip.has(k) && !k.startsWith("Sysvar"));

const args = [];
const writeAccount = (name, pubkey, owner, lamports, data) => {
  const file = path.join(out, name + ".json");
  writeFileSync(file, `{"pubkey":"${pubkey}","account":{"lamports":${lamports},"data":["${data.toString("base64")}","base64"],` +
    `"owner":"${owner}","executable":false,"rentEpoch":18446744073709551615,"space":${data.length}}}\n`);
  args.push("--account", pubkey, file);
};
let copied = 0;
for (let i = 0; i < wanted.length; i += 100) {
  const batch = wanted.slice(i, i + 100);
  const infos = await conn.getMultipleAccountsInfo(batch.map((k) => new PublicKey(k)));
  infos.forEach((info, j) => {
    if (!info || info.executable) return;
    // A local validator asks more rent than the main network (6,960 lamports a byte against 5,080 read
    // today), so an account copied as it is can fall short there. Topped up to the local minimum.
    const local = (info.data.length + 128) * 6960;
    // With --admin-stand-in yes, the stand-in also becomes the settings' administrator (bytes 9 to 41), as
    // the multi-signature wallet's vault is on the network, so the rehearsal can use administrator switches.
    let data = info.data;
    if (opts["admin-stand-in"] === "yes" && batch[j] === configAddress(idl).toBase58()) {
      data = Buffer.from(data);
      standIn.toBuffer().copy(data, 9);
    }
    // With --rewards-stand-in <address>, that throwaway key becomes the rewards wallet (bytes 105 to 137), so
    // the rehearsal can release live coins' holder pots, which only the rewards wallet may do.
    if (opts["rewards-stand-in"] && batch[j] === configAddress(idl).toBase58()) {
      data = Buffer.from(data);
      address(opts["rewards-stand-in"], "--rewards-stand-in").toBuffer().copy(data, 105);
    }
    // A local validator's slots start near 0, but a live coin's ledger records the main network's slot of its
    // last holder release, which a new release's snapshot must come after. On the copy only, that slot is set
    // to 0 so a release can be rehearsed; nothing else in the ledger is touched.
    if (opts["rewards-stand-in"] && info.owner.equals(idl.programId)) {
      const ledger = (() => { try { return decodeAccount(idl, "Ledger", info.data); } catch { return null; } })();
      if (ledger && ledger.holders && BigInt(ledger.holders.last_release_slot) > 0n && batch[j] === pda(["ledger", new PublicKey(ledger.mint)], idl.programId).toBase58()) {
        const want = Buffer.alloc(8);
        want.writeBigUInt64LE(BigInt(ledger.holders.last_release_slot));
        const at = data.indexOf(want);
        if (at < 0 || data.indexOf(want, at + 1) >= 0) fail(`cannot place the last release slot of ledger ${batch[j]}`);
        data = Buffer.from(data);
        Buffer.alloc(8).copy(data, at);
      }
    }
    writeAccount("a" + (i + j), batch[j], info.owner.toBase58(), Math.max(info.lamports, local), data);
    copied++;
  });
}
// Programs: their code as on the network. Burrpad's room is kept exactly; its upgrade key becomes the stand-in.
for (const p of programs) {
  const id = new PublicKey(p);
  const data = await conn.getAccountInfo(pda([id], LOADER));
  if (!data) fail(`no program data for ${id.toBase58()}`);
  const file = path.join(out, id.toBase58() + ".so");
  writeFileSync(file, data.data.subarray(45));
  args.push("--upgradeable-program", id.toBase58(), file, id.equals(new PublicKey(idl.programId)) ? standIn.toBase58() : "none");
}
// 100,000 test ZEC for the trader: a token account that exists only on the local validator.
const token = Buffer.alloc(165);
ZEC.toBuffer().copy(token, 0);
trader.toBuffer().copy(token, 32);
token.writeBigUInt64LE(100_000n * 100_000_000n, 64);
token[108] = 1;
writeAccount("trader_zec", ata(trader, ZEC, TOKEN).toBase58(), TOKEN.toBase58(), 2039280, token);
// 10 million test $BURR for the trader, for coins priced in $BURR: a Token-2022 account that exists only
// locally, laid out as the standard one is (165 bytes, the account-type byte, "immutable owner": 170).
const burrToken = Buffer.alloc(170);
burrToken[165] = 2;
burrToken.writeUInt16LE(7, 166);
BURR.toBuffer().copy(burrToken, 0);
trader.toBuffer().copy(burrToken, 32);
burrToken.writeBigUInt64LE(10_000_000n * 1_000_000n, 64);
burrToken[108] = 1;
writeAccount("trader_burr", ata(trader, BURR, TOKEN_2022).toBase58(), TOKEN_2022.toBase58(), 2074080, burrToken);
console.error(`copied ${copied} accounts and ${programs.length} programs; settings ${configAddress(idl).toBase58()}`);
console.log(args.join("\n"));
