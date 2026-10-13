//! Buy back and burn, on the curve.

use crate::bench::{custom_code, logs};
use crate::kit::{self, err, LaunchPlan, World};
use crate::pump::{self, ata, TOKEN, TOKEN_2022, ZEC};
use burr::error::BurrError as E;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

const ZEC1: u64 = 100_000_000;

fn burn_plan(fee_bps: u16) -> LaunchPlan {
    LaunchPlan { fee_bps, shares: [0, 0, 10_000, 0, 0], ..Default::default() }
}

/// A coin whose whole fee goes to buy back and burn, with `pot` units ready.
fn coin_with_burn_pot(w: &mut World, fee_bps: u16, pot: u64) -> Pubkey {
    let mint = w.launch(&burn_plan(fee_bps));
    // Fee arriving in the vault; the platform takes its eighth, the rest is the burn pot.
    w.b.give_zec(&kit::authority(&mint), pot * 8 / 7 + 8);
    assert!(w.split(&mint).is_ok());
    assert!(w.ledger(&mint).pots[3] >= pot);
    mint
}

#[test]
fn a_burn_buys_the_coin_and_destroys_it() {
    let mut w = World::new();
    let mint = coin_with_burn_pot(&mut w, 200, ZEC1);
    let supply = w.b.mint_supply(&mint);
    let pot = w.ledger(&mint).pots[3];
    let vault = w.vault_balance(&mint);
    let reserve = w.b.curve(&mint).virtual_quote_reserves;

    let r = w.burn_curve(&mint);
    assert!(r.is_ok(), "{}", logs(&r));

    let l = w.ledger(&mint);
    let spent = pot - l.pots[3];
    // The limit: reserve x fee / 4, half a percent of the pool on a 2% coin.
    assert_eq!(spent, reserve * 200 / 40_000);
    assert_eq!(w.vault_balance(&mint), vault - spent);
    assert_eq!(l.burn.total_spent, spent);
    let burned = supply - w.b.mint_supply(&mint);
    assert!(burned > 0);
    assert_eq!(l.burn.total_burned, burned);
    // The burner keeps nothing.
    let bn = kit::burner(&mint);
    assert_eq!(w.b.token_balance(&ata(&bn, &ZEC, &TOKEN)), 0);
    assert_eq!(w.b.token_balance(&ata(&bn, &mint, &TOKEN_2022)), 0);
    w.assert_sound(&mint);
}

#[test]
fn burns_wait_between_calls() {
    let mut w = World::new();
    let mint = coin_with_burn_pot(&mut w, 200, ZEC1);
    assert!(w.burn_curve(&mint).is_ok());
    let r = w.burn_curve(&mint);
    assert_eq!(custom_code(&r), Some(err(E::BurnTooSoon)));
    w.b.wait(59);
    let r = w.burn_curve(&mint);
    assert_eq!(custom_code(&r), Some(err(E::BurnTooSoon)));
    w.b.wait(1);
    let r = w.burn_curve(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    w.assert_sound(&mint);
}

#[test]
fn the_spend_never_passes_the_pot_or_the_limit() {
    let mut w = World::new();
    // A pot smaller than the limit is spent whole and no more.
    let small = coin_with_burn_pot(&mut w, 300, 1_000);
    let pot = w.ledger(&small).pots[3];
    assert!(w.burn_curve(&small).is_ok());
    assert_eq!(w.ledger(&small).pots[3], 0);
    assert_eq!(w.ledger(&small).burn.total_spent, pot);
    // With nothing left there is nothing to do.
    w.b.wait(60);
    let r = w.burn_curve(&small);
    assert_eq!(custom_code(&r), Some(err(E::NothingToBurn)));

    // The limit scales with the coin's fee.
    for fee in [50u16, 100, 300] {
        let mint = coin_with_burn_pot(&mut w, fee, 10 * ZEC1);
        let reserve = w.b.curve(&mint).virtual_quote_reserves;
        let before = w.ledger(&mint).pots[3];
        assert!(w.burn_curve(&mint).is_ok());
        assert_eq!(before - w.ledger(&mint).pots[3], reserve * fee as u64 / 40_000);
    }
}

#[test]
fn a_price_pushed_up_is_not_bought_at() {
    let mut w = World::new();
    let mint = coin_with_burn_pot(&mut w, 200, 10 * ZEC1);
    assert!(w.burn_curve(&mint).is_ok());
    let after_first = w.ledger(&mint);

    // Someone pushes the price up well over 5%, then the burn is called.
    let pusher = w.trader(100 * ZEC1);
    assert!(w.buy(&pusher, &mint, ZEC1).is_ok());
    w.b.wait(60);
    let supply = w.b.mint_supply(&mint);
    let r = w.burn_curve(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    let l = w.ledger(&mint);
    assert_eq!(l.pots[3], after_first.pots[3], "nothing was spent");
    assert_eq!(w.b.mint_supply(&mint), supply, "nothing was burned");
    assert!(l.burn.last_time > after_first.burn.last_time, "the new price and time were recorded");

    // It cannot be retried at once.
    let r = w.burn_curve(&mint);
    assert_eq!(custom_code(&r), Some(err(E::BurnTooSoon)));
    // If the price is still there one waiting time later, it is accepted as real.
    w.b.wait(60);
    assert!(w.burn_curve(&mint).is_ok());
    assert!(w.ledger(&mint).pots[3] < after_first.pots[3]);
    w.assert_sound(&mint);
}

#[test]
fn a_price_that_fell_is_bought_at_once() {
    let mut w = World::new();
    let mint = coin_with_burn_pot(&mut w, 200, 10 * ZEC1);
    let t = w.trader(100 * ZEC1);
    assert!(w.buy(&t, &mint, ZEC1).is_ok());
    assert!(w.burn_curve(&mint).is_ok());
    let have = w.coins_of(&t.pubkey(), &mint);
    assert!(w.sell(&t, &mint, have).is_ok());
    w.b.wait(60);
    let before = w.ledger(&mint).pots[3];
    assert!(w.burn_curve(&mint).is_ok());
    assert!(w.ledger(&mint).pots[3] < before);
}

/// The classic attack: buy just before our buy, sell just after.
/// Returns the attacker's ZEC gain (negative is a loss).
fn sandwich(fee_bps: u16, attack_zec: u64) -> i128 {
    let mut w = World::new();
    let mint = coin_with_burn_pot(&mut w, fee_bps, 50 * ZEC1);
    // Some ordinary trading first, so the pool is not at its starting point.
    let other = w.trader(100 * ZEC1);
    assert!(w.buy(&other, &mint, 2 * ZEC1).is_ok());
    assert!(w.burn_curve(&mint).is_ok());
    w.b.wait(60);

    let attacker = w.trader(1_000 * ZEC1);
    let start = w.zec_of(&attacker.pubkey());
    assert!(w.buy(&attacker, &mint, attack_zec).is_ok());
    let r = w.burn_curve(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    let coins = w.coins_of(&attacker.pubkey(), &mint);
    assert!(w.sell(&attacker, &mint, coins).is_ok());
    w.zec_of(&attacker.pubkey()) as i128 - start as i128
}

#[test]
fn buying_before_and_selling_after_a_burn_loses_money() {
    // Every allowed fee, small and large attacks. Sizes above about 5% of the
    // pool trip the price check instead, which the next test covers.
    for fee in [50u16, 100, 200, 300] {
        for attack in [ZEC1 / 1_000, ZEC1 / 100, ZEC1 / 20, ZEC1 / 10] {
            let gain = sandwich(fee, attack);
            assert!(gain < 0, "fee {fee} attack {attack}: attacker gained {gain}");
            // The loss is at least the coin's fee paid twice, less what our buy moved the price.
            let floor = attack as i128 * (fee as i128) / 10_000;
            assert!(-gain >= floor, "fee {fee} attack {attack}: lost only {}", -gain);
        }
    }
}

#[test]
fn a_large_push_before_a_burn_just_makes_it_skip() {
    let mut w = World::new();
    let mint = coin_with_burn_pot(&mut w, 200, 50 * ZEC1);
    assert!(w.burn_curve(&mint).is_ok());
    w.b.wait(60);
    let attacker = w.trader(1_000 * ZEC1);
    let start = w.zec_of(&attacker.pubkey());
    assert!(w.buy(&attacker, &mint, 5 * ZEC1).is_ok());
    let pot = w.ledger(&mint).pots[3];
    assert!(w.burn_curve(&mint).is_ok());
    assert_eq!(w.ledger(&mint).pots[3], pot, "the burn did not buy");
    let coins = w.coins_of(&attacker.pubkey(), &mint);
    assert!(w.sell(&attacker, &mint, coins).is_ok());
    assert!(w.zec_of(&attacker.pubkey()) < start, "the attacker only paid fees");
}

#[test]
fn a_coin_without_the_hook_cannot_burn() {
    let mut w = World::new();
    let mint = w.launch(&LaunchPlan { shares: [10_000, 0, 0, 0, 0], ..Default::default() });
    w.b.give_zec(&kit::authority(&mint), ZEC1);
    assert!(w.split(&mint).is_ok());
    let r = w.burn_curve(&mint);
    assert_eq!(custom_code(&r), Some(err(E::HookNotChosen)));
}

#[test]
fn burn_refuses_substituted_accounts() {
    let mut w = World::new();
    let mint = coin_with_burn_pot(&mut w, 200, ZEC1);
    let other = coin_with_burn_pot(&mut w, 200, ZEC1);
    let payer = w.b.payer.pubkey();
    let thief = w.trader(ZEC1);
    let thief_zec = ata(&thief.pubkey(), &ZEC, &TOKEN);
    let fake = Pubkey::new_unique();
    type Swap = fn(&mut burr::accounts::BurnCurve, Pubkey, &Pubkey, Pubkey);
    let swaps: Vec<(&str, Swap)> = vec![
        ("pump_program", |a, f, _, _| a.pump_program = f),
        ("pump_global", |a, f, _, _| a.pump_global = f),
        ("bonding_curve (another coin's)", |a, _, o, _| a.bonding_curve = pump::curve(o)),
        ("associated_base_bonding_curve", |a, f, _, _| a.associated_base_bonding_curve = f),
        ("associated_quote_bonding_curve", |a, f, _, _| a.associated_quote_bonding_curve = f),
        ("user_volume", |a, f, _, _| a.user_volume = f),
        ("fee_config", |a, f, _, _| a.fee_config = f),
        ("buyback_fee_recipient (a thief's ZEC account)", |a, _, _, t| a.buyback_fee_recipient = t),
        ("pump_event_authority", |a, f, _, _| a.pump_event_authority = f),
        ("vault (another coin's)", |a, _, o, _| a.vault = kit::vault(o)),
        ("ledger (another coin's)", |a, _, o, _| a.ledger = kit::ledger(o)),
        ("coin (another coin's)", |a, _, o, _| a.coin = kit::coin(o)),
        ("authority (another coin's)", |a, _, o, _| a.authority = kit::authority(o)),
        ("burner (another coin's)", |a, _, o, _| a.burner = kit::burner(o)),
        ("burner_zec (a thief's)", |a, _, _, t| a.burner_zec = t),
        ("burner_coin", |a, f, _, _| a.burner_coin = f),
        ("zec_mint", |a, _, o, _| a.zec_mint = *o),
        ("token_program", |a, _, _, _| a.token_program = TOKEN_2022),
        ("token_2022_program", |a, _, _, _| a.token_2022_program = TOKEN),
    ];
    for (name, swap) in swaps {
        let mut acc = w.burn_curve_accounts(&payer, &mint);
        swap(&mut acc, fake, &other, thief_zec);
        let r = w.b.send(&[kit::raw(acc, burr::instruction::BurnCurve {})], &[]);
        assert!(r.is_err(), "swapping {name} was accepted");
    }
    assert_eq!(w.b.token_balance(&thief_zec), ZEC1);
    assert!(w.burn_curve(&mint).is_ok());
}

#[test]
fn gifts_to_the_burner_do_not_confuse_it() {
    let mut w = World::new();
    let mint = coin_with_burn_pot(&mut w, 200, ZEC1);
    let bn = kit::burner(&mint);
    // Someone sends ZEC and coins to the burner's accounts before the first burn.
    let t = w.trader(10 * ZEC1);
    assert!(w.buy(&t, &mint, ZEC1 / 100).is_ok());
    let payer = w.b.payer.pubkey();
    let mk = [pump::create_ata(&payer, &bn, &ZEC, &TOKEN), pump::create_ata(&payer, &bn, &mint, &TOKEN_2022)];
    assert!(w.b.send(&mk, &[]).is_ok());
    w.b.give_zec(&bn, 5_000);
    let gift = w.coins_of(&t.pubkey(), &mint);
    w.b.write_token_account(ata(&bn, &mint, &TOKEN_2022), &mint, &bn, gift, &TOKEN_2022);

    let pot = w.ledger(&mint).pots[3];
    let vault = w.vault_balance(&mint);
    let r = w.burn_curve(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    let l = w.ledger(&mint);
    let spent = pot - l.pots[3];
    // The gift of ZEC ends up in the vault, as fee still to be split.
    assert_eq!(w.vault_balance(&mint), vault - spent + 5_000);
    assert_eq!(w.b.token_balance(&ata(&bn, &ZEC, &TOKEN)), 0);
    assert_eq!(w.b.token_balance(&ata(&bn, &mint, &TOKEN_2022)), 0);
    assert!(w.split(&mint).is_ok());
    w.assert_sound(&mint);
}
