//! Nothing is concluded from a call into pump.fun having succeeded.
//!
//! Here pump.fun's programs are replaced, inside the test network, by a
//! stand-in that reports success while doing the wrong thing. Burrpad must
//! notice from the accounts and balances it reads afterwards.

use crate::bench::{custom_code, logs};
use crate::kit::{self, err, LaunchPlan, World};
use crate::pump::{self, ata, PUMP, PUMP_AMM, TOKEN, TOKEN_2022, ZEC};
use burrpad::error::BurrpadError as E;
use pump_standin as s;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

const ZEC1: u64 = 100_000_000;

#[test]
fn a_launch_is_refused_if_the_coin_made_is_not_the_coin_asked_for() {
    let cases = [
        (s::CREATE_WRONG_CREATOR, E::CurveMismatch, "another creator"),
        (s::CREATE_WRONG_QUOTE, E::CurveMismatch, "another quote coin"),
        (s::CREATE_WRONG_FEE, E::CurveMismatch, "another fee rate"),
        (s::CREATE_HOLDER_REWARDS, E::CurveMismatch, "a holder-rewards coin"),
        (s::CREATE_CASHBACK, E::CurveMismatch, "a cashback coin"),
        (s::CREATE_MAYHEM, E::CurveMismatch, "a mayhem coin"),
        (s::CREATE_COMPLETE, E::CurveMismatch, "a curve already complete"),
        (s::CREATE_RIGHT_CURVE_NO_MINT, E::UnexpectedMint, "the right curve but no proper mint"),
        (s::CREATE_NOTHING, E::WrongPumpAccount, "nothing at all"),
    ];
    for (mode, want, what) in cases {
        let mut w = World::new();
        w.b.use_standin(&PUMP, &pump::global(), mode);
        let (mint, r) = w.try_launch(&LaunchPlan::default());
        assert_eq!(custom_code(&r), Some(err(want)), "pump.fun made {what}\n{}", logs(&r));
        assert!(w.b.account(&kit::coin(&mint)).is_none(), "no record may exist: {what}");
        assert!(w.b.account(&kit::ledger(&mint)).is_none());
    }
}

fn burn_coin() -> (World, Pubkey) {
    let mut w = World::new();
    let mint = w.launch(&LaunchPlan { shares: [0, 0, 10_000, 0], ..Default::default() });
    w.b.give_zec(&kit::authority(&mint), 50 * ZEC1);
    assert!(w.split(&mint).is_ok());
    w.b.wait(60);
    (w, mint)
}

#[test]
fn a_burn_is_undone_if_the_buy_did_not_deliver() {
    let cases = [
        (s::BUY_TAKES_GIVES_NOTHING, E::BurnBadFill, "took the ZEC and gave nothing"),
        (s::BUY_GIVES_ONE_TOO_FEW, E::BurnBadFill, "gave one coin too few"),
        (s::BUY_DOES_NOTHING, E::BurnBadFill, "did nothing"),
        (s::BUY_GIVES_A_CRUMB, E::BurnBadFill, "gave a crumb for the whole amount"),
    ];
    for (mode, want, what) in cases {
        let (mut w, mint) = burn_coin();
        let before = (w.ledger(&mint).pots, w.vault_balance(&mint), w.b.mint_supply(&mint));
        w.b.use_standin(&PUMP, &pump::global(), mode);
        let r = w.burn_curve(&mint);
        assert_eq!(custom_code(&r), Some(err(want)), "the buy {what}\n{}", logs(&r));
        assert_eq!((w.ledger(&mint).pots, w.vault_balance(&mint), w.b.mint_supply(&mint)), before, "{what}");
    }
}

#[test]
fn a_buy_cannot_take_more_than_was_set_aside() {
    let (mut w, mint) = burn_coin();
    // Extra ZEC sits in the burner's account (someone sent it there).
    let bn = kit::burner(&mint);
    let payer = w.b.payer.pubkey();
    assert!(w.b.send(&[pump::create_ata(&payer, &bn, &ZEC, &TOKEN)], &[]).is_ok());
    w.b.give_zec(&bn, 7_777);
    let vault = w.vault_balance(&mint);
    w.b.use_standin(&PUMP, &pump::global(), s::BUY_TAKES_EVERYTHING);
    let r = w.burn_curve(&mint);
    assert_eq!(custom_code(&r), Some(err(E::BurnOverspent)), "{}", logs(&r));
    assert_eq!(w.vault_balance(&mint), vault);
    assert_eq!(w.b.token_balance(&ata(&bn, &ZEC, &TOKEN)), 7_777);
}

#[test]
fn the_stand_in_behaving_properly_is_accepted() {
    // Shows the tests above fail for the reason they say, not because of the stand-in itself.
    let (mut w, mint) = burn_coin();
    let supply = w.b.mint_supply(&mint);
    let pot = w.ledger(&mint).pots[3];
    w.b.use_standin(&PUMP, &pump::global(), s::BUY_HONEST);
    let r = w.burn_curve(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(w.ledger(&mint).pots[3] < pot);
    assert!(w.b.mint_supply(&mint) < supply);
    w.assert_sound(&mint);
}

#[test]
fn collect_credits_only_what_really_arrived() {
    let mut w = World::new();
    let mint = w.launch(&LaunchPlan::default());
    let t = w.trader(100 * ZEC1);
    assert!(w.buy(&t, &mint, ZEC1).is_ok());
    assert!(w.b.curve(&mint).creator_fee > 0);
    // pump.fun's sweep and collect now succeed and move nothing.
    w.b.use_standin(&PUMP, &pump::global(), s::BUY_DOES_NOTHING);
    let r = w.collect_curve(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    let l = w.ledger(&mint);
    assert_eq!((l.accounted, l.pots), (0, [0; 5]), "nothing arrived, so nothing was credited");
}

// ---- the same on the pool ----

fn graduated_burn_coin() -> (World, Pubkey) {
    let (mut w, mint) = burn_coin();
    w.graduate(&mint);
    // With the real pool: the burn follows the price up to where it graduated.
    w.burn_until_it_buys(&mint);
    w.b.wait(60);
    (w, mint)
}

#[test]
fn a_pool_burn_is_undone_if_the_buy_did_not_deliver() {
    let cases = [
        (s::BUY_TAKES_GIVES_NOTHING, E::BurnBadFill),
        (s::BUY_GIVES_ONE_TOO_FEW, E::BurnBadFill),
        (s::BUY_DOES_NOTHING, E::BurnBadFill),
    ];
    for (mode, want) in cases {
        let (mut w, mint) = graduated_burn_coin();
        let before = (w.ledger(&mint).pots, w.vault_balance(&mint), w.b.mint_supply(&mint));
        w.b.use_standin(&PUMP_AMM, &pump::amm_global_config(), mode);
        let r = w.burn_pool(&mint);
        assert_eq!(custom_code(&r), Some(err(want)), "mode {mode}\n{}", logs(&r));
        assert_eq!((w.ledger(&mint).pots, w.vault_balance(&mint), w.b.mint_supply(&mint)), before);
    }
    // Overspending.
    let (mut w, mint) = graduated_burn_coin();
    let bn = kit::burner(&mint);
    let payer = w.b.payer.pubkey();
    assert!(w.b.send(&[pump::create_ata(&payer, &bn, &ZEC, &TOKEN)], &[]).is_ok());
    w.b.give_zec(&bn, 7_777);
    w.b.use_standin(&PUMP_AMM, &pump::amm_global_config(), s::BUY_TAKES_EVERYTHING);
    let r = w.burn_pool(&mint);
    assert_eq!(custom_code(&r), Some(err(E::BurnOverspent)), "{}", logs(&r));
    // And behaving properly.
    let (mut w, mint) = graduated_burn_coin();
    let supply = w.b.mint_supply(&mint);
    w.b.use_standin(&PUMP_AMM, &pump::amm_global_config(), s::BUY_HONEST);
    let r = w.burn_pool(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(w.b.mint_supply(&mint) < supply);
    assert_eq!(w.b.token_balance(&ata(&bn, &mint, &TOKEN_2022)), 0);
    w.assert_sound(&mint);
}
