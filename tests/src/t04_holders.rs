//! Holders: the coin's holder pot is handed whole to the rewards wallet,
//! which pays holders from it. The program decides only where the pot may
//! go, how often, and that each release names its snapshot.

use crate::bench::{custom_code, logs};
use crate::kit::{self, err, LaunchPlan, World};
use crate::pump::{ata, TOKEN, ZEC};
use burrpad::error::BurrpadError as E;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

const ZEC1: u64 = 100_000_000;
const HALF_HOUR: i64 = 1_800;

fn holders_plan() -> LaunchPlan {
    LaunchPlan { shares: [0, 0, 0, 10_000], ..Default::default() }
}

/// A coin whose whole fee goes to holders, with fee split and the first
/// half hour already passed.
fn setup() -> (World, Pubkey) {
    let mut w = World::new();
    let mint = w.launch(&holders_plan());
    fee(&mut w, &mint, ZEC1);
    w.b.wait(HALF_HOUR);
    (w, mint)
}

fn fee(w: &mut World, mint: &Pubkey, amount: u64) {
    w.b.give_zec(&kit::authority(mint), amount);
    assert!(w.split(mint).is_ok());
}

fn release_ix(w: &World, wallet: &Pubkey, mint: &Pubkey, slot: u64, hash: [u8; 32]) -> solana_instruction::Instruction {
    kit::raw(
        w.release_accounts(wallet, mint),
        burrpad::instruction::ReleaseHolderRewards { snapshot_slot: slot, snapshot_hash: hash },
    )
}

#[test]
fn the_whole_pot_goes_to_the_rewards_wallet_and_nowhere_else() {
    let (mut w, mint) = setup();
    let before = w.ledger(&mint);
    let pot = before.pots[4];
    assert!(pot > 0);
    let vault = w.vault_balance(&mint);
    let slot = w.slot();
    let r = w.release_holders(&mint);
    assert!(r.is_ok(), "{}", logs(&r));

    assert_eq!(w.zec_of(&w.rewards_wallet.pubkey()), pot);
    assert_eq!(w.vault_balance(&mint), vault - pot);
    let l = w.ledger(&mint);
    assert_eq!(l.pots[4], 0);
    assert_eq!(l.pots[..4], before.pots[..4], "no other pot was touched");
    assert_eq!((l.holders.runs, l.holders.total_released, l.holders.last_snapshot_slot), (1, pot, slot));
    assert_eq!(l.holders.last_release, w.b.now());
    w.assert_sound(&mint);

    // The release is announced with the snapshot it was for.
    let announced = r.unwrap().logs.iter().any(|line| {
        use base64::Engine;
        line.strip_prefix("Program data: ")
            .and_then(|d| base64::engine::general_purpose::STANDARD.decode(d).ok())
            .is_some_and(|d| {
                d.len() == 8 + 32 + 32 + 8 + 8 + 8 + 32
                    && d[..8]
                        == <burrpad::events::HolderRewardsReleased as anchor_lang::Discriminator>::DISCRIMINATOR[..]
                    && d[8..40] == mint.to_bytes()
                    && d[40..72] == w.rewards_wallet.pubkey().to_bytes()
                    && d[72..80] == pot.to_le_bytes()
                    && d[88..96] == slot.to_le_bytes()
                    && d[96..128] == [7u8; 32]
            })
    });
    assert!(announced, "no event for the release");
}

#[test]
fn releases_are_at_least_half_an_hour_apart() {
    let mut w = World::new();
    let mint = w.launch(&holders_plan());
    fee(&mut w, &mint, ZEC1);
    // Not in the first half hour after launch.
    w.b.wait(HALF_HOUR - 1);
    let r = w.release_holders(&mint);
    assert_eq!(custom_code(&r), Some(err(E::HolderReleaseTooSoon)), "{}", logs(&r));
    w.b.wait(1);
    assert!(w.release_holders(&mint).is_ok());

    // Nor within half an hour of the last one, however much has come in.
    fee(&mut w, &mint, 10 * ZEC1);
    w.b.wait(HALF_HOUR - 1);
    let r = w.release_holders(&mint);
    assert_eq!(custom_code(&r), Some(err(E::HolderReleaseTooSoon)));
    assert!(w.ledger(&mint).pots[4] > 0);
    w.b.wait(1);
    let r = w.release_holders(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.ledger(&mint).holders.runs, 2);
    w.assert_sound(&mint);
}

#[test]
fn only_the_rewards_wallet_can_take_the_pot() {
    let (mut w, mint) = setup();
    let pot = w.ledger(&mint).pots[4];
    let slot = w.slot();
    let real = w.rewards_wallet.pubkey();

    // A stranger names themselves.
    let thief = w.trader(ZEC1);
    let i = release_ix(&w, &thief.pubkey(), &mint, slot, [7; 32]);
    let r = w.b.send_as(Some(&thief), &[i], &[]);
    assert_eq!(custom_code(&r), Some(err(E::NotRewardsWallet)), "{}", logs(&r));

    // The administrator is not the rewards wallet either.
    let admin = w.admin.insecure_clone();
    let i = release_ix(&w, &admin.pubkey(), &mint, slot, [7; 32]);
    let r = w.b.send_as(Some(&admin), &[i], &[]);
    assert_eq!(custom_code(&r), Some(err(E::NotRewardsWallet)));

    // The right wallet is named, but has not signed. (Its ZEC account
    // already exists, so nothing else in the call needs its signature.)
    w.b.give_zec(&real, 0);
    let mut i = release_ix(&w, &real, &mint, slot, [7; 32]);
    i.accounts[0].is_signer = false;
    assert!(w.b.send_as(Some(&thief), &[i], &[]).is_err());

    // The right wallet signs, but the destination is someone else's ZEC account.
    let mut acc = w.release_accounts(&real, &mint);
    acc.rewards_zec = ata(&thief.pubkey(), &ZEC, &TOKEN);
    let i = kit::raw(acc, burrpad::instruction::ReleaseHolderRewards { snapshot_slot: slot, snapshot_hash: [7; 32] });
    let wallet = w.rewards_wallet.insecure_clone();
    let r = w.b.send_as(Some(&wallet), &[i], &[]);
    assert_eq!(custom_code(&r), Some(err(E::WrongReceiver)));

    assert_eq!(w.ledger(&mint).pots[4], pot);
    assert_eq!(w.zec_of(&thief.pubkey()), ZEC1);
}

#[test]
fn a_release_must_name_a_real_snapshot() {
    let (mut w, mint) = setup();
    let wallet = w.rewards_wallet.insecure_clone();
    let slot = w.slot();
    // No fingerprint.
    let i = release_ix(&w, &wallet.pubkey(), &mint, slot, [0; 32]);
    let r = w.b.send_as(Some(&wallet), &[i], &[]);
    assert_eq!(custom_code(&r), Some(err(E::BadSnapshot)), "no fingerprint");
    // A slot that has not happened yet.
    let i = release_ix(&w, &wallet.pubkey(), &mint, slot + 1, [7; 32]);
    let r = w.b.send_as(Some(&wallet), &[i], &[]);
    assert_eq!(custom_code(&r), Some(err(E::BadSnapshot)), "a future slot");
    // A good one.
    let i = release_ix(&w, &wallet.pubkey(), &mint, slot, [7; 32]);
    assert!(w.b.send_as(Some(&wallet), &[i], &[]).is_ok());
    // The next run cannot reuse that snapshot's slot, or an earlier one.
    fee(&mut w, &mint, ZEC1);
    w.b.wait(HALF_HOUR);
    for old in [slot, slot - 1] {
        let i = release_ix(&w, &wallet.pubkey(), &mint, old, [8; 32]);
        let r = w.b.send_as(Some(&wallet), &[i], &[]);
        assert_eq!(custom_code(&r), Some(err(E::BadSnapshot)), "an old slot");
    }
    let i = release_ix(&w, &wallet.pubkey(), &mint, slot + 1, [8; 32]);
    assert!(w.b.send_as(Some(&wallet), &[i], &[]).is_ok());
}

#[test]
fn an_empty_pot_and_a_coin_without_the_hook_release_nothing() {
    let mut w = World::new();
    let mint = w.launch(&holders_plan());
    w.b.wait(HALF_HOUR);
    let r = w.release_holders(&mint);
    assert_eq!(custom_code(&r), Some(err(E::NothingToRelease)));
    assert_eq!(w.ledger(&mint).holders.runs, 0, "an empty call does not use up the half hour");

    let plain = w.launch(&LaunchPlan { shares: [10_000, 0, 0, 0], ..Default::default() });
    fee(&mut w, &plain, ZEC1);
    w.b.wait(HALF_HOUR);
    let r = w.release_holders(&plain);
    assert_eq!(custom_code(&r), Some(err(E::HookNotChosen)));
    assert_eq!(w.zec_of(&w.rewards_wallet.pubkey()), 0);
}

#[test]
fn the_rewards_wallet_reaches_only_holder_pots() {
    // The widest harm a stolen rewards wallet can do: every coin's holder
    // pot, once per half hour each. Nothing else.
    let mut w = World::new();
    let a = w.launch(&LaunchPlan::default());
    let b = w.launch(&LaunchPlan::default());
    fee(&mut w, &a, 8 * ZEC1);
    fee(&mut w, &b, 8 * ZEC1);
    w.b.wait(HALF_HOUR);
    let (la, lb) = (w.ledger(&a), w.ledger(&b));
    assert!(w.release_holders(&a).is_ok());
    assert!(w.release_holders(&b).is_ok());
    assert_eq!(w.zec_of(&w.rewards_wallet.pubkey()), la.pots[4] + lb.pots[4]);
    for (m, before) in [(a, la), (b, lb)] {
        let l = w.ledger(&m);
        assert_eq!(l.pots[..4], before.pots[..4], "platform, creator, give and burn pots are untouched");
        assert_eq!(w.vault_balance(&m), l.accounted);
        w.assert_sound(&m);
    }
    // One coin's accounts cannot be used to take another's pot.
    fee(&mut w, &a, ZEC1);
    fee(&mut w, &b, ZEC1);
    w.b.wait(HALF_HOUR);
    let wallet = w.rewards_wallet.insecure_clone();
    let mut acc = w.release_accounts(&wallet.pubkey(), &a);
    acc.vault = kit::vault(&b);
    let slot = w.slot();
    let i = kit::raw(acc, burrpad::instruction::ReleaseHolderRewards { snapshot_slot: slot, snapshot_hash: [9; 32] });
    assert!(w.b.send_as(Some(&wallet), &[i], &[]).is_err());
}

#[test]
fn the_administrator_can_replace_the_rewards_wallet_for_every_coin() {
    // This is the power the owner accepted with this hook: it reaches coins
    // already launched. It exists so a lost or stolen wallet can be replaced.
    let (mut w, mint) = setup();
    let old = w.rewards_wallet.insecure_clone();
    let new = w.b.wallet();
    let r = w.as_admin(burrpad::instruction::SetRewardsWallet { wallet: Pubkey::default() });
    assert_eq!(custom_code(&r), Some(err(E::EmptyAddress)));
    assert!(w.as_admin(burrpad::instruction::SetRewardsWallet { wallet: new.pubkey() }).is_ok());
    assert_eq!(w.config().rewards_wallet, new.pubkey());

    let slot = w.slot();
    let i = release_ix(&w, &old.pubkey(), &mint, slot, [7; 32]);
    let r = w.b.send_as(Some(&old), &[i], &[]);
    assert_eq!(custom_code(&r), Some(err(E::NotRewardsWallet)), "the old wallet is shut out at once");
    let pot = w.ledger(&mint).pots[4];
    let i = release_ix(&w, &new.pubkey(), &mint, slot, [7; 32]);
    let r = w.b.send_as(Some(&new), &[i], &[]);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.zec_of(&new.pubkey()), pot);
    // It did not change the coin's record.
    assert_eq!(w.coin(&mint).shares, [0, 0, 0, 10_000]);
}

#[test]
fn holder_releases_work_the_same_after_graduation() {
    let mut w = World::new();
    let mint = w.launch(&holders_plan());
    w.graduate(&mint);
    let t = w.trader(100 * ZEC1);
    assert!(w.pool_buy(&t, &mint, 2 * ZEC1).is_ok());
    assert!(w.collect_curve(&mint).is_ok());
    assert!(w.collect_pool(&mint).is_ok());
    let pot = w.ledger(&mint).pots[4];
    assert!(pot > 0);
    w.b.wait(HALF_HOUR);
    let r = w.release_holders(&mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.zec_of(&w.rewards_wallet.pubkey()), pot);
    w.assert_sound(&mint);
}
