//! Vault share (the brief's "holders' share"): people who place their coins
//! in the coin's vault share this pot, by amount and by time.
//!
//! No lock and no fee. Placing and taking out is open to anyone at any time.
//! Everything is worked out on-chain; nobody tells the program who is owed.

use crate::constants::*;
use crate::error::BurrError;
use crate::events::StakeChanged;
use crate::ops;
use crate::state::{Coin, Ledger, Stake};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::Token;
use anchor_spl::token_2022::Token2022;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TransferChecked};

/// Brings the pot's running total and one person's stake up to date.
fn settle(coin: &Coin, ledger: &mut Ledger, stake: &mut Stake, now: i64) -> Result<()> {
    let vs = &mut ledger.vault_share;
    vs.update(now, coin.vault_release_secs).ok_or(BurrError::Overflow)?;
    let earned = vs.earned(stake.amount, stake.acc_snapshot).ok_or(BurrError::Overflow)?;
    stake.owed = stake.owed.checked_add(earned).ok_or(BurrError::Overflow)?;
    stake.acc_snapshot = vs.acc;
    Ok(())
}

#[derive(Accounts)]
pub struct StakeCoins<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    #[account(mint::token_program = token_2022_program)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    #[account(seeds = [SEED_COIN, mint.key().as_ref()], bump = coin.bump)]
    pub coin: Box<Account<'info, Coin>>,
    #[account(mut, seeds = [SEED_LEDGER, mint.key().as_ref()], bump = ledger.bump)]
    pub ledger: Box<Account<'info, Ledger>>,
    /// CHECK: the coin's authority, a derived address
    #[account(seeds = [SEED_AUTHORITY, mint.key().as_ref()], bump = coin.authority_bump)]
    pub authority: UncheckedAccount<'info>,
    #[account(
        init_if_needed,
        payer = owner,
        space = 8 + Stake::INIT_SPACE,
        seeds = [SEED_STAKE, mint.key().as_ref(), owner.key().as_ref()],
        bump
    )]
    pub stake: Box<Account<'info, Stake>>,
    /// Where placed coins are kept: the authority's own account for this coin.
    #[account(
        init_if_needed,
        payer = owner,
        associated_token::mint = mint,
        associated_token::authority = authority,
        associated_token::token_program = token_2022_program
    )]
    pub stake_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, token::mint = mint, token::authority = owner, token::token_program = token_2022_program)]
    pub owner_coin: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_2022_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handle_stake(ctx: Context<StakeCoins>, amount: u64) -> Result<()> {
    require!(amount > 0, BurrError::ZeroAmount);
    require!(ctx.accounts.coin.share_of(SINK_VAULT_SHARE) > 0, BurrError::HookNotChosen);
    let now = Clock::get()?.unix_timestamp;
    let mint_key = ctx.accounts.mint.key();

    let stake = &mut ctx.accounts.stake;
    if stake.owner == Pubkey::default() {
        stake.mint = mint_key;
        stake.owner = ctx.accounts.owner.key();
        stake.bump = ctx.bumps.stake;
    }
    settle(&ctx.accounts.coin, &mut ctx.accounts.ledger, stake, now)?;

    let before = ctx.accounts.stake_vault.amount;
    token_interface::transfer_checked(
        CpiContext::new(
            ctx.accounts.token_2022_program.key(),
            TransferChecked {
                from: ctx.accounts.owner_coin.to_account_info(),
                mint: ctx.accounts.mint.to_account_info(),
                to: ctx.accounts.stake_vault.to_account_info(),
                authority: ctx.accounts.owner.to_account_info(),
            },
        ),
        amount,
        ctx.accounts.mint.decimals,
    )?;
    ctx.accounts.stake_vault.reload()?;
    // What arrived must be exactly what was sent.
    require!(ctx.accounts.stake_vault.amount.checked_sub(before) == Some(amount), BurrError::TransferMismatch);

    let stake = &mut ctx.accounts.stake;
    stake.amount = stake.amount.checked_add(amount).ok_or(BurrError::Overflow)?;
    let vs = &mut ctx.accounts.ledger.vault_share;
    vs.total_staked = vs.total_staked.checked_add(amount).ok_or(BurrError::Overflow)?;
    emit!(StakeChanged {
        mint: mint_key,
        owner: stake.owner,
        staked: stake.amount,
        total_staked: vs.total_staked,
        paid: 0,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct UnstakeCoins<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    #[account(mint::token_program = token_2022_program)]
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
        seeds = [SEED_STAKE, mint.key().as_ref(), owner.key().as_ref()],
        bump = stake.bump,
        has_one = owner,
        has_one = mint
    )]
    pub stake: Box<Account<'info, Stake>>,
    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = authority,
        associated_token::token_program = token_2022_program
    )]
    pub stake_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(mut, token::mint = mint, token::authority = owner, token::token_program = token_2022_program)]
    pub owner_coin: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        associated_token::mint = zec_mint,
        associated_token::authority = authority,
        associated_token::token_program = token_program
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(address = ZEC_MINT)]
    pub zec_mint: Box<InterfaceAccount<'info, Mint>>,
    /// CHECK: the staker's standard ZEC account, checked by address in the handler and made if missing
    #[account(mut)]
    pub owner_zec: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    pub token_2022_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// Takes `amount` coins out (zero is allowed: that is a plain claim) and pays
/// everything the person has earned.
pub fn handle_unstake(ctx: Context<UnstakeCoins>, amount: u64) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let mint_key = ctx.accounts.mint.key();
    settle(&ctx.accounts.coin, &mut ctx.accounts.ledger, &mut ctx.accounts.stake, now)?;

    let stake = &mut ctx.accounts.stake;
    require!(stake.amount >= amount, BurrError::NotEnoughStaked);
    stake.amount -= amount;
    let pay = stake.owed;
    stake.owed = 0;
    let staked_now = stake.amount;

    let ledger = &mut ctx.accounts.ledger;
    ledger.vault_share.total_staked = ledger.vault_share.total_staked.checked_sub(amount).ok_or(BurrError::Overflow)?;
    ops::debit(ledger, SINK_VAULT_SHARE, pay)?;
    if ledger.vault_share.total_staked == 0 {
        // Nobody is left, so nobody is owed anything: all that remains in the
        // pot (the unreleased queue and rounding dust) waits for the next staker.
        ledger.vault_share.queued = ledger.pots[SINK_VAULT_SHARE];
        // Its release starts afresh when the next staker arrives, even if
        // that is in this same second.
        ledger.vault_share.release_end =
            now.checked_add(ctx.accounts.coin.vault_release_secs as i64).ok_or(BurrError::Overflow)?;
    }
    let total_staked = ledger.vault_share.total_staked;

    let a = &ctx.accounts;
    let seeds: &[&[u8]] = &[SEED_AUTHORITY, mint_key.as_ref(), &[a.coin.authority_bump]];
    if amount > 0 {
        token_interface::transfer_checked(
            CpiContext::new_with_signer(
                a.token_2022_program.key(),
                TransferChecked {
                    from: a.stake_vault.to_account_info(),
                    mint: a.mint.to_account_info(),
                    to: a.owner_coin.to_account_info(),
                    authority: a.authority.to_account_info(),
                },
                &[seeds],
            ),
            amount,
            a.mint.decimals,
        )?;
    }
    ops::pay_to_standard_account(
        &ops::PayTo {
            payer: &a.owner.to_account_info(),
            owner: &a.owner.to_account_info(),
            destination: &a.owner_zec.to_account_info(),
            vault: &a.vault.to_account_info(),
            zec_mint: &a.zec_mint.to_account_info(),
            authority: &a.authority.to_account_info(),
            token_program: &a.token_program.to_account_info(),
            associated_token_program: &a.associated_token_program.to_account_info(),
            system_program: &a.system_program.to_account_info(),
        },
        &mint_key,
        a.coin.authority_bump,
        pay,
    )?;
    emit!(StakeChanged { mint: mint_key, owner: a.owner.key(), staked: staked_now, total_staked, paid: pay });
    Ok(())
}

#[derive(Accounts)]
pub struct CloseStake<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    /// CHECK: only its address is used, to derive the stake's address
    pub mint: UncheckedAccount<'info>,
    #[account(
        mut,
        close = owner,
        seeds = [SEED_STAKE, mint.key().as_ref(), owner.key().as_ref()],
        bump = stake.bump,
        has_one = owner,
        has_one = mint
    )]
    pub stake: Box<Account<'info, Stake>>,
}

/// Closes an empty stake record and returns its rent to its owner.
pub fn handle_close_stake(ctx: Context<CloseStake>) -> Result<()> {
    let s = &ctx.accounts.stake;
    require!(s.amount == 0 && s.owed == 0, BurrError::StakeNotEmpty);
    Ok(())
}
