// Builders for everything that happens to one coin: Burrpad's instructions
// and the pump.fun trades around them. Used by the first-coin check and the
// rehearsal, and a worked example for a site. Same account lists as
// tests/src/kit.rs and tests/src/pump.rs.
import { PublicKey, TransactionInstruction } from "@solana/web3.js";
import { createHash } from "node:crypto";
import { ata, ATA_PROGRAM, BUYBACK, coinAddresses, configAddress, instruction, MAYHEM, pda, PUMP, PUMP_AMM, PUMP_FEES, readAccount,
  SYSTEM, TOKEN, TOKEN_2022, ZEC } from "./lib.mjs";

const pumpDisc = (name) => createHash("sha256").update("global:" + name).digest().subarray(0, 8);
const u64 = (v) => { if (v === undefined) throw new Error("an amount was not given"); const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(v)); return b; };
const meta = (pubkey, isWritable = false, isSigner = false) => ({ pubkey, isWritable, isSigner });
const CLOCK = new PublicKey("SysvarC1ock11111111111111111111111111111111");

export const curveOf = (mint) => pda(["bonding-curve", mint], PUMP);

// The clock the programs themselves see.
export async function clock(conn) {
  const d = (await conn.getAccountInfo(CLOCK)).data;
  return { slot: d.readBigUInt64LE(0), time: d.readBigInt64LE(32) };
}
export async function tokenBalance(conn, account) {
  const info = await conn.getAccountInfo(account);
  return info ? info.data.readBigUInt64LE(64) : 0n;
}
export const supplyOf = async (conn, mint) => (await conn.getAccountInfo(mint)).data.readBigUInt64LE(36);
export const zecAccount = (owner) => ata(owner, ZEC, TOKEN);
export const coinAccount = (owner, mint) => ata(owner, mint, TOKEN_2022);

// `plan`: name, symbol, uri, fee_bps, shares [creator, give, burn, holders], creator_receiver, give_receiver.
// The platform terms are read from the settings, so the launch states the terms it saw.
// `plan.burr_share`, `plan.liquidity_share`, `plan.payout_options` and `plan.shadow_share` (optional):
// the extra hooks and the payout choices, sent with `launch_v2`, `launch_v3` or `launch_v4`;
// without them it is the plain `launch`.
// `makeOwnAccount`: the launch also makes the launcher's own account for the
// new coin (after the next upgrade), so a first buy needs no instruction of its own for it.
export async function launchIx(conn, idl, mint, payer, plan, { makeOwnAccount = false } = {}) {
  const cfg = await readAccount(conn, idl, "Config", configAddress(idl));
  const a = coinAddresses(idl, mint);
  const curve = curveOf(mint);
  const solVault = pda(["sol-vault"], MAYHEM);
  const { burr_share = 0, liquidity_share = 0, payout_options = 0, long_holder_step_bps = 0, long_holder_max_bps = 0, shadow_share = 0, ...terms } = plan;
  const name = shadow_share ? "launch_v4" : liquidity_share || payout_options ? "launch_v3" : burr_share ? "launch_v2" : "launch";
  const ix = instruction(idl, name, {
    payer, mint, config: configAddress(idl), coin: a.coin, ledger: a.ledger, authority: a.authority, vault: a.vault, zec_mint: ZEC,
    pump_mint_authority: pda(["mint-authority"], PUMP),
    bonding_curve: curve,
    associated_bonding_curve: ata(curve, mint, TOKEN_2022),
    pump_global: pda(["global"], PUMP),
    mayhem_program: MAYHEM,
    mayhem_global_params: pda(["global-params"], MAYHEM),
    mayhem_sol_vault: solVault,
    mayhem_state: pda(["mayhem-state", mint], MAYHEM),
    mayhem_token_vault: ata(solVault, mint, TOKEN_2022),
    pump_event_authority: pda(["__event_authority"], PUMP),
    associated_quote_bonding_curve: ata(curve, ZEC, TOKEN),
    quote_control: pda(["quote-control"], PUMP),
    pump_program: PUMP, token_program: TOKEN, token_2022_program: TOKEN_2022,
    associated_token_program: ATA_PROGRAM, system_program: SYSTEM,
  }, { args: { ...terms, max_platform_fraction_bps: cfg.platform_fraction_bps, platform_receiver: cfg.platform_receiver }, burr_share, liquidity_share,
    payout: { options: payout_options, long_holder_step_bps, long_holder_max_bps }, shadow_share });
  if (makeOwnAccount) ix.keys.push(meta(coinAccount(payer, mint), true));
  return ix;
}

// Makes `owner`'s account for a token if it is not there yet.
export const makeTokenAccountIx = (payer, owner, mint, tokenProgram) => new TransactionInstruction({
  programId: ATA_PROGRAM, data: Buffer.from([1]),
  keys: [meta(payer, true, true), meta(ata(owner, mint, tokenProgram), true), meta(owner), meta(mint), meta(SYSTEM), meta(tokenProgram)],
});

const tradeKeys = (mint, user) => {
  const curve = curveOf(mint);
  return [
    meta(pda(["global"], PUMP)), meta(mint), meta(ZEC), meta(TOKEN_2022), meta(TOKEN), meta(curve, true),
    meta(ata(curve, mint, TOKEN_2022), true), meta(ata(curve, ZEC, TOKEN), true), meta(user, true, true),
    meta(coinAccount(user, mint), true), meta(zecAccount(user), true), meta(pda(["user_volume_accumulator", user], PUMP), true),
    meta(pda(["fee_config", PUMP], PUMP_FEES)), meta(ata(BUYBACK, ZEC, TOKEN), true), meta(SYSTEM),
    meta(pda(["__event_authority"], PUMP)), meta(PUMP),
  ];
};
// pump.fun's own buy on the curve: spend `zec` units of ZEC, accept no fewer than `minCoins`.
// There is no default for `minCoins`: a real buy must work it out from the
// curve with a stated tolerance, or anyone ordering transactions can take the difference.
export const buyIx = (mint, user, zec, minCoins) => new TransactionInstruction({
  programId: PUMP, keys: tradeKeys(mint, user),
  data: Buffer.concat([pumpDisc("buy_exact_quote_in_v3"), u64(zec), u64(minCoins), Buffer.from([0])]),
});
// pump.fun's own sell on the curve.
export const sellIx = (mint, user, coins, minZec) => new TransactionInstruction({
  programId: PUMP, keys: tradeKeys(mint, user),
  data: Buffer.concat([pumpDisc("sell_v3"), u64(coins), u64(minZec)]),
});
// A trade straight on a PumpSwap pool, as an ordinary trader makes it.
const poolTradeKeys = (mint, user) => {
  const pool = poolOf(mint);
  return [
    meta(pool, true), meta(user, true, true), meta(pda(["global_config"], PUMP_AMM)), meta(mint), meta(ZEC),
    meta(ata(user, mint, TOKEN_2022), true), meta(ata(user, ZEC, TOKEN), true),
    meta(ata(pool, mint, TOKEN_2022), true), meta(ata(pool, ZEC, TOKEN), true),
    meta(TOKEN_2022), meta(TOKEN), meta(SYSTEM),
    meta(pda(["user_volume_accumulator", user], PUMP_AMM), true), meta(pda(["fee_config", PUMP_AMM], PUMP_FEES)),
    meta(ata(BUYBACK, ZEC, TOKEN), true), meta(pda(["__event_authority"], PUMP_AMM)), meta(PUMP_AMM),
  ];
};
export const poolBuyIx = (mint, user, zec, minCoins) => new TransactionInstruction({
  programId: PUMP_AMM, keys: poolTradeKeys(mint, user), data: Buffer.concat([pumpDisc("buy_exact_quote_in_v2"), u64(zec), u64(minCoins)]),
});
export const poolSellIx = (mint, user, coins, minZec) => new TransactionInstruction({
  programId: PUMP_AMM, keys: poolTradeKeys(mint, user), data: Buffer.concat([pumpDisc("sell_v2"), u64(coins), u64(minZec)]),
});
// A plain transfer of a Token-2022 coin between `owner`'s account and `to`'s (which must exist).
export const coinTransferIx = (mint, owner, to, amount, decimals) => new TransactionInstruction({
  programId: TOKEN_2022, data: Buffer.concat([Buffer.from([12]), u64(amount), Buffer.from([decimals])]),
  keys: [meta(coinAccount(owner, mint), true), meta(mint), meta(coinAccount(to, mint), true), meta(owner, false, true)],
});
// A plain ZEC transfer between two existing ZEC accounts.
export const zecTransferIx = (from, to, owner, amount) => new TransactionInstruction({
  programId: TOKEN, data: Buffer.concat([Buffer.from([3]), u64(amount)]), keys: [meta(from, true), meta(to, true), meta(owner, false, true)],
});

export function collectCurveIx(idl, mint, payer) {
  const a = coinAddresses(idl, mint);
  const curve = curveOf(mint);
  const creatorVault = pda(["creator-vault", a.authority], PUMP);
  return instruction(idl, "collect_curve", {
    payer, mint, coin: a.coin, ledger: a.ledger, authority: a.authority, vault: a.vault, zec_mint: ZEC,
    pump_global: pda(["global"], PUMP), bonding_curve: curve, associated_quote_bonding_curve: ata(curve, ZEC, TOKEN),
    creator_vault: creatorVault, creator_vault_ata: ata(creatorVault, ZEC, TOKEN),
    pump_event_authority: pda(["__event_authority"], PUMP), pump_program: PUMP,
    token_program: TOKEN, associated_token_program: ATA_PROGRAM, system_program: SYSTEM,
  });
}
export function splitIx(idl, mint) {
  const a = coinAddresses(idl, mint);
  return instruction(idl, "split", { mint, coin: a.coin, ledger: a.ledger, authority: a.authority, vault: a.vault, zec_mint: ZEC, token_program: TOKEN });
}
// `payee` is "Platform", "Creator" or "Give"; `receiver` must be the one in the coin's record.
export function payOutIx(idl, mint, payer, payee, receiver) {
  const a = coinAddresses(idl, mint);
  return instruction(idl, "pay_out", {
    payer, mint, coin: a.coin, ledger: a.ledger, authority: a.authority, vault: a.vault, zec_mint: ZEC,
    receiver, receiver_zec: zecAccount(receiver),
    token_program: TOKEN, associated_token_program: ATA_PROGRAM, system_program: SYSTEM,
  }, { payee });
}
export function burnCurveIx(idl, mint, payer) {
  const a = coinAddresses(idl, mint);
  const curve = curveOf(mint);
  return instruction(idl, "burn_curve", {
    payer, mint, coin: a.coin, ledger: a.ledger, authority: a.authority, vault: a.vault, zec_mint: ZEC,
    burner: a.burner, burner_zec: zecAccount(a.burner), burner_coin: coinAccount(a.burner, mint),
    pump_global: pda(["global"], PUMP), bonding_curve: curve,
    associated_base_bonding_curve: ata(curve, mint, TOKEN_2022), associated_quote_bonding_curve: ata(curve, ZEC, TOKEN),
    user_volume: pda(["user_volume_accumulator", a.burner], PUMP), fee_config: pda(["fee_config", PUMP], PUMP_FEES),
    buyback_fee_recipient: ata(BUYBACK, ZEC, TOKEN), pump_event_authority: pda(["__event_authority"], PUMP), pump_program: PUMP,
    token_program: TOKEN, token_2022_program: TOKEN_2022, associated_token_program: ATA_PROGRAM, system_program: SYSTEM,
  });
}
// pump.fun's `migrate_v2`: moves a completed curve to its PumpSwap pool. Open to anyone; `user` pays.
export const WITHDRAW_AUTHORITY = new PublicKey("39azUYFWPz3VHgKCf3VChUwbpURdCHRxjWVowf5jUJjg");
export function migrateIx(mint, user) {
  const curve = curveOf(mint);
  const pa = pda(["pool-authority", mint], PUMP);
  const pool = pda(["pool", Buffer.from([0, 0]), pa, mint, ZEC], PUMP_AMM);
  const lp = pda(["pool_lp_mint", pool], PUMP_AMM);
  const boost = pda(["boost_vault", pool], PUMP_AMM);
  return new TransactionInstruction({
    programId: PUMP, data: Buffer.from([187, 203, 18, 31, 206, 237, 254, 41]),
    keys: [
      meta(pda(["global"], PUMP)), meta(WITHDRAW_AUTHORITY, true), meta(mint), meta(ZEC), meta(curve, true),
      meta(ata(curve, mint, TOKEN_2022), true), meta(ata(curve, ZEC, TOKEN), true), meta(user, true, true), meta(SYSTEM), meta(PUMP_AMM),
      meta(pool, true), meta(pa, true), meta(ata(pa, mint, TOKEN_2022), true), meta(ata(pa, ZEC, TOKEN), true),
      meta(pda(["global_config"], PUMP_AMM)), meta(lp, true), meta(ata(pa, lp, TOKEN_2022), true),
      meta(ata(pool, mint, TOKEN_2022), true), meta(ata(pool, ZEC, TOKEN), true),
      meta(TOKEN_2022), meta(TOKEN), meta(TOKEN_2022), meta(ATA_PROGRAM), meta(pda(["__event_authority"], PUMP_AMM)),
      meta(new PublicKey("SysvarRent111111111111111111111111111111111")), meta(pda(["__event_authority"], PUMP)), meta(PUMP),
      // Two accounts the interface file does not list, learned from a real graduation.
      meta(boost, true), meta(ata(boost, ZEC, TOKEN), true),
    ],
  });
}


// ---- after graduation: the coin's PumpSwap pool ----
export const BURR = new PublicKey("GiwMfYS4jHi256G8K41snZhKUovrMbyQL2hqv3fqr7WU");
export const poolAuthorityOf = (mint) => pda(["pool-authority", mint], PUMP);
export const poolOf = (mint) => pda(["pool", Buffer.from([0, 0]), poolAuthorityOf(mint), mint, ZEC], PUMP_AMM);
const poolAccounts = (mint) => {
  const pool = poolOf(mint);
  return {
    amm_global_config: pda(["global_config"], PUMP_AMM), pool_authority: poolAuthorityOf(mint), pool,
    pool_base_token_account: ata(pool, mint, TOKEN_2022), pool_quote_token_account: ata(pool, ZEC, TOKEN),
    amm_event_authority: pda(["__event_authority"], PUMP_AMM), amm_program: PUMP_AMM,
  };
};
export function collectPoolIx(idl, mint, payer) {
  const a = coinAddresses(idl, mint);
  const p = poolAccounts(mint);
  const va = pda(["creator_vault", a.authority], PUMP_AMM);
  return instruction(idl, "collect_pool", {
    payer, mint, coin: a.coin, ledger: a.ledger, authority: a.authority, vault: a.vault, zec_mint: ZEC,
    amm_global_config: p.amm_global_config, pool_authority: p.pool_authority, pool: p.pool,
    pool_quote_token_account: p.pool_quote_token_account, amm_creator_vault_authority: va, amm_creator_vault_ata: ata(va, ZEC, TOKEN),
    amm_event_authority: p.amm_event_authority, amm_program: PUMP_AMM,
    token_program: TOKEN, associated_token_program: ATA_PROGRAM, system_program: SYSTEM,
  });
}
const buyOnPoolAccounts = (idl, mint, payer, burnerMint) => {
  const a = coinAddresses(idl, mint);
  return {
    payer, mint, coin: a.coin, ledger: a.ledger, authority: a.authority, vault: a.vault, zec_mint: ZEC,
    burner: a.burner, burner_zec: zecAccount(a.burner), burner_coin: coinAccount(a.burner, burnerMint),
    ...poolAccounts(burnerMint),
    user_volume: pda(["user_volume_accumulator", a.burner], PUMP_AMM), fee_config: pda(["fee_config", PUMP_AMM], PUMP_FEES),
    buyback_fee_recipient: ata(BUYBACK, ZEC, TOKEN),
    token_program: TOKEN, token_2022_program: TOKEN_2022, associated_token_program: ATA_PROGRAM, system_program: SYSTEM,
  };
};
// The liquidity hook: after graduation, adds to the coin's own pool and burns the receipts.
export function addLiquidityIx(idl, mint, payer) {
  const x = buyOnPoolAccounts(idl, mint, payer, mint);
  const lp = pda(["pool_lp_mint", poolOf(mint)], PUMP_AMM);
  return instruction(idl, "add_liquidity", { ...x, lp_mint: lp, burner_lp: ata(x.burner, lp, TOKEN_2022) });
}

export const burnPoolIx = (idl, mint, payer) => instruction(idl, "burn_pool", buyOnPoolAccounts(idl, mint, payer, mint));
// A coin's $BURR pot buys $BURR on $BURR's pool (after the 16 Oct upgrade).
export function burnBurrIx(idl, mint, payer) {
  const x = buyOnPoolAccounts(idl, mint, payer, BURR);
  const { burner_coin, ...rest } = x;
  return instruction(idl, "burn_burr", {
    ...rest, config: configAddress(idl), burr_ledger: coinAddresses(idl, BURR).ledger, burr_mint: BURR, burner_burr: burner_coin,
  });
}

// The shadow burn: the same accounts as a $BURR buy, then the coin's watch, then the watched wallets' $BURR
// accounts. `sold` 0 only records.
export function shadowBurnIx(idl, mint, keeper, watched = [], sold = 0n, received = 0n) {
  const ix = burnBurrIx(idl, mint, keeper);
  const shadow = instruction(idl, "shadow_burn", { ...accountsByName(idl, "burn_burr", ix), watch: pda(["shadow", mint], idl.programId) }, { sold, received });
  shadow.keys.push(...watched.map((w) => ({ pubkey: ata(w, BURR, TOKEN_2022), isSigner: false, isWritable: false })));
  return shadow;
}
// The launcher sets or changes a coin's shadow watch: after the listed accounts, the old wallets' $BURR accounts, then the new ones'.
export function setShadowWatchIx(idl, mint, launcher, wallets, oldWallets, keeper, follow) {
  const ix = instruction(idl, "set_shadow_watch", {
    launcher, mint, coin: coinAddresses(idl, mint).coin, watch: pda(["shadow", mint], idl.programId), system_program: SYSTEM,
  }, { wallets, keeper, follow });
  ix.keys.push(...[...oldWallets, ...wallets].map((w) => meta(ata(w, BURR, TOKEN_2022))));
  return ix;
}
// The keeper adds `wallet`, which watched wallet `sender` (number `from` on the list) sent $BURR to.
export const followWalletIx = (idl, mint, keeper, from, sender, wallet) => instruction(idl, "follow_wallet", {
  keeper, mint, watch: pda(["shadow", mint], idl.programId), wallet,
  wallet_account: ata(wallet, BURR, TOKEN_2022), from_account: ata(sender, BURR, TOKEN_2022),
}, { from });
const accountsByName = (idl, name, ix) =>
  Object.fromEntries(idl.instructions.find((x) => x.name === name).accounts.map((a, i) => [a.name, ix.keys[i].pubkey]));

// `amount` of the holder pot, for the snapshot taken at `slot` whose fingerprint is `hash` (32 bytes).
export function releaseIx(idl, mint, rewardsWallet, amount, slot, hash) {
  const a = coinAddresses(idl, mint);
  return instruction(idl, "release_holder_rewards", {
    rewards_wallet: rewardsWallet, config: configAddress(idl), mint, coin: a.coin, ledger: a.ledger, authority: a.authority, vault: a.vault,
    zec_mint: ZEC, rewards_zec: zecAccount(rewardsWallet),
    token_program: TOKEN, associated_token_program: ATA_PROGRAM, system_program: SYSTEM,
  }, { amount, snapshot_slot: slot, snapshot_hash: hash });
}
// A snapshot's fingerprint: sha256 of a fixed text form. `holders` is [[wallet, balance], ...].
export const snapshotHash = (mint, slot, holders) => createHash("sha256").update(JSON.stringify({
  mint: mint.toBase58(), slot: String(slot),
  holders: holders.map(([w, b]) => [String(w), String(b)]).sort((x, y) => (x[0] < y[0] ? -1 : 1)),
})).digest();
