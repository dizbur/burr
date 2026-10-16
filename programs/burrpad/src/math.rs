//! All of the arithmetic, kept apart from accounts so it can be tested on its
//! own with fixed and random inputs. Every operation is checked.

use crate::constants::*;
use anchor_lang::prelude::*;

/// Sum of all split weights. Platform weight is `fraction * 10_000`; a hook's
/// is `(10_000 - fraction) * share`. They add to exactly this.
pub const WEIGHT_TOTAL: u128 = 100_000_000;

/// The split weights of a coin: the platform's, then one per hook.
pub fn weights(platform_fraction_bps: u16, shares: &[u16; HOOKS]) -> Option<[u128; SINKS]> {
    let w = weights_all(platform_fraction_bps, shares, [0, 0, 0])?;
    let mut out = [0u128; SINKS];
    out.copy_from_slice(&w[..SINKS]);
    Some(out)
}

/// The weights of all eight pots: the platform's, the four hooks', and the
/// extra ones (`extra`: $BURR, liquidity, shadow burn). The seven shares must
/// add to the whole.
pub fn weights_all(platform_fraction_bps: u16, shares: &[u16; HOOKS], extra: [u16; 3]) -> Option<[u128; ALL_SINKS]> {
    let [burr_share, liquidity_share, shadow_share] = extra;
    if platform_fraction_bps > BPS {
        return None;
    }
    let mut sum: u32 = burr_share as u32 + liquidity_share as u32 + shadow_share as u32;
    for s in shares {
        sum = sum.checked_add(*s as u32)?;
    }
    if sum != BPS as u32 {
        return None;
    }
    let rest = (BPS - platform_fraction_bps) as u128;
    let mut w = [0u128; ALL_SINKS];
    w[SINK_PLATFORM] = platform_fraction_bps as u128 * BPS as u128;
    for (i, s) in shares.iter().enumerate() {
        w[i + 1] = rest * (*s as u128);
    }
    w[SINK_BURR] = rest * burr_share as u128;
    w[SINK_LIQUIDITY] = rest * liquidity_share as u128;
    w[SINK_SHADOW] = rest * shadow_share as u128;
    Some(w)
}

/// Splits `new` units between the pots.
///
/// Dues are worked out on the running total since launch, so the outcome is
/// the same however the same money is cut into calls. Returns what each pot
/// is credited now. What flooring leaves over (always fewer than `N` units in
/// all) stays in the vault as the carry and is credited by later calls.
pub fn split<const N: usize>(
    total_split: &mut u128,
    credited: &mut [u128; N],
    w: &[u128; N],
    new: u64,
) -> Option<[u64; N]> {
    let total = total_split.checked_add(new as u128)?;
    let mut out = [0u64; N];
    for i in 0..N {
        let due = total.checked_mul(w[i])? / WEIGHT_TOTAL;
        let add = due.checked_sub(credited[i])?;
        out[i] = u64::try_from(add).ok()?;
        credited[i] = due;
    }
    *total_split = total;
    Some(out)
}

/// The price of a coin as the pool's two reserve figures, quote over base.
#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reserves {
    pub quote: u64,
    pub base: u64,
}

impl Reserves {
    /// True if this price is more than `band_bps` above `reference`.
    pub fn above(&self, reference: &Reserves, band_bps: u16) -> Option<bool> {
        // self.quote / self.base  >  reference.quote / reference.base * (1 + band)
        let lhs = (self.quote as u128).checked_mul(reference.base as u128)?.checked_mul(BPS as u128)?;
        let rhs =
            (reference.quote as u128).checked_mul(self.base as u128)?.checked_mul(BPS as u128 + band_bps as u128)?;
        Some(lhs > rhs)
    }

    /// The same reserve figure at a price `band_bps` higher, for stepping a
    /// reference price upward. Only the coin figure moves. The ZEC figure is
    /// what the spend limit is measured against, and a refused call must
    /// never make it larger.
    pub fn price_raised_by(&self, band_bps: u16) -> Option<Reserves> {
        let b = (self.base as u128).checked_mul(BPS as u128)? / (BPS as u128 + band_bps as u128);
        let b = b.min((self.base as u128).saturating_sub(1)).max(1);
        Some(Reserves { quote: self.quote, base: u64::try_from(b).ok()? })
    }

    /// The reserve figure raised by `band_bps`: the most a recorded reserve
    /// may grow from one buy to the next.
    pub fn raised_by(&self, band_bps: u16) -> Option<Reserves> {
        let q = (self.quote as u128).checked_mul(BPS as u128 + band_bps as u128)? / BPS as u128;
        Some(Reserves { quote: u64::try_from(q.max(self.quote as u128 + 1)).ok()?, base: self.base })
    }

    /// The same price, with the quote figure held to at most `max_quote`.
    /// Used when recording a reference, so that the reserve the spend limit
    /// is measured against can grow only a step at a time.
    pub fn held_to(&self, max_quote: u64) -> Option<Reserves> {
        if self.quote <= max_quote || self.quote == 0 {
            return Some(*self);
        }
        let base = (self.base as u128).checked_mul(max_quote as u128)? / self.quote as u128;
        Some(Reserves { quote: max_quote, base: u64::try_from(base.max(1)).ok()? })
    }

    /// Coins a fee-free constant-product pool would give for `spend`.
    pub fn ideal_out(&self, spend: u64) -> Option<u64> {
        let d = (self.quote as u128).checked_add(spend as u128)?;
        if d == 0 {
            return None;
        }
        u64::try_from((spend as u128).checked_mul(self.base as u128)? / d).ok()
    }
}

/// Most a single buy back may spend: `reserve * fee_bps / (10_000 * divisor)`,
/// and never more than the pot.
pub fn burn_spend_limit(pot: u64, quote_reserve: u64, fee_bps: u16, divisor: u16) -> Option<u64> {
    if divisor == 0 {
        return None;
    }
    let cap = (quote_reserve as u128).checked_mul(fee_bps as u128)? / (BPS as u128).checked_mul(divisor as u128)?;
    Some(pot.min(u64::try_from(cap).ok()?))
}

/// The part of `whole` in proportion to `spent` out of a full limit `cap`,
/// rounded up, at least 1 and at most `whole`. A $BURR buy that uses all its
/// limit takes the whole wait and the whole reserve step; a crumb takes a crumb.
pub fn part_used(spent: u64, cap: u64, whole: u64) -> Option<u64> {
    if cap == 0 {
        return None;
    }
    let p = (whole as u128).checked_mul(spent as u128)?.div_ceil(cap as u128);
    Some(u64::try_from(p).ok()?.clamp(1, whole))
}

/// The recorded reserve after a $BURR buy: the pool's figure after it, held
/// within `step_bps` of the last recorded one, up or down.
pub fn burr_next_reserve(last: u64, after: u64, step_bps: u64) -> Option<u64> {
    let move_by = ((last as u128).checked_mul(step_bps as u128)? / BPS as u128).max(1);
    let low = (last as u128).saturating_sub(move_by);
    let high = (last as u128).checked_add(move_by)?;
    u64::try_from((after as u128).clamp(low, high)).ok()
}

/// The pool receipts a deposit can take for `coins` and `zec`: the most both
/// cover at the pool's own figures (`pool_zec` as the deposit counts it).
pub fn lp_for(coins: u64, zec: u64, pool_coins: u64, pool_zec: u64, lp_supply: u64) -> Option<u64> {
    if pool_coins == 0 || pool_zec == 0 {
        return None;
    }
    let by_coins = (coins as u128).checked_mul(lp_supply as u128)? / pool_coins as u128;
    let by_zec = (zec as u128).checked_mul(lp_supply as u128)? / pool_zec as u128;
    u64::try_from(by_coins.min(by_zec)).ok()
}

/// What a deposit of `lp` receipts takes from one side holding `pool_side`:
/// rounded up, as the pool works it out.
pub fn deposit_side(lp: u64, pool_side: u64, lp_supply: u64) -> Option<u64> {
    if lp_supply == 0 {
        return None;
    }
    u64::try_from((lp as u128).checked_mul(pool_side as u128)?.div_ceil(lp_supply as u128)).ok()
}

/// The ZEC that buys at most `coins` from `reserves` at the price before any
/// fee: `quote × coins ÷ (base − coins)`, rounded down. Fees only make it
/// fewer coins, so a buy of this much never takes more than `coins`.
pub fn spend_for_at_most(reserves: &Reserves, coins: u64) -> Option<u64> {
    if coins == 0 || coins >= reserves.base {
        return Some(if coins == 0 { 0 } else { u64::MAX });
    }
    let q = (reserves.quote as u128).checked_mul(coins as u128)? / (reserves.base - coins) as u128;
    u64::try_from(q).ok()
}

/// Fewest coins a buy back accepts: the fee-free figure, less the coin's own
/// fee and an allowance for pump.fun's.
pub fn burn_min_out(reserves: &Reserves, spend: u64, fee_bps: u16, allowance_bps: u16) -> Option<u64> {
    let ideal = reserves.ideal_out(spend)? as u128;
    let keep = (BPS as u128).checked_sub(fee_bps as u128)?.checked_sub(allowance_bps as u128)?;
    u64::try_from(ideal.checked_mul(keep)? / BPS as u128).ok()
}
