//! Collect and split, and the three pots that are simply paid out.

use crate::bench::{custom_code, logs};
use crate::kit::{self, err, LaunchPlan, World};
use crate::pump::{self, ata, TOKEN, ZEC};
use burr::error::BurrError as E;
use burr::state::Payee;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

const ZEC1: u64 = 100_000_000;

/// Launches a coin and trades it so a fee is waiting. Returns (mint, fee waiting).
fn coin_with_fee(w: &mut World, plan: &LaunchPlan) -> (Pubkey, u64) {
    let mint = w.launch(plan);
    let t = w.trader(100 * ZEC1);
    assert!(w.buy(&t, &mint, ZEC1).is_ok());
    let have = w.coins_of(&t.pubkey(), &mint);
    assert!(w.sell(&t, &mint, have / 3).is_ok());
    let fee = w.b.curve(&mint).creator_fee;
    assert!(fee > 0);
    (mint, fee)
}

#[test]
fn collect_brings_the_fee_in_and_splits_it_exactly() {
    let mut w = World::new();
    let (mint, fee) = coin_with_fee(&mut w, &LaunchPlan::default());
    let r = w.collect_curve(&mint);
    assert!(r.is_ok(), "{}", logs(&r));

    assert_eq!(w.vault_balance(&mint), fee);
    let l = w.ledger(&mint);
    assert_eq!(l.accounted, fee);
    assert_eq!(l.total_split, fee as u128);
    // One eighth to the platform, the rest in five equal parts, all rounded down.
    let platform = fee as u128 * 1_250 / 10_000;
    assert_eq!(l.pots[0] as u128, platform);
    for i in 1..6 {
        assert_eq!(l.pots[i] as u128, fee as u128 * (8_750 * 2_000) / 100_000_000);
    }
    let pots: u64 = l.pots.iter().sum();
    assert!(fee - pots < 6, "in = credited + a carry of under 6 units");
    w.assert_sound(&mint);
    // Nothing is left on pump.fun's side.
    assert_eq!(w.b.curve(&mint).creator_fee, 0);
}

#[test]
fn collect_with_nothing_waiting_is_harmless() {
    let mut w = World::new();
    let mint = w.launch(&LaunchPlan::default());
    let r = w.collect_curve(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.ledger(&mint).accounted, 0);
    let (mint, _) = coin_with_fee(&mut w, &LaunchPlan::default());
    assert!(w.collect_curve(&mint).is_ok());
    let before = w.ledger(&mint).pots;
    assert!(w.collect_curve(&mint).is_ok());
    assert!(w.split(&mint).is_ok());
    assert_eq!(w.ledger(&mint).pots, before);
}

#[test]
fn collect_can_be_called_by_a_stranger_and_done_outside_burr() {
    let mut w = World::new();
    let (mint, fee) = coin_with_fee(&mut w, &LaunchPlan::default());
    // A stranger calls pump.fun's own instructions directly.
    let s = w.b.wallet();
    let auth = kit::authority(&mint);
    let ixs = [pump::sweep_creator_fee(&mint, &s.pubkey(), &auth), pump::collect_creator_fee_v2(&auth)];
    let r = w.b.send_as(Some(&s), &ixs, &[]);
    assert!(r.is_ok(), "{}", logs(&r));
    // The fee is in the vault, not yet split.
    assert_eq!(w.vault_balance(&mint), fee);
    assert_eq!(w.ledger(&mint).accounted, 0);
    // The stranger splits it too.
    let i = w.split_ix(&mint);
    assert!(w.b.send_as(Some(&s), &[i], &[]).is_ok());
    assert_eq!(w.ledger(&mint).accounted, fee);
    w.assert_sound(&mint);
}

#[test]
fn zec_sent_straight_to_a_vault_counts_as_fee() {
    let mut w = World::new();
    let mint = w.launch(&LaunchPlan::default());
    w.b.give_zec(&kit::authority(&mint), 12_345);
    assert!(w.split(&mint).is_ok());
    assert_eq!(w.ledger(&mint).accounted, 12_345);
    w.assert_sound(&mint);
}

#[test]
fn splitting_in_many_small_calls_gives_the_same_as_one() {
    let mut w = World::new();
    let plan = LaunchPlan { shares: [3_333, 1, 3_333, 3_332, 1], ..Default::default() };
    let a = w.launch(&plan);
    let b = w.launch(&plan);
    // Coin A gets 9,999 units in one go; coin B gets them 7 at a time.
    w.b.give_zec(&kit::authority(&a), 9_996);
    assert!(w.split(&a).is_ok());
    for _ in 0..1_428 {
        w.b.give_zec(&kit::authority(&b), 7);
        assert!(w.split(&b).is_ok());
    }
    assert_eq!(w.ledger(&a).pots, w.ledger(&b).pots);
    assert_eq!(w.ledger(&a).accounted, w.ledger(&b).accounted);
    w.assert_sound(&a);
    w.assert_sound(&b);
}

#[test]
fn two_coins_never_share_money() {
    let mut w = World::new();
    let (a, fee_a) = coin_with_fee(&mut w, &LaunchPlan::default());
    let (b, fee_b) = coin_with_fee(&mut w, &LaunchPlan::default());
    assert!(w.collect_curve(&a).is_ok());
    assert_eq!(w.vault_balance(&a), fee_a);
    assert_eq!(w.vault_balance(&b), 0, "coin B's fee is still on pump.fun, untouched");
    assert!(w.collect_curve(&b).is_ok());
    assert_eq!(w.vault_balance(&b), fee_b);

    // Every way of mixing coin A's accounts into a call for coin B is refused.
    let payer = w.b.payer.pubkey();
    type Mix = fn(&mut burr::accounts::SplitOnly, &Pubkey);
    let mixes: Vec<(&str, Mix)> = vec![
        ("coin", |x, a| x.coin = kit::coin(a)),
        ("ledger", |x, a| x.ledger = kit::ledger(a)),
        ("authority", |x, a| x.authority = kit::authority(a)),
        ("vault", |x, a| x.vault = kit::vault(a)),
    ];
    for (name, mix) in mixes {
        let mut acc = burr::accounts::SplitOnly {
            mint: b,
            coin: kit::coin(&b),
            ledger: kit::ledger(&b),
            authority: kit::authority(&b),
            vault: kit::vault(&b),
            zec_mint: ZEC,
            token_program: TOKEN,
        };
        mix(&mut acc, &a);
        let r = w.b.send(&[kit::raw(acc, burr::instruction::Split {})], &[]);
        assert!(r.is_err(), "mixing in coin A's {name} was accepted");
    }
    // The same for a payout: coin A's vault cannot pay coin B's creator.
    let receiver = w.coin(&b).creator_receiver;
    let mut acc = w.pay_out_accounts(&payer, &b, &receiver);
    acc.vault = kit::vault(&a);
    let r = w.b.send(&[kit::raw(acc, burr::instruction::PayOut { payee: Payee::Creator })], &[]);
    assert!(r.is_err());
    let mut acc = w.pay_out_accounts(&payer, &b, &receiver);
    acc.ledger = kit::ledger(&a);
    let r = w.b.send(&[kit::raw(acc, burr::instruction::PayOut { payee: Payee::Creator })], &[]);
    assert!(r.is_err());
    assert_eq!(w.vault_balance(&a), fee_a);
}

#[test]
fn collect_refuses_substituted_accounts() {
    let mut w = World::new();
    let (mint, _) = coin_with_fee(&mut w, &LaunchPlan::default());
    let (other, _) = coin_with_fee(&mut w, &LaunchPlan::default());
    let payer = w.b.payer.pubkey();
    let fake = Pubkey::new_unique();
    type Swap = fn(&mut burr::accounts::CollectCurve, Pubkey, &Pubkey);
    let swaps: Vec<(&str, Swap)> = vec![
        ("pump_program", |a, f, _| a.pump_program = f),
        ("pump_global", |a, f, _| a.pump_global = f),
        ("bonding_curve (another coin's)", |a, _, o| a.bonding_curve = pump::curve(o)),
        ("associated_quote_bonding_curve", |a, f, _| a.associated_quote_bonding_curve = f),
        ("creator_vault (another coin's)", |a, _, o| a.creator_vault = pump::creator_vault(&kit::authority(o))),
        ("creator_vault_ata", |a, f, _| a.creator_vault_ata = f),
        ("pump_event_authority", |a, f, _| a.pump_event_authority = f),
        ("vault (another coin's)", |a, _, o| a.vault = kit::vault(o)),
        ("zec_mint (the coin itself)", |a, _, o| a.zec_mint = *o),
        ("token_program (Token-2022)", |a, _, _| a.token_program = pump::TOKEN_2022),
    ];
    for (name, swap) in swaps {
        let mut acc = w.collect_curve_accounts(&payer, &mint);
        swap(&mut acc, fake, &other);
        let r = w.b.send(&[kit::raw(acc, burr::instruction::CollectCurve {})], &[]);
        assert!(r.is_err(), "swapping {name} was accepted");
    }
    assert!(w.collect_curve(&mint).is_ok());
}

// ---- creator's share, give, and the platform's part ----

#[test]
fn each_payout_pot_goes_whole_to_its_fixed_receiver() {
    let mut w = World::new();
    let plan = LaunchPlan::default();
    let (mint, _) = coin_with_fee(&mut w, &plan);
    assert!(w.collect_curve(&mint).is_ok());
    let pots = w.ledger(&mint).pots;
    for (payee, receiver, pot) in [
        (Payee::Platform, w.platform_receiver, pots[0]),
        (Payee::Creator, plan.creator_receiver, pots[1]),
        (Payee::Give, plan.give_receiver, pots[2]),
    ] {
        assert!(pot > 0);
        let r = w.pay_out(&mint, payee);
        assert!(r.is_ok(), "{}", logs(&r));
        assert_eq!(w.zec_of(&receiver), pot);
        assert_eq!(w.ledger(&mint).pots[payee.sink()], 0);
        // A second call pays nothing more.
        assert!(w.pay_out(&mint, payee).is_ok());
        assert_eq!(w.zec_of(&receiver), pot);
        w.assert_sound(&mint);
    }
    // The other pots were not touched.
    assert_eq!(w.ledger(&mint).pots[3..], pots[3..]);
}

#[test]
fn a_payout_cannot_be_pointed_anywhere_else() {
    let mut w = World::new();
    let plan = LaunchPlan::default();
    let (mint, _) = coin_with_fee(&mut w, &plan);
    assert!(w.collect_curve(&mint).is_ok());
    let payer = w.b.payer.pubkey();
    let thief = w.b.wallet();

    // The thief names themselves as receiver.
    let acc = w.pay_out_accounts(&payer, &mint, &thief.pubkey());
    let r = w.b.send(&[kit::raw(acc, burr::instruction::PayOut { payee: Payee::Creator })], &[]);
    assert_eq!(custom_code(&r), Some(err(E::WrongReceiver)));

    // The right receiver, but the thief's ZEC account as destination.
    w.b.give_zec(&thief.pubkey(), 1);
    let mut acc = w.pay_out_accounts(&payer, &mint, &plan.creator_receiver);
    acc.receiver_zec = ata(&thief.pubkey(), &ZEC, &TOKEN);
    let r = w.b.send(&[kit::raw(acc, burr::instruction::PayOut { payee: Payee::Creator })], &[]);
    assert!(r.is_err());

    // The give receiver cannot take the creator's pot, nor the reverse.
    let acc = w.pay_out_accounts(&payer, &mint, &plan.give_receiver);
    let r = w.b.send(&[kit::raw(acc, burr::instruction::PayOut { payee: Payee::Creator })], &[]);
    assert_eq!(custom_code(&r), Some(err(E::WrongReceiver)));
    assert_eq!(w.zec_of(&thief.pubkey()), 1);
}

#[test]
fn the_administrator_cannot_move_a_coins_money() {
    let mut w = World::new();
    let (mint, _) = coin_with_fee(&mut w, &LaunchPlan::default());
    assert!(w.collect_curve(&mint).is_ok());
    let admin = w.admin.insecure_clone();
    // There is no instruction for it. The nearest thing, a payout naming the
    // administrator as receiver, is refused like anyone else's.
    let acc = w.pay_out_accounts(&admin.pubkey(), &mint, &admin.pubkey());
    for payee in [Payee::Platform, Payee::Creator, Payee::Give] {
        let r = w.b.send_as(Some(&admin), &[kit::raw(acc, burr::instruction::PayOut { payee })], &[]);
        assert_eq!(custom_code(&r), Some(err(E::WrongReceiver)));
    }
    // Changing the platform receiver afterwards does not redirect this coin.
    assert!(w.as_admin(burr::instruction::SetPlatformReceiver { receiver: admin.pubkey() }).is_ok());
    assert!(w.pay_out(&mint, Payee::Platform).is_ok());
    assert_eq!(w.zec_of(&admin.pubkey()), 0);
    assert!(w.zec_of(&w.platform_receiver) > 0);
}

#[test]
fn the_platforms_part_scales_with_the_fee() {
    let mut w = World::new();
    for fee_bps in [100u16, 200, 300] {
        let (mint, fee) = coin_with_fee(&mut w, &LaunchPlan { fee_bps, ..Default::default() });
        assert!(w.collect_curve(&mint).is_ok());
        assert_eq!(w.ledger(&mint).pots[0] as u128, fee as u128 / 8);
    }
}

#[test]
fn a_vault_holding_less_than_the_ledger_says_stops_the_split() {
    // This cannot happen by any instruction; the vault is rewritten by hand.
    // If it ever did, the program must stop rather than carry on with wrong sums.
    let mut w = World::new();
    let (mint, fee) = coin_with_fee(&mut w, &LaunchPlan::default());
    assert!(w.collect_curve(&mint).is_ok());
    let auth = kit::authority(&mint);
    w.b.write_token_account(kit::vault(&mint), &ZEC, &auth, fee - 1, &TOKEN);
    let r = w.split(&mint);
    assert_eq!(custom_code(&r), Some(err(E::VaultShort)), "{}", logs(&r));
}

#[test]
fn a_pot_cannot_be_debited_below_zero() {
    let mut l = burr::state::Ledger {
        mint: Pubkey::default(),
        bump: 0,
        accounted: 100,
        total_split: 100,
        credited: [0; 6],
        pots: [10, 20, 30, 40, 0, 0],
        burn: Default::default(),
        vault_share: Default::default(),
        king: Default::default(),
    };
    assert!(burr::ops::debit(&mut l, 1, 21).is_err(), "more than the pot holds");
    assert!(burr::ops::debit(&mut l, 1, 20).is_ok());
    assert_eq!((l.pots[1], l.accounted), (0, 80));
    l.accounted = 5;
    assert!(burr::ops::debit(&mut l, 2, 30).is_err(), "more than the ledger's total");
}

#[test]
fn a_receiver_cannot_make_their_pot_unpayable() {
    let mut w = World::new();
    let receiver = w.trader(1);
    let other = w.b.wallet();
    let plan = LaunchPlan { creator_receiver: receiver.pubkey(), ..Default::default() };
    let (mint, _) = coin_with_fee(&mut w, &plan);
    assert!(w.collect_curve(&mint).is_ok());
    // The receiver hands their ZEC account to another key.
    let r_zec = ata(&receiver.pubkey(), &ZEC, &TOKEN);
    let i = kit::give_token_account_away(&r_zec, &receiver.pubkey(), &other.pubkey());
    assert!(w.b.send_as(Some(&receiver), &[i], &[]).is_ok());
    let pot = w.ledger(&mint).pots[1];
    let r = w.pay_out(&mint, Payee::Creator);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.b.token_balance(&r_zec), 1 + pot, "paid to the same address as always");
    assert_eq!(w.ledger(&mint).pots[1], 0);
}

#[test]
fn a_coins_own_addresses_cannot_be_named_as_receivers() {
    let mut w = World::new();
    for which in 0..4 {
        let mint = solana_keypair::Keypair::new();
        let own = if which < 2 { kit::authority(&mint.pubkey()) } else { kit::burner(&mint.pubkey()) };
        let mut plan = LaunchPlan::default();
        if which % 2 == 0 {
            plan.creator_receiver = own;
        } else {
            plan.give_receiver = own;
        }
        let payer = w.b.payer.pubkey();
        let i = w.launch_ix(&payer, &mint.pubkey(), &plan);
        let r = w.b.send(&[i], &[&mint]);
        assert_eq!(custom_code(&r), Some(err(E::ReceiverNotAllowed)), "case {which}\n{}", logs(&r));
    }
}

#[test]
fn an_empty_pot_pays_nothing_and_makes_no_account() {
    let mut w = World::new();
    // No give hook: its receiver is the empty address and its pot stays empty.
    let plan = LaunchPlan { shares: [10_000, 0, 0, 0, 0], give_receiver: Pubkey::default(), ..Default::default() };
    let (mint, _) = coin_with_fee(&mut w, &plan);
    assert!(w.collect_curve(&mint).is_ok());
    let r = w.pay_out(&mint, Payee::Give);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(w.b.account(&ata(&Pubkey::default(), &ZEC, &TOKEN)).is_none(), "no account was made for nobody");
}

#[test]
fn the_platforms_receiver_cannot_be_a_coins_own_address_either() {
    let mut w = World::new();
    let mint = solana_keypair::Keypair::new();
    let own = kit::authority(&mint.pubkey());
    assert!(w.as_admin(burr::instruction::SetPlatformReceiver { receiver: own }).is_ok());
    let payer = w.b.payer.pubkey();
    let i = w.launch_ix(&payer, &mint.pubkey(), &LaunchPlan::default());
    let r = w.b.send(&[i], &[&mint]);
    assert_eq!(custom_code(&r), Some(err(E::ReceiverNotAllowed)), "{}", logs(&r));
}
