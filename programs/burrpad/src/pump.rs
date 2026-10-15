//! Everything that touches pump.fun's programs: their addresses, the derived
//! accounts we expect, readers for the accounts we rely on, and the calls.
//!
//! pump.fun's programs are closed source. Layouts and instruction codes are
//! taken from its public interface files as read on 13 Oct 2026 and are held
//! by tests that run against real copies of the programs.
//!
//! Rules kept here:
//! - calls go only to the exact program addresses below;
//! - nothing is concluded from a call having succeeded: callers read balances
//!   and accounts again afterwards.

use crate::error::BurrpadError;
use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::solana_program::program::invoke_signed;

pub const PUMP_ID: Pubkey = pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");
pub const PUMP_AMM_ID: Pubkey = pubkey!("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA");
pub const PUMP_FEES_ID: Pubkey = pubkey!("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ");
pub const MAYHEM_ID: Pubkey = pubkey!("MAyhSmzXzV1pTf7LsNkrNwkWKTo4ougAJ1PPg47MD4e");

// Seeds of pump.fun's derived accounts.
pub const SEED_GLOBAL: &[u8] = b"global";
pub const SEED_MINT_AUTHORITY: &[u8] = b"mint-authority";
pub const SEED_BONDING_CURVE: &[u8] = b"bonding-curve";
pub const SEED_EVENT_AUTHORITY: &[u8] = b"__event_authority";
pub const SEED_QUOTE_CONTROL: &[u8] = b"quote-control";
pub const SEED_CREATOR_VAULT: &[u8] = b"creator-vault";
pub const SEED_USER_VOLUME: &[u8] = b"user_volume_accumulator";
pub const SEED_FEE_CONFIG: &[u8] = b"fee_config";
pub const SEED_POOL_AUTHORITY: &[u8] = b"pool-authority";
pub const SEED_MAYHEM_GLOBAL_PARAMS: &[u8] = b"global-params";
pub const SEED_MAYHEM_SOL_VAULT: &[u8] = b"sol-vault";
pub const SEED_MAYHEM_STATE: &[u8] = b"mayhem-state";
pub const SEED_AMM_GLOBAL_CONFIG: &[u8] = b"global_config";
pub const SEED_AMM_POOL: &[u8] = b"pool";
pub const SEED_POOL_LP_MINT: &[u8] = b"pool_lp_mint";
pub const SEED_AMM_CREATOR_VAULT: &[u8] = b"creator_vault";
/// Pools made by graduation always have index 0.
pub const CANONICAL_POOL_INDEX: [u8; 2] = [0, 0];

// Instruction codes.
pub const IX_CREATE_V2: [u8; 8] = [214, 144, 76, 236, 95, 139, 49, 180];
pub const IX_SWEEP_CREATOR_FEE: [u8; 8] = [32, 246, 191, 52, 8, 201, 73, 186];
pub const IX_COLLECT_CREATOR_FEE_V2: [u8; 8] = [207, 17, 138, 242, 4, 34, 19, 56];
pub const IX_BUY_EXACT_QUOTE_IN_V3: [u8; 8] = [225, 247, 80, 30, 213, 179, 132, 136];
pub const IX_INIT_USER_VOLUME: [u8; 8] = [94, 6, 202, 115, 255, 96, 232, 183];
pub const IX_AMM_COLLECT_COIN_CREATOR_FEE: [u8; 8] = [160, 57, 89, 42, 181, 139, 43, 66];
pub const IX_AMM_BUY_EXACT_QUOTE_IN_V2: [u8; 8] = [194, 171, 28, 70, 104, 77, 91, 47];
pub const IX_AMM_DEPOSIT: [u8; 8] = [242, 35, 198, 137, 82, 225, 242, 182];

// Account codes.
pub const ACC_BONDING_CURVE: [u8; 8] = [23, 183, 248, 55, 96, 216, 172, 96];
pub const ACC_GLOBAL: [u8; 8] = [167, 232, 232, 177, 200, 108, 114, 127];
pub const ACC_POOL: [u8; 8] = [241, 154, 109, 4, 17, 177, 109, 188];
pub const ACC_AMM_GLOBAL_CONFIG: [u8; 8] = [149, 8, 156, 202, 160, 252, 176, 217];

/// Where the eight buy-back receivers sit in pump.fun's settings accounts.
pub const GLOBAL_BUYBACK_OFFSET: usize = 741;
pub const AMM_GLOBAL_BUYBACK_OFFSET: usize = 643;
pub const BUYBACK_COUNT: usize = 8;

fn u64_at(d: &[u8], o: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&d[o..o + 8]);
    u64::from_le_bytes(b)
}
fn key_at(d: &[u8], o: usize) -> Pubkey {
    let mut b = [0u8; 32];
    b.copy_from_slice(&d[o..o + 32]);
    Pubkey::new_from_array(b)
}

/// The fields of a pump.fun curve that we rely on.
pub struct CurveView {
    pub virtual_token_reserves: u64,
    pub virtual_quote_reserves: u64,
    pub complete: bool,
    pub creator: Pubkey,
    pub is_mayhem_mode: bool,
    pub is_cashback_coin: bool,
    pub quote_mint: Pubkey,
    pub creator_fee_bps: u64,
    /// pump.fun's flag for a fee that can be edited later. Documented as
    /// reserved and always false.
    pub can_edit_creator_fee: bool,
    pub is_holder_reward: bool,
}

const CURVE_MIN_LEN: usize = 125;

/// Reads a curve account. It must be owned by pump.fun's program and carry
/// the curve's code. The caller checks its address.
pub fn read_curve(ai: &AccountInfo) -> Result<CurveView> {
    require_keys_eq!(*ai.owner, PUMP_ID, BurrpadError::WrongPumpAccount);
    let d = ai.try_borrow_data()?;
    require!(d.len() >= CURVE_MIN_LEN && d[..8] == ACC_BONDING_CURVE, BurrpadError::UnreadablePumpAccount);
    Ok(CurveView {
        virtual_token_reserves: u64_at(&d, 8),
        virtual_quote_reserves: u64_at(&d, 16),
        complete: d[48] != 0,
        creator: key_at(&d, 49),
        is_mayhem_mode: d[81] != 0,
        is_cashback_coin: d[82] != 0,
        quote_mint: key_at(&d, 83),
        creator_fee_bps: u64_at(&d, 115),
        can_edit_creator_fee: d[123] != 0,
        is_holder_reward: d[124] != 0,
    })
}

/// The fields of a PumpSwap pool that we rely on.
pub struct PoolView {
    /// Who made the pool. For a graduated coin, pump.fun's own derived address.
    pub pool_creator: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub pool_base_token_account: Pubkey,
    pub pool_quote_token_account: Pubkey,
    /// Added to the pool's ZEC balance to get the reserve prices are worked from.
    pub virtual_quote_reserves: i128,
    /// The creator fee the pool charges now.
    pub creator_fee_bps: u64,
}

const POOL_MIN_LEN: usize = 269;

/// Reads a pool account. It must be owned by PumpSwap and carry the pool's code.
pub fn read_pool(ai: &AccountInfo) -> Result<PoolView> {
    require_keys_eq!(*ai.owner, PUMP_AMM_ID, BurrpadError::WrongPumpAccount);
    let d = ai.try_borrow_data()?;
    require!(d.len() >= POOL_MIN_LEN && d[..8] == ACC_POOL, BurrpadError::UnreadablePumpAccount);
    let mut v = [0u8; 16];
    v.copy_from_slice(&d[245..261]);
    Ok(PoolView {
        pool_creator: key_at(&d, 11),
        base_mint: key_at(&d, 43),
        quote_mint: key_at(&d, 75),
        pool_base_token_account: key_at(&d, 139),
        pool_quote_token_account: key_at(&d, 171),
        virtual_quote_reserves: i128::from_le_bytes(v),
        creator_fee_bps: u64_at(&d, 261),
    })
}

/// A pool's receipts, and the fees waiting in its ZEC account.
pub struct PoolShares {
    pub lp_mint: Pubkey,
    /// Receipts issued, burned ones included: a burn does not lower it.
    pub lp_supply: u64,
    /// Creator and protocol fees waiting in the pool's ZEC account. A deposit
    /// counts the pool's ZEC as its balance less these (measured on the bench
    /// against PumpSwap's own program).
    pub fees_waiting: u64,
}

const POOL_SHARES_LEN: usize = 287;

/// Reads a pool's receipt figures. Same owner and code checks as `read_pool`.
pub fn read_pool_shares(ai: &AccountInfo) -> Result<PoolShares> {
    require_keys_eq!(*ai.owner, PUMP_AMM_ID, BurrpadError::WrongPumpAccount);
    let d = ai.try_borrow_data()?;
    require!(d.len() >= POOL_SHARES_LEN && d[..8] == ACC_POOL, BurrpadError::UnreadablePumpAccount);
    Ok(PoolShares {
        lp_mint: key_at(&d, 107),
        lp_supply: u64_at(&d, 203),
        fees_waiting: u64_at(&d, 271).checked_add(u64_at(&d, 279)).ok_or(BurrpadError::Overflow)?,
    })
}

/// The reserve a pool's prices are worked from: its ZEC balance plus the
/// pool's own adjustment. Must be positive and fit.
pub fn effective_quote_reserves(balance: u64, virtual_quote_reserves: i128) -> Result<u64> {
    let v = (balance as i128).checked_add(virtual_quote_reserves).ok_or(BurrpadError::Overflow)?;
    require!(v > 0, BurrpadError::UnreadablePumpAccount);
    u64::try_from(v).map_err(|_| BurrpadError::Overflow.into())
}

/// True if `token_account` is a ZEC account owned by one of the eight
/// buy-back receivers listed in pump.fun's own settings account.
pub fn check_buyback_recipient(
    settings: &AccountInfo,
    settings_owner: &Pubkey,
    settings_code: &[u8; 8],
    list_offset: usize,
    token_account: &AccountInfo,
    zec_mint: &Pubkey,
    token_program: &Pubkey,
) -> Result<()> {
    require_keys_eq!(*settings.owner, *settings_owner, BurrpadError::WrongPumpAccount);
    require_keys_eq!(*token_account.owner, *token_program, BurrpadError::WrongPumpAccount);
    let t = token_account.try_borrow_data()?;
    require!(t.len() == 165, BurrpadError::WrongPumpAccount);
    require_keys_eq!(key_at(&t, 0), *zec_mint, BurrpadError::WrongPumpAccount);
    let holder = key_at(&t, 32);
    let s = settings.try_borrow_data()?;
    require!(
        s.len() >= list_offset + 32 * BUYBACK_COUNT && s[..8] == *settings_code,
        BurrpadError::UnreadablePumpAccount
    );
    let listed = (0..BUYBACK_COUNT).any(|i| key_at(&s, list_offset + 32 * i) == holder);
    require!(listed && holder != Pubkey::default(), BurrpadError::WrongPumpAccount);
    Ok(())
}

/// Checks that a coin's mint is the plain kind pump.fun makes today:
/// Token-2022, 6 decimals, nobody able to mint or freeze, and no extension
/// other than its metadata. Anything else could break our accounting
/// (a transfer fee, a hook, a delegate), so it is refused.
pub fn check_plain_coin_mint(mint: &AccountInfo, token_2022: &Pubkey) -> Result<()> {
    use crate::constants::{COIN_DECIMALS, EXT_METADATA_POINTER, EXT_TOKEN_METADATA};
    require_keys_eq!(*mint.owner, *token_2022, BurrpadError::UnexpectedMint);
    let d = mint.try_borrow_data()?;
    require!(d.len() >= 82, BurrpadError::UnexpectedMint);
    let mint_authority_set = d[0..4] != [0, 0, 0, 0];
    let freeze_authority_set = d[46..50] != [0, 0, 0, 0];
    require!(
        !mint_authority_set && !freeze_authority_set && d[44] == COIN_DECIMALS && d[45] == 1,
        BurrpadError::UnexpectedMint
    );
    if d.len() > 82 {
        // Token-2022 pads the base to 165 bytes, then one byte for the kind
        // of account (1 = mint), then extensions as: kind u16, length u16, data.
        require!(d.len() > 166 && d[165] == 1, BurrpadError::UnexpectedMint);
        let mut o = 166;
        while o + 4 <= d.len() {
            let kind = u16::from_le_bytes([d[o], d[o + 1]]);
            let len = u16::from_le_bytes([d[o + 2], d[o + 3]]) as usize;
            if kind == 0 {
                break;
            }
            require!(kind == EXT_METADATA_POINTER || kind == EXT_TOKEN_METADATA, BurrpadError::UnexpectedMint);
            o = o.checked_add(4 + len).ok_or(BurrpadError::UnexpectedMint)?;
        }
    }
    Ok(())
}

fn ro(k: &AccountInfo) -> AccountMeta {
    AccountMeta::new_readonly(*k.key, false)
}
fn rw(k: &AccountInfo) -> AccountMeta {
    AccountMeta::new(*k.key, false)
}
fn rws(k: &AccountInfo) -> AccountMeta {
    AccountMeta::new(*k.key, true)
}

fn call(
    program: &AccountInfo,
    expected: &Pubkey,
    accounts: Vec<AccountMeta>,
    infos: &[AccountInfo],
    data: Vec<u8>,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    require_keys_eq!(*program.key, *expected, BurrpadError::WrongPumpAccount);
    let ix = Instruction { program_id: *expected, accounts, data };
    invoke_signed(&ix, infos, signer_seeds).map_err(Into::into)
}

fn put_str(v: &mut Vec<u8>, s: &str) {
    v.extend_from_slice(&(s.len() as u32).to_le_bytes());
    v.extend_from_slice(s.as_bytes());
}

/// Accounts of `create_v2` for a ZEC-priced coin.
pub struct CreateV2<'a, 'info> {
    pub mint: &'a AccountInfo<'info>,
    pub mint_authority: &'a AccountInfo<'info>,
    pub bonding_curve: &'a AccountInfo<'info>,
    pub associated_bonding_curve: &'a AccountInfo<'info>,
    pub global: &'a AccountInfo<'info>,
    pub user: &'a AccountInfo<'info>,
    pub system_program: &'a AccountInfo<'info>,
    pub token_2022_program: &'a AccountInfo<'info>,
    pub associated_token_program: &'a AccountInfo<'info>,
    pub mayhem_program: &'a AccountInfo<'info>,
    pub global_params: &'a AccountInfo<'info>,
    pub sol_vault: &'a AccountInfo<'info>,
    pub mayhem_state: &'a AccountInfo<'info>,
    pub mayhem_token_vault: &'a AccountInfo<'info>,
    pub event_authority: &'a AccountInfo<'info>,
    pub pump_program: &'a AccountInfo<'info>,
    pub quote_mint: &'a AccountInfo<'info>,
    pub associated_quote_bonding_curve: &'a AccountInfo<'info>,
    pub quote_token_program: &'a AccountInfo<'info>,
    pub quote_control: &'a AccountInfo<'info>,
}

/// Creates the coin on pump.fun. Mayhem, cashback and holder-rewards are
/// always off; the caller chooses only the text, the creator and the fee.
pub fn create_v2(a: CreateV2, name: &str, symbol: &str, uri: &str, creator: &Pubkey, fee_bps: u16) -> Result<()> {
    let mut data = IX_CREATE_V2.to_vec();
    put_str(&mut data, name);
    put_str(&mut data, symbol);
    put_str(&mut data, uri);
    data.extend_from_slice(creator.as_ref());
    data.push(0); // is_mayhem_mode
    data.push(0); // is_cashback_enabled
    data.extend_from_slice(&(fee_bps as u64).to_le_bytes());
    data.push(0); // is_holder_reward
    let metas = vec![
        rws(a.mint),
        ro(a.mint_authority),
        rw(a.bonding_curve),
        rw(a.associated_bonding_curve),
        ro(a.global),
        rws(a.user),
        ro(a.system_program),
        ro(a.token_2022_program),
        ro(a.associated_token_program),
        rw(a.mayhem_program),
        ro(a.global_params),
        rw(a.sol_vault),
        rw(a.mayhem_state),
        rw(a.mayhem_token_vault),
        ro(a.event_authority),
        ro(a.pump_program),
        ro(a.quote_mint),
        rw(a.associated_quote_bonding_curve),
        ro(a.quote_token_program),
        ro(a.quote_control),
    ];
    let infos = [
        a.mint.clone(),
        a.mint_authority.clone(),
        a.bonding_curve.clone(),
        a.associated_bonding_curve.clone(),
        a.global.clone(),
        a.user.clone(),
        a.system_program.clone(),
        a.token_2022_program.clone(),
        a.associated_token_program.clone(),
        a.mayhem_program.clone(),
        a.global_params.clone(),
        a.sol_vault.clone(),
        a.mayhem_state.clone(),
        a.mayhem_token_vault.clone(),
        a.event_authority.clone(),
        a.pump_program.clone(),
        a.quote_mint.clone(),
        a.associated_quote_bonding_curve.clone(),
        a.quote_token_program.clone(),
        a.quote_control.clone(),
    ];
    call(a.pump_program, &PUMP_ID, metas, &infos, data, &[])
}

/// pump.fun `sweep_creator_fee`: moves the fee waiting on the curve into
/// pump.fun's creator vault. Open to anyone.
#[allow(clippy::too_many_arguments)]
pub fn sweep_creator_fee_curve<'info>(
    pump_program: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    global: &AccountInfo<'info>,
    base_mint: &AccountInfo<'info>,
    quote_mint: &AccountInfo<'info>,
    quote_token_program: &AccountInfo<'info>,
    associated_token_program: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    bonding_curve: &AccountInfo<'info>,
    associated_quote_bonding_curve: &AccountInfo<'info>,
    creator_vault: &AccountInfo<'info>,
    creator_vault_ata: &AccountInfo<'info>,
    event_authority: &AccountInfo<'info>,
) -> Result<()> {
    let metas = vec![
        rws(payer),
        ro(global),
        ro(base_mint),
        ro(quote_mint),
        ro(quote_token_program),
        ro(associated_token_program),
        ro(system_program),
        rw(bonding_curve),
        rw(associated_quote_bonding_curve),
        rw(creator_vault),
        rw(creator_vault_ata),
        ro(event_authority),
        ro(pump_program),
    ];
    let infos = [
        payer.clone(),
        global.clone(),
        base_mint.clone(),
        quote_mint.clone(),
        quote_token_program.clone(),
        associated_token_program.clone(),
        system_program.clone(),
        bonding_curve.clone(),
        associated_quote_bonding_curve.clone(),
        creator_vault.clone(),
        creator_vault_ata.clone(),
        event_authority.clone(),
        pump_program.clone(),
    ];
    call(pump_program, &PUMP_ID, metas, &infos, IX_SWEEP_CREATOR_FEE.to_vec(), &[])
}

/// pump.fun `collect_creator_fee_v2`: moves pump.fun's creator vault into the
/// creator's own ZEC account. Open to anyone; the creator does not sign.
#[allow(clippy::too_many_arguments)]
pub fn collect_creator_fee_curve<'info>(
    pump_program: &AccountInfo<'info>,
    creator: &AccountInfo<'info>,
    creator_token_account: &AccountInfo<'info>,
    creator_vault: &AccountInfo<'info>,
    creator_vault_ata: &AccountInfo<'info>,
    quote_mint: &AccountInfo<'info>,
    quote_token_program: &AccountInfo<'info>,
    associated_token_program: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    event_authority: &AccountInfo<'info>,
) -> Result<()> {
    let metas = vec![
        rw(creator),
        rw(creator_token_account),
        rw(creator_vault),
        rw(creator_vault_ata),
        ro(quote_mint),
        ro(quote_token_program),
        ro(associated_token_program),
        ro(system_program),
        ro(event_authority),
        ro(pump_program),
    ];
    let infos = [
        creator.clone(),
        creator_token_account.clone(),
        creator_vault.clone(),
        creator_vault_ata.clone(),
        quote_mint.clone(),
        quote_token_program.clone(),
        associated_token_program.clone(),
        system_program.clone(),
        event_authority.clone(),
        pump_program.clone(),
    ];
    call(pump_program, &PUMP_ID, metas, &infos, IX_COLLECT_CREATOR_FEE_V2.to_vec(), &[])
}

/// PumpSwap `sweep_creator_fee`: moves the fee waiting in the pool into
/// PumpSwap's creator vault. Open to anyone.
#[allow(clippy::too_many_arguments)]
pub fn sweep_creator_fee_pool<'info>(
    amm_program: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    global_config: &AccountInfo<'info>,
    pool: &AccountInfo<'info>,
    quote_mint: &AccountInfo<'info>,
    quote_token_program: &AccountInfo<'info>,
    pool_quote_token_account: &AccountInfo<'info>,
    vault_authority: &AccountInfo<'info>,
    vault_ata: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    associated_token_program: &AccountInfo<'info>,
    event_authority: &AccountInfo<'info>,
) -> Result<()> {
    let metas = vec![
        rws(payer),
        ro(global_config),
        rw(pool),
        ro(quote_mint),
        ro(quote_token_program),
        rw(pool_quote_token_account),
        ro(vault_authority),
        rw(vault_ata),
        ro(system_program),
        ro(associated_token_program),
        ro(event_authority),
        ro(amm_program),
    ];
    let infos = [
        payer.clone(),
        global_config.clone(),
        pool.clone(),
        quote_mint.clone(),
        quote_token_program.clone(),
        pool_quote_token_account.clone(),
        vault_authority.clone(),
        vault_ata.clone(),
        system_program.clone(),
        associated_token_program.clone(),
        event_authority.clone(),
        amm_program.clone(),
    ];
    call(amm_program, &PUMP_AMM_ID, metas, &infos, IX_SWEEP_CREATOR_FEE.to_vec(), &[])
}

/// PumpSwap `collect_coin_creator_fee`: moves PumpSwap's creator vault into
/// the creator's own ZEC account. Open to anyone.
#[allow(clippy::too_many_arguments)]
pub fn collect_creator_fee_pool<'info>(
    amm_program: &AccountInfo<'info>,
    quote_mint: &AccountInfo<'info>,
    quote_token_program: &AccountInfo<'info>,
    coin_creator: &AccountInfo<'info>,
    vault_authority: &AccountInfo<'info>,
    vault_ata: &AccountInfo<'info>,
    coin_creator_token_account: &AccountInfo<'info>,
    event_authority: &AccountInfo<'info>,
) -> Result<()> {
    let metas = vec![
        ro(quote_mint),
        ro(quote_token_program),
        ro(coin_creator),
        ro(vault_authority),
        rw(vault_ata),
        rw(coin_creator_token_account),
        ro(event_authority),
        ro(amm_program),
    ];
    let infos = [
        quote_mint.clone(),
        quote_token_program.clone(),
        coin_creator.clone(),
        vault_authority.clone(),
        vault_ata.clone(),
        coin_creator_token_account.clone(),
        event_authority.clone(),
        amm_program.clone(),
    ];
    call(amm_program, &PUMP_AMM_ID, metas, &infos, IX_AMM_COLLECT_COIN_CREATOR_FEE.to_vec(), &[])
}

/// `init_user_volume_accumulator`, the same on both programs: makes the
/// buyer's volume record with a separate payer, so a buyer that is a
/// program-controlled address needs no SOL of its own.
pub fn init_user_volume<'info>(
    program: &AccountInfo<'info>,
    expected: &Pubkey,
    payer: &AccountInfo<'info>,
    user: &AccountInfo<'info>,
    user_volume: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    event_authority: &AccountInfo<'info>,
) -> Result<()> {
    let metas = vec![rws(payer), ro(user), rw(user_volume), ro(system_program), ro(event_authority), ro(program)];
    let infos = [
        payer.clone(),
        user.clone(),
        user_volume.clone(),
        system_program.clone(),
        event_authority.clone(),
        program.clone(),
    ];
    call(program, expected, metas, &infos, IX_INIT_USER_VOLUME.to_vec(), &[])
}

/// Accounts of a buy on the curve (`buy_exact_quote_in_v3`).
pub struct CurveBuy<'a, 'info> {
    pub pump_program: &'a AccountInfo<'info>,
    pub global: &'a AccountInfo<'info>,
    pub base_mint: &'a AccountInfo<'info>,
    pub quote_mint: &'a AccountInfo<'info>,
    pub base_token_program: &'a AccountInfo<'info>,
    pub quote_token_program: &'a AccountInfo<'info>,
    pub bonding_curve: &'a AccountInfo<'info>,
    pub associated_base_bonding_curve: &'a AccountInfo<'info>,
    pub associated_quote_bonding_curve: &'a AccountInfo<'info>,
    pub user: &'a AccountInfo<'info>,
    pub user_base: &'a AccountInfo<'info>,
    pub user_quote: &'a AccountInfo<'info>,
    pub user_volume: &'a AccountInfo<'info>,
    pub fee_config: &'a AccountInfo<'info>,
    pub buyback_fee_recipient: &'a AccountInfo<'info>,
    pub system_program: &'a AccountInfo<'info>,
    pub event_authority: &'a AccountInfo<'info>,
}

/// Buys on the curve, spending exactly `quote_in` (fees included) for at
/// least `min_out` coins. `signer_seeds` is empty when the buyer signed the
/// transaction, or the buyer's seeds when it is a program-controlled address.
pub fn buy_on_curve(a: CurveBuy, quote_in: u64, min_out: u64, signer_seeds: &[&[&[u8]]]) -> Result<()> {
    let mut data = IX_BUY_EXACT_QUOTE_IN_V3.to_vec();
    data.extend_from_slice(&quote_in.to_le_bytes());
    data.extend_from_slice(&min_out.to_le_bytes());
    data.push(0); // partial_fill: off
    let metas = vec![
        ro(a.global),
        ro(a.base_mint),
        ro(a.quote_mint),
        ro(a.base_token_program),
        ro(a.quote_token_program),
        rw(a.bonding_curve),
        rw(a.associated_base_bonding_curve),
        rw(a.associated_quote_bonding_curve),
        rws(a.user),
        rw(a.user_base),
        rw(a.user_quote),
        rw(a.user_volume),
        ro(a.fee_config),
        rw(a.buyback_fee_recipient),
        ro(a.system_program),
        ro(a.event_authority),
        ro(a.pump_program),
    ];
    let infos = [
        a.global.clone(),
        a.base_mint.clone(),
        a.quote_mint.clone(),
        a.base_token_program.clone(),
        a.quote_token_program.clone(),
        a.bonding_curve.clone(),
        a.associated_base_bonding_curve.clone(),
        a.associated_quote_bonding_curve.clone(),
        a.user.clone(),
        a.user_base.clone(),
        a.user_quote.clone(),
        a.user_volume.clone(),
        a.fee_config.clone(),
        a.buyback_fee_recipient.clone(),
        a.system_program.clone(),
        a.event_authority.clone(),
        a.pump_program.clone(),
    ];
    call(a.pump_program, &PUMP_ID, metas, &infos, data, signer_seeds)
}

/// Accounts of a buy on a pool (`buy_exact_quote_in_v2`).
pub struct PoolBuy<'a, 'info> {
    pub amm_program: &'a AccountInfo<'info>,
    pub pool: &'a AccountInfo<'info>,
    pub user: &'a AccountInfo<'info>,
    pub global_config: &'a AccountInfo<'info>,
    pub base_mint: &'a AccountInfo<'info>,
    pub quote_mint: &'a AccountInfo<'info>,
    pub user_base: &'a AccountInfo<'info>,
    pub user_quote: &'a AccountInfo<'info>,
    pub pool_base: &'a AccountInfo<'info>,
    pub pool_quote: &'a AccountInfo<'info>,
    pub base_token_program: &'a AccountInfo<'info>,
    pub quote_token_program: &'a AccountInfo<'info>,
    pub system_program: &'a AccountInfo<'info>,
    pub user_volume: &'a AccountInfo<'info>,
    pub fee_config: &'a AccountInfo<'info>,
    pub buyback_fee_recipient: &'a AccountInfo<'info>,
    pub event_authority: &'a AccountInfo<'info>,
}

/// Buys on a pool, spending exactly `quote_in` (fees included) for at least
/// `min_out` coins.
pub fn buy_on_pool(a: PoolBuy, quote_in: u64, min_out: u64, signer_seeds: &[&[&[u8]]]) -> Result<()> {
    let mut data = IX_AMM_BUY_EXACT_QUOTE_IN_V2.to_vec();
    data.extend_from_slice(&quote_in.to_le_bytes());
    data.extend_from_slice(&min_out.to_le_bytes());
    let metas = vec![
        rw(a.pool),
        rws(a.user),
        ro(a.global_config),
        ro(a.base_mint),
        ro(a.quote_mint),
        rw(a.user_base),
        rw(a.user_quote),
        rw(a.pool_base),
        rw(a.pool_quote),
        ro(a.base_token_program),
        ro(a.quote_token_program),
        ro(a.system_program),
        rw(a.user_volume),
        ro(a.fee_config),
        rw(a.buyback_fee_recipient),
        ro(a.event_authority),
        ro(a.amm_program),
    ];
    let infos = [
        a.pool.clone(),
        a.user.clone(),
        a.global_config.clone(),
        a.base_mint.clone(),
        a.quote_mint.clone(),
        a.user_base.clone(),
        a.user_quote.clone(),
        a.pool_base.clone(),
        a.pool_quote.clone(),
        a.base_token_program.clone(),
        a.quote_token_program.clone(),
        a.system_program.clone(),
        a.user_volume.clone(),
        a.fee_config.clone(),
        a.buyback_fee_recipient.clone(),
        a.event_authority.clone(),
        a.amm_program.clone(),
    ];
    call(a.amm_program, &PUMP_AMM_ID, metas, &infos, data, signer_seeds)
}

/// Accounts of a deposit into a pool (`deposit`).
pub struct PoolDeposit<'a, 'info> {
    pub amm_program: &'a AccountInfo<'info>,
    pub pool: &'a AccountInfo<'info>,
    pub global_config: &'a AccountInfo<'info>,
    pub user: &'a AccountInfo<'info>,
    pub base_mint: &'a AccountInfo<'info>,
    pub quote_mint: &'a AccountInfo<'info>,
    pub lp_mint: &'a AccountInfo<'info>,
    pub user_base: &'a AccountInfo<'info>,
    pub user_quote: &'a AccountInfo<'info>,
    pub user_lp: &'a AccountInfo<'info>,
    pub pool_base: &'a AccountInfo<'info>,
    pub pool_quote: &'a AccountInfo<'info>,
    pub token_program: &'a AccountInfo<'info>,
    pub token_2022_program: &'a AccountInfo<'info>,
    pub event_authority: &'a AccountInfo<'info>,
}

/// Deposits into a pool for exactly `lp_out` receipts, taking at most the two amounts.
pub fn deposit_to_pool(
    a: PoolDeposit,
    lp_out: u64,
    max_base: u64,
    max_quote: u64,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    let mut data = IX_AMM_DEPOSIT.to_vec();
    for v in [lp_out, max_base, max_quote] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    let metas = vec![
        rw(a.pool),
        ro(a.global_config),
        rws(a.user),
        ro(a.base_mint),
        ro(a.quote_mint),
        rw(a.lp_mint),
        rw(a.user_base),
        rw(a.user_quote),
        rw(a.user_lp),
        rw(a.pool_base),
        rw(a.pool_quote),
        ro(a.token_program),
        ro(a.token_2022_program),
        ro(a.event_authority),
        ro(a.amm_program),
    ];
    let infos = [
        a.pool.clone(),
        a.global_config.clone(),
        a.user.clone(),
        a.base_mint.clone(),
        a.quote_mint.clone(),
        a.lp_mint.clone(),
        a.user_base.clone(),
        a.user_quote.clone(),
        a.user_lp.clone(),
        a.pool_base.clone(),
        a.pool_quote.clone(),
        a.token_program.clone(),
        a.token_2022_program.clone(),
        a.event_authority.clone(),
        a.amm_program.clone(),
    ];
    call(a.amm_program, &PUMP_AMM_ID, metas, &infos, data, signer_seeds)
}
