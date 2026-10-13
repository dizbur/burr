//! Buy back and burn. Open to anyone.
//!
//! Protections, all worked out by the program from the pool's own figures:
//! a waiting time between calls, a limit on the spend per call measured
//! against the pool, a refusal to buy at a price well above the last one
//! recorded, and a least-coins-out that is checked again from balances.
//! Only the amount set aside for one buy ever leaves the vault for the trade.

use crate::constants::*;
use crate::error::BurrError;
use crate::events::{BurnSkipped, Burned};
use crate::math::{self, Reserves};
use crate::ops;
use crate::pump;
use crate::state::{Coin, Ledger};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::{get_associated_token_address_with_program_id, AssociatedToken};
use anchor_spl::token::Token;
use anchor_spl::token_2022::Token2022;
use anchor_spl::token_interface::{self, Burn, Mint, TokenAccount, TransferChecked};

/// Decides whether to buy now, and how much. `None` means the price check
/// refused: the new price and time are recorded and nothing is bought.
///
/// `venue_fee_bps` is the creator fee the curve or pool charges right now,
/// read from pump.fun's own account. The spend limit uses the lower of that
/// and the coin's recorded fee, because the limit's safety rests on traders
/// really paying that fee. The least-coins-out allows for what is charged.
fn plan(
    coin: &Coin,
    ledger: &mut Ledger,
    reserves: Reserves,
    venue_fee_bps: u64,
    now: i64,
) -> Result<Option<(u64, u64)>> {
    require!(coin.share_of(SINK_BURN) > 0, BurrError::HookNotChosen);
    // Nothing is recorded unless there is something to spend, so an empty
    // pot cannot be used to move the reference price.
    require!(ledger.pots[SINK_BURN] > 0, BurrError::NothingToBurn);
    let b = &mut ledger.burn;
    let ready = b.last_time.checked_add(coin.burn_cooldown_secs as i64).ok_or(BurrError::Overflow)?;
    require!(now >= ready, BurrError::BurnTooSoon);
    if reserves.above(&b.reference, coin.burn_price_band_bps).ok_or(BurrError::Overflow)? {
        // Do not take the price seen now as the new reference: it may have
        // been pushed for this one call. Raise the reference by one band
        // only. A real rise is followed step by step, one waiting time each.
        b.reference = b.reference.raised_by(coin.burn_price_band_bps).ok_or(BurrError::Overflow)?;
        b.last_time = now;
        emit!(BurnSkipped { mint: coin.mint });
        return Ok(None);
    }
    // The limit is measured against the smaller of the reserve now and the
    // reserve recorded last time, so liquidity added just for this call
    // cannot raise it.
    let charged = u16::try_from(venue_fee_bps).unwrap_or(u16::MAX);
    let spend = math::burn_spend_limit(
        ledger.pots[SINK_BURN],
        reserves.quote.min(b.reference.quote),
        coin.fee_bps.min(charged),
        coin.burn_spend_divisor,
    )
    .ok_or(BurrError::Overflow)?;
    let min_out =
        math::burn_min_out(&reserves, spend, charged, coin.burn_fee_allowance_bps).ok_or(BurrError::Overflow)?;
    // Zero here means nothing can be spent now (no fee is being charged, or
    // the amount is too small to buy one unit).
    require!(min_out > 0, BurrError::NothingToBurn);
    Ok(Some((spend, min_out)))
}

#[derive(Accounts)]
pub struct BurnCurve<'info> {
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
    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = mint,
        associated_token::authority = burner,
        associated_token::token_program = token_2022_program
    )]
    pub burner_coin: Box<InterfaceAccount<'info, TokenAccount>>,

    /// CHECK: derived address of pump.fun
    #[account(seeds = [pump::SEED_GLOBAL], bump, seeds::program = pump::PUMP_ID)]
    pub pump_global: UncheckedAccount<'info>,
    /// CHECK: derived address of pump.fun; read here
    #[account(mut, seeds = [pump::SEED_BONDING_CURVE, mint.key().as_ref()], bump, seeds::program = pump::PUMP_ID)]
    pub bonding_curve: UncheckedAccount<'info>,
    /// CHECK: the curve's coin account
    #[account(mut, address = get_associated_token_address_with_program_id(&bonding_curve.key(), &mint.key(), &token_2022_program.key()))]
    pub associated_base_bonding_curve: UncheckedAccount<'info>,
    /// CHECK: the curve's ZEC account
    #[account(mut, address = get_associated_token_address_with_program_id(&bonding_curve.key(), &ZEC_MINT, &token_program.key()))]
    pub associated_quote_bonding_curve: UncheckedAccount<'info>,
    /// CHECK: pump.fun's volume record for the burner
    #[account(mut, seeds = [pump::SEED_USER_VOLUME, burner.key().as_ref()], bump, seeds::program = pump::PUMP_ID)]
    pub user_volume: UncheckedAccount<'info>,
    /// CHECK: pump.fun's fee settings for the curve
    #[account(seeds = [pump::SEED_FEE_CONFIG, pump::PUMP_ID.as_ref()], bump, seeds::program = pump::PUMP_FEES_ID)]
    pub fee_config: UncheckedAccount<'info>,
    /// CHECK: checked in the handler against the list in pump.fun's settings
    #[account(mut)]
    pub buyback_fee_recipient: UncheckedAccount<'info>,
    /// CHECK: derived address of pump.fun
    #[account(seeds = [pump::SEED_EVENT_AUTHORITY], bump, seeds::program = pump::PUMP_ID)]
    pub pump_event_authority: UncheckedAccount<'info>,
    /// CHECK: pinned address
    #[account(address = pump::PUMP_ID)]
    pub pump_program: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub token_2022_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handle_burn_curve(ctx: Context<BurnCurve>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let curve = pump::read_curve(&ctx.accounts.bonding_curve.to_account_info())?;
    require!(!curve.complete && curve.quote_mint == ZEC_MINT, BurrError::WrongVenue);
    let reserves = Reserves { quote: curve.virtual_quote_reserves, base: curve.virtual_token_reserves };
    let Some((spend, min_out)) =
        plan(&ctx.accounts.coin, &mut ctx.accounts.ledger, reserves, curve.creator_fee_bps, now)?
    else {
        return Ok(());
    };

    let a = &ctx.accounts;
    pump::check_buyback_recipient(
        &a.pump_global.to_account_info(),
        &pump::PUMP_ID,
        &pump::ACC_GLOBAL,
        pump::GLOBAL_BUYBACK_OFFSET,
        &a.buyback_fee_recipient.to_account_info(),
        &ZEC_MINT,
        &a.token_program.key(),
    )?;
    if a.user_volume.data_is_empty() {
        pump::init_user_volume(
            &a.pump_program.to_account_info(),
            &pump::PUMP_ID,
            &a.payer.to_account_info(),
            &a.burner.to_account_info(),
            &a.user_volume.to_account_info(),
            &a.system_program.to_account_info(),
            &a.pump_event_authority.to_account_info(),
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
    let coin_before = ctx.accounts.burner_coin.amount;

    let a = &ctx.accounts;
    let mint_key = a.mint.key();
    let burner_seeds: &[&[u8]] = &[SEED_BURNER, mint_key.as_ref(), &[a.coin.burner_bump]];
    pump::buy_on_curve(
        pump::CurveBuy {
            pump_program: &a.pump_program.to_account_info(),
            global: &a.pump_global.to_account_info(),
            base_mint: &a.mint.to_account_info(),
            quote_mint: &a.zec_mint.to_account_info(),
            base_token_program: &a.token_2022_program.to_account_info(),
            quote_token_program: &a.token_program.to_account_info(),
            bonding_curve: &a.bonding_curve.to_account_info(),
            associated_base_bonding_curve: &a.associated_base_bonding_curve.to_account_info(),
            associated_quote_bonding_curve: &a.associated_quote_bonding_curve.to_account_info(),
            user: &a.burner.to_account_info(),
            user_base: &a.burner_coin.to_account_info(),
            user_quote: &a.burner_zec.to_account_info(),
            user_volume: &a.user_volume.to_account_info(),
            fee_config: &a.fee_config.to_account_info(),
            buyback_fee_recipient: &a.buyback_fee_recipient.to_account_info(),
            system_program: &a.system_program.to_account_info(),
            event_authority: &a.pump_event_authority.to_account_info(),
        },
        spend,
        min_out,
        &[burner_seeds],
    )?;

    ctx.accounts.burner_zec.reload()?;
    ctx.accounts.burner_coin.reload()?;
    let (spent, burned) = finish(
        &mut ctx.accounts.ledger,
        spend,
        min_out,
        zec_before,
        coin_before,
        ctx.accounts.burner_zec.amount,
        ctx.accounts.burner_coin.amount,
    )?;
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
        a.burner_coin.amount,
        a.burner_zec.amount,
    )?;

    // Record the price after our own buy as the next reference.
    let after = pump::read_curve(&ctx.accounts.bonding_curve.to_account_info())?;
    let band = ctx.accounts.coin.burn_price_band_bps;
    let b = &mut ctx.accounts.ledger.burn;
    b.reference = next_reference(
        &b.reference,
        Reserves { quote: after.virtual_quote_reserves, base: after.virtual_token_reserves },
        band,
    )?;
    b.last_time = now;
    emit!(Burned { mint: mint_key, spent, burned });
    Ok(())
}

#[derive(Accounts)]
pub struct BurnPool<'info> {
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
    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = mint,
        associated_token::authority = burner,
        associated_token::token_program = token_2022_program
    )]
    pub burner_coin: Box<InterfaceAccount<'info, TokenAccount>>,

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

/// The pool's price figures and the creator fee it charges, from its own accounts.
pub fn pool_reserves(
    pool: &AccountInfo,
    mint: &Pubkey,
    base_amount: u64,
    quote_amount: u64,
) -> Result<(Reserves, u64)> {
    let p = pump::read_pool(pool)?;
    require!(p.base_mint == *mint && p.quote_mint == ZEC_MINT, BurrError::WrongPumpAccount);
    let quote = pump::effective_quote_reserves(quote_amount, p.virtual_quote_reserves)?;
    Ok((Reserves { quote, base: base_amount }, p.creator_fee_bps))
}

pub fn handle_burn_pool(ctx: Context<BurnPool>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let mint_key = ctx.accounts.mint.key();
    let (reserves, venue_fee_bps) = pool_reserves(
        &ctx.accounts.pool.to_account_info(),
        &mint_key,
        ctx.accounts.pool_base_token_account.amount,
        ctx.accounts.pool_quote_token_account.amount,
    )?;
    let Some((spend, min_out)) = plan(&ctx.accounts.coin, &mut ctx.accounts.ledger, reserves, venue_fee_bps, now)?
    else {
        return Ok(());
    };

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
    let coin_before = ctx.accounts.burner_coin.amount;

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
        spend,
        min_out,
        &[burner_seeds],
    )?;

    ctx.accounts.burner_zec.reload()?;
    ctx.accounts.burner_coin.reload()?;
    let (spent, burned) = finish(
        &mut ctx.accounts.ledger,
        spend,
        min_out,
        zec_before,
        coin_before,
        ctx.accounts.burner_zec.amount,
        ctx.accounts.burner_coin.amount,
    )?;
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
        a.burner_coin.amount,
        a.burner_zec.amount,
    )?;

    ctx.accounts.pool_base_token_account.reload()?;
    ctx.accounts.pool_quote_token_account.reload()?;
    let (after, _) = pool_reserves(
        &ctx.accounts.pool.to_account_info(),
        &mint_key,
        ctx.accounts.pool_base_token_account.amount,
        ctx.accounts.pool_quote_token_account.amount,
    )?;
    let band = ctx.accounts.coin.burn_price_band_bps;
    let b = &mut ctx.accounts.ledger.burn;
    b.reference = next_reference(&b.reference, after, band)?;
    b.last_time = now;
    emit!(Burned { mint: mint_key, spent, burned });
    Ok(())
}

/// The reference to record after a buy: the pool's price right after it,
/// with the reserve figure allowed to grow by at most one band over the last
/// one. Without that bound, liquidity added around one burn would raise the
/// limit of the next.
fn next_reference(last: &Reserves, after: Reserves, band_bps: u16) -> Result<Reserves> {
    let ceiling = last.raised_by(band_bps).ok_or(BurrError::Overflow)?.quote;
    after.held_to(ceiling).ok_or(BurrError::Overflow.into())
}

/// Moves exactly `spend` from the vault to the burner. Nothing else in the
/// vault is within the trade's reach.
fn fund_burner<'info>(
    token_program: &AccountInfo<'info>,
    vault: &AccountInfo<'info>,
    zec_mint: &AccountInfo<'info>,
    burner_zec: &AccountInfo<'info>,
    authority: &AccountInfo<'info>,
    coin: &Coin,
    spend: u64,
) -> Result<()> {
    ops::pay_from_vault(token_program, vault, zec_mint, burner_zec, authority, &coin.mint, coin.authority_bump, spend)
}

/// Checks the trade by balances and books it. Returns (ZEC spent, coins to burn).
fn finish(
    ledger: &mut Ledger,
    spend: u64,
    min_out: u64,
    zec_before: u64,
    coin_before: u64,
    zec_after: u64,
    coin_after: u64,
) -> Result<(u64, u64)> {
    let spent = zec_before.checked_sub(zec_after).ok_or(BurrError::BurnOverspent)?;
    require!(spent <= spend, BurrError::BurnOverspent);
    let bought = coin_after.checked_sub(coin_before).ok_or(BurrError::BurnBadFill)?;
    require!(bought >= min_out, BurrError::BurnBadFill);
    ops::debit(ledger, SINK_BURN, spent)?;
    ledger.burn.total_spent = ledger.burn.total_spent.checked_add(spent).ok_or(BurrError::Overflow)?;
    ledger.burn.total_burned = ledger.burn.total_burned.checked_add(coin_after).ok_or(BurrError::Overflow)?;
    Ok((spent, coin_after))
}

/// Burns every coin the burner holds and sends any ZEC it still holds back
/// to the vault.
#[allow(clippy::too_many_arguments)]
fn burn_and_return<'info>(
    token_2022_program: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    mint: &AccountInfo<'info>,
    zec_mint: &AccountInfo<'info>,
    burner: &AccountInfo<'info>,
    burner_coin: &AccountInfo<'info>,
    burner_zec: &AccountInfo<'info>,
    vault: &AccountInfo<'info>,
    burner_seeds: &[&[u8]],
    coins: u64,
    zec_left: u64,
) -> Result<()> {
    if coins > 0 {
        token_interface::burn(
            CpiContext::new_with_signer(
                *token_2022_program.key,
                Burn { mint: mint.clone(), from: burner_coin.clone(), authority: burner.clone() },
                &[burner_seeds],
            ),
            coins,
        )?;
    }
    if zec_left > 0 {
        token_interface::transfer_checked(
            CpiContext::new_with_signer(
                *token_program.key,
                TransferChecked {
                    from: burner_zec.clone(),
                    mint: zec_mint.clone(),
                    to: vault.clone(),
                    authority: burner.clone(),
                },
                &[burner_seeds],
            ),
            zec_left,
            ZEC_DECIMALS,
        )?;
    }
    Ok(())
}
