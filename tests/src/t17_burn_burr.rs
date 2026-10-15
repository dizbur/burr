//! The sixth hook: part of a coin's fee buys the platform's own coin, $BURR,
//! on its pool and burns it. Same rules as a coin's own buy back on a pool,
//! with one waiting time and one recorded reserve shared by all coins.

use crate::bench::{custom_code, logs};
use crate::kit::{self, err, LaunchPlan, World};
use crate::pump::{ata, TOKEN, TOKEN_2022, ZEC};
use burrpad::error::BurrpadError as E;
use burrpad::BURR_MINT;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

const ZEC1: u64 = 100_000_000;

/// Creator, give, burn, holders 2,000 each, and 2,000 to burning $BURR.
fn burr_plan() -> LaunchPlan {
    LaunchPlan { shares: [2_000, 2_000, 2_000, 2_000], burr_share: 2_000, ..Default::default() }
}

/// A world with $BURR in place and one coin that chose the hook, with `fee` units split.
fn setup(fee: u64) -> (World, Pubkey) {
    let mut w = World::new();
    w.make_burr();
    let mint = w.launch(&burr_plan());
    w.b.give_zec(&kit::authority(&mint), fee);
    assert!(w.split(&mint).is_ok());
    w.b.wait(60);
    (w, mint)
}

/// The most one $BURR buy may spend: a quarter of the pool's fee rate of the
/// smaller of its reserve now and the recorded one (1 ZEC before any buy).
fn burr_limit(w: &World) -> u128 {
    let (q, _) = w.pool_reserves(&BURR_MINT);
    let recorded = match w.config().burr_buy.reference.quote {
        0 => burrpad::BURR_START_RESERVE,
        r => r,
    };
    q.min(recorded) as u128 * w.pool(&BURR_MINT).creator_fee_bps as u128 / 40_000
}

#[test]
fn a_launch_can_give_part_of_the_fee_to_burning_burr() {
    let (w, mint) = setup(10 * ZEC1);
    assert_eq!(w.coin(&mint).burr_share, 2_000);
    let l = w.ledger(&mint);
    // Platform half, then a tenth of the fee to each of the five hooks.
    assert_eq!(l.pots[0], 5 * ZEC1);
    assert_eq!(l.pots[1..5], [ZEC1; 4]);
    assert_eq!((l.burr.pot, l.burr.credited), (ZEC1, ZEC1 as u128));
    w.assert_sound(&mint);
}

#[test]
fn shares_with_burr_must_add_to_the_whole() {
    let mut w = World::new();
    for (shares, burr) in [([2_000u16, 2_000, 2_000, 2_000], 1_999u16), ([2_500, 2_500, 2_500, 2_500], 1)] {
        let (_, r) = w.try_launch(&LaunchPlan { shares, burr_share: burr, ..Default::default() });
        assert_eq!(custom_code(&r), Some(err(E::SharesDoNotAddUp)), "{shares:?} + {burr}");
    }
}

#[test]
fn a_coin_launched_the_old_way_has_no_burr_share() {
    let mut w = World::new();
    w.make_burr();
    let mint = w.launch(&LaunchPlan::default());
    assert_eq!(w.coin(&mint).burr_share, 0);
    w.b.give_zec(&kit::authority(&mint), ZEC1);
    assert!(w.split(&mint).is_ok());
    assert_eq!(w.ledger(&mint).burr.pot, 0, "nothing went to a pot it did not choose");
    w.b.wait(60);
    let r = w.burn_burr(&mint);
    assert_eq!(custom_code(&r), Some(err(E::HookNotChosen)), "{}", logs(&r));
}

#[test]
fn burn_burr_buys_burr_and_destroys_it_within_the_limit() {
    let (mut w, mint) = setup(10 * ZEC1);
    let before = w.ledger(&mint);
    let supply = w.b.mint_supply(&BURR_MINT);
    let vault = w.vault_balance(&mint);
    let limit = burr_limit(&w);
    let r = w.burn_burr(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    let l = w.ledger(&mint);
    let spent = before.burr.pot - l.burr.pot;
    assert!(spent > 0, "it bought");
    assert!(spent as u128 <= limit, "spent {spent}, above the limit {limit}");
    assert_eq!(l.burr.total_spent, spent);
    let burned = supply - w.b.mint_supply(&BURR_MINT);
    assert!(burned > 0, "and destroyed what it bought");
    assert_eq!(l.burr.total_burned, burned);
    assert_eq!(l.pots, before.pots, "the other pots are untouched");
    assert_eq!(w.vault_balance(&mint), vault - spent);
    let bn = kit::burner(&mint);
    assert_eq!(w.b.token_balance(&ata(&bn, &ZEC, &TOKEN)), 0, "the burner keeps no ZEC");
    assert_eq!(w.b.token_balance(&ata(&bn, &BURR_MINT, &TOKEN_2022)), 0, "nor any $BURR");
    w.assert_sound(&mint);
}

#[test]
fn one_burr_buy_a_minute_across_all_coins() {
    // Several coins' buys cannot be packed into one moment.
    let (mut w, a) = setup(10 * ZEC1);
    let b = w.launch(&burr_plan());
    w.b.give_zec(&kit::authority(&b), 10 * ZEC1);
    assert!(w.split(&b).is_ok());
    w.b.wait(60);
    assert!(w.burn_burr(&a).is_ok());
    let r = w.burn_burr(&b);
    assert_eq!(custom_code(&r), Some(err(E::BurnTooSoon)), "a second coin in the same minute");
    w.b.wait(59);
    let r = w.burn_burr(&b);
    assert_eq!(custom_code(&r), Some(err(E::BurnTooSoon)), "a second early");
    w.b.wait(1);
    let r = w.burn_burr(&b);
    assert!(r.is_ok(), "{}", logs(&r));
}

#[test]
fn the_recorded_reserve_starts_at_one_zec_and_grows_a_step_per_buy() {
    let (mut w, mint) = setup(50 * ZEC1);
    assert_eq!(w.config().burr_buy.reference.quote, 0);
    // Someone adds a lot to the pool's price first: it cannot raise the first limit above 1 ZEC's worth.
    let pusher = w.trader(1_000 * ZEC1);
    assert!(w.pool_buy(&pusher, &BURR_MINT, 5 * ZEC1).is_ok());
    let fee = w.pool(&BURR_MINT).creator_fee_bps as u128;
    let mut recorded = burrpad::BURR_START_RESERVE as u128;
    for buy in 0..5 {
        let pot = w.ledger(&mint).burr.pot;
        assert!(w.burn_burr(&mint).is_ok());
        let spent = pot - w.ledger(&mint).burr.pot;
        assert!(spent as u128 <= recorded * fee / 40_000, "buy {buy}: spent {spent}");
        let now = w.config().burr_buy.reference.quote as u128;
        assert!(now <= recorded * 10_500 / 10_000 + 1, "buy {buy}: the recorded reserve grew more than a step");
        recorded = now;
        w.b.wait(60);
    }
}

#[test]
fn buying_burr_before_a_burr_burn_and_selling_after_loses_money() {
    for attack in [ZEC1 / 100, ZEC1 / 10, ZEC1, 5 * ZEC1] {
        let (mut w, mint) = setup(50 * ZEC1);
        // Bring the recorded reserve up to the pool's first, so the burn spends its full limit.
        for _ in 0..80 {
            if w.config().burr_buy.reference.quote >= w.pool_reserves(&BURR_MINT).0 {
                break;
            }
            assert!(w.burn_burr(&mint).is_ok());
            w.b.wait(60);
        }
        let attacker = w.trader(1_000 * ZEC1);
        let start = w.zec_of(&attacker.pubkey());
        assert!(w.pool_buy(&attacker, &BURR_MINT, attack).is_ok());
        let pot = w.ledger(&mint).burr.pot;
        assert!(w.burn_burr(&mint).is_ok());
        assert!(w.ledger(&mint).burr.pot < pot, "the burn bought");
        let held = w.coins_of(&attacker.pubkey(), &BURR_MINT);
        assert!(w.pool_sell(&attacker, &BURR_MINT, held).is_ok());
        let gain = w.zec_of(&attacker.pubkey()) as i128 - start as i128;
        assert!(gain < 0, "attack {attack}: the attacker gained {gain}");
    }
}

#[test]
fn burn_burr_takes_only_burrs_own_pool_and_this_coins_money() {
    let (mut w, mint) = setup(10 * ZEC1);
    let other = w.launch(&burr_plan());
    w.b.give_zec(&kit::authority(&other), 10 * ZEC1);
    assert!(w.split(&other).is_ok());
    // The coin $BURR was copied from still has its own pool: it is not $BURR's.
    let copied_from = w.make_burr();
    let payer = w.b.payer.pubkey();
    type Change<'x> = Box<dyn Fn(&mut burrpad::accounts::BurnBurr) + 'x>;
    let swaps: Vec<(&str, Change)> = vec![
        ("another coin's ledger", Box::new(|x| x.ledger = kit::ledger(&other))),
        ("another coin's vault", Box::new(|x| x.vault = kit::vault(&other))),
        (
            "another coin's burner",
            Box::new(|x| {
                x.burner = kit::burner(&other);
                x.burner_zec = ata(&kit::burner(&other), &ZEC, &TOKEN);
                x.burner_burr = ata(&kit::burner(&other), &BURR_MINT, &TOKEN_2022);
            }),
        ),
        (
            "another coin as $BURR",
            Box::new(move |x| {
                x.burr_mint = copied_from;
                x.burner_burr = ata(&kit::burner(&mint), &copied_from, &TOKEN_2022);
            }),
        ),
        (
            "another coin's pool",
            Box::new(move |x| {
                let p = crate::pump::pool(&copied_from);
                x.pool_authority = crate::pump::pool_authority(&copied_from);
                x.pool = p;
                x.pool_base_token_account = ata(&p, &copied_from, &TOKEN_2022);
                x.pool_quote_token_account = ata(&p, &ZEC, &TOKEN);
            }),
        ),
        ("a stranger's ZEC account as the vault", Box::new(|x| x.vault = ata(&Pubkey::new_unique(), &ZEC, &TOKEN))),
    ];
    let before = (w.ledger(&mint), w.ledger(&other), w.vault_balance(&mint), w.vault_balance(&other));
    for (what, change) in swaps {
        let mut acc = w.burn_burr_accounts(&payer, &mint);
        change(&mut acc);
        let r = w.b.send(&[kit::raw(acc, burrpad::instruction::BurnBurr {})], &[]);
        assert!(r.is_err(), "{what} was accepted");
    }
    assert_eq!((w.vault_balance(&mint), w.vault_balance(&other)), (before.2, before.3));
    assert_eq!((w.ledger(&mint).burr.pot, w.ledger(&other).burr.pot), (before.0.burr.pot, before.1.burr.pot));
    assert!(w.burn_burr(&mint).is_ok(), "with the right accounts it buys");
}
