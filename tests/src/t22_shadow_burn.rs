//! The eighth hook, the shadow burn: part of a coin's fee buys back and burns
//! $BURR up to what the watched wallets let go of. The program records their
//! holding and allows burning only up to the fall since the record began.

use crate::bench::{custom_code, logs};
use crate::kit::{self, err, LaunchPlan, World};
use crate::pump::{ata, TOKEN, TOKEN_2022, ZEC};
use burrpad::error::BurrpadError as E;
use burrpad::BURR_MINT;
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

/// $BURR in place; a coin that chose the shadow burn, with `fee` split; a target
/// holding $BURR, watched by the launcher's watch; the wait over.
fn setup(fee: u64) -> (World, Pubkey, Keypair) {
    let mut w = World::new();
    w.make_burr();
    let target = w.trader(1_000 * ZEC1);
    assert!(w.pool_buy(&target, &BURR_MINT, 5 * ZEC1).is_ok());
    let mint = w.launch(&shadow_plan());
    let launcher = w.b.payer.insecure_clone();
    let r = w.set_shadow_watch(&launcher, &mint, &[target.pubkey()], Pubkey::default(), false);
    assert!(r.is_ok(), "{}", logs(&r));
    w.b.give_zec(&kit::authority(&mint), fee);
    assert!(w.split(&mint).is_ok());
    w.b.wait(60);
    (w, mint, target)
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
    let t = Pubkey::new_unique();
    assert_eq!(
        custom_code(&w.set_shadow_watch(&stranger, &mint, &[t], Pubkey::default(), false)),
        Some(err(E::NotTheLauncher))
    );
    for (what, list) in [
        ("empty", vec![]),
        ("repeated", vec![t, t]),
        ("five", (0..5).map(|_| Pubkey::new_unique()).collect()),
        ("the empty address", vec![Pubkey::default()]),
    ] {
        let r = w.set_shadow_watch(&launcher, &mint, &list, Pubkey::default(), false);
        assert_eq!(custom_code(&r), Some(err(E::BadWatchList)), "{what}");
    }
    // A coin that did not choose the hook has no watch.
    let old = w.launch(&LaunchPlan::default());
    assert_eq!(
        custom_code(&w.set_shadow_watch(&launcher, &old, &[t], Pubkey::default(), false)),
        Some(err(E::HookNotChosen))
    );
    // Someone else's account in place of the watched wallet's.
    let acc = burrpad::accounts::SetShadowWatch {
        launcher: launcher.pubkey(),
        mint,
        coin: kit::coin(&mint),
        watch: kit::shadow(&mint),
        system_program: crate::pump::SYSTEM,
    };
    let mut i = kit::raw(
        acc,
        burrpad::instruction::SetShadowWatch { wallets: vec![t], follower: Pubkey::default(), follow: false },
    );
    i.accounts.push(AccountMeta::new_readonly(ata(&Pubkey::new_unique(), &BURR_MINT, &TOKEN_2022), false));
    assert_eq!(custom_code(&w.b.send(&[i], &[])), Some(err(E::WrongWatchedAccount)));
    assert!(w.set_shadow_watch(&launcher, &mint, &[t], Pubkey::default(), false).is_ok());
}

#[test]
fn with_nothing_let_go_of_a_shadow_burn_only_records() {
    let (mut w, mint, _) = setup(10 * ZEC1);
    let pot = w.ledger(&mint).shadow.pot;
    let r = w.shadow_burn(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.ledger(&mint).shadow.pot, pot, "nothing bought");
    assert_eq!(w.shadow_watch(&mint).unwrap().owed, 0);
}

#[test]
fn a_sale_by_the_target_is_bought_back_and_burned_never_more() {
    let (mut w, mint, target) = setup(50 * ZEC1);
    let held = burr_of(&w, &target.pubkey());
    let sold = held / 50;
    assert!(w.pool_sell(&target, &BURR_MINT, sold).is_ok());
    let supply = w.b.mint_supply(&BURR_MINT);
    let (vault, pot) = (w.vault_balance(&mint), w.ledger(&mint).shadow.pot);
    let r = w.shadow_burn(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    let watch = w.shadow_watch(&mint).unwrap();
    let spent = pot - w.ledger(&mint).shadow.pot;
    let burned = supply - w.b.mint_supply(&BURR_MINT);
    assert!(spent > 0 && burned > 0, "it bought back");
    assert!(burned <= sold, "burned {burned}, more than the {sold} let go of");
    assert_eq!(watch.owed, sold - burned, "what was bought came off what is owed");
    assert_eq!((watch.total_spent, watch.total_burned), (spent, burned));
    assert_eq!(w.vault_balance(&mint), vault - spent);
    let bn = kit::burner(&mint);
    assert_eq!(w.b.token_balance(&ata(&bn, &ZEC, &TOKEN)), 0);
    assert_eq!(w.b.token_balance(&ata(&bn, &BURR_MINT, &TOKEN_2022)), 0);
    w.assert_sound(&mint);
    // More calls, a minute apart, burn the rest, and never past what was let go of.
    for _ in 0..200 {
        if w.shadow_watch(&mint).unwrap().owed == 0 {
            break;
        }
        w.b.wait(60);
        assert!(w.shadow_burn(&mint).is_ok());
    }
    let watch = w.shadow_watch(&mint).unwrap();
    assert!(watch.total_burned <= sold, "burned {} in all, more than {sold}", watch.total_burned);
    assert_eq!(watch.owed + watch.total_burned, sold, "every coin let go of is burned or still owed");
    w.assert_sound(&mint);
}

#[test]
fn coins_moved_between_watched_wallets_count_for_nothing_sent_away_count() {
    let (mut w, mint, target) = setup(10 * ZEC1);
    let second = w.b.wallet();
    let launcher = w.b.payer.insecure_clone();
    assert!(w
        .set_shadow_watch(&launcher, &mint, &[target.pubkey(), second.pubkey()], Pubkey::default(), false)
        .is_ok());
    let held = burr_of(&w, &target.pubkey());
    send_burr(&mut w, &target, &second.pubkey(), held / 3);
    assert!(w.shadow_burn(&mint).is_ok());
    assert_eq!(w.shadow_watch(&mint).unwrap().owed, 0, "a move between watched wallets is not a fall");
    // Sent to an unwatched wallet (an exchange deposit, say): a fall at once.
    let exchange = Pubkey::new_unique();
    send_burr(&mut w, &target, &exchange, 1_000_000);
    w.b.wait(60);
    let r = w.shadow_burn(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    let watch = w.shadow_watch(&mint).unwrap();
    assert_eq!(watch.owed + watch.total_burned, 1_000_000);
}

#[test]
fn a_change_of_wallets_keeps_what_is_owed_and_starts_the_new_ones_from_now() {
    let (mut w, mint, target) = setup(10 * ZEC1);
    let launcher = w.b.payer.insecure_clone();
    // The target lets go of some, then the launcher switches to a new wallet before any burn.
    send_burr(&mut w, &target, &Pubkey::new_unique(), 2_000_000);
    let other = w.trader(100 * ZEC1);
    assert!(w.pool_buy(&other, &BURR_MINT, ZEC1).is_ok());
    assert!(w.set_shadow_watch(&launcher, &mint, &[other.pubkey()], Pubkey::default(), false).is_ok());
    let watch = w.shadow_watch(&mint).unwrap();
    assert_eq!(watch.owed, 2_000_000, "the old wallet's fall was recorded before the change");
    assert_eq!(watch.recorded, burr_of(&w, &other.pubkey()), "the new wallet starts from its holding now");
    // The old wallet's later sales no longer count; the new one's do.
    send_burr(&mut w, &target, &Pubkey::new_unique(), 3_000_000);
    send_burr(&mut w, &other, &Pubkey::new_unique(), 1_000_000);
    assert!(w.shadow_burn(&mint).is_ok());
    let watch = w.shadow_watch(&mint).unwrap();
    assert_eq!(watch.owed + watch.total_burned, 3_000_000);
}

#[test]
fn a_pot_bigger_than_the_fall_spends_only_what_buys_the_fall() {
    let (mut w, mint, target) = setup(200 * ZEC1);
    let sold = 10_000;
    send_burr(&mut w, &target, &Pubkey::new_unique(), sold);
    let pot = w.ledger(&mint).shadow.pot;
    let supply = w.b.mint_supply(&BURR_MINT);
    assert!(w.shadow_burn(&mint).is_ok());
    let burned = supply - w.b.mint_supply(&BURR_MINT);
    assert!(burned <= sold);
    assert!(pot - w.ledger(&mint).shadow.pot < pot / 1_000, "most of the pot waits");
}

#[test]
fn the_target_buying_back_in_moves_the_record_and_keeps_what_is_owed() {
    let (mut w, mint, target) = setup(10 * ZEC1);
    send_burr(&mut w, &target, &Pubkey::new_unique(), 4_000_000);
    // Recorded once the wait is not over: only records, no buy.
    let launcher = w.b.payer.insecure_clone();
    let keep = w.shadow_watch(&mint).unwrap().wallets[0];
    assert!(w.set_shadow_watch(&launcher, &mint, &[keep], Pubkey::default(), false).is_ok());
    assert_eq!(w.shadow_watch(&mint).unwrap().owed, 4_000_000);
    assert!(w.pool_buy(&target, &BURR_MINT, ZEC1).is_ok());
    assert!(w.shadow_burn(&mint).is_ok());
    let watch = w.shadow_watch(&mint).unwrap();
    assert_eq!(watch.recorded, burr_of(&w, &target.pubkey()), "the record rose with the holding");
    assert_eq!(watch.owed + watch.total_burned, 4_000_000, "what was owed stays owed");
}

#[test]
fn a_follower_may_only_add_and_only_as_allowed() {
    let (mut w, mint, target) = setup(10 * ZEC1);
    let launcher = w.b.payer.insecure_clone();
    let follower = w.b.wallet();
    let new = w.trader(10 * ZEC1);
    // Not allowed yet.
    assert_eq!(custom_code(&w.follow_wallet(&follower, &mint, &new.pubkey())), Some(err(E::FollowerNotAllowed)));
    assert!(w.set_shadow_watch(&launcher, &mint, &[target.pubkey()], follower.pubkey(), true).is_ok());
    // A stranger cannot follow; the follower adds one wallet, recorded as it is now.
    let stranger = w.b.wallet();
    assert_eq!(custom_code(&w.follow_wallet(&stranger, &mint, &new.pubkey())), Some(err(E::FollowerNotAllowed)));
    assert!(w.pool_buy(&new, &BURR_MINT, ZEC1).is_ok());
    // The target lets go of some just before the follower acts: that fall is recorded, not lost.
    send_burr(&mut w, &target, &Pubkey::new_unique(), 500_000);
    let r = w.follow_wallet(&follower, &mint, &new.pubkey());
    assert!(r.is_ok(), "{}", logs(&r));
    let watch = w.shadow_watch(&mint).unwrap();
    assert_eq!(watch.count, 2);
    assert_eq!(watch.owed, 500_000, "the fall before the addition is owed; the addition itself adds nothing");
    // The added wallet's own later sale counts.
    send_burr(&mut w, &new, &Pubkey::new_unique(), 700_000);
    assert!(w.shadow_burn(&mint).is_ok());
    let watch = w.shadow_watch(&mint).unwrap();
    assert_eq!(watch.owed + watch.total_burned, 1_200_000, "the added wallet's sale counts too");
    // At most once an hour; never a wallet already watched; never past four.
    let more = Pubkey::new_unique();
    assert_eq!(custom_code(&w.follow_wallet(&follower, &mint, &more)), Some(err(E::FollowerNotAllowed)));
    w.b.wait(3_600);
    assert_eq!(custom_code(&w.follow_wallet(&follower, &mint, &new.pubkey())), Some(err(E::BadWatchList)));
    for _ in 0..2 {
        assert!(w.follow_wallet(&follower, &mint, &Pubkey::new_unique()).is_ok());
        w.b.wait(3_600);
    }
    assert_eq!(
        custom_code(&w.follow_wallet(&follower, &mint, &Pubkey::new_unique())),
        Some(err(E::BadWatchList)),
        "a fifth"
    );
    // The launcher turns following off: the follower can do nothing more.
    assert!(w.set_shadow_watch(&launcher, &mint, &[target.pubkey()], follower.pubkey(), false).is_ok());
    w.b.wait(3_600);
    assert_eq!(
        custom_code(&w.follow_wallet(&follower, &mint, &Pubkey::new_unique())),
        Some(err(E::FollowerNotAllowed))
    );
}

#[test]
fn a_crumb_let_go_of_is_recorded_but_too_small_to_buy() {
    let (mut w, mint, target) = setup(10 * ZEC1);
    send_burr(&mut w, &target, &Pubkey::new_unique(), 1);
    let pot = w.ledger(&mint).shadow.pot;
    assert!(w.shadow_burn(&mint).is_ok(), "a call that cannot buy still records, without failing");
    assert_eq!(w.shadow_watch(&mint).unwrap().owed, 1);
    assert_eq!(w.ledger(&mint).shadow.pot, pot);
}

#[test]
fn shadow_burns_share_the_one_wait_for_every_burr_buy() {
    let (mut w, mint, target) = setup(50 * ZEC1);
    let burr_coin = w.launch(&LaunchPlan { shares: [2_000; 4], burr_share: 2_000, ..Default::default() });
    w.b.give_zec(&kit::authority(&burr_coin), 10 * ZEC1);
    assert!(w.split(&burr_coin).is_ok());
    let tenth = burr_of(&w, &target.pubkey()) / 10;
    send_burr(&mut w, &target, &Pubkey::new_unique(), tenth);
    w.b.wait(60);
    assert!(w.burn_burr(&burr_coin).is_ok());
    // Inside the wait a shadow burn only records; it does not fail, and buys nothing.
    let pot = w.ledger(&mint).shadow.pot;
    assert!(w.shadow_burn(&mint).is_ok());
    assert_eq!(w.ledger(&mint).shadow.pot, pot);
    assert!(w.shadow_watch(&mint).unwrap().owed > 0, "the fall was recorded");
    w.b.wait(60);
    assert!(w.shadow_burn(&mint).is_ok());
    assert!(w.ledger(&mint).shadow.pot < pot, "after the wait it buys");
    assert_eq!(custom_code(&w.burn_burr(&burr_coin)), Some(err(E::BurnTooSoon)), "and holds the next $BURR buy");
}

#[test]
fn a_closed_target_account_counts_as_nothing_held() {
    let (mut w, mint, target) = setup(10 * ZEC1);
    let all = burr_of(&w, &target.pubkey());
    send_burr(&mut w, &target, &Pubkey::new_unique(), all);
    // Close the emptied account.
    let i = Instruction {
        program_id: TOKEN_2022,
        accounts: vec![
            AccountMeta::new(ata(&target.pubkey(), &BURR_MINT, &TOKEN_2022), false),
            AccountMeta::new(target.pubkey(), false),
            AccountMeta::new_readonly(target.pubkey(), true),
        ],
        data: vec![9],
    };
    let r = w.b.send_as(Some(&target), &[i], &[]);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(w.b.account(&ata(&target.pubkey(), &BURR_MINT, &TOKEN_2022)).is_none());
    let r = w.shadow_burn(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    let watch = w.shadow_watch(&mint).unwrap();
    assert_eq!(watch.owed + watch.total_burned, all);
}

#[test]
fn shadow_burn_takes_only_this_coins_watch_pot_and_the_watched_accounts() {
    let (mut w, mint, target) = setup(10 * ZEC1);
    let tenth = burr_of(&w, &target.pubkey()) / 10;
    send_burr(&mut w, &target, &Pubkey::new_unique(), tenth);
    // Another shadow coin, with its own watch.
    let other = w.launch(&shadow_plan());
    let launcher = w.b.payer.insecure_clone();
    let them = w.trader(10 * ZEC1);
    assert!(w.set_shadow_watch(&launcher, &other, &[them.pubkey()], Pubkey::default(), false).is_ok());
    w.b.give_zec(&kit::authority(&other), 10 * ZEC1);
    assert!(w.split(&other).is_ok());
    let payer = w.b.payer.pubkey();
    let watched = World::watched_accounts(&[target.pubkey()]);
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
            Box::new(|_, r| r[0] = AccountMeta::new_readonly(ata(&target.pubkey(), &ZEC, &TOKEN), false)),
        ),
        ("no watched account", Box::new(|_, r| r.clear())),
        ("one account too many", Box::new(|_, r| r.push(r[0].clone()))),
    ];
    let before = (w.ledger(&mint), w.ledger(&other), w.vault_balance(&mint), w.vault_balance(&other));
    for (what, change) in swaps {
        let mut acc =
            burrpad::accounts::ShadowBurn { burr: w.burn_burr_accounts(&payer, &mint), watch: kit::shadow(&mint) };
        let mut rest = watched.clone();
        change(&mut acc, &mut rest);
        let mut i = kit::raw(acc, burrpad::instruction::ShadowBurn {});
        i.accounts.extend(rest);
        assert!(w.b.send(&[i], &[]).is_err(), "{what} was accepted");
    }
    assert_eq!((w.vault_balance(&mint), w.vault_balance(&other)), (before.2, before.3));
    assert_eq!((w.ledger(&mint).shadow.pot, w.ledger(&other).shadow.pot), (before.0.shadow.pot, before.1.shadow.pot));
    assert_eq!(w.shadow_watch(&mint).unwrap().owed, 0, "nothing was recorded by a refused call");
    let r = w.shadow_burn(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(w.ledger(&mint).shadow.pot < before.0.shadow.pot, "with the right accounts it buys");
}
