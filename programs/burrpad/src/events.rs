use anchor_lang::prelude::*;

#[event]
pub struct ConfigChanged {
    pub admin: Pubkey,
    pub platform_receiver: Pubkey,
    pub rewards_wallet: Pubkey,
    pub platform_fraction_bps: u16,
    pub min_fee_bps: u16,
    pub max_fee_bps: u16,
    pub default_fee_bps: u16,
    pub launches_paused: bool,
    pub holder_releases_paused: bool,
}

/// The administrator named a successor, who has not yet accepted.
#[event]
pub struct AdminProposed {
    pub admin: Pubkey,
    pub pending_admin: Pubkey,
}

/// The administrator proposed a new rewards wallet, or cancelled a proposal
/// (then `proposed` is the empty address). It can be applied at `ready_at`.
#[event]
pub struct RewardsWalletProposed {
    pub admin: Pubkey,
    pub current: Pubkey,
    pub proposed: Pubkey,
    pub ready_at: i64,
}

#[event]
pub struct Launched {
    pub mint: Pubkey,
    pub launcher: Pubkey,
    pub fee_bps: u16,
    pub platform_fraction_bps: u16,
    pub shares: [u16; 4],
}

#[event]
pub struct Split {
    pub mint: Pubkey,
    pub amount: u64,
    pub credited: [u64; 5],
}

/// ZEC found in another account of the coin's authority was moved to its vault.
#[event]
pub struct Recovered {
    pub mint: Pubkey,
    pub from: Pubkey,
    pub amount: u64,
}

/// What a split credited to the pot that buys $BURR.
#[event]
pub struct SplitBurr {
    pub mint: Pubkey,
    pub credited: u64,
}

/// What a split gave the liquidity pot.
#[event]
pub struct SplitLiquidity {
    pub mint: Pubkey,
    pub credited: u64,
}

/// Liquidity added to a coin's own pool for good: ZEC and coins deposited,
/// ZEC spent buying those coins, and pool receipts burned.
#[event]
pub struct LiquidityAdded {
    pub mint: Pubkey,
    pub zec_spent_buying: u64,
    pub zec_deposited: u64,
    pub coins_deposited: u64,
    pub lp_burned: u64,
}

/// A liquidity call that found the pool's price outside the band: it only
/// stepped the recorded price toward the pool's.
#[event]
pub struct LiquidityStepped {
    pub mint: Pubkey,
}

/// $BURR was bought with a coin's sixth pot and burned.
#[event]
pub struct BurrBurned {
    pub mint: Pubkey,
    pub spent: u64,
    pub burned: u64,
}

#[event]
pub struct PaidOut {
    pub mint: Pubkey,
    pub pot: u8,
    pub receiver: Pubkey,
    pub amount: u64,
}

#[event]
pub struct Burned {
    pub mint: Pubkey,
    pub spent: u64,
    pub burned: u64,
}

#[event]
pub struct BurnSkipped {
    pub mint: Pubkey,
}

/// A coin's holder pot was handed to the rewards wallet for one payout run.
/// Who is paid what in that run is worked out off the chain; the fingerprint
/// lets anyone check the published snapshot against what was committed here.
#[event]
pub struct HolderRewardsReleased {
    pub mint: Pubkey,
    pub rewards_wallet: Pubkey,
    pub amount: u64,
    pub run: u64,
    pub snapshot_slot: u64,
    pub snapshot_hash: [u8; 32],
}
