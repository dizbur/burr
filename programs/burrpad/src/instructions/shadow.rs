//! The eighth hook, the shadow burn: part of a coin's fee buys back and burns
//! $BURR when the wallets it watches let go of $BURR (sold, or sent away). The
//! program records each watched wallet's holding, and never buys back more
//! coins than the watched wallets have let go of since the watch began.
//!
//! The buy is made at once, in one buy, by the watch's keeper (Burrpad's keeper,
//! named by the launcher), with the ZEC the sale received, or the whole pot if
//! smaller. None of `burn_burr`'s limit, reserve step or shared wait applies;
//! only a least-out: the buy must return the coins let go of, at the sale's own
//! price, less 5%. The program cannot read past transactions, so the keeper
//! reports the sale (`sold`, `received`); the program holds `sold` to what was
//! recorded as let go of, and `received` to what buys those coins back at the
//! pool's price now.
//!
//! The coin's launcher sets and changes the watched wallets with one
//! signature. If the launcher allows it, the keeper may add a plain wallet that
//! a watched wallet has just sent $BURR to, never removing or replacing one.
//!
//! The watched wallets' accounts for the target coin are passed after the
//! listed accounts, in the order of the list; each is checked to be that
//! wallet's own associated account. One that does not exist counts as 0.

use crate::constants::*;
use crate::error::BurrpadError;
use crate::events::{ShadowBurned, ShadowRecorded, ShadowWatchChanged};
use crate::instructions::burr::*;
use crate::math;
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

/// Each of `wallets`' holdings, from their accounts in the same order.
fn holdings(wallets: &[Pubkey], target: &Pubkey, accounts: &[AccountInfo]) -> Result<Vec<u64>> {
    require!(accounts.len() == wallets.len(), BurrpadError::WrongWatchedAccount);
    wallets.iter().zip(accounts).map(|(w, a)| holding(w, target, a)).collect()
}

fn sum(v: &[u64]) -> Result<u64> {
    v.iter().try_fold(0u64, |t, h| t.checked_add(*h)).ok_or(error!(BurrpadError::Overflow))
}

/// Records the watched wallets' holdings now: a fall of their total since the
/// last record is added to what is owed. The fall is measured on the total, so
/// coins moved between watched wallets count for nothing.
fn record(w: &mut ShadowWatch, now: &[u64]) -> Result<u64> {
    let n = w.count as usize;
    let (before, held) = (sum(&w.held[..n])?, sum(now)?);
    let fell = before.saturating_sub(held);
    w.owed = w.owed.checked_add(fell).ok_or(BurrpadError::Overflow)?;
    w.held[..n].copy_from_slice(now);
    emit!(ShadowRecorded { mint: w.mint, held, fell, owed: w.owed });
    Ok(fell)
}

fn watched(w: &ShadowWatch) -> Vec<Pubkey> {
    w.wallets[..w.count as usize].to_vec()
}

/// A key that can sign: on the curve, so not a derived address (a pool, a
/// vault, a bonding curve).
fn can_sign(k: &Pubkey) -> bool {
    *k != Pubkey::default() && k.is_on_curve()
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

/// Sets the watched wallets (one to sixteen), the keeper, and whether the keeper
/// may add wallets. The old wallets' fall is recorded first; the new wallets
/// start from their holding now, so a change never creates a fall by itself.
/// What is owed stays owed.
/// After the listed accounts: the old wallets' target accounts, then the new ones'.
pub fn handle_set_shadow_watch<'info>(
    ctx: Context<'info, SetShadowWatch<'info>>,
    wallets: Vec<Pubkey>,
    keeper: Pubkey,
    follow: bool,
) -> Result<()> {
    require!(ctx.accounts.coin.shadow_share > 0, BurrpadError::HookNotChosen);
    require!(!wallets.is_empty() && wallets.len() <= SHADOW_MAX_WALLETS, BurrpadError::BadWatchList);
    for (i, a) in wallets.iter().enumerate() {
        require!(*a != Pubkey::default() && !wallets[..i].contains(a), BurrpadError::BadWatchList);
        require!(can_sign(a), BurrpadError::NotAPlainWallet);
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
    let target = w.target;
    let now = holdings(&old, &target, &rest[..old.len()])?;
    record(w, &now)?;
    let new = holdings(&wallets, &target, &rest[old.len()..])?;
    w.wallets = [Pubkey::default(); SHADOW_MAX_WALLETS];
    w.wallets[..wallets.len()].copy_from_slice(&wallets);
    w.held = [0; SHADOW_MAX_WALLETS];
    w.held[..wallets.len()].copy_from_slice(&new);
    w.count = wallets.len() as u8;
    w.keeper = keeper;
    w.follow = follow && keeper != Pubkey::default();
    emit!(ShadowWatchChanged { mint, wallets, keeper, follow: w.follow, by_keeper: false });
    Ok(())
}

#[derive(Accounts)]
pub struct FollowWallet<'info> {
    /// The watch's keeper.
    pub keeper: Signer<'info>,
    /// CHECK: the coin's mint; the watch below is found from it
    pub mint: UncheckedAccount<'info>,
    #[account(mut, seeds = [SEED_SHADOW, mint.key().as_ref()], bump = watch.bump)]
    pub watch: Box<Account<'info, ShadowWatch>>,
    /// CHECK: the wallet to add: checked in the handler to be a plain wallet
    pub wallet: UncheckedAccount<'info>,
    /// CHECK: the new wallet's account for the target coin, checked in the handler
    pub wallet_account: UncheckedAccount<'info>,
    /// CHECK: the account for the target coin of the watched wallet that sent to it, checked in the handler
    pub from_account: UncheckedAccount<'info>,
}

/// The keeper adds `wallet`, which watched wallet number `from` has sent $BURR
/// to: only while the launcher allows it, never past sixteen, never removing or
/// replacing one. Only a plain wallet (a key that can sign, owned by the system
/// program, holding no data), and only one that holds no more $BURR than wallet
/// `from` has let go of since the last record. Its holding moves from `from`'s
/// record to its own, so the move itself is not a fall; its later sales are.
pub fn handle_follow_wallet(ctx: Context<FollowWallet>, from: u8) -> Result<()> {
    let x = &ctx.accounts;
    let w = &x.watch;
    require_keys_eq!(x.keeper.key(), w.keeper, BurrpadError::NotTheKeeper);
    require!(w.follow, BurrpadError::FollowingOff);
    let wallet = x.wallet.key();
    let n = w.count as usize;
    require!(
        n < SHADOW_MAX_WALLETS && (from as usize) < n && !w.wallets[..n].contains(&wallet),
        BurrpadError::BadWatchList
    );
    require!(
        can_sign(&wallet) && *x.wallet.owner == anchor_lang::system_program::ID && x.wallet.data_is_empty(),
        BurrpadError::NotAPlainWallet
    );
    let f = from as usize;
    let sender = holding(&w.wallets[f], &w.target, &x.from_account)?;
    let added = holding(&wallet, &w.target, &x.wallet_account)?;
    let let_go = w.held[f].saturating_sub(sender);
    require!(added > 0 && added <= let_go, BurrpadError::NotTraced);
    let w = &mut ctx.accounts.watch;
    w.held[f] -= added;
    w.wallets[n] = wallet;
    w.held[n] = added;
    w.count += 1;
    emit!(ShadowWatchChanged { mint: w.mint, wallets: watched(w), keeper: w.keeper, follow: true, by_keeper: true });
    Ok(())
}

#[derive(Accounts)]
pub struct ShadowBurn<'info> {
    /// Every account of a $BURR buy, exactly as `burn_burr` takes them; its payer must be the watch's keeper.
    pub burr: BurnBurr<'info>,
    #[account(mut, seeds = [SEED_SHADOW, burr.mint.key().as_ref()], bump = watch.bump)]
    pub watch: Box<Account<'info, ShadowWatch>>,
}

/// The keeper records the watched wallets' holdings; then, for a sale of `sold`
/// coins that received `received` ZEC, buys back and burns $BURR at once, in one
/// buy: with that ZEC, or the whole pot if smaller. Never more coins than are
/// owed: `sold` is held to what is owed, and the ZEC to what buys those coins at
/// the pool's price now. The buy must return the coins sold, at the sale's price,
/// less 5%, or it is refused. With nothing to buy, or `sold` 0, it only records.
/// After the listed accounts: the watched wallets' target accounts, in order.
pub fn handle_shadow_burn<'info>(ctx: Context<'info, ShadowBurn<'info>>, sold: u64, received: u64) -> Result<()> {
    require!(ctx.accounts.burr.coin.shadow_share > 0, BurrpadError::HookNotChosen);
    let w = &mut ctx.accounts.watch;
    require_keys_eq!(ctx.accounts.burr.payer.key(), w.keeper, BurrpadError::NotTheKeeper);
    let now = holdings(&watched(w), &w.target, ctx.remaining_accounts)?;
    record(w, &now)?;
    let (pot, owed) = (ctx.accounts.burr.ledger.shadow.pot, ctx.accounts.watch.owed);
    if sold == 0 || received == 0 || pot == 0 || owed == 0 {
        return Ok(());
    }
    // The coins this buy may take back, and the ZEC the sale received for them.
    let coins = sold.min(owed);
    let paid = math::mul_div(received, coins, sold).ok_or(BurrpadError::Overflow)?;
    let (reserves, _) = burr_pool(&ctx.accounts.burr)?;
    let paid = paid.min(math::spend_for_at_most(&reserves, coins).ok_or(BurrpadError::Overflow)?);
    let spend = pot.min(paid);
    // At least the coins that ZEC got at the sale's price, less the allowance.
    let min_out = math::mul_div(coins, spend, paid)
        .and_then(|c| math::mul_div(c, 10_000 - SHADOW_SLIPPAGE_BPS, 10_000))
        .unwrap_or(0);
    if spend < BURR_MIN_SPEND || min_out == 0 {
        return Ok(());
    }
    let done = burr_buy_now(&mut ctx.accounts.burr, spend, min_out)?;
    // Booked against the eighth pot only; what was bought comes off what is owed.
    let ledger = &mut ctx.accounts.burr.ledger;
    ledger.shadow.pot = ledger.shadow.pot.checked_sub(done.spent).ok_or(BurrpadError::Overflow)?;
    ledger.accounted = ledger.accounted.checked_sub(done.spent).ok_or(BurrpadError::Overflow)?;
    let w = &mut ctx.accounts.watch;
    w.owed = w.owed.saturating_sub(done.bought);
    w.total_spent = w.total_spent.checked_add(done.spent).ok_or(BurrpadError::Overflow)?;
    w.total_burned = w.total_burned.checked_add(done.burned).ok_or(BurrpadError::Overflow)?;
    emit!(ShadowBurned { mint: w.mint, sold, received, spent: done.spent, burned: done.burned, owed: w.owed });
    Ok(())
}
