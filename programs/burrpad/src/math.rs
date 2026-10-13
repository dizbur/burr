//! All of the arithmetic, kept apart from accounts so it can be tested on its
//! own with fixed and random inputs. Every operation is checked.

use crate::constants::*;
use anchor_lang::prelude::*;

/// Sum of all split weights. Platform weight is `fraction * 10_000`; a hook's
/// is `(10_000 - fraction) * share`. They add to exactly this.
pub const WEIGHT_TOTAL: u128 = 100_000_000;

/// Fixed-point scale of the vault share's running total.
pub const ACC_SCALE: u128 = 1_000_000_000_000_000_000;

/// The six split weights of a coin.
pub fn weights(platform_fraction_bps: u16, shares: &[u16; HOOKS]) -> Option<[u128; SINKS]> {
    if platform_fraction_bps > BPS {
        return None;
    }
    let mut sum: u32 = 0;
    for s in shares {
        sum = sum.checked_add(*s as u32)?;
    }
    if sum != BPS as u32 {
        return None;
    }
    let rest = (BPS - platform_fraction_bps) as u128;
    let mut w = [0u128; SINKS];
    w[SINK_PLATFORM] = platform_fraction_bps as u128 * BPS as u128;
    for (i, s) in shares.iter().enumerate() {
        w[i + 1] = rest * (*s as u128);
    }
    Some(w)
}

/// Splits `new` units between the pots.
///
/// Dues are worked out on the running total since launch, so the outcome is
/// the same however the same money is cut into calls. Returns what each pot
/// is credited now. What flooring leaves over (always fewer than `SINKS`
/// units in all) stays in the vault as the carry and is credited by later calls.
pub fn split(
    total_split: &mut u128,
    credited: &mut [u128; SINKS],
    w: &[u128; SINKS],
    new: u64,
) -> Option<[u64; SINKS]> {
    let total = total_split.checked_add(new as u128)?;
    let mut out = [0u64; SINKS];
    for i in 0..SINKS {
        let due = total.checked_mul(w[i])? / WEIGHT_TOTAL;
        let add = due.checked_sub(credited[i])?;
        out[i] = u64::try_from(add).ok()?;
        credited[i] = due;
    }
    *total_split = total;
    Some(out)
}

/// Vault share: the running-total reward method, with credited fee released
/// evenly over time.
#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VaultShare {
    /// Coins placed in the vault, all stakers together.
    pub total_staked: u64,
    /// ZEC credited to this pot and not yet released to stakers.
    pub queued: u64,
    /// When the figures below were last brought up to date.
    pub last_update: i64,
    /// When the queue will be fully released.
    pub release_end: i64,
    /// Running total of released ZEC per staked coin, times `ACC_SCALE`.
    pub acc: u128,
}

impl VaultShare {
    /// Brings the running total up to `now`.
    ///
    /// While nobody is staked nothing is released: the queue waits, and its
    /// release period starts afresh when the first staker arrives.
    pub fn update(&mut self, now: i64, release_secs: u32) -> Option<()> {
        if now <= self.last_update {
            return Some(());
        }
        if self.total_staked == 0 {
            if self.queued > 0 {
                self.release_end = now.checked_add(release_secs as i64)?;
            }
            self.last_update = now;
            return Some(());
        }
        if self.queued > 0 {
            let release = if now >= self.release_end {
                self.queued
            } else {
                let elapsed = now.checked_sub(self.last_update)? as u128;
                let window = self.release_end.checked_sub(self.last_update)? as u128;
                // elapsed < window here, so the result is below `queued`.
                ((self.queued as u128).checked_mul(elapsed)? / window) as u64
            };
            let add = (release as u128).checked_mul(ACC_SCALE)? / self.total_staked as u128;
            self.acc = self.acc.checked_add(add)?;
            self.queued = self.queued.checked_sub(release)?;
        }
        self.last_update = now;
        Some(())
    }

    /// New fee for this pot. Call `update` first.
    ///
    /// The new amount is due over the full release time; what is already
    /// queued keeps the time it had left. The queue has one end time, so it
    /// is set to the average of the two, weighted by amount. A small credit
    /// therefore moves the end by very little: it cannot be used to hold back
    /// a large queue, and a busy coin is not slowed by its own trading.
    pub fn credit(&mut self, amount: u64, now: i64, release_secs: u32) -> Option<()> {
        if amount == 0 {
            return Some(());
        }
        let left = if self.queued == 0 { 0 } else { self.release_end.checked_sub(now)?.max(0) as u128 };
        let old = self.queued as u128;
        let new = amount as u128;
        let window =
            old.checked_mul(left)?.checked_add(new.checked_mul(release_secs as u128)?)? / old.checked_add(new)?;
        self.queued = self.queued.checked_add(amount)?;
        self.release_end = now.checked_add(i64::try_from(window).ok()?)?;
        Some(())
    }

    /// What `staked` coins have earned since the snapshot `since`, rounded down.
    ///
    /// Cannot overflow: over any stretch in which a stake is unchanged it is
    /// part of `total_staked`, so `staked * growth <= released * ACC_SCALE`,
    /// and `released` is a u64.
    pub fn earned(&self, staked: u64, since: u128) -> Option<u64> {
        let growth = self.acc.checked_sub(since)?;
        let v = (staked as u128).checked_mul(growth)? / ACC_SCALE;
        u64::try_from(v).ok()
    }
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
    let cap = (quote_reserve as u128).checked_mul(fee_bps as u128)? / (BPS as u128).checked_mul(divisor as u128)?;
    Some(pot.min(u64::try_from(cap).ok()?))
}

/// Fewest coins a buy back accepts: the fee-free figure, less the coin's own
/// fee and an allowance for pump.fun's.
pub fn burn_min_out(reserves: &Reserves, spend: u64, fee_bps: u16, allowance_bps: u16) -> Option<u64> {
    let ideal = reserves.ideal_out(spend)? as u128;
    let keep = (BPS as u128).checked_sub(fee_bps as u128)?.checked_sub(allowance_bps as u128)?;
    u64::try_from(ideal.checked_mul(keep)? / BPS as u128).ok()
}
