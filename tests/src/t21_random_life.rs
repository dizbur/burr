//! Random lives of many coins at once, for the Liquidity hook upgrade: coins
//! are launched along the way in all three forms (with random shares and
//! payout choices, some with a first buy in the same transaction), traded on
//! the curve and the pool, graduated, collected, split, paid out, burned, used
//! to buy $BURR and to add liquidity, and have holder pots released, in any
//! order. $BURR's own coin lives alongside them.
//!
//! After every step, for every coin, $BURR's included:
//! - its record is byte for byte as it was launched (no receiver ever changes);
//! - **the vault holds exactly what the books account for.** In these lives every
//!   fee that arrives is split at once, so any call that moved a different
//!   amount of ZEC than it booked, by one unit, shows here;
//! - **each pot's credit is its share of everything split**, worked out by the
//!   test itself from the coin's recorded shares, not by the program's arithmetic;
//! - every pot's whole history closes to the unit: credited = still there +
//!   paid out by its one way out (receivers' balances, the payout wallet, the
//!   rewards wallet, the platform receiver, what burns and adds spent).
//!
//! A payout of a pot above zero must never be refused. Each life ends with a
//! closing round in which a payout, a holder release, a coin burn, a $BURR buy
//! and a liquidity add must each go through, so a life where nothing happened fails.
//!
//! BURRPAD_DEEP=<n> runs n lives (the ordinary suite runs 16).

use crate::bench::logs;
use crate::kit::{self, LaunchPlan, World};
use burrpad::state::Payee;
use burrpad::BURR_MINT;
use proptest::prelude::*;
use solana_instruction::AccountMeta;
use solana_keypair::Keypair;
use solana_message::AddressLookupTableAccount;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

const ZEC1: u64 = 100_000_000;
const MAX_COINS: usize = 7;
/// The seed that launches a launch_v3 with every share and every payout choice.
const EVERY_OPTION: u64 = 0x0700_4040_4040_4041;

#[derive(Debug, Clone)]
struct Step {
    op: u8,
    coin: u8,
    seed: u64,
}

fn steps() -> impl Strategy<Value = Vec<Step>> {
    proptest::collection::vec(
        (0u8..14, 0u8..8, any::<u64>()).prop_map(|(op, coin, seed)| Step { op, coin, seed }),
        40..120,
    )
}

struct Coin {
    mint: Pubkey,
    creator_receiver: Pubkey,
    give_receiver: Pubkey,
    private: bool,
    /// Priced in $BURR: its fee, pots and payouts are $BURR.
    burr: bool,
    record: Vec<u8>,
}

struct Life {
    w: World,
    coins: Vec<Coin>,
    payout_wallet: Pubkey,
    trader: Keypair,
    table: AddressLookupTableAccount,
    /// ZEC released to the rewards wallet, over all coins.
    /// Released to the rewards wallet: in ZEC, and in $BURR.
    released: [u128; 2],
    /// What went through: launches, payouts, releases, burns, $BURR buys, adds, graduations.
    did: [u32; 7],
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
        let table = w.b.address_table(&kit::fixed_accounts());
        let mut life = Life { w, coins: vec![], payout_wallet, trader, table, released: [0; 2], did: [0; 7] };
        // Coins priced in $BURR: the trader holds $BURR, and pump.fun's buy-back receiver a $BURR account.
        life.w.give_burr(&life.trader.pubkey(), 1 << 50);
        life.w.give_burr(&crate::pump::BUYBACK, 0);
        // $BURR's own coin, with its receivers as recorded.
        let c = life.w.coin(&BURR_MINT);
        let record = life.w.b.account(&kit::coin(&BURR_MINT)).unwrap().data;
        life.coins.push(Coin {
            mint: BURR_MINT,
            creator_receiver: c.creator_receiver,
            give_receiver: c.give_receiver,
            private: false,
            burr: false,
            record,
        });
        life.launch(1);
        life.launch(EVERY_OPTION);
        life
    }

    fn launch(&mut self, seed: u64) {
        if self.coins.len() >= MAX_COINS {
            return;
        }
        // About one launch in four is priced in $BURR: the four hooks and the $BURR share only.
        let burr_priced = (seed >> 48) & 3 == 0 && seed != EVERY_OPTION && seed != 1;
        let form = if burr_priced { seed % 2 } else { seed % 3 };
        let (shares, burr_share, liquidity_share) = shares_from(seed >> 2, form >= 1, form == 2);
        let mut plan = LaunchPlan {
            shares,
            burr_share,
            liquidity_share,
            creator_receiver: Pubkey::new_unique(),
            give_receiver: Pubkey::new_unique(),
            fee_bps: [50u16, 200, 300][(seed >> 60) as usize % 3],
            quote: if burr_priced { burrpad::QUOTE_BURR } else { burrpad::QUOTE_ZEC },
            ..Default::default()
        };
        if form == 2 {
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
        let mint = if (seed >> 40) & 1 == 1 && seed != EVERY_OPTION && !burr_priced {
            // A launch with a first buy in the same transaction, the launch making the buyer's account.
            let k = Keypair::new();
            let t = self.trader.insecure_clone();
            let mut launch = self.w.launch_ix(&t.pubkey(), &k.pubkey(), &plan);
            launch
                .accounts
                .push(AccountMeta::new(crate::pump::ata(&t.pubkey(), &k.pubkey(), &crate::pump::TOKEN_2022), false));
            let buy = crate::pump::buy_exact_quote_in_v3(&k.pubkey(), &t.pubkey(), ZEC1 / 10, 1);
            let (_, r) = self.w.b.send_with_table(&t, &[launch, buy], &[&k], &self.table);
            assert!(r.is_ok(), "a launch with a first buy was refused (seed {seed})\n{}", logs(&r));
            assert!(self.w.coins_of(&t.pubkey(), &k.pubkey()) > 0, "the first buy delivered");
            k.pubkey()
        } else {
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
            mint
        };
        let record = self.w.b.account(&kit::coin(&mint)).unwrap().data;
        self.coins.push(Coin {
            mint,
            creator_receiver: plan.creator_receiver,
            give_receiver: plan.give_receiver,
            private: plan.payout_options & 1 == 1,
            burr: burr_priced,
            record,
        });
        self.did[0] += 1;
    }

    fn graduated(&self, m: &Pubkey) -> bool {
        self.w.b.account(&crate::pump::pool_with(m, &self.w.quote(m).0)).is_some()
    }

    fn is_burr(&self, m: &Pubkey) -> bool {
        self.coins.iter().any(|c| c.mint == *m && c.burr)
    }

    fn graduate(&mut self, m: &Pubkey) {
        if self.graduated(m) {
            return;
        }
        if self.is_burr(m) {
            self.w.graduate(m);
            self.did[6] += 1;
            return;
        }
        let whale = self.w.trader(10_000 * ZEC1);
        let _ = self.w.buy(&whale, m, 20 * ZEC1);
        if self.w.b.curve(m).complete {
            self.w.b.svm.airdrop(&crate::pump::WITHDRAW_AUTHORITY, 1_000_000_000).unwrap();
            let r = self.w.b.send_as(Some(&whale), &[crate::pump::migrate_v2(m, &whale.pubkey())], &[]);
            assert!(r.is_ok(), "{}", logs(&r));
            self.did[6] += 1;
        }
    }

    /// A payout; if the pot holds anything and has a receiver, it must go through.
    fn pay(&mut self, m: &Pubkey, payee: Payee) -> bool {
        let l = self.w.ledger(m);
        let c = self.w.coin(m);
        let (pot, receiver) = match payee {
            Payee::Platform => (l.pots[0], c.platform_receiver),
            Payee::Creator => (l.pots[1], c.creator_receiver),
            Payee::Give => (l.pots[2], c.give_receiver),
        };
        let r = self.w.pay_out(m, payee);
        if pot > 0 && receiver != Pubkey::default() {
            assert!(r.is_ok(), "a payout of a pot of {pot} was refused\n{}", logs(&r));
            assert_eq!(self.w.ledger(m).pots[payee as usize], 0, "the payout did not empty its pot");
            self.did[1] += 1;
            return true;
        }
        false
    }

    /// A holder release by the rewards wallet, of `part` of the pot (at least one unit).
    fn release(&mut self, m: &Pubkey, part: u64) -> bool {
        let pot = self.w.ledger(m).pots[4];
        if pot == 0 {
            return false;
        }
        let amount = (pot / part.max(1)).max(1);
        let wallet = self.w.rewards_wallet.insecure_clone();
        let i = kit::raw(
            self.w.release_accounts(&wallet.pubkey(), m),
            burrpad::instruction::ReleaseHolderRewards {
                amount,
                snapshot_slot: self.w.slot(),
                snapshot_hash: [3u8; 32],
            },
        );
        let r = self.w.b.send_as(Some(&wallet), &[i], &[]);
        if r.is_ok() {
            self.released[self.is_burr(m) as usize] += amount as u128;
            self.did[2] += 1;
        }
        r.is_ok()
    }

    fn step(&mut self, s: &Step) {
        let i = s.coin as usize % self.coins.len();
        let m = self.coins[i].mint;
        let grad = self.graduated(&m);
        let t = self.trader.insecure_clone();
        // A $BURR coin's amounts are in $BURR units, a thousand to the ZEC unit here.
        let amount = (s.seed % (3 * ZEC1) + 1) * if self.is_burr(&m) { 1_000 } else { 1 };
        match s.op {
            0 => self.launch(s.seed),
            1 => {
                let cap = if self.is_burr(&m) { 1_000 * ZEC1 / 4 } else { ZEC1 / 4 };
                let _ = if grad { self.w.pool_buy(&t, &m, amount) } else { self.w.buy(&t, &m, amount.min(cap)) };
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
                if self.is_burr(&m) {
                    self.w.give_burr(&kit::authority(&m), amount);
                } else {
                    self.w.b.give_zec(&kit::authority(&m), amount);
                }
                let r = self.w.split(&m);
                assert!(r.is_ok(), "{}", logs(&r));
            }
            5 => {
                let payee = [Payee::Platform, Payee::Creator, Payee::Give][s.seed as usize % 3];
                self.pay(&m, payee);
            }
            6 => {
                let before = self.w.ledger(&m).pots[3];
                let _ = if grad { self.w.burn_pool(&m) } else { self.w.burn_curve(&m) };
                self.did[3] += (self.w.ledger(&m).pots[3] < before) as u32;
            }
            7 => {
                let before = self.w.ledger(&m).burr.pot;
                if self.is_burr(&m) {
                    // A $BURR coin's $BURR pot is burned as it is: whenever it holds anything.
                    let r = self.w.burn_burr_pot(&m);
                    assert_eq!(r.is_ok(), before > 0 && self.w.coin(&m).burr_share > 0, "{}", logs(&r));
                } else {
                    let _ = self.w.burn_burr(&m);
                }
                self.did[4] += (self.w.ledger(&m).burr.pot < before) as u32;
            }
            8 => {
                let before = self.w.ledger(&m).liquidity.pot;
                let _ = self.w.add_liquidity(&m);
                self.did[5] += (self.w.ledger(&m).liquidity.pot < before) as u32;
            }
            9 => self.graduate(&m),
            10 => {
                let _ = self.w.pool_buy(&t, &BURR_MINT, amount);
            }
            11 => {
                self.release(&m, 1 + s.seed % 3);
            }
            _ => self.w.b.wait([1, 7, 30, 61, 400, 1_801, 3_600][s.seed as usize % 7]),
        }
    }

    /// The end of every life: on the coin with every option, each kind of money call must go through.
    fn close(&mut self) {
        let m = self.coins[2].mint;
        self.w.b.wait(3_600);
        let _ = self.w.collect_curve(&m);
        self.w.b.give_zec(&kit::authority(&m), 20 * ZEC1);
        assert!(self.w.split(&m).is_ok());
        self.check("closing: after the fee");
        for payee in [Payee::Platform, Payee::Give] {
            assert!(self.pay(&m, payee), "closing: the {payee:?} payout did not go through");
        }
        assert!(self.pay(&m, Payee::Creator), "closing: the private creator payout did not go through");
        assert!(self.release(&m, 1), "closing: the holder release did not go through");
        self.check("closing: after the payouts");
        let pot = self.w.ledger(&m).pots[3];
        let _ = if self.graduated(&m) { self.w.burn_pool(&m) } else { self.w.burn_curve(&m) };
        assert!(self.w.ledger(&m).pots[3] < pot, "closing: the coin burn did not buy");
        self.w.b.wait(61);
        let pot = self.w.ledger(&m).burr.pot;
        let r = self.w.burn_burr(&m);
        assert!(self.w.ledger(&m).burr.pot < pot, "closing: the $BURR buy did not go through\n{}", logs(&r));
        self.check("closing: after the burns");
        self.graduate(&m);
        assert!(self.graduated(&m), "closing: the coin did not graduate");
        self.w.b.wait(61);
        let _ = self.w.add_liquidity(&m);
        self.w.b.wait(61);
        let pot = self.w.ledger(&m).liquidity.pot;
        let r = self.w.add_liquidity(&m);
        assert!(self.w.ledger(&m).liquidity.pot < pot, "closing: the liquidity add did not go through\n{}", logs(&r));
        self.check("closing: after the add");
    }

    fn check(&self, at: &str) {
        let w = &self.w;
        let (mut platform_paid, mut private_paid) = ([0u128; 2], 0u128);
        for (i, c) in self.coins.iter().enumerate() {
            let m = &c.mint;
            // A receiver is paid in the coin's own quote coin.
            let paid_to = |owner: &Pubkey| if c.burr { w.burr_of(owner) } else { w.zec_of(owner) } as u128;
            assert_eq!(w.b.account(&kit::coin(m)).unwrap().data, c.record, "{at}: coin {i}'s record changed");
            let l = w.ledger(m);
            // The vault holds exactly what the books account for: every fee here is split as it arrives.
            assert_eq!(w.vault_balance(m), l.accounted, "{at}: coin {i}: the vault and the books differ");
            // Each pot's credit is its share of everything split, worked out here from the recorded shares.
            let r = w.coin(m);
            let rest = (10_000 - r.platform_fraction_bps) as u128;
            let weights = [
                r.platform_fraction_bps as u128 * 10_000,
                rest * r.shares[0] as u128,
                rest * r.shares[1] as u128,
                rest * r.shares[2] as u128,
                rest * r.shares[3] as u128,
                rest * r.burr_share as u128,
                rest * r.liquidity_share as u128,
                rest * r.shadow_share as u128,
            ];
            assert_eq!(weights.iter().sum::<u128>(), 100_000_000, "{at}: coin {i}: the recorded shares do not add up");
            let credited = [
                l.credited[0],
                l.credited[1],
                l.credited[2],
                l.credited[3],
                l.credited[4],
                l.burr.credited,
                l.liquidity.credited,
                l.shadow.credited,
            ];
            for (k, (got, weight)) in credited.iter().zip(weights).enumerate() {
                assert_eq!(
                    *got,
                    l.total_split * weight / 100_000_000,
                    "{at}: coin {i}: pot {k}'s credit is not its share"
                );
            }
            let pots: u128 = l.pots.iter().map(|p| *p as u128).sum::<u128>()
                + l.burr.pot as u128
                + l.liquidity.pot as u128
                + l.shadow.pot as u128;
            assert_eq!(
                l.accounted as u128 - pots,
                l.total_split - credited.iter().sum::<u128>(),
                "{at}: coin {i}: the carry"
            );
            assert!(l.total_split - credited.iter().sum::<u128>() < 8, "{at}: coin {i}: the carry is too large");
            let shadow_spent = w.shadow_watch(m).map(|x| x.total_spent).unwrap_or(0) as u128;
            assert_eq!(l.shadow.credited, l.shadow.pot as u128 + shadow_spent, "{at}: coin {i}: shadow");
            // Every pot closes: credited = still there + what its one way out paid.
            let paid_creator = l.credited[1] - l.pots[1] as u128;
            if c.private {
                private_paid += paid_creator;
            } else if c.creator_receiver != Pubkey::default() {
                assert_eq!(paid_creator, paid_to(&c.creator_receiver), "{at}: coin {i}: the creator pot");
            }
            if c.give_receiver != Pubkey::default() {
                assert_eq!(l.credited[2] - l.pots[2] as u128, paid_to(&c.give_receiver), "{at}: coin {i}: give");
            }
            assert_eq!(l.credited[3], l.pots[3] as u128 + l.burn.total_spent as u128, "{at}: coin {i}: burn");
            assert_eq!(l.credited[4], l.pots[4] as u128 + l.holders.total_released as u128, "{at}: coin {i}: holders");
            assert_eq!(l.burr.credited, l.burr.pot as u128 + l.burr.total_spent as u128, "{at}: coin {i}: $BURR");
            assert_eq!(
                l.liquidity.credited,
                l.liquidity.pot as u128 + l.liquidity.total_spent as u128,
                "{at}: coin {i}: liquidity"
            );
            platform_paid[c.burr as usize] += l.credited[0] - l.pots[0] as u128;
        }
        assert_eq!(w.zec_of(&self.payout_wallet) as u128, private_paid, "{at}: the payout wallet's ZEC");
        assert_eq!(w.zec_of(&w.platform_receiver) as u128, platform_paid[0], "{at}: the platform receiver's ZEC");
        assert_eq!(w.burr_of(&w.platform_receiver) as u128, platform_paid[1], "{at}: the platform receiver's $BURR");
        assert_eq!(w.zec_of(&w.rewards_wallet.pubkey()) as u128, self.released[0], "{at}: the rewards wallet's ZEC");
        assert_eq!(w.burr_of(&w.rewards_wallet.pubkey()) as u128, self.released[1], "{at}: the rewards wallet's $BURR");
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
        life.close();
        if std::env::var("BURRPAD_DEEP").is_ok() {
            let d = life.did;
            println!("launches {} ($BURR-priced {}) payouts {} releases {} burns {} burr {} adds {} graduations {}", d[0], life.coins.iter().filter(|c| c.burr).count(), d[1], d[2], d[3], d[4], d[5], d[6]);
        }
    }
}
