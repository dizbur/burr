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
// `plan.burr_share`, `plan.liquidity_share` and `plan.payout_options` (optional):
// the extra hooks and the payout choices, sent with `launch_v2` or `launch_v3`;
// without them it is the plain `launch`.
// `makeOwnAccount`: the launch also makes the launcher's own account for the
// new coin (after the next upgrade), so a first buy needs no instruction of its own for it.
export async function launchIx(conn, idl, mint, payer, plan, { makeOwnAccount = false } = {}) {
  const cfg = await readAccount(conn, idl, "Config", configAddress(idl));
  const a = coinAddresses(idl, mint);
  const curve = curveOf(mint);
  const solVault = pda(["sol-vault"], MAYHEM);
  const { burr_share = 0, liquidity_share = 0, payout_options = 0, ...terms } = plan;
  const name = liquidity_share || payout_options ? "launch_v3" : burr_share ? "launch_v2" : "launch";
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
  }, { args: { ...terms, max_platform_fraction_bps: cfg.platform_fraction_bps, platform_receiver: cfg.platform_receiver }, burr_share, liquidity_share, payout_options });
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
export const burnPoolIx = (idl, mint, payer) => instruction(idl, "burn_pool", buyOnPoolAccounts(idl, mint, payer, mint));
// A coin's $BURR pot buys $BURR on $BURR's pool (after the 16 Oct upgrade).
export function burnBurrIx(idl, mint, payer) {
  const x = buyOnPoolAccounts(idl, mint, payer, BURR);
  const { burner_coin, ...rest } = x;
  return instruction(idl, "burn_burr", {
    ...rest, config: configAddress(idl), burr_ledger: coinAddresses(idl, BURR).ledger, burr_mint: BURR, burner_burr: burner_coin,
  });
}

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
