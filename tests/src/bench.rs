//! The local network: an in-process Solana runtime loaded with pump.fun's
//! real programs and settings accounts.

use base64::Engine;
use litesvm::types::{FailedTransactionMetadata, TransactionMetadata};
use litesvm::LiteSVM;
use solana_account::Account;
use solana_clock::Clock;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;
use std::path::PathBuf;

use crate::pump::{self, ata, TOKEN, ZEC};

#[derive(serde::Deserialize)]
struct Fixture {
    pubkey: String,
    owner: String,
    lamports: u64,
    executable: bool,
    data: String,
}

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

pub struct Bench {
    pub svm: LiteSVM,
    pub payer: Keypair,
}

pub type TxResult = Result<TransactionMetadata, Box<FailedTransactionMetadata>>;

impl Default for Bench {
    fn default() -> Self {
        Self::new()
    }
}

impl Bench {
    /// A fresh network with pump.fun's programs and accounts, and a funded payer.
    pub fn new() -> Self {
        let mut svm = LiteSVM::new().with_sigverify(true).with_blockhash_check(false);
        let dir = root().join("fixtures");
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap_or_else(|_| panic!("fixtures/ missing: run scripts/ground/07-fetch-fixtures.mjs"))
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        // Program code first, so the program accounts that point at it load.
        entries.sort_by_key(|p| {
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            (!n.starts_with("programdata_"), n)
        });
        for p in entries {
            let f: Fixture = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
            let acct = Account {
                lamports: f.lamports,
                data: base64::engine::general_purpose::STANDARD.decode(&f.data).unwrap(),
                owner: f.owner.parse().unwrap(),
                executable: f.executable,
                rent_epoch: u64::MAX,
            };
            svm.set_account(f.pubkey.parse().unwrap(), acct)
                .unwrap_or_else(|e| panic!("loading {}: {e:?}", p.display()));
        }
        let payer = Keypair::new();
        svm.airdrop(&payer.pubkey(), 1_000_000_000_000).unwrap();
        let mut b = Self { svm, payer };
        b.set_time(1_760_000_000);
        b
    }

    /// Sends one transaction. The payer always signs; `signers` are the others.
    pub fn send(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> TxResult {
        self.send_as(None, ixs, signers)
    }

    pub fn send_as(&mut self, fee_payer: Option<&Keypair>, ixs: &[Instruction], signers: &[&Keypair]) -> TxResult {
        let payer = fee_payer.unwrap_or(&self.payer).insecure_clone();
        let mut all = vec![ComputeBudgetInstruction::set_compute_unit_limit(1_400_000)];
        all.extend_from_slice(ixs);
        let msg = Message::new(&all, Some(&payer.pubkey()));
        let mut ks: Vec<&Keypair> = vec![&payer];
        for s in signers {
            if s.pubkey() != payer.pubkey() {
                ks.push(s);
            }
        }
        let tx = Transaction::new(&ks, msg, self.svm.latest_blockhash());
        self.svm.expire_blockhash();
        self.svm.send_transaction(tx).map_err(Box::new)
    }

    /// A new wallet with SOL.
    pub fn wallet(&mut self) -> Keypair {
        let k = Keypair::new();
        self.svm.airdrop(&k.pubkey(), 100_000_000_000).unwrap();
        k
    }

    pub fn now(&self) -> i64 {
        self.svm.get_sysvar::<Clock>().unix_timestamp
    }

    pub fn set_time(&mut self, unix: i64) {
        let mut c = self.svm.get_sysvar::<Clock>();
        c.unix_timestamp = unix;
        self.svm.set_sysvar(&c);
    }

    /// Moves the clock forward.
    pub fn wait(&mut self, seconds: i64) {
        let mut c = self.svm.get_sysvar::<Clock>();
        c.unix_timestamp += seconds;
        c.slot += (seconds as u64 * 5 / 2).max(1);
        self.svm.set_sysvar(&c);
    }

    /// Gives `owner` a ZEC balance by writing the token account directly.
    /// Test money: it exists only inside this in-process network.
    pub fn give_zec(&mut self, owner: &Pubkey, amount: u64) -> Pubkey {
        let addr = ata(owner, &ZEC, &TOKEN);
        let prev = self.token_balance(&addr);
        self.write_token_account(addr, &ZEC, owner, prev + amount, &TOKEN);
        addr
    }

    pub fn write_token_account(&mut self, addr: Pubkey, mint: &Pubkey, owner: &Pubkey, amount: u64, program: &Pubkey) {
        let mut d = vec![0u8; 165];
        d[0..32].copy_from_slice(mint.as_ref());
        d[32..64].copy_from_slice(owner.as_ref());
        d[64..72].copy_from_slice(&amount.to_le_bytes());
        d[108] = 1; // initialised
        let lamports = self.svm.minimum_balance_for_rent_exemption(165);
        self.svm
            .set_account(addr, Account { lamports, data: d, owner: *program, executable: false, rent_epoch: u64::MAX })
            .unwrap();
    }

    /// Balance of a token account; zero if it does not exist.
    pub fn token_balance(&self, addr: &Pubkey) -> u64 {
        match self.svm.get_account(addr) {
            Some(a) if a.data.len() >= 72 => u64::from_le_bytes(a.data[64..72].try_into().unwrap()),
            _ => 0,
        }
    }

    pub fn mint_supply(&self, mint: &Pubkey) -> u64 {
        let a = self.svm.get_account(mint).expect("mint");
        u64::from_le_bytes(a.data[36..44].try_into().unwrap())
    }

    pub fn account(&self, addr: &Pubkey) -> Option<Account> {
        self.svm.get_account(addr).filter(|a| a.lamports > 0)
    }

    /// Replaces the program at `address` with the misbehaving stand-in, and
    /// tells it how to misbehave by one byte in that program's settings account.
    pub fn use_standin(&mut self, address: &Pubkey, settings: &Pubkey, mode: u8) {
        self.load_standin_at(address);
        self.set_standin_mode(settings, mode);
    }

    /// A program at a fresh address that accepts any call and does nothing.
    /// For passing in place of a real program.
    pub fn do_nothing_program(&mut self) -> Pubkey {
        let k = Pubkey::new_unique();
        self.load_standin_at(&k);
        k
    }

    fn load_standin_at(&mut self, address: &Pubkey) {
        let so = std::fs::read(root().join("target/deploy/pump_standin.so"))
            .expect("build the stand-in first: cargo build-sbf --manifest-path tests-standin/Cargo.toml");
        let loader: Pubkey = "BPFLoaderUpgradeab1e11111111111111111111111".parse().unwrap();
        let program_data = Pubkey::find_program_address(&[address.as_ref()], &loader).0;
        let mut data = vec![3, 0, 0, 0];
        data.extend_from_slice(&0u64.to_le_bytes());
        data.push(0);
        data.extend_from_slice(&[0u8; 32]);
        data.extend_from_slice(&so);
        let rent = self.svm.minimum_balance_for_rent_exemption(data.len());
        self.svm
            .set_account(
                program_data,
                Account { lamports: rent, data, owner: loader, executable: false, rent_epoch: u64::MAX },
            )
            .unwrap();
        let mut p = vec![2, 0, 0, 0];
        p.extend_from_slice(program_data.as_ref());
        self.svm
            .set_account(
                *address,
                Account { lamports: 1_000_000_000, data: p, owner: loader, executable: true, rent_epoch: u64::MAX },
            )
            .unwrap();
    }

    pub fn set_standin_mode(&mut self, settings: &Pubkey, mode: u8) {
        let mut s = self.svm.get_account(settings).expect("settings account");
        s.data[8] = mode;
        self.svm.set_account(*settings, s).unwrap();
    }

    pub fn curve(&self, mint: &Pubkey) -> pump::Curve {
        pump::Curve::read(&self.svm.get_account(&pump::curve(mint)).expect("curve").data)
    }
}

/// The custom error number of a failed transaction, if it has one.
pub fn custom_code(r: &TxResult) -> Option<u32> {
    use solana_instruction::error::InstructionError;
    use solana_transaction::TransactionError;
    match r {
        Err(f) => match &f.err {
            TransactionError::InstructionError(_, InstructionError::Custom(c)) => Some(*c),
            _ => None,
        },
        Ok(_) => None,
    }
}

pub fn logs(r: &TxResult) -> String {
    match r {
        Ok(m) => m.logs.join("\n"),
        Err(f) => format!("{:?}\n{}", f.err, f.meta.logs.join("\n")),
    }
}

/// True if a transaction failed without pump.fun's programs ever being
/// called: that is, Burr itself refused it.
pub fn refused_by_burr(r: &TxResult) -> bool {
    match r {
        Ok(_) => false,
        Err(f) => !f.meta.logs.iter().any(|l| {
            l.starts_with(&format!("Program {} invoke", pump::PUMP))
                || l.starts_with(&format!("Program {} invoke", pump::PUMP_AMM))
        }),
    }
}
