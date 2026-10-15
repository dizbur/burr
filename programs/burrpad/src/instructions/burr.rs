//! Burn $BURR: a coin's sixth pot buys the platform's own coin, $BURR, on its
//! pool and burns it. Open to anyone.
//!
//! The same rules as a coin's own buy back on a pool, against $BURR's pool:
//! a waiting time, a limit on each spend measured against the pool (and
//! against a recorded reserve that grows 5% per buy at most), a least-coins-out
//! worked out from the pool's figures, and balances checked afterwards. The
//! waiting time and the recorded reserve are shared by all coins (in
//! `Config`): one $BURR buy a minute in all, so that several coins' buys cannot
//! be packed into one moment for someone trading around them.

use crate::constants::*;
use crate::error::BurrpadError;
use crate::events::BurrBurned;
use crate::instructions::burn::{burn_and_return, fund_burner, pool_reserves, PoolExpect};
use crate::math::{self, Reserves};
use crate::pump;
use crate::state::{Coin, Config, Ledger};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::Token;
use anchor_spl::token_2022::Token2022;
use anchor_spl::token_interface::{Mint, TokenAccount};

#[derive(Accounts)]
pub struct BurnBurr<'info> {
    /// Pays rent for the burner's accounts the first time.
    #[account(mut)]
    pub payer: Signer<'info>,
    /// CHECK: only its address is used, to derive the coin's accounts
    pub mint: UncheckedAccount<'info>,
    #[account(seeds = [SEED_COIN, mint.key().as_ref()], bump = coin.bump)]
    pub coin: Box<Account<'info, Coin>>,
    #[account(mut, seeds = [SEED_LEDGER, mint.key().as_ref()], bump = ledger.bump)]
    pub ledger: Box<Account<'info, Ledger>>,
    /// CHECK: the coin's authority, a derived address
    #[account(seeds = [SEED_AUTHORITY, mint.key().as_ref()], bump = coin.authority_bump)]
    pub authority: UncheckedAccount<'info>,
    #[account(
        mut,
        associated_token::mint = zec_mint,
        associated_token::authority = authority,
        associated_token::token_program = token_program
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(address = ZEC_MINT)]
    pub zec_mint: Box<InterfaceAccount<'info, Mint>>,
    /// The waiting time and recorded reserve shared by every coin's $BURR buys.
    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    /// $BURR's own ledger: its own buy back and this one share one wait.
    #[account(mut, seeds = [SEED_LEDGER, BURR_MINT.as_ref()], bump = burr_ledger.bump)]
    pub burr_ledger: Box<Account<'info, Ledger>>,
    /// CHECK: the coin's burner, a derived address that makes the trade
    #[account(mut, seeds = [SEED_BURNER, mint.key().as_ref()], bump = coin.burner_bump)]
    pub burner: UncheckedAccount<'info>,
    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = zec_mint,
        associated_token::authority = burner,
        associated_token::token_program = token_program
    )]
    pub burner_zec: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, address = BURR_MINT, mint::token_program = token_2022_program)]
    pub burr_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = burr_mint,
        associated_token::authority = burner,
        associated_token::token_program = token_2022_program
    )]
    pub burner_burr: Box<InterfaceAccount<'info, TokenAccount>>,

    /// CHECK: derived address of PumpSwap
    #[account(seeds = [pump::SEED_AMM_GLOBAL_CONFIG], bump, seeds::program = pump::PUMP_AMM_ID)]
    pub amm_global_config: UncheckedAccount<'info>,
    /// CHECK: pump.fun's derived address that created $BURR's pool at graduation
    #[account(seeds = [pump::SEED_POOL_AUTHORITY, BURR_MINT.as_ref()], bump, seeds::program = pump::PUMP_ID)]
    pub pool_authority: UncheckedAccount<'info>,
    /// CHECK: $BURR's one pool made by graduation, by derived address; read here
    #[account(
        mut,
        seeds = [pump::SEED_AMM_POOL, &pump::CANONICAL_POOL_INDEX, pool_authority.key().as_ref(), BURR_MINT.as_ref(), ZEC_MINT.as_ref()],
        bump,
        seeds::program = pump::PUMP_AMM_ID
    )]
    pub pool: UncheckedAccount<'info>,
    #[account(
        mut,
        associated_token::mint = burr_mint,
        associated_token::authority = pool,
        associated_token::token_program = token_2022_program
    )]
    pub pool_base_token_account: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        associated_token::mint = zec_mint,
        associated_token::authority = pool,
        associated_token::token_program = token_program
    )]
    pub pool_quote_token_account: Box<InterfaceAccount<'info, TokenAccount>>,
    /// CHECK: PumpSwap's volume record for the burner
    #[account(mut, seeds = [pump::SEED_USER_VOLUME, burner.key().as_ref()], bump, seeds::program = pump::PUMP_AMM_ID)]
    pub user_volume: UncheckedAccount<'info>,
    /// CHECK: pump.fun's fee settings for pools
    #[account(seeds = [pump::SEED_FEE_CONFIG, pump::PUMP_AMM_ID.as_ref()], bump, seeds::program = pump::PUMP_FEES_ID)]
    pub fee_config: UncheckedAccount<'info>,
    /// CHECK: checked in the handler against the list in PumpSwap's settings
    #[account(mut)]
    pub buyback_fee_recipient: UncheckedAccount<'info>,
    /// CHECK: derived address of PumpSwap
    #[account(seeds = [pump::SEED_EVENT_AUTHORITY], bump, seeds::program = pump::PUMP_AMM_ID)]
    pub amm_event_authority: UncheckedAccount<'info>,
    /// CHECK: pinned address
    #[account(address = pump::PUMP_AMM_ID)]
    pub amm_program: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub token_2022_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handle_burn_burr(ctx: Context<BurnBurr>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(ctx.accounts.coin.burr_share > 0, BurrpadError::HookNotChosen);
    require!(ctx.accounts.ledger.burr.pot > 0, BurrpadError::NothingToBurn);
    let shared = ctx.accounts.config.burr_buy;
    // Not before the shared wait is over, nor within a minute of $BURR's own buy back.
    let own =
        ctx.accounts.burr_ledger.burn.last_time.checked_add(BURN_COOLDOWN_SECS as i64).ok_or(BurrpadError::Overflow)?;
    require!(now >= shared.ready_at && now >= own, BurrpadError::BurnTooSoon);

    let pool_authority_key = ctx.accounts.pool_authority.key();
    let base_account_key = ctx.accounts.pool_base_token_account.key();
    let quote_account_key = ctx.accounts.pool_quote_token_account.key();
    let expect = PoolExpect {
        mint: &BURR_MINT,
        pool_authority: &pool_authority_key,
        base_account: &base_account_key,
        quote_account: &quote_account_key,
    };
    let (reserves, venue_fee_bps) = pool_reserves(
        &ctx.accounts.pool.to_account_info(),
        &expect,
        ctx.accounts.pool_base_token_account.amount,
        ctx.accounts.pool_quote_token_account.amount,
    )?;
    // Before any $BURR buy has been recorded, the limit is measured against a
    // fixed starting figure, not against whatever the pool shows at that moment.
    let recorded = if shared.reference.quote == 0 { BURR_START_RESERVE } else { shared.reference.quote };
    // The limit's safety rests on traders paying the fee $BURR's pool charges.
    // It is held to the highest fee a coin may be launched with.
    let charged = u16::try_from(venue_fee_bps).unwrap_or(u16::MAX).min(ctx.accounts.config.max_fee_bps);
    let cap = math::burn_spend_limit(u64::MAX, reserves.quote.min(recorded), charged, BURN_SPEND_DIVISOR)
        .ok_or(BurrpadError::Overflow)?;
    let spend = ctx.accounts.ledger.burr.pot.min(cap);
    let min_out =
        math::burn_min_out(&reserves, spend, charged, BURN_FEE_ALLOWANCE_BPS).ok_or(BurrpadError::Overflow)?;
    require!(min_out > 0, BurrpadError::NothingToBurn);

    let a = &ctx.accounts;
    pump::check_buyback_recipient(
        &a.amm_global_config.to_account_info(),
        &pump::PUMP_AMM_ID,
        &pump::ACC_AMM_GLOBAL_CONFIG,
        pump::AMM_GLOBAL_BUYBACK_OFFSET,
        &a.buyback_fee_recipient.to_account_info(),
        &ZEC_MINT,
        &a.token_program.key(),
    )?;
    if a.user_volume.data_is_empty() {
        pump::init_user_volume(
            &a.amm_program.to_account_info(),
            &pump::PUMP_AMM_ID,
            &a.payer.to_account_info(),
            &a.burner.to_account_info(),
            &a.user_volume.to_account_info(),
            &a.system_program.to_account_info(),
            &a.amm_event_authority.to_account_info(),
        )?;
    }
    fund_burner(
        &a.token_program.to_account_info(),
        &a.vault.to_account_info(),
        &a.zec_mint.to_account_info(),
        &a.burner_zec.to_account_info(),
        &a.authority.to_account_info(),
        &a.coin,
        spend,
    )?;
    ctx.accounts.burner_zec.reload()?;
    let zec_before = ctx.accounts.burner_zec.amount;
    let burr_before = ctx.accounts.burner_burr.amount;

    let a = &ctx.accounts;
    let coin_mint = a.coin.mint;
    let burner_seeds: &[&[u8]] = &[SEED_BURNER, coin_mint.as_ref(), &[a.coin.burner_bump]];
    pump::buy_on_pool(
        pump::PoolBuy {
            amm_program: &a.amm_program.to_account_info(),
            pool: &a.pool.to_account_info(),
            user: &a.burner.to_account_info(),
            global_config: &a.amm_global_config.to_account_info(),
            base_mint: &a.burr_mint.to_account_info(),
            quote_mint: &a.zec_mint.to_account_info(),
            user_base: &a.burner_burr.to_account_info(),
            user_quote: &a.burner_zec.to_account_info(),
            pool_base: &a.pool_base_token_account.to_account_info(),
            pool_quote: &a.pool_quote_token_account.to_account_info(),
            base_token_program: &a.token_2022_program.to_account_info(),
            quote_token_program: &a.token_program.to_account_info(),
            system_program: &a.system_program.to_account_info(),
            user_volume: &a.user_volume.to_account_info(),
            fee_config: &a.fee_config.to_account_info(),
            buyback_fee_recipient: &a.buyback_fee_recipient.to_account_info(),
            event_authority: &a.amm_event_authority.to_account_info(),
        },
        spend,
        min_out,
        &[burner_seeds],
    )?;

    // Checked by balances, and booked against the sixth pot only.
    ctx.accounts.burner_zec.reload()?;
    ctx.accounts.burner_burr.reload()?;
    let zec_after = ctx.accounts.burner_zec.amount;
    let burr_after = ctx.accounts.burner_burr.amount;
    let spent = zec_before.checked_sub(zec_after).ok_or(BurrpadError::BurnOverspent)?;
    require!(spent <= spend, BurrpadError::BurnOverspent);
    let bought = burr_after.checked_sub(burr_before).ok_or(BurrpadError::BurnBadFill)?;
    require!(bought >= min_out, BurrpadError::BurnBadFill);
    let ledger = &mut ctx.accounts.ledger;
    ledger.burr.pot = ledger.burr.pot.checked_sub(spent).ok_or(BurrpadError::Overflow)?;
    ledger.accounted = ledger.accounted.checked_sub(spent).ok_or(BurrpadError::Overflow)?;
    ledger.burr.total_spent = ledger.burr.total_spent.checked_add(spent).ok_or(BurrpadError::Overflow)?;
    ledger.burr.total_burned = ledger.burr.total_burned.checked_add(burr_after).ok_or(BurrpadError::Overflow)?;

    let a = &ctx.accounts;
    burn_and_return(
        &a.token_2022_program.to_account_info(),
        &a.token_program.to_account_info(),
        &a.burr_mint.to_account_info(),
        &a.zec_mint.to_account_info(),
        &a.burner.to_account_info(),
        &a.burner_burr.to_account_info(),
        &a.burner_zec.to_account_info(),
        &a.vault.to_account_info(),
        burner_seeds,
        burr_after,
        zec_after,
    )?;

    ctx.accounts.pool_base_token_account.reload()?;
    ctx.accounts.pool_quote_token_account.reload()?;
    let (after, _) = pool_reserves(
        &ctx.accounts.pool.to_account_info(),
        &expect,
        ctx.accounts.pool_base_token_account.amount,
        ctx.accounts.pool_quote_token_account.amount,
    )?;
    // The next buy waits, and the recorded reserve moves, in proportion to the
    // part of the limit used: crumbs can neither hold the wait nor walk the reserve.
    let wait = math::part_used(spent, cap, BURN_COOLDOWN_SECS as u64).ok_or(BurrpadError::Overflow)?;
    let step = math::part_used(spent, cap, BURN_RESERVE_STEP_BPS as u64).ok_or(BurrpadError::Overflow)?;
    let buy = &mut ctx.accounts.config.burr_buy;
    buy.reference = Reserves {
        quote: math::burr_next_reserve(recorded, after.quote, step).ok_or(BurrpadError::Overflow)?,
        base: after.base,
    };
    buy.ready_at = now.checked_add(wait as i64).ok_or(BurrpadError::Overflow)?;
    // $BURR's own buy back waits as long.
    let held = buy.ready_at - BURN_COOLDOWN_SECS as i64;
    let own = &mut ctx.accounts.burr_ledger.burn;
    own.last_time = own.last_time.max(held);
    emit!(BurrBurned { mint: coin_mint, spent, burned: burr_after });
    Ok(())
}
