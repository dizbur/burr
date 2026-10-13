//! A stand-in for pump.fun's programs, for tests only. It is loaded at
//! pump.fun's addresses inside the test network and misbehaves in a chosen
//! way, to prove that Burrpad concludes nothing from a call having succeeded.
//!
//! The way to misbehave is a single byte the test writes into pump.fun's
//! settings account (byte 8, which Burrpad never reads).

use solana_account_info::AccountInfo;
use solana_cpi::{invoke, invoke_signed};
use solana_instruction::{AccountMeta, Instruction};
use solana_program_error::{ProgramError, ProgramResult};
use solana_pubkey::Pubkey;

pub const IX_CREATE_V2: [u8; 8] = [214, 144, 76, 236, 95, 139, 49, 180];
pub const IX_BUY_CURVE: [u8; 8] = [225, 247, 80, 30, 213, 179, 132, 136];
pub const IX_BUY_POOL: [u8; 8] = [194, 171, 28, 70, 104, 77, 91, 47];
pub const ACC_BONDING_CURVE: [u8; 8] = [23, 183, 248, 55, 96, 216, 172, 96];

// What `create_v2` does.
pub const CREATE_RIGHT_CURVE_NO_MINT: u8 = 10;
pub const CREATE_WRONG_CREATOR: u8 = 11;
pub const CREATE_WRONG_QUOTE: u8 = 12;
pub const CREATE_WRONG_FEE: u8 = 13;
pub const CREATE_HOLDER_REWARDS: u8 = 14;
pub const CREATE_CASHBACK: u8 = 15;
pub const CREATE_MAYHEM: u8 = 16;
pub const CREATE_COMPLETE: u8 = 17;
pub const CREATE_NOTHING: u8 = 18;
pub const CREATE_EDITABLE_FEE: u8 = 19;

// What a buy does.
pub const BUY_TAKES_GIVES_NOTHING: u8 = 20;
pub const BUY_GIVES_ONE_TOO_FEW: u8 = 21;
pub const BUY_DOES_NOTHING: u8 = 22;
pub const BUY_TAKES_EVERYTHING: u8 = 23;
pub const BUY_HONEST: u8 = 25;
pub const BUY_GIVES_A_CRUMB: u8 = 26;

#[cfg(not(feature = "no-entrypoint"))]
solana_program_entrypoint::entrypoint!(process);

fn u64_at(d: &[u8], o: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&d[o..o + 8]);
    u64::from_le_bytes(b)
}

fn token_transfer<'a>(
    token_program: &AccountInfo<'a>,
    from: &AccountInfo<'a>,
    to: &AccountInfo<'a>,
    authority: &AccountInfo<'a>,
    amount: u64,
    seeds: &[&[&[u8]]],
) -> ProgramResult {
    if amount == 0 {
        return Ok(());
    }
    let mut data = vec![3u8];
    data.extend_from_slice(&amount.to_le_bytes());
    let ix = Instruction {
        program_id: *token_program.key,
        accounts: vec![
            AccountMeta::new(*from.key, false),
            AccountMeta::new(*to.key, false),
            AccountMeta::new_readonly(*authority.key, true),
        ],
        data,
    };
    let infos = [from.clone(), to.clone(), authority.clone()];
    if seeds.is_empty() {
        invoke(&ix, &infos)
    } else {
        invoke_signed(&ix, &infos, seeds)
    }
}

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    if data.len() < 8 {
        return Ok(());
    }
    let mut code = [0u8; 8];
    code.copy_from_slice(&data[..8]);
    match code {
        IX_CREATE_V2 => create(program_id, accounts, data),
        IX_BUY_CURVE => buy_curve(program_id, accounts, data),
        IX_BUY_POOL => buy_pool(program_id, accounts, data),
        // Sweep, collect, the volume record: succeed and do nothing.
        _ => Ok(()),
    }
}

fn create(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let mint = &accounts[0];
    let curve = &accounts[2];
    let global = &accounts[4];
    let user = &accounts[5];
    let system = &accounts[6];
    let quote_mint = &accounts[16];
    let mode = global.try_borrow_data()?[8];
    if mode == CREATE_NOTHING {
        return Ok(());
    }

    // Arguments: three strings, then creator, two flags, the fee.
    let mut o = 8;
    for _ in 0..3 {
        let n = u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]) as usize;
        o += 4 + n;
    }
    let mut creator = [0u8; 32];
    creator.copy_from_slice(&data[o..o + 32]);
    let fee = u64_at(data, o + 34);

    let (expected, bump) = Pubkey::find_program_address(&[b"bonding-curve", mint.key.as_ref()], program_id);
    if expected != *curve.key {
        return Err(ProgramError::InvalidSeeds);
    }
    // System program: create account (tag 0), lamports, space, owner.
    let mut d = vec![0u8, 0, 0, 0];
    d.extend_from_slice(&3_000_000u64.to_le_bytes());
    d.extend_from_slice(&166u64.to_le_bytes());
    d.extend_from_slice(program_id.as_ref());
    let ix = Instruction {
        program_id: *system.key,
        accounts: vec![AccountMeta::new(*user.key, true), AccountMeta::new(*curve.key, true)],
        data: d,
    };
    invoke_signed(&ix, &[user.clone(), curve.clone()], &[&[b"bonding-curve", mint.key.as_ref(), &[bump]]])?;

    let mut c = curve.try_borrow_mut_data()?;
    c[..8].copy_from_slice(&ACC_BONDING_CURVE);
    c[8..16].copy_from_slice(&1_073_000_000_000_000u64.to_le_bytes());
    c[16..24].copy_from_slice(&275_548_581u64.to_le_bytes());
    c[24..32].copy_from_slice(&793_100_000_000_000u64.to_le_bytes());
    c[40..48].copy_from_slice(&1_000_000_000_000_000u64.to_le_bytes());
    c[48] = (mode == CREATE_COMPLETE) as u8;
    if mode == CREATE_WRONG_CREATOR {
        creator[0] ^= 1;
    }
    c[49..81].copy_from_slice(&creator);
    c[81] = (mode == CREATE_MAYHEM) as u8;
    c[82] = (mode == CREATE_CASHBACK) as u8;
    if mode == CREATE_WRONG_QUOTE {
        c[83..115].copy_from_slice(mint.key.as_ref());
    } else {
        c[83..115].copy_from_slice(quote_mint.key.as_ref());
    }
    let fee = if mode == CREATE_WRONG_FEE { fee + 1 } else { fee };
    c[115..123].copy_from_slice(&fee.to_le_bytes());
    c[123] = (mode == CREATE_EDITABLE_FEE) as u8;
    c[124] = (mode == CREATE_HOLDER_REWARDS) as u8;
    Ok(())
}

/// The trade, shared by curve and pool once the accounts are picked out.
#[allow(clippy::too_many_arguments)]
fn trade<'a>(
    mode: u8,
    quote_in: u64,
    min_out: u64,
    base_token_program: &AccountInfo<'a>,
    quote_token_program: &AccountInfo<'a>,
    venue: &AccountInfo<'a>,
    venue_base: &AccountInfo<'a>,
    venue_quote: &AccountInfo<'a>,
    user: &AccountInfo<'a>,
    user_base: &AccountInfo<'a>,
    user_quote: &AccountInfo<'a>,
    venue_seeds: &[&[u8]],
) -> ProgramResult {
    let all = u64_at(&user_quote.try_borrow_data()?, 64);
    let (take, give) = match mode {
        BUY_TAKES_GIVES_NOTHING => (quote_in, 0),
        BUY_GIVES_ONE_TOO_FEW => (quote_in, min_out.saturating_sub(1)),
        BUY_DOES_NOTHING => (0, 0),
        BUY_TAKES_EVERYTHING => (all, min_out),
        BUY_HONEST => (quote_in, min_out),
        BUY_GIVES_A_CRUMB => (quote_in, 1_000),
        _ => return Err(ProgramError::InvalidArgument),
    };
    token_transfer(quote_token_program, user_quote, venue_quote, user, take, &[])?;
    token_transfer(base_token_program, venue_base, user_base, venue, give, &[venue_seeds])
}

fn buy_curve(program_id: &Pubkey, a: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let mode = a[0].try_borrow_data()?[8];
    let mint = *a[1].key;
    let (_, bump) = Pubkey::find_program_address(&[b"bonding-curve", mint.as_ref()], program_id);
    trade(
        mode,
        u64_at(data, 8),
        u64_at(data, 16),
        &a[3],
        &a[4],
        &a[5],
        &a[6],
        &a[7],
        &a[8],
        &a[9],
        &a[10],
        &[b"bonding-curve", mint.as_ref(), &[bump]],
    )
}

fn buy_pool(_program_id: &Pubkey, a: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let mode = a[2].try_borrow_data()?[8];
    // The pool's own seeds are in its account: bump, index, creator, the two mints.
    let (bump, creator, base, quote) = {
        let p = a[0].try_borrow_data()?;
        let mut k = [[0u8; 32]; 3];
        k[0].copy_from_slice(&p[11..43]);
        k[1].copy_from_slice(&p[43..75]);
        k[2].copy_from_slice(&p[75..107]);
        (p[8], k[0], k[1], k[2])
    };
    trade(
        mode,
        u64_at(data, 8),
        u64_at(data, 16),
        &a[9],
        &a[10],
        &a[0],
        &a[7],
        &a[8],
        &a[1],
        &a[5],
        &a[6],
        &[b"pool", &[0, 0], &creator, &base, &quote, &[bump]],
    )
}
