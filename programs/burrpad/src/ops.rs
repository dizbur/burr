//! Steps shared by several instructions: the split, and paying ZEC out of a
//! coin's vault.

use crate::constants::*;
use crate::error::BurrpadError;
use crate::events::{Split, SplitBurr, SplitLiquidity};
use crate::math;
use crate::state::{Coin, Ledger};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::{self, get_associated_token_address_with_program_id};
use anchor_spl::token_interface::{self, TransferChecked};

/// Splits whatever ZEC has arrived in the vault since the last split.
///
/// `vault_amount` must be the vault's balance read from the chain just now.
pub fn split_new(coin: &Coin, ledger: &mut Ledger, vault_amount: u64) -> Result<u64> {
    let new = vault_amount.checked_sub(ledger.accounted).ok_or(BurrpadError::VaultShort)?;
    if new == 0 {
        return Ok(0);
    }
    let w = math::weights_all(coin.platform_fraction_bps, &coin.shares, coin.burr_share, coin.liquidity_share)
        .ok_or(BurrpadError::SharesDoNotAddUp)?;
    let mut credited = [0u128; ALL_SINKS];
    credited[..SINKS].copy_from_slice(&ledger.credited);
    credited[SINK_BURR] = ledger.burr.credited;
    credited[SINK_LIQUIDITY] = ledger.liquidity.credited;
    let all = math::split(&mut ledger.total_split, &mut credited, &w, new).ok_or(BurrpadError::Overflow)?;
    ledger.credited.copy_from_slice(&credited[..SINKS]);
    ledger.burr.credited = credited[SINK_BURR];
    ledger.liquidity.credited = credited[SINK_LIQUIDITY];
    let mut credits = [0u64; SINKS];
    credits.copy_from_slice(&all[..SINKS]);
    for (pot, add) in ledger.pots.iter_mut().zip(credits.iter()) {
        *pot = pot.checked_add(*add).ok_or(BurrpadError::Overflow)?;
    }
    ledger.burr.pot = ledger.burr.pot.checked_add(all[SINK_BURR]).ok_or(BurrpadError::Overflow)?;
    ledger.liquidity.pot = ledger.liquidity.pot.checked_add(all[SINK_LIQUIDITY]).ok_or(BurrpadError::Overflow)?;
    ledger.accounted = ledger.accounted.checked_add(new).ok_or(BurrpadError::Overflow)?;
    emit!(Split { mint: coin.mint, amount: new, credited: credits });
    if all[SINK_BURR] > 0 {
        emit!(SplitBurr { mint: coin.mint, credited: all[SINK_BURR] });
    }
    if all[SINK_LIQUIDITY] > 0 {
        emit!(SplitLiquidity { mint: coin.mint, credited: all[SINK_LIQUIDITY] });
    }
    Ok(new)
}

/// Takes `amount` out of pot `sink` in the ledger. The caller then moves the ZEC.
pub fn debit(ledger: &mut Ledger, sink: usize, amount: u64) -> Result<()> {
    ledger.pots[sink] = ledger.pots[sink].checked_sub(amount).ok_or(BurrpadError::Overflow)?;
    ledger.accounted = ledger.accounted.checked_sub(amount).ok_or(BurrpadError::Overflow)?;
    Ok(())
}

/// Moves ZEC out of a coin's vault (or, for `recover`, out of another ZEC
/// account of the same authority), signed by the coin's authority.
#[allow(clippy::too_many_arguments)]
pub fn pay_from_vault<'info>(
    token_program: &AccountInfo<'info>,
    vault: &AccountInfo<'info>,
    zec_mint: &AccountInfo<'info>,
    to: &AccountInfo<'info>,
    authority: &AccountInfo<'info>,
    mint: &Pubkey,
    authority_bump: u8,
    amount: u64,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    let seeds: &[&[u8]] = &[SEED_AUTHORITY, mint.as_ref(), &[authority_bump]];
    token_interface::transfer_checked(
        CpiContext::new_with_signer(
            *token_program.key,
            TransferChecked {
                from: vault.clone(),
                mint: zec_mint.clone(),
                to: to.clone(),
                authority: authority.clone(),
            },
            &[seeds],
        ),
        amount,
        ZEC_DECIMALS,
    )
}

/// The accounts needed to pay ZEC from a coin's vault to someone's standard
/// ZEC account.
pub struct PayTo<'a, 'info> {
    pub payer: &'a AccountInfo<'info>,
    /// Who is being paid.
    pub owner: &'a AccountInfo<'info>,
    /// Their standard ZEC account. Checked here by address.
    pub destination: &'a AccountInfo<'info>,
    pub vault: &'a AccountInfo<'info>,
    pub zec_mint: &'a AccountInfo<'info>,
    pub authority: &'a AccountInfo<'info>,
    pub token_program: &'a AccountInfo<'info>,
    pub associated_token_program: &'a AccountInfo<'info>,
    pub system_program: &'a AccountInfo<'info>,
}

/// Pays `amount` to the standard ZEC account of `owner`, creating it if it
/// does not exist.
///
/// The destination is fixed by its address alone: nobody chooses another.
/// Nothing about the account's present state can block the payment. In
/// particular the receiver cannot make themselves unpayable by handing the
/// account to someone else: the ZEC still goes to that same address.
pub fn pay_to_standard_account(p: &PayTo, mint: &Pubkey, authority_bump: u8, amount: u64) -> Result<()> {
    let expected = get_associated_token_address_with_program_id(p.owner.key, &ZEC_MINT, p.token_program.key);
    require_keys_eq!(*p.destination.key, expected, BurrpadError::WrongReceiver);
    if amount == 0 {
        return Ok(());
    }
    if p.destination.data_is_empty() {
        associated_token::create(CpiContext::new(
            *p.associated_token_program.key,
            associated_token::Create {
                payer: p.payer.clone(),
                associated_token: p.destination.clone(),
                authority: p.owner.clone(),
                mint: p.zec_mint.clone(),
                system_program: p.system_program.clone(),
                token_program: p.token_program.clone(),
            },
        ))?;
    }
    pay_from_vault(p.token_program, p.vault, p.zec_mint, p.destination, p.authority, mint, authority_bump, amount)
}
