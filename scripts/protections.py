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
SRC = "programs/burrpad/src"
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
HOLD = "instructions/holders.rs"
BURR = "instructions/burr.rs"
LIQ = "instructions/liquidity.rs"
SHADOW = "instructions/shadow.rs"
ADMIN_RS = ADMIN
T_SALE = "t22_shadow_burn::a_sale_is_bought_back_at_once_with_what_it_received_never_more"
T_LIST = "t22_shadow_burn::only_the_launcher_sets_the_watch_and_only_sensible_lists"
T_CHANGE = "t22_shadow_burn::a_change_of_wallets_keeps_what_is_owed_and_starts_the_new_ones_from_now"
T_FOLLOW = "t22_shadow_burn::the_keeper_follows_plain_wallets_the_target_sends_to_and_only_those"
T_SIXTEEN = "t22_shadow_burn::the_keeper_never_follows_past_sixteen_and_the_launcher_can_stop_it"
T_DUMP = "t22_shadow_burn::a_dump_larger_than_the_pot_spends_the_whole_pot_and_the_rest_waits"
T_TRADE = "t22_shadow_burn::a_trade_around_the_instant_buy_is_refused"
T_SUBST = "t22_shadow_burn::shadow_burn_takes_only_this_coins_watch_pot_and_the_watched_accounts"

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
      "require!(fraction_bps <= HARD_MAX_PLATFORM_FRACTION_BPS, BurrpadError::PlatformFractionTooHigh);", "",
      "admin_settings_stay_inside_the_hard_limits"),
    M("A20", "The platform fraction can never be set below a quarter", ADMIN,
      "    require!(fraction_bps >= HARD_MIN_PLATFORM_FRACTION_BPS, BurrpadError::PlatformFractionTooLow);\n", "",
      "admin_settings_stay_inside_the_hard_limits", "platform fraction 0"),
    M("A8", "The lowest fee setting can never go below 0.10%", ADMIN,
      "min_bps >= HARD_MIN_FEE_BPS", "true", "admin_settings_stay_inside_the_hard_limits", "9 300 200"),
    M("A9", "The highest fee setting can never go above 5.00%", ADMIN,
      "max_bps <= HARD_MAX_FEE_BPS", "true", "admin_settings_stay_inside_the_hard_limits", "50 501 200"),
    M("A10", "The first-offered fee cannot be below the lowest", ADMIN,
      "min_bps <= default_bps", "true", "admin_settings_stay_inside_the_hard_limits", "50 300 49"),
    M("A11", "The first-offered fee cannot be above the highest", ADMIN,
      "default_bps <= max_bps", "true", "admin_settings_stay_inside_the_hard_limits", "50 300 301"),
    M("A12", "The platform receiver cannot be set to the empty address", ADMIN,
      "require!(receiver != Pubkey::default(), BurrpadError::EmptyAddress);", "",
      "admin_settings_stay_inside_the_hard_limits"),
    M("A16", "Proposing a new administrator is announced", ADMIN,
      "    emit!(AdminProposed { admin: ctx.accounts.config.admin, pending_admin: new_admin });\n", "",
      "the_admin_role_moves_in_two_steps"),
    M("A13", "Only the proposed administrator can accept the role", ADMIN,
      "constraint = config.pending_admin == new_admin.key() @", "constraint = true @",
      "the_admin_role_moves_in_two_steps"),
    M("A14", "The proposed administrator must sign", ADMIN,
      "pub new_admin: Signer<'info>,", "/// CHECK: switched off\n    pub new_admin: UncheckedAccount<'info>,",
      "naming_a_key_is_not_signing_with_it", "proposed administrator"),

    M("A15", "Config records its layout version", ADMIN, "    c.version = LAYOUT_VERSION;\n", "",
      "every_account_carries_a_layout_version_and_spare_room"),
    M("L27", "A coin's record carries its layout version", LAUNCH, "    coin.version = LAYOUT_VERSION;\n", "",
      "every_account_carries_a_layout_version_and_spare_room"),
    M("L28", "A coin's ledger carries its layout version", LAUNCH, "    ledger.version = LAYOUT_VERSION;\n", "",
      "every_account_carries_a_layout_version_and_spare_room"),

    # ---- launch ----
    M("L1", "No launch while launches are switched off", LAUNCH,
      "require!(!cfg.launches_paused, BurrpadError::LaunchesPaused);", "", "pausing_stops_new_launches_only"),
    M("L2", "A coin's fee cannot be below the lowest allowed", LAUNCH,
      "args.fee_bps >= cfg.min_fee_bps && ", "", "launch_refuses_bad_plans"),
    M("L3", "A coin's fee cannot be above the highest allowed", LAUNCH,
      " && args.fee_bps <= cfg.max_fee_bps", "", "launch_refuses_bad_plans"),
    M("L4", "Hook shares must add up to exactly the whole", LAUNCH,
      "require!(\n        math::weights_all(cfg.platform_fraction_bps, &args.shares, [burr_share, liquidity_share, shadow_share])\n            .is_some(),\n        BurrpadError::SharesDoNotAddUp\n    );", "",
      "launch_refuses_bad_plans"),
    M("L5", "A chosen creator's share needs a receiver", LAUNCH,
      "args.shares[SINK_CREATOR - 1] == 0 || args.creator_receiver != Pubkey::default()", "true",
      "launch_refuses_bad_plans"),
    M("L6", "A chosen give hook needs a receiver", LAUNCH,
      "args.shares[SINK_GIVE - 1] == 0 || args.give_receiver != Pubkey::default()", "true",
      "launch_refuses_bad_plans"),
    M("L9", "Name, ticker and link lengths are checked before calling pump.fun", LAUNCH,
      "args.name.len() <= MAX_NAME_LEN && ", "", "launch_refuses_bad_plans"),
    M("L29", "A picture link may be at most 100 characters, so launch and first buy fit one transaction", LAUNCH,
      " && args.uri.len() <= MAX_URI_LEN", "", "a_link_longer_than_fits_is_refused"),
    M("L10", "After the call: the coin's creator is our authority", LAUNCH,
      "curve.creator == authority_key", "true", "a_launch_is_refused_if_the_coin_made_is_not", "another creator"),
    M("L11", "After the call: the coin is priced in its quote coin", LAUNCH,
      "&& curve.quote_mint == quote_mint\n            && curve.creator_fee_bps", "&& curve.creator_fee_bps",
      "a_launch_is_refused_if_the_coin_made_is_not", "another quote coin"),
    M("L12", "After the call: the coin's fee is the one asked for", LAUNCH,
      "&& curve.creator_fee_bps == args.fee_bps as u64", "", "a_launch_is_refused_if_the_coin_made_is_not",
      "another fee rate"),
    M("L25", "After the call: the coin's fee is not marked editable", LAUNCH,
      "&& !curve.can_edit_creator_fee", "", "a_launch_is_refused_if_the_coin_made_is_not", "marked editable"),
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
      "liquidity_added_at_the_end_of_a_climb_does_not_raise_the_pool_burns_limit"),
    M("L19", "The first burn cannot come in the first minute", LAUNCH,
      "ledger.burn.last_time = now;", "", "a_push_before_the_first_burn_cannot_raise_what_it_spends"),

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
      "require!(d.len() > 166 && d[165] == 1, BurrpadError::UnexpectedMint);",
      "require!(d.len() > 166, BurrpadError::UnexpectedMint);", "a_mint_that_is_not_the_plain_kind_is_refused"),
    M("N7", "A coin's mint may carry no extension but its metadata", "pump.rs",
      "require!(kind == EXT_METADATA_POINTER || kind == EXT_TOKEN_METADATA, BurrpadError::UnexpectedMint);", "",
      "a_mint_that_is_not_the_plain_kind_is_refused", "was accepted"),

    # ---- reading pump.fun's accounts ----
    M("R1", "A curve account must be owned by pump.fun's program", "pump.rs",
      "require_keys_eq!(*ai.owner, PUMP_ID, BurrpadError::WrongPumpAccount);", "",
      "the_curve_reader_checks_owner_code_and_length"),
    M("R2", "A curve account must be long enough and carry the curve's code", "pump.rs",
      "require!(d.len() >= CURVE_MIN_LEN && d[..8] == ACC_BONDING_CURVE, BurrpadError::UnreadablePumpAccount);",
      "require!(d.len() >= CURVE_MIN_LEN, BurrpadError::UnreadablePumpAccount);",
      "the_curve_reader_checks_owner_code_and_length"),
    M("R3", "A pool account must be owned by PumpSwap", "pump.rs",
      "require_keys_eq!(*ai.owner, PUMP_AMM_ID, BurrpadError::WrongPumpAccount);\n    let d = ai.try_borrow_data()?;\n    require!(d.len() >= POOL_MIN_LEN",
      "let d = ai.try_borrow_data()?;\n    require!(d.len() >= POOL_MIN_LEN",
      "the_pool_reader_checks_owner_code_and_length"),
    M("R4", "A pool account must be long enough and carry the pool's code", "pump.rs",
      "require!(d.len() >= POOL_MIN_LEN && d[..8] == ACC_POOL, BurrpadError::UnreadablePumpAccount);",
      "require!(d.len() >= POOL_MIN_LEN, BurrpadError::UnreadablePumpAccount);",
      "the_pool_reader_checks_owner_code_and_length"),
    M("R5", "A pool's price reserve must be positive", "pump.rs",
      "require!(v > 0, BurrpadError::UnreadablePumpAccount);", "",
      "the_pools_price_reserve_must_be_positive_and_fit"),
    M("R6", "The settings account must be owned by pump.fun's program", "pump.rs",
      "require_keys_eq!(*settings.owner, *settings_owner, BurrpadError::WrongPumpAccount);", "",
      "only_a_listed_buy_back_receivers_zec_account_is_accepted"),
    M("R7", "A buy-back receiver's account must belong to the classic token program", "pump.rs",
      "require_keys_eq!(*token_account.owner, *token_program, BurrpadError::WrongPumpAccount);", "",
      "only_a_listed_buy_back_receivers_zec_account_is_accepted"),
    M("R8", "A buy-back receiver's account must be a ZEC account", "pump.rs",
      "require_keys_eq!(key_at(&t, 0), *zec_mint, BurrpadError::WrongPumpAccount);", "",
      "only_a_listed_buy_back_receivers_zec_account_is_accepted"),
    M("R9", "A buy-back receiver must be on pump.fun's own list", "pump.rs",
      "require!(listed && holder != Pubkey::default(), BurrpadError::WrongPumpAccount);",
      "require!(holder != Pubkey::default(), BurrpadError::WrongPumpAccount);",
      "only_a_listed_buy_back_receivers_zec_account_is_accepted"),
    M("R10", "The empty address is never a buy-back receiver", "pump.rs",
      "require!(listed && holder != Pubkey::default(), BurrpadError::WrongPumpAccount);",
      "require!(listed, BurrpadError::WrongPumpAccount);",
      "only_a_listed_buy_back_receivers_zec_account_is_accepted"),
    M("R11", "The settings account must carry the settings' code and be long enough", "pump.rs",
      "s.len() >= list_offset + 32 * BUYBACK_COUNT && s[..8] == *settings_code", "s.len() >= list_offset + 32 * BUYBACK_COUNT",
      "only_a_listed_buy_back_receivers_zec_account_is_accepted"),
    M("R12", "Calls go only to the exact pump.fun program addresses", "pump.rs",
      "require_keys_eq!(*program.key, *expected, BurrpadError::WrongPumpAccount);", "",
      "t11_substitution", "swapping (pump_program|amm_program)"),

    # ---- collect and split ----
    M("C1", "The vault is read again after pump.fun's collect, on the curve side", COLLECT,
      "ctx.accounts.vault.reload()?;", "", "collect_brings_the_fee_in_and_splits_it_exactly", nth=1),
    M("C2", "The vault is read again after pump.fun's collect, on the pool side", COLLECT,
      "ctx.accounts.vault.reload()?;", "", "fees_from_both_the_curve_and_the_pool_reach_the_vault", nth=2),
    M("C3", "Collecting from a pool that does not exist yet is refused clearly", COLLECT,
      "require!(!a.pool.data_is_empty(), BurrpadError::WrongVenue);", "", "collect_pool_before_graduation_is_refused"),
    M("C4", "A vault holding less than the ledger says stops the split", "ops.rs",
      "vault_amount.checked_sub(ledger.accounted).ok_or(BurrpadError::VaultShort)?", "vault_amount.saturating_sub(ledger.accounted)",
      "a_vault_holding_less_than_the_ledger_says_stops_the_split"),
    M("C5", "Dues are worked out on the running total, so call pattern cannot tilt the split", "math.rs",
      "let total = total_split.checked_add(new as u128)?;",
      "let total = new as u128;\n    *credited = [0u128; SINKS];",
      "splitting_in_many_small_calls_gives_the_same_as_one"),
    M("C6", "A pot is credited only the difference between its due and what it already had", "math.rs",
      "let add = due.checked_sub(credited[i])?;", "let add = due;", "the_split_does_not_depend_on_how_it_is_called"),
    M("C7", "A pot cannot be debited below zero", "ops.rs",
      "ledger.pots[sink] = ledger.pots[sink].checked_sub(amount).ok_or(BurrpadError::Overflow)?;",
      "ledger.pots[sink] = ledger.pots[sink].wrapping_sub(amount);", "a_pot_cannot_be_debited_below_zero",
      "more than the pot"),
    M("C8", "The ledger's total cannot be debited below zero", "ops.rs",
      "ledger.accounted = ledger.accounted.checked_sub(amount).ok_or(BurrpadError::Overflow)?;",
      "ledger.accounted = ledger.accounted.wrapping_sub(amount);", "a_pot_cannot_be_debited_below_zero",
      "more than the ledger"),
    M("P3", "An empty pot pays nothing and makes no account", "ops.rs",
      "    if amount == 0 {\n        return Ok(());\n    }\n    if p.destination.data_is_empty()", "    if p.destination.data_is_empty()",
      "an_empty_pot_pays_nothing_and_makes_no_account"),

    M("L41", "pump.fun's curve fee vault for the coin cannot be a receiver", LAUNCH,
      "r != curve_fee_vault && ", "", "pump_funs_fee_vaults_for_a_coin_cannot_be_named_as_its_receivers", "fee vault case 0"),
    M("L42", "pump.fun's pool fee vault for the coin cannot be a receiver", LAUNCH,
      "r != curve_fee_vault && r != pool_fee_vault", "r != curve_fee_vault", "pump_funs_fee_vaults_for_a_coin_cannot_be_named_as_its_receivers", "fee vault case 2"),
    M("L43", "Only the launcher's own coin account can be made by a launch", LAUNCH,
      "require_keys_eq!(buyer.key(), own, BurrpadError::WrongBuyerAccount);", "",
      "only_the_launchers_own_coin_account_can_be_made"),
    M("L40", "Launch makes up the rent pump.fun leaves a new mint short of", LAUNCH,
      "    if short > 0 {", "    if false {", "a_launch_works_at_every_text_length"),

    M("B60", "A burn limit worked out with a divisor of zero is refused, not a crash", "math.rs",
      "    if divisor == 0 {\n        return None;\n    }\n", "", "the_burn_limit_and_floor"),

    # ---- recover ----
    M("C30", "Recovered ZEC is split, not left unaccounted", COLLECT,
      "    ops::split_new(&ctx.accounts.coin, &mut ctx.accounts.ledger, ctx.accounts.vault.amount)?;\n    emit!(Recovered", "    emit!(Recovered",
      "a_pool_fee_paid_into_a_side_account_can_be_brought_home_by_anyone", "the recovered fee was not split"),
    M("C31", "Recover reads the vault afresh after moving the ZEC", COLLECT,
      "    ctx.accounts.vault.reload()?;\n    ops::split_new(&ctx.accounts.coin, &mut ctx.accounts.ledger, ctx.accounts.vault.amount)?;\n    emit!(Recovered",
      "    ops::split_new(&ctx.accounts.coin, &mut ctx.accounts.ledger, ctx.accounts.vault.amount)?;\n    emit!(Recovered",
      "a_pool_fee_paid_into_a_side_account_can_be_brought_home_by_anyone", "the recovered fee was not split"),

    # ---- holders ----
    M("H1", "Only the rewards wallet named in the settings can take a holder pot", HOLD,
      "#[account(mut, address = config.rewards_wallet @ BurrpadError::NotRewardsWallet)]", "#[account(mut)]",
      "only_the_rewards_wallet_can_take_the_pot"),
    M("H2", "The rewards wallet must sign", HOLD,
      "pub rewards_wallet: Signer<'info>,", "pub rewards_wallet: UncheckedAccount<'info>,",
      "only_the_rewards_wallet_can_take_the_pot"),
    M("H3", "Holder releases are at least half an hour apart", HOLD,
      "require!(clock.unix_timestamp >= ready, BurrpadError::HolderReleaseTooSoon);", "",
      "releases_are_at_least_half_an_hour_apart"),
    M("H4", "A release of nothing is refused and does not use up the interval", HOLD,
      "require!(amount > 0, BurrpadError::NothingToRelease);", "",
      "an_empty_pot_and_a_coin_without_the_hook_release_nothing"),
    M("H23", "A release cannot take more than the holder pot holds", HOLD,
      "require!(amount <= ledger.pots[SINK_HOLDERS], BurrpadError::ReleaseAboveThePot);", "",
      "a_release_takes_the_amount_it_states_and_the_rest_stays", "more than the pot"),
    M("H5", "A release must carry a snapshot fingerprint", HOLD,
      "snapshot_hash != [0u8; 32] && ", "", "a_release_must_name_a_real_snapshot", "no fingerprint"),
    M("H6", "A release cannot name a slot that has not happened", HOLD,
      "snapshot_slot <= clock.slot && ", "", "a_release_must_name_a_real_snapshot", "a future slot"),
    M("H7", "A snapshot must be later than the release before it", HOLD,
      " && snapshot_slot > ledger.holders.last_release_slot", "", "a_release_must_name_a_real_snapshot", "an old slot"),
    M("H24", "The slot a release landed in is recorded", HOLD,
      "    h.last_release_slot = clock.slot;\n", "", "a_snapshot_must_be_later_than_the_release_before_it"),
    M("H25", "The snapshot's fingerprint is recorded", HOLD,
      "    h.last_snapshot_hash = snapshot_hash;\n", "", "a_snapshot_must_be_later_than_the_release_before_it"),
    M("H26", "A snapshot cannot be from before the coin existed", LAUNCH,
      "    ledger.holders.last_release_slot = clock.slot;\n", "",
      "a_snapshot_must_be_later_than_the_release_before_it"),
    M("H8", "A coin without the holders hook releases nothing", HOLD,
      "require!(coin.share_of(SINK_HOLDERS) > 0, BurrpadError::HookNotChosen);", "",
      "an_empty_pot_and_a_coin_without_the_hook_release_nothing"),
    M("H9", "The holder pot is emptied in the ledger when released", HOLD,
      "ops::debit(ledger, SINK_HOLDERS, amount)?;", "", "the_whole_pot_goes_to_the_rewards_wallet_and_nowhere_else"),
    M("H10", "The time of a release is recorded", HOLD,
      "    h.last_release = clock.unix_timestamp;\n", "", "releases_are_at_least_half_an_hour_apart"),
    M("H11", "The snapshot's slot is recorded", HOLD,
      "    h.last_snapshot_slot = snapshot_slot;\n", "", "a_snapshot_must_be_later_than_the_release_before_it"),
    M("H12", "The first holder release cannot come in the first half hour", LAUNCH,
      "    ledger.holders.last_release = now;\n", "", "releases_are_at_least_half_an_hour_apart"),
    M("H13", "A proposed rewards wallet cannot be the empty address", ADMIN,
      "require!(wallet != Pubkey::default(), BurrpadError::EmptyAddress);\n    require!(wallet != ctx.accounts.config.creator_payout_wallet",
      "require!(wallet != ctx.accounts.config.creator_payout_wallet",
      "a_new_rewards_wallet_is_proposed_in_the_open_and_waits_a_day"),
    M("H15", "A proposed rewards wallet cannot be applied before its wait is over", ADMIN,
      "require!(now >= c.rewards_wallet_ready_at, BurrpadError::RewardsWalletWaitNotOver);", "",
      "a_new_rewards_wallet_is_proposed_in_the_open_and_waits_a_day", "applied at once"),
    M("H16", "The wait for a proposed rewards wallet is a full day", ADMIN,
      "checked_add(REWARDS_WALLET_WAIT_SECS)", "checked_add(REWARDS_WALLET_WAIT_SECS - 1)",
      "a_new_rewards_wallet_is_proposed_in_the_open_and_waits_a_day"),
    M("H17", "Nothing can be applied when no rewards wallet is proposed", ADMIN,
      "require!(c.pending_rewards_wallet != Pubkey::default(), BurrpadError::NoRewardsWalletProposed);", "",
      "a_new_rewards_wallet_is_proposed_in_the_open_and_waits_a_day", "nothing proposed", nth=1),
    M("H18", "A proposal is announced with the new address", ADMIN,
      "    emit!(RewardsWalletProposed { admin: c.admin, current: c.rewards_wallet, proposed: wallet, ready_at });\n", "",
      "a_new_rewards_wallet_is_proposed_in_the_open_and_waits_a_day", "no event for the proposal"),
    M("H19", "Cancelling removes the proposal", ADMIN,
      "    c.pending_rewards_wallet = Pubkey::default();\n    c.rewards_wallet_ready_at = 0;\n    emit!", "    emit!",
      "a_proposed_rewards_wallet_can_be_cancelled_or_replaced"),
    M("H20", "Applying a proposal uses it up", ADMIN,
      "    c.rewards_wallet = c.pending_rewards_wallet;\n    c.pending_rewards_wallet = Pubkey::default();\n",
      "    c.rewards_wallet = c.pending_rewards_wallet;\n",
      "a_new_rewards_wallet_is_proposed_in_the_open_and_waits_a_day"),
    M("H21", "No holder pot can be released while releases are switched off", HOLD,
      "require!(!ctx.accounts.config.holder_releases_paused, BurrpadError::HolderReleasesPaused);", "",
      "the_pause_stops_every_coins_release_at_once_and_nothing_else", "released while paused"),
    M("H22", "The switch for holder releases is recorded", ADMIN,
      "    ctx.accounts.config.holder_releases_paused = paused;\n", "",
      "the_pause_stops_every_coins_release_at_once_and_nothing_else"),
    M("H27", "A release's event carries its run number", HOLD,
      "    h.runs = h.runs.checked_add(1).ok_or(BurrpadError::Overflow)?;\n", "",
      "each_release_is_announced_with_its_run_number"),
    M("H28", "A new administrator does not inherit a proposed rewards wallet", ADMIN,
      "    c.pending_admin = Pubkey::default();\n    // A new administrator does not inherit a rewards-wallet change it never\n    // proposed: it must propose its own, in the open, and wait the full time.\n    c.pending_rewards_wallet = Pubkey::default();\n    c.rewards_wallet_ready_at = 0;\n",
      "    c.pending_admin = Pubkey::default();\n",
      "a_new_administrator_does_not_inherit_a_proposed_rewards_wallet"),
    M("H14", "The rewards wallet cannot be the empty address at initialisation", ADMIN,
      "require!(rewards_wallet != Pubkey::default(), BurrpadError::EmptyAddress);", "",
      "initialise_refuses_empty_addresses", "empty rewards wallet"),

    # ---- buy back and burn ----
    M("B1", "A coin without the burn hook cannot burn", BURN,
      "require!(coin.share_of(SINK_BURN) > 0, BurrpadError::HookNotChosen);", "", "a_coin_without_the_hook_cannot_burn"),
    M("B2", "Burns wait between calls", BURN,
      "require!(now >= ready, BurrpadError::BurnTooSoon);", "", "burns_wait_between_calls"),
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
      "require!(ledger.pots[SINK_BURN] > 0, BurrpadError::NothingToBurn);", "",
      "an_empty_pot_cannot_be_used_to_move_the_reference"),
    M("B23", "The limit is measured against the smaller of the reserve now and the reserve last recorded", BURN,
      "reserves.quote.min(b.reference.quote),", "reserves.quote,",
      "liquidity_added_just_for_one_call_does_not_raise_the_limit"),
    M("B24", "If nothing can be bought (no fee charged, or too little), the burn stops", BURN,
      "require!(min_out > 0, BurrpadError::NothingToBurn);", "", "the_limit_follows_the_fee_pump_fun_really_charges"),
    M("B10", "The curve burn works only while the curve is trading", BURN,
      "!curve.complete && curve.quote_mint == ctx.accounts.coin.quote_mint()", "curve.quote_mint == ctx.accounts.coin.quote_mint()",
      "each_burn_instruction_only_works_on_its_own_venue"),
    M("B11", "The buy-back receiver is checked before the curve buy", BURN,
      "pump::check_buyback_recipient(\n        &a.pump_global.to_account_info(),\n        &pump::PUMP_ID,\n        &pump::ACC_GLOBAL,\n        pump::GLOBAL_BUYBACK_OFFSET,\n        &a.buyback_fee_recipient.to_account_info(),\n        &a.zec_mint.key(),\n        &a.token_program.key(),\n    )?;",
      "", "t11_substitution::burn_curve", "swapping buyback_fee_recipient"),
    M("B12", "The buy-back receiver is checked before the pool buy", BURN,
      "pump::check_buyback_recipient(\n        &a.amm_global_config.to_account_info(),\n        &pump::PUMP_AMM_ID,\n        &pump::ACC_AMM_GLOBAL_CONFIG,\n        pump::AMM_GLOBAL_BUYBACK_OFFSET,\n        &a.buyback_fee_recipient.to_account_info(),\n        &a.zec_mint.key(),\n        &a.token_program.key(),\n    )?;",
      "", "t11_substitution::burn_pool", "swapping buyback_fee_recipient"),
    M("B13", "The buy may not take more ZEC than was set aside", BURN,
      "require!(spent <= spend, BurrpadError::BurnOverspent);", "", "a_buy_cannot_take_more_than_was_set_aside"),
    M("B14", "The buy must deliver at least the least-coins-out, checked from balances", BURN,
      "require!(bought >= min_out, BurrpadError::BurnBadFill);", "", "a_burn_is_undone_if_the_buy_did_not_deliver"),
    M("B15", "What was spent is taken off the burn pot", BURN,
      "ops::debit(ledger, SINK_BURN, spent)?;", "", "a_burn_buys_the_coin_and_destroys_it"),
    M("B16", "Every coin bought is burned", BURN,
      "if coins > 0 {", "if false && coins > 0 {", "a_burn_buys_the_coin_and_destroys_it"),
    M("B17", "ZEC left with the burner goes back to the vault", BURN,
      "if zec_left > 0 {", "if false && zec_left > 0 {", "gifts_to_the_burner_do_not_confuse_it"),
    M("B18", "After a burn the pool's price right after it becomes the reference", BURN,
      "after.held_to(ceiling).ok_or(BurrpadError::Overflow.into())", "let _ = (after, ceiling);\n    Ok(*last)",
      "a_burn_buys_the_coin_and_destroys_it"),
    M("B19", "After a curve burn the time is recorded", BURN,
      "        Reserves { quote: after.virtual_quote_reserves, base: after.virtual_token_reserves },\n    )?;\n    b.last_time = now;",
      "        Reserves { quote: after.virtual_quote_reserves, base: after.virtual_token_reserves },\n    )?;", "burns_wait_between_calls"),
    M("B22", "After a pool burn the time is recorded", BURN,
      "    b.reference = next_reference(&b.reference, after)?;\n    b.last_time = now;",
      "    b.reference = next_reference(&b.reference, after)?;", "a_burn_on_the_pool_buys_and_destroys"),
    M("B40", "The reserve the limit is measured against grows by the program's own 5% step per buy, not more", BURN,
      "last.raised_by(BURN_RESERVE_STEP_BPS)", "last.raised_by(10_000)",
      "pushing_the_price_for_each_call_raises_the_limit_one_step_at_most", "the recorded reserve grew"),
    M("B26", "The reserve recorded after a burn can grow by one step at most", BURN,
      "after.held_to(ceiling).ok_or(BurrpadError::Overflow.into())", "let _ = ceiling;\n    Ok(after)",
      "liquidity_added_just_for_one_call_does_not_raise_the_limit"),
    M("B28", "A pool's record must name pump.fun's own address as its maker", BURN,
      "            && p.pool_creator == *expect.pool_authority\n", "",
      "a_pool_record_that_does_not_read_as_expected_stops_the_burn", "maker"),
    M("B29", "A pool's record must name the coin account we were given", BURN,
      "            && p.pool_base_token_account == *expect.base_account\n", "",
      "a_pool_record_that_does_not_read_as_expected_stops_the_burn", "coin account"),
    M("B30", "A pool's record must name the ZEC account we were given", BURN,
      "            && p.pool_quote_token_account == *expect.quote_account,", ",",
      "a_pool_record_that_does_not_read_as_expected_stops_the_burn", "ZEC account"),
    M("B31", "A pool's record must name this coin and ZEC", BURN,
      "p.base_mint == *expect.mint\n            && p.quote_mint == *expect.quote_mint\n            && ", "",
      "a_pool_record_that_does_not_read_as_expected_stops_the_burn", "coin mint"),
    M("B20", "A pool's price uses its ZEC balance plus the pool's own adjustment", BURN,
      "let quote = pump::effective_quote_reserves(quote_amount, p.virtual_quote_reserves)?;",
      "let quote = quote_amount;", "a_burn_on_the_pool_buys_and_destroys"),
    M("B21", "The least-coins-out is worked out from the pool's own figures", "math.rs",
      "u64::try_from(ideal.checked_mul(keep)? / BPS as u128).ok()", "let _ = (ideal, keep);\n    Some(1)",
      "a_burn_is_undone_if_the_buy_did_not_deliver", "a crumb"),

    # ---- burning $BURR ----
    M("U1", "The $BURR shares count in the whole the shares must add to", "math.rs",
      "let mut sum: u32 = burr_share as u32 + liquidity_share as u32 + shadow_share as u32;", "let mut sum: u32 = liquidity_share as u32 + shadow_share as u32;",
      "shares_with_burr_must_add_to_the_whole"),
    M("U2", "The $BURR pot gets its weight of every split", "math.rs",
      "w[SINK_BURR] = rest * burr_share as u128;", "", "a_launch_can_give_part_of_the_fee_to_burning_burr"),
    M("U3", "What the split gives the $BURR pot is added to it", "ops.rs",
      "ledger.burr.pot = ledger.burr.pot.checked_add(all[SINK_BURR]).ok_or(BurrpadError::Overflow)?;", "",
      "a_launch_can_give_part_of_the_fee_to_burning_burr"),
    M("U4", "The $BURR pot's running total is kept", "ops.rs",
      "ledger.burr.credited = credited[SINK_BURR];", "", "a_launch_can_give_part_of_the_fee_to_burning_burr"),
    M("U5", "A coin that did not choose the hook cannot buy $BURR", BURR,
      "require!(ctx.accounts.coin.burr_share > 0, BurrpadError::HookNotChosen);", "",
      "a_coin_launched_the_old_way_has_no_burr_share"),
    M("U6", "One $BURR buy a minute over all coins", BURR,
      "now >= x.config.burr_buy.ready_at && now >= own", "now >= own", "one_burr_buy_a_minute_across_all_coins"),
    M("U15", "No $BURR buy within a minute of $BURR's own buy back", BURR,
      "now >= x.config.burr_buy.ready_at && now >= own", "now >= x.config.burr_buy.ready_at", "burrs_own_buy_back_and_burn_burr_share_one_wait"),
    M("U16", "$BURR's own buy back waits after a $BURR buy as long as the next $BURR buy does", BURR,
      "own.last_time = own.last_time.max(held);", "let _ = (own, held);", "burrs_own_buy_back_and_burn_burr_share_one_wait"),
    M("U17", "A $BURR buy holds the next one off only in proportion to the part of its limit used", BURR,
      "math::part_used(done.spent, cap, BURN_COOLDOWN_SECS as u64)", "Some(BURN_COOLDOWN_SECS as u64)",
      "crumbs_can_neither_hold_the_wait_nor_walk_the_reserve"),
    M("U18", "A $BURR buy moves the recorded reserve only in proportion to the part of its limit used", BURR,
      "math::part_used(done.spent, cap, BURN_RESERVE_STEP_BPS as u64)", "Some(BURN_RESERVE_STEP_BPS as u64)",
      "crumbs_can_neither_hold_the_wait_nor_walk_the_reserve"),
    M("U19", "The $BURR limit counts no more fee than a coin may launch with", BURR,
      ".min(x.config.max_fee_bps)", "", "burr_limit_counts_no_more_fee_than_a_coin_may_launch_with"),
    M("U20", "The recorded $BURR reserve falls a step at most per buy", "math.rs",
      "(after as u128).clamp(low, high)", "(after as u128).min(high)", "the_recorded_reserve_moves_a_step_at_most_either_way"),
    M("U7", "Before any $BURR buy the limit is measured against the fixed starting reserve", BURR,
      "if shared.reference.quote == 0 { BURR_START_RESERVE }", "if shared.reference.quote == 0 { u64::MAX }",
      "the_recorded_reserve_starts_at_one_zec_and_grows_a_step_per_buy"),
    M("U8", "The $BURR limit is measured against the smaller of the reserve now and the recorded one", BURR,
      "reserves.quote.min(recorded)", "reserves.quote", "burn_burr_buys_burr_and_destroys_it_within_the_limit"),
    M("U9", "The buy-back receiver is checked before the $BURR buy", BURR,
      "pump::check_buyback_recipient(\n        &a.amm_global_config.to_account_info(),\n        &pump::PUMP_AMM_ID,\n        &pump::ACC_AMM_GLOBAL_CONFIG,\n        pump::AMM_GLOBAL_BUYBACK_OFFSET,\n        &a.buyback_fee_recipient.to_account_info(),\n        &ZEC_MINT,\n        &a.token_program.key(),\n    )?;",
      "", "t11_substitution::burn_burr", "swapping buyback_fee_recipient"),
    M("U10", "What was spent is taken off the $BURR pot", BURR,
      "ledger.burr.pot = ledger.burr.pot.checked_sub(done.spent).ok_or(BurrpadError::Overflow)?;", "",
      "burn_burr_buys_burr_and_destroys_it_within_the_limit"),
    M("U11", "What was spent is taken off what the vault must cover", BURR,
      "    ledger.accounted = ledger.accounted.checked_sub(done.spent).ok_or(BurrpadError::Overflow)?;\n    ledger.burr.total_spent", "    ledger.burr.total_spent",
      "burn_burr_buys_burr_and_destroys_it_within_the_limit"),
    M("U12", "Every $BURR bought is burned", BURR,
      "        burr_after,\n        zec_after,\n        ZEC_DECIMALS,\n    )?;", "        0,\n        zec_after,\n        ZEC_DECIMALS,\n    )?;",
      "burn_burr_buys_burr_and_destroys_it_within_the_limit"),
    M("U13", "The recorded $BURR reserve grows one step per buy at most", BURR,
      "quote: math::burr_next_reserve(recorded, after.quote, step).ok_or(BurrpadError::Overflow)?,",
      "quote: after.quote,", "the_recorded_reserve_starts_at_one_zec_and_grows_a_step_per_buy"),
    M("U14", "After a $BURR buy the time is recorded for all coins", BURR,
      "buy.ready_at = now.checked_add(wait as i64).ok_or(BurrpadError::Overflow)?;", "",
      "one_burr_buy_a_minute_across_all_coins"),

    # ---- liquidity ----
    M("Q1", "The liquidity share counts in the whole the shares must add to", "math.rs",
      "let mut sum: u32 = burr_share as u32 + liquidity_share as u32 + shadow_share as u32;", "let mut sum: u32 = burr_share as u32 + shadow_share as u32;",
      "a_launch_can_give_part_of_the_fee_to_its_own_pool"),
    M("Q2", "The liquidity pot gets its weight of every split", "math.rs",
      "w[SINK_LIQUIDITY] = rest * liquidity_share as u128;", "", "a_launch_can_give_part_of_the_fee_to_its_own_pool"),
    M("Q3", "What the split gives the liquidity pot is added to it", "ops.rs",
      "ledger.liquidity.pot = ledger.liquidity.pot.checked_add(all[SINK_LIQUIDITY]).ok_or(BurrpadError::Overflow)?;", "",
      "a_launch_can_give_part_of_the_fee_to_its_own_pool"),
    M("Q4", "The liquidity pot's running total is kept", "ops.rs",
      "ledger.liquidity.credited = credited[SINK_LIQUIDITY];", "", "a_launch_can_give_part_of_the_fee_to_its_own_pool"),
    M("Q5", "A coin that did not choose the hook adds nothing", LIQ,
      "require!(ctx.accounts.coin.liquidity_share > 0, BurrpadError::HookNotChosen);", "",
      "coins_without_the_share_and_coins_on_the_curve_add_nothing"),
    M("Q6", "A liquidity call waits out the coin's burn wait", LIQ,
      "require!(now >= ready, BurrpadError::BurnTooSoon);", "", "the_coins_burn_and_its_liquidity_share_one_wait"),
    M("Q7", "The receipt mint passed is the one the pool's record names", LIQ,
      "require_keys_eq!(shares.lp_mint, ctx.accounts.lp_mint.key(), BurrpadError::WrongPumpAccount);", "",
      "t11_substitution::add_liquidity", "swapping lp_mint"),
    M("Q8", "The first call only records the pool's figures", LIQ,
      "if last.quote == 0 {", "if false {",
      "an_add_buys_deposits_both_sides_and_burns_every_receipt"),
    M("Q11", "The first call, which only records, still starts the wait", LIQ,
      "        ledger.burn.last_time = now;\n        emit!(LiquidityRecorded", "        emit!(LiquidityRecorded",
      "the_coins_burn_and_its_liquidity_share_one_wait"),
    M("Q12", "The buy's limit is measured against the smaller of the reserve now and the one recorded", LIQ,
      "reserves.quote.min(last.quote),", "reserves.quote,", "liquidity_added_just_for_one_call_does_not_raise_the_limit"),
    M("Q13", "A call uses at most twice the buy's limit", LIQ,
      "cap.checked_mul(2)", "cap.checked_mul(4)", "an_add_buys_deposits_both_sides_and_burns_every_receipt"),
    M("Q14", "The buy-back receiver is checked before the liquidity buy", LIQ,
      "pump::check_buyback_recipient(\n        &a.amm_global_config.to_account_info(),\n        &pump::PUMP_AMM_ID,\n        &pump::ACC_AMM_GLOBAL_CONFIG,\n        pump::AMM_GLOBAL_BUYBACK_OFFSET,\n        &a.buyback_fee_recipient.to_account_info(),\n        &ZEC_MINT,\n        &a.token_program.key(),\n    )?;",
      "", "t11_substitution::add_liquidity", "swapping buyback_fee_recipient"),
    M("Q15", "The pool may take no more of either side than its own figures ask", LIQ,
      "require!(zec_deposited <= want_zec && coins_deposited <= want_coins, BurrpadError::DepositOffRatio);", "",
      "an_add_buys_deposits_both_sides_and_burns_every_receipt"),
    M("Q16", "Every pool receipt is burned", LIQ,
      "        receipts,\n    )?;\n\n    // Booked", "        0,\n    )?;\n\n    // Booked",
      "an_add_buys_deposits_both_sides_and_burns_every_receipt"),
    M("Q17", "What was spent is taken off the liquidity pot", LIQ,
      "ledger.liquidity.pot = ledger.liquidity.pot.checked_sub(spent).ok_or(BurrpadError::Overflow)?;", "",
      "an_add_buys_deposits_both_sides_and_burns_every_receipt"),
    M("Q18", "What was spent is taken off what the vault must cover", LIQ,
      "ledger.accounted = ledger.accounted.checked_sub(spent).ok_or(BurrpadError::Overflow)?;", "",
      "an_add_buys_deposits_both_sides_and_burns_every_receipt"),
    M("Q19", "The recorded reserve moves a step at most per call", LIQ,
      "let quote = math::burr_next_reserve(last.quote, after.quote, step).ok_or(BurrpadError::Overflow)?;",
      "let quote = after.quote;", "liquidity_added_just_for_one_call_does_not_raise_the_limit"),
    M("Q20", "After an add the coin's burn waits too", LIQ,
      "ledger.burn.last_time = ledger.burn.last_time.max(held);", "let _ = held;",
      "the_coins_burn_and_its_liquidity_share_one_wait"),

    # ---- payout choices and the creator payout wallet ----
    M("W1", "Only payout choices that are defined may be recorded", LAUNCH,
      "require!(payout_options & !PAYOUT_DEFINED == 0, BurrpadError::PayoutOptionNotAllowed);", "",
      "choices_not_yet_defined_are_refused"),
    M("W2", "A private launch needs a confirmed payout wallet", LAUNCH,
      "require!(cfg.creator_payout_confirmed && payout_wallet != Pubkey::default(), BurrpadError::PrivatePayoutsOff);", "",
      "a_private_launch_needs_a_confirmed_wallet_a_creator_share_and_that_wallet_as_receiver"),
    M("W3", "A private launch needs a creator share", LAUNCH,
      "require!(args.shares[SINK_CREATOR - 1] > 0, BurrpadError::MissingReceiver);", "",
      "a_private_launch_needs_a_confirmed_wallet_a_creator_share_and_that_wallet_as_receiver"),
    M("W4", "A private launch's creator receiver is the payout wallet", LAUNCH,
      "require!(args.creator_receiver == payout_wallet, BurrpadError::WrongReceiver);", "",
      "a_private_launch_needs_a_confirmed_wallet_a_creator_share_and_that_wallet_as_receiver"),
    M("W5", "Without the choice, the payout wallet cannot be a creator receiver", LAUNCH,
      "require!(args.creator_receiver != payout_wallet, BurrpadError::ReceiverNotAllowed);", "",
      "the_payout_wallet_receives_nothing_but_private_creator_pots"),
    M("W6", "The payout wallet can never be a give receiver", LAUNCH,
      "payout_wallet == Pubkey::default() || args.give_receiver != payout_wallet,", "true,",
      "the_payout_wallet_receives_nothing_but_private_creator_pots"),
    M("W7", "The payout choices are recorded in the coin's record", LAUNCH,
      "    coin.payout_options = payout_options;\n", "",
      "a_private_launch_records_the_choice_and_its_creator_pot_goes_to_the_payout_wallet"),
    M("W14", "The holders' payout choices need a holders' share", LAUNCH,
      "payout_options & (PAYOUT_HOLDERS_IN_BURR | PAYOUT_LONG_HOLDER_BONUS) == 0 || args.shares[SINK_HOLDERS - 1] > 0,", "true,",
      "the_holders_choices_need_a_holders_share_and_a_sensible_bonus"),
    M("W15", "The long-holder bonus's step is at least 1, at most its most, and its most at most 300%", LAUNCH,
      "step >= 1 && step <= most && most <= LONG_HOLDER_BONUS_MAX_BPS", "true",
      "the_holders_choices_need_a_holders_share_and_a_sensible_bonus"),
    M("W16", "Without the bonus, no step or most is written", LAUNCH,
      "require!(step == 0 && most == 0, BurrpadError::BonusOutOfRange);", "",
      "the_holders_choices_need_a_holders_share_and_a_sensible_bonus"),
    M("W17", "The bonus's step is recorded in the coin's record", LAUNCH,
      "    coin.long_holder_step_bps = step;\n", "", "the_holders_choices_are_recorded_with_the_bonus_fixed_at_launch"),
    M("W18", "The bonus's most is recorded in the coin's record", LAUNCH,
      "    coin.long_holder_max_bps = most;\n", "", "the_holders_choices_are_recorded_with_the_bonus_fixed_at_launch"),
    M("W8", "Once confirmed, the payout wallet cannot be changed", ADMIN,
      "    require!(!c.creator_payout_confirmed, BurrpadError::PayoutWalletFixed);\n    require!(wallet != Pubkey::default(), BurrpadError::EmptyAddress);",
      "    require!(wallet != Pubkey::default(), BurrpadError::EmptyAddress);",
      "the_payout_wallet_is_fixed_once_it_confirms_itself"),
    M("W9", "The payout wallet is never also the platform receiver or a holders' wallet", ADMIN,
      "wallet != c.platform_receiver && wallet != c.rewards_wallet && wallet != c.pending_rewards_wallet", "true",
      "the_payout_wallet_is_fixed_once_it_confirms_itself"),
    M("W10", "Only the wallet named can confirm itself", ADMIN,
      "c.creator_payout_wallet != Pubkey::default() && c.creator_payout_wallet == ctx.accounts.wallet.key()", "true",
      "the_payout_wallet_is_fixed_once_it_confirms_itself"),
    M("W11", "A confirmed payout wallet cannot be confirmed again", ADMIN,
      "    require!(!c.creator_payout_confirmed, BurrpadError::PayoutWalletFixed);\n    require!(\n        c.creator_payout_wallet",
      "    require!(\n        c.creator_payout_wallet",
      "the_payout_wallet_is_fixed_once_it_confirms_itself"),
    M("W12", "The platform receiver cannot be set to the payout wallet", ADMIN,
      "require!(receiver != ctx.accounts.config.creator_payout_wallet, BurrpadError::ReceiverNotAllowed);", "",
      "the_payout_wallet_receives_nothing_but_private_creator_pots"),
    M("W13", "The holders' wallet cannot be proposed as the payout wallet", ADMIN,
      "require!(wallet != ctx.accounts.config.creator_payout_wallet, BurrpadError::ReceiverNotAllowed);", "",
      "the_payout_wallet_receives_nothing_but_private_creator_pots"),

    # ---- one-unit slips the random lives must catch (lane B's five, 15 Oct) ----
    M("S1", "A payout pays exactly the pot it empties, not one unit less", "ops.rs",
      "        amount,\n        decimals,\n    )\n}\n\n/// The accounts needed", "        amount - 1,\n        decimals,\n    )\n}\n\n/// The accounts needed",
      "t21_random_life::many_coins_live_any_order_and_the_books_close_after_every_step"),
    M("S2", "A split books exactly what arrived, not one unit more", "ops.rs",
      "ledger.accounted = ledger.accounted.checked_add(new).ok_or(BurrpadError::Overflow)?;",
      "ledger.accounted = ledger.accounted.checked_add(new + 1).ok_or(BurrpadError::Overflow)?;", "t21_random_life::many_coins_live_any_order_and_the_books_close_after_every_step"),
    M("S3", "A split moves nothing from one pot to another", "ops.rs",
      "    credits.copy_from_slice(&all[..SINKS]);\n",
      "    credits.copy_from_slice(&all[..SINKS]);\n    if credits[2] > 0 {\n        credits[2] -= 1;\n        credits[1] += 1;\n    }\n",
      "t21_random_life::many_coins_live_any_order_and_the_books_close_after_every_step"),
    M("S4", "A burn books exactly what it spent, not one unit more", "instructions/burn.rs",
      "    ops::debit(ledger, SINK_BURN, spent)?;\n    ledger.burn.total_spent = ledger.burn.total_spent.checked_add(spent).ok_or(BurrpadError::Overflow)?;",
      "    let spent = (spent + 1).min(ledger.pots[SINK_BURN]);\n    ops::debit(ledger, SINK_BURN, spent)?;\n    ledger.burn.total_spent = ledger.burn.total_spent.checked_add(spent).ok_or(BurrpadError::Overflow)?;",
      "t21_random_life::many_coins_live_any_order_and_the_books_close_after_every_step"),
    M("S5", "Each hook's weight is exactly its share: none of the give share goes to the creator", "math.rs",
      "    w[SINK_BURR] = rest * burr_share as u128;\n",
      "    w[SINK_BURR] = rest * burr_share as u128;\n    if shares[1] > 0 {\n        w[1] += rest;\n        w[2] -= rest;\n    }\n",
      "t21_random_life::many_coins_live_any_order_and_the_books_close_after_every_step"),

    # ---- shadow burn ----
    M("K1", "The shadow share counts in the whole the shares must add to", "math.rs",
      "let mut sum: u32 = burr_share as u32 + liquidity_share as u32 + shadow_share as u32;",
      "let mut sum: u32 = burr_share as u32 + liquidity_share as u32;", "t22_shadow_burn::a_launch_can_give_part_of_the_fee_to_a_shadow_burn"),
    M("K2", "The shadow pot gets its weight of every split", "math.rs",
      "w[SINK_SHADOW] = rest * shadow_share as u128;", "", "t22_shadow_burn::a_launch_can_give_part_of_the_fee_to_a_shadow_burn"),
    M("K3", "What the split gives the shadow pot is added to it", "ops.rs",
      "ledger.shadow.pot = ledger.shadow.pot.checked_add(all[SINK_SHADOW]).ok_or(BurrpadError::Overflow)?;", "",
      "t22_shadow_burn::a_launch_can_give_part_of_the_fee_to_a_shadow_burn"),
    M("K4", "The shadow pot's running total is kept", "ops.rs",
      "ledger.shadow.credited = credited[SINK_SHADOW];", "", "t22_shadow_burn::a_launch_can_give_part_of_the_fee_to_a_shadow_burn"),
    M("K5", "A watched wallet's account must be its own account for the target", SHADOW,
      "require_keys_eq!(account.key(), own, BurrpadError::WrongWatchedAccount);", "", T_SUBST),
    M("K6", "A watched account that does not exist counts as nothing held", SHADOW,
      "    if account.data_is_empty() {\n        return Ok(0);\n    }\n", "", "t22_shadow_burn::a_closed_target_account_counts_as_nothing_held"),
    M("K7", "A fall of the watched wallets' total since the last record is added to what is owed", SHADOW,
      "let fell = before.saturating_sub(held);", "let fell = 0;", T_SALE),
    M("K8", "Each record moves to the holdings now", SHADOW,
      "    w.held[..n].copy_from_slice(now);\n", "", "t22_shadow_burn::the_target_buying_back_in_moves_the_record_and_keeps_what_is_owed"),
    M("K9", "Only a coin that chose the shadow burn has a watch", SHADOW,
      "require!(ctx.accounts.coin.shadow_share > 0, BurrpadError::HookNotChosen);", "", T_LIST),
    M("K10", "A watch follows one to sixteen wallets", SHADOW,
      "require!(!wallets.is_empty() && wallets.len() <= SHADOW_MAX_WALLETS, BurrpadError::BadWatchList);", "", T_LIST),
    M("K11", "Watched wallets are real and not repeated", SHADOW,
      "require!(*a != Pubkey::default() && !wallets[..i].contains(a), BurrpadError::BadWatchList);", "", T_LIST),
    M("K12", "A change of wallets records the old ones' fall first", SHADOW,
      "    record(w, &now)?;\n    let new = holdings", "    let _ = now;\n    let new = holdings", T_CHANGE),
    M("K13", "The new wallets start from their holding now", SHADOW,
      "    w.held[..wallets.len()].copy_from_slice(&new);\n", "", T_CHANGE),
    M("K14", "The keeper adds only while the launcher allows it", SHADOW,
      "    require!(w.follow, BurrpadError::FollowingOff);\n", "", T_SIXTEEN),
    M("K15", "Only the watch's keeper may add", SHADOW,
      "    require_keys_eq!(x.keeper.key(), w.keeper, BurrpadError::NotTheKeeper);\n", "", T_FOLLOW),
    M("K16", "Following is off without a keeper", SHADOW,
      "w.follow = follow && keeper != Pubkey::default();", "w.follow = follow;", T_LIST),
    M("K17", "The keeper never adds past sixteen", SHADOW,
      "n < SHADOW_MAX_WALLETS && (from as usize) < n && !w.wallets[..n].contains(&wallet)",
      "(from as usize) < n && !w.wallets[..n].contains(&wallet)", T_SIXTEEN),
    M("K18", "The sender named must be a watched wallet", SHADOW,
      "n < SHADOW_MAX_WALLETS && (from as usize) < n && !w.wallets[..n].contains(&wallet)",
      "n < SHADOW_MAX_WALLETS && !w.wallets[..n].contains(&wallet)", T_FOLLOW),
    M("K19", "The keeper never adds a wallet already watched", SHADOW,
      "n < SHADOW_MAX_WALLETS && (from as usize) < n && !w.wallets[..n].contains(&wallet)",
      "n < SHADOW_MAX_WALLETS && (from as usize) < n", T_FOLLOW),
    M("K20", "An added wallet is a key that can sign, not a derived address", SHADOW,
      "can_sign(&wallet) && *x.wallet.owner", "*x.wallet.owner", T_FOLLOW),
    M("K21", "An added wallet is owned by the system program", SHADOW,
      "*x.wallet.owner == anchor_lang::system_program::ID && x.wallet.data_is_empty()", "x.wallet.data_is_empty()", T_FOLLOW),
    M("K22", "An added wallet holds no data", SHADOW,
      "*x.wallet.owner == anchor_lang::system_program::ID && x.wallet.data_is_empty()",
      "*x.wallet.owner == anchor_lang::system_program::ID", T_FOLLOW),
    M("K23", "An added wallet holds no more than the sender let go of since the last record", SHADOW,
      "added > 0 && added <= let_go", "added > 0", T_FOLLOW),
    M("K24", "An added wallet holds something", SHADOW,
      "added > 0 && added <= let_go", "added <= let_go", T_FOLLOW),
    M("K25", "The added wallet's holding moves off the sender's record", SHADOW,
      "    w.held[f] -= added;\n", "", T_FOLLOW),
    M("K26", "The added wallet starts from what it holds", SHADOW,
      "    w.held[n] = added;\n", "", T_FOLLOW),
    M("K27", "The launcher's wallets are keys that can sign, not derived addresses", SHADOW,
      "        require!(can_sign(a), BurrpadError::NotAPlainWallet);\n", "", T_LIST),
    M("K28", "Only the watch's keeper buys back", SHADOW,
      "require_keys_eq!(ctx.accounts.burr.payer.key(), w.keeper, BurrpadError::NotTheKeeper);", "",
      "t22_shadow_burn::only_the_keeper_buys_back"),
    M("K29", "A reported sale is held to the coins owed", SHADOW,
      "let coins = sold.min(owed);", "let coins = sold;", "t22_shadow_burn::a_report_of_more_coins_than_are_owed_pays_only_their_share"),
    M("K30", "The ZEC counted is the sale's share for the coins owed", SHADOW,
      "let paid = math::mul_div(received, coins, sold).ok_or", "let paid = Some(received).ok_or",
      "t22_shadow_burn::a_report_of_more_coins_than_are_owed_pays_only_their_share"),
    M("K31", "The ZEC counted is held to what buys those coins at the pool's price now", SHADOW,
      "    let paid = paid.min(math::spend_for_at_most(&reserves, coins).ok_or(BurrpadError::Overflow)?);\n", "", T_TRADE),
    M("K32", "The spend is the sale's ZEC, or the whole pot if smaller", SHADOW,
      "let spend = pot.min(paid);", "let spend = paid;", T_DUMP),
    M("K33", "The least-out allows 35% short, no more", SHADOW,
      "10_000 - SHADOW_SLIPPAGE_BPS", "1", "t22_shadow_burn::the_least_out_allows_35_percent_short_and_no_more"),
    M("K42", "The least-out's allowance is 35%", "constants.rs",
      "pub const SHADOW_SLIPPAGE_BPS: u64 = 3_500;", "pub const SHADOW_SLIPPAGE_BPS: u64 = 3_700;",
      "t22_shadow_burn::the_least_out_allows_35_percent_short_and_no_more"),
    M("K43", "The least-out asks no more than 35% short", "constants.rs",
      "pub const SHADOW_SLIPPAGE_BPS: u64 = 3_500;", "pub const SHADOW_SLIPPAGE_BPS: u64 = 3_300;",
      "t22_shadow_burn::the_least_out_allows_35_percent_short_and_no_more"),
    M("K34", "A part spend asks for its part of the coins", SHADOW,
      "math::mul_div(coins, spend, paid)", "Some(coins)", T_DUMP),
    M("K35", "What was spent is taken off the shadow pot", SHADOW,
      "ledger.shadow.pot = ledger.shadow.pot.checked_sub(done.spent).ok_or(BurrpadError::Overflow)?;", "", T_SALE),
    M("K36", "What was spent is taken off what the vault must cover", SHADOW,
      "    ledger.accounted = ledger.accounted.checked_sub(done.spent).ok_or(BurrpadError::Overflow)?;\n    let w = &mut ctx.accounts.watch;",
      "    let w = &mut ctx.accounts.watch;", T_SALE),
    M("K37", "What was bought comes off what is owed", SHADOW,
      "w.owed = w.owed.saturating_sub(done.bought);", "", T_SALE),
    M("K38", "The ZEC for at most the owed coins is worked out at the price before fees, rounded down", "math.rs",
      "    u64::try_from(q).ok()\n}\n\n/// `a × b", "    u64::try_from(q * 2).ok()\n}\n\n/// `a × b", T_TRADE),
    M("K39", "A shadow buy below the least spend is not tried", SHADOW,
      "if spend < BURR_MIN_SPEND || min_out == 0 {", "if min_out == 0 {", "t22_shadow_burn::a_crumb_let_go_of_is_recorded_but_too_small_to_buy"),
    M("K40", "A $BURR buy below the least spend is not tried", BURR,
      "if spend < BURR_MIN_SPEND || min_out == 0 {", "if min_out == 0 {", "a_burr_pot_below_the_least_spend_is_refused_cleanly"),
    M("K41", "Nothing sold or nothing received means only a record", SHADOW,
      "if sold == 0 || received == 0 || pot == 0 || owed == 0 {", "if pot == 0 || owed == 0 {", T_CHANGE),

    # ---- coins priced in $BURR ----
    M("Z1", "A launch's quote mint is the one its quote names", LAUNCH,
      "    require_keys_eq!(a.zec_mint.key(), quote_mint, BurrpadError::WrongQuote);\n", "",
      "t23_burr_pairs::a_launch_must_name_the_quote_and_its_accounts_as_they_are"),
    M("Z2", "A launch's quote token program is the one its quote names", LAUNCH,
      "    require_keys_eq!(a.token_program.key(), quote_program, BurrpadError::WrongQuote);\n", "",
      "t23_burr_pairs::a_launch_must_name_the_quote_and_its_accounts_as_they_are"),
    M("Z3", "Only a defined quote", LAUNCH,
      "let (quote_mint, quote_program, _) = quote_parts(quote).ok_or(BurrpadError::WrongQuote)?;",
      "let (quote_mint, quote_program, _) = quote_parts(quote).unwrap_or((ZEC_MINT, anchor_spl::token::ID, 8));",
      "t23_burr_pairs::a_launch_must_name_the_quote_and_its_accounts_as_they_are"),
    M("Z4", "A $BURR launch names $BURR's curve, pool and vaults, all four", LAUNCH,
      "        require!(rest.len() >= 4, BurrpadError::WrongPumpAccount);\n", "",
      "t23_burr_pairs::a_launch_must_name_the_quote_and_its_accounts_as_they_are"),
    M("Z5", "Each of $BURR's four accounts is the one derived", LAUNCH,
      "            require_keys_eq!(acc.key(), *want, BurrpadError::WrongPumpAccount);\n", "",
      "t23_burr_pairs::a_launch_must_name_the_quote_and_its_accounts_as_they_are"),
    M("Z6", "A $BURR coin chooses only the four hooks and the $BURR share", LAUNCH,
      "        require!(liquidity_share == 0 && shadow_share == 0 && payout_options == 0, BurrpadError::NotForThisQuote);\n", "",
      "t23_burr_pairs::a_burr_coin_cannot_choose_the_hooks_not_built_for_it_yet"),
    M("Z7", "The coin's record keeps its quote", LAUNCH,
      "    coin.quote = quote;\n", "", "t23_burr_pairs::a_coin_can_be_priced_in_burr_and_zec_coins_are_untouched"),
    M("Z8", "The ZEC buy of $BURR refuses a coin priced in $BURR", BURR,
      "    require!(ctx.accounts.coin.quote == QUOTE_ZEC, BurrpadError::NotForThisQuote);\n    require!(ctx.accounts.coin.burr_share > 0", "    require!(ctx.accounts.coin.burr_share > 0",
      "t23_burr_pairs::a_burr_coin_cannot_choose_the_hooks_not_built_for_it_yet"),
    M("Z9", "Liquidity refuses a coin priced in $BURR", LIQ,
      "    require!(ctx.accounts.coin.quote == QUOTE_ZEC, BurrpadError::NotForThisQuote);\n", "",
      "t23_burr_pairs::a_burr_coin_cannot_choose_the_hooks_not_built_for_it_yet"),
    M("Z10", "Shadow Burn refuses a coin priced in $BURR", SHADOW,
      "    require!(ctx.accounts.burr.coin.quote == QUOTE_ZEC, BurrpadError::NotForThisQuote);\n", "",
      "t23_burr_pairs::a_burr_coin_cannot_choose_the_hooks_not_built_for_it_yet"),
    M("Z11", "Only a coin priced in $BURR burns its $BURR pot as it is", BURR,
      "    require!(coin.quote == QUOTE_BURR, BurrpadError::NotForThisQuote);\n", "",
      "t23_burr_pairs::a_burr_coin_s_burr_share_is_burned_as_it_is"),
    M("Z12", "Only a coin that chose the $BURR share burns a $BURR pot", BURR,
      "    require!(coin.burr_share > 0, BurrpadError::HookNotChosen);\n    let pot", "    let pot",
      "t23_burr_pairs::a_burr_coin_s_burr_share_is_burned_as_it_is"),
    M("Z13", "Nothing to burn is refused", BURR,
      "    require!(pot > 0, BurrpadError::NothingToBurn);\n", "", "t23_burr_pairs::a_burr_coin_s_burr_share_is_burned_as_it_is"),
    M("Z14", "The burned pot is emptied", BURR,
      "    ledger.burr.pot = 0;\n", "", "t23_burr_pairs::a_burr_coin_s_burr_share_is_burned_as_it_is"),
    M("Z15", "The burned pot is taken off what the vault must cover", BURR,
      "    ledger.accounted = ledger.accounted.checked_sub(pot).ok_or(BurrpadError::Overflow)?;\n    ledger.burr.total_spent", "    ledger.burr.total_spent",
      "t23_burr_pairs::a_burr_coin_s_burr_share_is_burned_as_it_is"),
    M("Z16", "The burned pot is counted as burned", BURR,
      "    ledger.burr.total_burned = ledger.burr.total_burned.checked_add(pot).ok_or(BurrpadError::Overflow)?;\n", "",
      "t23_burr_pairs::a_burr_coin_s_burr_share_is_burned_as_it_is"),
    M("Z17", "Exactly the pot is burned", BURR,
      "        pot,\n    )?;\n    emit!(BurrBurned", "        pot - 1,\n    )?;\n    emit!(BurrBurned",
      "t23_burr_pairs::a_burr_coin_s_burr_share_is_burned_as_it_is"),
    M("Z18", "A payout goes to the receiver's account in the coin's own quote", "ops.rs",
      "let expected = get_associated_token_address_with_program_id(p.owner.key, p.zec_mint.key, p.token_program.key);",
      "let expected = get_associated_token_address_with_program_id(p.owner.key, &ZEC_MINT, &anchor_spl::token::ID);",
      "t23_burr_pairs::a_burr_coin_s_fee_is_collected_split_and_paid_in_burr"),
    M("Z19", "A payout moves the quote coin with its own decimals", PAY,
      "        amount,\n        a.coin.quote_decimals(),\n    )?;", "        amount,\n        ZEC_DECIMALS,\n    )?;",
      "t23_burr_pairs::a_burr_coin_s_fee_is_collected_split_and_paid_in_burr"),
    M("Z20", "A pool's record must name the coin's own quote", BURN,
      "    let quote_key = ctx.accounts.coin.quote_mint();", "    let quote_key = ZEC_MINT;",
      "t23_burr_pairs::a_graduated_burr_coin_collects_and_burns_on_its_burr_pool"),
    M("Z21", "A Token-2022 buy-back account must be shaped as an account", "pump.rs",
      "t.len() == 165 || (t.len() > 165 && t.len() != 355 && t[165] == 2)", "t.len() >= 165",
      "t23_burr_pairs::the_buy_back_receivers_account_must_be_shaped_as_its_program_makes_it"),
    M("Z22", "A classic token buy-back account is exactly 165 bytes", "pump.rs",
      "    } else {\n        t.len() == 165\n    };", "    } else {\n        t.len() >= 165\n    };",
      "t23_burr_pairs::the_buy_back_receivers_account_must_be_shaped_as_its_program_makes_it"),
]

# ---------------------------------------------------------------------------
# Protections on accounts, taken from the code itself: every constraint on
# every account of every instruction, and every program type.
# ---------------------------------------------------------------------------

STRUCT_TEST = {
    "Launch": "launch", "SplitOnly": "split", "CollectCurve": "collect_curve", "CollectPool": "collect_pool",
    "PayOut": "pay_out", "BurnCurve": "burn_curve", "BurnPool": "burn_pool", "BurnBurr": "burn_burr", "AddLiquidity": "add_liquidity",
    "SetShadowWatch": "t22_shadow_burn::only_the_launcher_sets_the_watch_and_only_sensible_lists",
    "FollowWallet": T_FOLLOW,
    "ShadowBurn": "t22_shadow_burn::shadow_burn_takes_only_this_coins_watch_pot_and_the_watched_accounts",
    "BurnBurrPot": "t23_burr_pairs::burn_burr_pot_takes_only_this_coins_record_ledger_vault_and_burr",
    "ReleaseHolderRewards": "release_holder_rewards", "Recover": "recover",
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
            test = STRUCT_TEST[struct] if "::" in STRUCT_TEST[struct] else f"t11_substitution::{STRUCT_TEST[struct]}"
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
                if (struct, field) == ("ReleaseHolderRewards", "rewards_wallet"):
                    continue  # held by hand as H1 and H2
                if "init" in parts or (struct, field) == ("Launch", "authority"):
                    continue  # the address is fixed by creation itself; see the list's notes
                if "init_if_needed" in parts:
                    if not any(p.startswith("associated_token::") for p in parts):
                        continue
                    new_attr = "#[account(mut)]"
                else:
                    new_attr = f"#[account({', '.join(keep)})]" if keep else ""
                old_attr = body[am.start():attr_end]
                a, b = body_start + am.start(), body_start + attr_end
                # A few are held by a sharper test than the general substitution one.
                third = "t23_burr_pairs::a_third_coin_in_the_place_of_the_quote_coin_is_refused_everywhere"
                swap = "t23_burr_pairs::a_burr_coin_s_accounts_cannot_be_swapped_for_zec_ones_or_the_other_way"
                test_for = {
                    ("PayOut", "vault"): "t15_random_sequences::the_books_close_after_every_step_of_any_sequence",
                    # The coin's quote mint: only it stops a third coin on the same token program.
                    ("SplitOnly", "zec_mint"): third,
                    ("PayOut", "zec_mint"): third,
                    ("ReleaseHolderRewards", "zec_mint"): third,
                    ("Recover", "zec_mint"): third,
                    # The coin's quote token program: swapped for the other quote coin's.
                    ("SplitOnly", "token_program"): swap,
                    ("PayOut", "token_program"): swap,
                }.get((struct, field), test)
                out.append(dict(
                    id=f"X.{struct}.{field}", what=f"{struct}: `{field}` must be the right account",
                    file=rel, span=(a, b), old=old_attr, repl=new_attr, test=test_for,
                    # Tests outside the substitution suite (named in full) do not print "swapping <field>".
                    msg=rf"swapping {field}( \(|\s+was)" if test_for == test and test.startswith("t11_") else None))
            # program types
            for pm in re.finditer(r"pub (\w+): Program<'info, \w+>,", body):
                field = pm.group(1)
                a, b = body_start + pm.start(), body_start + pm.end()
                out.append(dict(
                    id=f"Y.{struct}.{field}", what=f"{struct}: `{field}` must be the real program",
                    file=rel, span=(a, b), old=pm.group(0),
                    repl=f"/// CHECK: switched off\n    pub {field}: UncheckedAccount<'info>,", test=test,
                    msg=rf"swapping {field}( \(|\s+was)" if test.startswith("t11_") else None))
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
    code, out = run(["cargo", "build-sbf", "--manifest-path", "programs/burrpad/Cargo.toml"], w)
    if code != 0:
        return "does not build", out[-3000:]
    code, out = run(["cargo", "test", "-p", "burrpad-tests"], w)
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
def second_line(entry_id):
    """Why no test can see this check missing on its own: what else stops the same swap."""
    if entry_id == "X.FollowWallet.watch":
        return ("the watch passed is the one changed, and only that watch's own keeper may change it; the mint "
                "named beside it is not otherwise used")
    if entry_id == "Q7":
        return "the receipt mint is also pinned by its derived address from the pool's own address, which is checked"
    if entry_id == "Q15":
        return ("the deposit's own limits are what the burner holds, and PumpSwap takes exactly its own figures (pinned "
                "to the unit by t12's deposit test); this check stands in case PumpSwap ever changes")
    if entry_id == "U6":
        return ("every $BURR buy also writes the same time into $BURR's own ledger, which is checked too (U15, held "
                "alone); this one is kept so the shared record says when the next buy may run")
    if entry_id == "R12":
        return "the call's target is a constant in the code, not the account passed; and the account's address is pinned too"
    m = re.match(r"([XY])\.(\w+)\.(\w+)$", entry_id)
    if not m:
        return None
    kind, struct, field = m.groups()
    if kind == "Y":
        if field in ("token_program", "token_2022_program"):
            return "every token account and mint in the instruction is tied to its token program by its own check"
        if field == "system_program":
            return "accounts are created through the system program's fixed address"
        if field == "associated_token_program":
            return "a payment needs a real token account at the fixed address, which only the real program can make"
    if field in ("pump_program", "amm_program"):
        return "the call's target is a constant in the code, not the account passed"
    if field == "mint":
        return "a record exists only for mints this program launched, all Token-2022; any other mint has no record"
    if field == "zec_mint":
        return "the vault's own check ties it to the ZEC mint, and the token program refuses a transfer naming another mint"
    if field == "coin":
        return ("the record's own mint field is used to sign for the vault, so another coin's record cannot move this "
                "coin's money; the same address check is held alone in split and collect")
    if field == "authority":
        return ("signing uses this coin's own seeds, so another coin's authority cannot sign; "
                "the same check is held alone in split")
    if struct == "Recover" and field == "stray":
        return ("the token program refuses a transfer out of any account the coin's authority does not own, or of another "
                "mint, and the framework refuses the vault passed twice")
    if field == "vault":
        return "the token program refuses: the signer does not own that account"
    if field in ("pump_global", "amm_global_config"):
        return "the buy-back check reads this account and requires pump.fun's owner and its settings code"
    if struct in ("BurnPool", "BurnBurr", "AddLiquidity") and field in ("pool_base_token_account", "pool_quote_token_account"):
        return "the pool's own record names its two token accounts, and the burn compares them (held as B29 and B30)"
    if field == "pool_authority":
        return "the pool's own address is derived from it and checked"
    if field == "pool" and struct == "AddLiquidity":
        return "the pool's own record is read and must name this coin and ZEC, and it must name the receipt mint passed (Q7)"
    if struct == "AddLiquidity" and field == "lp_mint":
        return "the pool's own record must name the receipt mint passed (Q7)"
    if struct == "AddLiquidity" and field in ("burner_zec", "burner_coin", "burner_lp"):
        return ("the same constraint is what makes the account on first use, so without it every liquidity call fails "
                "(the tests that fail are the ones that add)")
    if field == "pool" and struct == "BurnPool":
        return "the pool's own record is read and must name this coin and ZEC"
    if field == "pool" and struct == "BurnBurr":
        return "the pool's own record is read and must name $BURR and ZEC"
    if struct == "BurnBurr" and field == "burner":
        return ("the buy is signed with this coin's own burner seeds, so any other account is not a signer; pump.fun "
                "refuses the buy and the whole call, the ZEC moved to it included, is undone")
    if struct == "BurnBurr" and field == "burr_mint":
        return ("$BURR's pool is found from the fixed $BURR address and its record must name $BURR; pump.fun refuses a "
                "buy naming another mint than its pool's, and the whole call is undone")
    if struct == "BurnBurr" and field in ("burner_zec", "burner_burr"):
        return ("the same constraint is what makes the account on first use, so without it every $BURR buy fails "
                "(the tests that fail are the ones that buy)")
    if field == "config":
        return "only `initialize` can make a Config, at one address; anything else fails to load as one"
    return None


SECOND_LINE = {}

GROUPS = [
    ("Z", "Pairs of checks that cover the same thing, removed together"),
    ("A", "The administrator"), ("L", "Launch"), ("N", "The coin's mint"), ("R", "Reading pump.fun's accounts"),
    ("C", "Collect and split"), ("P", "Payouts"), ("H", "Holders"), ("B", "Buy back and burn"), ("X", "Every account is the one it should be"), ("Y", "Every program is the real one"),
]


def write_report():
    results = json.loads((ROOT / "notes" / "protections.json").read_text())
    by_id = {r["id"]: r for r in results}
    for r in results:
        if r["status"] != "noticed" and second_line(r["id"]):
            SECOND_LINE[r["id"]] = second_line(r["id"])
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
        "**Second line** means the check was removed and no test failed, because another check stops the very same "
        "wrong account. The reason given is by reasoning about the code; it was not shown by a run. An auditor "
        "should treat each of these as a place where two checks overlap, and confirm it.",
        "",
        "Not in the table because they cannot be removed without removing the instruction itself: the addresses of "
        "`Config`, `Coin` and `Ledger` when they are first made (fixed by creation), the coin authority's "
        "address at launch (its bump is taken from the same check), and the kind of each "
        "program-owned account (`Coin`, `Ledger`, `Config` each carry their own 8-byte code, checked on every "
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
        allr = [r for r in merged.values() if r["id"] in order]
        allr.sort(key=lambda r: order[r["id"]])
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
