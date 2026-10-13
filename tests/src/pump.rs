//! pump.fun's addresses, derived accounts, instruction builders and account
//! readers, as the tests need them. Layouts follow pump.fun's public
//! interface files as read on 13 Oct 2026.

use sha2::{Digest, Sha256};
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::{pubkey, Pubkey};

pub const PUMP: Pubkey = pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");
pub const PUMP_AMM: Pubkey = pubkey!("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA");
pub const PUMP_FEES: Pubkey = pubkey!("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ");
pub const MAYHEM: Pubkey = pubkey!("MAyhSmzXzV1pTf7LsNkrNwkWKTo4ougAJ1PPg47MD4e");
pub const ZEC: Pubkey = pubkey!("A7bdiYdS5GjqGFtxf17ppRHtDKPkkRqbKtR27dxvQXaS");
pub const TOKEN: Pubkey = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
pub const TOKEN_2022: Pubkey = pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
pub const ATA_PROGRAM: Pubkey = pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
pub const SYSTEM: Pubkey = pubkey!("11111111111111111111111111111111");
/// One of pump.fun's eight buy-back receivers.
pub const BUYBACK: Pubkey = pubkey!("9M4giFFMxmFGXtc3feFzRai56WbBqehoSeRE5GK7gf7");

pub fn pda(seeds: &[&[u8]], program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(seeds, program).0
}
pub fn ata(owner: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    pda(&[owner.as_ref(), token_program.as_ref(), mint.as_ref()], &ATA_PROGRAM)
}
pub fn disc(name: &str) -> [u8; 8] {
    let h = Sha256::digest(format!("global:{name}").as_bytes());
    h[..8].try_into().unwrap()
}

pub fn global() -> Pubkey {
    pda(&[b"global"], &PUMP)
}
pub fn event_authority() -> Pubkey {
    pda(&[b"__event_authority"], &PUMP)
}
pub fn curve(mint: &Pubkey) -> Pubkey {
    pda(&[b"bonding-curve", mint.as_ref()], &PUMP)
}
pub fn creator_vault(creator: &Pubkey) -> Pubkey {
    pda(&[b"creator-vault", creator.as_ref()], &PUMP)
}
pub fn fee_config() -> Pubkey {
    pda(&[b"fee_config", PUMP.as_ref()], &PUMP_FEES)
}
pub fn user_volume(user: &Pubkey) -> Pubkey {
    pda(&[b"user_volume_accumulator", user.as_ref()], &PUMP)
}
pub fn quote_control() -> Pubkey {
    pda(&[b"quote-control"], &PUMP)
}
pub fn pool_authority(mint: &Pubkey) -> Pubkey {
    pda(&[b"pool-authority", mint.as_ref()], &PUMP)
}
/// The one pool pump.fun makes for a coin when it graduates.
pub fn pool(mint: &Pubkey) -> Pubkey {
    pda(&[b"pool", &[0, 0], pool_authority(mint).as_ref(), mint.as_ref(), ZEC.as_ref()], &PUMP_AMM)
}
pub fn amm_global_config() -> Pubkey {
    pda(&[b"global_config"], &PUMP_AMM)
}
pub fn amm_event_authority() -> Pubkey {
    pda(&[b"__event_authority"], &PUMP_AMM)
}

fn ro(k: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(k, false)
}
fn rw(k: Pubkey) -> AccountMeta {
    AccountMeta::new(k, false)
}
fn rws(k: Pubkey) -> AccountMeta {
    AccountMeta::new(k, true)
}
fn string(s: &str) -> Vec<u8> {
    let mut v = (s.len() as u32).to_le_bytes().to_vec();
    v.extend_from_slice(s.as_bytes());
    v
}

/// The accounts `create_v2` takes for a ZEC-priced coin, in order.
pub fn create_v2_accounts(mint: &Pubkey, user: &Pubkey) -> Vec<AccountMeta> {
    let c = curve(mint);
    let sol_vault = pda(&[b"sol-vault"], &MAYHEM);
    vec![
        rws(*mint),
        ro(pda(&[b"mint-authority"], &PUMP)),
        rw(c),
        rw(ata(&c, mint, &TOKEN_2022)),
        ro(global()),
        rws(*user),
        ro(SYSTEM),
        ro(TOKEN_2022),
        ro(ATA_PROGRAM),
        rw(MAYHEM),
        ro(pda(&[b"global-params"], &MAYHEM)),
        rw(sol_vault),
        rw(pda(&[b"mayhem-state", mint.as_ref()], &MAYHEM)),
        rw(ata(&sol_vault, mint, &TOKEN_2022)),
        ro(event_authority()),
        ro(PUMP),
        ro(ZEC),
        rw(ata(&c, &ZEC, &TOKEN)),
        ro(TOKEN),
        ro(quote_control()),
    ]
}

/// pump.fun's `create_v2`, called directly (not through Burr).
pub fn create_v2(mint: &Pubkey, user: &Pubkey, creator: &Pubkey, fee_bps: u64) -> Instruction {
    let mut data = disc("create_v2").to_vec();
    data.extend(string("Test"));
    data.extend(string("TST"));
    data.extend(string("https://example.com/t.json"));
    data.extend_from_slice(creator.as_ref());
    data.push(0); // mayhem off
    data.push(0); // cashback off
    data.extend_from_slice(&fee_bps.to_le_bytes());
    data.push(0); // holder rewards off
    Instruction { program_id: PUMP, accounts: create_v2_accounts(mint, user), data }
}

/// Creates an associated token account, or does nothing if it exists.
pub fn create_ata(payer: &Pubkey, owner: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Instruction {
    Instruction {
        program_id: ATA_PROGRAM,
        accounts: vec![
            rws(*payer),
            rw(ata(owner, mint, token_program)),
            ro(*owner),
            ro(*mint),
            ro(SYSTEM),
            ro(*token_program),
        ],
        data: vec![1],
    }
}

/// The 17 accounts of pump.fun's v3 curve trades.
pub fn trade_v3_accounts(mint: &Pubkey, user: &Pubkey) -> Vec<AccountMeta> {
    let c = curve(mint);
    vec![
        ro(global()),
        ro(*mint),
        ro(ZEC),
        ro(TOKEN_2022),
        ro(TOKEN),
        rw(c),
        rw(ata(&c, mint, &TOKEN_2022)),
        rw(ata(&c, &ZEC, &TOKEN)),
        rws(*user),
        rw(ata(user, mint, &TOKEN_2022)),
        rw(ata(user, &ZEC, &TOKEN)),
        rw(user_volume(user)),
        ro(fee_config()),
        rw(ata(&BUYBACK, &ZEC, &TOKEN)),
        ro(SYSTEM),
        ro(event_authority()),
        ro(PUMP),
    ]
}

pub fn buy_exact_quote_in_v3(mint: &Pubkey, user: &Pubkey, quote_in: u64, min_out: u64) -> Instruction {
    let mut data = disc("buy_exact_quote_in_v3").to_vec();
    data.extend_from_slice(&quote_in.to_le_bytes());
    data.extend_from_slice(&min_out.to_le_bytes());
    data.push(0);
    Instruction { program_id: PUMP, accounts: trade_v3_accounts(mint, user), data }
}

pub fn sell_v3(mint: &Pubkey, user: &Pubkey, amount: u64, min_out: u64) -> Instruction {
    let mut data = disc("sell_v3").to_vec();
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&min_out.to_le_bytes());
    Instruction { program_id: PUMP, accounts: trade_v3_accounts(mint, user), data }
}

pub fn sweep_creator_fee(mint: &Pubkey, payer: &Pubkey, creator: &Pubkey) -> Instruction {
    let c = curve(mint);
    let v = creator_vault(creator);
    Instruction {
        program_id: PUMP,
        accounts: vec![
            rws(*payer),
            ro(global()),
            ro(*mint),
            ro(ZEC),
            ro(TOKEN),
            ro(ATA_PROGRAM),
            ro(SYSTEM),
            rw(c),
            rw(ata(&c, &ZEC, &TOKEN)),
            rw(v),
            rw(ata(&v, &ZEC, &TOKEN)),
            ro(event_authority()),
            ro(PUMP),
        ],
        data: disc("sweep_creator_fee").to_vec(),
    }
}

pub fn collect_creator_fee_v2(creator: &Pubkey) -> Instruction {
    let v = creator_vault(creator);
    Instruction {
        program_id: PUMP,
        accounts: vec![
            rw(*creator),
            rw(ata(creator, &ZEC, &TOKEN)),
            rw(v),
            rw(ata(&v, &ZEC, &TOKEN)),
            ro(ZEC),
            ro(TOKEN),
            ro(ATA_PROGRAM),
            ro(SYSTEM),
            ro(event_authority()),
            ro(PUMP),
        ],
        data: disc("collect_creator_fee_v2").to_vec(),
    }
}

/// The fields of pump.fun's curve account that the tests read.
#[derive(Debug, Clone)]
pub struct Curve {
    pub virtual_token_reserves: u64,
    pub virtual_quote_reserves: u64,
    pub real_token_reserves: u64,
    pub real_quote_reserves: u64,
    pub complete: bool,
    pub creator: Pubkey,
    pub quote_mint: Pubkey,
    pub creator_fee_bps: u64,
    pub is_holder_reward: bool,
    pub creator_fee: u64,
    pub protocol_fees: u64,
}

impl Curve {
    pub fn read(d: &[u8]) -> Self {
        let u = |o: usize| u64::from_le_bytes(d[o..o + 8].try_into().unwrap());
        let k = |o: usize| Pubkey::new_from_array(d[o..o + 32].try_into().unwrap());
        Self {
            virtual_token_reserves: u(8),
            virtual_quote_reserves: u(16),
            real_token_reserves: u(24),
            real_quote_reserves: u(32),
            complete: d[48] != 0,
            creator: k(49),
            quote_mint: k(83),
            creator_fee_bps: u(115),
            is_holder_reward: d[124] != 0,
            creator_fee: u(125),
            protocol_fees: u(133),
        }
    }
}
