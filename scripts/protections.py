#!/usr/bin/env python3
"""Switch off each protection in turn and prove that a test notices.

For every protection in the program this script makes a throwaway copy of the
code with that one protection removed, builds it, and runs the whole test
suite. A protection counts as held only if the test named for it fails, with
the message expected. The real code is never touched.

    python3 scripts/protections.py --check      # only check the list applies cleanly
    python3 scripts/protections.py              # run everything (about an hour)
    python3 scripts/protections.py --only B7,K3 # run some

Results go to notes/protections.json and docs/protections.md (neither is committed).
"""

import argparse
import json
import multiprocessing as mp
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = "programs/burr/src"
WORK = ROOT / "tmp" / "mut"

# ---------------------------------------------------------------------------
# Protections written by hand: checks inside the instructions.
# Each: id, what it protects, file, text to find, what to put instead,
# the test that must fail, and (optionally) text its failure must contain.
# ---------------------------------------------------------------------------

def M(id, what, file, find, repl, test, msg=None, nth=0):
    return dict(id=id, what=what, file=f"{SRC}/{file}", find=find, repl=repl, test=test, msg=msg, nth=nth)

ADMIN = "instructions/admin.rs"
LAUNCH = "instructions/launch.rs"
COLLECT = "instructions/collect.rs"
PAY = "instructions/pay_out.rs"
BURN = "instructions/burn.rs"
VS = "instructions/vault_share.rs"
KING = "instructions/king.rs"

MANUAL = [
    # ---- the administrator ----
    M("A1", "Only the program's upgrade key can initialise", ADMIN,
      "constraint = program_data.upgrade_authority_address == Some(upgrade_authority.key()) @",
      "constraint = true @", "only_the_upgrade_key_can_initialise"),
    M("A2", "The upgrade record passed must be this program's own", ADMIN,
      "constraint = program.programdata_address()? == Some(program_data.key()) @",
      "constraint = true @", "initialise_cannot_be_passed_another_programs_upgrade_record"),
    M("A3", "The administrator cannot be the empty address", ADMIN,
      "admin != Pubkey::default() && platform_receiver != Pubkey::default()",
      "platform_receiver != Pubkey::default()", "initialise_refuses_empty_addresses", "empty administrator"),
    M("A4", "The platform receiver cannot be the empty address, at initialisation", ADMIN,
      "admin != Pubkey::default() && platform_receiver != Pubkey::default()",
      "admin != Pubkey::default()", "initialise_refuses_empty_addresses", "empty platform receiver"),
    M("A5", "Only the administrator can change settings", ADMIN,
      "constraint = config.admin == admin.key() @", "constraint = true @", "a_stranger_cannot_use_any_admin_instruction"),
    M("A6", "The administrator must sign", ADMIN,
      "pub struct AdminOnly<'info> {\n    pub admin: Signer<'info>,",
      "pub struct AdminOnly<'info> {\n    /// CHECK: switched off\n    pub admin: UncheckedAccount<'info>,",
      "naming_a_key_is_not_signing_with_it", "administrator's address"),
    M("A7", "The platform fraction can never be set above half", ADMIN,
      "require!(fraction_bps <= HARD_MAX_PLATFORM_FRACTION_BPS, BurrError::PlatformFractionTooHigh);", "",
      "admin_settings_stay_inside_the_hard_limits"),
    M("A8", "The lowest fee setting can never go below 0.10%", ADMIN,
      "min_bps >= HARD_MIN_FEE_BPS", "true", "admin_settings_stay_inside_the_hard_limits", "9 300 200"),
    M("A9", "The highest fee setting can never go above 5.00%", ADMIN,
      "max_bps <= HARD_MAX_FEE_BPS", "true", "admin_settings_stay_inside_the_hard_limits", "50 501 200"),
    M("A10", "The first-offered fee cannot be below the lowest", ADMIN,
      "min_bps <= default_bps", "true", "admin_settings_stay_inside_the_hard_limits", "50 300 49"),
    M("A11", "The first-offered fee cannot be above the highest", ADMIN,
      "default_bps <= max_bps", "true", "admin_settings_stay_inside_the_hard_limits", "50 300 301"),
    M("A12", "The platform receiver cannot be set to the empty address", ADMIN,
      "require!(receiver != Pubkey::default(), BurrError::EmptyAddress);", "",
      "admin_settings_stay_inside_the_hard_limits"),
    M("A13", "Only the proposed administrator can accept the role", ADMIN,
      "constraint = config.pending_admin == new_admin.key() @", "constraint = true @",
      "the_admin_role_moves_in_two_steps"),
    M("A14", "The proposed administrator must sign", ADMIN,
      "pub new_admin: Signer<'info>,", "/// CHECK: switched off\n    pub new_admin: UncheckedAccount<'info>,",
      "naming_a_key_is_not_signing_with_it", "proposed administrator"),

    # ---- launch ----
    M("L1", "No launch while launches are switched off", LAUNCH,
      "require!(!cfg.launches_paused, BurrError::LaunchesPaused);", "", "pausing_stops_new_launches_only"),
    M("L2", "A coin's fee cannot be below the lowest allowed", LAUNCH,
      "args.fee_bps >= cfg.min_fee_bps && ", "", "launch_refuses_bad_plans"),
    M("L3", "A coin's fee cannot be above the highest allowed", LAUNCH,
      " && args.fee_bps <= cfg.max_fee_bps", "", "launch_refuses_bad_plans"),
    M("L4", "Hook shares must add up to exactly the whole", LAUNCH,
      "require!(math::weights(cfg.platform_fraction_bps, &args.shares).is_some(), BurrError::SharesDoNotAddUp);", "",
      "launch_refuses_bad_plans"),
    M("L5", "A chosen creator's share needs a receiver", LAUNCH,
      "args.shares[SINK_CREATOR - 1] == 0 || args.creator_receiver != Pubkey::default()", "true",
      "launch_refuses_bad_plans"),
    M("L6", "A chosen give hook needs a receiver", LAUNCH,
      "args.shares[SINK_GIVE - 1] == 0 || args.give_receiver != Pubkey::default()", "true",
      "launch_refuses_bad_plans"),
    M("L7", "A king round cannot be shorter than 5 minutes", LAUNCH,
      "(KING_MIN_ROUND_SECS..=KING_MAX_ROUND_SECS)", "(0..=KING_MAX_ROUND_SECS)", "launch_refuses_bad_plans"),
    M("L8", "A king round cannot be longer than 7 days", LAUNCH,
      "(KING_MIN_ROUND_SECS..=KING_MAX_ROUND_SECS)", "(KING_MIN_ROUND_SECS..=u32::MAX)", "launch_refuses_bad_plans"),
    M("L9", "Name, ticker and link lengths are checked before calling pump.fun", LAUNCH,
      "args.name.len() <= MAX_NAME_LEN && ", "", "launch_refuses_bad_plans"),
    M("L10", "After the call: the coin's creator is our authority", LAUNCH,
      "curve.creator == authority_key", "true", "a_launch_is_refused_if_the_coin_made_is_not", "another creator"),
    M("L11", "After the call: the coin is priced in ZEC", LAUNCH,
      "&& curve.quote_mint == ZEC_MINT\n            && curve.creator_fee_bps", "&& curve.creator_fee_bps",
      "a_launch_is_refused_if_the_coin_made_is_not", "another quote coin"),
    M("L12", "After the call: the coin's fee is the one asked for", LAUNCH,
      "&& curve.creator_fee_bps == args.fee_bps as u64", "", "a_launch_is_refused_if_the_coin_made_is_not",
      "another fee rate"),
    M("L13", "After the call: the coin is not a holder-rewards coin", LAUNCH,
      "&& !curve.is_holder_reward", "", "a_launch_is_refused_if_the_coin_made_is_not", "holder-rewards"),
    M("L14", "After the call: the coin is not a cashback coin", LAUNCH,
      "&& !curve.is_cashback_coin", "", "a_launch_is_refused_if_the_coin_made_is_not", "cashback"),
    M("L15", "After the call: the coin is not a mayhem coin", LAUNCH,
      "&& !curve.is_mayhem_mode", "", "a_launch_is_refused_if_the_coin_made_is_not", "mayhem"),
    M("L16", "After the call: the curve is not already complete", LAUNCH,
      "&& !curve.complete,", ",", "a_launch_is_refused_if_the_coin_made_is_not", "already complete"),
    M("L17", "After the call: the coin's mint is checked to be the plain kind", LAUNCH,
      "pump::check_plain_coin_mint(&a.mint.to_account_info(), &a.token_2022_program.key())?;", "",
      "a_launch_is_refused_if_the_coin_made_is_not", "no proper mint"),
    M("L18", "The launch price is recorded as the first reference for buy back and burn", LAUNCH,
      "ledger.burn.reference = math::Reserves { quote: curve.virtual_quote_reserves, base: curve.virtual_token_reserves };",
      "ledger.burn.reference = math::Reserves { quote: u64::MAX / 2, base: 1 };",
      "the_first_burn_is_checked_against_the_launch_price"),
    M("L19", "The first burn cannot come in the first minute", LAUNCH,
      "ledger.burn.last_time = now;", "", "the_first_burn_is_checked_against_the_launch_price"),

    M("L20", "A launch stops if the platform's fraction is above what the launcher agreed", LAUNCH,
      "cfg.platform_fraction_bps <= args.max_platform_fraction_bps && ", "",
      "a_launch_is_stopped_if_the_platforms_terms_changed_on_the_way", "a higher fraction"),
    M("L21", "A launch stops if the platform's receiver is not the one the launcher expected", LAUNCH,
      " && cfg.platform_receiver == args.platform_receiver", "",
      "a_launch_is_stopped_if_the_platforms_terms_changed_on_the_way", "another receiver"),
    M("L22", "A receiver cannot be the coin's own authority", LAUNCH,
      "r != authority_key && r != burner_key", "r != burner_key", "a_coins_own_addresses_cannot_be_named_as_receivers"),
    M("L23", "A receiver cannot be the coin's own burner", LAUNCH,
      "r != authority_key && r != burner_key", "r != authority_key", "a_coins_own_addresses_cannot_be_named_as_receivers"),

    # ---- the coin's mint ----
    M("N1", "A coin's mint must belong to Token-2022", "pump.rs",
      "require_keys_eq!(*mint.owner, *token_2022, BurrError::UnexpectedMint);", "",
      "a_mint_that_is_not_the_plain_kind_is_refused"),
    M("N2", "Nobody may be able to mint more of a coin", "pump.rs",
      "!mint_authority_set && !freeze_authority_set", "!freeze_authority_set",
      "a_mint_that_is_not_the_plain_kind_is_refused"),
    M("N3", "Nobody may be able to freeze a coin", "pump.rs",
      "!mint_authority_set && !freeze_authority_set", "!mint_authority_set",
      "a_mint_that_is_not_the_plain_kind_is_refused"),
    M("N4", "A coin has 6 decimals", "pump.rs", "&& d[44] == COIN_DECIMALS", "",
      "a_mint_that_is_not_the_plain_kind_is_refused"),
    M("N5", "A coin's mint must be initialised", "pump.rs", "&& d[45] == 1,", ",",
      "a_mint_that_is_not_the_plain_kind_is_refused"),
    M("N6", "The account must be a mint, not another Token-2022 account", "pump.rs",
      "require!(d.len() > 166 && d[165] == 1, BurrError::UnexpectedMint);",
      "require!(d.len() > 166, BurrError::UnexpectedMint);", "a_mint_that_is_not_the_plain_kind_is_refused"),
    M("N7", "A coin's mint may carry no extension but its metadata", "pump.rs",
      "require!(kind == EXT_METADATA_POINTER || kind == EXT_TOKEN_METADATA, BurrError::UnexpectedMint);", "",
      "a_mint_that_is_not_the_plain_kind_is_refused", "was accepted"),

    # ---- reading pump.fun's accounts ----
    M("R1", "A curve account must be owned by pump.fun's program", "pump.rs",
      "require_keys_eq!(*ai.owner, PUMP_ID, BurrError::WrongPumpAccount);", "",
      "the_curve_reader_checks_owner_code_and_length"),
    M("R2", "A curve account must be long enough and carry the curve's code", "pump.rs",
      "require!(d.len() >= CURVE_MIN_LEN && d[..8] == ACC_BONDING_CURVE, BurrError::UnreadablePumpAccount);",
      "require!(d.len() >= CURVE_MIN_LEN, BurrError::UnreadablePumpAccount);",
      "the_curve_reader_checks_owner_code_and_length"),
    M("R3", "A pool account must be owned by PumpSwap", "pump.rs",
      "require_keys_eq!(*ai.owner, PUMP_AMM_ID, BurrError::WrongPumpAccount);", "",
      "the_pool_reader_checks_owner_code_and_length"),
    M("R4", "A pool account must be long enough and carry the pool's code", "pump.rs",
      "require!(d.len() >= POOL_MIN_LEN && d[..8] == ACC_POOL, BurrError::UnreadablePumpAccount);",
      "require!(d.len() >= POOL_MIN_LEN, BurrError::UnreadablePumpAccount);",
      "the_pool_reader_checks_owner_code_and_length"),
    M("R5", "A pool's price reserve must be positive", "pump.rs",
      "require!(v > 0, BurrError::UnreadablePumpAccount);", "",
      "the_pools_price_reserve_must_be_positive_and_fit"),
    M("R6", "The settings account must be owned by pump.fun's program", "pump.rs",
      "require_keys_eq!(*settings.owner, *settings_owner, BurrError::WrongPumpAccount);", "",
      "only_a_listed_buy_back_receivers_zec_account_is_accepted"),
    M("R7", "A buy-back receiver's account must belong to the classic token program", "pump.rs",
      "require_keys_eq!(*token_account.owner, *token_program, BurrError::WrongPumpAccount);", "",
      "only_a_listed_buy_back_receivers_zec_account_is_accepted"),
    M("R8", "A buy-back receiver's account must be a ZEC account", "pump.rs",
      "require_keys_eq!(key_at(&t, 0), *zec_mint, BurrError::WrongPumpAccount);", "",
      "only_a_listed_buy_back_receivers_zec_account_is_accepted"),
    M("R9", "A buy-back receiver must be on pump.fun's own list", "pump.rs",
      "require!(listed && holder != Pubkey::default(), BurrError::WrongPumpAccount);",
      "require!(holder != Pubkey::default(), BurrError::WrongPumpAccount);",
      "only_a_listed_buy_back_receivers_zec_account_is_accepted"),
    M("R10", "The empty address is never a buy-back receiver", "pump.rs",
      "require!(listed && holder != Pubkey::default(), BurrError::WrongPumpAccount);",
      "require!(listed, BurrError::WrongPumpAccount);",
      "only_a_listed_buy_back_receivers_zec_account_is_accepted"),
    M("R11", "The settings account must carry the settings' code and be long enough", "pump.rs",
      "s.len() >= list_offset + 32 * BUYBACK_COUNT && s[..8] == *settings_code", "s.len() >= list_offset + 32 * BUYBACK_COUNT",
      "only_a_listed_buy_back_receivers_zec_account_is_accepted"),
    M("R12", "Calls go only to the exact pump.fun program addresses", "pump.rs",
      "require_keys_eq!(*program.key, *expected, BurrError::WrongPumpAccount);", "",
      "t11_substitution", "swapping (pump_program|amm_program)"),

    # ---- collect and split ----
    M("C1", "The vault is read again after pump.fun's collect, on the curve side", COLLECT,
      "ctx.accounts.vault.reload()?;", "", "collect_brings_the_fee_in_and_splits_it_exactly", nth=0),
    M("C2", "The vault is read again after pump.fun's collect, on the pool side", COLLECT,
      "ctx.accounts.vault.reload()?;", "", "fees_from_both_the_curve_and_the_pool_reach_the_vault", nth=1),
    M("C3", "Collecting from a pool that does not exist yet is refused clearly", COLLECT,
      "require!(!a.pool.data_is_empty(), BurrError::WrongVenue);", "", "collect_pool_before_graduation_is_refused"),
    M("C4", "A vault holding less than the ledger says stops the split", "ops.rs",
      "vault_amount.checked_sub(ledger.accounted).ok_or(BurrError::VaultShort)?", "vault_amount.saturating_sub(ledger.accounted)",
      "a_vault_holding_less_than_the_ledger_says_stops_the_split"),
    M("C5", "Dues are worked out on the running total, so call pattern cannot tilt the split", "math.rs",
      "let total = total_split.checked_add(new as u128)?;",
      "let total = new as u128;\n    *credited = [0u128; SINKS];",
      "splitting_in_many_small_calls_gives_the_same_as_one"),
    M("C6", "A pot is credited only the difference between its due and what it already had", "math.rs",
      "let add = due.checked_sub(credited[i])?;", "let add = due;", "the_split_does_not_depend_on_how_it_is_called"),
    M("C7", "A pot cannot be debited below zero", "ops.rs",
      "ledger.pots[sink] = ledger.pots[sink].checked_sub(amount).ok_or(BurrError::Overflow)?;",
      "ledger.pots[sink] = ledger.pots[sink].wrapping_sub(amount);", "a_pot_cannot_be_debited_below_zero",
      "more than the pot"),
    M("C8", "The ledger's total cannot be debited below zero", "ops.rs",
      "ledger.accounted = ledger.accounted.checked_sub(amount).ok_or(BurrError::Overflow)?;",
      "ledger.accounted = ledger.accounted.wrapping_sub(amount);", "a_pot_cannot_be_debited_below_zero",
      "more than the ledger"),
    M("C9", "The vault share's credit is queued for release, not paid at once", "ops.rs",
      "ledger.vault_share.credit(credits[SINK_VAULT_SHARE], now, coin.vault_release_secs).ok_or(BurrError::Overflow)?;",
      "", "stakers_share_by_amount"),

    # ---- payouts ----
    M("P1", "A payout pot is emptied in the ledger, so a second call pays nothing", PAY,
      "ops::debit(&mut ctx.accounts.ledger, sink, amount)?;", "", "each_payout_pot_goes_whole_to_its_fixed_receiver"),

    M("P2", "A payout goes only to the receiver's standard ZEC account, fixed by its address", "ops.rs",
      "require_keys_eq!(*p.destination.key, expected, BurrError::WrongReceiver);", "",
      "t11_substitution", "swapping (receiver_zec|winner_zec|owner_zec)"),
    M("P3", "An empty pot pays nothing and makes no account", "ops.rs",
      "    if amount == 0 {\n        return Ok(());\n    }\n    if p.destination.data_is_empty()", "    if p.destination.data_is_empty()",
      "an_empty_pot_pays_nothing_and_makes_no_account"),

    # ---- buy back and burn ----
    M("B1", "A coin without the burn hook cannot burn", BURN,
      "require!(coin.share_of(SINK_BURN) > 0, BurrError::HookNotChosen);", "", "a_coin_without_the_hook_cannot_burn"),
    M("B2", "Burns wait between calls", BURN,
      "require!(now >= ready, BurrError::BurnTooSoon);", "", "burns_wait_between_calls"),
    M("B3", "A price well above the last recorded one is not bought at", BURN,
      "if reserves.above(&b.reference, coin.burn_price_band_bps).ok_or(BurrError::Overflow)? {",
      "if false && reserves.above(&b.reference, coin.burn_price_band_bps).ok_or(BurrError::Overflow)? {",
      "a_price_pushed_up_is_not_bought_at"),
    M("B4", "When the price check refuses, the reference rises by one step, not to the price seen", BURN,
      "b.reference = b.reference.raised_by(coin.burn_price_band_bps).ok_or(BurrError::Overflow)?;",
      "b.reference = reserves;", "pushing_the_price_for_one_call_does_not_move_the_reference_to_it"),
    M("B25", "When the price check refuses, the time is recorded, so it cannot be retried at once", BURN,
      "        b.last_time = now;\n        emit!(BurnSkipped", "        emit!(BurnSkipped", "a_price_pushed_up_is_not_bought_at"),
    M("B5", "The spend limit follows the fee pump.fun really charges", BURN,
      "coin.fee_bps.min(charged),", "coin.fee_bps,", "the_limit_follows_the_fee_pump_fun_really_charges"),
    M("B6", "The spend limit never exceeds the coin's recorded fee", BURN,
      "coin.fee_bps.min(charged),", "charged,", "the_limit_follows_the_fee_pump_fun_really_charges"),
    M("B7", "A burn never spends more than the limit measured against the pool", "math.rs",
      "Some(pot.min(u64::try_from(cap).ok()?))", "let _ = cap;\n    Some(pot)", "a_burn_buys_the_coin_and_destroys_it"),
    M("B8", "A burn never spends more than the pot", "math.rs",
      "Some(pot.min(u64::try_from(cap).ok()?))", "let _ = pot;\n    Some(u64::try_from(cap).ok()?)",
      "the_spend_never_passes_the_pot_or_the_limit"),
    M("B9", "With an empty pot nothing is recorded and nothing is spent", BURN,
      "require!(ledger.pots[SINK_BURN] > 0, BurrError::NothingToBurn);", "",
      "an_empty_pot_cannot_be_used_to_move_the_reference"),
    M("B23", "The limit is measured against the smaller of the reserve now and the reserve last recorded", BURN,
      "reserves.quote.min(b.reference.quote),", "reserves.quote,",
      "liquidity_added_just_for_one_call_does_not_raise_the_limit"),
    M("B24", "If nothing can be bought (no fee charged, or too little), the burn stops", BURN,
      "require!(min_out > 0, BurrError::NothingToBurn);", "", "the_limit_follows_the_fee_pump_fun_really_charges"),
    M("B10", "The curve burn works only while the curve is trading", BURN,
      "!curve.complete && curve.quote_mint == ZEC_MINT", "curve.quote_mint == ZEC_MINT",
      "each_burn_instruction_only_works_on_its_own_venue"),
    M("B11", "The buy-back receiver is checked before the curve buy", BURN,
      "pump::check_buyback_recipient(\n        &a.pump_global.to_account_info(),\n        &pump::PUMP_ID,\n        &pump::ACC_GLOBAL,\n        pump::GLOBAL_BUYBACK_OFFSET,\n        &a.buyback_fee_recipient.to_account_info(),\n        &ZEC_MINT,\n        &a.token_program.key(),\n    )?;",
      "", "t11_substitution::burn_curve", "swapping buyback_fee_recipient"),
    M("B12", "The buy-back receiver is checked before the pool buy", BURN,
      "pump::check_buyback_recipient(\n        &a.amm_global_config.to_account_info(),\n        &pump::PUMP_AMM_ID,\n        &pump::ACC_AMM_GLOBAL_CONFIG,\n        pump::AMM_GLOBAL_BUYBACK_OFFSET,\n        &a.buyback_fee_recipient.to_account_info(),\n        &ZEC_MINT,\n        &a.token_program.key(),\n    )?;",
      "", "t11_substitution::burn_pool", "swapping buyback_fee_recipient"),
    M("B13", "The buy may not take more ZEC than was set aside", BURN,
      "require!(spent <= spend, BurrError::BurnOverspent);", "", "a_buy_cannot_take_more_than_was_set_aside"),
    M("B14", "The buy must deliver at least the least-coins-out, checked from balances", BURN,
      "require!(bought >= min_out, BurrError::BurnBadFill);", "", "a_burn_is_undone_if_the_buy_did_not_deliver"),
    M("B15", "What was spent is taken off the burn pot", BURN,
      "ops::debit(ledger, SINK_BURN, spent)?;", "", "a_burn_buys_the_coin_and_destroys_it"),
    M("B16", "Every coin bought is burned", BURN,
      "if coins > 0 {", "if false && coins > 0 {", "a_burn_buys_the_coin_and_destroys_it"),
    M("B17", "ZEC left with the burner goes back to the vault", BURN,
      "if zec_left > 0 {", "if false && zec_left > 0 {", "gifts_to_the_burner_do_not_confuse_it"),
    M("B18", "After a curve burn the new price becomes the reference", BURN,
      "    b.reference = Reserves { quote: after.virtual_quote_reserves, base: after.virtual_token_reserves };\n    b.last_time = now;",
      "    let _ = after;\n    b.last_time = now;", "a_burn_buys_the_coin_and_destroys_it"),
    M("B19", "After a burn the time is recorded", BURN,
      "    b.reference = Reserves { quote: after.virtual_quote_reserves, base: after.virtual_token_reserves };\n    b.last_time = now;",
      "    b.reference = Reserves { quote: after.virtual_quote_reserves, base: after.virtual_token_reserves };",
      "burns_wait_between_calls"),
    M("B22", "After a pool burn the new price becomes the reference", BURN,
      "    b.reference = after;\n", "", "a_burn_on_the_pool_buys_and_destroys"),
    M("B20", "A pool's price uses its ZEC balance plus the pool's own adjustment", BURN,
      "let quote = pump::effective_quote_reserves(quote_amount, p.virtual_quote_reserves)?;",
      "let quote = quote_amount;", "a_burn_on_the_pool_buys_and_destroys"),
    M("B21", "The least-coins-out is worked out from the pool's own figures", "math.rs",
      "u64::try_from(ideal.checked_mul(keep)? / BPS as u128).ok()", "let _ = (ideal, keep);\n    Some(1)",
      "a_burn_is_undone_if_the_buy_did_not_deliver", "a crumb"),

    # ---- vault share ----
    M("V1", "Staking nothing is refused", VS, "require!(amount > 0, BurrError::ZeroAmount);", "",
      "staking_needs_the_hook_and_a_real_amount"),
    M("V2", "A coin without the vault share hook takes no stakes", VS,
      "require!(ctx.accounts.coin.share_of(SINK_VAULT_SHARE) > 0, BurrError::HookNotChosen);", "",
      "staking_needs_the_hook_and_a_real_amount"),
    M("V3", "A stake is settled before it grows", VS,
      "    settle(&ctx.accounts.coin, &mut ctx.accounts.ledger, stake, now)?;\n", "",
      "stakers_share_by_time"),
    M("V4", "A stake is settled before it shrinks or is paid", VS,
      "    settle(&ctx.accounts.coin, &mut ctx.accounts.ledger, &mut ctx.accounts.stake, now)?;\n", "",
      "stakers_share_by_amount"),
    M("V5", "Nobody takes out more coins than they put in", VS,
      "    require!(stake.amount >= amount, BurrError::NotEnoughStaked);\n    stake.amount -= amount;",
      "    stake.amount = stake.amount.saturating_sub(amount);",
      "nobody_takes_out_more_than_they_put_in_or_anothers_coins"),
    M("V6", "The staker must sign to take coins out", VS,
      "pub struct UnstakeCoins<'info> {\n    #[account(mut)]\n    pub owner: Signer<'info>,",
      "pub struct UnstakeCoins<'info> {\n    /// CHECK: switched off\n    #[account(mut)]\n    pub owner: UncheckedAccount<'info>,",
      "nobody_takes_out_more_than_they_put_in_or_anothers_coins"),
    M("V7", "What is paid is taken off the vault share pot", VS,
      "ops::debit(ledger, SINK_VAULT_SHARE, pay)?;", "", "random_sequences_never_pay_out_more_than_came_in"),
    M("V8", "A paid reward is cleared, so it is not paid twice", VS,
      "    stake.owed = 0;\n", "", "random_sequences_never_pay_out_more_than_came_in"),
    M("V9", "When the last staker leaves, what remains is queued again", VS,
      "ledger.vault_share.queued = ledger.pots[SINK_VAULT_SHARE];", "",
      "when_the_last_staker_leaves_nothing_is_stranded"),
    M("V10", "Credited fee is released over time, not at once", "math.rs",
      "            let release = if now >= self.release_end {",
      "            let release = if true {", "a_short_visit_earns_only_for_its_time"),
    M("V11", "Nothing is released while nobody is staked", "math.rs",
      "        if self.total_staked == 0 {\n            if self.queued > 0 {\n                self.release_end = now.checked_add(release_secs as i64)?;\n            }\n            self.last_update = now;\n            return Some(());\n        }",
      "        if self.total_staked == 0 {\n            self.last_update = now;\n            return Some(());\n        }",
      "a_backlog_waits_for_the_first_staker_and_then_runs_its_course"),
    M("V12", "A snapshot is taken when a stake is settled, so nothing is earned twice", VS,
      "    stake.acc_snapshot = vs.acc;\n", "", "random_sequences_never_pay_out_more_than_came_in"),
    M("V13", "Rewards are rounded down", "math.rs",
      "let v = (staked as u128).checked_mul(growth)? / ACC_SCALE;",
      "let v = (staked as u128).checked_mul(growth)?.div_ceil(ACC_SCALE);",
      "the_vault_share_never_owes_more_than_it_was_given"),

    M("V14", "What the last staker leaves starts its release afresh", VS,
      "        ledger.vault_share.release_end =\n            now.checked_add(ctx.accounts.coin.vault_release_secs as i64).ok_or(BurrError::Overflow)?;\n", "",
      "what_the_last_staker_leaves_starts_its_release_afresh"),
    M("V15", "A stake record can be closed only when it is empty", VS,
      "require!(s.amount == 0 && s.owed == 0, BurrError::StakeNotEmpty);", "",
      "an_empty_stake_record_can_be_closed_by_its_owner_only"),

    # ---- king of the hill ----
    M("K1", "A coin without the king hook takes no king buys", KING,
      "require!(coin.share_of(SINK_KING) > 0, BurrError::HookNotChosen);", "",
      "king_buys_need_the_hook_and_cannot_use_another_coins_accounts"),
    M("K2", "A finished round must be settled before the next king buy", KING,
      "require!(k.leader == Pubkey::default(), BurrError::KingSettleFirst);", "",
      "a_buy_in_the_last_second_counts_and_one_second_later_does_not"),
    M("K3", "A tie does not take the lead", KING, "let leads = spent > k.leader_amount;",
      "let leads = spent >= k.leader_amount;", "a_tie_does_not_take_the_lead"),
    M("K4", "A buy that spent nothing is not a buy", KING, "spent > 0 && received > 0", "received > 0",
      "a_king_buy_counts_only_what_really_happened", "gave coins and took no ZEC"),
    M("K5", "A buy that received nothing is not a buy", KING, "spent > 0 && received > 0", "spent > 0",
      "a_king_buy_counts_only_what_really_happened", "took the ZEC and gave nothing"),
    M("K6", "The buy-back receiver is checked before the curve king buy", KING,
      "pump::check_buyback_recipient(\n        &a.pump_global.to_account_info(),\n        &pump::PUMP_ID,\n        &pump::ACC_GLOBAL,\n        pump::GLOBAL_BUYBACK_OFFSET,\n        &a.buyback_fee_recipient.to_account_info(),\n        &ZEC_MINT,\n        &a.token_program.key(),\n    )?;",
      "", "t11_substitution::king_buy_curve", "swapping buyback_fee_recipient"),
    M("K7", "The buy-back receiver is checked before the pool king buy", KING,
      "pump::check_buyback_recipient(\n        &a.amm_global_config.to_account_info(),\n        &pump::PUMP_AMM_ID,\n        &pump::ACC_AMM_GLOBAL_CONFIG,\n        pump::AMM_GLOBAL_BUYBACK_OFFSET,\n        &a.buyback_fee_recipient.to_account_info(),\n        &ZEC_MINT,\n        &a.token_program.key(),\n    )?;",
      "", "t11_substitution::king_buy_pool", "swapping buyback_fee_recipient"),
    M("K8", "A round cannot be settled before it ends", KING,
      "round > ledger.king.round && ledger.king.leader != Pubkey::default()", "ledger.king.leader != Pubkey::default()",
      "settling_is_open_to_anyone_once_and_only_after_the_round"),
    M("K9", "A round with no leader has nothing to settle", KING,
      "round > ledger.king.round && ledger.king.leader != Pubkey::default()", "round > ledger.king.round",
      "a_round_with_no_buys_carries_its_pot_forward"),
    M("K10", "The king pot is emptied in the ledger when paid", KING,
      "ops::debit(ledger, SINK_KING, amount)?;", "", "the_largest_buy_through_burr_leads_and_wins_the_pot"),
    M("K11", "The lead is cleared when a round is settled", KING,
      "    ledger.king.leader = Pubkey::default();\n", "", "the_largest_buy_through_burr_leads_and_wins_the_pot"),
    M("K12", "The leader's amount is cleared when a round is settled", KING,
      "    ledger.king.leader_amount = 0;\n    ledger.king.round = round;", "    ledger.king.round = round;",
      "after_a_round_is_settled_any_buy_leads_the_next"),
    M("K13", "The size of a buy is measured from the buyer's ZEC balance after the call", KING,
      "    ctx.accounts.buyer_zec.reload()?;\n", "", "the_largest_buy_through_burr_leads_and_wins_the_pot", nth=0),
    M("K14", "The size of a pool buy is measured from the buyer's ZEC balance after the call", KING,
      "    ctx.accounts.buyer_zec.reload()?;\n", "", "king_of_the_hill_works_on_the_pool", nth=1),
    # ---- pairs: two checks cover the same thing; removing both must be noticed ----
    dict(id="Z1", what="pump.fun's program address is pinned (both checks removed together)", test="t11_substitution",
         msg="swapping pump_program", nth=0, file=f"{SRC}/pump.rs",
         find="require_keys_eq!(*program.key, *expected, BurrError::WrongPumpAccount);", repl="",
         also=[(f"{SRC}/instructions/collect.rs", "    #[account(address = pump::PUMP_ID)]\n    pub pump_program", "    pub pump_program")]),
    dict(id="Z2", what="PumpSwap's program address is pinned (both checks removed together)", test="t11_substitution",
         msg="swapping amm_program", nth=0, file=f"{SRC}/pump.rs",
         find="require_keys_eq!(*program.key, *expected, BurrError::WrongPumpAccount);", repl="",
         also=[(f"{SRC}/instructions/collect.rs", "    #[account(address = pump::PUMP_AMM_ID)]\n    pub amm_program", "    pub amm_program")]),
]

# ---------------------------------------------------------------------------
# Protections on accounts, taken from the code itself: every constraint on
# every account of every instruction, and every program type.
# ---------------------------------------------------------------------------

STRUCT_TEST = {
    "Launch": "launch", "SplitOnly": "split", "CollectCurve": "collect_curve", "CollectPool": "collect_pool",
    "PayOut": "pay_out", "BurnCurve": "burn_curve", "BurnPool": "burn_pool", "StakeCoins": "stake",
    "UnstakeCoins": "unstake", "KingBuyCurve": "king_buy_curve", "KingBuyPool": "king_buy_pool",
    "KingSettle": "king_settle",
}
CONSTRAINT_KEYS = ("seeds", "bump", "address", "associated_token::", "token::", "mint::", "has_one", "constraint")


def split_top(s):
    parts, depth, cur = [], 0, ""
    for ch in s:
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append(cur.strip())
            cur = ""
        else:
            cur += ch
    if cur.strip():
        parts.append(cur.strip())
    return parts


def auto_entries():
    out = []
    for path in sorted((ROOT / SRC / "instructions").glob("*.rs")):
        if path.name in ("admin.rs", "mod.rs"):
            continue
        text = path.read_text()
        rel = f"{SRC}/instructions/{path.name}"
        for sm in re.finditer(r"pub struct (\w+)<'info> \{\n(.*?)\n\}\n", text, re.S):
            struct, body_start = sm.group(1), sm.start(2)
            if struct not in STRUCT_TEST:
                continue
            body = sm.group(2)
            test = f"t11_substitution::{STRUCT_TEST[struct]}"
            # account attributes
            for am in re.finditer(r"#\[account\(", body):
                i, depth = am.end(), 1
                while depth:
                    depth += {"(": 1, ")": -1}.get(body[i], 0)
                    i += 1
                attr_end = i + 1  # the closing ]
                inner = body[am.end():i - 1]
                fm = re.match(r"\s*(?:///[^\n]*\n\s*)*pub (\w+):", body[attr_end:])
                if not fm:
                    continue
                field = fm.group(1)
                parts = split_top(inner)
                keep = [p for p in parts if not p.startswith(CONSTRAINT_KEYS)]
                if len(keep) == len(parts):
                    continue
                if "init" in parts:
                    continue  # the address is fixed by creation itself; see the list's notes
                if "init_if_needed" in parts:
                    if not any(p.startswith("associated_token::") for p in parts):
                        continue
                    new_attr = "#[account(mut)]"
                else:
                    new_attr = f"#[account({', '.join(keep)})]" if keep else ""
                old_attr = body[am.start():attr_end]
                a, b = body_start + am.start(), body_start + attr_end
                out.append(dict(
                    id=f"X.{struct}.{field}", what=f"{struct}: `{field}` must be the right account",
                    file=rel, span=(a, b), old=old_attr, repl=new_attr, test=test,
                    msg=rf"swapping {field}( \(|\s+was)"))
            # program types
            for pm in re.finditer(r"pub (\w+): Program<'info, \w+>,", body):
                field = pm.group(1)
                a, b = body_start + pm.start(), body_start + pm.end()
                out.append(dict(
                    id=f"Y.{struct}.{field}", what=f"{struct}: `{field}` must be the real program",
                    file=rel, span=(a, b), old=pm.group(0),
                    repl=f"/// CHECK: switched off\n    pub {field}: UncheckedAccount<'info>,", test=test,
                    msg=rf"swapping {field}( \(|\s+was)"))
    return out


def apply(entry, text):
    if "span" in entry:
        a, b = entry["span"]
        assert text[a:b] == entry["old"], f"{entry['id']}: the code moved"
        return text[:a] + entry["repl"] + text[b:]
    count = text.count(entry["find"])
    assert count > entry["nth"], f"{entry['id']}: text not found ({count} found, need #{entry['nth']})"
    if entry["nth"] == 0 and count != 1:
        # several places: the first is meant only when nth was given on purpose
        shared = any(e["id"] != entry["id"] and e.get("find") == entry["find"] for e in MANUAL)
        assert shared, f"{entry['id']}: text found {count} times; say which"
    pos = -1
    for _ in range(entry["nth"] + 1):
        pos = text.index(entry["find"], pos + 1)
    return text[:pos] + entry["repl"] + text[pos + len(entry["find"]):]


ENV = dict(os.environ)
ENV["PATH"] = f"{Path.home()}/.cargo/bin:{Path.home()}/.local/share/solana/install/active_release/bin:" + ENV["PATH"]
# Faster builds for the copies; the code compiled is the same.
ENV["CARGO_PROFILE_RELEASE_LTO"] = "off"
ENV["CARGO_PROFILE_RELEASE_CODEGEN_UNITS"] = "16"


def run(cmd, cwd, timeout=1500):
    p = subprocess.run(cmd, cwd=cwd, env=ENV, capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout + p.stderr


def prepare(n):
    w = WORK / f"w{n}"
    if w.exists():
        shutil.rmtree(w)
    w.mkdir(parents=True)
    for f in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rustfmt.toml"):
        shutil.copy(ROOT / f, w / f)
    for d in ("programs", "tests", "tests-standin"):
        shutil.copytree(ROOT / d, w / d)
    os.symlink(ROOT / "fixtures", w / "fixtures")
    # A copy-on-write copy of the build cache, so dependencies are not rebuilt.
    subprocess.run(["cp", "-Rc", str(ROOT / "target"), str(w / "target")], check=True)
    return w


def build_and_test(w):
    code, out = run(["cargo", "build-sbf", "--manifest-path", "programs/burr/Cargo.toml"], w)
    if code != 0:
        return "does not build", out[-3000:]
    code, out = run(["cargo", "test", "-p", "burr-tests"], w)
    return ("fails" if code != 0 else "passes"), out


def judge(entry, out):
    failed = re.findall(r"^test (\S+) \.\.\. FAILED", out, re.M)
    blocks = dict(re.findall(r"^---- (\S+) stdout ----\n(.*?)(?=^---- |\nfailures:\n)", out, re.M | re.S))
    guards = [t for t in failed if entry["test"] in t]
    ok = False
    for t in guards:
        if not entry.get("msg") or re.search(entry["msg"], blocks.get(t, "")):
            ok = True
    return ok, failed


def worker(n, queue, results):
    w = prepare(n)
    state, out = build_and_test(w)
    if state != "passes":
        results.put(dict(id=f"baseline-{n}", status="BASELINE BROKEN", detail=out[-2000:]))
        return
    while True:
        try:
            entry = queue.get_nowait()
        except Exception:
            return
        path = w / entry["file"]
        original = (ROOT / entry["file"]).read_text()
        extra = []
        try:
            path.write_text(apply(entry, original))
            for f, old, new in entry.get("also", []):
                t = (ROOT / f).read_text()
                assert t.count(old) == 1, f"{entry['id']}: extra text not found once in {f}"
                (w / f).write_text(t.replace(old, new))
                extra.append((f, t))
            state, out = build_and_test(w)
            if state == "does not build":
                r = dict(status="DOES NOT BUILD", detail=out[-1500:])
            elif state == "passes":
                r = dict(status="NOT NOTICED", failed=[])
            else:
                ok, failed = judge(entry, out)
                r = dict(status="noticed" if ok else "NOTICED ONLY BY OTHER TESTS", failed=failed[:8])
        except Exception as e:  # noqa
            r = dict(status="ERROR", detail=str(e))
        finally:
            path.write_text(original)
            for f, t in extra:
                (w / f).write_text(t)
        r.update(id=entry["id"], what=entry["what"], test=entry["test"])
        results.put(r)
        print(f"[w{n}] {entry['id']:34} {r['status']}", flush=True)


# Protections that a second check also covers, so that no test can see the
# first one missing on its own. Each is listed with the check that still
# stands. Filled from the run's results, by reasoning about each one.
SECOND_LINE = {}

GROUPS = [
    ("Z", "Pairs of checks that cover the same thing, removed together"),
    ("A", "The administrator"), ("L", "Launch"), ("N", "The coin's mint"), ("R", "Reading pump.fun's accounts"),
    ("C", "Collect and split"), ("P", "Payouts"), ("B", "Buy back and burn"), ("V", "Vault share"),
    ("K", "King of the hill"), ("X", "Every account is the one it should be"), ("Y", "Every program is the real one"),
]


def write_report():
    results = json.loads((ROOT / "notes" / "protections.json").read_text())
    by_id = {r["id"]: r for r in results}
    held = [r for r in results if r["status"] == "noticed"]
    second = [r for r in results if r["status"] != "noticed" and r["id"] in SECOND_LINE]
    open_ = [r for r in results if r["status"] != "noticed" and r["id"] not in SECOND_LINE]
    out = ["# Protections, and the test that holds each one", ""]
    out += [
        "Made by `scripts/protections.py`. For each protection the script removes it in a throwaway copy of the "
        "code, rebuilds, and runs the whole suite. **Held** means the test named for it failed, with the message "
        "expected. The real code is never touched.",
        "",
        f"- Protections listed: **{len(results)}**",
        f"- Held by their own test: **{len(held)}**",
        f"- Second line (another check also covers it, so no test can see it missing alone): **{len(second)}**",
        f"- Not held: **{len(open_)}**",
        "",
        "Not in the table because they cannot be removed without removing the instruction itself: the addresses of "
        "`Config`, `Coin`, `Ledger` and `Stake` when they are first made (fixed by creation), and the kind of each "
        "program-owned account (`Coin`, `Ledger`, `Stake`, `Config` each carry their own 8-byte code, checked on every "
        "load). The substitution tests swap these too and they are refused.",
        "",
    ]
    for prefix, title in GROUPS:
        rows = [r for r in results if re.match(rf"{prefix}[\d.]", r["id"])]
        if not rows:
            continue
        out += [f"## {title}", "", "| Id | Protection | Result | Test that holds it |", "|---|---|---|---|"]
        for r in rows:
            if r["status"] == "noticed":
                res = "Held"
            elif r["id"] in SECOND_LINE:
                res = "Second line: " + SECOND_LINE[r["id"]]
            else:
                res = "**NOT HELD** (" + r["status"].lower() + ")"
            out.append(f"| {r['id']} | {r['what']} | {res} | `{r['test']}` |")
        out.append("")
    (ROOT / "docs" / "protections.md").write_text("\n".join(out))
    print(f"docs/protections.md written: {len(held)} held, {len(second)} second line, {len(open_)} not held")
    return by_id


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--report", action="store_true", help="only rewrite docs/protections.md from the last results")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--only")
    ap.add_argument("--workers", type=int, default=4)
    args = ap.parse_args()

    if args.report:
        write_report()
        return
    entries = MANUAL + auto_entries()
    ids = [e["id"] for e in entries]
    assert len(ids) == len(set(ids)), "duplicate ids"
    problems = []
    for e in entries:
        try:
            apply(e, (ROOT / e["file"]).read_text())
        except AssertionError as err:
            problems.append(str(err))
    if problems:
        sys.exit("The list does not match the code:\n  " + "\n  ".join(problems))
    print(f"{len(entries)} protections ({len(MANUAL)} written by hand, {len(entries) - len(MANUAL)} taken from the accounts)")
    if args.check:
        return
    if args.only:
        want = set(args.only.split(","))
        entries = [e for e in entries if e["id"] in want or any(e["id"].startswith(x) for x in want if x.endswith("."))]

    free_gb = shutil.disk_usage(ROOT).free / 1e9
    if free_gb < 12:
        sys.exit(f"Only {free_gb:.0f} GB free. Clean up first.")

    mgr = mp.Manager()
    queue, results = mgr.Queue(), mgr.Queue()
    for e in entries:
        queue.put(e)
    procs = [mp.Process(target=worker, args=(n, queue, results)) for n in range(min(args.workers, len(entries)))]
    for p in procs:
        p.start()
    for p in procs:
        p.join()
    got = []
    while not results.empty():
        got.append(results.get())
    order = {i: n for n, i in enumerate(ids)}
    got.sort(key=lambda r: order.get(r["id"], -1))

    notes = ROOT / "notes"
    notes.mkdir(exist_ok=True)
    full = notes / "protections.json"
    if args.only and full.exists():
        # A partial run replaces just those entries in the full results.
        merged = {r["id"]: r for r in json.loads(full.read_text())}
        merged.update({r["id"]: r for r in got})
        allr = sorted(merged.values(), key=lambda r: order.get(r["id"], -1))
        allr = [r for r in allr if r["id"] in order]
        full.write_text(json.dumps(allr, indent=1))
    else:
        full.write_text(json.dumps(got, indent=1))
    bad = [r for r in got if r["status"] != "noticed"]
    print(f"\n{len(got) - len(bad)} of {len(got)} noticed by their own test.")
    for r in bad:
        print(f"  {r['id']:34} {r['status']}  {r.get('failed', '')}")
    shutil.rmtree(WORK, ignore_errors=True)


if __name__ == "__main__":
    main()
