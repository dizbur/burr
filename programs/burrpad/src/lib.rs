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

declare_id!("42w1jV45WeSmfM96vr6GrqSWRcyF5rVbPWjmQ89bvdhg");

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

    pub fn set_creator_payout_wallet(ctx: Context<AdminOnly>, wallet: Pubkey) -> Result<()> {
        instructions::admin::handle_set_creator_payout_wallet(ctx, wallet)
    }

    pub fn confirm_creator_payout_wallet(ctx: Context<ConfirmPayoutWallet>) -> Result<()> {
        instructions::admin::handle_confirm_creator_payout_wallet(ctx)
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

    pub fn launch<'info>(ctx: Context<'info, Launch<'info>>, args: LaunchArgs) -> Result<()> {
        instructions::launch::handle_launch(ctx, args)
    }

    /// The same, with part of the hooks' share buying and burning $BURR.
    pub fn launch_v2<'info>(ctx: Context<'info, Launch<'info>>, args: LaunchArgs, burr_share: u16) -> Result<()> {
        instructions::launch::handle_launch_with_extras(
            ctx,
            args,
            burr_share,
            0,
            PayoutChoices::default(),
            0,
            QUOTE_ZEC,
        )
    }

    /// The same, with part of the hooks' share also added to the coin's own
    /// pool for good, and the payout choices made at launch.
    pub fn launch_v3<'info>(
        ctx: Context<'info, Launch<'info>>,
        args: LaunchArgs,
        burr_share: u16,
        liquidity_share: u16,
        payout: PayoutChoices,
    ) -> Result<()> {
        instructions::launch::handle_launch_with_extras(ctx, args, burr_share, liquidity_share, payout, 0, QUOTE_ZEC)
    }

    /// The same, with part of the hooks' share also filling the shadow-burn pot.
    pub fn launch_v4<'info>(
        ctx: Context<'info, Launch<'info>>,
        args: LaunchArgs,
        burr_share: u16,
        liquidity_share: u16,
        payout: PayoutChoices,
        shadow_share: u16,
    ) -> Result<()> {
        instructions::launch::handle_launch_with_extras(
            ctx,
            args,
            burr_share,
            liquidity_share,
            payout,
            shadow_share,
            QUOTE_ZEC,
        )
    }

    /// A launch priced in `quote`: ZEC, or $BURR (`QUOTE_BURR`). A coin priced in
    /// $BURR may choose the four hooks and the $BURR share only, and passes
    /// $BURR's curve, its pool and the pool's two vaults first after the listed
    /// accounts (then, optionally, the launcher's coin account).
    pub fn launch_v5<'info>(
        ctx: Context<'info, Launch<'info>>,
        args: LaunchArgs,
        burr_share: u16,
        quote: u8,
    ) -> Result<()> {
        instructions::launch::handle_launch_with_extras(ctx, args, burr_share, 0, PayoutChoices::default(), 0, quote)
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

    /// Moves ZEC from another account of the coin's authority into its vault, and splits it.
    pub fn recover(ctx: Context<Recover>) -> Result<()> {
        instructions::collect::handle_recover(ctx)
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

    /// Buys $BURR with the coin's sixth pot and burns it.
    pub fn burn_burr(ctx: Context<BurnBurr>) -> Result<()> {
        instructions::burr::handle_burn_burr(ctx)
    }

    /// For a coin priced in $BURR: burns its sixth pot, already $BURR, as it is.
    pub fn burn_burr_pot(ctx: Context<BurnBurrPot>) -> Result<()> {
        instructions::burr::handle_burn_burr_pot(ctx)
    }

    /// Sets or changes a coin's shadow-burn watch. Only its launcher.
    pub fn set_shadow_watch<'info>(
        ctx: Context<'info, SetShadowWatch<'info>>,
        wallets: Vec<Pubkey>,
        keeper: Pubkey,
        follow: bool,
    ) -> Result<()> {
        instructions::shadow::handle_set_shadow_watch(ctx, wallets, keeper, follow)
    }

    /// The keeper adds the plain wallet that watched wallet `from` sent $BURR to.
    pub fn follow_wallet(ctx: Context<FollowWallet>, from: u8) -> Result<()> {
        instructions::shadow::handle_follow_wallet(ctx, from)
    }

    /// The keeper records the watched wallets' holding and buys back and burns
    /// $BURR at once for a sale of `sold` coins that received `received` ZEC.
    pub fn shadow_burn<'info>(ctx: Context<'info, ShadowBurn<'info>>, sold: u64, received: u64) -> Result<()> {
        instructions::shadow::handle_shadow_burn(ctx, sold, received)
    }

    /// Adds part of a graduated coin's fee to its own pool and burns the receipts.
    pub fn add_liquidity(ctx: Context<AddLiquidity>) -> Result<()> {
        instructions::liquidity::handle_add_liquidity(ctx)
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
