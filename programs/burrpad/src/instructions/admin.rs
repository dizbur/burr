//! The administrator. It can set four things for future launches, switch new
//! launches off and on, and name the wallet that receives holder pots. That
//! last power reaches existing coins (see `holders.rs`), so it is slow: a new
//! wallet is proposed in the open and can be applied only after a fixed wait.
//! The administrator can touch no other pot and no coin's record.

use crate::constants::*;
use crate::error::BurrpadError;
use crate::events::{AdminProposed, ConfigChanged, RewardsWalletProposed};
use crate::state::Config;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct Initialize<'info> {
    /// Must be the program's upgrade key, so nobody else can initialise first.
    #[account(mut)]
    pub upgrade_authority: Signer<'info>,
    #[account(
        init,
        payer = upgrade_authority,
        space = 8 + Config::INIT_SPACE,
        seeds = [SEED_CONFIG],
        bump
    )]
    pub config: Account<'info, Config>,
    #[account(constraint = program.programdata_address()? == Some(program_data.key()) @ BurrpadError::NotUpgradeAuthority)]
    pub program: Program<'info, crate::program::Burrpad>,
    #[account(constraint = program_data.upgrade_authority_address == Some(upgrade_authority.key()) @ BurrpadError::NotUpgradeAuthority)]
    pub program_data: Account<'info, ProgramData>,
    pub system_program: Program<'info, System>,
}

pub fn handle_initialize(
    ctx: Context<Initialize>,
    admin: Pubkey,
    platform_receiver: Pubkey,
    rewards_wallet: Pubkey,
) -> Result<()> {
    require!(admin != Pubkey::default() && platform_receiver != Pubkey::default(), BurrpadError::EmptyAddress);
    require!(rewards_wallet != Pubkey::default(), BurrpadError::EmptyAddress);
    let c = &mut ctx.accounts.config;
    c.version = LAYOUT_VERSION;
    c.admin = admin;
    c.pending_admin = Pubkey::default();
    c.platform_receiver = platform_receiver;
    c.rewards_wallet = rewards_wallet;
    c.pending_rewards_wallet = Pubkey::default();
    c.rewards_wallet_ready_at = 0;
    c.holder_releases_paused = false;
    c.platform_fraction_bps = START_PLATFORM_FRACTION_BPS;
    c.min_fee_bps = START_MIN_FEE_BPS;
    c.max_fee_bps = START_MAX_FEE_BPS;
    c.default_fee_bps = START_DEFAULT_FEE_BPS;
    c.launches_paused = false;
    c.bump = ctx.bumps.config;
    announce(c);
    Ok(())
}

#[derive(Accounts)]
pub struct AdminOnly<'info> {
    pub admin: Signer<'info>,
    #[account(
        mut,
        seeds = [SEED_CONFIG],
        bump = config.bump,
        constraint = config.admin == admin.key() @ BurrpadError::NotAdmin
    )]
    pub config: Account<'info, Config>,
}

pub fn handle_set_platform_fraction(ctx: Context<AdminOnly>, fraction_bps: u16) -> Result<()> {
    require!(fraction_bps <= HARD_MAX_PLATFORM_FRACTION_BPS, BurrpadError::PlatformFractionTooHigh);
    ctx.accounts.config.platform_fraction_bps = fraction_bps;
    announce(&ctx.accounts.config);
    Ok(())
}

pub fn handle_set_fee_range(ctx: Context<AdminOnly>, min_bps: u16, max_bps: u16, default_bps: u16) -> Result<()> {
    require!(
        min_bps >= HARD_MIN_FEE_BPS && max_bps <= HARD_MAX_FEE_BPS && min_bps <= default_bps && default_bps <= max_bps,
        BurrpadError::FeeSettingOutOfRange
    );
    let c = &mut ctx.accounts.config;
    c.min_fee_bps = min_bps;
    c.max_fee_bps = max_bps;
    c.default_fee_bps = default_bps;
    announce(c);
    Ok(())
}

pub fn handle_set_platform_receiver(ctx: Context<AdminOnly>, receiver: Pubkey) -> Result<()> {
    require!(receiver != Pubkey::default(), BurrpadError::EmptyAddress);
    ctx.accounts.config.platform_receiver = receiver;
    announce(&ctx.accounts.config);
    Ok(())
}

/// First step of replacing the wallet that receives holder pots. The new
/// address is announced now and can be applied only after the wait. A second
/// proposal replaces the first and starts the wait again.
pub fn handle_propose_rewards_wallet(ctx: Context<AdminOnly>, wallet: Pubkey) -> Result<()> {
    require!(wallet != Pubkey::default(), BurrpadError::EmptyAddress);
    let ready_at = Clock::get()?.unix_timestamp.checked_add(REWARDS_WALLET_WAIT_SECS).ok_or(BurrpadError::Overflow)?;
    let c = &mut ctx.accounts.config;
    c.pending_rewards_wallet = wallet;
    c.rewards_wallet_ready_at = ready_at;
    emit!(RewardsWalletProposed { admin: c.admin, current: c.rewards_wallet, proposed: wallet, ready_at });
    Ok(())
}

/// Withdraws a proposed rewards wallet.
pub fn handle_cancel_rewards_wallet(ctx: Context<AdminOnly>) -> Result<()> {
    let c = &mut ctx.accounts.config;
    require!(c.pending_rewards_wallet != Pubkey::default(), BurrpadError::NoRewardsWalletProposed);
    c.pending_rewards_wallet = Pubkey::default();
    c.rewards_wallet_ready_at = 0;
    emit!(RewardsWalletProposed {
        admin: c.admin,
        current: c.rewards_wallet,
        proposed: Pubkey::default(),
        ready_at: 0
    });
    Ok(())
}

/// Second step: the proposed wallet replaces the current one, for every
/// coin, from now on. Only once the wait is over.
pub fn handle_apply_rewards_wallet(ctx: Context<AdminOnly>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let c = &mut ctx.accounts.config;
    require!(c.pending_rewards_wallet != Pubkey::default(), BurrpadError::NoRewardsWalletProposed);
    require!(now >= c.rewards_wallet_ready_at, BurrpadError::RewardsWalletWaitNotOver);
    c.rewards_wallet = c.pending_rewards_wallet;
    c.pending_rewards_wallet = Pubkey::default();
    c.rewards_wallet_ready_at = 0;
    announce(c);
    Ok(())
}

pub fn handle_set_launches_paused(ctx: Context<AdminOnly>, paused: bool) -> Result<()> {
    ctx.accounts.config.launches_paused = paused;
    announce(&ctx.accounts.config);
    Ok(())
}

/// Stops, or allows again, the release of every coin's holder pot. At once:
/// this is how a stolen rewards wallet is shut out without waiting. Nothing
/// is moved; the pots wait.
pub fn handle_set_holder_releases_paused(ctx: Context<AdminOnly>, paused: bool) -> Result<()> {
    ctx.accounts.config.holder_releases_paused = paused;
    announce(&ctx.accounts.config);
    Ok(())
}

/// First step of handing the role over. Proposing the empty address cancels.
pub fn handle_propose_admin(ctx: Context<AdminOnly>, new_admin: Pubkey) -> Result<()> {
    ctx.accounts.config.pending_admin = new_admin;
    emit!(AdminProposed { admin: ctx.accounts.config.admin, pending_admin: new_admin });
    Ok(())
}

#[derive(Accounts)]
pub struct AcceptAdmin<'info> {
    pub new_admin: Signer<'info>,
    #[account(
        mut,
        seeds = [SEED_CONFIG],
        bump = config.bump,
        constraint = config.pending_admin == new_admin.key() @ BurrpadError::NotPendingAdmin
    )]
    pub config: Account<'info, Config>,
}

pub fn handle_accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
    let c = &mut ctx.accounts.config;
    c.admin = c.pending_admin;
    c.pending_admin = Pubkey::default();
    // A new administrator does not inherit a rewards-wallet change it never
    // proposed: it must propose its own, in the open, and wait the full time.
    c.pending_rewards_wallet = Pubkey::default();
    c.rewards_wallet_ready_at = 0;
    announce(c);
    Ok(())
}

fn announce(c: &Config) {
    emit!(ConfigChanged {
        admin: c.admin,
        platform_receiver: c.platform_receiver,
        rewards_wallet: c.rewards_wallet,
        platform_fraction_bps: c.platform_fraction_bps,
        min_fee_bps: c.min_fee_bps,
        max_fee_bps: c.max_fee_bps,
        default_fee_bps: c.default_fee_bps,
        launches_paused: c.launches_paused,
        holder_releases_paused: c.holder_releases_paused,
    });
}
