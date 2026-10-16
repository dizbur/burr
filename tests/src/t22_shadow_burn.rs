//! The eighth hook, the shadow burn: part of a coin's fee buys back and burns
//! $BURR at once when the watched wallets let go of $BURR, with the ZEC the sale
//! received (or the whole pot), never more coins than were let go of, and with
//! only a least-out (the coins at the sale's price, less 35%): no limit, no
//! reserve step, no shared wait.

use crate::bench::{custom_code, logs};
use crate::kit::{self, err, LaunchPlan, World};
use crate::pump::{ata, TOKEN, TOKEN_2022, ZEC};
use burrpad::error::BurrpadError as E;
use burrpad::BURR_MINT;
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

const ZEC1: u64 = 100_000_000;

fn shadow_plan() -> LaunchPlan {
    LaunchPlan { shares: [2_000, 2_000, 2_000, 2_000], shadow_share: 2_000, ..Default::default() }
}

/// Sends `amount` $BURR from `from` to `to`'s account (made if missing).
fn send_burr(w: &mut World, from: &Keypair, to: &Pubkey, amount: u64) {
    let decimals = w.b.account(&BURR_MINT).unwrap().data[44];
    let mut data = vec![12u8];
    data.extend_from_slice(&amount.to_le_bytes());
    data.push(decimals);
    let i = Instruction {
        program_id: TOKEN_2022,
        accounts: vec![
            AccountMeta::new(ata(&from.pubkey(), &BURR_MINT, &TOKEN_2022), false),
            AccountMeta::new_readonly(BURR_MINT, false),
            AccountMeta::new(ata(to, &BURR_MINT, &TOKEN_2022), false),
            AccountMeta::new_readonly(from.pubkey(), true),
        ],
        data,
    };
    let mk = crate::pump::create_ata(&from.pubkey(), to, &BURR_MINT, &TOKEN_2022);
    let r = w.b.send_as(Some(from), &[mk, i], &[]);
    assert!(r.is_ok(), "{}", logs(&r));
}

fn burr_of(w: &World, k: &Pubkey) -> u64 {
    w.b.token_balance(&ata(k, &BURR_MINT, &TOKEN_2022))
}

fn zec_of(w: &World, k: &Pubkey) -> u64 {
    w.b.token_balance(&ata(k, &ZEC, &TOKEN))
}

/// `who` sells `coins` $BURR on its pool; returns the ZEC it received.
fn sell(w: &mut World, who: &Keypair, coins: u64) -> u64 {
    let before = zec_of(w, &who.pubkey());
    let r = w.pool_sell(who, &BURR_MINT, coins);
    assert!(r.is_ok(), "{}", logs(&r));
    zec_of(w, &who.pubkey()) - before
}

struct Shadow {
    w: World,
    mint: Pubkey,
    target: Keypair,
    keeper: Keypair,
    launcher: Keypair,
}

impl Shadow {
    fn burn(&mut self, sold: u64, received: u64) -> crate::bench::TxResult {
        let k = self.keeper.insecure_clone();
        self.w.shadow_burn(&k, &self.mint, sold, received)
    }
    fn watch(&self) -> burrpad::state::ShadowWatch {
        self.w.shadow_watch(&self.mint).unwrap()
    }
    fn pot(&self) -> u64 {
        self.w.ledger(&self.mint).shadow.pot
    }
    fn held(&self) -> u64 {
        let x = self.watch();
        x.held[..x.count as usize].iter().sum()
    }
    fn follow(&mut self, from: u8, wallet: &Pubkey) -> crate::bench::TxResult {
        let k = self.keeper.insecure_clone();
        self.w.follow_wallet(&k, &self.mint, from, wallet)
    }
    fn set(&mut self, wallets: &[Pubkey], follow: bool) -> crate::bench::TxResult {
        let (l, k) = (self.launcher.insecure_clone(), self.keeper.pubkey());
        self.w.set_shadow_watch(&l, &self.mint, wallets, k, follow)
    }
}

/// $BURR in place; a coin that chose the shadow burn, with `fee` split; a target
/// holding $BURR, watched by the launcher's watch, with a keeper allowed to follow.
fn setup(fee: u64) -> Shadow {
    let mut w = World::new();
    w.make_burr();
    let target = w.trader(1_000 * ZEC1);
    assert!(w.pool_buy(&target, &BURR_MINT, 5 * ZEC1).is_ok());
    let mint = w.launch(&shadow_plan());
    let launcher = w.b.payer.insecure_clone();
    let keeper = w.b.wallet();
    let mut s = Shadow { w, mint, target, keeper, launcher };
    let t = s.target.pubkey();
    let r = s.set(&[t], true);
    assert!(r.is_ok(), "{}", logs(&r));
    s.w.b.give_zec(&kit::authority(&mint), fee);
    assert!(s.w.split(&mint).is_ok());
    s
}

#[test]
fn a_launch_can_give_part_of_the_fee_to_a_shadow_burn() {
    let mut w = World::new();
    let mint = w.launch(&shadow_plan());
    assert_eq!(w.coin(&mint).shadow_share, 2_000);
    w.b.give_zec(&kit::authority(&mint), 10 * ZEC1);
    assert!(w.split(&mint).is_ok());
    let l = w.ledger(&mint);
    assert_eq!(l.pots[1..5], [ZEC1; 4]);
    assert_eq!((l.shadow.pot, l.shadow.credited), (ZEC1, ZEC1 as u128));
    w.assert_sound(&mint);
    // All seven shares add to the whole; old launches record no share.
    let (_, r) = w.try_launch(&LaunchPlan { shares: [2_000; 4], shadow_share: 1_999, ..Default::default() });
    assert_eq!(custom_code(&r), Some(err(E::SharesDoNotAddUp)));
    let old = w.launch(&LaunchPlan::default());
    assert_eq!(w.coin(&old).shadow_share, 0);
}

#[test]
fn only_the_launcher_sets_the_watch_and_only_sensible_lists() {
    let mut w = World::new();
    w.make_burr();
    let mint = w.launch(&shadow_plan());
    let launcher = w.b.payer.insecure_clone();
    let stranger = w.b.wallet();
    let t = Keypair::new().pubkey();
    assert_eq!(
        custom_code(&w.set_shadow_watch(&stranger, &mint, &[t], Pubkey::default(), false)),
        Some(err(E::NotTheLauncher))
    );
    for (what, list) in [
        ("empty", vec![]),
        ("repeated", vec![t, t]),
        ("seventeen", (0..17).map(|_| Keypair::new().pubkey()).collect()),
        ("the empty address", vec![Pubkey::default()]),
    ] {
        let r = w.set_shadow_watch(&launcher, &mint, &list, Pubkey::default(), false);
        assert_eq!(custom_code(&r), Some(err(E::BadWatchList)), "{what}");
    }
    // A derived address (a pool, a vault) is not a wallet anyone sells from.
    let pool = crate::pump::pool(&BURR_MINT);
    let r = w.set_shadow_watch(&launcher, &mint, &[pool], Pubkey::default(), false);
    assert_eq!(custom_code(&r), Some(err(E::NotAPlainWallet)));
    // Sixteen fit, with an address table.
    let sixteen: Vec<Pubkey> = (0..16).map(|_| Keypair::new().pubkey()).collect();
    let r = w.set_shadow_watch(&launcher, &mint, &sixteen, Pubkey::default(), false);
    assert!(r.is_ok(), "{}", logs(&r));
    // A coin that did not choose the hook has no watch.
    let old = w.launch(&LaunchPlan::default());
    assert_eq!(
        custom_code(&w.set_shadow_watch(&launcher, &old, &[t], Pubkey::default(), false)),
        Some(err(E::HookNotChosen))
    );
    // Someone else's account in place of a watched wallet's.
    let acc = burrpad::accounts::SetShadowWatch {
        launcher: launcher.pubkey(),
        mint,
        coin: kit::coin(&mint),
        watch: kit::shadow(&mint),
        system_program: crate::pump::SYSTEM,
    };
    let mut i = kit::raw(
        acc,
        burrpad::instruction::SetShadowWatch { wallets: vec![t], keeper: Pubkey::default(), follow: false },
    );
    i.accounts.extend(World::watched_accounts(&sixteen));
    i.accounts.push(AccountMeta::new_readonly(ata(&Pubkey::new_unique(), &BURR_MINT, &TOKEN_2022), false));
    assert_eq!(custom_code(&w.send_tabled(&launcher, i)), Some(err(E::WrongWatchedAccount)));
    assert!(w.set_shadow_watch(&launcher, &mint, &[t], Pubkey::default(), true).is_ok());
    // Following is off without a keeper.
    assert!(!w.shadow_watch(&mint).unwrap().follow);
}

#[test]
fn with_nothing_let_go_of_a_shadow_burn_only_records() {
    let mut s = setup(10 * ZEC1);
    let pot = s.pot();
    let r = s.burn(0, 0);
    assert!(r.is_ok(), "{}", logs(&r));
    // A reported sale with nothing recorded as let go of buys nothing.
    assert!(s.burn(1_000_000, ZEC1).is_ok());
    assert_eq!(s.pot(), pot, "nothing bought");
    assert_eq!(s.watch().owed, 0);
}

#[test]
fn only_the_keeper_buys_back() {
    let mut s = setup(10 * ZEC1);
    let t = s.target.insecure_clone();
    let sold = burr_of(&s.w, &t.pubkey()) / 50;
    let got = sell(&mut s.w, &t, sold);
    let stranger = s.w.b.wallet();
    let launcher = s.launcher.insecure_clone();
    for who in [&stranger, &launcher] {
        let r = s.w.shadow_burn(who, &s.mint, sold, got);
        assert_eq!(custom_code(&r), Some(err(E::NotTheKeeper)));
    }
    assert!(s.burn(sold, got).is_ok());
}

#[test]
fn a_sale_is_bought_back_at_once_with_what_it_received_never_more() {
    let mut s = setup(50 * ZEC1);
    let t = s.target.insecure_clone();
    let sold = burr_of(&s.w, &t.pubkey()) / 50;
    let got = sell(&mut s.w, &t, sold);
    let supply = s.w.b.mint_supply(&BURR_MINT);
    let (vault, pot) = (s.w.vault_balance(&s.mint), s.pot());
    let r = s.burn(sold, got);
    assert!(r.is_ok(), "{}", logs(&r));
    let watch = s.watch();
    let spent = pot - s.pot();
    let burned = supply - s.w.b.mint_supply(&BURR_MINT);
    assert!(spent <= got && spent > got - got / 100, "spent {spent}: the ZEC the sale received, {got}");
    assert!(burned <= sold, "burned {burned}, more than the {sold} sold");
    assert!(burned >= sold - sold / 20, "burned {burned}: nearly all the {sold} sold, at an unmoved price");
    assert_eq!(watch.owed, sold - burned, "what was bought came off what is owed");
    assert_eq!((watch.total_spent, watch.total_burned), (spent, burned));
    assert_eq!(s.w.vault_balance(&s.mint), vault - spent);
    let bn = kit::burner(&s.mint);
    assert_eq!(s.w.b.token_balance(&ata(&bn, &ZEC, &TOKEN)), 0);
    assert_eq!(s.w.b.token_balance(&ata(&bn, &BURR_MINT, &TOKEN_2022)), 0);
    s.w.assert_sound(&s.mint);
    // Sales one after another are each bought back at once: no wait.
    for _ in 0..3 {
        let more = burr_of(&s.w, &t.pubkey()) / 50;
        let got = sell(&mut s.w, &t, more);
        let pot = s.pot();
        let r = s.burn(more, got);
        assert!(r.is_ok(), "{}", logs(&r));
        assert!(s.pot() < pot, "bought straight away");
    }
    assert_eq!(s.held(), burr_of(&s.w, &t.pubkey()));
    s.w.assert_sound(&s.mint);
}

#[test]
fn no_limit_no_reserve_step_and_no_shared_wait() {
    let mut s = setup(500 * ZEC1);
    // Another coin's $BURR buy has just started the shared wait.
    let burr_coin = s.w.launch(&LaunchPlan { shares: [2_000; 4], burr_share: 2_000, ..Default::default() });
    s.w.b.give_zec(&kit::authority(&burr_coin), 10 * ZEC1);
    assert!(s.w.split(&burr_coin).is_ok());
    s.w.b.wait(60);
    assert!(s.w.burn_burr(&burr_coin).is_ok());
    let shared = s.w.config().burr_buy;
    // A sale far bigger than a $BURR buy's limit, bought back inside the wait, in one buy.
    let t = s.target.insecure_clone();
    let sold = burr_of(&s.w, &t.pubkey()) / 2;
    let got = sell(&mut s.w, &t, sold);
    let pot = s.pot();
    let r = s.burn(sold, got);
    assert!(r.is_ok(), "{}", logs(&r));
    let spent = pot - s.pot();
    assert!(spent > got - got / 100, "spent {spent} of the {got} received, in one buy");
    let after = s.w.config().burr_buy;
    assert_eq!(
        (after.ready_at, after.reference.quote, after.reference.base),
        (shared.ready_at, shared.reference.quote, shared.reference.base),
        "the shared wait and recorded reserve are untouched"
    );
    // And the next $BURR buy is not held by it.
    s.w.b.give_zec(&kit::authority(&burr_coin), 10 * ZEC1);
    assert!(s.w.split(&burr_coin).is_ok());
    s.w.b.wait(60);
    let r = s.w.burn_burr(&burr_coin);
    assert!(r.is_ok(), "{}", logs(&r));
}

#[test]
fn a_dump_larger_than_the_pot_spends_the_whole_pot_and_the_rest_waits() {
    let mut s = setup(ZEC1);
    let t = s.target.insecure_clone();
    let sold = burr_of(&s.w, &t.pubkey()) / 2;
    let got = sell(&mut s.w, &t, sold);
    let pot = s.pot();
    assert!(got > 10 * pot);
    let r = s.burn(sold, got);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(s.pot(), 0, "the whole pot, in one buy");
    let first = s.watch();
    assert!(first.owed > 0 && first.owed < sold);
    // The pot fills again; the keeper reports the same sale; the rest is bought at its share of the price.
    s.w.b.give_zec(&kit::authority(&s.mint), ZEC1);
    assert!(s.w.split(&s.mint).is_ok());
    let r = s.burn(sold, got);
    assert!(r.is_ok(), "{}", logs(&r));
    let watch = s.watch();
    assert!(watch.owed < first.owed);
    assert_eq!(watch.owed + watch.total_burned, sold, "every coin sold is burned or still owed");
    s.w.assert_sound(&s.mint);
    // Others buy the price back up past the sale's: the rest is not bought dearer than the target sold.
    let (quote, _) = s.w.pool_reserves(&BURR_MINT);
    let bull = s.w.trader(10 * quote);
    assert!(s.w.pool_buy(&bull, &BURR_MINT, 2 * quote).is_ok());
    s.w.b.give_zec(&kit::authority(&s.mint), 100 * ZEC1);
    assert!(s.w.split(&s.mint).is_ok());
    let pot = s.pot();
    let r = s.burn(sold, got);
    assert!(logs(&r).contains("BuySlippageBelowMinBaseAmountOut"), "refused by the least-out");
    assert_eq!(s.pot(), pot);
}

#[test]
fn a_report_of_more_coins_than_are_owed_pays_only_their_share() {
    let mut s = setup(50 * ZEC1);
    let t = s.target.insecure_clone();
    let giver = s.w.trader(10 * ZEC1);
    assert!(s.w.pool_buy(&giver, &BURR_MINT, ZEC1).is_ok());
    let sold = burr_of(&s.w, &t.pubkey()) / 50;
    let got = sell(&mut s.w, &t, sold);
    // Someone gives the target half as many coins back before the keeper acts: only half is owed.
    send_burr(&mut s.w, &giver, &t.pubkey(), sold / 2);
    let pot = s.pot();
    let r = s.burn(sold, got);
    assert!(r.is_ok(), "{}", logs(&r));
    let owed = sold - sold / 2;
    let share = (got as u128 * owed as u128 / sold as u128) as u64;
    let spent = pot - s.pot();
    assert!(
        spent <= share && spent > share - share / 100,
        "spent {spent}: the sale's share for the coins owed, {share}"
    );
}

/// After the same sale in a fresh world, the coins `zec` buys on $BURR's pool.
fn coins_bought_for(zec: u64) -> u64 {
    let mut s = setup(50 * ZEC1);
    let t = s.target.insecure_clone();
    let sold = burr_of(&s.w, &t.pubkey()) / 20;
    sell(&mut s.w, &t, sold);
    let buyer = s.w.trader(10 * ZEC1);
    let before = burr_of(&s.w, &buyer.pubkey());
    assert!(s.w.pool_buy(&buyer, &BURR_MINT, zec).is_ok());
    burr_of(&s.w, &buyer.pubkey()) - before
}

#[test]
fn the_least_out_allows_35_percent_short_and_no_more() {
    let mut s = setup(50 * ZEC1);
    let t = s.target.insecure_clone();
    let sold = burr_of(&s.w, &t.pubkey()) / 20;
    let got = sell(&mut s.w, &t, sold);
    // The ZEC spent buys `coins`; a report of more coins for it says the price moved against the buy.
    let spend = got / 2;
    let coins = coins_bought_for(spend);
    // 36% fewer coins than the reported price gives: refused, nothing spent.
    let pot = s.pot();
    let r = s.burn(coins * 100 / 64, spend);
    assert!(logs(&r).contains("BuySlippageBelowMinBaseAmountOut"), "36% short is refused");
    assert_eq!(s.pot(), pot);
    // 34% fewer: it fills.
    let r = s.burn(coins * 100 / 66, spend);
    assert!(r.is_ok(), "34% short fills: {}", logs(&r));
    assert_eq!(pot - s.pot(), spend);
    assert_eq!(s.watch().total_burned, coins, "the same coins as the same buy in a fresh world");
}

#[test]
fn a_trade_around_the_instant_buy_is_refused() {
    let mut s = setup(50 * ZEC1);
    let t = s.target.insecure_clone();
    let sold = burr_of(&s.w, &t.pubkey()) / 20;
    let got = sell(&mut s.w, &t, sold);
    // Someone buys just before the keeper, to sell into the pot's buy.
    let (quote, _) = s.w.pool_reserves(&BURR_MINT);
    let front = s.w.trader(quote);
    let before = burr_of(&s.w, &front.pubkey());
    assert!(s.w.pool_buy(&front, &BURR_MINT, quote / 3).is_ok());
    let pot = s.pot();
    let r = s.burn(sold, got);
    assert!(
        logs(&r).contains("BuySlippageBelowMinBaseAmountOut"),
        "a price moved by the trade gives too few coins: refused by the least-out"
    );
    assert_eq!(s.pot(), pot, "nothing spent");
    // Once the price is back, the same report buys.
    let bought = burr_of(&s.w, &front.pubkey()) - before;
    sell(&mut s.w, &front, bought);
    let r = s.burn(sold, got);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(s.pot() < pot);
    // A keeper reporting far more ZEC than the sale received still buys no more coins than were sold.
    let more = burr_of(&s.w, &t.pubkey()) / 20;
    let got = sell(&mut s.w, &t, more);
    let owed = s.watch().owed + more;
    let burned = s.watch().total_burned;
    let r = s.burn(more, got * 100);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(s.watch().total_burned - burned <= owed, "held to what is owed");
}

#[test]
fn coins_moved_between_watched_wallets_count_for_nothing_sent_away_count() {
    let mut s = setup(10 * ZEC1);
    let second = s.w.b.wallet();
    let t = s.target.insecure_clone();
    assert!(s.set(&[t.pubkey(), second.pubkey()], false).is_ok());
    let held = burr_of(&s.w, &t.pubkey());
    send_burr(&mut s.w, &t, &second.pubkey(), held / 3);
    assert!(s.burn(0, 0).is_ok());
    assert_eq!(s.watch().owed, 0, "a move between watched wallets is not a fall");
    // Sent to an unwatched wallet: a fall at once.
    send_burr(&mut s.w, &t, &Pubkey::new_unique(), 1_000_000);
    assert!(s.burn(0, 0).is_ok());
    assert_eq!(s.watch().owed, 1_000_000);
}

#[test]
fn a_change_of_wallets_keeps_what_is_owed_and_starts_the_new_ones_from_now() {
    let mut s = setup(10 * ZEC1);
    let t = s.target.insecure_clone();
    // The target lets go of some, then the launcher switches to a new wallet before any burn.
    send_burr(&mut s.w, &t, &Pubkey::new_unique(), 2_000_000);
    let other = s.w.trader(100 * ZEC1);
    assert!(s.w.pool_buy(&other, &BURR_MINT, ZEC1).is_ok());
    assert!(s.set(&[other.pubkey()], false).is_ok());
    assert_eq!(s.watch().owed, 2_000_000, "the old wallet's fall was recorded before the change");
    assert_eq!(s.held(), burr_of(&s.w, &other.pubkey()), "the new wallet starts from its holding now");
    // The old wallet's later sales no longer count; the new one's do.
    send_burr(&mut s.w, &t, &Pubkey::new_unique(), 3_000_000);
    send_burr(&mut s.w, &other, &Pubkey::new_unique(), 1_000_000);
    assert!(s.burn(0, 0).is_ok());
    assert_eq!(s.watch().owed, 3_000_000);
}

#[test]
fn coins_sent_away_are_bought_back_at_the_keepers_price_held_to_the_pool() {
    let mut s = setup(200 * ZEC1);
    let t = s.target.insecure_clone();
    let gone = burr_of(&s.w, &t.pubkey()) / 50;
    send_burr(&mut s.w, &t, &Pubkey::new_unique(), gone);
    // No ZEC was received: the keeper reports the coins' worth at the pool's price.
    let (quote, base) = s.w.pool_reserves(&BURR_MINT);
    let worth = (quote as u128 * gone as u128 / base as u128) as u64;
    let pot = s.pot();
    let supply = s.w.b.mint_supply(&BURR_MINT);
    let r = s.burn(gone, worth);
    assert!(r.is_ok(), "{}", logs(&r));
    let burned = supply - s.w.b.mint_supply(&BURR_MINT);
    assert!(burned <= gone && burned >= gone - gone / 20, "burned {burned} of {gone}, spent {}", pot - s.pot());
    assert!(pot - s.pot() <= worth, "never more than reported");
    assert!(s.pot() > pot / 2, "the rest of a pot bigger than the fall waits");
}

#[test]
fn the_target_buying_back_in_moves_the_record_and_keeps_what_is_owed() {
    let mut s = setup(10 * ZEC1);
    let t = s.target.insecure_clone();
    send_burr(&mut s.w, &t, &Pubkey::new_unique(), 4_000_000);
    assert!(s.burn(0, 0).is_ok());
    assert_eq!(s.watch().owed, 4_000_000);
    assert!(s.w.pool_buy(&t, &BURR_MINT, ZEC1).is_ok());
    assert!(s.burn(0, 0).is_ok());
    assert_eq!(s.held(), burr_of(&s.w, &t.pubkey()), "the record rose with the holding");
    assert_eq!(s.watch().owed, 4_000_000, "what was owed stays owed");
}

#[test]
fn the_keeper_follows_plain_wallets_the_target_sends_to_and_only_those() {
    let mut s = setup(50 * ZEC1);
    let t = s.target.insecure_clone();
    let held = burr_of(&s.w, &t.pubkey());
    // A fresh wallet that never held SOL: the target sends it coins, and the keeper adds it at once.
    let b = Keypair::new();
    send_burr(&mut s.w, &t, &b.pubkey(), held / 10);
    let stranger = s.w.b.wallet();
    assert_eq!(custom_code(&s.w.follow_wallet(&stranger, &s.mint, 0, &b.pubkey())), Some(err(E::NotTheKeeper)));
    let r = s.follow(0, &b.pubkey());
    assert!(r.is_ok(), "{}", logs(&r));
    let watch = s.watch();
    assert_eq!(watch.count, 2);
    assert_eq!(watch.held[1], held / 10, "its holding moved from the sender's record to its own");
    assert!(s.burn(0, 0).is_ok());
    assert_eq!(s.watch().owed, 0, "the move itself is not a fall");
    // Its own later sale counts, and is bought back at once.
    s.w.b.svm.airdrop(&b.pubkey(), 1_000_000_000).unwrap();
    s.w.b.give_zec(&b.pubkey(), 1);
    let got = sell(&mut s.w, &b, held / 20);
    let r = s.burn(held / 20, got);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(s.watch().total_burned > 0);
    // Rapid follow-ups: two more wallets in the same minute.
    for _ in 0..2 {
        let c = Keypair::new();
        send_burr(&mut s.w, &t, &c.pubkey(), 1_000_000);
        let r = s.follow(0, &c.pubkey());
        assert!(r.is_ok(), "{}", logs(&r));
    }
    // A forwarding hop: a followed wallet's own transfer is followed too.
    let d = Keypair::new();
    send_burr(&mut s.w, &b, &d.pubkey(), 500_000);
    assert!(s.follow(1, &d.pubkey()).is_ok());
    assert_eq!(s.watch().count, 5);
    assert!(s.burn(0, 0).is_ok());
    let owed = s.watch().owed;
    // Not traced: nothing let go of since the last record, or more held than was let go of.
    let e = s.w.trader(10 * ZEC1);
    assert!(s.w.pool_buy(&e, &BURR_MINT, ZEC1).is_ok());
    assert_eq!(custom_code(&s.follow(0, &e.pubkey())), Some(err(E::NotTraced)), "nothing let go of");
    send_burr(&mut s.w, &t, &e.pubkey(), 1_000);
    assert_eq!(custom_code(&s.follow(0, &e.pubkey())), Some(err(E::NotTraced)), "it held coins of its own");
    // The wrong sender named.
    let f = Keypair::new();
    send_burr(&mut s.w, &t, &f.pubkey(), 1_000);
    assert_eq!(custom_code(&s.follow(2, &f.pubkey())), Some(err(E::NotTraced)), "wallet 2 sent nothing");
    assert_eq!(custom_code(&s.follow(9, &f.pubkey())), Some(err(E::BadWatchList)), "no wallet 9");
    // Never a wallet already watched.
    assert_eq!(custom_code(&s.follow(0, &b.pubkey())), Some(err(E::BadWatchList)));
    // Never a derived address, nor an account a program owns.
    let vault_owner = kit::authority(&s.mint);
    send_burr(&mut s.w, &t, &vault_owner, 1_000);
    assert_eq!(custom_code(&s.follow(0, &vault_owner)), Some(err(E::NotAPlainWallet)), "derived");
    let owned = Keypair::new().pubkey();
    s.w.b
        .svm
        .set_account(
            owned,
            Account { lamports: 10_000_000, data: vec![1; 8], owner: TOKEN, executable: false, rent_epoch: 0 },
        )
        .unwrap();
    send_burr(&mut s.w, &t, &owned, 1_000);
    assert_eq!(custom_code(&s.follow(0, &owned)), Some(err(E::NotAPlainWallet)), "program-owned");
    for (what, data, owner) in
        [("program-owned, empty", vec![], TOKEN), ("holding data", vec![1; 8], crate::pump::SYSTEM)]
    {
        let k = Keypair::new().pubkey();
        let a = Account { lamports: 10_000_000, data, owner, executable: false, rent_epoch: 0 };
        s.w.b.svm.set_account(k, a).unwrap();
        send_burr(&mut s.w, &t, &k, 1_000);
        assert_eq!(custom_code(&s.follow(0, &k)), Some(err(E::NotAPlainWallet)), "{what}");
    }
    assert_eq!(custom_code(&s.follow(0, &Keypair::new().pubkey())), Some(err(E::NotTraced)), "holding nothing");
    assert!(s.follow(0, &f.pubkey()).is_ok(), "the right sender");
    assert!(s.watch().owed >= owed);
    s.w.assert_sound(&s.mint);
}

#[test]
fn the_keeper_never_follows_past_sixteen_and_the_launcher_can_stop_it() {
    let mut s = setup(50 * ZEC1);
    let t = s.target.insecure_clone();
    for _ in 0..15 {
        let c = Keypair::new();
        send_burr(&mut s.w, &t, &c.pubkey(), 1_000_000);
        let r = s.follow(0, &c.pubkey());
        assert!(r.is_ok(), "{}", logs(&r));
    }
    assert_eq!(s.watch().count, 16);
    let c = Keypair::new();
    send_burr(&mut s.w, &t, &c.pubkey(), 1_000_000);
    assert_eq!(custom_code(&s.follow(0, &c.pubkey())), Some(err(E::BadWatchList)), "a seventeenth");
    // With sixteen watched, a buy back still fits one transaction (with an address table).
    let sold = burr_of(&s.w, &t.pubkey()) / 20;
    let got = sell(&mut s.w, &t, sold);
    let r = s.burn(sold, got);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(s.watch().total_burned > 0);
    // The launcher turns following off: the keeper can add nothing more.
    assert!(s.set(&[t.pubkey()], false).is_ok());
    let c = Keypair::new();
    send_burr(&mut s.w, &t, &c.pubkey(), 1_000_000);
    assert_eq!(custom_code(&s.follow(0, &c.pubkey())), Some(err(E::FollowingOff)));
}

#[test]
fn a_crumb_let_go_of_is_recorded_but_too_small_to_buy() {
    let mut s = setup(10 * ZEC1);
    let t = s.target.insecure_clone();
    send_burr(&mut s.w, &t, &Pubkey::new_unique(), 1);
    let pot = s.pot();
    assert!(s.burn(1, 1).is_ok(), "a call that cannot buy still records, without failing");
    assert_eq!(s.watch().owed, 1);
    assert_eq!(s.pot(), pot);
    // A real fall reported with less than the least spend: not tried, so PumpSwap's rounding cannot fail it.
    let gone = burr_of(&s.w, &t.pubkey()) / 100;
    send_burr(&mut s.w, &t, &Pubkey::new_unique(), gone);
    let r = s.burn(gone, burrpad::constants::BURR_MIN_SPEND - 1);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!((s.watch().owed, s.pot()), (gone + 1, pot));
}

#[test]
fn a_closed_target_account_counts_as_nothing_held() {
    let mut s = setup(10 * ZEC1);
    let t = s.target.insecure_clone();
    let all = burr_of(&s.w, &t.pubkey());
    send_burr(&mut s.w, &t, &Pubkey::new_unique(), all);
    // Close the emptied account.
    let i = Instruction {
        program_id: TOKEN_2022,
        accounts: vec![
            AccountMeta::new(ata(&t.pubkey(), &BURR_MINT, &TOKEN_2022), false),
            AccountMeta::new(t.pubkey(), false),
            AccountMeta::new_readonly(t.pubkey(), true),
        ],
        data: vec![9],
    };
    let r = s.w.b.send_as(Some(&t), &[i], &[]);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(s.w.b.account(&ata(&t.pubkey(), &BURR_MINT, &TOKEN_2022)).is_none());
    let r = s.burn(0, 0);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(s.watch().owed, all);
}

#[test]
fn shadow_burn_takes_only_this_coins_watch_pot_and_the_watched_accounts() {
    let mut s = setup(10 * ZEC1);
    let t = s.target.insecure_clone();
    let sold = burr_of(&s.w, &t.pubkey()) / 10;
    let got = sell(&mut s.w, &t, sold);
    // Another shadow coin, with its own watch and the same keeper.
    let other = s.w.launch(&shadow_plan());
    let launcher = s.launcher.insecure_clone();
    let them = s.w.trader(10 * ZEC1);
    let keeper = s.keeper.insecure_clone();
    assert!(s.w.set_shadow_watch(&launcher, &other, &[them.pubkey()], keeper.pubkey(), false).is_ok());
    s.w.b.give_zec(&kit::authority(&other), 10 * ZEC1);
    assert!(s.w.split(&other).is_ok());
    let watched = World::watched_accounts(&[t.pubkey()]);
    type A = burrpad::accounts::ShadowBurn;
    type Change<'x> = Box<dyn Fn(&mut A, &mut Vec<AccountMeta>) + 'x>;
    let swaps: Vec<(&str, Change)> = vec![
        ("another coin's watch", Box::new(|x, _| x.watch = kit::shadow(&other))),
        (
            "another coin's watch, with that watch's own wallets' accounts",
            Box::new(|x, r| {
                x.watch = kit::shadow(&other);
                *r = World::watched_accounts(&[them.pubkey()]);
            }),
        ),
        ("another coin's ledger", Box::new(|x, _| x.burr.ledger = kit::ledger(&other))),
        ("another coin's vault", Box::new(|x, _| x.burr.vault = kit::vault(&other))),
        (
            "a stranger's account for the watched wallet",
            Box::new(|_, r| {
                r[0] = AccountMeta::new_readonly(ata(&Pubkey::new_unique(), &BURR_MINT, &TOKEN_2022), false)
            }),
        ),
        (
            "the watched wallet's ZEC account",
            Box::new(|_, r| r[0] = AccountMeta::new_readonly(ata(&t.pubkey(), &ZEC, &TOKEN), false)),
        ),
        ("no watched account", Box::new(|_, r| r.clear())),
        ("one account too many", Box::new(|_, r| r.push(r[0].clone()))),
    ];
    let mint = s.mint;
    let before = (s.w.ledger(&mint), s.w.ledger(&other), s.w.vault_balance(&mint), s.w.vault_balance(&other));
    for (what, change) in swaps {
        let mut acc = burrpad::accounts::ShadowBurn {
            burr: s.w.burn_burr_accounts(&keeper.pubkey(), &mint),
            watch: kit::shadow(&mint),
        };
        let mut rest = watched.clone();
        change(&mut acc, &mut rest);
        let mut i = kit::raw(acc, burrpad::instruction::ShadowBurn { sold, received: got });
        i.accounts.extend(rest);
        assert!(s.w.send_tabled(&keeper, i).is_err(), "{what} was accepted");
    }
    assert_eq!((s.w.vault_balance(&mint), s.w.vault_balance(&other)), (before.2, before.3));
    assert_eq!(
        (s.w.ledger(&mint).shadow.pot, s.w.ledger(&other).shadow.pot),
        (before.0.shadow.pot, before.1.shadow.pot)
    );
    assert_eq!(s.watch().owed, 0, "nothing was recorded by a refused call");
    let r = s.burn(sold, got);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(s.pot() < before.0.shadow.pot, "with the right accounts it buys");
}
