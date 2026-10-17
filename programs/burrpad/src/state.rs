use crate::constants::*;
use crate::math::Reserves;
use anchor_lang::prelude::*;

/// The program's settings. One account.
#[account]
#[derive(InitSpace)]
pub struct Config {
    /// Layout version of this account.
    pub version: u8,
    pub admin: Pubkey,
    /// Set by `propose_admin`; becomes `admin` when it accepts.
    pub pending_admin: Pubkey,
    /// Where the platform's share of future launches is paid.
    pub platform_receiver: Pubkey,
    /// The wallet that receives every coin's holder pot and pays holders
    /// from it. It is read at each release, so replacing it reaches every
    /// coin at once. Replacing it takes a proposal and a fixed wait.
    pub rewards_wallet: Pubkey,
    /// The proposed next rewards wallet; the empty address when there is none.
    pub pending_rewards_wallet: Pubkey,
    /// When the proposed wallet may be applied.
    pub rewards_wallet_ready_at: i64,
    /// While set, no coin's holder pot can be released. The pots keep filling.
    pub holder_releases_paused: bool,
    /// The platform's part of a coin's fee, for future launches.
    pub platform_fraction_bps: u16,
    pub min_fee_bps: u16,
    pub max_fee_bps: u16,
    /// The rate a launch page offers first. Informational.
    pub default_fee_bps: u16,
    pub launches_paused: bool,
    pub bump: u8,
    /// One waiting time and one recorded reserve for every coin's $BURR buys
    /// together, so that several coins' buys cannot be packed into one moment.
    pub burr_buy: BurrBuy,
    /// Burrpad's one wallet for private creator payouts: it receives the
    /// creator pot of every coin launched with that choice, and forwards it to
    /// the creator's Zcash address. Set by the administrator, then confirmed
    /// by the wallet itself; once confirmed it never changes.
    pub creator_payout_wallet: Pubkey,
    pub creator_payout_confirmed: bool,
    /// Unused. Room for fields a later version may add without moving the account.
    pub reserved: [u8; 7],
}

/// A coin's record. Written once by `launch`. No instruction changes it.
#[account]
#[derive(InitSpace)]
pub struct Coin {
    /// Layout version of this account.
    pub version: u8,
    pub mint: Pubkey,
    pub bump: u8,
    pub authority_bump: u8,
    pub burner_bump: u8,
    /// The coin's fee, as set on pump.fun.
    pub fee_bps: u16,
    /// The platform's part of the fee, copied from `Config` at launch.
    pub platform_fraction_bps: u16,
    pub platform_receiver: Pubkey,
    /// Shares of the rest, in order: creator, give, burn, holders.
    pub shares: [u16; HOOKS],
    pub creator_receiver: Pubkey,
    pub give_receiver: Pubkey,
    pub burn_cooldown_secs: u32,
    pub burn_price_band_bps: u16,
    pub burn_fee_allowance_bps: u16,
    pub burn_spend_divisor: u16,
    pub holders_release_interval_secs: u32,
    pub launched_at: i64,
    pub launcher: Pubkey,
    /// Share of the hooks' part that buys and burns $BURR. Zero for every coin
    /// launched before this field existed: they did not choose it.
    pub burr_share: u16,
    /// Share of the hooks' part that is added to the coin's own pool for good.
    /// Zero for every coin launched before this field existed.
    pub liquidity_share: u16,
    /// Payout choices made at launch, as bits (`PAYOUT_*`). Paid by Burrpad's
    /// server; recorded here so they are fixed and public. Zero for every coin
    /// launched before this field existed.
    pub payout_options: u8,
    /// With the long-holder bonus: the bonus per day held, and the most it
    /// reaches, in hundredths of a percent. Zero without it.
    pub long_holder_step_bps: u16,
    pub long_holder_max_bps: u16,
    /// Share of the hooks' part that fills the shadow-burn pot. Zero for every
    /// coin launched before this field existed.
    pub shadow_share: u16,
    /// What the coin is priced in (`QUOTE_ZEC` or `QUOTE_BURR`). Zero, ZEC, for
    /// every coin launched before this field existed.
    pub quote: u8,
    /// Unused. Room for fields a later version may add without moving the account.
    pub reserved: [u8; 52],
}

impl Coin {
    pub fn share_of(&self, sink: usize) -> u16 {
        if (1..SINKS).contains(&sink) {
            self.shares[sink - 1]
        } else {
            0
        }
    }

    /// The coin's quote mint: ZEC or $BURR. A record only ever holds a defined quote.
    pub fn quote_mint(&self) -> Pubkey {
        quote_parts(self.quote).map(|q| q.0).unwrap_or_default()
    }

    /// The token program of the coin's quote mint.
    pub fn quote_program(&self) -> Pubkey {
        quote_parts(self.quote).map(|q| q.1).unwrap_or_default()
    }

    /// The decimals of the coin's quote mint.
    pub fn quote_decimals(&self) -> u8 {
        quote_parts(self.quote).map(|q| q.2).unwrap_or_default()
    }
}

/// A coin's money, by pot, and the state of its hooks.
#[account]
#[derive(InitSpace)]
pub struct Ledger {
    /// Layout version of this account.
    pub version: u8,
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
    pub holders: HoldersState,
    /// The sixth pot, for buying and burning $BURR.
    pub burr: BurrPot,
    /// The seventh pot, for adding liquidity to the coin's own pool.
    pub liquidity: LiquidityPot,
    /// The eighth pot, for the shadow burn (its totals are in the coin's `ShadowWatch`).
    pub shadow: ShadowPot,
    /// Unused. Room for fields a later version may add without moving the account.
    pub reserved: [u8; 8],
}

#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Default)]
pub struct BurnState {
    pub last_time: i64,
    /// The pool's figures as recorded at the last call; at first, those at launch.
    pub reference: Reserves,
    pub total_spent: u64,
    pub total_burned: u64,
}

#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Default)]
pub struct LiquidityPot {
    /// What the pot holds now.
    pub pot: u64,
    /// All ZEC ever credited to it.
    pub credited: u128,
    /// ZEC that went into the pool: the half spent buying coins and the half deposited.
    pub total_spent: u64,
    /// Pool receipts (LP tokens) burned, so that liquidity can never be withdrawn.
    pub total_lp_burned: u64,
    /// The pool's figures as recorded at the last call: the reserve the limit
    /// is measured against.
    pub reference: Reserves,
}

#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Default)]
pub struct ShadowPot {
    /// What the pot holds now.
    pub pot: u64,
    /// All ZEC ever credited to it.
    pub credited: u128,
}

/// A coin's shadow burn: the wallets it watches, each one's holding of the
/// target coin at the last record, and how many target coins they have let go
/// of that have not yet been bought back and burned. One per coin, made by its launcher.
#[account]
#[derive(InitSpace)]
pub struct ShadowWatch {
    pub version: u8,
    pub bump: u8,
    pub mint: Pubkey,
    /// The coin bought back and burned ($BURR).
    pub target: Pubkey,
    /// The key that buys back and may add wallets (Burrpad's keeper), named by the launcher.
    pub keeper: Pubkey,
    /// Whether the keeper may add wallets.
    pub follow: bool,
    pub count: u8,
    pub wallets: [Pubkey; SHADOW_MAX_WALLETS],
    /// Each watched wallet's holding of the target at the last record.
    pub held: [u64; SHADOW_MAX_WALLETS],
    /// Target coins let go of since the watch began, not yet bought back and burned.
    pub owed: u64,
    /// ZEC the pot spent, and $BURR burned, by shadow burns.
    pub total_spent: u64,
    pub total_burned: u64,
    /// Unused. Room for fields a later version may add without moving the account.
    pub reserved: [u8; 64],
}

#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Default)]
pub struct BurrPot {
    /// What the pot holds now.
    pub pot: u64,
    /// All ZEC ever credited to it.
    pub credited: u128,
    pub total_spent: u64,
    pub total_burned: u64,
}

/// Shared by all coins: when the next $BURR buy may run, and the reserve the
/// limit is measured against. A buy holds the next one off, and moves the
/// recorded reserve, in proportion to how much of its limit it used.
#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Default)]
pub struct BurrBuy {
    pub ready_at: i64,
    pub reference: Reserves,
}

#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Default)]
pub struct HoldersState {
    /// When this coin's holder pot was last released; at first, its launch.
    pub last_release: i64,
    /// The slot in which that release landed; at first, the launch's slot.
    /// The next snapshot must be later than this.
    pub last_release_slot: u64,
    /// The slot and fingerprint of the snapshot the last release was for.
    pub last_snapshot_slot: u64,
    pub last_snapshot_hash: [u8; 32],
    pub runs: u64,
    pub total_released: u64,
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
