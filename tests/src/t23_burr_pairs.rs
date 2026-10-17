//! Coins priced in $BURR instead of ZEC: pump.fun's coin priced in another
//! pump coin, one hop from ZEC. The fee arrives in $BURR and every pot holds
//! $BURR; the four hooks work as for ZEC, the $BURR share is burned as it is,
//! and coins priced in ZEC are untouched.

use crate::bench::{custom_code, logs};
use crate::kit::{self, err, LaunchPlan, World};
use crate::pump::{ata, BUYBACK, TOKEN, TOKEN_2022, ZEC};
use burrpad::error::BurrpadError as E;
use burrpad::state::Payee;
use burrpad::{BURR_MINT, QUOTE_BURR, QUOTE_ZEC};
use solana_instruction::AccountMeta;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

/// Whole $BURR, in units (6 decimals).
const BURR: u64 = 1_000_000;

/// $BURR in place, with pump.fun's buy-back receiver holding a $BURR account
/// (on the main network someone must make one first: see the plan).
fn burr_world() -> World {
    let mut w = World::new();
    w.make_burr();
    w.give_burr(&BUYBACK, 0);
    w
}

fn burr_plan() -> LaunchPlan {
    LaunchPlan { shares: [2_000, 2_000, 2_000, 2_000], burr_share: 2_000, quote: QUOTE_BURR, ..Default::default() }
}

fn burr_of(w: &World, owner: &Pubkey) -> u64 {
    w.b.token_balance(&ata(owner, &BURR_MINT, &TOKEN_2022))
}

/// A coin priced in $BURR, with trades in $BURR on its curve and the fee collected and split.
fn traded() -> (World, Pubkey, Keypair) {
    let mut w = burr_world();
    let mint = w.launch(&burr_plan());
    let trader = w.burr_trader(10_000_000 * BURR);
    let r = w.buy(&trader, &mint, 500_000 * BURR);
    assert!(r.is_ok(), "{}", logs(&r));
    let coins = w.coins_of(&trader.pubkey(), &mint);
    assert!(w.sell(&trader, &mint, coins / 2).is_ok());
    let r = w.collect_curve(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    (w, mint, trader)
}

#[test]
fn a_coin_can_be_priced_in_burr_and_zec_coins_are_untouched() {
    let mut w = burr_world();
    let (mint, r) = w.try_launch(&burr_plan());
    assert!(r.is_ok(), "{}", logs(&r));
    let coin = w.coin(&mint);
    assert_eq!((coin.quote, coin.burr_share), (QUOTE_BURR, 2_000));
    let curve = w.b.curve(&mint);
    assert_eq!(curve.quote_mint, BURR_MINT, "pump.fun prices the coin in $BURR");
    assert_eq!(curve.creator, kit::authority(&mint), "with Burrpad's authority as its creator");
    assert_eq!(curve.creator_fee_bps, 200);
    assert_eq!(w.vault_of(&mint), ata(&kit::authority(&mint), &BURR_MINT, &TOKEN_2022));
    assert!(w.b.account(&w.vault_of(&mint)).is_some(), "the vault is the authority's $BURR account");
    // A coin launched the old ways, and one through launch_v5 in ZEC, are priced in ZEC exactly as before.
    for plan in [
        LaunchPlan::default(),
        LaunchPlan { shares: [2_000; 4], burr_share: 2_000, ..Default::default() },
        LaunchPlan { shares: [2_000; 4], burr_share: 2_000, quote: QUOTE_ZEC, ..Default::default() },
    ] {
        let m = w.launch(&plan);
        assert_eq!(w.coin(&m).quote, QUOTE_ZEC);
        assert_eq!(w.b.curve(&m).quote_mint, ZEC);
        assert_eq!(w.vault_of(&m), kit::vault(&m));
    }
}

#[test]
fn a_launch_must_name_the_quote_and_its_accounts_as_they_are() {
    let mut w = burr_world();
    let payer = w.b.payer.pubkey();
    let cfg = w.config();
    let args = || burrpad::LaunchArgs {
        name: "Test".into(),
        symbol: "TST".into(),
        uri: "https://e.co/t".into(),
        fee_bps: 200,
        shares: [2_500; 4],
        creator_receiver: Pubkey::new_unique(),
        give_receiver: Pubkey::new_unique(),
        max_platform_fraction_bps: cfg.platform_fraction_bps,
        platform_receiver: cfg.platform_receiver,
    };
    type Fix = fn(&mut Vec<AccountMeta>);
    let cases: Vec<(&str, u8, u8, Fix, E)> = vec![
        ("an undefined quote", 2, QUOTE_BURR, |_| {}, E::WrongQuote),
        ("$BURR chosen, ZEC's accounts passed", QUOTE_BURR, QUOTE_ZEC, |_| {}, E::WrongQuote),
        ("ZEC chosen, $BURR's accounts passed", QUOTE_ZEC, QUOTE_BURR, |r| r.truncate(r.len() - 4), E::WrongQuote),
        (
            "$BURR without $BURR's four accounts",
            QUOTE_BURR,
            QUOTE_BURR,
            |r| r.truncate(r.len() - 4),
            E::WrongPumpAccount,
        ),
        (
            "$BURR with a stranger in place of $BURR's pool",
            QUOTE_BURR,
            QUOTE_BURR,
            |r| {
                let n = r.len();
                r[n - 3] = AccountMeta::new_readonly(Pubkey::new_unique(), false);
            },
            E::WrongPumpAccount,
        ),
    ];
    for (what, quote, accounts_for, fix, want) in cases {
        let mint = Keypair::new();
        let mut i = kit::raw(
            w.launch_accounts_for(&payer, &mint.pubkey(), accounts_for),
            burrpad::instruction::LaunchV5 { args: args(), burr_share: 0, quote },
        );
        if accounts_for == QUOTE_BURR {
            i.accounts.extend(World::burr_quote_accounts());
        }
        fix(&mut i.accounts);
        let r = w.send_launch_tabled(i, &mint);
        assert_eq!(custom_code(&r), Some(err(want)), "{what}\n{}", logs(&r));
    }
}

#[test]
fn a_burr_coin_is_launched_with_a_first_buy_within_the_size_limit() {
    let mut w = burr_world();
    w.give_burr(&w.b.payer.pubkey(), 1_000_000 * BURR);
    let mint = Keypair::new();
    let payer = w.b.payer.pubkey();
    let plan = LaunchPlan {
        uri: format!("https://example.com/{}", "x".repeat(80)),
        name: "A name of twenty-five ch".into(),
        ..burr_plan()
    };
    let mut launch = w.launch_ix(&payer, &mint.pubkey(), &plan);
    launch.accounts.push(AccountMeta::new(ata(&payer, &mint.pubkey(), &TOKEN_2022), false));
    let buy = crate::pump::buy_exact_quote_in_v3_q(&mint.pubkey(), &payer, 10_000 * BURR, 1, (BURR_MINT, TOKEN_2022));
    let mut keys: Vec<Pubkey> = vec![launch.program_id, buy.program_id];
    for a in launch.accounts.iter().chain(buy.accounts.iter()) {
        if !a.is_signer && !keys.contains(&a.pubkey) {
            keys.push(a.pubkey);
        }
    }
    let table = w.b.address_table(&keys);
    let p = w.b.payer.insecure_clone();
    let (size, r) = w.b.send_with_table(&p, &[launch, buy], &[&mint], &table);
    assert!(r.is_ok(), "{}", logs(&r));
    println!("a $BURR launch with a first buy and a 100-character link: {size} of 1,232 bytes");
    assert!(size <= crate::bench::MAX_TRANSACTION);
    assert!(w.coins_of(&payer, &mint.pubkey()) > 0, "the launcher bought in the same transaction");
}

#[test]
fn a_burr_coin_s_fee_is_collected_split_and_paid_in_burr() {
    let (mut w, mint, _) = traded();
    let l = w.ledger(&mint);
    assert!(l.total_split > 0, "the creator fee, in $BURR, reached the vault and was split");
    assert_eq!(w.vault_balance(&mint), l.accounted);
    w.assert_sound(&mint);
    let coin = w.coin(&mint);
    for (payee, sink, receiver) in [
        (Payee::Creator, 1, coin.creator_receiver),
        (Payee::Give, 2, coin.give_receiver),
        (Payee::Platform, 0, coin.platform_receiver),
    ] {
        let pot = w.ledger(&mint).pots[sink];
        assert!(pot > 0);
        let before = burr_of(&w, &receiver);
        let r = w.pay_out(&mint, payee);
        assert!(r.is_ok(), "{}", logs(&r));
        assert_eq!(burr_of(&w, &receiver), before + pot, "paid whole, in $BURR, to the receiver's $BURR account");
        assert_eq!(w.b.token_balance(&ata(&receiver, &ZEC, &TOKEN)), 0, "and no ZEC");
        assert_eq!(w.ledger(&mint).pots[sink], 0);
    }
    w.assert_sound(&mint);
}

#[test]
fn a_burr_coin_s_burn_buys_the_coin_with_burr() {
    let (mut w, mint, _) = traded();
    w.b.wait(60);
    let (supply, vault, pot) = (w.b.mint_supply(&mint), w.vault_balance(&mint), w.ledger(&mint).pots[3]);
    let r = w.burn_curve(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    let l = w.ledger(&mint);
    assert!(l.burn.total_spent > 0 && l.burn.total_burned > 0, "it bought and burned");
    assert_eq!(w.b.mint_supply(&mint), supply - l.burn.total_burned);
    assert_eq!(w.vault_balance(&mint), vault - l.burn.total_spent);
    assert_eq!(l.pots[3], pot - l.burn.total_spent);
    w.assert_sound(&mint);
}

#[test]
fn a_burr_coin_s_holders_pot_goes_to_the_rewards_wallet_in_burr() {
    let (mut w, mint, _) = traded();
    w.b.wait(1_800);
    let wallet = w.rewards_wallet.pubkey();
    let pot = w.ledger(&mint).pots[4];
    let r = w.release_holders(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(burr_of(&w, &wallet), pot);
    w.assert_sound(&mint);
}

#[test]
fn a_burr_coin_s_burr_share_is_burned_as_it_is() {
    let (mut w, mint, _) = traded();
    let (supply, vault, pot) = (w.b.mint_supply(&BURR_MINT), w.vault_balance(&mint), w.ledger(&mint).burr.pot);
    assert!(pot > 0);
    // The ZEC coins' buy of $BURR is not for it (here even a ZEC vault someone made for it changes nothing).
    w.b.give_zec(&kit::authority(&mint), 0);
    assert_eq!(custom_code(&w.burn_burr(&mint)), Some(err(E::NotForThisQuote)));
    let r = w.burn_burr_pot(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.b.mint_supply(&BURR_MINT), supply - pot, "exactly the pot, burned");
    assert_eq!(w.vault_balance(&mint), vault - pot);
    let l = w.ledger(&mint);
    assert_eq!((l.burr.pot, l.burr.total_spent, l.burr.total_burned), (0, pot, pot));
    w.assert_sound(&mint);
    // At once again: nothing there.
    assert_eq!(custom_code(&w.burn_burr_pot(&mint)), Some(err(E::NothingToBurn)));
    // Not for a coin priced in ZEC, nor for a $BURR coin without the share.
    let zec_coin = w.launch(&LaunchPlan { shares: [2_000; 4], burr_share: 2_000, ..Default::default() });
    w.b.give_zec(&kit::authority(&zec_coin), 100_000_000);
    assert!(w.split(&zec_coin).is_ok());
    assert!(w.burn_burr_pot(&zec_coin).is_err(), "a ZEC coin has no $BURR vault, and is refused");
    // Even with a $BURR account someone made for its authority and filled.
    w.give_burr(&kit::authority(&zec_coin), 1_000 * BURR);
    assert_eq!(custom_code(&w.burn_burr_pot(&zec_coin)), Some(err(E::NotForThisQuote)));
    let plain = w.launch(&LaunchPlan { quote: QUOTE_BURR, ..Default::default() });
    assert_eq!(custom_code(&w.burn_burr_pot(&plain)), Some(err(E::HookNotChosen)));
}

#[test]
fn a_graduated_burr_coin_collects_and_burns_on_its_burr_pool() {
    let (mut w, mint, trader) = traded();
    w.graduate(&mint);
    assert_eq!(w.pool(&mint).quote_mint, BURR_MINT, "pump.fun's migration made a pool priced in $BURR");
    let r = w.pool_buy(&trader, &mint, 100_000 * BURR);
    assert!(r.is_ok(), "{}", logs(&r));
    let split = w.ledger(&mint).total_split;
    let r = w.collect_pool(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(w.ledger(&mint).total_split > split, "the pool's creator fee, in $BURR, was collected and split");
    w.b.wait(60);
    let spent = w.ledger(&mint).burn.total_spent;
    let r = w.burn_pool(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(w.ledger(&mint).burn.total_spent > spent, "it bought on the pool with $BURR and burned");
    w.assert_sound(&mint);
}

#[test]
fn a_burr_coin_cannot_choose_the_hooks_not_built_for_it_yet() {
    let mut w = burr_world();
    // launch_v5 has no liquidity, shadow or payout arguments: those stay with ZEC coins for now.
    let mint = w.launch(&burr_plan());
    let coin = w.coin(&mint);
    assert_eq!((coin.liquidity_share, coin.shadow_share, coin.payout_options), (0, 0, 0));
    // And the instructions that buy with ZEC refuse a $BURR coin: without a ZEC vault, and with one.
    assert!(w.burn_burr(&mint).is_err());
    w.b.give_zec(&kit::authority(&mint), 0);
    assert_eq!(custom_code(&w.burn_burr(&mint)), Some(err(E::NotForThisQuote)));
}

#[test]
fn a_burr_coin_s_accounts_cannot_be_swapped_for_zec_ones_or_the_other_way() {
    let (mut w, mint, _) = traded();
    let zec_coin = w.launch(&LaunchPlan::default());
    w.b.give_zec(&kit::authority(&zec_coin), 100_000_000);
    // Someone makes and fills each coin's account in the other quote coin, so only the quote checks stand in the way.
    w.b.give_zec(&kit::authority(&mint), 100_000_000);
    w.give_burr(&kit::authority(&zec_coin), 1_000 * BURR);
    let books = |w: &World| (w.ledger(&mint), w.ledger(&zec_coin), w.vault_balance(&mint), w.vault_balance(&zec_coin));
    let before = books(&w);
    // The $BURR coin with ZEC's mint, program and vault; the ZEC coin with $BURR's.
    for (m, q) in [(mint, (ZEC, TOKEN)), (zec_coin, (BURR_MINT, TOKEN_2022))] {
        let auth = kit::authority(&m);
        let split = burrpad::accounts::SplitOnly {
            mint: m,
            coin: kit::coin(&m),
            ledger: kit::ledger(&m),
            authority: auth,
            vault: ata(&auth, &q.0, &q.1),
            zec_mint: q.0,
            token_program: q.1,
        };
        let r = w.b.send(&[kit::raw(split, burrpad::instruction::Split {})], &[]);
        assert_eq!(custom_code(&r), Some(err(E::WrongQuote)), "split of {m}");
        let mut pay = w.pay_out_accounts(&w.b.payer.pubkey(), &m, &w.coin(&m).creator_receiver);
        pay.zec_mint = q.0;
        pay.token_program = q.1;
        pay.vault = ata(&auth, &q.0, &q.1);
        pay.receiver_zec = ata(&w.coin(&m).creator_receiver, &q.0, &q.1);
        let i = kit::raw(pay, burrpad::instruction::PayOut { payee: Payee::Creator });
        assert_eq!(custom_code(&w.b.send(&[i], &[])), Some(err(E::WrongQuote)), "pay out of {m}");
        // Only the mint swapped, or only the program.
        let mut only_mint = w.pay_out_accounts(&w.b.payer.pubkey(), &m, &w.coin(&m).creator_receiver);
        only_mint.zec_mint = q.0;
        let i = kit::raw(only_mint, burrpad::instruction::PayOut { payee: Payee::Creator });
        assert!(w.b.send(&[i], &[]).is_err(), "the mint alone, {m}");
        let mut only_program = w.pay_out_accounts(&w.b.payer.pubkey(), &m, &w.coin(&m).creator_receiver);
        only_program.token_program = q.1;
        let i = kit::raw(only_program, burrpad::instruction::PayOut { payee: Payee::Creator });
        assert!(w.b.send(&[i], &[]).is_err(), "the program alone, {m}");
    }
    let after = books(&w);
    assert_eq!(
        (after.0.accounted, after.1.accounted, after.2, after.3),
        (before.0.accounted, before.1.accounted, before.2, before.3)
    );
}
