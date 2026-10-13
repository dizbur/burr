//! Vault share: people who place coins in the coin's vault share this pot,
//! by amount and by time.

use crate::bench::{custom_code, logs};
use crate::kit::{self, err, LaunchPlan, World};
use crate::pump::{ata, TOKEN_2022};
use burr::error::BurrError as E;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

const ZEC1: u64 = 100_000_000;
const DAY: i64 = 86_400;

fn vs_plan() -> LaunchPlan {
    LaunchPlan { shares: [0, 0, 0, 10_000], ..Default::default() }
}

/// A coin whose whole fee goes to the vault share, and holders with coins.
fn setup(n: usize) -> (World, Pubkey, Vec<Keypair>) {
    let mut w = World::new();
    let mint = w.launch(&vs_plan());
    let mut holders = vec![];
    for _ in 0..n {
        let h = w.trader(100 * ZEC1);
        assert!(w.buy(&h, &mint, ZEC1 / 10).is_ok());
        holders.push(h);
    }
    (w, mint, holders)
}

/// Fee arrives and is split; returns what the vault share pot was credited.
fn fee(w: &mut World, mint: &Pubkey, amount: u64) -> u64 {
    let before = w.ledger(mint).pots[4];
    w.b.give_zec(&kit::authority(mint), amount);
    assert!(w.split(mint).is_ok());
    w.ledger(mint).pots[4] - before
}

#[test]
fn coins_go_in_and_come_out_at_any_time() {
    let (mut w, mint, h) = setup(1);
    let a = &h[0];
    let had = w.coins_of(&a.pubkey(), &mint);
    let r = w.stake(a, &mint, had);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.coins_of(&a.pubkey(), &mint), 0);
    assert_eq!(w.b.token_balance(&kit::stake_vault(&mint)), had);
    assert_eq!(w.ledger(&mint).vault_share.total_staked, had);
    // Out again in the very next transaction: no lock.
    let r = w.unstake(a, &mint, had);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.coins_of(&a.pubkey(), &mint), had);
    assert_eq!(w.ledger(&mint).vault_share.total_staked, 0);
}

#[test]
fn stakers_share_by_amount() {
    let (mut w, mint, h) = setup(2);
    assert!(w.stake(&h[0], &mint, 1_000_000).is_ok());
    assert!(w.stake(&h[1], &mint, 3_000_000).is_ok());
    let credited = fee(&mut w, &mint, ZEC1);
    w.b.wait(DAY);
    assert!(w.unstake(&h[0], &mint, 0).is_ok());
    assert!(w.unstake(&h[1], &mint, 0).is_ok());
    let (a, b) = (w.zec_of(&h[0].pubkey()) - 100 * ZEC1 + ZEC1 / 10, w.zec_of(&h[1].pubkey()) - 100 * ZEC1 + ZEC1 / 10);
    assert!(credited / 4 - a <= 1, "{a}");
    assert!(credited * 3 / 4 - b <= 1, "{b}");
    assert!(a + b <= credited);
    w.assert_sound(&mint);
}

#[test]
fn stakers_share_by_time() {
    let (mut w, mint, h) = setup(2);
    assert!(w.stake(&h[0], &mint, 1_000_000).is_ok());
    let credited = fee(&mut w, &mint, ZEC1);
    // B joins with the same amount half way through the release.
    w.b.wait(DAY / 2);
    assert!(w.stake(&h[1], &mint, 1_000_000).is_ok());
    w.b.wait(DAY / 2);
    assert!(w.unstake(&h[0], &mint, 0).is_ok());
    assert!(w.unstake(&h[1], &mint, 0).is_ok());
    let base = 100 * ZEC1 - ZEC1 / 10;
    let (a, b) = (w.zec_of(&h[0].pubkey()) - base, w.zec_of(&h[1].pubkey()) - base);
    // A alone for the first half, then half each of the second half.
    assert!((credited * 3 / 4).abs_diff(a) <= 2, "{a}");
    assert!((credited / 4).abs_diff(b) <= 2, "{b}");
    assert!(a + b <= credited);
}

#[test]
fn deposit_split_withdraw_in_one_transaction_earns_nothing() {
    let (mut w, mint, h) = setup(2);
    // An honest staker has been in for a while; fee is waiting, not yet split.
    assert!(w.stake(&h[0], &mint, 1_000).is_ok());
    w.b.wait(DAY);
    w.b.give_zec(&kit::authority(&mint), 10 * ZEC1);

    // The attacker stakes a far larger amount, triggers the split, and leaves.
    let attacker = &h[1];
    let big = w.coins_of(&attacker.pubkey(), &mint);
    let zec_before = w.zec_of(&attacker.pubkey());
    let ixs =
        [w.stake_ix(&attacker.pubkey(), &mint, big), w.split_ix(&mint), w.unstake_ix(&attacker.pubkey(), &mint, big)];
    let r = w.b.send_as(Some(attacker), &ixs, &[]);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.zec_of(&attacker.pubkey()), zec_before, "the attacker earned nothing");
    assert_eq!(w.coins_of(&attacker.pubkey(), &mint), big);

    // The honest staker still gets everything, over the release time.
    w.b.wait(DAY);
    let pot = w.ledger(&mint).pots[4];
    assert!(w.unstake(&h[0], &mint, 0).is_ok());
    assert!(pot - (w.zec_of(&h[0].pubkey()) - (100 * ZEC1 - ZEC1 / 10)) <= 1);
}

#[test]
fn a_short_visit_earns_only_for_its_time() {
    let (mut w, mint, h) = setup(2);
    assert!(w.stake(&h[0], &mint, 1_000_000).is_ok());
    let credited = fee(&mut w, &mint, 10 * ZEC1);
    // The attacker arrives with the same amount for one minute.
    assert!(w.stake(&h[1], &mint, 1_000_000).is_ok());
    w.b.wait(60);
    let before = w.zec_of(&h[1].pubkey());
    assert!(w.unstake(&h[1], &mint, 1_000_000).is_ok());
    let got = w.zec_of(&h[1].pubkey()) - before;
    // Half of one minute's release, no more.
    let fair = credited as u128 * 60 / DAY as u128 / 2;
    assert!(got as u128 <= fair, "got {got}, fair share {fair}");
}

#[test]
fn dust_deposits_earn_no_more_than_their_share() {
    let (mut w, mint, h) = setup(2);
    assert!(w.stake(&h[0], &mint, 1_000_000_000).is_ok());
    // Many tiny deposits by the same person.
    for _ in 0..20 {
        assert!(w.stake(&h[1], &mint, 1).is_ok());
    }
    let credited = fee(&mut w, &mint, ZEC1);
    w.b.wait(DAY);
    let before = w.zec_of(&h[1].pubkey());
    // And many tiny claims.
    for _ in 0..20 {
        assert!(w.unstake(&h[1], &mint, 1).is_ok());
    }
    let got = w.zec_of(&h[1].pubkey()) - before;
    let fair = credited as u128 * 20 / 1_000_000_020;
    assert!(got as u128 <= fair, "got {got}, fair share {fair}");
    w.assert_sound(&mint);
}

#[test]
fn a_backlog_waits_for_the_first_staker_and_then_runs_its_course() {
    let (mut w, mint, h) = setup(2);
    // Fee arrives for ten days with nobody in the vault.
    let credited = fee(&mut w, &mint, ZEC1);
    w.b.wait(10 * DAY);
    // The first arrival gets nothing at once.
    assert!(w.stake(&h[0], &mint, 1_000).is_ok());
    let before = w.zec_of(&h[0].pubkey());
    assert!(w.unstake(&h[0], &mint, 0).is_ok());
    assert_eq!(w.zec_of(&h[0].pubkey()), before);
    // After a quarter of the release time, a quarter.
    w.b.wait(DAY / 4);
    assert!(w.unstake(&h[0], &mint, 0).is_ok());
    let got = w.zec_of(&h[0].pubkey()) - before;
    assert!((credited / 4).abs_diff(got) <= 1, "{got}");
    // Someone joining now shares what is left.
    assert!(w.stake(&h[1], &mint, 1_000).is_ok());
    w.b.wait(DAY);
    assert!(w.unstake(&h[0], &mint, 1_000).is_ok());
    assert!(w.unstake(&h[1], &mint, 1_000).is_ok());
    let a = w.zec_of(&h[0].pubkey()) - before;
    let b = w.zec_of(&h[1].pubkey()) - (100 * ZEC1 - ZEC1 / 10);
    assert!((credited * 5 / 8).abs_diff(a) <= 2, "{a}");
    assert!((credited * 3 / 8).abs_diff(b) <= 2, "{b}");
}

#[test]
fn when_the_last_staker_leaves_nothing_is_stranded() {
    let (mut w, mint, h) = setup(2);
    assert!(w.stake(&h[0], &mint, 7).is_ok());
    assert!(w.stake(&h[1], &mint, 3).is_ok());
    fee(&mut w, &mint, 1_000_003);
    w.b.wait(DAY / 3);
    assert!(w.unstake(&h[0], &mint, 7).is_ok());
    assert!(w.unstake(&h[1], &mint, 3).is_ok());
    let l = w.ledger(&mint);
    assert_eq!(l.vault_share.total_staked, 0);
    // All that is left in the pot (unreleased queue and rounding dust) is queued again.
    assert!(l.pots[4] > 0);
    assert_eq!(l.vault_share.queued, l.pots[4]);
    // The next staker can earn every unit of it.
    assert!(w.stake(&h[0], &mint, 5).is_ok());
    w.b.wait(DAY);
    assert!(w.unstake(&h[0], &mint, 5).is_ok());
    assert_eq!(w.ledger(&mint).pots[4], 0);
    w.assert_sound(&mint);
}

#[test]
fn nobody_takes_out_more_than_they_put_in_or_anothers_coins() {
    let (mut w, mint, h) = setup(2);
    assert!(w.stake(&h[0], &mint, 500).is_ok());
    assert!(w.stake(&h[1], &mint, 900).is_ok());
    let r = w.unstake(&h[0], &mint, 501);
    assert_eq!(custom_code(&r), Some(err(E::NotEnoughStaked)));

    // B signs, but passes A's stake record.
    let mut acc = w.unstake_accounts(&h[1].pubkey(), &mint);
    acc.stake = kit::stake(&mint, &h[0].pubkey());
    let r = w.b.send_as(Some(&h[1]), &[kit::raw(acc, burr::instruction::Unstake { amount: 500 })], &[]);
    assert!(r.is_err());
    // A's record with A named as owner, but A has not signed.
    let acc = w.unstake_accounts(&h[0].pubkey(), &mint);
    let mut i = kit::raw(acc, burr::instruction::Unstake { amount: 500 });
    i.accounts[0].is_signer = false;
    let r = w.b.send_as(Some(&h[1]), &[i], &[]);
    assert!(r.is_err());
    // B sends A's coins to B's own account.
    let mut acc = w.unstake_accounts(&h[0].pubkey(), &mint);
    acc.owner_coin = ata(&h[1].pubkey(), &mint, &TOKEN_2022);
    let r = w.b.send_as(Some(&h[0]), &[kit::raw(acc, burr::instruction::Unstake { amount: 500 })], &[]);
    assert!(r.is_err(), "coins may only go back to an account of the staker");

    assert!(w.unstake(&h[0], &mint, 500).is_ok());
    assert!(w.unstake(&h[1], &mint, 900).is_ok());
    assert_eq!(w.b.token_balance(&kit::stake_vault(&mint)), 0);
}

#[test]
fn staking_needs_the_hook_and_a_real_amount() {
    let mut w = World::new();
    let mint = w.launch(&LaunchPlan { shares: [10_000, 0, 0, 0], ..Default::default() });
    let t = w.trader(10 * ZEC1);
    assert!(w.buy(&t, &mint, ZEC1 / 10).is_ok());
    let r = w.stake(&t, &mint, 10);
    assert_eq!(custom_code(&r), Some(err(E::HookNotChosen)));

    let (mut w, mint, h) = setup(1);
    let r = w.stake(&h[0], &mint, 0);
    assert_eq!(custom_code(&r), Some(err(E::ZeroAmount)));
    let have = w.coins_of(&h[0].pubkey(), &mint);
    assert!(w.stake(&h[0], &mint, have + 1).is_err(), "cannot stake coins one does not have");
}

#[test]
fn vault_share_refuses_another_coins_accounts() {
    let (mut w, mint, h) = setup(1);
    let other = w.launch(&vs_plan());
    assert!(w.buy(&h[0], &other, ZEC1 / 10).is_ok());
    assert!(w.stake(&h[0], &other, 1_000).is_ok());
    assert!(w.stake(&h[0], &mint, 1_000).is_ok());
    fee(&mut w, &mint, ZEC1);
    w.b.wait(DAY);
    type Mix = fn(&mut burr::accounts::UnstakeCoins, &Pubkey, &Pubkey);
    let mixes: Vec<(&str, Mix)> = vec![
        ("ledger", |a, o, _| a.ledger = kit::ledger(o)),
        ("coin", |a, o, _| a.coin = kit::coin(o)),
        ("vault", |a, o, _| a.vault = kit::vault(o)),
        ("stake_vault", |a, o, _| a.stake_vault = kit::stake_vault(o)),
        ("stake", |a, o, me| a.stake = kit::stake(o, me)),
        ("authority", |a, o, _| a.authority = kit::authority(o)),
    ];
    for (name, mix) in mixes {
        let mut acc = w.unstake_accounts(&h[0].pubkey(), &mint);
        mix(&mut acc, &other, &h[0].pubkey());
        let r = w.b.send_as(Some(&h[0]), &[kit::raw(acc, burr::instruction::Unstake { amount: 1 })], &[]);
        assert!(r.is_err(), "mixing in another coin's {name} was accepted");
    }
    assert!(w.unstake(&h[0], &mint, 1_000).is_ok());
}

#[test]
fn coins_sent_straight_to_the_stake_vault_give_nobody_a_claim() {
    let (mut w, mint, h) = setup(2);
    assert!(w.stake(&h[0], &mint, 1_000).is_ok());
    let sv = kit::stake_vault(&mint);
    let auth = kit::authority(&mint);
    w.b.write_token_account(sv, &mint, &auth, 1_000 + 999_999, &TOKEN_2022);
    assert_eq!(w.ledger(&mint).vault_share.total_staked, 1_000);
    // Nobody can take the extra out.
    let r = w.unstake(&h[0], &mint, 1_001);
    assert_eq!(custom_code(&r), Some(err(E::NotEnoughStaked)));
    assert!(w.stake(&h[1], &mint, 10).is_ok());
    let r = w.unstake(&h[1], &mint, 11);
    assert_eq!(custom_code(&r), Some(err(E::NotEnoughStaked)));
}

/// A long random sequence of stakes, unstakes, fees and waits. After every
/// step the vault must cover the ledger, and at the end, with everyone out,
/// the total paid must not exceed the total credited.
#[test]
fn random_sequences_never_pay_out_more_than_came_in() {
    for seed in 1u64..=6 {
        let (mut w, mint, h) = setup(4);
        let base: Vec<u64> = h.iter().map(|k| w.zec_of(&k.pubkey())).collect();
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut next = |m: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % m
        };
        let mut credited = 0u64;
        for _ in 0..120 {
            let who = next(4) as usize;
            match next(6) {
                0 | 1 => {
                    let have = w.coins_of(&h[who].pubkey(), &mint);
                    let amt = 1 + next(have.max(2) / 2);
                    if amt <= have {
                        let r = w.stake(&h[who], &mint, amt);
                        assert!(r.is_ok(), "{}", logs(&r));
                    }
                }
                2 => {
                    let staked = w.stake_of(&mint, &h[who].pubkey()).map(|s| s.amount).unwrap_or(0);
                    if staked > 0 {
                        let r = w.unstake(&h[who], &mint, next(staked + 1));
                        assert!(r.is_ok(), "{}", logs(&r));
                    }
                }
                3 => credited += fee(&mut w, &mint, 1 + next(3 * ZEC1)),
                4 => w.b.wait(1 + next(DAY as u64 / 2) as i64),
                _ => {
                    if w.stake_of(&mint, &h[who].pubkey()).is_some() {
                        assert!(w.unstake(&h[who], &mint, 0).is_ok());
                    }
                }
            }
            w.assert_sound(&mint);
            let l = w.ledger(&mint);
            assert!(l.vault_share.queued <= l.pots[4]);
        }
        // Everyone leaves.
        w.b.wait(2 * DAY);
        let mut paid = 0u64;
        for (i, k) in h.iter().enumerate() {
            if let Some(s) = w.stake_of(&mint, &k.pubkey()) {
                let r = w.unstake(k, &mint, s.amount);
                assert!(r.is_ok(), "{}", logs(&r));
            }
            paid += w.zec_of(&k.pubkey()) - base[i];
        }
        let l = w.ledger(&mint);
        assert_eq!(l.vault_share.total_staked, 0);
        assert!(paid <= credited, "seed {seed}: paid {paid} of {credited}");
        assert_eq!(paid + l.pots[4], credited, "seed {seed}: every unit is either paid or still in the pot");
        assert_eq!(w.b.token_balance(&kit::stake_vault(&mint)), 0);
        w.assert_sound(&mint);
    }
}

#[test]
fn what_the_last_staker_leaves_starts_its_release_afresh() {
    let (mut w, mint, h) = setup(2);
    assert!(w.stake(&h[0], &mint, 1_000).is_ok());
    let credited = fee(&mut w, &mint, ZEC1);
    // Three quarters of the way through, the only staker leaves and another
    // arrives in the same second.
    w.b.wait(DAY * 3 / 4);
    assert!(w.unstake(&h[0], &mint, 1_000).is_ok());
    let left = w.ledger(&mint).pots[4];
    assert!((credited / 4).abs_diff(left) <= 2);
    assert!(w.stake(&h[1], &mint, 1_000).is_ok());
    // A quarter of a day later they have a quarter of what was left, not all of it.
    w.b.wait(DAY / 4);
    let before = w.zec_of(&h[1].pubkey());
    assert!(w.unstake(&h[1], &mint, 0).is_ok());
    let got = w.zec_of(&h[1].pubkey()) - before;
    assert!((left / 4).abs_diff(got) <= 2, "got {got} of {left}");
}

#[test]
fn a_staker_who_gave_their_zec_account_away_can_still_take_coins_out() {
    let (mut w, mint, h) = setup(1);
    let other = w.b.wallet();
    assert!(w.stake(&h[0], &mint, 1_000).is_ok());
    fee(&mut w, &mint, ZEC1);
    w.b.wait(DAY);
    let z = ata(&h[0].pubkey(), &crate::pump::ZEC, &crate::pump::TOKEN);
    let i = kit::give_token_account_away(&z, &h[0].pubkey(), &other.pubkey());
    assert!(w.b.send_as(Some(&h[0]), &[i], &[]).is_ok());
    let coins = w.coins_of(&h[0].pubkey(), &mint);
    let r = w.unstake(&h[0], &mint, 1_000);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.coins_of(&h[0].pubkey(), &mint), coins + 1_000);
}

#[test]
fn an_empty_stake_record_can_be_closed_by_its_owner_only() {
    let (mut w, mint, h) = setup(2);
    assert!(w.stake(&h[0], &mint, 1_000).is_ok());
    fee(&mut w, &mint, ZEC1);
    w.b.wait(DAY / 2);
    // Not while coins are in it.
    let r = w.close_stake(&h[0], &mint);
    assert_eq!(custom_code(&r), Some(err(E::StakeNotEmpty)));
    assert!(w.unstake(&h[0], &mint, 1_000).is_ok());
    // Not by someone else, even naming the owner.
    let mut i = kit::raw(
        burr::accounts::CloseStake { owner: h[0].pubkey(), mint, stake: kit::stake(&mint, &h[0].pubkey()) },
        burr::instruction::CloseStake {},
    );
    i.accounts[0].is_signer = false;
    assert!(w.b.send_as(Some(&h[1]), &[i], &[]).is_err());
    let i = kit::raw(
        burr::accounts::CloseStake { owner: h[1].pubkey(), mint, stake: kit::stake(&mint, &h[0].pubkey()) },
        burr::instruction::CloseStake {},
    );
    assert!(w.b.send_as(Some(&h[1]), &[i], &[]).is_err());
    // By its owner: the rent comes back.
    let sol = w.b.svm.get_balance(&h[0].pubkey()).unwrap();
    let r = w.close_stake(&h[0], &mint);
    assert!(r.is_ok(), "{}", logs(&r));
    assert!(w.b.svm.get_balance(&h[0].pubkey()).unwrap() > sol);
    assert!(w.stake_of(&mint, &h[0].pubkey()).is_none());
    // And they can come back later.
    assert!(w.stake(&h[0], &mint, 10).is_ok());
    assert_eq!(w.stake_of(&mint, &h[0].pubkey()).unwrap().amount, 10);
    assert!(w.unstake(&h[0], &mint, 10).is_ok());
    w.assert_sound(&mint);
}
