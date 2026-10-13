//! The arithmetic on its own, with fixed and random inputs, and the constants
//! that must match pump.fun's interface.

use burrpad::constants::*;
use burrpad::math::{self, Reserves, WEIGHT_TOTAL};
use proptest::prelude::*;

// ---- fixed cases ----

#[test]
fn weights_always_add_to_the_whole() {
    let w = math::weights(1_250, &[2_500, 2_500, 2_500, 2_500]).unwrap();
    assert_eq!(w.iter().sum::<u128>(), WEIGHT_TOTAL);
    assert_eq!(w[0], 12_500_000);
    assert_eq!(w[1], 21_875_000);
    assert!(math::weights(1_250, &[2_500, 2_500, 2_500, 2_499]).is_none());
    assert!(math::weights(10_001, &[10_000, 0, 0, 0]).is_none());
    assert!(math::weights(0, &[u16::MAX; 4]).is_none());
}

#[test]
fn the_examples_in_the_brief_come_out_right() {
    // One eighth of the fee: on a 2.00% coin, 0.25% to the platform and 1.75% to the hooks.
    let w = math::weights(1_250, &[10_000, 0, 0, 0]).unwrap();
    let (mut total, mut credited) = (0u128, [0u128; SINKS]);
    // 2.00% of a 1,000,000-unit trade is 20,000 units of fee.
    let c = math::split(&mut total, &mut credited, &w, 20_000).unwrap();
    assert_eq!(c[SINK_PLATFORM], 2_500);
    assert_eq!(c[SINK_CREATOR], 17_500);
}

#[test]
fn the_burn_limit_and_floor() {
    assert_eq!(math::burn_spend_limit(u64::MAX, 1_000_000, 200, 4), Some(5_000));
    assert_eq!(math::burn_spend_limit(100, 1_000_000, 200, 4), Some(100));
    assert_eq!(math::burn_spend_limit(u64::MAX, u64::MAX, 500, 4), Some((u64::MAX as u128 * 500 / 40_000) as u64));
    let r = Reserves { quote: 1_000_000, base: 2_000_000 };
    assert_eq!(r.ideal_out(1_000), Some(1_998));
    // 2% for the coin, 2% allowed for pump.fun: at least 96% of the fee-free figure.
    assert_eq!(math::burn_min_out(&r, 1_000, 200, 200), Some(1_998 * 9_600 / 10_000));
}

#[test]
fn the_price_band() {
    let reference = Reserves { quote: 100, base: 1_000 };
    assert_eq!(Reserves { quote: 105, base: 1_000 }.above(&reference, 500), Some(false), "exactly 5% is allowed");
    assert_eq!(Reserves { quote: 106, base: 1_000 }.above(&reference, 500), Some(true));
    assert_eq!(Reserves { quote: 100, base: 951 }.above(&reference, 500), Some(true));
    assert_eq!(Reserves { quote: 50, base: 1_000 }.above(&reference, 500), Some(false), "a lower price is fine");
    // The largest figures do not overflow.
    let big = Reserves { quote: u64::MAX, base: u64::MAX };
    assert!(big.above(&big, 500).is_none() || big.above(&big, 500) == Some(false));
}

// ---- constants against pump.fun's interface ----

#[test]
fn instruction_codes_match_their_names() {
    use crate::pump::disc;
    use burrpad::pump as p;
    assert_eq!(p::IX_CREATE_V2, disc("create_v2"));
    assert_eq!(p::IX_SWEEP_CREATOR_FEE, disc("sweep_creator_fee"));
    assert_eq!(p::IX_COLLECT_CREATOR_FEE_V2, disc("collect_creator_fee_v2"));
    assert_eq!(p::IX_BUY_EXACT_QUOTE_IN_V3, disc("buy_exact_quote_in_v3"));
    assert_eq!(p::IX_INIT_USER_VOLUME, disc("init_user_volume_accumulator"));
    assert_eq!(p::IX_AMM_COLLECT_COIN_CREATOR_FEE, disc("collect_coin_creator_fee"));
    assert_eq!(p::IX_AMM_BUY_EXACT_QUOTE_IN_V2, disc("buy_exact_quote_in_v2"));
}

#[test]
fn account_codes_and_positions_match_the_real_accounts() {
    use burrpad::pump as p;
    let b = crate::bench::Bench::new();
    let global = b.account(&crate::pump::global()).unwrap();
    assert_eq!(global.data[..8], p::ACC_GLOBAL);
    let amm = b.account(&crate::pump::amm_global_config()).unwrap();
    assert_eq!(amm.data[..8], p::ACC_AMM_GLOBAL_CONFIG);
    // The eight buy-back receivers sit where we read them, in both accounts.
    let listed = |d: &[u8], o: usize| -> Vec<solana_pubkey::Pubkey> {
        (0..8)
            .map(|i| solana_pubkey::Pubkey::new_from_array(d[o + 32 * i..o + 32 * i + 32].try_into().unwrap()))
            .collect()
    };
    let known: Vec<solana_pubkey::Pubkey> = [
        "5YxQFdt3Tr9zJLvkFccqXVUwhdTWJQc1fFg2YPbxvxeD",
        "9M4giFFMxmFGXtc3feFzRai56WbBqehoSeRE5GK7gf7",
        "GXPFM2caqTtQYC2cJ5yJRi9VDkpsYZXzYdwYpGnLmtDL",
        "3BpXnfJaUTiwXnJNe7Ej1rcbzqTTQUvLShZaWazebsVR",
        "5cjcW9wExnJJiqgLjq7DEG75Pm6JBgE1hNv4B2vHXUW6",
        "EHAAiTxcdDwQ3U4bU6YcMsQGaekdzLS3B5SmYo46kJtL",
        "5eHhjP8JaYkz83CWwvGU2uMUXefd3AazWGx4gpcuEEYD",
        "A7hAgCzFw14fejgCp387JUJRMNyz4j89JKnhtKU8piqW",
    ]
    .iter()
    .map(|s| s.parse().unwrap())
    .collect();
    assert_eq!(listed(&global.data, p::GLOBAL_BUYBACK_OFFSET), known);
    assert_eq!(listed(&amm.data, p::AMM_GLOBAL_BUYBACK_OFFSET), known);
    assert_eq!(burrpad::constants::ZEC_MINT, crate::pump::ZEC);
    assert_eq!(p::PUMP_ID, crate::pump::PUMP);
    assert_eq!(p::PUMP_AMM_ID, crate::pump::PUMP_AMM);
    assert_eq!(p::PUMP_FEES_ID, crate::pump::PUMP_FEES);
}

// ---- random inputs ----

fn shares() -> impl Strategy<Value = [u16; 4]> {
    // Four shares that add to 10,000, some possibly zero.
    proptest::collection::vec(0u32..=10_000, 3).prop_map(|mut cuts| {
        cuts.sort();
        let p = [0, cuts[0], cuts[1], cuts[2], 10_000];
        [(p[1] - p[0]) as u16, (p[2] - p[1]) as u16, (p[3] - p[2]) as u16, (p[4] - p[3]) as u16]
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2_000, ..ProptestConfig::default() })]

    /// However the same money is cut into calls, every pot ends the same,
    /// nothing is invented, and under five units are ever left over.
    #[test]
    fn the_split_does_not_depend_on_how_it_is_called(
        fraction in 0u16..=5_000,
        sh in shares(),
        amounts in proptest::collection::vec(0u64..=5_000_000_000_000, 1..40),
    ) {
        let w = math::weights(fraction, &sh).unwrap();
        prop_assert_eq!(w.iter().sum::<u128>(), WEIGHT_TOTAL);

        let (mut total, mut credited, mut pots) = (0u128, [0u128; SINKS], [0u128; SINKS]);
        let mut money_in = 0u128;
        for a in &amounts {
            let c = math::split(&mut total, &mut credited, &w, *a).unwrap();
            let call_sum: u128 = c.iter().map(|x| *x as u128).sum();
            money_in += *a as u128;
            for i in 0..SINKS { pots[i] += c[i] as u128; }
            let all: u128 = pots.iter().sum();
            // Never more out than in, at any point; the leftover stays tiny.
            prop_assert!(all <= money_in);
            prop_assert!(money_in - all < SINKS as u128);
            prop_assert!(call_sum <= *a as u128 + SINKS as u128);
        }
        // The same total in one call.
        let (mut t1, mut c1) = (0u128, [0u128; SINKS]);
        let mut left = money_in;
        let mut once = [0u128; SINKS];
        while left > 0 {
            let chunk = left.min(u64::MAX as u128) as u64;
            let c = math::split(&mut t1, &mut c1, &w, chunk).unwrap();
            for i in 0..SINKS { once[i] += c[i] as u128; }
            left -= chunk as u128;
        }
        prop_assert_eq!(pots, once);
        // Each pot is its exact proportion, rounded down.
        for i in 0..SINKS {
            prop_assert_eq!(pots[i], money_in * w[i] / WEIGHT_TOTAL);
        }
        // A hook with no share gets nothing.
        for i in 0..4 {
            if sh[i] == 0 { prop_assert_eq!(pots[i + 1], 0); }
        }
    }

    /// The platform never gets more than its fraction, whatever the hooks are.
    #[test]
    fn the_platforms_part_is_exactly_its_fraction(fraction in 0u16..=5_000, sh in shares(), amount in 0u64..=u64::MAX / 2) {
        let w = math::weights(fraction, &sh).unwrap();
        let (mut t, mut c) = (0u128, [0u128; SINKS]);
        let out = math::split(&mut t, &mut c, &w, amount).unwrap();
        prop_assert_eq!(out[SINK_PLATFORM] as u128, amount as u128 * fraction as u128 / 10_000);
    }

    /// A burn never spends more than the pot or the limit, and never accepts
    /// more than a fee-free pool could give.
    #[test]
    fn burn_figures_stay_inside_their_bounds(
        pot in 0u64..=u64::MAX, quote in 1u64..=u64::MAX / 2, base in 1u64..=1_000_000_000_000_000, fee in 10u16..=500,
    ) {
        let spend = math::burn_spend_limit(pot, quote, fee, BURN_SPEND_DIVISOR).unwrap();
        prop_assert!(spend <= pot);
        prop_assert!(spend as u128 <= quote as u128 * fee as u128 / 40_000);
        let r = Reserves { quote, base };
        let ideal = r.ideal_out(spend).unwrap();
        prop_assert!(ideal <= base);
        let min = math::burn_min_out(&r, spend, fee, BURN_FEE_ALLOWANCE_BPS).unwrap();
        prop_assert!(min <= ideal);
        // Our own buy moves the price by no more than half the coin's fee.
        // price after / price before = ((quote + spend) / quote)^2 at most
        let lhs = (quote as u128 + spend as u128) * 40_000;
        prop_assert!(lhs <= quote as u128 * (40_000 + fee as u128));
    }
}
