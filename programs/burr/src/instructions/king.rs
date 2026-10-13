//! King of the hill: in each round, the largest single buy made through this
//! program takes the round's pot.
//!
//! The program makes the buy itself and measures it from the buyer's balances
//! before and after. No number supplied by the caller decides anything.

use crate::constants::*;
use crate::error::BurrError;
use crate::events::{KingBuy, KingSettled};
use crate::math;
use crate::ops;
use crate::pump;
use crate::state::{Coin, Ledger};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::{get_associated_token_address_with_program_id, AssociatedToken};
use anchor_spl::token::Token;
use anchor_spl::token_2022::Token2022;
use anchor_spl::token_interface::{Mint, TokenAccount};

/// Before a buy: moves to the current round. A finished round that has a
/// leader must be settled first, so its pot cannot be put off or mixed.
fn enter_round(coin: &Coin, ledger: &mut Ledger, now: i64) -> Result<u64> {
    require!(coin.share_of(SINK_KING) > 0, BurrError::HookNotChosen);
    let round = math::king_round(now, coin.launched_at, coin.king_round_secs).ok_or(BurrError::Overflow)?;
    let k = &mut ledger.king;
    if round > k.round {
        require!(k.leader == Pubkey::default(), BurrError::KingSettleFirst);
        k.round = round;
        k.leader_amount = 0;
    }
    Ok(round)
}

/// After a buy: measures it and updates the lead. A tie does not take the lead.
fn record_buy(
    ledger: &mut Ledger,
    mint: Pubkey,
    buyer: Pubkey,
    round: u64,
    zec_before_after: (u64, u64),
    coin_before_after: (u64, u64),
) -> Result<()> {
    let spent = zec_before_after.0.checked_sub(zec_before_after.1).ok_or(BurrError::KingBadBuy)?;
    let received = coin_before_after.1.checked_sub(coin_before_after.0).ok_or(BurrError::KingBadBuy)?;
    require!(spent > 0 && received > 0, BurrError::KingBadBuy);
    let k = &mut ledger.king;
    let leads = spent > k.leader_amount;
    if leads {
        k.leader = buyer;
        k.leader_amount = spent;
    }
    emit!(KingBuy { mint, buyer, round, spent, received, leads });
    Ok(())
}

#[derive(Accounts)]
pub struct KingBuyCurve<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,
    #[account(mint::token_program = token_2022_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(seeds = [SEED_COIN, mint.key().as_ref()], bump = coin.bump)]
    pub coin: Box<Account<'info, Coin>>,
    #[account(mut, seeds = [SEED_LEDGER, mint.key().as_ref()], bump = ledger.bump)]
    pub ledger: Box<Account<'info, Ledger>>,
    #[account(address = ZEC_MINT)]
    pub zec_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        init_if_needed,
        payer = buyer,
        associated_token::mint = mint,
        associated_token::authority = buyer,
        associated_token::token_program = token_2022_program
    )]
    pub buyer_coin: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        associated_token::mint = zec_mint,
        associated_token::authority = buyer,
        associated_token::token_program = token_program
    )]
    pub buyer_zec: Box<InterfaceAccount<'info, TokenAccount>>,

    /// CHECK: derived address of pump.fun
    #[account(seeds = [pump::SEED_GLOBAL], bump, seeds::program = pump::PUMP_ID)]
    pub pump_global: UncheckedAccount<'info>,
    /// CHECK: derived address of pump.fun
    #[account(mut, seeds = [pump::SEED_BONDING_CURVE, mint.key().as_ref()], bump, seeds::program = pump::PUMP_ID)]
    pub bonding_curve: UncheckedAccount<'info>,
    /// CHECK: the curve's coin account
    #[account(mut, address = get_associated_token_address_with_program_id(&bonding_curve.key(), &mint.key(), &token_2022_program.key()))]
    pub associated_base_bonding_curve: UncheckedAccount<'info>,
    /// CHECK: the curve's ZEC account
    #[account(mut, address = get_associated_token_address_with_program_id(&bonding_curve.key(), &ZEC_MINT, &token_program.key()))]
    pub associated_quote_bonding_curve: UncheckedAccount<'info>,
    /// CHECK: pump.fun's volume record for the buyer
    #[account(mut, seeds = [pump::SEED_USER_VOLUME, buyer.key().as_ref()], bump, seeds::program = pump::PUMP_ID)]
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

/// Buys on the curve for exactly `quote_in` ZEC (fees included) and at least
/// `min_out` coins. Both limits are the buyer's own; neither decides the lead.
pub fn handle_king_buy_curve(ctx: Context<KingBuyCurve>, quote_in: u64, min_out: u64) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let round = enter_round(&ctx.accounts.coin, &mut ctx.accounts.ledger, now)?;
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
    let zec_before = a.buyer_zec.amount;
    let coin_before = a.buyer_coin.amount;
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
            user: &a.buyer.to_account_info(),
            user_base: &a.buyer_coin.to_account_info(),
            user_quote: &a.buyer_zec.to_account_info(),
            user_volume: &a.user_volume.to_account_info(),
            fee_config: &a.fee_config.to_account_info(),
            buyback_fee_recipient: &a.buyback_fee_recipient.to_account_info(),
            system_program: &a.system_program.to_account_info(),
            event_authority: &a.pump_event_authority.to_account_info(),
        },
        quote_in,
        min_out,
        &[],
    )?;
    ctx.accounts.buyer_zec.reload()?;
    ctx.accounts.buyer_coin.reload()?;
    record_buy(
        &mut ctx.accounts.ledger,
        ctx.accounts.mint.key(),
        ctx.accounts.buyer.key(),
        round,
        (zec_before, ctx.accounts.buyer_zec.amount),
        (coin_before, ctx.accounts.buyer_coin.amount),
    )
}

#[derive(Accounts)]
pub struct KingBuyPool<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,
    #[account(mint::token_program = token_2022_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(seeds = [SEED_COIN, mint.key().as_ref()], bump = coin.bump)]
    pub coin: Box<Account<'info, Coin>>,
    #[account(mut, seeds = [SEED_LEDGER, mint.key().as_ref()], bump = ledger.bump)]
    pub ledger: Box<Account<'info, Ledger>>,
    #[account(address = ZEC_MINT)]
    pub zec_mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(
        init_if_needed,
        payer = buyer,
        associated_token::mint = mint,
        associated_token::authority = buyer,
        associated_token::token_program = token_2022_program
    )]
    pub buyer_coin: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        associated_token::mint = zec_mint,
        associated_token::authority = buyer,
        associated_token::token_program = token_program
    )]
    pub buyer_zec: Box<InterfaceAccount<'info, TokenAccount>>,

    /// CHECK: derived address of PumpSwap
    #[account(seeds = [pump::SEED_AMM_GLOBAL_CONFIG], bump, seeds::program = pump::PUMP_AMM_ID)]
    pub amm_global_config: UncheckedAccount<'info>,
    /// CHECK: pump.fun's derived address that creates a coin's pool at graduation
    #[account(seeds = [pump::SEED_POOL_AUTHORITY, mint.key().as_ref()], bump, seeds::program = pump::PUMP_ID)]
    pub pool_authority: UncheckedAccount<'info>,
    /// CHECK: the coin's one pool made by graduation, by derived address
    #[account(
        mut,
        seeds = [pump::SEED_AMM_POOL, &pump::CANONICAL_POOL_INDEX, pool_authority.key().as_ref(), mint.key().as_ref(), ZEC_MINT.as_ref()],
        bump,
        seeds::program = pump::PUMP_AMM_ID
    )]
    pub pool: UncheckedAccount<'info>,
    /// CHECK: the pool's coin account
    #[account(mut, address = get_associated_token_address_with_program_id(&pool.key(), &mint.key(), &token_2022_program.key()))]
    pub pool_base_token_account: UncheckedAccount<'info>,
    /// CHECK: the pool's ZEC account
    #[account(mut, address = get_associated_token_address_with_program_id(&pool.key(), &ZEC_MINT, &token_program.key()))]
    pub pool_quote_token_account: UncheckedAccount<'info>,
    /// CHECK: PumpSwap's volume record for the buyer
    #[account(mut, seeds = [pump::SEED_USER_VOLUME, buyer.key().as_ref()], bump, seeds::program = pump::PUMP_AMM_ID)]
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

/// The same buy, on the coin's pool after graduation.
pub fn handle_king_buy_pool(ctx: Context<KingBuyPool>, quote_in: u64, min_out: u64) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(!ctx.accounts.pool.data_is_empty(), BurrError::WrongVenue);
    let round = enter_round(&ctx.accounts.coin, &mut ctx.accounts.ledger, now)?;
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
    let zec_before = a.buyer_zec.amount;
    let coin_before = a.buyer_coin.amount;
    pump::buy_on_pool(
        pump::PoolBuy {
            amm_program: &a.amm_program.to_account_info(),
            pool: &a.pool.to_account_info(),
            user: &a.buyer.to_account_info(),
            global_config: &a.amm_global_config.to_account_info(),
            base_mint: &a.mint.to_account_info(),
            quote_mint: &a.zec_mint.to_account_info(),
            user_base: &a.buyer_coin.to_account_info(),
            user_quote: &a.buyer_zec.to_account_info(),
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
        quote_in,
        min_out,
        &[],
    )?;
    ctx.accounts.buyer_zec.reload()?;
    ctx.accounts.buyer_coin.reload()?;
    record_buy(
        &mut ctx.accounts.ledger,
        ctx.accounts.mint.key(),
        ctx.accounts.buyer.key(),
        round,
        (zec_before, ctx.accounts.buyer_zec.amount),
        (coin_before, ctx.accounts.buyer_coin.amount),
    )
}

#[derive(Accounts)]
pub struct KingSettle<'info> {
    /// Pays the small rent if the winner has no ZEC account yet.
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
    /// CHECK: must be the leader recorded in the ledger
    #[account(address = ledger.king.leader @ BurrError::KingWrongWinner)]
    pub winner: UncheckedAccount<'info>,
    /// The winner's standard ZEC account. Nobody chooses another.
    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = zec_mint,
        associated_token::authority = winner,
        associated_token::token_program = token_program
    )]
    pub winner_zec: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// Pays a finished round's pot to its leader. Open to anyone.
pub fn handle_king_settle(ctx: Context<KingSettle>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let coin = &ctx.accounts.coin;
    let round = math::king_round(now, coin.launched_at, coin.king_round_secs).ok_or(BurrError::Overflow)?;
    let ledger = &mut ctx.accounts.ledger;
    require!(round > ledger.king.round && ledger.king.leader != Pubkey::default(), BurrError::KingNothingToSettle);
    let settled_round = ledger.king.round;
    let winner = ledger.king.leader;
    let amount = ledger.pots[SINK_KING];
    // The pot and the lead are cleared before the ZEC moves.
    ops::debit(ledger, SINK_KING, amount)?;
    ledger.king.leader = Pubkey::default();
    ledger.king.leader_amount = 0;
    ledger.king.round = round;

    let a = &ctx.accounts;
    ops::pay_from_vault(
        &a.token_program.to_account_info(),
        &a.vault.to_account_info(),
        &a.zec_mint.to_account_info(),
        &a.winner_zec.to_account_info(),
        &a.authority.to_account_info(),
        &a.coin.mint,
        a.coin.authority_bump,
        amount,
    )?;
    emit!(KingSettled { mint: a.coin.mint, round: settled_round, winner, amount });
    Ok(())
}
