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

#[event]
pub struct StakeChanged {
    pub mint: Pubkey,
    pub owner: Pubkey,
    pub staked: u64,
    pub total_staked: u64,
    pub paid: u64,
}
