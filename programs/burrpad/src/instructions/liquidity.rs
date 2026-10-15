//! The seventh hook: part of a coin's fee added to the coin's own pool for
//! good. After graduation, anyone may call it: it buys the coin with half of
//! what it may use, deposits the coins and the ZEC they match into the pool,
//! and burns the pool receipts, so that liquidity can never be withdrawn.
//!
//! The buy keeps the burn's rules: spend limit, least-out, balance checks,
//! the recorded reserve's step and a wait shared with the coin's own burn.
//! There is no price band, as for the burn since 14 Oct: the deposit is at
//! most twice the buy's limit, so whatever someone could win back from it by
//! pushing the price around a call is a small part of what the push costs
//! them in fees (worked out in docs/plan-liquidity-hook.md).

use crate::constants::*;
use crate::error::BurrpadError;
use crate::events::{LiquidityAdded, LiquidityRecorded};
use crate::instructions::burn::{burn_and_return, fund_burner, pool_reserves, PoolExpect};
use crate::math::{self, Reserves};
use crate::pump;
use crate::state::{Coin, Ledger};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::Token;
use anchor_spl::token_2022::Token2022;
use anchor_spl::token_interface::{self, Burn, Mint, TokenAccount};

#[derive(Accounts)]
pub struct AddLiquidity<'info> {
    /// Pays rent for the burner's accounts the first time.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, mint::token_program = token_2022_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
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
    /// CHECK: the coin's burner, a derived address that makes the trade and the deposit
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
    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = mint,
        associated_token::authority = burner,
        associated_token::token_program = token_2022_program
    )]
    pub burner_coin: Box<InterfaceAccount<'info, TokenAccount>>,
    /// The pool's receipt mint, by its derived address; the pool's record must name it too.
    #[account(
        mut,
        seeds = [pump::SEED_POOL_LP_MINT, pool.key().as_ref()],
        bump,
        seeds::program = pump::PUMP_AMM_ID,
        mint::token_program = token_2022_program
    )]
    pub lp_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = lp_mint,
        associated_token::authority = burner,
        associated_token::token_program = token_2022_program
    )]
    pub burner_lp: Box<InterfaceAccount<'info, TokenAccount>>,

    /// CHECK: derived address of PumpSwap
    #[account(seeds = [pump::SEED_AMM_GLOBAL_CONFIG], bump, seeds::program = pump::PUMP_AMM_ID)]
    pub amm_global_config: UncheckedAccount<'info>,
    /// CHECK: pump.fun's derived address that creates a coin's pool at graduation
    #[account(seeds = [pump::SEED_POOL_AUTHORITY, mint.key().as_ref()], bump, seeds::program = pump::PUMP_ID)]
    pub pool_authority: UncheckedAccount<'info>,
    /// CHECK: the coin's one pool made by graduation, by derived address; read here
    #[account(
        mut,
        seeds = [pump::SEED_AMM_POOL, &pump::CANONICAL_POOL_INDEX, pool_authority.key().as_ref(), mint.key().as_ref(), ZEC_MINT.as_ref()],
        bump,
        seeds::program = pump::PUMP_AMM_ID
    )]
    pub pool: UncheckedAccount<'info>,
    #[account(
        mut,
        associated_token::mint = mint,
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

pub fn handle_add_liquidity(ctx: Context<AddLiquidity>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let mint_key = ctx.accounts.mint.key();
    require!(ctx.accounts.coin.liquidity_share > 0, BurrpadError::HookNotChosen);
    require!(ctx.accounts.ledger.liquidity.pot > 0, BurrpadError::NothingToBurn);
    // One wait with the coin's own burn: the two buy on the same pool.
    let cooldown = ctx.accounts.coin.burn_cooldown_secs as i64;
    let ready = ctx.accounts.ledger.burn.last_time.checked_add(cooldown).ok_or(BurrpadError::Overflow)?;
    require!(now >= ready, BurrpadError::BurnTooSoon);

    let pool_authority_key = ctx.accounts.pool_authority.key();
    let base_account_key = ctx.accounts.pool_base_token_account.key();
    let quote_account_key = ctx.accounts.pool_quote_token_account.key();
    let expect = PoolExpect {
        mint: &mint_key,
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
    let shares = pump::read_pool_shares(&ctx.accounts.pool.to_account_info())?;
    require_keys_eq!(shares.lp_mint, ctx.accounts.lp_mint.key(), BurrpadError::WrongPumpAccount);

    // The first call after graduation only records the pool's reserve, which
    // the limit is then measured against; the pot is untouched and the next
    // call waits the whole minute.
    let last = ctx.accounts.ledger.liquidity.reference;
    if last.quote == 0 {
        let ledger = &mut ctx.accounts.ledger;
        ledger.liquidity.reference = reserves;
        ledger.burn.last_time = now;
        emit!(LiquidityRecorded { mint: mint_key });
        return Ok(());
    }

    // The limit: the buy is held to the burn's limit, measured against the
    // smaller of the reserve now and the one recorded; the deposit, which does
    // not move the price, may use as much again.
    let coin = &ctx.accounts.coin;
    let charged = u16::try_from(venue_fee_bps).unwrap_or(u16::MAX);
    let cap = math::burn_spend_limit(
        u64::MAX,
        reserves.quote.min(last.quote),
        coin.fee_bps.min(charged),
        coin.burn_spend_divisor,
    )
    .ok_or(BurrpadError::Overflow)?;
    let using = ctx.accounts.ledger.liquidity.pot.min(cap.checked_mul(2).ok_or(BurrpadError::Overflow)?);
    let buy = using / 2;
    let min_out =
        math::burn_min_out(&reserves, buy, charged, coin.burn_fee_allowance_bps).ok_or(BurrpadError::Overflow)?;
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
        using,
    )?;
    ctx.accounts.burner_zec.reload()?;
    let zec_before = ctx.accounts.burner_zec.amount;
    let coins_before = ctx.accounts.burner_coin.amount;

    // 1. The buy.
    let a = &ctx.accounts;
    let burner_seeds: &[&[u8]] = &[SEED_BURNER, mint_key.as_ref(), &[a.coin.burner_bump]];
    pump::buy_on_pool(
        pump::PoolBuy {
            amm_program: &a.amm_program.to_account_info(),
            pool: &a.pool.to_account_info(),
            user: &a.burner.to_account_info(),
            global_config: &a.amm_global_config.to_account_info(),
            base_mint: &a.mint.to_account_info(),
            quote_mint: &a.zec_mint.to_account_info(),
            user_base: &a.burner_coin.to_account_info(),
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
        buy,
        min_out,
        &[burner_seeds],
    )?;
    ctx.accounts.burner_zec.reload()?;
    ctx.accounts.burner_coin.reload()?;
    let zec_held = ctx.accounts.burner_zec.amount;
    let coins_held = ctx.accounts.burner_coin.amount;
    let spent_buying = zec_before.checked_sub(zec_held).ok_or(BurrpadError::BurnOverspent)?;
    require!(spent_buying <= buy, BurrpadError::BurnOverspent);
    let bought = coins_held.checked_sub(coins_before).ok_or(BurrpadError::BurnBadFill)?;
    require!(bought >= min_out, BurrpadError::BurnBadFill);

    // 2. The deposit, at the pool's own figures after the buy: its coin
    // balance, and its ZEC balance less the fees waiting in it.
    ctx.accounts.pool_base_token_account.reload()?;
    ctx.accounts.pool_quote_token_account.reload()?;
    let shares = pump::read_pool_shares(&ctx.accounts.pool.to_account_info())?;
    let pool_coins = ctx.accounts.pool_base_token_account.amount;
    let pool_zec =
        ctx.accounts.pool_quote_token_account.amount.checked_sub(shares.fees_waiting).ok_or(BurrpadError::Overflow)?;
    let lp =
        math::lp_for(coins_held, zec_held, pool_coins, pool_zec, shares.lp_supply).ok_or(BurrpadError::Overflow)?;
    require!(lp > 0, BurrpadError::NothingToBurn);
    let want_coins = math::deposit_side(lp, pool_coins, shares.lp_supply).ok_or(BurrpadError::Overflow)?;
    let want_zec = math::deposit_side(lp, pool_zec, shares.lp_supply).ok_or(BurrpadError::Overflow)?;
    let lp_before = ctx.accounts.burner_lp.amount;
    let a = &ctx.accounts;
    pump::deposit_to_pool(
        pump::PoolDeposit {
            amm_program: &a.amm_program.to_account_info(),
            pool: &a.pool.to_account_info(),
            global_config: &a.amm_global_config.to_account_info(),
            user: &a.burner.to_account_info(),
            base_mint: &a.mint.to_account_info(),
            quote_mint: &a.zec_mint.to_account_info(),
            lp_mint: &a.lp_mint.to_account_info(),
            user_base: &a.burner_coin.to_account_info(),
            user_quote: &a.burner_zec.to_account_info(),
            user_lp: &a.burner_lp.to_account_info(),
            pool_base: &a.pool_base_token_account.to_account_info(),
            pool_quote: &a.pool_quote_token_account.to_account_info(),
            token_program: &a.token_program.to_account_info(),
            token_2022_program: &a.token_2022_program.to_account_info(),
            event_authority: &a.amm_event_authority.to_account_info(),
        },
        lp,
        coins_held,
        zec_held,
        &[burner_seeds],
    )?;
    ctx.accounts.burner_zec.reload()?;
    ctx.accounts.burner_coin.reload()?;
    ctx.accounts.burner_lp.reload()?;
    let zec_left = ctx.accounts.burner_zec.amount;
    let coins_left = ctx.accounts.burner_coin.amount;
    let receipts = ctx.accounts.burner_lp.amount;
    let zec_deposited = zec_held.checked_sub(zec_left).ok_or(BurrpadError::DepositOffRatio)?;
    let coins_deposited = coins_held.checked_sub(coins_left).ok_or(BurrpadError::DepositOffRatio)?;
    // The pool may take no more of either side than its own figures ask.
    require!(zec_deposited <= want_zec && coins_deposited <= want_coins, BurrpadError::DepositOffRatio);
    require!(receipts.checked_sub(lp_before).ok_or(BurrpadError::BurnBadFill)? >= lp, BurrpadError::BurnBadFill);

    // 3. Every receipt burned: that liquidity can never be withdrawn.
    let a = &ctx.accounts;
    token_interface::burn(
        CpiContext::new_with_signer(
            a.token_2022_program.key(),
            Burn {
                mint: a.lp_mint.to_account_info(),
                from: a.burner_lp.to_account_info(),
                authority: a.burner.to_account_info(),
            },
            &[burner_seeds],
        ),
        receipts,
    )?;

    // Booked against the seventh pot only.
    let spent = spent_buying.checked_add(zec_deposited).ok_or(BurrpadError::Overflow)?;
    let ledger = &mut ctx.accounts.ledger;
    ledger.liquidity.pot = ledger.liquidity.pot.checked_sub(spent).ok_or(BurrpadError::Overflow)?;
    ledger.accounted = ledger.accounted.checked_sub(spent).ok_or(BurrpadError::Overflow)?;
    ledger.liquidity.total_spent = ledger.liquidity.total_spent.checked_add(spent).ok_or(BurrpadError::Overflow)?;
    ledger.liquidity.total_lp_burned =
        ledger.liquidity.total_lp_burned.checked_add(receipts).ok_or(BurrpadError::Overflow)?;

    // Coin dust burned; ZEC not used back to the vault (it stays in the pot).
    let a = &ctx.accounts;
    burn_and_return(
        &a.token_2022_program.to_account_info(),
        &a.token_program.to_account_info(),
        &a.mint.to_account_info(),
        &a.zec_mint.to_account_info(),
        &a.burner.to_account_info(),
        &a.burner_coin.to_account_info(),
        &a.burner_zec.to_account_info(),
        &a.vault.to_account_info(),
        burner_seeds,
        coins_left,
        zec_left,
    )?;

    // The record moves, and the next call waits, in proportion to the part of
    // the buy's limit used.
    ctx.accounts.pool_base_token_account.reload()?;
    ctx.accounts.pool_quote_token_account.reload()?;
    let (after, _) = pool_reserves(
        &ctx.accounts.pool.to_account_info(),
        &expect,
        ctx.accounts.pool_base_token_account.amount,
        ctx.accounts.pool_quote_token_account.amount,
    )?;
    let wait = math::part_used(spent_buying, cap, cooldown as u64).ok_or(BurrpadError::Overflow)?;
    let step = math::part_used(spent_buying, cap, BURN_RESERVE_STEP_BPS as u64).ok_or(BurrpadError::Overflow)?;
    let quote = math::burr_next_reserve(last.quote, after.quote, step).ok_or(BurrpadError::Overflow)?;
    let base = (after.base as u128).checked_mul(quote as u128).ok_or(BurrpadError::Overflow)? / after.quote as u128;
    let ledger = &mut ctx.accounts.ledger;
    ledger.liquidity.reference =
        Reserves { quote, base: u64::try_from(base.max(1)).map_err(|_| BurrpadError::Overflow)? };
    let held = now.checked_add(wait as i64).ok_or(BurrpadError::Overflow)? - cooldown;
    ledger.burn.last_time = ledger.burn.last_time.max(held);
    emit!(LiquidityAdded {
        mint: mint_key,
        zec_spent_buying: spent_buying,
        zec_deposited,
        coins_deposited,
        lp_burned: receipts
    });
    Ok(())
}
