//! Collect and split. Open to anyone.
//!
//! pump.fun's own collect instructions are open to anyone too, and always pay
//! into the coin's vault. So these instructions do not rely on being the ones
//! that made the call: they read the vault and split what is new.

use crate::constants::*;
use crate::error::BurrpadError;
use crate::ops;
use crate::pump;
use crate::state::{Coin, Ledger};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::{get_associated_token_address_with_program_id, AssociatedToken};
use anchor_spl::token::Token;
use anchor_spl::token_interface::{Mint, TokenAccount};

/// The accounts every per-coin instruction starts from. All are derived from
/// the one mint, so accounts of two coins cannot be mixed.
#[derive(Accounts)]
pub struct SplitOnly<'info> {
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
        associated_token::mint = zec_mint,
        associated_token::authority = authority,
        associated_token::token_program = token_program
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(address = ZEC_MINT)]
    pub zec_mint: Box<InterfaceAccount<'info, Mint>>,
    pub token_program: Program<'info, Token>,
}

pub fn handle_split(ctx: Context<SplitOnly>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    ops::split_new(&ctx.accounts.coin, &mut ctx.accounts.ledger, ctx.accounts.vault.amount, now)?;
    Ok(())
}

#[derive(Accounts)]
pub struct CollectCurve<'info> {
    /// Pays the small rent if pump.fun's fee vault account does not exist yet.
    #[account(mut)]
    pub payer: Signer<'info>,
    /// CHECK: only its address is used, to derive the coin's accounts
    pub mint: UncheckedAccount<'info>,
    #[account(seeds = [SEED_COIN, mint.key().as_ref()], bump = coin.bump)]
    pub coin: Box<Account<'info, Coin>>,
    #[account(mut, seeds = [SEED_LEDGER, mint.key().as_ref()], bump = ledger.bump)]
    pub ledger: Box<Account<'info, Ledger>>,
    /// CHECK: the coin's authority, a derived address
    #[account(mut, seeds = [SEED_AUTHORITY, mint.key().as_ref()], bump = coin.authority_bump)]
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

    /// CHECK: derived address of pump.fun
    #[account(seeds = [pump::SEED_GLOBAL], bump, seeds::program = pump::PUMP_ID)]
    pub pump_global: UncheckedAccount<'info>,
    /// CHECK: derived address of pump.fun
    #[account(mut, seeds = [pump::SEED_BONDING_CURVE, mint.key().as_ref()], bump, seeds::program = pump::PUMP_ID)]
    pub bonding_curve: UncheckedAccount<'info>,
    /// CHECK: the curve's ZEC account
    #[account(mut, address = get_associated_token_address_with_program_id(&bonding_curve.key(), &ZEC_MINT, &token_program.key()))]
    pub associated_quote_bonding_curve: UncheckedAccount<'info>,
    /// CHECK: pump.fun's fee vault for this coin's creator, which is our authority
    #[account(mut, seeds = [pump::SEED_CREATOR_VAULT, authority.key().as_ref()], bump, seeds::program = pump::PUMP_ID)]
    pub creator_vault: UncheckedAccount<'info>,
    /// CHECK: that vault's ZEC account
    #[account(mut, address = get_associated_token_address_with_program_id(&creator_vault.key(), &ZEC_MINT, &token_program.key()))]
    pub creator_vault_ata: UncheckedAccount<'info>,
    /// CHECK: derived address of pump.fun
    #[account(seeds = [pump::SEED_EVENT_AUTHORITY], bump, seeds::program = pump::PUMP_ID)]
    pub pump_event_authority: UncheckedAccount<'info>,
    /// CHECK: pinned address
    #[account(address = pump::PUMP_ID)]
    pub pump_program: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handle_collect_curve(ctx: Context<CollectCurve>) -> Result<()> {
    let a = &ctx.accounts;
    pump::sweep_creator_fee_curve(
        &a.pump_program.to_account_info(),
        &a.payer.to_account_info(),
        &a.pump_global.to_account_info(),
        &a.mint.to_account_info(),
        &a.zec_mint.to_account_info(),
        &a.token_program.to_account_info(),
        &a.associated_token_program.to_account_info(),
        &a.system_program.to_account_info(),
        &a.bonding_curve.to_account_info(),
        &a.associated_quote_bonding_curve.to_account_info(),
        &a.creator_vault.to_account_info(),
        &a.creator_vault_ata.to_account_info(),
        &a.pump_event_authority.to_account_info(),
    )?;
    // pump.fun's fee vault account exists only once a fee has reached it.
    if !a.creator_vault_ata.data_is_empty() {
        pump::collect_creator_fee_curve(
            &a.pump_program.to_account_info(),
            &a.authority.to_account_info(),
            &a.vault.to_account_info(),
            &a.creator_vault.to_account_info(),
            &a.creator_vault_ata.to_account_info(),
            &a.zec_mint.to_account_info(),
            &a.token_program.to_account_info(),
            &a.associated_token_program.to_account_info(),
            &a.system_program.to_account_info(),
            &a.pump_event_authority.to_account_info(),
        )?;
    }
    ctx.accounts.vault.reload()?;
    let now = Clock::get()?.unix_timestamp;
    ops::split_new(&ctx.accounts.coin, &mut ctx.accounts.ledger, ctx.accounts.vault.amount, now)?;
    Ok(())
}

#[derive(Accounts)]
pub struct CollectPool<'info> {
    /// Pays the small rent if PumpSwap's fee vault account does not exist yet.
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
    /// CHECK: the pool's ZEC account
    #[account(mut, address = get_associated_token_address_with_program_id(&pool.key(), &ZEC_MINT, &token_program.key()))]
    pub pool_quote_token_account: UncheckedAccount<'info>,
    /// CHECK: PumpSwap's fee vault owner for this coin's creator, which is our authority
    #[account(seeds = [pump::SEED_AMM_CREATOR_VAULT, authority.key().as_ref()], bump, seeds::program = pump::PUMP_AMM_ID)]
    pub amm_creator_vault_authority: UncheckedAccount<'info>,
    /// CHECK: that vault's ZEC account
    #[account(mut, address = get_associated_token_address_with_program_id(&amm_creator_vault_authority.key(), &ZEC_MINT, &token_program.key()))]
    pub amm_creator_vault_ata: UncheckedAccount<'info>,
    /// CHECK: derived address of PumpSwap
    #[account(seeds = [pump::SEED_EVENT_AUTHORITY], bump, seeds::program = pump::PUMP_AMM_ID)]
    pub amm_event_authority: UncheckedAccount<'info>,
    /// CHECK: pinned address
    #[account(address = pump::PUMP_AMM_ID)]
    pub amm_program: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handle_collect_pool(ctx: Context<CollectPool>) -> Result<()> {
    let a = &ctx.accounts;
    // The pool exists only after graduation.
    require!(!a.pool.data_is_empty(), BurrpadError::WrongVenue);
    pump::sweep_creator_fee_pool(
        &a.amm_program.to_account_info(),
        &a.payer.to_account_info(),
        &a.amm_global_config.to_account_info(),
        &a.pool.to_account_info(),
        &a.zec_mint.to_account_info(),
        &a.token_program.to_account_info(),
        &a.pool_quote_token_account.to_account_info(),
        &a.amm_creator_vault_authority.to_account_info(),
        &a.amm_creator_vault_ata.to_account_info(),
        &a.system_program.to_account_info(),
        &a.associated_token_program.to_account_info(),
        &a.amm_event_authority.to_account_info(),
    )?;
    if !a.amm_creator_vault_ata.data_is_empty() {
        pump::collect_creator_fee_pool(
            &a.amm_program.to_account_info(),
            &a.zec_mint.to_account_info(),
            &a.token_program.to_account_info(),
            &a.authority.to_account_info(),
            &a.amm_creator_vault_authority.to_account_info(),
            &a.amm_creator_vault_ata.to_account_info(),
            &a.vault.to_account_info(),
            &a.amm_event_authority.to_account_info(),
        )?;
    }
    ctx.accounts.vault.reload()?;
    let now = Clock::get()?.unix_timestamp;
    ops::split_new(&ctx.accounts.coin, &mut ctx.accounts.ledger, ctx.accounts.vault.amount, now)?;
    Ok(())
}
