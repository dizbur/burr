//! The readers and checks on pump.fun's accounts and on a coin's mint, fed
//! with real accounts and with doctored ones.

use crate::bench::Bench;
use crate::kit::{LaunchPlan, World};
use crate::pump::{self, ata, PUMP, PUMP_AMM, TOKEN, TOKEN_2022, ZEC};
use anchor_lang::prelude::AccountInfo;
use burrpad::pump as p;
use solana_pubkey::Pubkey;

/// Runs `f` on an account built from the given parts.
fn with<R>(key: Pubkey, owner: Pubkey, mut data: Vec<u8>, f: impl FnOnce(&AccountInfo) -> R) -> R {
    let mut lamports = 1_000_000u64;
    let ai = AccountInfo::new(&key, false, false, &mut lamports, &mut data, &owner, false);
    f(&ai)
}

fn real_coin_mint() -> (World, Pubkey, Vec<u8>) {
    let mut w = World::new();
    let mint = w.launch(&LaunchPlan::default());
    let data = w.b.account(&mint).unwrap().data;
    (w, mint, data)
}

fn mint_ok(owner: Pubkey, data: Vec<u8>) -> bool {
    with(Pubkey::new_unique(), owner, data, |ai| p::check_plain_coin_mint(ai, &TOKEN_2022).is_ok())
}

/// Appends one Token-2022 extension of the given kind after the existing ones.
fn with_extension(mut data: Vec<u8>, kind: u16, len: u16) -> Vec<u8> {
    let mut o = 166;
    while o + 4 <= data.len() {
        let k = u16::from_le_bytes([data[o], data[o + 1]]);
        let l = u16::from_le_bytes([data[o + 2], data[o + 3]]) as usize;
        if k == 0 {
            break;
        }
        o += 4 + l;
    }
    data.truncate(o);
    data.extend_from_slice(&kind.to_le_bytes());
    data.extend_from_slice(&len.to_le_bytes());
    data.extend(std::iter::repeat_n(0u8, len as usize));
    data
}

#[test]
fn a_real_pump_fun_coin_mint_is_accepted() {
    let (_w, _mint, data) = real_coin_mint();
    assert!(mint_ok(TOKEN_2022, data));
}

#[test]
fn a_mint_that_is_not_the_plain_kind_is_refused() {
    let (_w, _mint, data) = real_coin_mint();
    // Owned by the classic token program.
    assert!(!mint_ok(TOKEN, data.clone()));
    // Someone can still mint.
    let mut d = data.clone();
    d[0] = 1;
    assert!(!mint_ok(TOKEN_2022, d));
    // Someone can freeze.
    let mut d = data.clone();
    d[46] = 1;
    assert!(!mint_ok(TOKEN_2022, d));
    // Wrong decimals.
    let mut d = data.clone();
    d[44] = 9;
    assert!(!mint_ok(TOKEN_2022, d));
    // Not initialised.
    let mut d = data.clone();
    d[45] = 0;
    assert!(!mint_ok(TOKEN_2022, d));
    // Not a mint at all (a token account's kind byte).
    let mut d = data.clone();
    d[165] = 2;
    assert!(!mint_ok(TOKEN_2022, d));
    // Too short.
    assert!(!mint_ok(TOKEN_2022, data[..50].to_vec()));
    // Any extension that could change how coins move or who controls them.
    for (kind, name) in [
        (1u16, "transfer fee"),
        (3, "mint close authority"),
        (4, "confidential transfer"),
        (6, "default account state"),
        (9, "non-transferable"),
        (10, "interest-bearing"),
        (12, "permanent delegate"),
        (14, "transfer hook"),
        (25, "scaled amount"),
        (26, "pausable"),
    ] {
        assert!(!mint_ok(TOKEN_2022, with_extension(data.clone(), kind, 32)), "{name} was accepted");
    }
    // A second metadata extension is still just metadata.
    assert!(mint_ok(TOKEN_2022, with_extension(data, 19, 8)));
}

#[test]
fn the_curve_reader_checks_owner_code_and_length() {
    let (w, mint, _) = real_coin_mint();
    let key = pump::curve(&mint);
    let data = w.b.account(&key).unwrap().data;
    let authority = crate::kit::authority(&mint);
    let good =
        with(key, PUMP, data.clone(), |ai| p::read_curve(ai).map(|c| (c.creator, c.quote_mint, c.creator_fee_bps)));
    assert_eq!(good.unwrap(), (authority, ZEC, 200));
    // Owned by another program: a look-alike.
    assert!(with(key, Pubkey::new_unique(), data.clone(), |ai| p::read_curve(ai).is_err()));
    assert!(with(key, PUMP_AMM, data.clone(), |ai| p::read_curve(ai).is_err()));
    // Another kind of pump.fun account.
    let mut d = data.clone();
    d[..8].copy_from_slice(&p::ACC_GLOBAL);
    assert!(with(key, PUMP, d, |ai| p::read_curve(ai).is_err()));
    // Cut short.
    assert!(with(key, PUMP, data[..124].to_vec(), |ai| p::read_curve(ai).is_err()));
    // Longer is fine: pump.fun adds fields at the end.
    let mut d = data;
    d.extend_from_slice(&[0u8; 64]);
    assert!(with(key, PUMP, d, |ai| p::read_curve(ai).is_ok()));
}

#[test]
fn the_pool_reader_checks_owner_code_and_length() {
    let mut w = World::new();
    let mint = w.launch(&LaunchPlan::default());
    w.graduate(&mint);
    let key = pump::pool(&mint);
    let data = w.b.account(&key).unwrap().data;
    let good = with(key, PUMP_AMM, data.clone(), |ai| p::read_pool(ai).map(|x| (x.base_mint, x.quote_mint)));
    assert_eq!(good.unwrap(), (mint, ZEC));
    assert!(with(key, PUMP, data.clone(), |ai| p::read_pool(ai).is_err()));
    let mut d = data.clone();
    d[0] ^= 1;
    assert!(with(key, PUMP_AMM, d, |ai| p::read_pool(ai).is_err()));
    assert!(with(key, PUMP_AMM, data[..260].to_vec(), |ai| p::read_pool(ai).is_err()));
}

#[test]
fn the_pools_price_reserve_must_be_positive_and_fit() {
    assert_eq!(p::effective_quote_reserves(100, 50).unwrap(), 150);
    assert_eq!(p::effective_quote_reserves(100, -40).unwrap(), 60);
    assert!(p::effective_quote_reserves(100, -100).is_err());
    assert!(p::effective_quote_reserves(100, -101).is_err());
    assert!(p::effective_quote_reserves(u64::MAX, 1).is_err());
    assert!(p::effective_quote_reserves(0, i128::MAX).is_err());
}

#[test]
fn only_a_listed_buy_back_receivers_zec_account_is_accepted() {
    let b = Bench::new();
    let global_key = pump::global();
    let global = b.account(&global_key).unwrap().data;
    let check = |settings_owner: Pubkey, settings: Vec<u8>, token_owner: Pubkey, token: Vec<u8>| -> bool {
        with(global_key, settings_owner, settings, |s| {
            with(Pubkey::new_unique(), token_owner, token, |t| {
                p::check_buyback_recipient(s, &PUMP, &p::ACC_GLOBAL, p::GLOBAL_BUYBACK_OFFSET, t, &ZEC, &TOKEN).is_ok()
            })
        })
    };
    let token = |mint: &Pubkey, holder: &Pubkey| -> Vec<u8> {
        let mut d = vec![0u8; 165];
        d[..32].copy_from_slice(mint.as_ref());
        d[32..64].copy_from_slice(holder.as_ref());
        d[108] = 1;
        d
    };
    let listed = pump::BUYBACK;
    assert!(check(PUMP, global.clone(), TOKEN, token(&ZEC, &listed)));
    // A real one from the chain too.
    let real = b.account(&ata(&listed, &ZEC, &TOKEN)).unwrap().data;
    assert!(check(PUMP, global.clone(), TOKEN, real));
    // Held by someone not on the list.
    assert!(!check(PUMP, global.clone(), TOKEN, token(&ZEC, &Pubkey::new_unique())));
    // The empty address is never a receiver, even if a list slot were empty.
    let mut emptied = global.clone();
    emptied[p::GLOBAL_BUYBACK_OFFSET..p::GLOBAL_BUYBACK_OFFSET + 32].fill(0);
    assert!(!check(PUMP, emptied, TOKEN, token(&ZEC, &Pubkey::default())));
    // Another coin's account.
    assert!(!check(PUMP, global.clone(), TOKEN, token(&Pubkey::new_unique(), &listed)));
    // A Token-2022 account dressed up the same.
    assert!(!check(PUMP, global.clone(), TOKEN_2022, token(&ZEC, &listed)));
    // A settings account that is not pump.fun's.
    assert!(!check(Pubkey::new_unique(), global.clone(), TOKEN, token(&ZEC, &listed)));
    // A settings account of the wrong kind, or cut short.
    let mut d = global.clone();
    d[0] ^= 1;
    assert!(!check(PUMP, d, TOKEN, token(&ZEC, &listed)));
    assert!(!check(PUMP, global[..800].to_vec(), TOKEN, token(&ZEC, &listed)));
    // A token account of the wrong size.
    assert!(!check(PUMP, global, TOKEN, token(&ZEC, &listed)[..100].to_vec()));
}
