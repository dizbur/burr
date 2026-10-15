//! The seventh hook: part of a coin's fee added to its own pool for good.
//! After graduation a call buys the coin with half of what it may use,
//! deposits both sides at the pool's own figures and burns the receipts.

use crate::bench::{custom_code, logs};
use crate::kit::{self, err, LaunchPlan, World};
use crate::pump::{self, ata, TOKEN, TOKEN_2022, ZEC};
use burrpad::error::BurrpadError as E;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

const ZEC1: u64 = 100_000_000;

/// Creator, give, burn, holders 2,000 each, and 2,000 to the pool.
fn liquidity_plan() -> LaunchPlan {
    LaunchPlan { shares: [2_000, 2_000, 2_000, 2_000], liquidity_share: 2_000, ..Default::default() }
}

/// A graduated coin that chose the hook, with `fee` units split, its first
/// (recording) call made, and the wait over: ready to add.
fn setup(fee: u64) -> (World, Pubkey) {
    let mut w = World::new();
    let mint = w.launch(&liquidity_plan());
    w.graduate(&mint);
    w.b.give_zec(&kit::authority(&mint), fee);
    assert!(w.split(&mint).is_ok());
    w.b.wait(60);
    let pot = w.ledger(&mint).liquidity.pot;
    let r = w.add_liquidity(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.ledger(&mint).liquidity.pot, pot, "the first call only records");
    assert!(w.ledger(&mint).liquidity.reference.quote > 0);
    w.b.wait(60);
    (w, mint)
}

fn lp_mint_supply(w: &World, mint: &Pubkey) -> u64 {
    w.b.mint_supply(&pump::lp_mint(mint))
}

/// The most the buy half may spend: a quarter of the fee rate of the smaller reserve.
fn buy_limit(w: &World, mint: &Pubkey) -> u128 {
    let (q, _) = w.pool_reserves(mint);
    let recorded = w.ledger(mint).liquidity.reference.quote;
    let fee = (w.pool(mint).creator_fee_bps as u128).min(w.coin(mint).fee_bps as u128);
    q.min(recorded) as u128 * fee / 40_000
}

#[test]
fn a_launch_can_give_part_of_the_fee_to_its_own_pool() {
    let mut w = World::new();
    let mint = w.launch(&liquidity_plan());
    assert_eq!(w.coin(&mint).liquidity_share, 2_000);
    w.b.give_zec(&kit::authority(&mint), 10 * ZEC1);
    assert!(w.split(&mint).is_ok());
    let l = w.ledger(&mint);
    assert_eq!(l.pots[0], 5 * ZEC1);
    assert_eq!(l.pots[1..5], [ZEC1; 4]);
    assert_eq!((l.liquidity.pot, l.liquidity.credited), (ZEC1, ZEC1 as u128));
    assert_eq!(l.burr.pot, 0);
    w.assert_sound(&mint);
    // All six shares must add to the whole.
    for (burr, liquidity) in [(0u16, 1_999u16), (1, 2_000), (2_000, 2_000)] {
        let (_, r) = w.try_launch(&LaunchPlan {
            shares: [2_000; 4],
            burr_share: burr,
            liquidity_share: liquidity,
            ..Default::default()
        });
        assert_eq!(custom_code(&r), Some(err(E::SharesDoNotAddUp)), "{burr} + {liquidity}");
    }
    let (m, r) = w.try_launch(&LaunchPlan {
        shares: [1_500; 4],
        burr_share: 2_000,
        liquidity_share: 2_000,
        ..Default::default()
    });
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!((w.coin(&m).burr_share, w.coin(&m).liquidity_share), (2_000, 2_000));
}

#[test]
fn coins_without_the_share_and_coins_on_the_curve_add_nothing() {
    let mut w = World::new();
    let old = w.launch(&LaunchPlan::default());
    w.graduate(&old);
    w.b.give_zec(&kit::authority(&old), ZEC1);
    assert!(w.split(&old).is_ok());
    assert_eq!(w.ledger(&old).liquidity.pot, 0);
    w.b.wait(60);
    assert_eq!(custom_code(&w.add_liquidity(&old)), Some(err(E::HookNotChosen)));
    // On the curve the pot saves up; there is no pool to add to.
    let young = w.launch(&liquidity_plan());
    w.b.give_zec(&kit::authority(&young), ZEC1);
    assert!(w.split(&young).is_ok());
    w.b.wait(60);
    assert!(w.add_liquidity(&young).is_err());
    assert_eq!(w.ledger(&young).liquidity.pot, ZEC1 / 10);
}

#[test]
fn an_add_buys_deposits_both_sides_and_burns_every_receipt() {
    let (mut w, mint) = setup(20 * ZEC1);
    let before = w.ledger(&mint);
    let vault = w.vault_balance(&mint);
    let lp_supply = lp_mint_supply(&w, &mint);
    let (q0, b0) = w.pool_reserves(&mint);
    let limit = buy_limit(&w, &mint);
    let r = w.add_liquidity(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    let l = w.ledger(&mint);
    let spent = before.liquidity.pot - l.liquidity.pot;
    assert!(spent > 0, "it added");
    assert!(spent as u128 <= 2 * limit, "spent {spent}, above twice the buy's limit {limit}");
    assert_eq!(l.liquidity.total_spent, spent);
    assert!(l.liquidity.total_lp_burned > 0, "receipts were burned");
    assert_eq!(lp_mint_supply(&w, &mint), lp_supply, "every receipt made was burned");
    let bn = kit::burner(&mint);
    for (what, a) in [
        ("ZEC", ata(&bn, &ZEC, &TOKEN)),
        ("coins", ata(&bn, &mint, &TOKEN_2022)),
        ("receipts", ata(&bn, &pump::lp_mint(&mint), &TOKEN_2022)),
    ] {
        assert_eq!(w.b.token_balance(&a), 0, "the burner keeps no {what}");
    }
    assert_eq!(l.pots, before.pots, "the other pots are untouched");
    assert_eq!(w.vault_balance(&mint), vault - spent);
    // The pool is deeper by what was spent, and its price moved by no more than the buy's own push.
    let (q1, b1) = w.pool_reserves(&mint);
    assert!(q1 > q0, "more ZEC in the pool");
    assert!(b1 + 1_000 >= b0, "its coins all went back in, less dust: {b0} -> {b1}");
    let rise = (q1 as u128 * b0 as u128 * 10_000) / (q0 as u128 * b1 as u128);
    assert!(rise <= 10_000 + 100, "the price moved {rise} / 10,000");
    w.assert_sound(&mint);
}

#[test]
fn outside_the_band_a_call_only_steps_its_record_and_nothing_is_lost() {
    let (mut w, mint) = setup(20 * ZEC1);
    // Push the price 20% up: calls step the recorded price 5% a minute, adding nothing, then add again.
    let t = w.trader(10_000 * ZEC1);
    let (q, _) = w.pool_reserves(&mint);
    assert!(w.pool_buy(&t, &mint, q / 10).is_ok());
    let pot = w.ledger(&mint).liquidity.pot;
    let mut steps = 0;
    loop {
        assert!(w.add_liquidity(&mint).is_ok());
        w.b.wait(60);
        if w.ledger(&mint).liquidity.pot < pot {
            break;
        }
        steps += 1;
        assert!(steps < 20, "it never added again");
    }
    assert!(steps >= 2, "a 20% push took {steps} steps");
    // And down: sell it all back and more.
    let held = w.coins_of(&t.pubkey(), &mint);
    assert!(w.pool_sell(&t, &mint, held).is_ok());
    let pot = w.ledger(&mint).liquidity.pot;
    assert!(w.add_liquidity(&mint).is_ok());
    assert_eq!(w.ledger(&mint).liquidity.pot, pot, "below the band nothing is added either");
    w.assert_sound(&mint);
}

#[test]
fn the_coins_burn_and_its_liquidity_share_one_wait() {
    let mut w = World::new();
    let mint = w.launch(&LaunchPlan { shares: [2_500, 2_500, 2_500, 0], liquidity_share: 2_500, ..Default::default() });
    w.graduate(&mint);
    w.b.give_zec(&kit::authority(&mint), 20 * ZEC1);
    assert!(w.split(&mint).is_ok());
    w.b.wait(700);
    w.burn_until_it_buys(&mint);
    w.b.wait(60);
    assert!(w.add_liquidity(&mint).is_ok(), "the first call records");
    assert_eq!(
        custom_code(&w.burn_pool(&mint)),
        Some(err(E::BurnTooSoon)),
        "a burn in the minute after a liquidity call"
    );
    w.b.wait(60);
    assert!(w.burn_pool(&mint).is_ok());
    assert_eq!(
        custom_code(&w.add_liquidity(&mint)),
        Some(err(E::BurnTooSoon)),
        "a liquidity call in the minute after a burn"
    );
    w.b.wait(60);
    let pot = w.ledger(&mint).liquidity.pot;
    assert!(w.add_liquidity(&mint).is_ok());
    assert!(w.ledger(&mint).liquidity.pot < pot);
    assert_eq!(custom_code(&w.burn_pool(&mint)), Some(err(E::BurnTooSoon)), "a burn right after an add");
}

#[test]
fn buying_before_an_add_and_selling_after_loses_money() {
    for attack in [ZEC1 / 100, ZEC1 / 10, ZEC1, 5 * ZEC1] {
        let (mut w, mint) = setup(50 * ZEC1);
        // Bring the recorded reserve up to the pool's first, so the add uses its full limit.
        for _ in 0..80 {
            if w.ledger(&mint).liquidity.reference.quote >= w.pool_reserves(&mint).0 {
                break;
            }
            assert!(w.add_liquidity(&mint).is_ok());
            w.b.wait(60);
        }
        let attacker = w.trader(1_000 * ZEC1);
        let start = w.zec_of(&attacker.pubkey());
        assert!(w.pool_buy(&attacker, &mint, attack).is_ok());
        assert!(w.add_liquidity(&mint).is_ok());
        let held = w.coins_of(&attacker.pubkey(), &mint);
        assert!(w.pool_sell(&attacker, &mint, held).is_ok());
        let gain = w.zec_of(&attacker.pubkey()) as i128 - start as i128;
        assert!(gain < 0, "attack {attack}: the attacker gained {gain}");
        w.assert_sound(&mint);
    }
}

#[test]
fn liquidity_added_just_for_one_call_does_not_raise_the_limit() {
    let (mut w, mint) = setup(50 * ZEC1);
    let recorded = w.ledger(&mint).liquidity.reference.quote;
    // A large deposit of someone else's, just before the call.
    let lp = w.trader(100_000 * ZEC1);
    assert!(w.pool_buy(&lp, &mint, 500 * ZEC1).is_ok());
    let mk = pump::create_ata(&lp.pubkey(), &lp.pubkey(), &pump::lp_mint(&mint), &TOKEN_2022);
    assert!(w.b.send_as(Some(&lp), &[mk], &[]).is_ok());
    // The push from that buy is stepped through first.
    for _ in 0..40 {
        let pot = w.ledger(&mint).liquidity.pot;
        assert!(w.add_liquidity(&mint).is_ok());
        w.b.wait(60);
        if w.ledger(&mint).liquidity.pot < pot {
            break;
        }
    }
    let recorded = w.ledger(&mint).liquidity.reference.quote.max(recorded);
    let supply = {
        let d = w.b.account(&pump::pool(&mint)).unwrap().data;
        u64::from_le_bytes(d[203..211].try_into().unwrap())
    };
    let r = w.b.send_as(Some(&lp), &[pump::pool_deposit(&mint, &lp.pubkey(), supply * 3, u64::MAX, u64::MAX)], &[]);
    assert!(r.is_ok(), "{}", logs(&r));
    let fee = (w.pool(&mint).creator_fee_bps as u128).min(w.coin(&mint).fee_bps as u128);
    let pot = w.ledger(&mint).liquidity.pot;
    assert!(w.add_liquidity(&mint).is_ok());
    let spent = (pot - w.ledger(&mint).liquidity.pot) as u128;
    assert!(
        spent <= 2 * (recorded as u128 * fee / 40_000) + 2,
        "spent {spent} against a recorded reserve of {recorded}"
    );
}

#[test]
fn add_liquidity_takes_only_this_coins_pool_and_money() {
    let (mut w, mint) = setup(20 * ZEC1);
    let other = w.launch(&liquidity_plan());
    w.graduate(&other);
    let payer = w.b.payer.pubkey();
    type A = burrpad::accounts::AddLiquidity;
    type Change<'x> = Box<dyn Fn(&mut A) + 'x>;
    let swaps: Vec<(&str, Change)> = vec![
        ("another coin's ledger", Box::new(|x| x.ledger = kit::ledger(&other))),
        ("another coin's vault", Box::new(|x| x.vault = kit::vault(&other))),
        (
            "another coin's pool",
            Box::new(|x| {
                let p = pump::pool(&other);
                x.pool_authority = pump::pool_authority(&other);
                x.pool = p;
                x.pool_base_token_account = ata(&p, &other, &TOKEN_2022);
                x.pool_quote_token_account = ata(&p, &ZEC, &TOKEN);
            }),
        ),
        (
            "another pool's receipt mint",
            Box::new(|x| {
                x.lp_mint = pump::lp_mint(&other);
                x.burner_lp = ata(&kit::burner(&mint), &pump::lp_mint(&other), &TOKEN_2022);
            }),
        ),
        (
            "a stranger's receipt account",
            Box::new(|x| x.burner_lp = ata(&Pubkey::new_unique(), &pump::lp_mint(&mint), &TOKEN_2022)),
        ),
    ];
    let pot = w.ledger(&mint).liquidity.pot;
    for (what, change) in swaps {
        let mut acc = w.add_liquidity_accounts(&payer, &mint);
        change(&mut acc);
        let r = w.b.send(&[kit::raw(acc, burrpad::instruction::AddLiquidity {})], &[]);
        assert!(r.is_err(), "{what} was accepted");
    }
    assert_eq!(w.ledger(&mint).liquidity.pot, pot);
    assert!(w.add_liquidity(&mint).is_ok());
}
