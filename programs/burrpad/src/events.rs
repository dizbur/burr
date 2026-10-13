use anchor_lang::prelude::*;

#[event]
pub struct ConfigChanged {
    pub admin: Pubkey,
    pub platform_receiver: Pubkey,
    pub platform_fraction_bps: u16,
    pub min_fee_bps: u16,
    pub max_fee_bps: u16,
    pub default_fee_bps: u16,
    pub launches_paused: bool,
}

/// The administrator named a successor, who has not yet accepted.
#[event]
pub struct AdminProposed {
    pub admin: Pubkey,
    pub pending_admin: Pubkey,
}

#[event]
pub struct Launched {
    pub mint: Pubkey,
    pub launcher: Pubkey,
    pub fee_bps: u16,
    pub platform_fraction_bps: u16,
    pub shares: [u16; 3],
}

#[event]
pub struct Split {
    pub mint: Pubkey,
    pub amount: u64,
    pub credited: [u64; 4],
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
