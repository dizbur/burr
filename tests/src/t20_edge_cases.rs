//! Edge cases hunted on 15 Oct for the Burn $BURR hook, the Liquidity hook
//! and the payout choices: gifts dropped on the burner, PumpSwap switching
//! deposits or buys off, a coin between its curve completing and its pool
//! existing, dust, a $BURR buy at exactly its full limit, and payout wallets
//! that should never open. See docs/edge-cases-15-oct.md.

use crate::bench::{custom_code, logs};
use crate::kit::{self, err, LaunchPlan, World};
use crate::pump::{self, ata, TOKEN, TOKEN_2022, ZEC};
use burrpad::error::BurrpadError as E;
use burrpad::BURR_MINT;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

const ZEC1: u64 = 100_000_000;

/// A Token-2022 `transfer_checked` of `amount` from `from`'s account to `to`'s.
fn send_2022(w: &mut World, from: &Keypair, mint: &Pubkey, to: &Pubkey, amount: u64) {
    let decimals = w.b.account(mint).unwrap().data[44];
    let mut data = vec![12u8];
    data.extend_from_slice(&amount.to_le_bytes());
    data.push(decimals);
    let i = Instruction {
        program_id: TOKEN_2022,
        accounts: vec![
            AccountMeta::new(ata(&from.pubkey(), mint, &TOKEN_2022), false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new(ata(to, mint, &TOKEN_2022), false),
            AccountMeta::new_readonly(from.pubkey(), true),
        ],
        data,
    };
    let mk = pump::create_ata(&from.pubkey(), to, mint, &TOKEN_2022);
    let r = w.b.send_as(Some(from), &[mk, i], &[]);
    assert!(r.is_ok(), "{}", logs(&r));
}

/// Switches PumpSwap's own flags (deposit 1 << 1, buy 1 << 3), as its administrator could.
fn set_amm_flags(w: &mut World, flags: u8) {
    let key = pump::amm_global_config();
    let mut a = w.b.account(&key).unwrap();
    a.data[56] = flags;
    w.b.svm.set_account(key, a).unwrap();
}

fn burr_world(fee: u64) -> (World, Pubkey) {
    let mut w = World::new();
    w.make_burr();
    let mint = w.launch(&LaunchPlan { shares: [2_000; 4], burr_share: 2_000, ..Default::default() });
    w.b.give_zec(&kit::authority(&mint), fee);
    assert!(w.split(&mint).is_ok());
    w.b.wait(60);
    (w, mint)
}

fn liquidity_world(fee: u64) -> (World, Pubkey) {
    let mut w = World::new();
    let mint = w.launch(&LaunchPlan { shares: [2_000; 4], liquidity_share: 2_000, ..Default::default() });
    w.graduate(&mint);
    w.b.give_zec(&kit::authority(&mint), fee);
    assert!(w.split(&mint).is_ok());
    w.b.wait(60);
    assert!(w.add_liquidity(&mint).is_ok(), "the first call records");
    w.b.wait(60);
    (w, mint)
}

#[test]
fn gifts_to_the_burner_do_not_confuse_a_burr_buy() {
    let (mut w, mint) = burr_world(10 * ZEC1);
    let bn = kit::burner(&mint);
    // Someone sends the burner ZEC and $BURR before the call.
    let giver = w.trader(100 * ZEC1);
    assert!(w.pool_buy(&giver, &BURR_MINT, ZEC1).is_ok());
    let gift_burr = w.coins_of(&giver.pubkey(), &BURR_MINT) / 2;
    send_2022(&mut w, &giver, &BURR_MINT, &bn, gift_burr);
    w.b.give_zec(&bn, 12_345);
    let (vault, pot, supply) = (w.vault_balance(&mint), w.ledger(&mint).burr.pot, w.b.mint_supply(&BURR_MINT));
    let r = w.burn_burr(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    let l = w.ledger(&mint);
    let spent = pot - l.burr.pot;
    // Only what the pot spent is booked; the ZEC gift lands in the vault, unbooked, for the next split.
    assert_eq!(l.burr.total_spent, spent);
    assert_eq!(w.vault_balance(&mint), vault - spent + 12_345);
    // The $BURR gift is burned with what was bought, and counted as burned.
    assert!(supply - w.b.mint_supply(&BURR_MINT) > gift_burr);
    assert_eq!(l.burr.total_burned, supply - w.b.mint_supply(&BURR_MINT));
    assert_eq!(w.b.token_balance(&ata(&bn, &ZEC, &TOKEN)), 0);
    assert_eq!(w.b.token_balance(&ata(&bn, &BURR_MINT, &TOKEN_2022)), 0);
    w.assert_sound(&mint);
}

#[test]
fn gifts_to_the_burner_do_not_confuse_an_add() {
    let (mut w, mint) = liquidity_world(20 * ZEC1);
    let bn = kit::burner(&mint);
    let lp_mint = pump::lp_mint(&mint);
    // Coins, ZEC and pool receipts dropped on the burner.
    let giver = w.trader(10_000 * ZEC1);
    assert!(w.pool_buy(&giver, &mint, 5 * ZEC1).is_ok());
    let coins = w.coins_of(&giver.pubkey(), &mint) / 3;
    let mk = pump::create_ata(&giver.pubkey(), &giver.pubkey(), &lp_mint, &TOKEN_2022);
    assert!(w.b.send_as(Some(&giver), &[mk], &[]).is_ok());
    let supply = {
        let d = w.b.account(&pump::pool(&mint)).unwrap().data;
        u64::from_le_bytes(d[203..211].try_into().unwrap())
    };
    let r = w.b.send_as(
        Some(&giver),
        &[pump::pool_deposit(&mint, &giver.pubkey(), supply / 1_000, u64::MAX, u64::MAX)],
        &[],
    );
    assert!(r.is_ok(), "{}", logs(&r));
    let receipts = w.b.token_balance(&ata(&giver.pubkey(), &lp_mint, &TOKEN_2022));
    send_2022(&mut w, &giver, &mint, &bn, coins);
    send_2022(&mut w, &giver, &lp_mint, &bn, receipts);
    w.b.give_zec(&bn, 54_321);
    let pot = w.ledger(&mint).liquidity.pot;
    let r = w.add_liquidity(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    let l = w.ledger(&mint);
    assert_eq!(l.liquidity.total_spent, pot - l.liquidity.pot, "only the pot's own spending is booked");
    for (what, a) in [
        ("ZEC", ata(&bn, &ZEC, &TOKEN)),
        ("coins", ata(&bn, &mint, &TOKEN_2022)),
        ("receipts", ata(&bn, &lp_mint, &TOKEN_2022)),
    ] {
        assert_eq!(w.b.token_balance(&a), 0, "the burner keeps no {what}");
    }
    assert!(l.liquidity.total_lp_burned >= receipts, "the gifted receipts were burned too");
    w.assert_sound(&mint);
}

#[test]
fn pumpswap_switching_deposits_or_buys_off_leaves_the_pots_waiting() {
    let (mut w, mint) = liquidity_world(20 * ZEC1);
    let (vault, pot) = (w.vault_balance(&mint), w.ledger(&mint).liquidity.pot);
    // Deposits off: the whole call is undone.
    set_amm_flags(&mut w, 1 << 1);
    assert!(w.add_liquidity(&mint).is_err());
    assert_eq!((w.vault_balance(&mint), w.ledger(&mint).liquidity.pot), (vault, pot));
    // Buys off: the same.
    set_amm_flags(&mut w, 1 << 3);
    assert!(w.add_liquidity(&mint).is_err());
    assert_eq!((w.vault_balance(&mint), w.ledger(&mint).liquidity.pot), (vault, pot));
    // Back on: it adds.
    set_amm_flags(&mut w, 0);
    assert!(w.add_liquidity(&mint).is_ok());
    assert!(w.ledger(&mint).liquidity.pot < pot);

    let (mut w, mint) = burr_world(10 * ZEC1);
    let pot = w.ledger(&mint).burr.pot;
    set_amm_flags(&mut w, 1 << 3);
    assert!(w.burn_burr(&mint).is_err());
    assert_eq!(w.ledger(&mint).burr.pot, pot);
    set_amm_flags(&mut w, 0);
    assert!(w.burn_burr(&mint).is_ok());
}

#[test]
fn between_the_curve_completing_and_the_pool_existing_nothing_moves() {
    let mut w = World::new();
    let mint = w.launch(&LaunchPlan { shares: [2_000; 4], liquidity_share: 2_000, ..Default::default() });
    w.b.give_zec(&kit::authority(&mint), 10 * ZEC1);
    assert!(w.split(&mint).is_ok());
    // The curve completes; pump.fun has not yet moved the coin to its pool.
    let whale = w.trader(10_000 * ZEC1);
    assert!(w.buy(&whale, &mint, 20 * ZEC1).is_ok());
    assert!(w.b.curve(&mint).complete);
    w.b.wait(60);
    let (vault, l) = (w.vault_balance(&mint), w.ledger(&mint));
    assert!(w.add_liquidity(&mint).is_err(), "no pool yet");
    assert!(w.burn_curve(&mint).is_err(), "the curve no longer trades");
    assert!(w.burn_pool(&mint).is_err(), "no pool yet");
    assert_eq!(w.vault_balance(&mint), vault);
    assert_eq!((w.ledger(&mint).pots, w.ledger(&mint).liquidity.pot), (l.pots, l.liquidity.pot));
    // Once the pool exists, the first call records and the next adds.
    w.b.svm.airdrop(&pump::WITHDRAW_AUTHORITY, 1_000_000_000).unwrap();
    assert!(w.b.send_as(Some(&whale), &[pump::migrate_v2(&mint, &whale.pubkey())], &[]).is_ok());
    assert!(w.add_liquidity(&mint).is_ok());
    assert_eq!(w.ledger(&mint).liquidity.pot, l.liquidity.pot, "the first call only records");
    w.b.wait(60);
    assert!(w.add_liquidity(&mint).is_ok());
    assert!(w.ledger(&mint).liquidity.pot < l.liquidity.pot);
}

#[test]
fn dust_in_a_pot_is_refused_and_stays() {
    // A liquidity pot of a few units, and a $BURR pot of a few units: refused, kept, nothing moves.
    let mut w = World::new();
    let mint = w.launch(&LaunchPlan { shares: [2_000; 4], liquidity_share: 2_000, ..Default::default() });
    w.graduate(&mint);
    w.b.give_zec(&kit::authority(&mint), 30);
    assert!(w.split(&mint).is_ok());
    w.b.wait(60);
    assert!(w.add_liquidity(&mint).is_ok(), "the first call records");
    w.b.wait(60);
    let (vault, pot) = (w.vault_balance(&mint), w.ledger(&mint).liquidity.pot);
    assert!(pot > 0 && pot < 10, "a pot of {pot}");
    assert!(w.add_liquidity(&mint).is_err());
    assert_eq!((w.vault_balance(&mint), w.ledger(&mint).liquidity.pot), (vault, pot));

    let (mut w, mint) = burr_world(30);
    let pot = w.ledger(&mint).burr.pot;
    assert!(pot > 0 && pot < 10);
    assert!(w.burn_burr(&mint).is_err());
    assert_eq!(w.ledger(&mint).burr.pot, pot);
    w.assert_sound(&mint);
}

#[test]
fn a_burr_buy_at_exactly_its_full_limit_holds_the_next_for_the_whole_minute() {
    // The first limit: 1 ZEC of recorded reserve x the pool's fee / 40,000. A split of ten times that
    // gives a $BURR pot of exactly the limit (a tenth of each split goes to it on this plan).
    let mut w = World::new();
    w.make_burr();
    let fee = w.pool(&BURR_MINT).creator_fee_bps.min(w.config().max_fee_bps as u64);
    let cap = burrpad::BURR_START_RESERVE * fee / 40_000;
    let plan = LaunchPlan { shares: [2_000; 4], burr_share: 2_000, ..Default::default() };
    let (a, b) = (w.launch(&plan), w.launch(&plan));
    for (m, extra) in [(a, 0u64), (b, 10)] {
        w.b.give_zec(&kit::authority(&m), cap * 10 + extra);
        assert!(w.split(&m).is_ok());
    }
    assert_eq!(w.ledger(&a).burr.pot, cap, "a pot of exactly the limit");
    w.b.wait(60);
    let r = w.burn_burr(&a);
    assert!(r.is_ok(), "{}", logs(&r));
    let spent = cap - w.ledger(&a).burr.pot;
    assert!(spent <= cap && spent * 60 > cap * 59, "spent {spent} of {cap}");
    // The next buy, by any coin, waits the whole minute: refused at 59 seconds, accepted at 60.
    w.b.wait(59);
    assert_eq!(custom_code(&w.burn_burr(&b)), Some(err(E::BurnTooSoon)));
    w.b.wait(1);
    assert!(w.burn_burr(&b).is_ok());
    // A first buy from a pot one unit over the limit spends no more than the limit and leaves the unit.
    let mut w = World::new();
    w.make_burr();
    let c = w.launch(&plan);
    w.b.give_zec(&kit::authority(&c), cap * 10 + 10);
    assert!(w.split(&c).is_ok());
    assert_eq!(w.ledger(&c).burr.pot, cap + 1);
    w.b.wait(60);
    assert!(w.burn_burr(&c).is_ok());
    assert!(w.ledger(&c).burr.pot >= 1, "it spent more than the limit");
}

#[test]
fn a_payout_wallet_that_cannot_sign_never_opens_private_payouts() {
    let mut w = World::new();
    // A program address cannot sign, so it can never confirm: private launches stay refused.
    let pda_wallet = kit::config();
    assert!(w.as_admin(burrpad::instruction::SetCreatorPayoutWallet { wallet: pda_wallet }).is_ok());
    let (_, r) = w.try_launch(&LaunchPlan { payout_options: 1, creator_receiver: pda_wallet, ..Default::default() });
    assert_eq!(custom_code(&r), Some(err(E::PrivatePayoutsOff)));
    // A wallet pending as the holders' next wallet cannot be named either.
    let next = w.b.wallet().pubkey();
    assert!(w.as_admin(burrpad::instruction::ProposeRewardsWallet { wallet: next }).is_ok());
    let r = w.as_admin(burrpad::instruction::SetCreatorPayoutWallet { wallet: next });
    assert_eq!(custom_code(&r), Some(err(E::ReceiverNotAllowed)));
    // Until a real wallet confirms, the administrator can still correct the mistake.
    let real = w.b.wallet();
    assert!(w.as_admin(burrpad::instruction::SetCreatorPayoutWallet { wallet: real.pubkey() }).is_ok());
    assert!(w.confirm_payout_wallet(&real).is_ok());
    let (_, r) = w.try_launch(&LaunchPlan { payout_options: 1, creator_receiver: real.pubkey(), ..Default::default() });
    assert!(r.is_ok(), "{}", logs(&r));
}

#[test]
fn a_pool_charging_no_creator_fee_stops_both_buys_and_nothing_is_lost() {
    // The limits are a share of the fee the pool charges: at 0 nothing may be bought.
    let set_fee = |w: &mut World, mint: &Pubkey, bps: u64| {
        let key = pump::pool(mint);
        let mut a = w.b.account(&key).unwrap();
        a.data[261..269].copy_from_slice(&bps.to_le_bytes());
        w.b.svm.set_account(key, a).unwrap();
    };
    let (mut w, mint) = liquidity_world(20 * ZEC1);
    let fee = w.pool(&mint).creator_fee_bps;
    let (vault, pot) = (w.vault_balance(&mint), w.ledger(&mint).liquidity.pot);
    set_fee(&mut w, &mint, 0);
    assert_eq!(custom_code(&w.add_liquidity(&mint)), Some(err(E::NothingToBurn)));
    assert_eq!((w.vault_balance(&mint), w.ledger(&mint).liquidity.pot), (vault, pot));
    set_fee(&mut w, &mint, fee);
    assert!(w.add_liquidity(&mint).is_ok());

    let (mut w, mint) = burr_world(10 * ZEC1);
    let fee = w.pool(&BURR_MINT).creator_fee_bps;
    let pot = w.ledger(&mint).burr.pot;
    set_fee(&mut w, &BURR_MINT, 0);
    assert_eq!(custom_code(&w.burn_burr(&mint)), Some(err(E::NothingToBurn)));
    assert_eq!(w.ledger(&mint).burr.pot, pot);
    set_fee(&mut w, &BURR_MINT, fee);
    assert!(w.burn_burr(&mint).is_ok());
}
