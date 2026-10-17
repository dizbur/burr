//! Edge cases of coins priced in $BURR: pots below any minimum, the $BURR
//! burn racing a collect, every share at its edge, graduating in the middle of
//! collecting, and the platform's $BURR split as the server makes it.

use crate::bench::{custom_code, logs};
use crate::kit::{self, err, LaunchPlan, World};
use crate::pump::{self, ata, BUYBACK, TOKEN, TOKEN_2022, ZEC};
use burrpad::error::BurrpadError as E;
use burrpad::state::Payee;
use burrpad::{BURR_MINT, QUOTE_BURR};
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

const BURR: u64 = 1_000_000;

fn burr_world() -> World {
    let mut w = World::new();
    w.make_burr();
    w.give_burr(&BUYBACK, 0);
    w
}

fn burr_plan() -> LaunchPlan {
    LaunchPlan { shares: [2_000, 2_000, 2_000, 2_000], burr_share: 2_000, quote: QUOTE_BURR, ..Default::default() }
}

/// Every pot's credit is exactly its share of all that was split, and the books close.
fn assert_shares(w: &World, mint: &Pubkey) {
    let (c, l) = (w.coin(mint), w.ledger(mint));
    let rest = (10_000 - c.platform_fraction_bps) as u128;
    let weights = [
        c.platform_fraction_bps as u128 * 10_000,
        rest * c.shares[0] as u128,
        rest * c.shares[1] as u128,
        rest * c.shares[2] as u128,
        rest * c.shares[3] as u128,
        rest * c.burr_share as u128,
    ];
    let credited = [l.credited[0], l.credited[1], l.credited[2], l.credited[3], l.credited[4], l.burr.credited];
    for (k, (got, weight)) in credited.iter().zip(weights).enumerate() {
        assert_eq!(*got, l.total_split * weight / 100_000_000, "pot {k}");
    }
    assert_eq!(w.vault_balance(mint), l.accounted);
    w.assert_sound(mint);
}

/// A Token-2022 instruction from `owner`'s $BURR account: burn (15) or transfer to `to` (12), checked.
fn burr_ix(code: u8, owner: &Pubkey, to: Option<&Pubkey>, amount: u64) -> Instruction {
    let mut data = vec![code];
    data.extend_from_slice(&amount.to_le_bytes());
    data.push(6);
    let from = ata(owner, &BURR_MINT, &TOKEN_2022);
    let accounts = match to {
        Some(to) => vec![
            AccountMeta::new(from, false),
            AccountMeta::new_readonly(BURR_MINT, false),
            AccountMeta::new(ata(to, &BURR_MINT, &TOKEN_2022), false),
            AccountMeta::new_readonly(*owner, true),
        ],
        None => vec![
            AccountMeta::new(from, false),
            AccountMeta::new(BURR_MINT, false),
            AccountMeta::new_readonly(*owner, true),
        ],
    };
    Instruction { program_id: TOKEN_2022, accounts, data }
}

#[test]
fn a_burr_coin_s_pots_below_any_minimum_are_paid_burned_or_kept_never_stuck() {
    let mut w = burr_world();
    let mint = w.launch(&burr_plan());
    // A fee of a handful of units: each pot gets one or two, or nothing.
    w.give_burr(&kit::authority(&mint), 7);
    assert!(w.split(&mint).is_ok());
    let l = w.ledger(&mint);
    assert!(l.total_split == 7 && l.pots.iter().all(|p| *p <= 4), "{:?}", l.pots);
    assert_shares(&w, &mint);
    // The coin burn: too little to buy is refused as "nothing to burn", never by pump.fun, and nothing moves.
    w.b.wait(60);
    let (vault, pot) = (w.vault_balance(&mint), w.ledger(&mint).pots[3]);
    let r = w.burn_curve(&mint);
    if r.is_err() {
        assert_eq!(custom_code(&r), Some(err(E::NothingToBurn)), "{}", logs(&r));
    }
    assert_eq!((w.vault_balance(&mint), w.ledger(&mint).pots[3]), (vault, pot));
    // The $BURR pot has no minimum: a single unit is burned.
    let mut grow = 0;
    while w.ledger(&mint).burr.pot == 0 && grow < 20 {
        w.give_burr(&kit::authority(&mint), 3);
        assert!(w.split(&mint).is_ok());
        grow += 1;
    }
    let pot = w.ledger(&mint).burr.pot;
    assert!(pot > 0 && pot < 10);
    let supply = w.b.mint_supply(&BURR_MINT);
    let r = w.burn_burr_pot(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.b.mint_supply(&BURR_MINT), supply - pot);
    // Payouts and a holder release of a unit or two go through.
    for payee in [Payee::Platform, Payee::Creator, Payee::Give] {
        let r = w.pay_out(&mint, payee);
        assert!(r.is_ok(), "{payee:?}: {}", logs(&r));
    }
    w.b.wait(1_800);
    if w.ledger(&mint).pots[4] > 0 {
        let r = w.release_holders(&mint);
        assert!(r.is_ok(), "{}", logs(&r));
    }
    assert_shares(&w, &mint);
}

#[test]
fn the_burr_burn_racing_a_collect_books_every_unit_once() {
    let mut w = burr_world();
    let mint = w.launch(&burr_plan());
    let t = w.burr_trader(10_000_000 * BURR);
    let payer = w.b.payer.pubkey();
    let collect = |w: &World| kit::raw(w.collect_curve_accounts(&payer, &mint), burrpad::instruction::CollectCurve {});
    let burn = |w: &World| kit::raw(w.burn_burr_pot_accounts(&mint), burrpad::instruction::BurnBurrPot {});
    assert!(w.buy(&t, &mint, 300_000 * BURR).is_ok());
    // Collect, then burn, in one transaction: the burn takes the pot the collect has just filled.
    let (c, b) = (collect(&w), burn(&w));
    let r = w.b.send(&[c, b], &[]);
    assert!(r.is_ok(), "{}", logs(&r));
    let l = w.ledger(&mint);
    assert!(l.burr.credited > 0);
    assert_eq!((l.burr.pot, l.burr.total_burned as u128), (0, l.burr.credited));
    // More trades; then burn first and collect after, in one transaction: the new share stays in the pot.
    assert!(w.buy(&t, &mint, 300_000 * BURR).is_ok());
    assert!(w.collect_curve(&mint).is_ok());
    assert!(w.buy(&t, &mint, 300_000 * BURR).is_ok());
    let before = w.ledger(&mint);
    let (b, c) = (burn(&w), collect(&w));
    let r = w.b.send(&[b, c], &[]);
    assert!(r.is_ok(), "{}", logs(&r));
    let l = w.ledger(&mint);
    assert_eq!(l.burr.total_burned, before.burr.total_burned + before.burr.pot, "the burn took the pot as it was");
    assert!(l.burr.pot > 0, "the collect's share arrived after it");
    assert_eq!(l.burr.credited, l.burr.pot as u128 + l.burr.total_burned as u128);
    // Two burns in one transaction: the second finds nothing, so the whole transaction is refused and nothing moves.
    let supply = w.b.mint_supply(&BURR_MINT);
    let (b1, b2) = (burn(&w), burn(&w));
    assert_eq!(custom_code(&w.b.send(&[b1, b2], &[])), Some(err(E::NothingToBurn)));
    assert_eq!((w.ledger(&mint).burr.pot, w.b.mint_supply(&BURR_MINT)), (l.burr.pot, supply));
    assert_shares(&w, &mint);
}

#[test]
fn a_burr_launch_takes_every_share_at_its_edge() {
    let mut w = burr_world();
    let cases: [([u16; 4], u16); 9] = [
        ([10_000, 0, 0, 0], 0),
        ([0, 10_000, 0, 0], 0),
        ([0, 0, 10_000, 0], 0),
        ([0, 0, 0, 10_000], 0),
        ([0, 0, 0, 0], 10_000),
        ([1, 1, 1, 1], 9_996),
        ([9_996, 1, 1, 1], 1),
        ([1, 0, 0, 0], 9_999),
        ([0, 0, 0, 1], 9_999),
    ];
    for (shares, burr_share) in cases {
        let mint = w.launch(&LaunchPlan { shares, burr_share, quote: QUOTE_BURR, ..Default::default() });
        // A fee that does not divide evenly.
        w.give_burr(&kit::authority(&mint), 1_000_003);
        assert!(w.split(&mint).is_ok());
        assert_shares(&w, &mint);
        // Each hook with no share refuses its way out; each with one works.
        w.b.wait(1_800);
        let holders = w.release_holders(&mint);
        assert_eq!(holders.is_ok(), shares[3] > 0, "holders, {shares:?} {burr_share}");
        let burr = w.burn_burr_pot(&mint);
        assert_eq!(burr.is_ok(), burr_share > 0, "$BURR, {shares:?} {burr_share}\n{}", logs(&burr));
        for payee in [Payee::Platform, Payee::Creator, Payee::Give] {
            assert!(w.pay_out(&mint, payee).is_ok());
        }
        assert_shares(&w, &mint);
    }
    // One unit over, or one short, is refused.
    for (shares, burr_share) in [([2_500u16; 4], 1u16), ([2_500, 2_500, 2_500, 2_499], 0)] {
        let (_, r) = w.try_launch(&LaunchPlan { shares, burr_share, quote: QUOTE_BURR, ..Default::default() });
        assert_eq!(custom_code(&r), Some(err(E::SharesDoNotAddUp)), "{shares:?} {burr_share}");
    }
}

#[test]
fn a_burr_coin_graduating_in_the_middle_of_collecting_loses_no_fee() {
    let mut w = burr_world();
    let mint = w.launch(&burr_plan());
    let t = w.burr_trader(10_000_000 * BURR);
    assert!(w.buy(&t, &mint, 500_000 * BURR).is_ok());
    // A whale buys the curve out: it is complete, but not yet moved to its pool.
    let whale = w.burr_trader(1 << 58);
    for _ in 0..40 {
        if w.b.curve(&mint).complete {
            break;
        }
        let r = w.buy(&whale, &mint, w.b.curve(&mint).virtual_quote_reserves / 4);
        assert!(r.is_ok(), "{}", logs(&r));
    }
    assert!(w.b.curve(&mint).complete);
    // The curve's fee is still collected; the burn knows the curve is done; there is no pool yet.
    let r = w.collect_curve(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    let after_curve = w.ledger(&mint).total_split;
    assert!(after_curve > 0);
    w.b.wait(60);
    assert_eq!(custom_code(&w.burn_curve(&mint)), Some(err(E::WrongVenue)));
    assert!(w.collect_pool(&mint).is_err(), "no pool before the move");
    // The move to the pool and both collects, in one transaction.
    w.b.svm.airdrop(&pump::WITHDRAW_AUTHORITY, 1_000_000_000).unwrap();
    let payer = w.b.payer.pubkey();
    let migrate = pump::migrate_v2_q(&mint, &whale.pubkey(), (BURR_MINT, TOKEN_2022));
    let cc = kit::raw(w.collect_curve_accounts(&payer, &mint), burrpad::instruction::CollectCurve {});
    let cp = kit::raw(w.collect_pool_accounts(&payer, &mint), burrpad::instruction::CollectPool {});
    let mut keys: Vec<Pubkey> = vec![pump::PUMP, burrpad::ID];
    for i in [&migrate, &cc, &cp] {
        for a in &i.accounts {
            if !a.is_signer && !keys.contains(&a.pubkey) {
                keys.push(a.pubkey);
            }
        }
    }
    let table = w.b.address_table(&keys);
    let (_, r) = w.b.send_with_table(&whale, &[migrate, cc, cp], &[&w.b.payer.insecure_clone()], &table);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.pool(&mint).quote_mint, BURR_MINT);
    assert_shares(&w, &mint);
    // Trades on the pool; its fee arrives too.
    assert!(w.pool_buy(&t, &mint, 200_000 * BURR).is_ok());
    let before = w.ledger(&mint).total_split;
    assert!(w.collect_pool(&mint).is_ok());
    assert!(w.ledger(&mint).total_split > before, "the pool's fee, in $BURR");
    assert_shares(&w, &mint);
}

#[test]
fn the_platform_s_burr_split_lands_four_fifths_burned_and_one_fifth_in_the_treasury_s_burr_account() {
    let mut w = burr_world();
    // The platform receiver, a wallet the server holds; the treasury, a wallet that never held anything.
    let platform = w.b.wallet();
    let treasury = Keypair::new().pubkey();
    assert!(w.as_admin(burrpad::instruction::SetPlatformReceiver { receiver: platform.pubkey() }).is_ok());
    let mint = w.launch(&burr_plan());
    let t = w.burr_trader(10_000_000 * BURR);
    assert!(w.buy(&t, &mint, 500_000 * BURR).is_ok());
    assert!(w.collect_curve(&mint).is_ok());
    let pot = w.ledger(&mint).pots[0];
    assert!(w.pay_out(&mint, Payee::Platform).is_ok());
    let received = w.burr_of(&platform.pubkey());
    assert_eq!(received, pot, "the platform's half arrives in $BURR");
    // The server's job, as written in the handoff: four fifths burned, the rest to the treasury's standard $BURR account.
    let burned = received * 4 / 5;
    let to_treasury = received - burned;
    let supply = w.b.mint_supply(&BURR_MINT);
    let p = platform.pubkey();
    let ixs = [
        pump::create_ata(&p, &treasury, &BURR_MINT, &TOKEN_2022),
        burr_ix(15, &p, None, burned),
        burr_ix(12, &p, Some(&treasury), to_treasury),
    ];
    let r = w.b.send_as(Some(&platform), &ixs, &[]);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.b.mint_supply(&BURR_MINT), supply - burned, "four fifths burned");
    let account = ata(&treasury, &BURR_MINT, &TOKEN_2022);
    assert_eq!(w.b.token_balance(&account), to_treasury, "one fifth in the treasury's standard $BURR account");
    assert_eq!(w.b.account(&account).unwrap().data.len(), 170);
    assert!(w.b.account(&ata(&treasury, &ZEC, &TOKEN)).is_none(), "and nothing in ZEC");
    assert_eq!(w.burr_of(&p), 0);
}

#[test]
fn a_burr_coin_recovers_a_stray_burr_account_and_never_a_zec_one() {
    let mut w = burr_world();
    let mint = w.launch(&burr_plan());
    let auth = kit::authority(&mint);
    // Someone has fee paid into another $BURR account of the coin's authority (as PumpSwap allows).
    let stray = Keypair::new().pubkey();
    w.b.write_token_account(stray, &BURR_MINT, &auth, 12_345 * BURR, &TOKEN_2022);
    let r = w.recover(&mint, &stray);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.b.token_balance(&stray), 0);
    assert_eq!(w.ledger(&mint).total_split, 12_345 * BURR as u128, "brought home and split");
    assert_shares(&w, &mint);
    // A ZEC account of the same authority is not this coin's quote coin: refused.
    let zec_stray = Keypair::new().pubkey();
    w.b.write_token_account(zec_stray, &ZEC, &auth, 5 * 100_000_000, &TOKEN);
    assert!(w.recover(&mint, &zec_stray).is_err());
    assert_eq!(w.b.token_balance(&zec_stray), 5 * 100_000_000);
}

#[test]
fn before_pump_s_buy_back_receiver_has_a_burr_account_a_burr_coin_s_burn_waits_and_loses_nothing() {
    let mut w = burr_world();
    let mint = w.launch(&burr_plan());
    let t = w.burr_trader(10_000_000 * BURR);
    assert!(w.buy(&t, &mint, 500_000 * BURR).is_ok());
    assert!(w.collect_curve(&mint).is_ok());
    // As on the main network today: the receiver's $BURR account does not exist yet.
    let key = ata(&BUYBACK, &BURR_MINT, &TOKEN_2022);
    let real = w.b.account(&key).unwrap();
    w.b.svm.set_account(key, solana_account::Account::default()).unwrap();
    w.b.wait(60);
    let (vault, pot) = (w.vault_balance(&mint), w.ledger(&mint).pots[3]);
    assert_eq!(custom_code(&w.burn_curve(&mint)), Some(err(E::WrongPumpAccount)));
    assert_eq!((w.vault_balance(&mint), w.ledger(&mint).pots[3]), (vault, pot), "nothing moved");
    // Once it exists (the owner's one-time step), the same burn goes through.
    w.b.svm.set_account(key, real).unwrap();
    let r = w.burn_curve(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(w.ledger(&mint).pots[3] < pot);
}
