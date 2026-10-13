//! Helpers to drive Burrpad on the bench: one function per instruction, each
//! building the instruction from the program's own account lists.

use crate::bench::{Bench, TxResult};
use crate::pump::{self, ata, pda, ATA_PROGRAM, BUYBACK, PUMP, PUMP_AMM, PUMP_FEES, SYSTEM, TOKEN, TOKEN_2022, ZEC};
use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use burrpad::state::{Coin, Config, Ledger, Payee, Stake};
use solana_account::Account;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_pubkey::{pubkey, Pubkey};
use solana_signer::Signer;

pub const BURRPAD: Pubkey = burrpad::ID;
pub const LOADER: Pubkey = pubkey!("BPFLoaderUpgradeab1e11111111111111111111111");

pub fn config() -> Pubkey {
    pda(&[b"config"], &BURRPAD)
}
pub fn coin(mint: &Pubkey) -> Pubkey {
    pda(&[b"coin", mint.as_ref()], &BURRPAD)
}
pub fn ledger(mint: &Pubkey) -> Pubkey {
    pda(&[b"ledger", mint.as_ref()], &BURRPAD)
}
pub fn authority(mint: &Pubkey) -> Pubkey {
    pda(&[b"authority", mint.as_ref()], &BURRPAD)
}
pub fn burner(mint: &Pubkey) -> Pubkey {
    pda(&[b"burner", mint.as_ref()], &BURRPAD)
}
pub fn stake(mint: &Pubkey, owner: &Pubkey) -> Pubkey {
    pda(&[b"stake", mint.as_ref(), owner.as_ref()], &BURRPAD)
}
pub fn vault(mint: &Pubkey) -> Pubkey {
    ata(&authority(mint), &ZEC, &TOKEN)
}
pub fn stake_authority(mint: &Pubkey) -> Pubkey {
    pda(&[b"stake-authority", mint.as_ref()], &BURRPAD)
}
pub fn stake_vault(mint: &Pubkey) -> Pubkey {
    ata(&stake_authority(mint), mint, &TOKEN_2022)
}
pub fn program_data() -> Pubkey {
    pda(&[BURRPAD.as_ref()], &LOADER)
}

fn ix(accounts: impl ToAccountMetas, data: impl InstructionData) -> Instruction {
    Instruction { program_id: BURRPAD, accounts: accounts.to_account_metas(None), data: data.data() }
}

/// What a launch asks for. Shares in order: creator, give, burn, vault share.
#[derive(Clone)]
pub struct LaunchPlan {
    pub fee_bps: u16,
    pub shares: [u16; 4],
    pub creator_receiver: Pubkey,
    pub give_receiver: Pubkey,
    pub name: String,
    /// The picture link.
    pub uri: String,
    /// The platform terms the launcher agrees to; `None` means those in force.
    pub max_platform_fraction_bps: Option<u16>,
    pub platform_receiver: Option<Pubkey>,
}

impl Default for LaunchPlan {
    fn default() -> Self {
        Self {
            fee_bps: 200,
            shares: [2_500, 2_500, 2_500, 2_500],
            creator_receiver: Pubkey::new_unique(),
            give_receiver: Pubkey::new_unique(),
            name: "Test".into(),
            uri: "https://e.co/t".into(),
            max_platform_fraction_bps: None,
            platform_receiver: None,
        }
    }
}

/// The bench with Burrpad loaded and initialised.
pub struct World {
    pub b: Bench,
    pub upgrade_key: Keypair,
    pub admin: Keypair,
    pub platform_receiver: Pubkey,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    /// Burrpad loaded but not initialised.
    pub fn bare() -> Self {
        let mut b = Bench::new();
        let upgrade_key = b.wallet();
        let admin = b.wallet();
        let so = std::fs::read(crate::bench::root().join("target/deploy/burrpad.so"))
            .expect("build the program first: cargo build-sbf --manifest-path programs/burrpad/Cargo.toml");
        // Loaded the way a real deployment is: a program account pointing at
        // its code, with an upgrade key recorded.
        let mut data = vec![3, 0, 0, 0];
        data.extend_from_slice(&0u64.to_le_bytes());
        data.push(1);
        data.extend_from_slice(upgrade_key.pubkey().as_ref());
        data.extend_from_slice(&so);
        let rent = b.svm.minimum_balance_for_rent_exemption(data.len());
        b.svm
            .set_account(
                program_data(),
                Account { lamports: rent, data, owner: LOADER, executable: false, rent_epoch: u64::MAX },
            )
            .unwrap();
        let mut p = vec![2, 0, 0, 0];
        p.extend_from_slice(program_data().as_ref());
        b.svm
            .set_account(
                BURRPAD,
                Account { lamports: 1_000_000_000, data: p, owner: LOADER, executable: true, rent_epoch: u64::MAX },
            )
            .unwrap();
        Self { b, upgrade_key, admin, platform_receiver: Pubkey::new_unique() }
    }

    pub fn new() -> Self {
        let mut w = Self::bare();
        let r = w.initialize();
        assert!(r.is_ok(), "{}", crate::bench::logs(&r));
        w
    }

    pub fn initialize_ix(&self, signer: &Pubkey, admin: Pubkey, receiver: Pubkey) -> Instruction {
        ix(
            burrpad::accounts::Initialize {
                upgrade_authority: *signer,
                config: config(),
                program: BURRPAD,
                program_data: program_data(),
                system_program: SYSTEM,
            },
            burrpad::instruction::Initialize { admin, platform_receiver: receiver },
        )
    }

    pub fn initialize(&mut self) -> TxResult {
        let i = self.initialize_ix(&self.upgrade_key.pubkey(), self.admin.pubkey(), self.platform_receiver);
        let k = self.upgrade_key.insecure_clone();
        self.b.send_as(Some(&k), &[i], &[])
    }

    pub fn admin_ix(&self, admin: &Pubkey, data: impl InstructionData) -> Instruction {
        ix(burrpad::accounts::AdminOnly { admin: *admin, config: config() }, data)
    }

    pub fn as_admin(&mut self, data: impl InstructionData) -> TxResult {
        let i = self.admin_ix(&self.admin.pubkey(), data);
        let k = self.admin.insecure_clone();
        self.b.send_as(Some(&k), &[i], &[])
    }

    pub fn launch_accounts(&self, payer: &Pubkey, mint: &Pubkey) -> burrpad::accounts::Launch {
        let curve = pump::curve(mint);
        let sol_vault = pda(&[b"sol-vault"], &pump::MAYHEM);
        burrpad::accounts::Launch {
            payer: *payer,
            mint: *mint,
            config: config(),
            coin: coin(mint),
            ledger: ledger(mint),
            authority: authority(mint),
            vault: vault(mint),
            zec_mint: ZEC,
            pump_mint_authority: pda(&[b"mint-authority"], &PUMP),
            bonding_curve: curve,
            associated_bonding_curve: ata(&curve, mint, &TOKEN_2022),
            pump_global: pump::global(),
            mayhem_program: pump::MAYHEM,
            mayhem_global_params: pda(&[b"global-params"], &pump::MAYHEM),
            mayhem_sol_vault: sol_vault,
            mayhem_state: pda(&[b"mayhem-state", mint.as_ref()], &pump::MAYHEM),
            mayhem_token_vault: ata(&sol_vault, mint, &TOKEN_2022),
            pump_event_authority: pump::event_authority(),
            associated_quote_bonding_curve: ata(&curve, &ZEC, &TOKEN),
            quote_control: pump::quote_control(),
            pump_program: PUMP,
            token_program: TOKEN,
            token_2022_program: TOKEN_2022,
            associated_token_program: ATA_PROGRAM,
            system_program: SYSTEM,
        }
    }

    pub fn launch_ix(&self, payer: &Pubkey, mint: &Pubkey, plan: &LaunchPlan) -> Instruction {
        // Unless a test says otherwise, the launcher agrees to the terms in force.
        let cfg = self.config();
        ix(
            self.launch_accounts(payer, mint),
            burrpad::instruction::Launch {
                args: burrpad::LaunchArgs {
                    name: plan.name.clone(),
                    symbol: "TST".into(),
                    uri: plan.uri.clone(),
                    fee_bps: plan.fee_bps,
                    shares: plan.shares,
                    creator_receiver: plan.creator_receiver,
                    give_receiver: plan.give_receiver,
                    max_platform_fraction_bps: plan.max_platform_fraction_bps.unwrap_or(cfg.platform_fraction_bps),
                    platform_receiver: plan.platform_receiver.unwrap_or(cfg.platform_receiver),
                },
            },
        )
    }

    /// Launches with a mint key chosen beforehand.
    pub fn launch_with_mint(&mut self, mint: &Keypair, plan: &LaunchPlan) -> TxResult {
        let i = self.launch_ix(&self.b.payer.pubkey(), &mint.pubkey(), plan);
        self.b.send(&[i], &[mint])
    }

    pub fn try_launch(&mut self, plan: &LaunchPlan) -> (Pubkey, TxResult) {
        let mint = Keypair::new();
        let i = self.launch_ix(&self.b.payer.pubkey(), &mint.pubkey(), plan);
        let r = self.b.send(&[i], &[&mint]);
        (mint.pubkey(), r)
    }

    /// Launches a coin and returns its mint.
    pub fn launch(&mut self, plan: &LaunchPlan) -> Pubkey {
        let (mint, r) = self.try_launch(plan);
        assert!(r.is_ok(), "{}", crate::bench::logs(&r));
        mint
    }

    // ---- reading ----

    pub fn config(&self) -> Config {
        let a = self.b.account(&config()).expect("config");
        Config::try_deserialize(&mut a.data.as_slice()).unwrap()
    }
    pub fn coin(&self, mint: &Pubkey) -> Coin {
        let a = self.b.account(&coin(mint)).expect("coin record");
        Coin::try_deserialize(&mut a.data.as_slice()).unwrap()
    }
    pub fn ledger(&self, mint: &Pubkey) -> Ledger {
        let a = self.b.account(&ledger(mint)).expect("ledger");
        Ledger::try_deserialize(&mut a.data.as_slice()).unwrap()
    }
    pub fn stake_of(&self, mint: &Pubkey, owner: &Pubkey) -> Option<Stake> {
        let a = self.b.account(&stake(mint, owner))?;
        Stake::try_deserialize(&mut a.data.as_slice()).ok()
    }
    pub fn vault_balance(&self, mint: &Pubkey) -> u64 {
        self.b.token_balance(&vault(mint))
    }
    pub fn zec_of(&self, owner: &Pubkey) -> u64 {
        self.b.token_balance(&ata(owner, &ZEC, &TOKEN))
    }
    pub fn coins_of(&self, owner: &Pubkey, mint: &Pubkey) -> u64 {
        self.b.token_balance(&ata(owner, mint, &TOKEN_2022))
    }

    /// The rule everything rests on: the vault holds at least what the ledger
    /// says, and the pots never add up to more than the ledger's total.
    pub fn assert_sound(&self, mint: &Pubkey) {
        let l = self.ledger(mint);
        let pots: u128 = l.pots.iter().map(|p| *p as u128).sum();
        assert!(self.vault_balance(mint) >= l.accounted, "vault below accounted");
        assert!(pots <= l.accounted as u128, "pots above accounted");
        assert!(l.accounted as u128 - pots < 5, "carry too large");
        let credited: u128 = l.credited.iter().sum();
        assert!(credited <= l.total_split && l.total_split - credited < 5, "credited does not match total");
    }

    // ---- trades made straight on pump.fun, as an ordinary trader would ----

    /// A wallet with SOL and `zec` units of test ZEC.
    pub fn trader(&mut self, zec: u64) -> Keypair {
        let k = self.b.wallet();
        self.b.give_zec(&k.pubkey(), zec);
        k
    }

    pub fn buy(&mut self, trader: &Keypair, mint: &Pubkey, zec: u64) -> TxResult {
        let ixs = [
            pump::create_ata(&trader.pubkey(), &trader.pubkey(), mint, &TOKEN_2022),
            pump::buy_exact_quote_in_v3(mint, &trader.pubkey(), zec, 1),
        ];
        self.b.send_as(Some(trader), &ixs, &[])
    }

    pub fn sell(&mut self, trader: &Keypair, mint: &Pubkey, coins: u64) -> TxResult {
        let i = pump::sell_v3(mint, &trader.pubkey(), coins, 0);
        self.b.send_as(Some(trader), &[i], &[])
    }

    // ---- graduation, and trades on the pool ----

    /// Buys out the rest of the curve and moves the coin to its PumpSwap pool,
    /// with pump.fun's own instructions, as anyone can.
    pub fn graduate(&mut self, mint: &Pubkey) {
        let whale = self.trader(10_000 * 100_000_000);
        let r = self.buy(&whale, mint, 20 * 100_000_000);
        assert!(r.is_ok(), "{}", crate::bench::logs(&r));
        assert!(self.b.curve(mint).complete, "the curve should be complete");
        self.b.svm.airdrop(&pump::WITHDRAW_AUTHORITY, 1_000_000_000).unwrap();
        let i = pump::migrate_v2(mint, &whale.pubkey());
        let r = self.b.send_as(Some(&whale), &[i], &[]);
        assert!(r.is_ok(), "{}", crate::bench::logs(&r));
        assert!(self.b.account(&pump::pool(mint)).is_some(), "the pool should exist");
    }

    pub fn pool(&self, mint: &Pubkey) -> pump::Pool {
        pump::Pool::read(&self.b.account(&pump::pool(mint)).expect("pool").data)
    }

    /// The pool's price figures: (ZEC reserve prices are worked from, coins).
    pub fn pool_reserves(&self, mint: &Pubkey) -> (u64, u64) {
        let p = pump::pool(mint);
        let q = self.b.token_balance(&ata(&p, &ZEC, &TOKEN)) as i128 + self.pool(mint).virtual_quote_reserves;
        (q as u64, self.b.token_balance(&ata(&p, mint, &TOKEN_2022)))
    }

    pub fn pool_buy(&mut self, trader: &Keypair, mint: &Pubkey, zec: u64) -> TxResult {
        let ixs = [
            pump::create_ata(&trader.pubkey(), &trader.pubkey(), mint, &TOKEN_2022),
            pump::pool_buy_exact_quote_in(mint, &trader.pubkey(), zec, 1),
        ];
        self.b.send_as(Some(trader), &ixs, &[])
    }

    pub fn pool_sell(&mut self, trader: &Keypair, mint: &Pubkey, coins: u64) -> TxResult {
        let i = pump::pool_sell(mint, &trader.pubkey(), coins, 0);
        self.b.send_as(Some(trader), &[i], &[])
    }

    // ---- collect, split, pay ----

    pub fn split_ix(&self, mint: &Pubkey) -> Instruction {
        ix(
            burrpad::accounts::SplitOnly {
                mint: *mint,
                coin: coin(mint),
                ledger: ledger(mint),
                authority: authority(mint),
                vault: vault(mint),
                zec_mint: ZEC,
                token_program: TOKEN,
            },
            burrpad::instruction::Split {},
        )
    }
    pub fn split(&mut self, mint: &Pubkey) -> TxResult {
        let i = self.split_ix(mint);
        self.b.send(&[i], &[])
    }

    pub fn collect_curve_accounts(&self, payer: &Pubkey, mint: &Pubkey) -> burrpad::accounts::CollectCurve {
        let curve = pump::curve(mint);
        let cv = pump::creator_vault(&authority(mint));
        burrpad::accounts::CollectCurve {
            payer: *payer,
            mint: *mint,
            coin: coin(mint),
            ledger: ledger(mint),
            authority: authority(mint),
            vault: vault(mint),
            zec_mint: ZEC,
            pump_global: pump::global(),
            bonding_curve: curve,
            associated_quote_bonding_curve: ata(&curve, &ZEC, &TOKEN),
            creator_vault: cv,
            creator_vault_ata: ata(&cv, &ZEC, &TOKEN),
            pump_event_authority: pump::event_authority(),
            pump_program: PUMP,
            token_program: TOKEN,
            associated_token_program: ATA_PROGRAM,
            system_program: SYSTEM,
        }
    }
    pub fn collect_curve(&mut self, mint: &Pubkey) -> TxResult {
        let i = ix(self.collect_curve_accounts(&self.b.payer.pubkey(), mint), burrpad::instruction::CollectCurve {});
        self.b.send(&[i], &[])
    }

    pub fn collect_pool_accounts(&self, payer: &Pubkey, mint: &Pubkey) -> burrpad::accounts::CollectPool {
        let pool = pump::pool(mint);
        let va = pda(&[b"creator_vault", authority(mint).as_ref()], &PUMP_AMM);
        burrpad::accounts::CollectPool {
            payer: *payer,
            mint: *mint,
            coin: coin(mint),
            ledger: ledger(mint),
            authority: authority(mint),
            vault: vault(mint),
            zec_mint: ZEC,
            amm_global_config: pump::amm_global_config(),
            pool_authority: pump::pool_authority(mint),
            pool,
            pool_quote_token_account: ata(&pool, &ZEC, &TOKEN),
            amm_creator_vault_authority: va,
            amm_creator_vault_ata: ata(&va, &ZEC, &TOKEN),
            amm_event_authority: pump::amm_event_authority(),
            amm_program: PUMP_AMM,
            token_program: TOKEN,
            associated_token_program: ATA_PROGRAM,
            system_program: SYSTEM,
        }
    }
    pub fn collect_pool(&mut self, mint: &Pubkey) -> TxResult {
        let i = ix(self.collect_pool_accounts(&self.b.payer.pubkey(), mint), burrpad::instruction::CollectPool {});
        self.b.send(&[i], &[])
    }

    pub fn pay_out_accounts(&self, payer: &Pubkey, mint: &Pubkey, receiver: &Pubkey) -> burrpad::accounts::PayOut {
        burrpad::accounts::PayOut {
            payer: *payer,
            mint: *mint,
            coin: coin(mint),
            ledger: ledger(mint),
            authority: authority(mint),
            vault: vault(mint),
            zec_mint: ZEC,
            receiver: *receiver,
            receiver_zec: ata(receiver, &ZEC, &TOKEN),
            token_program: TOKEN,
            associated_token_program: ATA_PROGRAM,
            system_program: SYSTEM,
        }
    }
    pub fn pay_out(&mut self, mint: &Pubkey, payee: Payee) -> TxResult {
        let receiver = payee.receiver(&self.coin(mint));
        let i =
            ix(self.pay_out_accounts(&self.b.payer.pubkey(), mint, &receiver), burrpad::instruction::PayOut { payee });
        self.b.send(&[i], &[])
    }

    // ---- buy back and burn ----

    pub fn burn_curve_accounts(&self, payer: &Pubkey, mint: &Pubkey) -> burrpad::accounts::BurnCurve {
        let curve = pump::curve(mint);
        let bn = burner(mint);
        burrpad::accounts::BurnCurve {
            payer: *payer,
            mint: *mint,
            coin: coin(mint),
            ledger: ledger(mint),
            authority: authority(mint),
            vault: vault(mint),
            zec_mint: ZEC,
            burner: bn,
            burner_zec: ata(&bn, &ZEC, &TOKEN),
            burner_coin: ata(&bn, mint, &TOKEN_2022),
            pump_global: pump::global(),
            bonding_curve: curve,
            associated_base_bonding_curve: ata(&curve, mint, &TOKEN_2022),
            associated_quote_bonding_curve: ata(&curve, &ZEC, &TOKEN),
            user_volume: pump::user_volume(&bn),
            fee_config: pump::fee_config(),
            buyback_fee_recipient: ata(&BUYBACK, &ZEC, &TOKEN),
            pump_event_authority: pump::event_authority(),
            pump_program: PUMP,
            token_program: TOKEN,
            token_2022_program: TOKEN_2022,
            associated_token_program: ATA_PROGRAM,
            system_program: SYSTEM,
        }
    }
    pub fn burn_curve(&mut self, mint: &Pubkey) -> TxResult {
        let i = ix(self.burn_curve_accounts(&self.b.payer.pubkey(), mint), burrpad::instruction::BurnCurve {});
        self.b.send(&[i], &[])
    }

    pub fn burn_pool_accounts(&self, payer: &Pubkey, mint: &Pubkey) -> burrpad::accounts::BurnPool {
        let pool = pump::pool(mint);
        let bn = burner(mint);
        burrpad::accounts::BurnPool {
            payer: *payer,
            mint: *mint,
            coin: coin(mint),
            ledger: ledger(mint),
            authority: authority(mint),
            vault: vault(mint),
            zec_mint: ZEC,
            burner: bn,
            burner_zec: ata(&bn, &ZEC, &TOKEN),
            burner_coin: ata(&bn, mint, &TOKEN_2022),
            amm_global_config: pump::amm_global_config(),
            pool_authority: pump::pool_authority(mint),
            pool,
            pool_base_token_account: ata(&pool, mint, &TOKEN_2022),
            pool_quote_token_account: ata(&pool, &ZEC, &TOKEN),
            user_volume: pda(&[b"user_volume_accumulator", bn.as_ref()], &PUMP_AMM),
            fee_config: pda(&[b"fee_config", PUMP_AMM.as_ref()], &PUMP_FEES),
            buyback_fee_recipient: ata(&BUYBACK, &ZEC, &TOKEN),
            amm_event_authority: pump::amm_event_authority(),
            amm_program: PUMP_AMM,
            token_program: TOKEN,
            token_2022_program: TOKEN_2022,
            associated_token_program: ATA_PROGRAM,
            system_program: SYSTEM,
        }
    }
    pub fn burn_pool(&mut self, mint: &Pubkey) -> TxResult {
        let i = ix(self.burn_pool_accounts(&self.b.payer.pubkey(), mint), burrpad::instruction::BurnPool {});
        self.b.send(&[i], &[])
    }

    /// Calls the burn once a minute until it really buys. After the price
    /// has risen, the burn follows it up one 5% step per call. Returns how
    /// many calls it took.
    pub fn burn_until_it_buys(&mut self, mint: &Pubkey) -> u32 {
        for n in 1..=300 {
            let before = self.ledger(mint).pots[3];
            let r = if self.b.curve(mint).complete { self.burn_pool(mint) } else { self.burn_curve(mint) };
            assert!(r.is_ok(), "{}", crate::bench::logs(&r));
            if self.ledger(mint).pots[3] < before {
                return n;
            }
            self.b.wait(60);
        }
        panic!("the burn never bought");
    }

    pub fn close_stake(&mut self, owner: &Keypair, mint: &Pubkey) -> TxResult {
        let i = ix(
            burrpad::accounts::CloseStake { owner: owner.pubkey(), mint: *mint, stake: stake(mint, &owner.pubkey()) },
            burrpad::instruction::CloseStake {},
        );
        self.b.send_as(Some(owner), &[i], &[])
    }

    // ---- vault share ----

    pub fn stake_accounts(&self, owner: &Pubkey, mint: &Pubkey) -> burrpad::accounts::StakeCoins {
        burrpad::accounts::StakeCoins {
            owner: *owner,
            mint: *mint,
            coin: coin(mint),
            ledger: ledger(mint),
            stake_authority: stake_authority(mint),
            stake: stake(mint, owner),
            stake_vault: stake_vault(mint),
            owner_coin: ata(owner, mint, &TOKEN_2022),
            token_2022_program: TOKEN_2022,
            associated_token_program: ATA_PROGRAM,
            system_program: SYSTEM,
        }
    }
    pub fn stake_ix(&self, owner: &Pubkey, mint: &Pubkey, amount: u64) -> Instruction {
        ix(self.stake_accounts(owner, mint), burrpad::instruction::Stake { amount })
    }
    pub fn stake(&mut self, owner: &Keypair, mint: &Pubkey, amount: u64) -> TxResult {
        let i = self.stake_ix(&owner.pubkey(), mint, amount);
        self.b.send_as(Some(owner), &[i], &[])
    }

    pub fn unstake_accounts(&self, owner: &Pubkey, mint: &Pubkey) -> burrpad::accounts::UnstakeCoins {
        burrpad::accounts::UnstakeCoins {
            owner: *owner,
            mint: *mint,
            coin: coin(mint),
            ledger: ledger(mint),
            authority: authority(mint),
            stake_authority: stake_authority(mint),
            stake: stake(mint, owner),
            stake_vault: stake_vault(mint),
            owner_coin: ata(owner, mint, &TOKEN_2022),
            vault: vault(mint),
            zec_mint: ZEC,
            owner_zec: ata(owner, &ZEC, &TOKEN),
            token_program: TOKEN,
            token_2022_program: TOKEN_2022,
            associated_token_program: ATA_PROGRAM,
            system_program: SYSTEM,
        }
    }
    pub fn unstake_ix(&self, owner: &Pubkey, mint: &Pubkey, amount: u64) -> Instruction {
        ix(self.unstake_accounts(owner, mint), burrpad::instruction::Unstake { amount })
    }
    pub fn unstake(&mut self, owner: &Keypair, mint: &Pubkey, amount: u64) -> TxResult {
        let i = self.unstake_ix(&owner.pubkey(), mint, amount);
        self.b.send_as(Some(owner), &[i], &[])
    }
}

/// Builds a Burrpad instruction from an account list and its data. For tests
/// that need to pass a wrong account on purpose.
pub fn raw(accounts: impl ToAccountMetas, data: impl InstructionData) -> Instruction {
    ix(accounts, data)
}

/// Burrpad's own error number for a named error.
pub fn err(e: burrpad::error::BurrpadError) -> u32 {
    e as u32 + 6000
}

/// The classic token program's "hand this account to another owner".
pub fn give_token_account_away(account: &Pubkey, owner: &Pubkey, new_owner: &Pubkey) -> Instruction {
    let mut data = vec![6u8, 2, 1];
    data.extend_from_slice(new_owner.as_ref());
    Instruction {
        program_id: TOKEN,
        accounts: vec![
            solana_instruction::AccountMeta::new(*account, false),
            solana_instruction::AccountMeta::new_readonly(*owner, true),
        ],
        data,
    }
}

/// A new mint key whose coin's five derived addresses all have the same
/// (highest) bump. Coins made from such keys differ only in their addresses,
/// so a test that swaps one coin's account for another's is stopped by the
/// address check alone and by nothing incidental.
pub fn mint_with_plain_bumps() -> Keypair {
    loop {
        let k = Keypair::new();
        let m = k.pubkey();
        let all_top = [&b"coin"[..], b"ledger", b"authority", b"burner", b"stake-authority"]
            .iter()
            .all(|seed| Pubkey::find_program_address(&[seed, m.as_ref()], &BURRPAD).1 == 255);
        if all_top {
            return k;
        }
    }
}

/// The accounts that are the same for every coin: programs, pump.fun's fixed
/// accounts, the ZEC mint, Burrpad's settings. A site puts these in an
/// address table so that larger transactions fit.
pub fn fixed_accounts() -> Vec<Pubkey> {
    vec![
        BURRPAD,
        config(),
        ZEC,
        PUMP,
        PUMP_AMM,
        PUMP_FEES,
        pump::MAYHEM,
        pump::global(),
        pump::event_authority(),
        pump::quote_control(),
        pump::fee_config(),
        pump::amm_global_config(),
        pump::amm_event_authority(),
        pda(&[b"fee_config", PUMP_AMM.as_ref()], &PUMP_FEES),
        pda(&[b"mint-authority"], &PUMP),
        pda(&[b"global-params"], &pump::MAYHEM),
        pda(&[b"sol-vault"], &pump::MAYHEM),
        ata(&BUYBACK, &ZEC, &TOKEN),
        TOKEN,
        TOKEN_2022,
        ATA_PROGRAM,
        SYSTEM,
        "ComputeBudget111111111111111111111111111111".parse().unwrap(),
    ]
}
