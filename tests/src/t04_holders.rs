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
const DAY: i64 = 86_400;

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
        burrpad::instruction::ReleaseHolderRewards {
            amount: w.ledger(mint).pots[4],
            snapshot_slot: slot,
            snapshot_hash: hash,
        },
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
    let i = kit::raw(
        acc,
        burrpad::instruction::ReleaseHolderRewards { amount: pot, snapshot_slot: slot, snapshot_hash: [7; 32] },
    );
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
fn a_snapshot_must_be_later_than_the_release_before_it() {
    let mut w = World::new();
    let wallet = w.rewards_wallet.insecure_clone();
    let mint = w.launch(&holders_plan());
    let born = w.slot();
    fee(&mut w, &mint, ZEC1);
    assert_eq!(w.ledger(&mint).holders.last_release_slot, born);
    w.b.wait(HALF_HOUR);

    // Not from before the coin existed, nor from the slot it was made in.
    assert!(born > 1);
    for early in [0, born - 1, born] {
        let i = release_ix(&w, &wallet.pubkey(), &mint, early, [7; 32]);
        let r = w.b.send_as(Some(&wallet), &[i], &[]);
        assert_eq!(custom_code(&r), Some(err(E::BadSnapshot)), "a snapshot from before the coin: slot {early}");
    }

    // A snapshot is taken, and its release lands some slots later.
    let first = born + 1;
    w.b.wait(60);
    let landed = w.slot();
    assert!(landed > first + 1);
    let i = release_ix(&w, &wallet.pubkey(), &mint, first, [7; 32]);
    assert!(w.b.send_as(Some(&wallet), &[i], &[]).is_ok());
    let h = w.ledger(&mint).holders;
    assert_eq!(
        (h.last_release_slot, h.last_snapshot_slot, h.last_snapshot_hash),
        (landed, first, [7; 32]),
        "the slot and fingerprint of the last run are kept"
    );

    // The next snapshot must be later than that landing, not merely later
    // than the last snapshot: holdings from before the last run was paid
    // cannot be used again.
    fee(&mut w, &mint, ZEC1);
    w.b.wait(HALF_HOUR);
    for stale in [first + 1, landed] {
        let i = release_ix(&w, &wallet.pubkey(), &mint, stale, [8; 32]);
        let r = w.b.send_as(Some(&wallet), &[i], &[]);
        assert_eq!(custom_code(&r), Some(err(E::BadSnapshot)), "a snapshot older than the last release: slot {stale}");
    }
    let i = release_ix(&w, &wallet.pubkey(), &mint, landed + 1, [8; 32]);
    assert!(w.b.send_as(Some(&wallet), &[i], &[]).is_ok());
    assert_eq!(w.ledger(&mint).holders.last_snapshot_hash, [8; 32]);
}

#[test]
fn a_release_takes_the_amount_it_states_and_the_rest_stays() {
    // A run is worked out for the pot as it was at the snapshot. Anyone can
    // split more fee into the pot before the release lands; that extra must
    // wait for the next run.
    let (mut w, mint) = setup();
    let planned = w.ledger(&mint).pots[4];
    fee(&mut w, &mint, 3 * ZEC1);
    let pot = w.ledger(&mint).pots[4];
    assert!(pot > planned, "the pot grew between planning and landing");

    let r = w.release_holders_amount(&mint, 0);
    assert_eq!(custom_code(&r), Some(err(E::NothingToRelease)), "a release of nothing");
    let r = w.release_holders_amount(&mint, pot + 1);
    assert_eq!(custom_code(&r), Some(err(E::ReleaseAboveThePot)), "more than the pot: {}", logs(&r));
    assert_eq!(w.ledger(&mint).holders.runs, 0);

    let vault = w.vault_balance(&mint);
    let r = w.release_holders_amount(&mint, planned);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.zec_of(&w.rewards_wallet.pubkey()), planned, "exactly what the run was worked out for");
    let l = w.ledger(&mint);
    assert_eq!((l.pots[4], l.holders.total_released), (pot - planned, planned));
    assert_eq!(w.vault_balance(&mint), vault - planned);
    let said = events::<burrpad::events::HolderRewardsReleased>(&r);
    assert_eq!(said[0][64..72], planned.to_le_bytes());
    w.assert_sound(&mint);

    // The rest is there for the next run, half an hour on, and not before.
    let r = w.release_holders_amount(&mint, pot - planned);
    assert_eq!(custom_code(&r), Some(err(E::HolderReleaseTooSoon)));
    w.b.wait(HALF_HOUR);
    assert!(w.release_holders_amount(&mint, pot - planned).is_ok());
    assert_eq!(w.ledger(&mint).pots[4], 0);
    w.assert_sound(&mint);
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
    let i = kit::raw(
        acc,
        burrpad::instruction::ReleaseHolderRewards { amount: 1, snapshot_slot: slot, snapshot_hash: [9; 32] },
    );
    assert!(w.b.send_as(Some(&wallet), &[i], &[]).is_err());
}

/// The bodies of the events of one kind that a transaction announced.
fn events<T: anchor_lang::Discriminator>(r: &crate::bench::TxResult) -> Vec<Vec<u8>> {
    use base64::Engine;
    let logs = match r {
        Ok(m) => &m.logs,
        Err(e) => &e.meta.logs,
    };
    logs.iter()
        .filter_map(|line| line.strip_prefix("Program data: "))
        .filter_map(|d| base64::engine::general_purpose::STANDARD.decode(d).ok())
        .filter(|d| d.len() >= 8 && d[..8] == T::DISCRIMINATOR[..])
        .map(|d| d[8..].to_vec())
        .collect()
}

#[test]
fn a_new_rewards_wallet_is_proposed_in_the_open_and_waits_a_day() {
    // Naming the rewards wallet reaches coins already launched. So it is
    // slow: anyone watching has a day to see a change coming.
    let (mut w, mint) = setup();
    let old = w.rewards_wallet.insecure_clone();
    let new = w.b.wallet();
    let r = w.as_admin(burrpad::instruction::ProposeRewardsWallet { wallet: Pubkey::default() });
    assert_eq!(custom_code(&r), Some(err(E::EmptyAddress)));
    let r = w.as_admin(burrpad::instruction::ApplyRewardsWallet {});
    assert_eq!(custom_code(&r), Some(err(E::NoRewardsWalletProposed)), "nothing proposed, nothing to apply");

    let r = w.as_admin(burrpad::instruction::ProposeRewardsWallet { wallet: new.pubkey() });
    assert!(r.is_ok(), "{}", logs(&r));
    let ready_at = w.b.now() + DAY;
    let c = w.config();
    assert_eq!((c.rewards_wallet, c.pending_rewards_wallet), (old.pubkey(), new.pubkey()));
    assert_eq!(c.rewards_wallet_ready_at, ready_at);
    // Announced with the new address and the time it can be applied.
    let said = events::<burrpad::events::RewardsWalletProposed>(&r);
    assert_eq!(said.len(), 1, "no event for the proposal");
    assert_eq!(said[0][..32], w.admin.pubkey().to_bytes());
    assert_eq!(said[0][32..64], old.pubkey().to_bytes());
    assert_eq!(said[0][64..96], new.pubkey().to_bytes());
    assert_eq!(said[0][96..104], ready_at.to_le_bytes());

    // Applying early is refused, to the last second.
    let r = w.as_admin(burrpad::instruction::ApplyRewardsWallet {});
    assert_eq!(custom_code(&r), Some(err(E::RewardsWalletWaitNotOver)), "applied at once");
    w.b.wait(DAY - 1);
    let r = w.as_admin(burrpad::instruction::ApplyRewardsWallet {});
    assert_eq!(custom_code(&r), Some(err(E::RewardsWalletWaitNotOver)), "applied a second early");

    // During the wait the proposed wallet can take nothing, and a release
    // still goes to the old wallet.
    let slot = w.slot();
    let i = release_ix(&w, &new.pubkey(), &mint, slot, [7; 32]);
    let r = w.b.send_as(Some(&new), &[i], &[]);
    assert_eq!(custom_code(&r), Some(err(E::NotRewardsWallet)), "the proposed wallet took a pot during the wait");
    let pot = w.ledger(&mint).pots[4];
    assert!(w.release_holders(&mint).is_ok());
    assert_eq!((w.zec_of(&old.pubkey()), w.zec_of(&new.pubkey())), (pot, 0));

    w.b.wait(1);
    assert!(w.as_admin(burrpad::instruction::ApplyRewardsWallet {}).is_ok());
    let c = w.config();
    assert_eq!(
        (c.rewards_wallet, c.pending_rewards_wallet, c.rewards_wallet_ready_at),
        (new.pubkey(), Pubkey::default(), 0)
    );
    let r = w.as_admin(burrpad::instruction::ApplyRewardsWallet {});
    assert_eq!(custom_code(&r), Some(err(E::NoRewardsWalletProposed)), "applied twice");

    // From then on the old wallet is shut out and the new one is paid.
    fee(&mut w, &mint, ZEC1);
    w.b.wait(HALF_HOUR);
    let slot = w.slot();
    let i = release_ix(&w, &old.pubkey(), &mint, slot, [8; 32]);
    let r = w.b.send_as(Some(&old), &[i], &[]);
    assert_eq!(custom_code(&r), Some(err(E::NotRewardsWallet)));
    let pot = w.ledger(&mint).pots[4];
    let i = release_ix(&w, &new.pubkey(), &mint, slot, [8; 32]);
    let r = w.b.send_as(Some(&new), &[i], &[]);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.zec_of(&new.pubkey()), pot);
    // It did not change the coin's record.
    assert_eq!(w.coin(&mint).shares, [0, 0, 0, 10_000]);
}

#[test]
fn a_proposed_rewards_wallet_can_be_cancelled_or_replaced() {
    let (mut w, mint) = setup();
    let old = w.rewards_wallet.pubkey();
    let (first, second) = (w.b.wallet(), w.b.wallet());
    let r = w.as_admin(burrpad::instruction::CancelRewardsWallet {});
    assert_eq!(custom_code(&r), Some(err(E::NoRewardsWalletProposed)), "nothing to cancel");

    // Cancelled: a day later there is nothing to apply.
    assert!(w.as_admin(burrpad::instruction::ProposeRewardsWallet { wallet: first.pubkey() }).is_ok());
    let r = w.as_admin(burrpad::instruction::CancelRewardsWallet {});
    assert!(r.is_ok(), "{}", logs(&r));
    let said = events::<burrpad::events::RewardsWalletProposed>(&r);
    assert_eq!(said.len(), 1, "no event for the cancelling");
    assert_eq!(said[0][64..96], [0u8; 32]);
    let c = w.config();
    assert_eq!((c.pending_rewards_wallet, c.rewards_wallet_ready_at), (Pubkey::default(), 0));
    w.b.wait(DAY);
    let r = w.as_admin(burrpad::instruction::ApplyRewardsWallet {});
    assert_eq!(custom_code(&r), Some(err(E::NoRewardsWalletProposed)), "a cancelled proposal was applied");
    assert_eq!(w.config().rewards_wallet, old);

    // Replaced: the second proposal starts its own full wait.
    assert!(w.as_admin(burrpad::instruction::ProposeRewardsWallet { wallet: first.pubkey() }).is_ok());
    w.b.wait(DAY - 1);
    assert!(w.as_admin(burrpad::instruction::ProposeRewardsWallet { wallet: second.pubkey() }).is_ok());
    w.b.wait(1);
    let r = w.as_admin(burrpad::instruction::ApplyRewardsWallet {});
    assert_eq!(custom_code(&r), Some(err(E::RewardsWalletWaitNotOver)), "the wait did not start again");
    w.b.wait(DAY - 1);
    assert!(w.as_admin(burrpad::instruction::ApplyRewardsWallet {}).is_ok());
    assert_eq!(w.config().rewards_wallet, second.pubkey());
    assert!(w.ledger(&mint).pots[4] > 0);
}

#[test]
fn the_pause_stops_every_coins_release_at_once_and_nothing_else() {
    // For a stolen rewards wallet: no waiting. The pots stay where they are.
    let mut w = World::new();
    let a = w.launch(&LaunchPlan::default());
    let b = w.launch(&holders_plan());
    fee(&mut w, &a, 8 * ZEC1);
    fee(&mut w, &b, ZEC1);
    w.b.wait(HALF_HOUR);
    let (pa, pb) = (w.ledger(&a).pots[4], w.ledger(&b).pots[4]);

    let r = w.as_admin(burrpad::instruction::SetHolderReleasesPaused { paused: true });
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(w.config().holder_releases_paused);
    let said = events::<burrpad::events::ConfigChanged>(&r);
    assert_eq!(said.len(), 1);
    assert_eq!(said[0].last(), Some(&1), "the pause was not announced");
    for m in [a, b] {
        let r = w.release_holders(&m);
        assert_eq!(custom_code(&r), Some(err(E::HolderReleasesPaused)), "released while paused: {}", logs(&r));
    }
    assert_eq!((w.ledger(&a).pots[4], w.ledger(&b).pots[4]), (pa, pb));
    assert_eq!(w.zec_of(&w.rewards_wallet.pubkey()), 0);

    // Everything else carries on: fees are split, other pots are paid, launches work.
    fee(&mut w, &a, ZEC1);
    assert!(w.ledger(&a).pots[4] > pa, "the pot keeps filling");
    assert!(w.pay_out(&a, burrpad::state::Payee::Creator).is_ok());
    w.launch(&LaunchPlan::default());

    // Switched back on, each coin releases as before.
    assert!(w.as_admin(burrpad::instruction::SetHolderReleasesPaused { paused: false }).is_ok());
    let (pa, pb) = (w.ledger(&a).pots[4], w.ledger(&b).pots[4]);
    assert!(w.release_holders(&a).is_ok());
    assert!(w.release_holders(&b).is_ok());
    assert_eq!(w.zec_of(&w.rewards_wallet.pubkey()), pa + pb);
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
