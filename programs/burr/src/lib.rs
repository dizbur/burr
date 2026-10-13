//! Burr: fee hooks for ZEC-priced pump.fun coins.
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
pub mod burr {
    use super::*;

    // ---- the administrator ----

    pub fn initialize(ctx: Context<Initialize>, admin: Pubkey, platform_receiver: Pubkey) -> Result<()> {
        instructions::admin::handle_initialize(ctx, admin, platform_receiver)
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

    // ---- vault share ----

    pub fn stake(ctx: Context<StakeCoins>, amount: u64) -> Result<()> {
        instructions::vault_share::handle_stake(ctx, amount)
    }

    /// Takes coins out and pays what was earned. An amount of zero is a plain claim.
    pub fn unstake(ctx: Context<UnstakeCoins>, amount: u64) -> Result<()> {
        instructions::vault_share::handle_unstake(ctx, amount)
    }

    /// Closes an empty stake record and returns its rent.
    pub fn close_stake(ctx: Context<CloseStake>) -> Result<()> {
        instructions::vault_share::handle_close_stake(ctx)
    }

    // ---- king of the hill ----

    pub fn king_buy_curve(ctx: Context<KingBuyCurve>, quote_in: u64, min_out: u64) -> Result<()> {
        instructions::king::handle_king_buy_curve(ctx, quote_in, min_out)
    }

    pub fn king_buy_pool(ctx: Context<KingBuyPool>, quote_in: u64, min_out: u64) -> Result<()> {
        instructions::king::handle_king_buy_pool(ctx, quote_in, min_out)
    }

    pub fn king_settle(ctx: Context<KingSettle>) -> Result<()> {
        instructions::king::handle_king_settle(ctx)
    }
}
