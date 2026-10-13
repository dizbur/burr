//! After graduation: the coin has moved to its PumpSwap pool. Collection and
//! every hook must work the same.

use crate::bench::logs;
use crate::kit::{LaunchPlan, World};
use solana_signer::Signer;

const ZEC1: u64 = 100_000_000;

#[test]
fn a_coin_graduates_and_keeps_its_creator_and_fee() {
    let mut w = World::new();
    let mint = w.launch(&LaunchPlan::default());
    w.graduate(&mint);
    let p = w.pool(&mint);
    assert_eq!(p.base_mint, mint);
    assert_eq!(p.coin_creator, crate::kit::authority(&mint), "the fee still belongs to our authority");
    assert_eq!(p.creator_fee_bps, 200);
    let t = w.trader(100 * ZEC1);
    let r = w.pool_buy(&t, &mint, ZEC1);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(w.coins_of(&t.pubkey(), &mint) > 0);
    assert!(w.pool(&mint).creator_fees > 0, "trades on the pool leave a fee waiting for us");
}

use crate::bench::custom_code;
use crate::kit::{self, err};
use crate::pump::{self, ata, TOKEN, TOKEN_2022, ZEC};
use burrpad::error::BurrpadError as E;
use burrpad::state::Payee;
use solana_pubkey::Pubkey;

const DAY: i64 = 86_400;

/// A graduated coin with all five hooks, and some trading on its pool.
fn graduated(plan: &LaunchPlan) -> (World, Pubkey) {
    let mut w = World::new();
    let mint = w.launch(plan);
    w.graduate(&mint);
    (w, mint)
}

#[test]
fn fees_from_both_the_curve_and_the_pool_reach_the_vault() {
    let (mut w, mint) = graduated(&LaunchPlan::default());
    // The buy that completed the curve left a fee waiting on the curve.
    let on_curve = w.b.curve(&mint).creator_fee;
    assert!(on_curve > 0);
    let t = w.trader(100 * ZEC1);
    assert!(w.pool_buy(&t, &mint, ZEC1).is_ok());
    let have = w.coins_of(&t.pubkey(), &mint);
    assert!(w.pool_sell(&t, &mint, have / 2).is_ok());
    let in_pool = w.pool(&mint).creator_fees;
    assert!(in_pool > 0);

    // The curve side still collects after graduation.
    let r = w.collect_curve(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.vault_balance(&mint), on_curve);
    // And the pool side.
    let r = w.collect_pool(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.vault_balance(&mint), on_curve + in_pool);
    assert_eq!(w.ledger(&mint).accounted, on_curve + in_pool);
    assert_eq!(w.pool(&mint).creator_fees, 0);
    w.assert_sound(&mint);

    // Collecting again with nothing waiting is harmless.
    assert!(w.collect_pool(&mint).is_ok());
    assert_eq!(w.ledger(&mint).accounted, on_curve + in_pool);

    // The fee rate on the pool is still the coin's own: 2.00% of what was traded.
    // (in_pool is the creator fee of one buy and one sell.)
    assert!(in_pool > ZEC1 * 2 / 100);
}

#[test]
fn collect_pool_before_graduation_is_refused() {
    let mut w = World::new();
    let mint = w.launch(&LaunchPlan::default());
    let r = w.collect_pool(&mint);
    assert_eq!(custom_code(&r), Some(err(E::WrongVenue)), "{}", logs(&r));
}

#[test]
fn payouts_work_the_same_after_graduation() {
    let plan = LaunchPlan::default();
    let (mut w, mint) = graduated(&plan);
    let t = w.trader(100 * ZEC1);
    assert!(w.pool_buy(&t, &mint, ZEC1).is_ok());
    assert!(w.collect_pool(&mint).is_ok());
    let pots = w.ledger(&mint).pots;
    assert!(w.pay_out(&mint, Payee::Creator).is_ok());
    assert!(w.pay_out(&mint, Payee::Give).is_ok());
    assert!(w.pay_out(&mint, Payee::Platform).is_ok());
    assert_eq!(w.zec_of(&plan.creator_receiver), pots[1]);
    assert_eq!(w.zec_of(&plan.give_receiver), pots[2]);
    assert_eq!(w.zec_of(&w.platform_receiver), pots[0]);
    w.assert_sound(&mint);
}

// ---- buy back and burn on the pool ----

fn burn_plan(fee_bps: u16) -> LaunchPlan {
    LaunchPlan { fee_bps, shares: [0, 0, 10_000, 0], ..Default::default() }
}

fn graduated_with_burn_pot(fee_bps: u16, pot: u64) -> (World, Pubkey) {
    let (mut w, mint) = graduated(&burn_plan(fee_bps));
    w.b.give_zec(&kit::authority(&mint), pot * 8 / 7 + 8);
    assert!(w.split(&mint).is_ok());
    // The price at graduation is far above the launch price. The burn
    // follows it up a 5% step a minute before it buys.
    w.b.wait(60);
    let calls = w.burn_until_it_buys(&mint);
    assert!(calls > 10);
    w.b.wait(60);
    (w, mint)
}

#[test]
fn a_burn_on_the_pool_buys_and_destroys() {
    let (mut w, mint) = graduated_with_burn_pot(200, 50 * ZEC1);
    let supply = w.b.mint_supply(&mint);
    let pot = w.ledger(&mint).pots[3];
    let burned_before = w.ledger(&mint).burn.total_burned;
    // The limit is measured against the smaller of the pool's reserve (its
    // ZEC balance plus the pool's own adjustment) and the reserve recorded at
    // the last burn, which grows a step at a time.
    let reserve = w.pool_reserves(&mint).0.min(w.ledger(&mint).burn.reference.quote);
    assert_ne!(w.pool(&mint).virtual_quote_reserves, 0);

    let r = w.burn_pool(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    let l = w.ledger(&mint);
    let spent = pot - l.pots[3];
    assert_eq!(spent, reserve * 200 / 40_000);
    let burned = supply - w.b.mint_supply(&mint);
    assert!(burned > 0);
    assert_eq!(l.burn.total_burned - burned_before, burned);
    let bn = kit::burner(&mint);
    assert_eq!(w.b.token_balance(&ata(&bn, &ZEC, &TOKEN)), 0);
    assert_eq!(w.b.token_balance(&ata(&bn, &mint, &TOKEN_2022)), 0);
    w.assert_sound(&mint);

    // Waiting time and price check, as on the curve.
    let r = w.burn_pool(&mint);
    assert_eq!(custom_code(&r), Some(err(E::BurnTooSoon)));
    let pusher = w.trader(1_000 * ZEC1);
    assert!(w.pool_buy(&pusher, &mint, 5 * ZEC1).is_ok());
    w.b.wait(60);
    let before = w.ledger(&mint).pots[3];
    assert!(w.burn_pool(&mint).is_ok());
    assert_eq!(w.ledger(&mint).pots[3], before, "a pushed-up price is not bought at");
    w.b.wait(60);
    w.burn_until_it_buys(&mint);
    assert!(w.ledger(&mint).pots[3] < before);
    // After a burn the pool's price at that moment is the new reference.
    let (q, b) = w.pool_reserves(&mint);
    let r = w.ledger(&mint).burn.reference;
    let (lhs, rhs) = (r.quote as u128 * b as u128, q as u128 * r.base as u128);
    assert!(lhs.abs_diff(rhs) <= rhs / 1_000_000, "recorded price is the pool's price");
    assert!(r.quote <= q, "the recorded reserve never exceeds the real one");
}

#[test]
fn each_burn_instruction_only_works_on_its_own_venue() {
    // Before graduation the pool does not exist.
    let mut w = World::new();
    let mint = w.launch(&burn_plan(200));
    w.b.give_zec(&kit::authority(&mint), ZEC1);
    assert!(w.split(&mint).is_ok());
    assert!(w.burn_pool(&mint).is_err());
    // After graduation the curve is closed for trading.
    w.graduate(&mint);
    w.b.wait(60);
    let r = w.burn_curve(&mint);
    assert_eq!(custom_code(&r), Some(err(E::WrongVenue)), "{}", logs(&r));
    assert!(w.burn_pool(&mint).is_ok());
}

#[test]
fn a_burn_started_on_the_curve_carries_on_from_the_pool() {
    let mut w = World::new();
    let mint = w.launch(&burn_plan(200));
    w.b.give_zec(&kit::authority(&mint), 50 * ZEC1);
    assert!(w.split(&mint).is_ok());
    w.b.wait(60);
    assert!(w.burn_curve(&mint).is_ok());
    assert!(w.ledger(&mint).burn.total_spent > 0);
    w.graduate(&mint);
    w.b.wait(60);
    // The price recorded on the curve is the reference for the first pool burn.
    // Graduation happened well above it, so the first call skips.
    let pot = w.ledger(&mint).pots[3];
    assert!(w.burn_pool(&mint).is_ok());
    assert_eq!(w.ledger(&mint).pots[3], pot);
    w.b.wait(60);
    w.burn_until_it_buys(&mint);
    assert!(w.ledger(&mint).pots[3] < pot);
    w.assert_sound(&mint);
}

#[test]
fn buying_before_and_selling_after_a_pool_burn_loses_money() {
    for fee in [50u16, 200, 300] {
        // Sizes that stay inside the 5% price band; larger ones make the burn skip.
        for attack in [ZEC1 / 100, ZEC1 / 20, ZEC1 / 8] {
            let (mut w, mint) = graduated_with_burn_pot(fee, 50 * ZEC1);
            assert!(w.burn_pool(&mint).is_ok());
            w.b.wait(60);
            let attacker = w.trader(1_000 * ZEC1);
            let start = w.zec_of(&attacker.pubkey());
            assert!(w.pool_buy(&attacker, &mint, attack).is_ok());
            let pot = w.ledger(&mint).pots[3];
            let r = w.burn_pool(&mint);
            assert!(r.is_ok(), "{}", logs(&r));
            assert!(w.ledger(&mint).pots[3] < pot, "fee {fee} attack {attack}: the burn should have bought");
            let coins = w.coins_of(&attacker.pubkey(), &mint);
            assert!(w.pool_sell(&attacker, &mint, coins).is_ok());
            let gain = w.zec_of(&attacker.pubkey()) as i128 - start as i128;
            assert!(gain < 0, "fee {fee} attack {attack}: attacker gained {gain}");
        }
    }
}

#[test]
fn burn_pool_refuses_substituted_accounts() {
    let (mut w, mint) = graduated_with_burn_pot(200, ZEC1);
    let other = w.launch(&burn_plan(200));
    w.graduate(&other);
    let payer = w.b.payer.pubkey();
    let thief = w.trader(ZEC1);
    let thief_zec = ata(&thief.pubkey(), &ZEC, &TOKEN);
    let fake = Pubkey::new_unique();
    type Swap = fn(&mut burrpad::accounts::BurnPool, Pubkey, &Pubkey, Pubkey);
    let swaps: Vec<(&str, Swap)> = vec![
        ("amm_program", |a, f, _, _| a.amm_program = f),
        ("amm_program (pump.fun's other program)", |a, _, _, _| a.amm_program = pump::PUMP),
        ("amm_global_config", |a, f, _, _| a.amm_global_config = f),
        ("pool (another coin's)", |a, _, o, _| a.pool = pump::pool(o)),
        ("pool_authority (another coin's)", |a, _, o, _| a.pool_authority = pump::pool_authority(o)),
        ("pool_base_token_account (another pool's)", |a, _, o, _| {
            a.pool_base_token_account = ata(&pump::pool(o), o, &TOKEN_2022)
        }),
        ("pool_quote_token_account (another pool's)", |a, _, o, _| {
            a.pool_quote_token_account = ata(&pump::pool(o), &ZEC, &TOKEN)
        }),
        ("pool_quote_token_account (a thief's)", |a, _, _, t| a.pool_quote_token_account = t),
        ("user_volume", |a, f, _, _| a.user_volume = f),
        ("fee_config (the curve's)", |a, _, _, _| a.fee_config = pump::fee_config()),
        ("buyback_fee_recipient (a thief's)", |a, _, _, t| a.buyback_fee_recipient = t),
        ("amm_event_authority", |a, f, _, _| a.amm_event_authority = f),
        ("vault (another coin's)", |a, _, o, _| a.vault = kit::vault(o)),
        ("ledger (another coin's)", |a, _, o, _| a.ledger = kit::ledger(o)),
        ("burner (another coin's)", |a, _, o, _| a.burner = kit::burner(o)),
        ("burner_zec (a thief's)", |a, _, _, t| a.burner_zec = t),
    ];
    for (name, swap) in swaps {
        let mut acc = w.burn_pool_accounts(&payer, &mint);
        swap(&mut acc, fake, &other, thief_zec);
        let r = w.b.send(&[kit::raw(acc, burrpad::instruction::BurnPool {})], &[]);
        assert!(r.is_err(), "swapping {name} was accepted");
    }
    assert_eq!(w.b.token_balance(&thief_zec), ZEC1);
    assert!(w.burn_pool(&mint).is_ok());
}

// ---- vault share after graduation ----

#[test]
fn vault_share_carries_straight_through_graduation() {
    let mut w = World::new();
    let mint = w.launch(&LaunchPlan { shares: [0, 0, 0, 10_000], ..Default::default() });
    let h = w.trader(100 * ZEC1);
    assert!(w.buy(&h, &mint, ZEC1 / 10).is_ok());
    let coins = w.coins_of(&h.pubkey(), &mint);
    assert!(w.stake(&h, &mint, coins).is_ok());
    w.graduate(&mint);
    // Fee from the pool is shared with the same staker, who never moved.
    let t = w.trader(100 * ZEC1);
    assert!(w.pool_buy(&t, &mint, 2 * ZEC1).is_ok());
    assert!(w.collect_curve(&mint).is_ok());
    assert!(w.collect_pool(&mint).is_ok());
    let pot = w.ledger(&mint).pots[4];
    assert!(pot > 0);
    w.b.wait(DAY);
    let before = w.zec_of(&h.pubkey());
    let r = w.unstake(&h, &mint, coins);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(pot - (w.zec_of(&h.pubkey()) - before) <= 1);
    assert_eq!(w.coins_of(&h.pubkey(), &mint), coins);
    // A newcomer can stake after graduation too.
    assert!(w.stake(&t, &mint, 1_000).is_ok());
    assert!(w.unstake(&t, &mint, 1_000).is_ok());
    w.assert_sound(&mint);
}

#[test]
fn collect_pool_refuses_substituted_accounts() {
    let (mut w, mint) = graduated(&LaunchPlan::default());
    let other = w.launch(&LaunchPlan::default());
    w.graduate(&other);
    let t = w.trader(100 * ZEC1);
    assert!(w.pool_buy(&t, &mint, ZEC1).is_ok());
    assert!(w.pool_buy(&t, &other, ZEC1).is_ok());
    let payer = w.b.payer.pubkey();
    let fake = Pubkey::new_unique();
    type Swap = fn(&mut burrpad::accounts::CollectPool, Pubkey, &Pubkey);
    let swaps: Vec<(&str, Swap)> = vec![
        ("amm_program", |a, f, _| a.amm_program = f),
        ("amm_global_config", |a, f, _| a.amm_global_config = f),
        ("pool (another coin's)", |a, _, o| a.pool = pump::pool(o)),
        ("pool_authority (another coin's)", |a, _, o| a.pool_authority = pump::pool_authority(o)),
        ("pool_quote_token_account (another pool's)", |a, _, o| {
            a.pool_quote_token_account = ata(&pump::pool(o), &ZEC, &TOKEN)
        }),
        ("amm_creator_vault_authority (another coin's)", |a, _, o| {
            a.amm_creator_vault_authority = pump::pda(&[b"creator_vault", kit::authority(o).as_ref()], &pump::PUMP_AMM)
        }),
        ("amm_creator_vault_ata", |a, f, _| a.amm_creator_vault_ata = f),
        ("amm_event_authority", |a, f, _| a.amm_event_authority = f),
        ("vault (another coin's)", |a, _, o| a.vault = kit::vault(o)),
        ("ledger (another coin's)", |a, _, o| a.ledger = kit::ledger(o)),
    ];
    for (name, swap) in swaps {
        let mut acc = w.collect_pool_accounts(&payer, &mint);
        swap(&mut acc, fake, &other);
        let r = w.b.send(&[kit::raw(acc, burrpad::instruction::CollectPool {})], &[]);
        assert!(r.is_err(), "swapping {name} was accepted");
    }
    assert!(w.collect_pool(&mint).is_ok());
    // Coin B's fee is untouched by collecting coin A's.
    assert!(w.pool(&other).creator_fees > 0);
    assert_eq!(w.vault_balance(&other), 0);
}

#[test]
fn the_pool_burn_limit_follows_the_fee_the_pool_really_charges() {
    let (mut w, mint) = graduated_with_burn_pot(200, 50 * ZEC1);
    // pump.fun's administrators lower this pool's creator fee to 0.50%.
    let key = pump::pool(&mint);
    let mut a = w.b.account(&key).unwrap();
    a.data[261..269].copy_from_slice(&50u64.to_le_bytes());
    w.b.svm.set_account(key, a).unwrap();
    let reserve = w.pool_reserves(&mint).0.min(w.ledger(&mint).burn.reference.quote);
    let before = w.ledger(&mint).pots[3];
    let r = w.burn_pool(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(before - w.ledger(&mint).pots[3], reserve * 50 / 40_000);
}

fn lp_supply(w: &World, mint: &Pubkey) -> u64 {
    let d = w.b.account(&pump::pool(mint)).unwrap().data;
    u64::from_le_bytes(d[203..211].try_into().unwrap())
}

#[test]
fn liquidity_added_at_the_end_of_a_climb_does_not_raise_the_pool_burns_limit() {
    // After graduation the price is far above the launch reference and the
    // burn climbs to it in steps. On the call where the climb ends, someone
    // adds liquidity to the pool for that one transaction, hoping the spend
    // limit follows the swollen reserve.
    let mut w = World::new();
    let mint = w.launch(&burn_plan(200));
    w.graduate(&mint);
    w.b.give_zec(&kit::authority(&mint), 2_000 * ZEC1);
    assert!(w.split(&mint).is_ok());
    let lp = w.trader(100_000 * ZEC1);
    assert!(w.pool_buy(&lp, &mint, 30 * ZEC1).is_ok());
    let mk = pump::create_ata(&lp.pubkey(), &lp.pubkey(), &pump::lp_mint(&mint), &TOKEN_2022);
    assert!(w.b.send_as(Some(&lp), &[mk], &[]).is_ok());

    for _ in 0..300 {
        w.b.wait(60);
        let (q, b) = w.pool_reserves(&mint);
        let r = w.ledger(&mint).burn.reference;
        let will_buy = q as u128 * r.base as u128 * 10_000 <= r.quote as u128 * b as u128 * 10_500;
        if !will_buy {
            assert!(w.burn_pool(&mint).is_ok());
            continue;
        }
        // The honest limit: the pool's real reserve times a quarter of the fee.
        let honest = q as u128 * 200 / 40_000;
        let pot = w.ledger(&mint).pots[3];
        let payer = w.b.payer.pubkey();
        let ixs = [
            // As much liquidity as the holder's coins allow.
            pump::pool_deposit(
                &mint,
                &lp.pubkey(),
                (lp_supply(&w, &mint) as u128 * w.coins_of(&lp.pubkey(), &mint) as u128 * 9 / 10 / b as u128) as u64,
                u64::MAX,
                u64::MAX,
            ),
            kit::raw(w.burn_pool_accounts(&payer, &mint), burrpad::instruction::BurnPool {}),
        ];
        let payer_kp = w.b.payer.insecure_clone();
        let r = w.b.send_as(Some(&lp), &ixs, &[&payer_kp]);
        assert!(r.is_ok(), "{}", logs(&r));
        let spent = (pot - w.ledger(&mint).pots[3]) as u128;
        assert!(spent > 0, "this was the call that buys");
        assert!(spent <= honest + 1, "spent {spent}, four times the pool's real reserve would allow {honest}");
        return;
    }
    panic!("the burn never reached the price");
}

#[test]
fn a_pool_record_that_does_not_read_as_expected_stops_the_burn() {
    // If pump.fun rearranged its pool account, the fields we compare would
    // stop matching. Each is changed in turn here, by hand.
    for (what, offset) in
        [("maker", 11usize), ("coin mint", 43), ("quote mint", 75), ("coin account", 139), ("ZEC account", 171)]
    {
        let (mut w, mint) = graduated_with_burn_pot(200, 50 * ZEC1);
        let key = pump::pool(&mint);
        let mut a = w.b.account(&key).unwrap();
        a.data[offset] ^= 1;
        w.b.svm.set_account(key, a).unwrap();
        let pot = w.ledger(&mint).pots[3];
        let r = w.burn_pool(&mint);
        assert_eq!(custom_code(&r), Some(err(E::WrongPumpAccount)), "{what}\n{}", logs(&r));
        assert!(crate::bench::refused_by_burrpad(&r), "{what}");
        assert_eq!(w.ledger(&mint).pots[3], pot);
    }
}
