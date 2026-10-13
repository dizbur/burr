//! Attacks tried in the second round of the attack audit, kept as guards.

use crate::bench::logs;
use crate::kit::{self, LaunchPlan, World};
use crate::pump::{TOKEN, TOKEN_2022, ZEC};
use solana_pubkey::Pubkey;
use solana_signer::Signer;

const ZEC1: u64 = 100_000_000;

/// A coin's creator, who gets back as much of the fee as the rules can ever
/// allow, buys just before their own coin's buy back and sells just after.
/// Returns (what they gained or lost on the trades, the same after the fee
/// that comes back to them).
fn creator_trades_around_own_burn(fee: u16, attack: u64, pool: bool) -> (i128, i128) {
    let mut w = World::new();
    // The lowest platform share the program allows: a quarter. The creator
    // takes all but a sliver of the rest.
    assert!(w.as_admin(burrpad::instruction::SetPlatformFraction { fraction_bps: 2_500 }).is_ok());
    let attacker = w.trader(100_000 * ZEC1);
    let plan = LaunchPlan {
        fee_bps: fee,
        shares: [9_990, 0, 10, 0],
        creator_receiver: attacker.pubkey(),
        ..Default::default()
    };
    let mint = w.launch(&plan);
    if pool {
        w.graduate(&mint);
    }
    w.b.give_zec(&kit::authority(&mint), 4_000 * ZEC1);
    assert!(w.split(&mint).is_ok());
    w.b.wait(60);
    w.burn_until_it_buys(&mint);
    w.b.wait(60);
    let waiting = |w: &World| if pool { w.pool(&mint).creator_fees } else { w.b.curve(&mint).creator_fee };
    let start = w.zec_of(&attacker.pubkey());
    let f0 = waiting(&w);
    let r = if pool { w.pool_buy(&attacker, &mint, attack) } else { w.buy(&attacker, &mint, attack) };
    assert!(r.is_ok(), "{}", logs(&r));
    let f1 = waiting(&w);
    let r = if pool { w.burn_pool(&mint) } else { w.burn_curve(&mint) };
    assert!(r.is_ok(), "{}", logs(&r));
    let f2 = waiting(&w);
    let coins = w.coins_of(&attacker.pubkey(), &mint);
    let r = if pool { w.pool_sell(&attacker, &mint, coins) } else { w.sell(&attacker, &mint, coins) };
    assert!(r.is_ok(), "{}", logs(&r));
    let f3 = waiting(&w);
    let gain = w.zec_of(&attacker.pubkey()) as i128 - start as i128;
    let paid = ((f1 - f0) + (f3 - f2)) as i128;
    let back = paid * 7_500 * 9_990 / 100_000_000;
    (gain, gain + back)
}

#[test]
fn a_creator_trading_around_their_own_burn_loses_even_with_the_most_fee_back() {
    // With the platform at its floor of a quarter, what the burn hands such a
    // trader and what the creator still pays in fee roughly cancel; what is
    // left is pump.fun's own fee, and that makes it a loss. Today the
    // platform takes a half, which makes the loss far larger.
    for pool in [false, true] {
        for fee in [50u16, 300] {
            for attack in [ZEC1 / 100, ZEC1 / 20, ZEC1 / 2] {
                let (gross, net) = creator_trades_around_own_burn(fee, attack, pool);
                assert!(
                    gross < 0 && net < 0,
                    "pool {pool}, fee {fee}, size {attack}: gained {gross}, {net} with the fee back"
                );
            }
        }
    }
}

#[test]
fn recover_takes_only_a_zec_account_of_this_coins_authority() {
    let mut w = World::new();
    let plan = LaunchPlan { fee_bps: 300, ..Default::default() };
    let a = w.launch(&plan);
    let b = w.launch(&plan);
    w.graduate(&a);
    w.b.give_zec(&kit::authority(&a), 400 * ZEC1);
    assert!(w.split(&a).is_ok());
    let t = w.trader(1_000 * ZEC1);
    assert!(w.pool_buy(&t, &a, 5 * ZEC1).is_ok());

    // The burner's own ZEC account, holding a gift: not the authority's.
    let burner_zec = w.b.give_zec(&kit::burner(&a), 7 * ZEC1);
    assert!(w.recover(&a, &burner_zec).is_err(), "the burner's account was emptied");
    // An account of the authority's, but of another coin's mint.
    let odd = Pubkey::new_unique();
    w.b.write_token_account(odd, &b, &kit::authority(&a), 5, &TOKEN);
    assert!(w.recover(&a, &odd).is_err(), "an account of another mint was accepted");
    // A ZEC-mint account under the other token program, either way round.
    let t22 = Pubkey::new_unique();
    w.b.write_token_account(t22, &ZEC, &kit::authority(&a), 5, &TOKEN_2022);
    assert!(w.recover(&a, &t22).is_err(), "an account of the other token program was accepted");
    let mut acc = w.recover_accounts(&a, &t22);
    acc.token_program = TOKEN_2022;
    assert!(w.b.send(&[kit::raw(acc, burrpad::instruction::Recover {})], &[]).is_err());
    // Another coin's side account: only into its own coin.
    let side_b = Pubkey::new_unique();
    w.b.write_token_account(side_b, &ZEC, &kit::authority(&b), 9 * ZEC1, &TOKEN);
    assert!(w.recover(&a, &side_b).is_err(), "another coin's side account was accepted");
    assert!(w.recover(&b, &side_b).is_ok());
    w.assert_sound(&b);
    assert_eq!(w.b.token_balance(&burner_zec), 7 * ZEC1);
    w.assert_sound(&a);
}

#[test]
fn recover_burn_recover_and_split_in_one_transaction_keep_the_books() {
    let mut w = World::new();
    let a = w.launch(&LaunchPlan { fee_bps: 300, ..Default::default() });
    w.graduate(&a);
    w.b.give_zec(&kit::authority(&a), 400 * ZEC1);
    assert!(w.split(&a).is_ok());
    let t = w.trader(1_000 * ZEC1);
    assert!(w.pool_buy(&t, &a, 5 * ZEC1).is_ok());
    w.b.wait(60);
    w.burn_until_it_buys(&a);
    w.b.wait(60);
    assert!(w.pool_buy(&t, &a, ZEC1 / 100).is_ok());
    let side = Pubkey::new_unique();
    w.b.write_token_account(side, &ZEC, &kit::authority(&a), 3 * ZEC1, &TOKEN);
    let payer = w.b.payer.pubkey();
    let before = w.ledger(&a);
    let ixs = vec![
        kit::raw(w.recover_accounts(&a, &side), burrpad::instruction::Recover {}),
        kit::raw(w.burn_pool_accounts(&payer, &a), burrpad::instruction::BurnPool {}),
        kit::raw(w.recover_accounts(&a, &side), burrpad::instruction::Recover {}),
        w.split_ix(&a),
    ];
    let r = w.b.send(&ixs, &[]);
    assert!(r.is_ok(), "{}", logs(&r));
    let after = w.ledger(&a);
    assert_eq!(
        after.total_split,
        before.total_split + 3 * ZEC1 as u128,
        "the recovered ZEC was counted once, and only once"
    );
    assert_eq!(w.b.token_balance(&side), 0);
    w.assert_sound(&a);
}
