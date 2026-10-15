//! Facts about pump.fun's programs that the design leans on, each checked
//! against the real copies rather than taken from a document.

use crate::bench::logs;
use crate::kit::{self, LaunchPlan, World};
use crate::pump::{self, ata, TOKEN, TOKEN_2022, ZEC};
use solana_keypair::Keypair;
use solana_signer::Signer;

const ZEC1: u64 = 100_000_000;

fn lp_supply(w: &World, mint: &solana_pubkey::Pubkey) -> u64 {
    let d = w.b.account(&pump::pool(mint)).unwrap().data;
    u64::from_le_bytes(d[203..211].try_into().unwrap())
}

/// A trader who has both coins and ZEC and an account for pool shares.
fn liquidity_provider(w: &mut World, mint: &solana_pubkey::Pubkey) -> Keypair {
    let lp = w.trader(10_000 * ZEC1);
    assert!(w.pool_buy(&lp, mint, 200 * ZEC1).is_ok());
    let mk = pump::create_ata(&lp.pubkey(), &lp.pubkey(), &pump::lp_mint(mint), &TOKEN_2022);
    assert!(w.b.send_as(Some(&lp), &[mk], &[]).is_ok());
    lp
}

#[test]
fn a_launch_can_be_blocked_by_making_the_curves_zec_account_first() {
    // pump.fun's create fails if the curve's ZEC account already exists.
    // Anyone who learns a mint's address before its launch lands can create
    // that account, for a little rent, and that mint can then never launch.
    // Nothing is lost: the launch is undone whole, and a launch with a new
    // mint key works. This is pump.fun's behaviour and is the same for every
    // launchpad built on it; a site must use a fresh, unpublished mint key
    // for each attempt and try again with a new one.
    let mut w = World::new();
    let mint = Keypair::new();
    let payer = w.b.payer.pubkey();
    let curve = pump::curve(&mint.pubkey());
    assert!(w.b.send(&[pump::create_ata(&payer, &curve, &ZEC, &TOKEN)], &[]).is_ok());
    let r = w.launch_with_mint(&mint, &LaunchPlan::default());
    assert!(r.is_err(), "pump.fun now accepts an existing account: update the note in docs/verified.md");
    assert!(w.b.account(&kit::coin(&mint.pubkey())).is_none(), "nothing was left behind");
    assert!(w.b.account(&mint.pubkey()).is_none());
    // A new mint key launches at once.
    w.launch(&LaunchPlan::default());

    // Making our own vault early does not block anything.
    let mint = Keypair::new();
    let early = pump::create_ata(&payer, &kit::authority(&mint.pubkey()), &ZEC, &TOKEN);
    assert!(w.b.send(&[early], &[]).is_ok());
    let r = w.launch_with_mint(&mint, &LaunchPlan::default());
    assert!(r.is_ok(), "{}", logs(&r));
}

#[test]
fn a_stranger_cannot_change_a_coins_creator_with_set_metaplex_creator() {
    // The instruction takes no signer. It reads the creator from the coin's
    // Metaplex metadata; coins made by create_v2 have none.
    let mut w = World::new();
    let mint = w.launch(&LaunchPlan::default());
    let s = w.b.wallet();
    let r = w.b.send_as(Some(&s), &[pump::set_metaplex_creator(&mint)], &[]);
    assert!(r.is_err(), "the call went through");
    assert_eq!(w.b.curve(&mint).creator, kit::authority(&mint));
    // The same after graduation.
    w.graduate(&mint);
    let r = w.b.send_as(Some(&s), &[pump::set_metaplex_creator(&mint)], &[]);
    assert!(r.is_err());
    assert_eq!(w.b.curve(&mint).creator, kit::authority(&mint));
    assert_eq!(w.pool(&mint).coin_creator, kit::authority(&mint));
}

#[test]
fn fees_waiting_in_a_pool_are_not_counted_in_the_reserve_we_read() {
    let mut w = World::new();
    let mint = w.launch(&LaunchPlan::default());
    w.graduate(&mint);
    let t = w.trader(1_000 * ZEC1);
    assert!(w.pool_buy(&t, &mint, 5 * ZEC1).is_ok());
    assert!(w.pool(&mint).creator_fees > 0, "a fee is waiting in the pool's ZEC account");
    let (with_fee_waiting, coins) = w.pool_reserves(&mint);
    // Sweeping the fee out moves ZEC out of the pool's account. The reserve
    // we read (balance plus the pool's own adjustment) must not change.
    assert!(w.collect_pool(&mint).is_ok());
    assert_eq!(w.pool(&mint).creator_fees, 0);
    assert_eq!(w.pool_reserves(&mint), (with_fee_waiting, coins));
}

#[test]
fn adding_liquidity_to_a_pool_is_proportional_and_does_not_move_the_price() {
    let mut w = World::new();
    let mint = w.launch(&LaunchPlan::default());
    w.graduate(&mint);
    let lp = liquidity_provider(&mut w, &mint);
    let (q0, b0) = w.pool_reserves(&mint);
    let supply = lp_supply(&w, &mint);
    let (zec0, coin0) = (w.zec_of(&lp.pubkey()), w.coins_of(&lp.pubkey(), &mint));

    // Ask for half as many pool shares again as exist.
    let r = w.b.send_as(Some(&lp), &[pump::pool_deposit(&mint, &lp.pubkey(), supply / 2, u64::MAX, u64::MAX)], &[]);
    assert!(r.is_ok(), "{}", logs(&r));
    let (q1, b1) = w.pool_reserves(&mint);
    let (paid_zec, paid_coin) = (zec0 - w.zec_of(&lp.pubkey()), coin0 - w.coins_of(&lp.pubkey(), &mint));
    println!("reserves before: {q0} ZEC, {b0} coins; after: {q1}, {b1}; paid {paid_zec} ZEC, {paid_coin} coins");
    assert_eq!(b1 - b0, paid_coin);
    // Coins grew by half. The reserve we read grew, and the price did not move.
    assert!((b1 as u128 * 2).abs_diff(b0 as u128 * 3) <= 1_000);
    assert!(q1 > q0);
    let (p0, p1) = (q0 as u128 * b1 as u128, q1 as u128 * b0 as u128);
    assert!(p0.abs_diff(p1) <= p0 / 100_000, "price moved: {p0} vs {p1}");

    // And it comes back out, for nothing but rounding.
    let shares = w.b.token_balance(&ata(&lp.pubkey(), &pump::lp_mint(&mint), &TOKEN_2022));
    let r = w.b.send_as(Some(&lp), &[pump::pool_withdraw(&mint, &lp.pubkey(), shares)], &[]);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(
        zec0 - w.zec_of(&lp.pubkey()) <= 10,
        "adding and removing liquidity cost {}",
        zec0 - w.zec_of(&lp.pubkey())
    );
}

#[test]
fn a_deposit_takes_zec_by_the_pools_balance_less_the_fees_waiting_in_it() {
    // How PumpSwap's deposit counts each side, pinned to the unit: the coins
    // by the pool's coin balance; the ZEC by its ZEC balance less the creator
    // and protocol fees waiting in that account, with no virtual adjustment.
    // The trading price (which uses the adjustment) does not move.
    for push in [0u64, 20 * ZEC1, 300 * ZEC1] {
        let mut w = World::new();
        let mint = w.launch(&LaunchPlan::default());
        w.graduate(&mint);
        let lp = liquidity_provider(&mut w, &mint);
        if push > 0 {
            let t = w.trader(10_000 * ZEC1);
            assert!(w.pool_buy(&t, &mint, push).is_ok());
        }
        for part in [1_000u64, 100, 10] {
            let p = pump::pool(&mint);
            let zec_balance = w.b.token_balance(&ata(&p, &pump::ZEC, &pump::TOKEN)) as u128;
            let coin_balance = w.b.token_balance(&ata(&p, &mint, &TOKEN_2022)) as u128;
            let raw = w.b.account(&p).unwrap().data;
            let protocol = u64::from_le_bytes(raw[271..279].try_into().unwrap()) as u128;
            let creator = w.pool(&mint).creator_fees as u128;
            assert_ne!(w.pool(&mint).virtual_quote_reserves, 0, "the adjustment is in play");
            let (q0, b0) = w.pool_reserves(&mint);
            let supply = lp_supply(&w, &mint) as u128;
            let out = supply / part as u128;
            let (z0, c0) = (w.zec_of(&lp.pubkey()), w.coins_of(&lp.pubkey(), &mint));
            let r =
                w.b.send_as(Some(&lp), &[pump::pool_deposit(&mint, &lp.pubkey(), out as u64, u64::MAX, u64::MAX)], &[]);
            assert!(r.is_ok(), "{}", logs(&r));
            let zec = (z0 - w.zec_of(&lp.pubkey())) as u128;
            let coins = (c0 - w.coins_of(&lp.pubkey(), &mint)) as u128;
            assert_eq!(zec, (out * (zec_balance - creator - protocol)).div_ceil(supply), "push {push}, 1/{part}: ZEC");
            assert_eq!(coins, (out * coin_balance).div_ceil(supply), "push {push}, 1/{part}: coins");
            let (q1, b1) = w.pool_reserves(&mint);
            let (p0, p1) = (q0 as u128 * b1 as u128, q1 as u128 * b0 as u128);
            assert!(p0.abs_diff(p1) <= p0 / 100_000, "push {push}, 1/{part}: price moved");
        }
    }
}
