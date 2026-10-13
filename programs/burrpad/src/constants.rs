//! Fixed addresses, seeds and limits.

use anchor_lang::prelude::*;

/// ZEC on Solana: an ordinary token of the classic token program, 8 decimals.
pub const ZEC_MINT: Pubkey = pubkey!("A7bdiYdS5GjqGFtxf17ppRHtDKPkkRqbKtR27dxvQXaS");

pub const SEED_CONFIG: &[u8] = b"config";
pub const SEED_COIN: &[u8] = b"coin";
pub const SEED_LEDGER: &[u8] = b"ledger";
pub const SEED_AUTHORITY: &[u8] = b"authority";
pub const SEED_BURNER: &[u8] = b"burner";

/// The layout version written into every account this program makes. A
/// later version that needs a different layout can tell old accounts apart.
pub const LAYOUT_VERSION: u8 = 1;

/// One whole, in hundredths of a percent.
pub const BPS: u16 = 10_000;

// Hard limits. The administrator's settings can never leave these.
pub const HARD_MIN_FEE_BPS: u16 = 10;
pub const HARD_MAX_FEE_BPS: u16 = 500;
pub const HARD_MAX_PLATFORM_FRACTION_BPS: u16 = 5_000;

// Starting settings.
pub const START_MIN_FEE_BPS: u16 = 50;
pub const START_MAX_FEE_BPS: u16 = 300;
pub const START_DEFAULT_FEE_BPS: u16 = 200;
pub const START_PLATFORM_FRACTION_BPS: u16 = 1_250;

/// A coin on pump.fun has 6 decimals and these two Token-2022 extensions only.
pub const COIN_DECIMALS: u8 = 6;
pub const EXT_METADATA_POINTER: u16 = 18;
pub const EXT_TOKEN_METADATA: u16 = 19;
pub const ZEC_DECIMALS: u8 = 8;

// Hook settings, copied into each coin's record at launch.
/// Buy back and burn: seconds between calls.
pub const BURN_COOLDOWN_SECS: u32 = 60;
/// Buy back and burn: refuse a price more than this far above the last recorded one.
pub const BURN_PRICE_BAND_BPS: u16 = 500;
/// Buy back and burn: room left for pump.fun's own fee when working out the least coins to accept.
pub const BURN_FEE_ALLOWANCE_BPS: u16 = 200;
/// Buy back and burn: spend per call is at most `reserve * fee_bps / (BPS * this)`.
pub const BURN_SPEND_DIVISOR: u16 = 4;

/// Holders: seconds that must pass between two releases of a coin's holder pot.
pub const HOLDERS_RELEASE_INTERVAL_SECS: u32 = 1_800;

// pump.fun's limits on text, checked here so a launch fails early and clearly.
pub const MAX_NAME_LEN: usize = 32;
pub const MAX_SYMBOL_LEN: usize = 13;
/// pump.fun allows 200. With 100, a launch and the launcher's first buy fit
/// one transaction (1,147 of 1,232 bytes, using an address table).
pub const MAX_URI_LEN: usize = 100;

/// How many hooks a coin can choose from.
pub const HOOKS: usize = 4;

/// The pots of a coin, in ledger order: the platform's, then one per hook.
pub const SINKS: usize = 5;
pub const SINK_PLATFORM: usize = 0;
pub const SINK_CREATOR: usize = 1;
pub const SINK_GIVE: usize = 2;
pub const SINK_BURN: usize = 3;
pub const SINK_HOLDERS: usize = 4;
