use crate::constants::*;
use crate::math::{Reserves, VaultShare};
use anchor_lang::prelude::*;

/// The program's settings. One account.
#[account]
#[derive(InitSpace)]
pub struct Config {
    pub admin: Pubkey,
    /// Set by `propose_admin`; becomes `admin` when it accepts.
    pub pending_admin: Pubkey,
    /// Where the platform's share of future launches is paid.
    pub platform_receiver: Pubkey,
    /// The platform's part of a coin's fee, for future launches.
    pub platform_fraction_bps: u16,
    pub min_fee_bps: u16,
    pub max_fee_bps: u16,
    /// The rate a launch page offers first. Informational.
    pub default_fee_bps: u16,
    pub launches_paused: bool,
    pub bump: u8,
}

/// A coin's record. Written once by `launch`. No instruction changes it.
#[account]
#[derive(InitSpace)]
pub struct Coin {
    pub mint: Pubkey,
    pub bump: u8,
    pub authority_bump: u8,
    pub burner_bump: u8,
    pub ledger_bump: u8,
    /// The coin's fee, as set on pump.fun.
    pub fee_bps: u16,
    /// The platform's part of the fee, copied from `Config` at launch.
    pub platform_fraction_bps: u16,
    pub platform_receiver: Pubkey,
    /// Shares of the rest, in order: creator, give, burn, vault share, king.
    pub shares: [u16; 5],
    pub creator_receiver: Pubkey,
    pub give_receiver: Pubkey,
    pub king_round_secs: u32,
    pub burn_cooldown_secs: u32,
    pub burn_price_band_bps: u16,
    pub burn_fee_allowance_bps: u16,
    pub burn_spend_divisor: u16,
    pub vault_release_secs: u32,
    pub launched_at: i64,
    pub launcher: Pubkey,
}

impl Coin {
    pub fn share_of(&self, sink: usize) -> u16 {
        if (1..SINKS).contains(&sink) {
            self.shares[sink - 1]
        } else {
            0
        }
    }
}

/// A coin's money, by pot, and the state of its hooks.
#[account]
#[derive(InitSpace)]
pub struct Ledger {
    pub mint: Pubkey,
    pub bump: u8,
    /// ZEC in the vault that has been through the split: the pots plus the carry.
    pub accounted: u64,
    /// All ZEC ever split.
    pub total_split: u128,
    /// All ZEC ever credited to each pot.
    pub credited: [u128; SINKS],
    /// What each pot holds now.
    pub pots: [u64; SINKS],
    pub burn: BurnState,
    pub vault_share: VaultShare,
    pub king: KingState,
}

#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Default)]
pub struct BurnState {
    pub last_time: i64,
    /// The pool's figures as recorded at the last call. Zero base means none yet.
    pub reference: Reserves,
    pub total_spent: u64,
    pub total_burned: u64,
}

#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Default)]
pub struct KingState {
    /// The round the leader below belongs to.
    pub round: u64,
    /// The empty address means no leader.
    pub leader: Pubkey,
    /// ZEC the leader's largest single buy really spent.
    pub leader_amount: u64,
}

/// One person's place in a coin's vault share.
#[account]
#[derive(InitSpace)]
pub struct Stake {
    pub mint: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
    /// The running total when this stake was last settled.
    pub acc_snapshot: u128,
    /// Earned and not yet paid.
    pub owed: u64,
    pub bump: u8,
}

/// The pots that are simply paid out to a fixed receiver.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Payee {
    Platform,
    Creator,
    Give,
}

impl Payee {
    pub fn sink(&self) -> usize {
        match self {
            Payee::Platform => SINK_PLATFORM,
            Payee::Creator => SINK_CREATOR,
            Payee::Give => SINK_GIVE,
        }
    }
    pub fn receiver(&self, coin: &Coin) -> Pubkey {
        match self {
            Payee::Platform => coin.platform_receiver,
            Payee::Creator => coin.creator_receiver,
            Payee::Give => coin.give_receiver,
        }
    }
}
