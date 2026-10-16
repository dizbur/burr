//! The eighth hook, the shadow burn: part of a coin's fee buys back and burns
//! $BURR, up to the $BURR that the wallets it watches have let go of (sold, or
//! sent away). The program records the watched wallets' holding and allows
//! burning only up to the fall since the record began; the buy keeps every
//! rule of `burn_burr`, and the same one wait for every $BURR buy.
//!
//! The coin's launcher sets and changes the watched wallets with one
//! signature. If the launcher allows it, a follower (Burrpad's keeper) may add
//! a wallet, at most once an hour, and never remove or replace one.
//!
//! The watched wallets' accounts for the target coin are passed after the
//! listed accounts, in the order of the list; each is checked to be that
//! wallet's own associated account. One that does not exist counts as 0.

use crate::constants::*;
use crate::error::BurrpadError;
use crate::events::{ShadowBurned, ShadowRecorded, ShadowWatchChanged};
use crate::instructions::burr::*;
use crate::state::{Coin, ShadowWatch};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::get_associated_token_address_with_program_id;

/// A watched wallet's holding of the target coin, from its associated account
/// (checked to be exactly that account); 0 if the account does not exist.
fn holding(wallet: &Pubkey, target: &Pubkey, account: &AccountInfo) -> Result<u64> {
    let own = get_associated_token_address_with_program_id(wallet, target, &anchor_spl::token_2022::ID);
    require_keys_eq!(account.key(), own, BurrpadError::WrongWatchedAccount);
    if account.data_is_empty() {
        return Ok(0);
    }
    require_keys_eq!(*account.owner, anchor_spl::token_2022::ID, BurrpadError::WrongWatchedAccount);
    let d = account.try_borrow_data()?;
    require!(d.len() >= 72, BurrpadError::WrongWatchedAccount);
    require!(d[0..32] == target.to_bytes() && d[32..64] == wallet.to_bytes(), BurrpadError::WrongWatchedAccount);
    Ok(u64::from_le_bytes(d[64..72].try_into().unwrap()))
}

/// The combined holding of `wallets`, from their accounts in the same order.
fn held_by(wallets: &[Pubkey], target: &Pubkey, accounts: &[AccountInfo]) -> Result<u64> {
    require!(accounts.len() >= wallets.len(), BurrpadError::WrongWatchedAccount);
    let mut total = 0u64;
    for (w, a) in wallets.iter().zip(accounts) {
        total = total.checked_add(holding(w, target, a)?).ok_or(BurrpadError::Overflow)?;
    }
    Ok(total)
}

/// Records the watched wallets' holding now: a fall since the last record is
/// added to what is owed; a rise only moves the record. The fall is measured
/// on the wallets' total, so coins moved between watched wallets count for nothing.
fn record(w: &mut ShadowWatch, held: u64) -> Result<u64> {
    let fell = w.recorded.saturating_sub(held);
    w.owed = w.owed.checked_add(fell).ok_or(BurrpadError::Overflow)?;
    w.recorded = held;
    emit!(ShadowRecorded { mint: w.mint, held, fell, owed: w.owed });
    Ok(fell)
}

fn watched(w: &ShadowWatch) -> Vec<Pubkey> {
    w.wallets[..w.count as usize].to_vec()
}

#[derive(Accounts)]
pub struct SetShadowWatch<'info> {
    /// The coin's launcher: only it sets or changes the watch, and it pays for the account the first time.
    #[account(mut)]
    pub launcher: Signer<'info>,
    /// CHECK: the coin's mint; the record below is found from it
    pub mint: UncheckedAccount<'info>,
    #[account(
        seeds = [SEED_COIN, mint.key().as_ref()],
        bump = coin.bump,
        constraint = coin.launcher == launcher.key() @ BurrpadError::NotTheLauncher
    )]
    pub coin: Box<Account<'info, Coin>>,
    #[account(
        init_if_needed,
        payer = launcher,
        space = 8 + ShadowWatch::INIT_SPACE,
        seeds = [SEED_SHADOW, mint.key().as_ref()],
        bump
    )]
    pub watch: Box<Account<'info, ShadowWatch>>,
    pub system_program: Program<'info, System>,
}

/// Sets the watched wallets (one to four), and whether a follower may add more.
/// The old wallets' fall is recorded first; the new wallets start from their
/// holding now, so a change never creates a fall by itself. What is owed stays owed.
/// After the listed accounts: the old wallets' target accounts, then the new ones'.
pub fn handle_set_shadow_watch<'info>(
    ctx: Context<'info, SetShadowWatch<'info>>,
    wallets: Vec<Pubkey>,
    follower: Pubkey,
    follow: bool,
) -> Result<()> {
    require!(ctx.accounts.coin.shadow_share > 0, BurrpadError::HookNotChosen);
    require!(!wallets.is_empty() && wallets.len() <= SHADOW_MAX_WALLETS, BurrpadError::BadWatchList);
    for (i, a) in wallets.iter().enumerate() {
        require!(*a != Pubkey::default() && !wallets[..i].contains(a), BurrpadError::BadWatchList);
    }
    let mint = ctx.accounts.mint.key();
    let bump = ctx.bumps.watch;
    let rest = ctx.remaining_accounts;
    let w = &mut ctx.accounts.watch;
    let old = if w.version == 0 {
        w.version = LAYOUT_VERSION;
        w.bump = bump;
        w.mint = mint;
        w.target = BURR_MINT;
        vec![]
    } else {
        watched(w)
    };
    require!(rest.len() == old.len() + wallets.len(), BurrpadError::WrongWatchedAccount);
    if !old.is_empty() {
        let held = held_by(&old, &w.target, &rest[..old.len()])?;
        record(w, held)?;
    }
    let target = w.target;
    w.recorded = held_by(&wallets, &target, &rest[old.len()..])?;
    w.wallets = [Pubkey::default(); SHADOW_MAX_WALLETS];
    w.wallets[..wallets.len()].copy_from_slice(&wallets);
    w.count = wallets.len() as u8;
    w.follower = follower;
    w.follow = follow && follower != Pubkey::default();
    emit!(ShadowWatchChanged { mint, wallets, follower, follow: w.follow, by_follower: false });
    Ok(())
}

#[derive(Accounts)]
pub struct FollowWallet<'info> {
    /// The follower the launcher allowed.
    pub follower: Signer<'info>,
    /// CHECK: the coin's mint; the watch below is found from it
    pub mint: UncheckedAccount<'info>,
    #[account(mut, seeds = [SEED_SHADOW, mint.key().as_ref()], bump = watch.bump)]
    pub watch: Box<Account<'info, ShadowWatch>>,
}

/// The follower adds one watched wallet: only while the launcher allows it,
/// at most once an hour, never past four, never removing or replacing one.
/// After the listed accounts: the watched wallets' target accounts, then the new wallet's.
pub fn handle_follow_wallet<'info>(ctx: Context<'info, FollowWallet<'info>>, wallet: Pubkey) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let rest = ctx.remaining_accounts;
    let w = &mut ctx.accounts.watch;
    require!(
        w.follow
            && w.follower == ctx.accounts.follower.key()
            && now >= w.follower_last.saturating_add(SHADOW_FOLLOW_WAIT_SECS),
        BurrpadError::FollowerNotAllowed
    );
    let old = watched(w);
    require!(
        old.len() < SHADOW_MAX_WALLETS && wallet != Pubkey::default() && !old.contains(&wallet),
        BurrpadError::BadWatchList
    );
    require!(rest.len() == old.len() + 1, BurrpadError::WrongWatchedAccount);
    let target = w.target;
    let held = held_by(&old, &target, &rest[..old.len()])?;
    record(w, held)?;
    let added = holding(&wallet, &target, &rest[old.len()])?;
    w.recorded = w.recorded.checked_add(added).ok_or(BurrpadError::Overflow)?;
    w.wallets[old.len()] = wallet;
    w.count += 1;
    w.follower_last = now;
    emit!(ShadowWatchChanged {
        mint: w.mint,
        wallets: watched(w),
        follower: w.follower,
        follow: true,
        by_follower: true
    });
    Ok(())
}

#[derive(Accounts)]
pub struct ShadowBurn<'info> {
    /// Every account of a $BURR buy, exactly as `burn_burr` takes them.
    pub burr: BurnBurr<'info>,
    #[account(mut, seeds = [SEED_SHADOW, burr.mint.key().as_ref()], bump = watch.bump)]
    pub watch: Box<Account<'info, ShadowWatch>>,
}

/// Records the watched wallets' holding; then, if the pot holds ZEC, something
/// is owed and the shared wait is over, buys back and burns $BURR: no more than
/// is owed, within every limit of a $BURR buy. Otherwise it only records.
/// After the listed accounts: the watched wallets' target accounts, in order.
pub fn handle_shadow_burn<'info>(ctx: Context<'info, ShadowBurn<'info>>) -> Result<()> {
    require!(ctx.accounts.burr.coin.shadow_share > 0, BurrpadError::HookNotChosen);
    let w = &mut ctx.accounts.watch;
    let wallets = watched(w);
    require!(ctx.remaining_accounts.len() == wallets.len(), BurrpadError::WrongWatchedAccount);
    let held = held_by(&wallets, &w.target, ctx.remaining_accounts)?;
    record(w, held)?;
    let (pot, owed) = (ctx.accounts.burr.ledger.shadow.pot, ctx.accounts.watch.owed);
    if pot == 0 || owed == 0 || !burr_ready(&ctx.accounts.burr)? {
        return Ok(());
    }
    let Some(done) = buy_and_burn_burr(&mut ctx.accounts.burr, pot, owed)? else {
        return Ok(());
    };
    // Booked against the eighth pot only; what was bought comes off what is owed.
    let ledger = &mut ctx.accounts.burr.ledger;
    ledger.shadow.pot = ledger.shadow.pot.checked_sub(done.spent).ok_or(BurrpadError::Overflow)?;
    ledger.accounted = ledger.accounted.checked_sub(done.spent).ok_or(BurrpadError::Overflow)?;
    let w = &mut ctx.accounts.watch;
    w.owed = w.owed.saturating_sub(done.bought);
    w.total_spent = w.total_spent.checked_add(done.spent).ok_or(BurrpadError::Overflow)?;
    w.total_burned = w.total_burned.checked_add(done.burned).ok_or(BurrpadError::Overflow)?;
    emit!(ShadowBurned { mint: w.mint, spent: done.spent, burned: done.burned, owed: w.owed });
    Ok(())
}
