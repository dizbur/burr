//! Helpers to drive Burr on the bench: one function per instruction, each
//! building the instruction from the program's own account lists.

use crate::bench::{Bench, TxResult};
use crate::pump::{self, ata, pda, ATA_PROGRAM, BUYBACK, PUMP, PUMP_AMM, PUMP_FEES, SYSTEM, TOKEN, TOKEN_2022, ZEC};
use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use burr::state::{Coin, Config, Ledger, Payee, Stake};
use solana_account::Account;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_pubkey::{pubkey, Pubkey};
use solana_signer::Signer;

pub const BURR: Pubkey = burr::ID;
pub const LOADER: Pubkey = pubkey!("BPFLoaderUpgradeab1e11111111111111111111111");

pub fn config() -> Pubkey {
    pda(&[b"config"], &BURR)
}
pub fn coin(mint: &Pubkey) -> Pubkey {
    pda(&[b"coin", mint.as_ref()], &BURR)
}
pub fn ledger(mint: &Pubkey) -> Pubkey {
    pda(&[b"ledger", mint.as_ref()], &BURR)
}
pub fn authority(mint: &Pubkey) -> Pubkey {
    pda(&[b"authority", mint.as_ref()], &BURR)
}
pub fn burner(mint: &Pubkey) -> Pubkey {
    pda(&[b"burner", mint.as_ref()], &BURR)
}
pub fn stake(mint: &Pubkey, owner: &Pubkey) -> Pubkey {
    pda(&[b"stake", mint.as_ref(), owner.as_ref()], &BURR)
}
pub fn vault(mint: &Pubkey) -> Pubkey {
    ata(&authority(mint), &ZEC, &TOKEN)
}
pub fn stake_vault(mint: &Pubkey) -> Pubkey {
    ata(&authority(mint), mint, &TOKEN_2022)
}
pub fn program_data() -> Pubkey {
    pda(&[BURR.as_ref()], &LOADER)
}

fn ix(accounts: impl ToAccountMetas, data: impl InstructionData) -> Instruction {
    Instruction { program_id: BURR, accounts: accounts.to_account_metas(None), data: data.data() }
}

/// What a launch asks for. Shares in order: creator, give, burn, vault share, king.
#[derive(Clone)]
pub struct LaunchPlan {
    pub fee_bps: u16,
    pub shares: [u16; 5],
    pub creator_receiver: Pubkey,
    pub give_receiver: Pubkey,
    pub king_round_secs: u32,
    pub name: String,
}

impl Default for LaunchPlan {
    fn default() -> Self {
        Self {
            fee_bps: 200,
            shares: [2_000, 2_000, 2_000, 2_000, 2_000],
            creator_receiver: Pubkey::new_unique(),
            give_receiver: Pubkey::new_unique(),
            king_round_secs: 3_600,
            name: "Test".into(),
        }
    }
}

/// The bench with Burr loaded and initialised.
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
    /// Burr loaded but not initialised.
    pub fn bare() -> Self {
        let mut b = Bench::new();
        let upgrade_key = b.wallet();
        let admin = b.wallet();
        let so = std::fs::read(crate::bench::root().join("target/deploy/burr.so"))
            .expect("build the program first: cargo build-sbf --manifest-path programs/burr/Cargo.toml");
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
                BURR,
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
            burr::accounts::Initialize {
                upgrade_authority: *signer,
                config: config(),
                program: BURR,
                program_data: program_data(),
                system_program: SYSTEM,
            },
            burr::instruction::Initialize { admin, platform_receiver: receiver },
        )
    }

    pub fn initialize(&mut self) -> TxResult {
        let i = self.initialize_ix(&self.upgrade_key.pubkey(), self.admin.pubkey(), self.platform_receiver);
        let k = self.upgrade_key.insecure_clone();
        self.b.send_as(Some(&k), &[i], &[])
    }

    pub fn admin_ix(&self, admin: &Pubkey, data: impl InstructionData) -> Instruction {
        ix(burr::accounts::AdminOnly { admin: *admin, config: config() }, data)
    }

    pub fn as_admin(&mut self, data: impl InstructionData) -> TxResult {
        let i = self.admin_ix(&self.admin.pubkey(), data);
        let k = self.admin.insecure_clone();
        self.b.send_as(Some(&k), &[i], &[])
    }

    pub fn launch_accounts(&self, payer: &Pubkey, mint: &Pubkey) -> burr::accounts::Launch {
        let curve = pump::curve(mint);
        let sol_vault = pda(&[b"sol-vault"], &pump::MAYHEM);
        burr::accounts::Launch {
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
        ix(
            self.launch_accounts(payer, mint),
            burr::instruction::Launch {
                args: burr::LaunchArgs {
                    name: plan.name.clone(),
                    symbol: "TST".into(),
                    uri: "https://e.co/t".into(),
                    fee_bps: plan.fee_bps,
                    shares: plan.shares,
                    creator_receiver: plan.creator_receiver,
                    give_receiver: plan.give_receiver,
                    king_round_secs: plan.king_round_secs,
                },
            },
        )
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
        assert!(l.accounted as u128 - pots < 6, "carry too large");
        let credited: u128 = l.credited.iter().sum();
        assert!(credited <= l.total_split && l.total_split - credited < 6, "credited does not match total");
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

    // ---- collect, split, pay ----

    pub fn split_ix(&self, mint: &Pubkey) -> Instruction {
        ix(
            burr::accounts::SplitOnly {
                mint: *mint,
                coin: coin(mint),
                ledger: ledger(mint),
                authority: authority(mint),
                vault: vault(mint),
                zec_mint: ZEC,
                token_program: TOKEN,
            },
            burr::instruction::Split {},
        )
    }
    pub fn split(&mut self, mint: &Pubkey) -> TxResult {
        let i = self.split_ix(mint);
        self.b.send(&[i], &[])
    }

    pub fn collect_curve_accounts(&self, payer: &Pubkey, mint: &Pubkey) -> burr::accounts::CollectCurve {
        let curve = pump::curve(mint);
        let cv = pump::creator_vault(&authority(mint));
        burr::accounts::CollectCurve {
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
        let i = ix(self.collect_curve_accounts(&self.b.payer.pubkey(), mint), burr::instruction::CollectCurve {});
        self.b.send(&[i], &[])
    }

    pub fn collect_pool_accounts(&self, payer: &Pubkey, mint: &Pubkey) -> burr::accounts::CollectPool {
        let pool = pump::pool(mint);
        let va = pda(&[b"creator_vault", authority(mint).as_ref()], &PUMP_AMM);
        burr::accounts::CollectPool {
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
        let i = ix(self.collect_pool_accounts(&self.b.payer.pubkey(), mint), burr::instruction::CollectPool {});
        self.b.send(&[i], &[])
    }

    pub fn pay_out_accounts(&self, payer: &Pubkey, mint: &Pubkey, receiver: &Pubkey) -> burr::accounts::PayOut {
        burr::accounts::PayOut {
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
        let i = ix(self.pay_out_accounts(&self.b.payer.pubkey(), mint, &receiver), burr::instruction::PayOut { payee });
        self.b.send(&[i], &[])
    }

    // ---- buy back and burn ----

    pub fn burn_curve_accounts(&self, payer: &Pubkey, mint: &Pubkey) -> burr::accounts::BurnCurve {
        let curve = pump::curve(mint);
        let bn = burner(mint);
        burr::accounts::BurnCurve {
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
        let i = ix(self.burn_curve_accounts(&self.b.payer.pubkey(), mint), burr::instruction::BurnCurve {});
        self.b.send(&[i], &[])
    }

    pub fn burn_pool_accounts(&self, payer: &Pubkey, mint: &Pubkey) -> burr::accounts::BurnPool {
        let pool = pump::pool(mint);
        let bn = burner(mint);
        burr::accounts::BurnPool {
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
        let i = ix(self.burn_pool_accounts(&self.b.payer.pubkey(), mint), burr::instruction::BurnPool {});
        self.b.send(&[i], &[])
    }

    // ---- vault share ----

    pub fn stake_accounts(&self, owner: &Pubkey, mint: &Pubkey) -> burr::accounts::StakeCoins {
        burr::accounts::StakeCoins {
            owner: *owner,
            mint: *mint,
            coin: coin(mint),
            ledger: ledger(mint),
            authority: authority(mint),
            stake: stake(mint, owner),
            stake_vault: stake_vault(mint),
            owner_coin: ata(owner, mint, &TOKEN_2022),
            token_2022_program: TOKEN_2022,
            associated_token_program: ATA_PROGRAM,
            system_program: SYSTEM,
        }
    }
    pub fn stake_ix(&self, owner: &Pubkey, mint: &Pubkey, amount: u64) -> Instruction {
        ix(self.stake_accounts(owner, mint), burr::instruction::Stake { amount })
    }
    pub fn stake(&mut self, owner: &Keypair, mint: &Pubkey, amount: u64) -> TxResult {
        let i = self.stake_ix(&owner.pubkey(), mint, amount);
        self.b.send_as(Some(owner), &[i], &[])
    }

    pub fn unstake_accounts(&self, owner: &Pubkey, mint: &Pubkey) -> burr::accounts::UnstakeCoins {
        burr::accounts::UnstakeCoins {
            owner: *owner,
            mint: *mint,
            coin: coin(mint),
            ledger: ledger(mint),
            authority: authority(mint),
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
        ix(self.unstake_accounts(owner, mint), burr::instruction::Unstake { amount })
    }
    pub fn unstake(&mut self, owner: &Keypair, mint: &Pubkey, amount: u64) -> TxResult {
        let i = self.unstake_ix(&owner.pubkey(), mint, amount);
        self.b.send_as(Some(owner), &[i], &[])
    }

    // ---- king of the hill ----

    pub fn king_buy_curve_accounts(&self, buyer: &Pubkey, mint: &Pubkey) -> burr::accounts::KingBuyCurve {
        let curve = pump::curve(mint);
        burr::accounts::KingBuyCurve {
            buyer: *buyer,
            mint: *mint,
            coin: coin(mint),
            ledger: ledger(mint),
            zec_mint: ZEC,
            buyer_coin: ata(buyer, mint, &TOKEN_2022),
            buyer_zec: ata(buyer, &ZEC, &TOKEN),
            pump_global: pump::global(),
            bonding_curve: curve,
            associated_base_bonding_curve: ata(&curve, mint, &TOKEN_2022),
            associated_quote_bonding_curve: ata(&curve, &ZEC, &TOKEN),
            user_volume: pump::user_volume(buyer),
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
    pub fn king_buy_curve_ix(&self, buyer: &Pubkey, mint: &Pubkey, zec: u64) -> Instruction {
        ix(self.king_buy_curve_accounts(buyer, mint), burr::instruction::KingBuyCurve { quote_in: zec, min_out: 1 })
    }
    pub fn king_buy_curve(&mut self, buyer: &Keypair, mint: &Pubkey, zec: u64) -> TxResult {
        let i = self.king_buy_curve_ix(&buyer.pubkey(), mint, zec);
        self.b.send_as(Some(buyer), &[i], &[])
    }

    pub fn king_buy_pool_accounts(&self, buyer: &Pubkey, mint: &Pubkey) -> burr::accounts::KingBuyPool {
        let pool = pump::pool(mint);
        burr::accounts::KingBuyPool {
            buyer: *buyer,
            mint: *mint,
            coin: coin(mint),
            ledger: ledger(mint),
            zec_mint: ZEC,
            buyer_coin: ata(buyer, mint, &TOKEN_2022),
            buyer_zec: ata(buyer, &ZEC, &TOKEN),
            amm_global_config: pump::amm_global_config(),
            pool_authority: pump::pool_authority(mint),
            pool,
            pool_base_token_account: ata(&pool, mint, &TOKEN_2022),
            pool_quote_token_account: ata(&pool, &ZEC, &TOKEN),
            user_volume: pda(&[b"user_volume_accumulator", buyer.as_ref()], &PUMP_AMM),
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
    pub fn king_buy_pool(&mut self, buyer: &Keypair, mint: &Pubkey, zec: u64) -> TxResult {
        let i = ix(
            self.king_buy_pool_accounts(&buyer.pubkey(), mint),
            burr::instruction::KingBuyPool { quote_in: zec, min_out: 1 },
        );
        self.b.send_as(Some(buyer), &[i], &[])
    }

    pub fn king_settle_accounts(&self, payer: &Pubkey, mint: &Pubkey, winner: &Pubkey) -> burr::accounts::KingSettle {
        burr::accounts::KingSettle {
            payer: *payer,
            mint: *mint,
            coin: coin(mint),
            ledger: ledger(mint),
            authority: authority(mint),
            vault: vault(mint),
            zec_mint: ZEC,
            winner: *winner,
            winner_zec: ata(winner, &ZEC, &TOKEN),
            token_program: TOKEN,
            associated_token_program: ATA_PROGRAM,
            system_program: SYSTEM,
        }
    }
    pub fn king_settle(&mut self, mint: &Pubkey) -> TxResult {
        let winner = self.ledger(mint).king.leader;
        let i = ix(self.king_settle_accounts(&self.b.payer.pubkey(), mint, &winner), burr::instruction::KingSettle {});
        self.b.send(&[i], &[])
    }
}

/// Builds a Burr instruction from an account list and its data. For tests
/// that need to pass a wrong account on purpose.
pub fn raw(accounts: impl ToAccountMetas, data: impl InstructionData) -> Instruction {
    ix(accounts, data)
}

/// Burr's own error number for a named error.
pub fn err(e: burr::error::BurrError) -> u32 {
    e as u32 + 6000
}
