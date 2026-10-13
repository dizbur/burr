//! Holders: a share of the fee set aside for the coin's holders.
//!
//! **This hook is different from the others, by the owner's decision.** A
//! program cannot see who held a coin over time. So this pot is handed whole
//! to a rewards wallet, and a server working with that wallet takes a
//! snapshot of holders and pays them. Who gets how much is decided off the
//! chain.
//!
//! What the program enforces:
//! - the pot can go only to the rewards wallet named in the settings, to its
//!   standard ZEC account, and that wallet must sign;
//! - at most one release per coin per interval (30 minutes);
//! - none at all while the administrator has releases switched off;
//! - each release records the slot and a 32-byte fingerprint of the snapshot
//!   it is for, so the published snapshot can be checked against the chain.
//!
//! What it does not enforce: that holders are then paid, or paid rightly.
//! Whoever controls the rewards wallet controls every coin's holder pot, and
//! the administrator chooses that wallet. No other pot is within its reach.

use crate::constants::*;
use crate::error::BurrpadError;
use crate::events::HolderRewardsReleased;
use crate::ops;
use crate::state::{Coin, Config, Ledger};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::Token;
use anchor_spl::token_interface::{Mint, TokenAccount};

#[derive(Accounts)]
pub struct ReleaseHolderRewards<'info> {
    /// The rewards wallet. It signs, and pays the small rent if its ZEC
    /// account does not exist yet.
    #[account(mut, address = config.rewards_wallet @ BurrpadError::NotRewardsWallet)]
    pub rewards_wallet: Signer<'info>,
    #[account(seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
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
    /// CHECK: the rewards wallet's standard ZEC account, checked by address in the handler and made if missing
    #[account(mut)]
    pub rewards_zec: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// Hands the coin's whole holder pot to the rewards wallet for one payout run.
pub fn handle_release_holder_rewards(
    ctx: Context<ReleaseHolderRewards>,
    snapshot_slot: u64,
    snapshot_hash: [u8; 32],
) -> Result<()> {
    let clock = Clock::get()?;
    require!(!ctx.accounts.config.holder_releases_paused, BurrpadError::HolderReleasesPaused);
    let coin = &ctx.accounts.coin;
    require!(coin.share_of(SINK_HOLDERS) > 0, BurrpadError::HookNotChosen);

    let ledger = &mut ctx.accounts.ledger;
    let ready = ledger
        .holders
        .last_release
        .checked_add(coin.holders_release_interval_secs as i64)
        .ok_or(BurrpadError::Overflow)?;
    require!(clock.unix_timestamp >= ready, BurrpadError::HolderReleaseTooSoon);

    let amount = ledger.pots[SINK_HOLDERS];
    require!(amount > 0, BurrpadError::NothingToRelease);

    // The snapshot must be named: a real fingerprint, of a slot that has
    // happened and is later than the last run's.
    require!(
        snapshot_hash != [0u8; 32] && snapshot_slot <= clock.slot && snapshot_slot > ledger.holders.last_snapshot_slot,
        BurrpadError::BadSnapshot
    );

    // The pot is emptied in the ledger before the ZEC moves.
    ops::debit(ledger, SINK_HOLDERS, amount)?;
    let h = &mut ledger.holders;
    h.last_release = clock.unix_timestamp;
    h.last_snapshot_slot = snapshot_slot;
    h.runs = h.runs.checked_add(1).ok_or(BurrpadError::Overflow)?;
    h.total_released = h.total_released.checked_add(amount).ok_or(BurrpadError::Overflow)?;
    let run = h.runs;

    let a = &ctx.accounts;
    ops::pay_to_standard_account(
        &ops::PayTo {
            payer: &a.rewards_wallet.to_account_info(),
            owner: &a.rewards_wallet.to_account_info(),
            destination: &a.rewards_zec.to_account_info(),
            vault: &a.vault.to_account_info(),
            zec_mint: &a.zec_mint.to_account_info(),
            authority: &a.authority.to_account_info(),
            token_program: &a.token_program.to_account_info(),
            associated_token_program: &a.associated_token_program.to_account_info(),
            system_program: &a.system_program.to_account_info(),
        },
        &a.coin.mint,
        a.coin.authority_bump,
        amount,
    )?;
    emit!(HolderRewardsReleased {
        mint: a.coin.mint,
        rewards_wallet: a.rewards_wallet.key(),
        amount,
        run,
        snapshot_slot,
        snapshot_hash,
    });
    Ok(())
}
