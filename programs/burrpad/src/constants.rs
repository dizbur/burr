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
/// The platform's part can never be set below a quarter. A creator then gets
/// back at most three quarters of the fee they pay, and the buy back's limit
/// hands a trader around it at most a quarter of the fees paid: so for a
/// coin's own creator the two cancel, and pump.fun's own fee makes trading
/// around the buy back a loss. At the starting half the loss is much larger.
pub const HARD_MIN_PLATFORM_FRACTION_BPS: u16 = 2_500;

// Starting settings.
pub const START_MIN_FEE_BPS: u16 = 50;
pub const START_MAX_FEE_BPS: u16 = 300;
pub const START_DEFAULT_FEE_BPS: u16 = 200;
/// The platform's part of each coin's fee: one half. The rest goes to the coin's hooks.
pub const START_PLATFORM_FRACTION_BPS: u16 = 5_000;

/// A coin on pump.fun has 6 decimals and these two Token-2022 extensions only.
pub const COIN_DECIMALS: u8 = 6;
pub const EXT_METADATA_POINTER: u16 = 18;
pub const EXT_TOKEN_METADATA: u16 = 19;
pub const ZEC_DECIMALS: u8 = 8;

// Hook settings, copied into each coin's record at launch.
/// Buy back and burn: seconds between calls.
pub const BURN_COOLDOWN_SECS: u32 = 60;
/// Buy back and burn: no longer used. Until 14 Oct 2026 a burn skipped a price
/// more than this far above its reference; that rule is gone for every coin, and
/// the figure in a coin's record is not read. New coins record 0.
pub const BURN_PRICE_BAND_BPS: u16 = 0;
/// Buy back and burn: the most the reserve the spend limit is measured against
/// may grow from one buy to the next. It stops liquidity added around one burn
/// from raising the limit of the next. The same for every coin.
pub const BURN_RESERVE_STEP_BPS: u16 = 500;
/// Buy back and burn: room left for pump.fun's own fee when working out the least coins to accept.
pub const BURN_FEE_ALLOWANCE_BPS: u16 = 200;
/// Buy back and burn: spend per call is at most `reserve * fee_bps / (BPS * this)`.
pub const BURN_SPEND_DIVISOR: u16 = 4;

/// Holders: seconds a newly proposed rewards wallet must wait before it can
/// take the place of the current one.
pub const REWARDS_WALLET_WAIT_SECS: i64 = 86_400;

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

/// The sixth pot: buying and burning $BURR. It is kept apart from the five
/// above (in spare room of the records), so that launched coins' records keep
/// their layout. Its share counts with the four hooks': all five add to 10,000.
pub const SINK_BURR: usize = 5;
/// The seventh pot: liquidity added to the coin's own pool for good. Kept
/// apart in spare room too; all six shares add to 10,000.
pub const SINK_LIQUIDITY: usize = 6;
/// Payout choice: the creator is paid in ZEC on Zcash. The creator pot goes
/// to `Config.creator_payout_wallet`, which Burrpad's server forwards.
pub const PAYOUT_PRIVATE_CREATOR: u8 = 1;
/// Payout choice: holders are paid in $BURR instead of ZEC (by the server).
pub const PAYOUT_HOLDERS_IN_BURR: u8 = 2;
/// Payout choice: holders who hold longer are weighted more (by the server),
/// with the step and the most fixed in the coin's record at launch.
pub const PAYOUT_LONG_HOLDER_BONUS: u8 = 4;
/// The payout choices defined so far. Every other bit is refused until an
/// upgrade defines it.
pub const PAYOUT_DEFINED: u8 = PAYOUT_PRIVATE_CREATOR | PAYOUT_HOLDERS_IN_BURR | PAYOUT_LONG_HOLDER_BONUS;
/// The most a long-holder bonus may reach, in hundredths of a percent: 300%.
pub const LONG_HOLDER_BONUS_MAX_BPS: u16 = 30_000;
/// The eighth pot: the shadow burn, which buys back and burns $BURR up to what
/// the watched wallets have let go of. Kept in spare room too.
pub const SINK_SHADOW: usize = 7;
/// All pots, the sixth to eighth included, as the split works them out.
pub const ALL_SINKS: usize = 8;
/// The least ZEC (in units) a $BURR buy is tried with: 0.00001 ZEC. Below it,
/// PumpSwap's fees, each rounded up, can leave fewer coins than the least-out
/// allows and it refuses the buy; a shadow burn then only records.
pub const BURR_MIN_SPEND: u64 = 1_000;
/// Seed of a coin's shadow-burn watch.
pub const SEED_SHADOW: &[u8] = b"shadow";
/// The most wallets one shadow watch follows. With an address lookup table,
/// a shadow burn's 29 listed accounts and 16 watched accounts stay well inside
/// a transaction's 64 accounts, and so do a change of list's 16 old and 16 new.
pub const SHADOW_MAX_WALLETS: usize = 16;
/// A shadow buy must return at least the coins let go of, at the sale's own
/// price, less this, in hundredths of a percent: 35%, the owner's choice. A
/// trade around the buy that moves the price more is refused. Kept as this one
/// figure so that it is easy to change: 10,000 would remove the guard.
pub const SHADOW_SLIPPAGE_BPS: u64 = 3_500;

/// $BURR, the platform's own coin: what the sixth pot buys and burns.
pub const BURR_MINT: Pubkey = pubkey!("GiwMfYS4jHi256G8K41snZhKUovrMbyQL2hqv3fqr7WU");
/// The reserve the first $BURR buy's limit is measured against, before any has
/// been recorded: 1 ZEC. It then grows 5% per buy towards the pool's own, so
/// nobody can raise the first limit by adding liquidity just before it.
pub const BURR_START_RESERVE: u64 = 100_000_000;
