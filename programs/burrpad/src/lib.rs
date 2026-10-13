//! Burrpad: fee hooks for ZEC-priced pump.fun coins.
//!
//! A coin launched through this program is a real pump.fun coin whose creator
//! is an address only this program can sign for. Its creator fee can
//! therefore only arrive in a vault this program controls, and is split by
//! rules written once at launch.

pub mod constants;
pub mod error;
pub mod events;
pub mod instructions;
pub mod math;
pub mod ops;
pub mod pump;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("GDbkhHSpg15QWNwqws6SKyrwFMhe2AWQ59UxJciLa6Qt");

#[program]
pub mod burrpad {
    use super::*;

    // ---- the administrator ----

    pub fn initialize(
        ctx: Context<Initialize>,
        admin: Pubkey,
        platform_receiver: Pubkey,
        rewards_wallet: Pubkey,
    ) -> Result<()> {
        instructions::admin::handle_initialize(ctx, admin, platform_receiver, rewards_wallet)
    }

    pub fn propose_rewards_wallet(ctx: Context<AdminOnly>, wallet: Pubkey) -> Result<()> {
        instructions::admin::handle_propose_rewards_wallet(ctx, wallet)
    }

    pub fn cancel_rewards_wallet(ctx: Context<AdminOnly>) -> Result<()> {
        instructions::admin::handle_cancel_rewards_wallet(ctx)
    }

    pub fn apply_rewards_wallet(ctx: Context<AdminOnly>) -> Result<()> {
        instructions::admin::handle_apply_rewards_wallet(ctx)
    }

    pub fn set_holder_releases_paused(ctx: Context<AdminOnly>, paused: bool) -> Result<()> {
        instructions::admin::handle_set_holder_releases_paused(ctx, paused)
    }

    pub fn set_platform_fraction(ctx: Context<AdminOnly>, fraction_bps: u16) -> Result<()> {
        instructions::admin::handle_set_platform_fraction(ctx, fraction_bps)
    }

    pub fn set_fee_range(ctx: Context<AdminOnly>, min_bps: u16, max_bps: u16, default_bps: u16) -> Result<()> {
        instructions::admin::handle_set_fee_range(ctx, min_bps, max_bps, default_bps)
    }

    pub fn set_platform_receiver(ctx: Context<AdminOnly>, receiver: Pubkey) -> Result<()> {
        instructions::admin::handle_set_platform_receiver(ctx, receiver)
    }

    pub fn set_launches_paused(ctx: Context<AdminOnly>, paused: bool) -> Result<()> {
        instructions::admin::handle_set_launches_paused(ctx, paused)
    }

    pub fn propose_admin(ctx: Context<AdminOnly>, new_admin: Pubkey) -> Result<()> {
        instructions::admin::handle_propose_admin(ctx, new_admin)
    }

    pub fn accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
        instructions::admin::handle_accept_admin(ctx)
    }

    // ---- a coin ----

    pub fn launch(ctx: Context<Launch>, args: LaunchArgs) -> Result<()> {
        instructions::launch::handle_launch(ctx, args)
    }

    pub fn split(ctx: Context<SplitOnly>) -> Result<()> {
        instructions::collect::handle_split(ctx)
    }

    pub fn collect_curve(ctx: Context<CollectCurve>) -> Result<()> {
        instructions::collect::handle_collect_curve(ctx)
    }

    pub fn collect_pool(ctx: Context<CollectPool>) -> Result<()> {
        instructions::collect::handle_collect_pool(ctx)
    }

    pub fn pay_out(ctx: Context<PayOut>, payee: Payee) -> Result<()> {
        instructions::pay_out::handle_pay_out(ctx, payee)
    }

    // ---- buy back and burn ----

    pub fn burn_curve(ctx: Context<BurnCurve>) -> Result<()> {
        instructions::burn::handle_burn_curve(ctx)
    }

    pub fn burn_pool(ctx: Context<BurnPool>) -> Result<()> {
        instructions::burn::handle_burn_pool(ctx)
    }

    // ---- holders ----

    /// Hands part or all of a coin's holder pot to the rewards wallet for one payout run.
    pub fn release_holder_rewards(
        ctx: Context<ReleaseHolderRewards>,
        amount: u64,
        snapshot_slot: u64,
        snapshot_hash: [u8; 32],
    ) -> Result<()> {
        instructions::holders::handle_release_holder_rewards(ctx, amount, snapshot_slot, snapshot_hash)
    }
}
