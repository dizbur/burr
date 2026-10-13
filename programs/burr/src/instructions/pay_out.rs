//! Pays the platform's, the creator's or the give pot, whole, to its fixed
//! receiver. Open to anyone: the receiver claiming is the receiver calling it.

use crate::constants::*;
use crate::error::BurrError;
use crate::events::PaidOut;
use crate::ops;
use crate::state::{Coin, Ledger, Payee};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::Token;
use anchor_spl::token_interface::{Mint, TokenAccount};

#[derive(Accounts)]
#[instruction(payee: Payee)]
pub struct PayOut<'info> {
    /// Pays the small rent if the receiver has no ZEC account yet.
    #[account(mut)]
    pub payer: Signer<'info>,
    /// CHECK: only its address is used, to derive the coin's accounts
    pub mint: UncheckedAccount<'info>,
    #[account(seeds = [SEED_COIN, mint.key().as_ref()], bump = coin.bump)]
    pub coin: Box<Account<'info, Coin>>,
    #[account(mut, seeds = [SEED_LEDGER, mint.key().as_ref()], bump = ledger.bump)]
    pub ledger: Box<Account<'info, Ledger>>,
    /// CHECK: the coin's authority, a derived address
    #[account(seeds = [SEED_AUTHORITY, mint.key().as_ref()], bump = coin.authority_bump)]
    pub authority: UncheckedAccount<'info>,
    #[account(
        mut,
        associated_token::mint = zec_mint,
        associated_token::authority = authority,
        associated_token::token_program = token_program
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(address = ZEC_MINT)]
    pub zec_mint: Box<InterfaceAccount<'info, Mint>>,
    /// CHECK: must be the receiver written in the coin's record for this pot
    #[account(address = payee.receiver(&coin) @ BurrError::WrongReceiver)]
    pub receiver: UncheckedAccount<'info>,
    /// The receiver's standard ZEC account. Nobody chooses another.
    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = zec_mint,
        associated_token::authority = receiver,
        associated_token::token_program = token_program
    )]
    pub receiver_zec: Box<InterfaceAccount<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handle_pay_out(ctx: Context<PayOut>, payee: Payee) -> Result<()> {
    let sink = payee.sink();
    let amount = ctx.accounts.ledger.pots[sink];
    // The pot is emptied in the ledger before the ZEC moves.
    ops::debit(&mut ctx.accounts.ledger, sink, amount)?;
    let a = &ctx.accounts;
    ops::pay_from_vault(
        &a.token_program.to_account_info(),
        &a.vault.to_account_info(),
        &a.zec_mint.to_account_info(),
        &a.receiver_zec.to_account_info(),
        &a.authority.to_account_info(),
        &a.coin.mint,
        a.coin.authority_bump,
        amount,
    )?;
    emit!(PaidOut { mint: a.coin.mint, pot: sink as u8, receiver: a.receiver.key(), amount });
    Ok(())
}
