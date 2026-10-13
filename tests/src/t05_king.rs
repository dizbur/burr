//! King of the hill: each round, the largest single buy made through Burr
//! takes the round's pot.

use crate::bench::{custom_code, logs};
use crate::kit::{self, err, LaunchPlan, World};
use crate::pump::{self, ata, TOKEN, ZEC};
use burr::error::BurrError as E;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

const ZEC1: u64 = 100_000_000;
const ROUND: i64 = 3_600;

fn king_plan() -> LaunchPlan {
    LaunchPlan { shares: [0, 0, 0, 0, 10_000], king_round_secs: ROUND as u32, ..Default::default() }
}

fn setup() -> (World, Pubkey) {
    let mut w = World::new();
    let mint = w.launch(&king_plan());
    (w, mint)
}

fn fee(w: &mut World, mint: &Pubkey, amount: u64) {
    w.b.give_zec(&kit::authority(mint), amount);
    assert!(w.split(mint).is_ok());
}

#[test]
fn the_largest_buy_through_burr_leads_and_wins_the_pot() {
    let (mut w, mint) = setup();
    let (a, b, c) = (w.trader(100 * ZEC1), w.trader(100 * ZEC1), w.trader(100 * ZEC1));
    let r = w.king_buy_curve(&a, &mint, ZEC1 / 10);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(w.coins_of(&a.pubkey(), &mint) > 0, "the buyer really bought");
    let k = w.ledger(&mint).king;
    assert_eq!(k.leader, a.pubkey());
    // The size is the ZEC that really left the buyer.
    assert_eq!(k.leader_amount, 100 * ZEC1 - w.zec_of(&a.pubkey()));
    assert_eq!(k.leader_amount, ZEC1 / 10);

    assert!(w.king_buy_curve(&b, &mint, ZEC1 / 5).is_ok());
    assert_eq!(w.ledger(&mint).king.leader, b.pubkey());
    // A smaller buy does not take the lead.
    assert!(w.king_buy_curve(&c, &mint, ZEC1 / 20).is_ok());
    assert_eq!(w.ledger(&mint).king.leader, b.pubkey());

    // The trades above made fee; collect it. Then the round ends.
    assert!(w.collect_curve(&mint).is_ok());
    let pot = w.ledger(&mint).pots[5];
    assert!(pot > 0);
    w.b.wait(ROUND);
    let before = w.zec_of(&b.pubkey());
    let r = w.king_settle(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.zec_of(&b.pubkey()), before + pot);
    let l = w.ledger(&mint);
    assert_eq!(l.pots[5], 0);
    assert_eq!(l.king.leader, Pubkey::default());
    w.assert_sound(&mint);
}

#[test]
fn a_buy_made_elsewhere_does_not_count() {
    let (mut w, mint) = setup();
    let t = w.trader(100 * ZEC1);
    assert!(w.buy(&t, &mint, 5 * ZEC1).is_ok());
    assert_eq!(w.ledger(&mint).king.leader, Pubkey::default());
}

#[test]
fn a_tie_does_not_take_the_lead() {
    let (mut w, mint) = setup();
    let (a, b) = (w.trader(100 * ZEC1), w.trader(100 * ZEC1));
    assert!(w.king_buy_curve(&a, &mint, ZEC1 / 10).is_ok());
    assert!(w.king_buy_curve(&b, &mint, ZEC1 / 10).is_ok());
    assert_eq!(w.ledger(&mint).king.leader, a.pubkey(), "first to reach the amount keeps it");
    assert!(w.king_buy_curve(&b, &mint, ZEC1 / 10 + 1).is_ok());
    assert_eq!(w.ledger(&mint).king.leader, b.pubkey(), "one unit more passes");
}

#[test]
fn a_round_with_no_buys_carries_its_pot_forward() {
    let (mut w, mint) = setup();
    fee(&mut w, &mint, ZEC1);
    let pot = w.ledger(&mint).pots[5];
    // Three rounds pass with nobody buying. There is nothing to settle.
    w.b.wait(3 * ROUND + 5);
    let payer = w.b.payer.pubkey();
    let acc = w.king_settle_accounts(&payer, &mint, &Pubkey::default());
    let r = w.b.send(&[kit::raw(acc, burr::instruction::KingSettle {})], &[]);
    assert_eq!(custom_code(&r), Some(err(E::KingNothingToSettle)), "{}", logs(&r));
    // The first buyer of a later round plays for the whole pot.
    let a = w.trader(100 * ZEC1);
    assert!(w.king_buy_curve(&a, &mint, ZEC1 / 100).is_ok());
    assert_eq!(w.ledger(&mint).king.round, 3);
    assert_eq!(w.ledger(&mint).pots[5], pot);
    w.b.wait(ROUND);
    let before = w.zec_of(&a.pubkey());
    assert!(w.king_settle(&mint).is_ok());
    assert!(w.zec_of(&a.pubkey()) >= before + pot);
}

#[test]
fn a_buy_in_the_last_second_counts_and_one_second_later_does_not() {
    let (mut w, mint) = setup();
    let (a, b, c) = (w.trader(100 * ZEC1), w.trader(100 * ZEC1), w.trader(100 * ZEC1));
    assert!(w.king_buy_curve(&a, &mint, ZEC1 / 10).is_ok());
    // One second before the round ends.
    w.b.wait(ROUND - 1);
    assert!(w.king_buy_curve(&b, &mint, ZEC1 / 5).is_ok());
    assert_eq!(w.ledger(&mint).king.leader, b.pubkey());
    assert_eq!(w.ledger(&mint).king.round, 0);
    // The round is over. A late buy cannot join it: it must be settled first.
    w.b.wait(1);
    let r = w.king_buy_curve(&c, &mint, ZEC1);
    assert_eq!(custom_code(&r), Some(err(E::KingSettleFirst)));
    assert_eq!(w.ledger(&mint).king.leader, b.pubkey());
    assert!(w.king_settle(&mint).is_ok());
    // Now the late buyer leads the new round.
    assert!(w.king_buy_curve(&c, &mint, ZEC1).is_ok());
    let k = w.ledger(&mint).king;
    assert_eq!((k.leader, k.round), (c.pubkey(), 1));
}

#[test]
fn settling_is_open_to_anyone_once_and_only_after_the_round() {
    let (mut w, mint) = setup();
    let a = w.trader(100 * ZEC1);
    assert!(w.king_buy_curve(&a, &mint, ZEC1 / 10).is_ok());
    fee(&mut w, &mint, ZEC1);
    // Not before the round ends.
    let r = w.king_settle(&mint);
    assert_eq!(custom_code(&r), Some(err(E::KingNothingToSettle)));
    w.b.wait(ROUND);
    // A stranger settles; the money goes to the winner, not the caller.
    let s = w.b.wallet();
    let acc = w.king_settle_accounts(&s.pubkey(), &mint, &a.pubkey());
    let before = w.zec_of(&a.pubkey());
    let pot = w.ledger(&mint).pots[5];
    let r = w.b.send_as(Some(&s), &[kit::raw(acc, burr::instruction::KingSettle {})], &[]);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.zec_of(&a.pubkey()), before + pot);
    assert_eq!(w.zec_of(&s.pubkey()), 0);
    // A second settle finds nothing.
    let acc = w.king_settle_accounts(&s.pubkey(), &mint, &a.pubkey());
    let r = w.b.send_as(Some(&s), &[kit::raw(acc, burr::instruction::KingSettle {})], &[]);
    assert!(r.is_err());
    assert_eq!(w.zec_of(&a.pubkey()), before + pot);
}

#[test]
fn the_pot_cannot_be_paid_to_anyone_but_the_leader() {
    let (mut w, mint) = setup();
    let a = w.trader(100 * ZEC1);
    let thief = w.trader(ZEC1);
    assert!(w.king_buy_curve(&a, &mint, ZEC1 / 10).is_ok());
    fee(&mut w, &mint, ZEC1);
    w.b.wait(ROUND);
    // The thief names themselves as winner.
    let acc = w.king_settle_accounts(&thief.pubkey(), &mint, &thief.pubkey());
    let r = w.b.send_as(Some(&thief), &[kit::raw(acc, burr::instruction::KingSettle {})], &[]);
    assert_eq!(custom_code(&r), Some(err(E::KingWrongWinner)));
    // The right winner, the thief's ZEC account.
    let mut acc = w.king_settle_accounts(&thief.pubkey(), &mint, &a.pubkey());
    acc.winner_zec = ata(&thief.pubkey(), &ZEC, &TOKEN);
    let r = w.b.send_as(Some(&thief), &[kit::raw(acc, burr::instruction::KingSettle {})], &[]);
    assert!(r.is_err());
    assert_eq!(w.zec_of(&thief.pubkey()), ZEC1);
}

#[test]
fn fee_collected_after_a_round_ends_goes_to_whoever_settles_it_promptly() {
    // A round's pot is the king pot at the moment it is settled. Anyone can
    // settle as soon as the round ends, and no king buy can happen before.
    let (mut w, mint) = setup();
    let (a, b) = (w.trader(100 * ZEC1), w.trader(100 * ZEC1));
    assert!(w.king_buy_curve(&a, &mint, ZEC1 / 10).is_ok());
    fee(&mut w, &mint, ZEC1);
    w.b.wait(ROUND);
    let pot_at_end = w.ledger(&mint).pots[5];
    // The next buyer settles first, in the same transaction as their buy.
    let ixs = [
        kit::raw(w.king_settle_accounts(&b.pubkey(), &mint, &a.pubkey()), burr::instruction::KingSettle {}),
        w.king_buy_curve_ix(&b.pubkey(), &mint, ZEC1 / 10),
    ];
    let before = w.zec_of(&a.pubkey());
    let r = w.b.send_as(Some(&b), &ixs, &[]);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.zec_of(&a.pubkey()), before + pot_at_end);
    assert_eq!(w.ledger(&mint).king.leader, b.pubkey());
    assert_eq!(w.ledger(&mint).pots[5], 0, "the new round starts from nothing");
}

#[test]
fn taking_the_lead_by_buying_and_selling_at_once_costs_the_fees_both_ways() {
    let (mut w, mint) = setup();
    let a = w.trader(100 * ZEC1);
    let size = ZEC1;
    let start = w.zec_of(&a.pubkey());
    assert!(w.king_buy_curve(&a, &mint, size).is_ok());
    let coins = w.coins_of(&a.pubkey(), &mint);
    assert!(w.sell(&a, &mint, coins).is_ok());
    let cost = start - w.zec_of(&a.pubkey());
    assert_eq!(w.ledger(&mint).king.leader, a.pubkey());
    // On a 2% coin: 2.95% in, 2.95% out, about 5.7% of the size in all.
    let pct_x100 = cost * 10_000 / size;
    assert!((550..=600).contains(&pct_x100), "cost was {pct_x100} hundredths of a percent");
}

#[test]
fn king_buys_need_the_hook_and_cannot_use_another_coins_accounts() {
    let mut w = World::new();
    let plain = w.launch(&LaunchPlan { shares: [10_000, 0, 0, 0, 0], ..Default::default() });
    let a = w.trader(100 * ZEC1);
    let r = w.king_buy_curve(&a, &plain, ZEC1 / 10);
    assert_eq!(custom_code(&r), Some(err(E::HookNotChosen)));

    let mint = w.launch(&king_plan());
    let other = w.launch(&king_plan());
    let thief_zec = ata(&w.trader(ZEC1).pubkey(), &ZEC, &TOKEN);
    let fake = Pubkey::new_unique();
    type Swap = fn(&mut burr::accounts::KingBuyCurve, Pubkey, &Pubkey, Pubkey);
    let swaps: Vec<(&str, Swap)> = vec![
        ("ledger (another coin's)", |x, _, o, _| x.ledger = kit::ledger(o)),
        ("coin (another coin's)", |x, _, o, _| x.coin = kit::coin(o)),
        ("bonding_curve (another coin's)", |x, _, o, _| x.bonding_curve = pump::curve(o)),
        ("pump_program", |x, f, _, _| x.pump_program = f),
        ("pump_global", |x, f, _, _| x.pump_global = f),
        ("fee_config", |x, f, _, _| x.fee_config = f),
        ("user_volume", |x, f, _, _| x.user_volume = f),
        ("buyback_fee_recipient (a thief's)", |x, _, _, t| x.buyback_fee_recipient = t),
        ("buyer_zec (someone else's)", |x, _, _, t| x.buyer_zec = t),
        ("associated_base_bonding_curve", |x, f, _, _| x.associated_base_bonding_curve = f),
        ("associated_quote_bonding_curve", |x, f, _, _| x.associated_quote_bonding_curve = f),
        ("zec_mint", |x, _, o, _| x.zec_mint = *o),
    ];
    for (name, swap) in swaps {
        let mut acc = w.king_buy_curve_accounts(&a.pubkey(), &mint);
        swap(&mut acc, fake, &other, thief_zec);
        let i = kit::raw(acc, burr::instruction::KingBuyCurve { quote_in: ZEC1 / 10, min_out: 1 });
        let r = w.b.send_as(Some(&a), &[i], &[]);
        assert!(r.is_err(), "swapping {name} was accepted");
    }
    assert_eq!(w.ledger(&mint).king.leader, Pubkey::default());
    assert_eq!(w.ledger(&other).king.leader, Pubkey::default());
}

#[test]
fn a_buy_that_buys_nothing_is_not_a_buy() {
    let (mut w, mint) = setup();
    let a = w.trader(100 * ZEC1);
    let r = w.king_buy_curve(&a, &mint, 0);
    assert!(r.is_err());
    assert_eq!(w.ledger(&mint).king.leader, Pubkey::default());
}

#[test]
fn a_winner_cannot_block_settlement_by_giving_their_account_away() {
    // The leader hands their ZEC account to another key, hoping that nobody
    // can settle, the game freezes, and the pot keeps growing for them.
    let (mut w, mint) = setup();
    let a = w.trader(100 * ZEC1);
    let other = w.b.wallet();
    let a_zec = ata(&a.pubkey(), &ZEC, &TOKEN);
    let ixs = [
        w.king_buy_curve_ix(&a.pubkey(), &mint, ZEC1 / 10),
        kit::give_token_account_away(&a_zec, &a.pubkey(), &other.pubkey()),
    ];
    let r = w.b.send_as(Some(&a), &ixs, &[]);
    assert!(r.is_ok(), "{}", logs(&r));
    fee(&mut w, &mint, ZEC1);
    w.b.wait(ROUND);

    // Anyone can still settle. The pot goes to the same address as always.
    let pot = w.ledger(&mint).pots[5];
    let before = w.b.token_balance(&a_zec);
    let r = w.king_settle(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.b.token_balance(&a_zec), before + pot);
    // And the game goes on.
    let b = w.trader(100 * ZEC1);
    assert!(w.king_buy_curve(&b, &mint, ZEC1 / 100).is_ok());
    assert_eq!(w.ledger(&mint).king.leader, b.pubkey());
}

#[test]
fn a_winner_who_closed_their_account_is_still_paid() {
    let (mut w, mint) = setup();
    let a = w.trader(ZEC1 / 10);
    assert!(w.king_buy_curve(&a, &mint, ZEC1 / 10).is_ok());
    // Their ZEC account is now empty; it is removed altogether.
    let a_zec = ata(&a.pubkey(), &ZEC, &TOKEN);
    w.b.svm.set_account(a_zec, solana_account::Account::default()).unwrap();
    fee(&mut w, &mint, ZEC1);
    w.b.wait(ROUND);
    let pot = w.ledger(&mint).pots[5];
    let r = w.king_settle(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.zec_of(&a.pubkey()), pot);
}

#[test]
fn after_a_round_is_settled_any_buy_leads_the_next() {
    let (mut w, mint) = setup();
    let (a, b) = (w.trader(100 * ZEC1), w.trader(100 * ZEC1));
    assert!(w.king_buy_curve(&a, &mint, ZEC1).is_ok());
    w.b.wait(ROUND);
    assert!(w.king_settle(&mint).is_ok());
    assert_eq!(w.ledger(&mint).king.leader_amount, 0);
    // A far smaller buy than last round's winner leads the new round.
    assert!(w.king_buy_curve(&b, &mint, ZEC1 / 1_000).is_ok());
    assert_eq!(w.ledger(&mint).king.leader, b.pubkey());
}

/// What it really costs to take the lead by buying and selling at once, when
/// the fee paid comes back through the pot the washer then wins.
#[test]
fn the_wash_trade_is_cheaper_when_the_washer_wins_their_own_fee_back() {
    // A coin whose whole fee goes to the king pot: the worst case.
    let (mut w, mint) = setup();
    let a = w.trader(100 * ZEC1);
    let size = ZEC1;
    let start = w.zec_of(&a.pubkey());
    assert!(w.king_buy_curve(&a, &mint, size).is_ok());
    let coins = w.coins_of(&a.pubkey(), &mint);
    assert!(w.sell(&a, &mint, coins).is_ok());
    assert!(w.collect_curve(&mint).is_ok());
    w.b.wait(ROUND);
    assert!(w.king_settle(&mint).is_ok());
    let net_cost = start - w.zec_of(&a.pubkey());
    let pct_x100 = net_cost * 10_000 / size;
    // Gross cost is about 5.7%. Seven eighths of the 2% fee, paid twice,
    // comes back: the net cost is about 2.3% of the size.
    assert!((200..=260).contains(&pct_x100), "net cost was {pct_x100} hundredths of a percent");
}
