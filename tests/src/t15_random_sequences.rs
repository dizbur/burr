//! Long random orders of every instruction, by several actors, across
//! several coins, before and after graduation. After every single step the
//! books of every coin must close, and money may only have gone where a
//! coin's record, its ledger or the settings say.

use crate::kit::{self, LaunchPlan, World};
use crate::pump::{ata, TOKEN, ZEC};
use burrpad::state::Payee;
use proptest::prelude::*;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

const ZEC1: u64 = 100_000_000;
const DAY: i64 = 86_400;

#[derive(Debug, Clone)]
struct Step {
    op: u8,
    coin: u8,
    actor: u8,
    amount: u64,
}

fn steps() -> impl Strategy<Value = Vec<Step>> {
    proptest::collection::vec(
        (0u8..16, 0u8..3, 0u8..3, 1u64..=3 * ZEC1).prop_map(|(op, coin, actor, amount)| Step {
            op,
            coin,
            actor,
            amount,
        }),
        30..90,
    )
}

struct Books {
    w: World,
    coins: Vec<Pubkey>,
    plans: Vec<LaunchPlan>,
    records: Vec<Vec<u8>>,
    traders: Vec<Keypair>,
    thief: Keypair,
    thief_zec: u64,
    /// Every wallet that has ever been the rewards wallet.
    rewards: Vec<Keypair>,
    /// A proposed rewards wallet and when it was proposed.
    proposed: Option<(Keypair, i64)>,
    /// How many burns bought, releases went through, wallets were applied, fees were recovered: so a run can show it did real work.
    did: [u32; 4],
}

fn open() -> Books {
    let mut w = World::new();
    let mut coins = vec![];
    let mut plans = vec![];
    // Three coins with different fees and shares; the third has graduated. Two give part to burning $BURR.
    w.make_burr();
    for (fee_bps, shares, burr_share, graduated) in [
        (200u16, [2_000u16, 2_000, 2_000, 2_000], 2_000u16, false),
        (300, [0, 1_000, 4_000, 5_000], 0, false),
        (100, [4_000, 0, 3_000, 1_000], 2_000, true),
    ] {
        let plan = LaunchPlan { fee_bps, shares, burr_share, ..Default::default() };
        let mint = w.launch(&plan);
        if graduated {
            w.graduate(&mint);
        }
        coins.push(mint);
        plans.push(plan);
    }
    let records = coins.iter().map(|m| w.b.account(&kit::coin(m)).unwrap().data).collect();
    let traders = (0..3).map(|_| w.trader(100_000 * ZEC1)).collect();
    let thief = w.trader(ZEC1);
    let thief_zec = w.zec_of(&thief.pubkey());
    let rewards = vec![w.rewards_wallet.insecure_clone()];
    Books { w, coins, plans, records, traders, thief, thief_zec, rewards, proposed: None, did: [0; 4] }
}

impl Books {
    fn graduated(&self, m: &Pubkey) -> bool {
        self.w.b.curve(m).complete
    }

    /// Everything that must hold after any step whatsoever.
    fn check(&self, at: &str) {
        let w = &self.w;
        let mut released = 0u64;
        let mut platform_due = 0u128;
        for (i, m) in self.coins.iter().enumerate() {
            // The record never changes.
            assert_eq!(w.b.account(&kit::coin(m)).unwrap().data, self.records[i], "{at}: coin {i}'s record changed");
            // The vault covers the ledger; the pots add up; nothing is negative (they are unsigned, so: nothing wrapped).
            w.assert_sound(m);
            let l = w.ledger(m);
            let pots: u128 = l.pots.iter().map(|p| *p as u128).sum::<u128>() + l.burr.pot as u128;
            let credited: u128 = l.credited.iter().sum::<u128>() + l.burr.credited;
            assert_eq!(
                l.accounted as u128 - pots,
                l.total_split - credited,
                "{at}: coin {i}: the carry does not match"
            );
            // Every pot's whole history closes to the unit: credited = still there + paid out by its one way out.
            let plan = &self.plans[i];
            assert_eq!(
                l.credited[1],
                l.pots[1] as u128 + w.zec_of(&plan.creator_receiver) as u128,
                "{at}: coin {i}: the creator's pot does not close"
            );
            assert_eq!(
                l.credited[2],
                l.pots[2] as u128 + w.zec_of(&plan.give_receiver) as u128,
                "{at}: coin {i}: the give pot does not close"
            );
            assert_eq!(
                l.credited[3],
                l.pots[3] as u128 + l.burn.total_spent as u128,
                "{at}: coin {i}: the burn pot does not close"
            );
            assert_eq!(
                l.credited[4],
                l.pots[4] as u128 + l.holders.total_released as u128,
                "{at}: coin {i}: the holders' pot does not close"
            );
            assert_eq!(
                l.burr.credited,
                l.burr.pot as u128 + l.burr.total_spent as u128,
                "{at}: coin {i}: the $BURR pot does not close"
            );
            platform_due += l.credited[0] - l.pots[0] as u128;
            released += l.holders.total_released;
            // The burner keeps nothing between calls.
            let bn = kit::burner(m);
            assert_eq!(w.b.token_balance(&ata(&bn, &ZEC, &TOKEN)), 0, "{at}: coin {i}: ZEC left with the burner");
        }
        // The platform's receiver holds exactly what the platform pots paid out, over all coins.
        assert_eq!(
            w.zec_of(&w.platform_receiver) as u128,
            platform_due,
            "{at}: the platform's receiver does not close"
        );
        // All holder releases, of all coins, are in wallets that were the rewards wallet. Nowhere else.
        let held: u64 = self.rewards.iter().map(|k| w.zec_of(&k.pubkey())).sum();
        assert_eq!(held, released, "{at}: holder releases went somewhere other than a rewards wallet");
        // A bystander who never trades never gains.
        assert_eq!(w.zec_of(&self.thief.pubkey()), self.thief_zec, "{at}: the bystander's balance moved");
    }

    fn step(&mut self, s: &Step) {
        let m = self.coins[s.coin as usize % self.coins.len()];
        let t = self.traders[s.actor as usize % self.traders.len()].insecure_clone();
        let grad = self.graduated(&m);
        let before = self.w.ledger(&m);
        let now = self.w.b.now();
        match s.op {
            0 | 1 => {
                let amount = s.amount / 20 + 1;
                let _ = if grad { self.w.pool_buy(&t, &m, amount) } else { self.w.buy(&t, &m, amount.min(ZEC1 / 20)) };
            }
            2 => {
                let have = self.w.coins_of(&t.pubkey(), &m);
                let part = have / (1 + s.amount % 4);
                if part > 0 {
                    let _ = if grad { self.w.pool_sell(&t, &m, part) } else { self.w.sell(&t, &m, part) };
                }
            }
            3 => {
                let _ = self.w.collect_curve(&m);
            }
            4 => {
                let _ = self.w.collect_pool(&m);
            }
            5 => {
                // ZEC sent straight to a vault, then split.
                self.w.b.give_zec(&kit::authority(&m), s.amount % 5_000 + 1);
                let _ = self.w.split(&m);
            }
            6 => {
                let payee = [Payee::Platform, Payee::Creator, Payee::Give][s.actor as usize % 3];
                let _ = self.w.pay_out(&m, payee);
            }
            7 | 8 if s.amount.is_multiple_of(3) => {
                // A $BURR burn: one a minute over all coins, never above its limit, only from its own pot.
                let (quote, _) = self.w.pool_reserves(&burrpad::BURR_MINT);
                let shared = self.w.config().burr_buy;
                let recorded =
                    if shared.reference.quote == 0 { burrpad::BURR_START_RESERVE } else { shared.reference.quote };
                let fee =
                    (self.w.pool(&burrpad::BURR_MINT).creator_fee_bps as u128).min(self.w.config().max_fee_bps as u128);
                let r = self.w.burn_burr(&m);
                let after = self.w.ledger(&m);
                let spent = before.burr.pot - after.burr.pot;
                assert_eq!(after.pots, before.pots, "a $BURR burn touched another pot");
                if r.is_ok() {
                    assert!(now >= shared.ready_at, "a $BURR burn ran inside the shared wait");
                    let limit = quote.min(recorded) as u128 * fee / 40_000;
                    assert!(spent > 0 && spent as u128 <= limit, "a $BURR burn spent {spent}, limit {limit}");
                    self.did[0] += 1;
                } else {
                    assert_eq!(spent, 0);
                }
            }
            7 | 8 => {
                // A burn never spends more than its limit, nor sooner than its wait.
                let (quote, _) = if grad {
                    self.w.pool_reserves(&m)
                } else {
                    let c = self.w.b.curve(&m);
                    (c.virtual_quote_reserves, c.virtual_token_reserves)
                };
                let coin = self.w.coin(&m);
                let r = if grad { self.w.burn_pool(&m) } else { self.w.burn_curve(&m) };
                let after = self.w.ledger(&m);
                let spent = before.pots[3] - after.pots[3];
                if r.is_ok() {
                    assert!(
                        now >= before.burn.last_time + coin.burn_cooldown_secs as i64,
                        "a burn ran inside its wait"
                    );
                    let limit = quote.min(before.burn.reference.quote) as u128 * coin.fee_bps as u128 / 40_000;
                    assert!(spent as u128 <= limit, "a burn spent {spent}, above its limit {limit}");
                    self.did[0] += (spent > 0) as u32;
                } else {
                    assert_eq!(spent, 0);
                }
            }
            9 | 10 => {
                // A holders' release: only by the rewards wallet, never above the pot, never inside thirty minutes, never while paused.
                let cfg = self.w.config();
                let wallet = self
                    .rewards
                    .iter()
                    .find(|k| k.pubkey() == cfg.rewards_wallet)
                    .expect("a known rewards wallet")
                    .insecure_clone();
                let pot = before.pots[4];
                let amount = match s.amount % 4 {
                    0 => pot,
                    1 => pot / 2 + 1,
                    2 => pot + 1 + s.amount % 7,
                    _ => s.amount % 50_000,
                };
                let i = kit::raw(
                    self.w.release_accounts(&wallet.pubkey(), &m),
                    burrpad::instruction::ReleaseHolderRewards {
                        amount,
                        snapshot_slot: self.w.slot(),
                        snapshot_hash: [3u8; 32],
                    },
                );
                let r = self.w.b.send_as(Some(&wallet), &[i], &[]);
                let after = self.w.ledger(&m);
                if r.is_ok() {
                    assert!(!cfg.holder_releases_paused, "a release went through while paused");
                    assert!(amount > 0 && amount <= pot, "a release of {amount} from a pot of {pot}");
                    assert!(now >= before.holders.last_release + 1_800, "a release inside thirty minutes");
                    assert_eq!(before.pots[4] - after.pots[4], amount);
                    self.did[1] += 1;
                } else {
                    assert_eq!(after.pots[4], before.pots[4]);
                }
                // The bystander tries the same and gets nothing.
                let thief = self.thief.insecure_clone();
                let i = kit::raw(
                    self.w.release_accounts(&thief.pubkey(), &m),
                    burrpad::instruction::ReleaseHolderRewards {
                        amount: 1,
                        snapshot_slot: self.w.slot(),
                        snapshot_hash: [3u8; 32],
                    },
                );
                assert!(self.w.b.send_as(Some(&thief), &[i], &[]).is_err(), "a stranger released a holders' pot");
            }
            11 => {
                let paused = s.amount.is_multiple_of(2);
                let _ = self.w.as_admin(burrpad::instruction::SetHolderReleasesPaused { paused });
            }
            12 => {
                // The administrator's one way to holder money: a new rewards wallet, never sooner than a day after proposing it.
                if s.amount.is_multiple_of(3) || self.proposed.is_none() {
                    let next = self.w.b.wallet();
                    if self.w.as_admin(burrpad::instruction::ProposeRewardsWallet { wallet: next.pubkey() }).is_ok() {
                        self.proposed = Some((next, now));
                    }
                } else if s.amount % 3 == 1 {
                    if self.w.as_admin(burrpad::instruction::CancelRewardsWallet {}).is_ok() {
                        self.proposed = None;
                    }
                } else if self.w.as_admin(burrpad::instruction::ApplyRewardsWallet {}).is_ok() {
                    let (next, at) = self.proposed.take().expect("something was applied that was never proposed");
                    assert!(now >= at + DAY, "a rewards wallet was applied {} seconds after it was proposed", now - at);
                    assert_eq!(self.w.config().rewards_wallet, next.pubkey());
                    self.rewards.push(next);
                    self.did[2] += 1;
                }
            }
            13 => {
                // Time passes: a little, or a lot.
                let secs = [7, 61, 400, 1_801, 40_000, 90_000][s.amount as usize % 6];
                self.w.b.wait(secs);
            }
            14 => {
                // A stranger has a graduated coin's pool fee paid to a side account; anyone brings it home.
                if let Some(side) = grad.then(|| self.w.try_divert_pool_fee(&m)).flatten() {
                    let stuck = self.w.b.token_balance(&side);
                    let r = self.w.recover(&m, &side);
                    assert!(r.is_ok(), "recover failed: {}", crate::bench::logs(&r));
                    assert_eq!(self.w.b.token_balance(&side), 0);
                    assert_eq!(self.w.ledger(&m).accounted, before.accounted + stuck);
                    self.did[3] += (stuck > 0) as u32;
                }
            }
            _ => {
                // A stranger names themselves as a receiver; and tries one coin's accounts against another's vault.
                let thief = self.thief.insecure_clone();
                for payee in [Payee::Platform, Payee::Creator, Payee::Give] {
                    let acc = self.w.pay_out_accounts(&thief.pubkey(), &m, &thief.pubkey());
                    let i = kit::raw(acc, burrpad::instruction::PayOut { payee });
                    assert!(self.w.b.send_as(Some(&thief), &[i], &[]).is_err(), "a stranger was paid a pot");
                }
                let other = self.coins[(s.coin as usize + 1) % self.coins.len()];
                let receiver = self.plans[s.coin as usize % self.coins.len()].creator_receiver;
                let mut acc = self.w.pay_out_accounts(&thief.pubkey(), &m, &receiver);
                acc.vault = kit::vault(&other);
                let i = kit::raw(acc, burrpad::instruction::PayOut { payee: Payee::Creator });
                assert!(self.w.b.send_as(Some(&thief), &[i], &[]).is_err(), "one coin was paid from another's vault");
            }
        }
    }
}

proptest! {
    // 48 sequences in the ordinary suite. BURRPAD_DEEP=<n> runs n of them.
    #![proptest_config(ProptestConfig {
        cases: std::env::var("BURRPAD_DEEP").ok().and_then(|n| n.parse().ok()).unwrap_or(48),
        max_shrink_iters: 60,
        ..ProptestConfig::default()
    })]

    #[test]
    fn the_books_close_after_every_step_of_any_sequence(steps in steps()) {
        let mut b = open();
        b.check("at the start");
        for (n, s) in steps.iter().enumerate() {
            b.step(s);
            b.check(&format!("after step {n}: {s:?}"));
        }
        // At the end, bring everything in and pay everything out: nothing is lost on the way.
        b.w.b.wait(2_000);
        let coins = b.coins.clone();
        for m in &coins {
            let _ = b.w.collect_curve(m);
            let _ = b.w.collect_pool(m);
            for p in [Payee::Platform, Payee::Creator, Payee::Give] {
                prop_assert!(b.w.pay_out(m, p).is_ok());
            }
            let l = b.w.ledger(m);
            prop_assert_eq!(l.pots[0] + l.pots[1] + l.pots[2], 0);
        }
        b.check("at the end");
        if std::env::var("BURRPAD_DEEP").is_ok() {
            let [burns, releases, wallets, recovered] = b.did;
            println!("{} steps: {burns} burns bought, {releases} releases, {wallets} wallets applied, {recovered} fees recovered", steps.len());
        }
    }
}

// ---- the account sweep, widened ----
//
// Every instruction that touches a coin, every account in it, replaced in
// turn by: each other account of the same instruction (the same account
// passed twice); the matching account of another coin; an ordinary wallet;
// someone else's ZEC account and coin account; the other token program and a
// program that does nothing; and a look-alike: a copy of the real account,
// same owner, with extra bytes, at another address. Whatever each attempt
// does, the books must still close and nobody else's money may have moved.

use anchor_lang::{InstructionData, ToAccountMetas};
use solana_instruction::Instruction;

fn built(accounts: impl ToAccountMetas, data: impl InstructionData) -> Instruction {
    Instruction { program_id: kit::BURRPAD, accounts: accounts.to_account_metas(None), data: data.data() }
}

/// The instructions for one coin, each as (name, instruction, who signs).
fn instructions_for(
    b: &Books,
    m: &Pubkey,
    payer: &Keypair,
    graduated: bool,
) -> Vec<(&'static str, Instruction, Vec<Keypair>)> {
    let w = &b.w;
    let pk = payer.pubkey();
    let me = || vec![payer.insecure_clone()];
    let plan = &b.plans[b.coins.iter().position(|c| c == m).unwrap()];
    let wallet = b.rewards[0].insecure_clone();
    let pot = w.ledger(m).pots[4].max(1);
    let mut v = vec![
        ("split", w.split_ix(m), me()),
        ("collect_curve", built(w.collect_curve_accounts(&pk, m), burrpad::instruction::CollectCurve {}), me()),
        (
            "pay_out creator",
            built(
                w.pay_out_accounts(&pk, m, &plan.creator_receiver),
                burrpad::instruction::PayOut { payee: Payee::Creator },
            ),
            me(),
        ),
        (
            "pay_out platform",
            built(
                w.pay_out_accounts(&pk, m, &w.platform_receiver),
                burrpad::instruction::PayOut { payee: Payee::Platform },
            ),
            me(),
        ),
        (
            "release_holder_rewards",
            built(
                w.release_accounts(&wallet.pubkey(), m),
                burrpad::instruction::ReleaseHolderRewards {
                    amount: pot,
                    snapshot_slot: w.slot(),
                    snapshot_hash: [4u8; 32],
                },
            ),
            vec![wallet],
        ),
    ];
    if plan.burr_share > 0 {
        v.push(("burn_burr", built(w.burn_burr_accounts(&pk, m), burrpad::instruction::BurnBurr {}), me()));
    }
    if graduated {
        v.push(("collect_pool", built(w.collect_pool_accounts(&pk, m), burrpad::instruction::CollectPool {}), me()));
        v.push(("burn_pool", built(w.burn_pool_accounts(&pk, m), burrpad::instruction::BurnPool {}), me()));
        v.push(("recover", built(w.recover_accounts(m, &kit::vault(m)), burrpad::instruction::Recover {}), me()));
    } else {
        v.push(("burn_curve", built(w.burn_curve_accounts(&pk, m), burrpad::instruction::BurnCurve {}), me()));
    }
    v
}

#[test]
fn no_substituted_account_moves_money_wrongly() {
    let mut b = open();
    let payer = b.w.b.payer.insecure_clone();
    let other_wallet = b.w.b.wallet().pubkey();
    let nothing = b.w.b.do_nothing_program();
    let bystander = b.traders[0].insecure_clone();
    let coins = b.coins.clone();
    // Everyone trades a little so every pot and every fee vault has something in it.
    for m in &coins {
        let t = b.traders[1].insecure_clone();
        let grad = b.graduated(m);
        let r = if grad { b.w.pool_buy(&t, m, ZEC1) } else { b.w.buy(&t, m, ZEC1 / 20) };
        assert!(r.is_ok());
        let r = if grad { b.w.pool_buy(&bystander, m, ZEC1 / 10) } else { b.w.buy(&bystander, m, ZEC1 / 50) };
        assert!(r.is_ok());
        let _ = b.w.collect_curve(m);
        let _ = b.w.collect_pool(m);
    }
    b.w.b.wait(2_000);
    b.check("before the sweep");
    let mut tried = 0u32;
    let mut went_through = 0u32;

    for (ci, m) in coins.iter().enumerate() {
        let grad = b.graduated(m);
        let other = coins[(ci + 1) % coins.len()];
        let names: Vec<&'static str> = instructions_for(&b, m, &payer, grad).iter().map(|x| x.0).collect();
        for (n, name) in names.iter().enumerate() {
            let width = instructions_for(&b, m, &payer, grad)[n].1.accounts.len();
            for i in 0..width {
                // What position `i` can be replaced by.
                let (_, good, _) = instructions_for(&b, m, &payer, grad).remove(n);
                let mut subs: Vec<(String, Pubkey)> = (0..width)
                    .filter(|j| *j != i)
                    .map(|j| (format!("the same as account {j}"), good.accounts[j].pubkey))
                    .collect();
                // The matching account of another coin (if that coin has this instruction).
                if let Some((_, theirs, _)) =
                    instructions_for(&b, &other, &payer, b.graduated(&other)).into_iter().find(|x| x.0 == *name)
                {
                    subs.push(("another coin's".into(), theirs.accounts[i].pubkey));
                }
                subs.push(("an ordinary wallet".into(), other_wallet));
                subs.push(("someone else's ZEC account".into(), ata(&bystander.pubkey(), &ZEC, &TOKEN)));
                subs.push((
                    "someone else's coin account".into(),
                    ata(&bystander.pubkey(), m, &crate::pump::TOKEN_2022),
                ));
                subs.push(("the classic token program".into(), TOKEN));
                subs.push(("the other token program".into(), crate::pump::TOKEN_2022));
                subs.push(("a program that does nothing".into(), nothing));
                // A look-alike: the real account's contents and owner, a few extra bytes, at an address of the attacker's choosing.
                if let Some(mut real) = b.w.b.account(&good.accounts[i].pubkey) {
                    if !real.executable {
                        let fake = Pubkey::new_unique();
                        real.data.extend_from_slice(&[0u8; 8]);
                        real.lamports += 1_000_000_000;
                        b.w.b.svm.set_account(fake, real).unwrap();
                        subs.push(("a look-alike with extra data".into(), fake));
                    }
                }

                for (what, sub) in subs {
                    if sub == good.accounts[i].pubkey {
                        continue;
                    }
                    // A fresh, valid instruction each time; fees arriving and time passing so it could really run.
                    b.w.b.give_zec(&kit::authority(m), 1_000);
                    b.w.b.wait(61);
                    let (_, mut ix, signers) = instructions_for(&b, m, &payer, grad).remove(n);
                    ix.accounts[i].pubkey = sub;
                    // A signature can only come from a key we hold.
                    if !signers.iter().any(|k| k.pubkey() == sub) {
                        ix.accounts[i].is_signer = false;
                    }
                    let held: Vec<&Keypair> = signers
                        .iter()
                        .filter(|k| ix.accounts.iter().any(|a| a.is_signer && a.pubkey == k.pubkey()))
                        .collect();
                    let before = (
                        b.w.zec_of(&bystander.pubkey()),
                        b.w.coins_of(&bystander.pubkey(), m),
                        b.w.zec_of(&other_wallet),
                    );
                    let r = b.w.b.send_as(Some(&payer), &[ix], &held);
                    tried += 1;
                    went_through += r.is_ok() as u32;
                    let at = format!("coin {ci}, {name}, account {i} replaced by {what}");
                    if r.is_ok() {
                        println!("  went through: {at}");
                    }
                    b.check(&at);
                    let after = (
                        b.w.zec_of(&bystander.pubkey()),
                        b.w.coins_of(&bystander.pubkey(), m),
                        b.w.zec_of(&other_wallet),
                    );
                    assert_eq!(before, after, "{at}: someone else's balance moved");
                }
            }
        }
    }
    println!("{tried} substitutions tried; {went_through} went through, all harmlessly");
    assert!(tried > 2_000);
}
