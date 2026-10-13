//! Launch: creates the coin on pump.fun and writes its record, together or
//! not at all.

use crate::constants::*;
use crate::error::BurrError;
use crate::events::Launched;
use crate::math;
use crate::pump;
use crate::state::{Coin, Config, Ledger};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::{get_associated_token_address_with_program_id, AssociatedToken};
use anchor_spl::token::Token;
use anchor_spl::token_2022::Token2022;
use anchor_spl::token_interface::{Mint, TokenAccount};

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct LaunchArgs {
    pub name: String,
    pub symbol: String,
    pub uri: String,
    /// The coin's fee, in hundredths of a percent.
    pub fee_bps: u16,
    /// Shares in order: creator, give, burn, vault share, king. They add to 10,000.
    pub shares: [u16; 5],
    pub creator_receiver: Pubkey,
    pub give_receiver: Pubkey,
    pub king_round_secs: u32,
}

#[derive(Accounts)]
pub struct Launch<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    /// The new coin's mint. A fresh key that signs once; pump.fun creates the
    /// account inside this instruction and it is checked afterwards.
    #[account(mut)]
    pub mint: Signer<'info>,
    #[account(seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(
        init,
        payer = payer,
        space = 8 + Coin::INIT_SPACE,
        seeds = [SEED_COIN, mint.key().as_ref()],
        bump
    )]
    pub coin: Box<Account<'info, Coin>>,
    #[account(
        init,
        payer = payer,
        space = 8 + Ledger::INIT_SPACE,
        seeds = [SEED_LEDGER, mint.key().as_ref()],
        bump
    )]
    pub ledger: Box<Account<'info, Ledger>>,
    /// CHECK: the coin's authority, a derived address with nothing in it. It becomes the coin's creator on pump.fun.
    #[account(seeds = [SEED_AUTHORITY, mint.key().as_ref()], bump)]
    pub authority: UncheckedAccount<'info>,
    /// The coin's vault: the authority's own ZEC account, where pump.fun pays the fee.
    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = zec_mint,
        associated_token::authority = authority,
        associated_token::token_program = token_program
    )]
    pub vault: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(address = ZEC_MINT)]
    pub zec_mint: Box<InterfaceAccount<'info, Mint>>,

    // pump.fun's accounts, each pinned to its derived address.
    /// CHECK: derived address of pump.fun
    #[account(seeds = [pump::SEED_MINT_AUTHORITY], bump, seeds::program = pump::PUMP_ID)]
    pub pump_mint_authority: UncheckedAccount<'info>,
    /// CHECK: derived address of pump.fun; read and checked after the call
    #[account(mut, seeds = [pump::SEED_BONDING_CURVE, mint.key().as_ref()], bump, seeds::program = pump::PUMP_ID)]
    pub bonding_curve: UncheckedAccount<'info>,
    /// CHECK: the curve's coin account
    #[account(mut, address = get_associated_token_address_with_program_id(&bonding_curve.key(), &mint.key(), &token_2022_program.key()))]
    pub associated_bonding_curve: UncheckedAccount<'info>,
    /// CHECK: derived address of pump.fun
    #[account(seeds = [pump::SEED_GLOBAL], bump, seeds::program = pump::PUMP_ID)]
    pub pump_global: UncheckedAccount<'info>,
    /// CHECK: pinned address
    #[account(mut, address = pump::MAYHEM_ID)]
    pub mayhem_program: UncheckedAccount<'info>,
    /// CHECK: derived address of pump.fun's mayhem program
    #[account(seeds = [pump::SEED_MAYHEM_GLOBAL_PARAMS], bump, seeds::program = pump::MAYHEM_ID)]
    pub mayhem_global_params: UncheckedAccount<'info>,
    /// CHECK: derived address of pump.fun's mayhem program
    #[account(mut, seeds = [pump::SEED_MAYHEM_SOL_VAULT], bump, seeds::program = pump::MAYHEM_ID)]
    pub mayhem_sol_vault: UncheckedAccount<'info>,
    /// CHECK: derived address of pump.fun's mayhem program
    #[account(mut, seeds = [pump::SEED_MAYHEM_STATE, mint.key().as_ref()], bump, seeds::program = pump::MAYHEM_ID)]
    pub mayhem_state: UncheckedAccount<'info>,
    /// CHECK: the mayhem vault's coin account
    #[account(mut, address = get_associated_token_address_with_program_id(&mayhem_sol_vault.key(), &mint.key(), &token_2022_program.key()))]
    pub mayhem_token_vault: UncheckedAccount<'info>,
    /// CHECK: derived address of pump.fun
    #[account(seeds = [pump::SEED_EVENT_AUTHORITY], bump, seeds::program = pump::PUMP_ID)]
    pub pump_event_authority: UncheckedAccount<'info>,
    /// CHECK: the curve's ZEC account
    #[account(mut, address = get_associated_token_address_with_program_id(&bonding_curve.key(), &ZEC_MINT, &token_program.key()))]
    pub associated_quote_bonding_curve: UncheckedAccount<'info>,
    /// CHECK: derived address of pump.fun: its list of quote coins
    #[account(seeds = [pump::SEED_QUOTE_CONTROL], bump, seeds::program = pump::PUMP_ID)]
    pub quote_control: UncheckedAccount<'info>,
    /// CHECK: pinned address
    #[account(address = pump::PUMP_ID)]
    pub pump_program: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub token_2022_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handle_launch(ctx: Context<Launch>, args: LaunchArgs) -> Result<()> {
    let a = &ctx.accounts;
    let cfg = &a.config;

    // 1. Checks before anything is created.
    require!(!cfg.launches_paused, BurrError::LaunchesPaused);
    require!(args.fee_bps >= cfg.min_fee_bps && args.fee_bps <= cfg.max_fee_bps, BurrError::FeeOutOfRange);
    require!(math::weights(cfg.platform_fraction_bps, &args.shares).is_some(), BurrError::SharesDoNotAddUp);
    require!(
        args.shares[SINK_CREATOR - 1] == 0 || args.creator_receiver != Pubkey::default(),
        BurrError::MissingReceiver
    );
    require!(args.shares[SINK_GIVE - 1] == 0 || args.give_receiver != Pubkey::default(), BurrError::MissingReceiver);
    require!(
        args.shares[SINK_KING - 1] == 0 || (KING_MIN_ROUND_SECS..=KING_MAX_ROUND_SECS).contains(&args.king_round_secs),
        BurrError::RoundLengthOutOfRange
    );
    require!(
        args.name.len() <= MAX_NAME_LEN && args.symbol.len() <= MAX_SYMBOL_LEN && args.uri.len() <= MAX_URI_LEN,
        BurrError::TextTooLong
    );

    // 2. Create the coin on pump.fun, with our authority as its creator.
    let mint_key = a.mint.key();
    let authority_key = a.authority.key();
    pump::create_v2(
        pump::CreateV2 {
            mint: &a.mint.to_account_info(),
            mint_authority: &a.pump_mint_authority.to_account_info(),
            bonding_curve: &a.bonding_curve.to_account_info(),
            associated_bonding_curve: &a.associated_bonding_curve.to_account_info(),
            global: &a.pump_global.to_account_info(),
            user: &a.payer.to_account_info(),
            system_program: &a.system_program.to_account_info(),
            token_2022_program: &a.token_2022_program.to_account_info(),
            associated_token_program: &a.associated_token_program.to_account_info(),
            mayhem_program: &a.mayhem_program.to_account_info(),
            global_params: &a.mayhem_global_params.to_account_info(),
            sol_vault: &a.mayhem_sol_vault.to_account_info(),
            mayhem_state: &a.mayhem_state.to_account_info(),
            mayhem_token_vault: &a.mayhem_token_vault.to_account_info(),
            event_authority: &a.pump_event_authority.to_account_info(),
            pump_program: &a.pump_program.to_account_info(),
            quote_mint: &a.zec_mint.to_account_info(),
            associated_quote_bonding_curve: &a.associated_quote_bonding_curve.to_account_info(),
            quote_token_program: &a.token_program.to_account_info(),
            quote_control: &a.quote_control.to_account_info(),
        },
        &args.name,
        &args.symbol,
        &args.uri,
        &authority_key,
        args.fee_bps,
    )?;

    // 3. Trust nothing from the call having succeeded: read what was made.
    let curve = pump::read_curve(&a.bonding_curve.to_account_info())?;
    require!(
        curve.creator == authority_key
            && curve.quote_mint == ZEC_MINT
            && curve.creator_fee_bps == args.fee_bps as u64
            && !curve.is_holder_reward
            && !curve.is_cashback_coin
            && !curve.is_mayhem_mode
            && !curve.complete,
        BurrError::CurveMismatch
    );
    pump::check_plain_coin_mint(&a.mint.to_account_info(), &a.token_2022_program.key())?;

    // 4. Write the record. Nothing writes to it again.
    let now = Clock::get()?.unix_timestamp;
    let platform_fraction_bps = cfg.platform_fraction_bps;
    let platform_receiver = cfg.platform_receiver;
    let (_, burner_bump) = Pubkey::find_program_address(&[SEED_BURNER, mint_key.as_ref()], ctx.program_id);
    let coin = &mut ctx.accounts.coin;
    coin.mint = mint_key;
    coin.bump = ctx.bumps.coin;
    coin.authority_bump = ctx.bumps.authority;
    coin.burner_bump = burner_bump;
    coin.ledger_bump = ctx.bumps.ledger;
    coin.fee_bps = args.fee_bps;
    coin.platform_fraction_bps = platform_fraction_bps;
    coin.platform_receiver = platform_receiver;
    coin.shares = args.shares;
    coin.creator_receiver = args.creator_receiver;
    coin.give_receiver = args.give_receiver;
    coin.king_round_secs = args.king_round_secs;
    coin.burn_cooldown_secs = BURN_COOLDOWN_SECS;
    coin.burn_price_band_bps = BURN_PRICE_BAND_BPS;
    coin.burn_fee_allowance_bps = BURN_FEE_ALLOWANCE_BPS;
    coin.burn_spend_divisor = BURN_SPEND_DIVISOR;
    coin.vault_release_secs = VAULT_RELEASE_SECS;
    coin.launched_at = now;
    coin.launcher = ctx.accounts.payer.key();

    let ledger = &mut ctx.accounts.ledger;
    ledger.mint = mint_key;
    ledger.bump = ctx.bumps.ledger;
    ledger.vault_share.last_update = now;

    emit!(Launched {
        mint: mint_key,
        launcher: coin.launcher,
        fee_bps: coin.fee_bps,
        platform_fraction_bps: coin.platform_fraction_bps,
        shares: coin.shares,
    });
    Ok(())
}
