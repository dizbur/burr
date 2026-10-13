//! One coin with every hook, through its whole life: launch, trading,
//! collection, every hook, graduation, and the same again on the pool.
//! Throughout, the coin's record must not change by a single byte, and the
//! vault must always cover the ledger.

use crate::bench::logs;
use crate::kit::{self, LaunchPlan, World};
use burrpad::state::Payee;
use solana_signer::Signer;

const ZEC1: u64 = 100_000_000;

#[test]
fn a_coins_whole_life_and_its_record_never_changes() {
    let mut w = World::new();
    let plan = LaunchPlan::default();
    let mint = w.launch(&plan);
    let record = w.b.account(&kit::coin(&mint)).unwrap().data;
    let check = |w: &World, at: &str| {
        assert_eq!(w.b.account(&kit::coin(&mint)).unwrap().data, record, "the record changed at: {at}");
        w.assert_sound(&mint);
    };

    let (a, b, c) = (w.trader(1_000 * ZEC1), w.trader(1_000 * ZEC1), w.trader(1_000 * ZEC1));
    let admin_moves = |w: &mut World| {
        // The administrator changes every setting it can, mid-life.
        assert!(w.as_admin(burrpad::instruction::SetPlatformFraction { fraction_bps: 5_000 }).is_ok());
        assert!(w.as_admin(burrpad::instruction::SetFeeRange { min_bps: 10, max_bps: 500, default_bps: 100 }).is_ok());
        assert!(w.as_admin(burrpad::instruction::SetPlatformReceiver { receiver: w.admin.pubkey() }).is_ok());
        assert!(w.as_admin(burrpad::instruction::SetLaunchesPaused { paused: true }).is_ok());
    };

    // On the curve.
    assert!(w.buy(&a, &mint, ZEC1 / 2).is_ok());
    assert!(w.buy(&b, &mint, ZEC1 / 4).is_ok());
    let staked = w.coins_of(&a.pubkey(), &mint) / 2;
    assert!(w.stake(&a, &mint, staked).is_ok());
    check(&w, "stake");
    assert!(w.collect_curve(&mint).is_ok());
    check(&w, "collect on the curve");
    admin_moves(&mut w);
    check(&w, "admin changes");
    w.b.wait(60);
    w.burn_until_it_buys(&mint);
    check(&w, "burn on the curve");
    for p in [Payee::Platform, Payee::Creator, Payee::Give] {
        assert!(w.pay_out(&mint, p).is_ok());
    }
    check(&w, "payouts");
    assert!(w.zec_of(&plan.creator_receiver) > 0 && w.zec_of(&plan.give_receiver) > 0);
    assert!(w.zec_of(&w.platform_receiver) > 0, "the platform's part went where the record says");
    assert_eq!(w.zec_of(&w.admin.pubkey()), 0, "not to the receiver set afterwards");

    // Graduation, by pump.fun's own instructions.
    w.graduate(&mint);
    check(&w, "graduation");

    // On the pool.
    assert!(w.pool_buy(&c, &mint, ZEC1).is_ok());
    let have = w.coins_of(&c.pubkey(), &mint);
    assert!(w.pool_sell(&c, &mint, have / 2).is_ok());
    assert!(w.pool_buy(&c, &mint, ZEC1 / 3).is_ok());
    assert!(w.collect_curve(&mint).is_ok());
    let r = w.collect_pool(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    check(&w, "collect on the pool");
    w.b.wait(60);
    let supply = w.b.mint_supply(&mint);
    w.burn_until_it_buys(&mint);
    assert!(w.b.mint_supply(&mint) < supply);
    check(&w, "burn on the pool");
    for p in [Payee::Platform, Payee::Creator, Payee::Give] {
        assert!(w.pay_out(&mint, p).is_ok());
    }
    w.b.wait(86_400);
    let before = w.zec_of(&a.pubkey());
    assert!(w.unstake(&a, &mint, staked).is_ok());
    assert!(w.zec_of(&a.pubkey()) > before, "the staker was paid from both curve and pool fees");
    check(&w, "the end");

    // What is left in the vault is only what the ledger says is still owed.
    let l = w.ledger(&mint);
    assert_eq!(l.pots[0] + l.pots[1] + l.pots[2], 0);
    assert_eq!(w.vault_balance(&mint), l.accounted);
}
