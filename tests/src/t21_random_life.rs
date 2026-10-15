//! Random lives of many coins at once, for the Liquidity hook upgrade: coins
//! are launched along the way in all three forms (with random shares and
//! payout choices), traded on the curve and the pool, graduated, collected,
//! split, paid out, burned, used to buy $BURR and to add liquidity, in any
//! order. After every step, for every coin:
//! - its record is byte for byte as it was launched (no receiver ever changes);
//! - the vault holds at least what the books say, and no pot went below zero
//!   (they are unsigned: an underflow would have failed the call);
//! - every pot's whole history closes to the unit: credited = still there +
//!   paid out by its one way out;
//! - after a split that went through, nothing is left unsplit.
//!
//! BURRPAD_DEEP=<n> runs n lives (the ordinary suite runs 16).

use crate::bench::logs;
use crate::kit::{self, LaunchPlan, World};
use burrpad::state::Payee;
use burrpad::BURR_MINT;
use proptest::prelude::*;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

const ZEC1: u64 = 100_000_000;
const MAX_COINS: usize = 6;

#[derive(Debug, Clone)]
struct Step {
    op: u8,
    coin: u8,
    seed: u64,
}

fn steps() -> impl Strategy<Value = Vec<Step>> {
    proptest::collection::vec(
        (0u8..13, 0u8..8, any::<u64>()).prop_map(|(op, coin, seed)| Step { op, coin, seed }),
        40..120,
    )
}

struct Coin {
    mint: Pubkey,
    plan: LaunchPlan,
    record: Vec<u8>,
}

struct Life {
    w: World,
    coins: Vec<Coin>,
    payout_wallet: Pubkey,
    trader: Keypair,
    /// What the launches, burns and adds did, so a run shows it did real work.
    did: [u32; 6],
}

/// Six shares (four hooks, $BURR, liquidity) adding to 10,000, some of them zero, from a seed.
fn shares_from(seed: u64, burr: bool, liquidity: bool) -> ([u16; 4], u16, u16) {
    let mut w = [0u64; 6];
    for (i, x) in w.iter_mut().enumerate() {
        *x = (seed >> (i * 8)) & 0xff;
        if (seed >> (48 + i)) & 1 == 1 {
            *x = 0;
        }
    }
    if !burr {
        w[4] = 0;
    }
    if !liquidity {
        w[5] = 0;
    }
    if w.iter().sum::<u64>() == 0 {
        w[0] = 1;
    }
    let total: u64 = w.iter().sum();
    let mut s: Vec<u16> = w.iter().map(|x| (x * 10_000 / total) as u16).collect();
    let short = 10_000 - s.iter().map(|x| *x as u32).sum::<u32>() as u16;
    let first = s.iter().position(|x| *x > 0).unwrap();
    s[first] += short;
    ([s[0], s[1], s[2], s[3]], s[4], s[5])
}

impl Life {
    fn open() -> Self {
        let mut w = World::new();
        w.make_burr();
        let payout_wallet = w.open_private_payouts().pubkey();
        let trader = w.trader(1_000_000 * ZEC1);
        let mut life = Life { w, coins: vec![], payout_wallet, trader, did: [0; 6] };
        life.launch(1);
        // A launch_v3 with every share and every payout choice.
        life.launch(0x0700_4040_4040_4041);
        life
    }

    fn launch(&mut self, seed: u64) {
        if self.coins.len() >= MAX_COINS {
            return;
        }
        let form = seed % 3;
        let (shares, burr_share, liquidity_share) = shares_from(seed >> 2, form >= 1, form == 2);
        let mut plan = LaunchPlan {
            shares,
            burr_share,
            liquidity_share,
            creator_receiver: Pubkey::new_unique(),
            give_receiver: Pubkey::new_unique(),
            fee_bps: [50u16, 200, 300][(seed >> 60) as usize % 3],
            ..Default::default()
        };
        if form == 2 {
            // Random payout choices, where they make sense.
            let bits = (seed >> 56) as u8 & 7;
            if bits & 1 == 1 && shares[0] > 0 {
                plan.payout_options |= 1;
                plan.creator_receiver = self.payout_wallet;
            }
            if shares[3] > 0 {
                plan.payout_options |= bits & 6;
            }
            if plan.payout_options & 4 != 0 {
                plan.long_holder_step_bps = 100;
                plan.long_holder_max_bps = 10_000;
            }
        }
        let (mint, r) = self.w.try_launch(&plan);
        assert!(
            r.is_ok(),
            "a valid plan was refused (seed {seed}, shares {:?}, $BURR {}, liquidity {}, choices {})\n{}",
            plan.shares,
            plan.burr_share,
            plan.liquidity_share,
            plan.payout_options,
            logs(&r)
        );
        let record = self.w.b.account(&kit::coin(&mint)).unwrap().data;
        self.coins.push(Coin { mint, plan, record });
        self.did[0] += 1;
    }

    fn graduated(&self, m: &Pubkey) -> bool {
        self.w.b.account(&crate::pump::pool(m)).is_some()
    }

    fn step(&mut self, s: &Step) {
        let i = s.coin as usize % self.coins.len();
        let m = self.coins[i].mint;
        let grad = self.graduated(&m);
        let t = self.trader.insecure_clone();
        let amount = s.seed % (3 * ZEC1) + 1;
        match s.op {
            0 => self.launch(s.seed),
            1 => {
                let _ = if grad { self.w.pool_buy(&t, &m, amount) } else { self.w.buy(&t, &m, amount.min(ZEC1 / 4)) };
            }
            2 => {
                let part = self.w.coins_of(&t.pubkey(), &m) / (1 + s.seed % 4);
                if part > 0 {
                    let _ = if grad { self.w.pool_sell(&t, &m, part) } else { self.w.sell(&t, &m, part) };
                }
            }
            3 => {
                let _ = self.w.collect_curve(&m);
                if grad {
                    let _ = self.w.collect_pool(&m);
                }
            }
            4 => {
                // Fee arriving straight in the vault, then split: afterwards nothing is unsplit.
                self.w.b.give_zec(&kit::authority(&m), amount);
                let r = self.w.split(&m);
                assert!(r.is_ok(), "{}", logs(&r));
                assert_eq!(self.w.vault_balance(&m), self.w.ledger(&m).accounted, "ZEC left unsplit after a split");
            }
            5 => {
                let payee = [Payee::Platform, Payee::Creator, Payee::Give][s.seed as usize % 3];
                let _ = self.w.pay_out(&m, payee);
            }
            6 => {
                let before = self.w.ledger(&m).pots[3];
                let _ = if grad { self.w.burn_pool(&m) } else { self.w.burn_curve(&m) };
                self.did[1] += (self.w.ledger(&m).pots[3] < before) as u32;
            }
            7 => {
                let before = self.w.ledger(&m).burr.pot;
                let _ = self.w.burn_burr(&m);
                self.did[2] += (self.w.ledger(&m).burr.pot < before) as u32;
            }
            8 => {
                let before = self.w.ledger(&m).liquidity.pot;
                let _ = self.w.add_liquidity(&m);
                self.did[3] += (self.w.ledger(&m).liquidity.pot < before) as u32;
            }
            9 => {
                // Graduation: a whale finishes the curve, then pump.fun's migration.
                if !grad {
                    let whale = self.w.trader(10_000 * ZEC1);
                    let _ = self.w.buy(&whale, &m, 20 * ZEC1);
                    if self.w.b.curve(&m).complete {
                        self.w.b.svm.airdrop(&crate::pump::WITHDRAW_AUTHORITY, 1_000_000_000).unwrap();
                        let r = self.w.b.send_as(Some(&whale), &[crate::pump::migrate_v2(&m, &whale.pubkey())], &[]);
                        assert!(r.is_ok(), "{}", logs(&r));
                        self.did[4] += 1;
                    }
                }
            }
            10 => {
                // $BURR's own life goes on too: its pool is traded.
                let _ = self.w.pool_buy(&t, &BURR_MINT, amount);
            }
            _ => self.w.b.wait([1, 7, 30, 61, 400, 3_600][s.seed as usize % 6]),
        }
    }

    fn check(&self, at: &str) {
        let w = &self.w;
        let (mut platform_due, mut private_due) = (0u128, 0u128);
        for (i, c) in self.coins.iter().enumerate() {
            let m = &c.mint;
            assert_eq!(w.b.account(&kit::coin(m)).unwrap().data, c.record, "{at}: coin {i}'s record changed");
            w.assert_sound(m);
            let l = w.ledger(m);
            let pots: u128 =
                l.pots.iter().map(|p| *p as u128).sum::<u128>() + l.burr.pot as u128 + l.liquidity.pot as u128;
            let credited: u128 = l.credited.iter().sum::<u128>() + l.burr.credited + l.liquidity.credited;
            assert_eq!(
                l.accounted as u128 - pots,
                l.total_split - credited,
                "{at}: coin {i}: the carry does not match"
            );
            // Each pot closes: credited = still there + what its one way out paid.
            let paid_creator = l.credited[1] - l.pots[1] as u128;
            if c.plan.payout_options & 1 == 1 {
                private_due += paid_creator;
            } else {
                assert_eq!(
                    paid_creator,
                    w.zec_of(&c.plan.creator_receiver) as u128,
                    "{at}: coin {i}: the creator pot does not close"
                );
            }
            assert_eq!(
                l.credited[2],
                l.pots[2] as u128 + w.zec_of(&c.plan.give_receiver) as u128,
                "{at}: coin {i}: give"
            );
            assert_eq!(l.credited[3], l.pots[3] as u128 + l.burn.total_spent as u128, "{at}: coin {i}: burn");
            assert_eq!(l.credited[4], l.pots[4] as u128 + l.holders.total_released as u128, "{at}: coin {i}: holders");
            assert_eq!(l.burr.credited, l.burr.pot as u128 + l.burr.total_spent as u128, "{at}: coin {i}: $BURR");
            assert_eq!(
                l.liquidity.credited,
                l.liquidity.pot as u128 + l.liquidity.total_spent as u128,
                "{at}: coin {i}: liquidity"
            );
            platform_due += l.credited[0] - l.pots[0] as u128;
        }
        // The payout wallet holds exactly the private coins' creator payouts; the platform receiver, the platform pots'.
        assert_eq!(w.zec_of(&self.payout_wallet) as u128, private_due, "{at}: the payout wallet's ZEC does not match");
        assert_eq!(
            w.zec_of(&w.platform_receiver) as u128,
            platform_due,
            "{at}: the platform receiver's ZEC does not match"
        );
        w.assert_sound(&BURR_MINT);
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("BURRPAD_DEEP").ok().and_then(|n| n.parse().ok()).unwrap_or(16),
        max_shrink_iters: 40,
        ..ProptestConfig::default()
    })]

    #[test]
    fn many_coins_live_any_order_and_the_books_close_after_every_step(steps in steps()) {
        let mut life = Life::open();
        life.check("at the start");
        for (n, s) in steps.iter().enumerate() {
            life.step(s);
            life.check(&format!("after step {n}: {s:?}"));
        }
        if std::env::var("BURRPAD_DEEP").is_ok() {
            println!("coins {}, burns {}, $BURR buys {}, adds {}, graduations {}", life.did[0], life.did[1], life.did[2], life.did[3], life.did[4]);
        }
    }
}
