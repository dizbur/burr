//! The frame: initialisation, the administrator, launch, the coin's record.

use crate::bench::{custom_code, logs};
use crate::kit::{self, err, LaunchPlan, World};
use crate::pump::{self, ZEC};
use burr::error::BurrError as E;
use solana_signer::Signer;

#[test]
fn launch_creates_the_coin_and_its_record_together() {
    let mut w = World::new();
    let plan = LaunchPlan::default();
    let mint = w.launch(&plan);

    let curve = w.b.curve(&mint);
    assert_eq!(curve.creator, kit::authority(&mint));
    assert_eq!(curve.quote_mint, ZEC);
    assert_eq!(curve.creator_fee_bps, 200);
    assert!(!curve.is_holder_reward);

    let c = w.coin(&mint);
    assert_eq!(c.mint, mint);
    assert_eq!(c.fee_bps, 200);
    assert_eq!(c.platform_fraction_bps, 1_250);
    assert_eq!(c.platform_receiver, w.platform_receiver);
    assert_eq!(c.shares, plan.shares);
    assert_eq!(c.creator_receiver, plan.creator_receiver);
    assert_eq!(c.give_receiver, plan.give_receiver);
    assert_eq!(c.king_round_secs, 3_600);
    assert_eq!(c.launcher, w.b.payer.pubkey());

    let l = w.ledger(&mint);
    assert_eq!(l.mint, mint);
    assert_eq!(l.accounted, 0);
    assert!(w.b.account(&kit::vault(&mint)).is_some());
    w.assert_sound(&mint);
    let _ = pump::PUMP;
}

#[test]
fn launch_fails_cleanly_when_refused() {
    let mut w = World::new();
    let (mint, r) = w.try_launch(&LaunchPlan { fee_bps: 301, ..Default::default() });
    assert_eq!(custom_code(&r), Some(err(E::FeeOutOfRange)), "{}", logs(&r));
    // Nothing was left behind: no record, no coin on pump.fun.
    assert!(w.b.account(&kit::coin(&mint)).is_none());
    assert!(w.b.account(&pump::curve(&mint)).is_none());
    assert!(w.b.account(&mint).is_none());
}

// ---- initialisation ----

#[test]
fn only_the_upgrade_key_can_initialise_and_only_once() {
    let mut w = World::bare();
    // A stranger tries first.
    let stranger = w.b.wallet();
    let i = w.initialize_ix(&stranger.pubkey(), stranger.pubkey(), stranger.pubkey());
    let r = w.b.send_as(Some(&stranger), &[i], &[]);
    assert_eq!(custom_code(&r), Some(err(E::NotUpgradeAuthority)), "{}", logs(&r));
    // The upgrade key succeeds.
    assert!(w.initialize().is_ok());
    let c = w.config();
    assert_eq!(c.admin, w.admin.pubkey());
    assert_eq!((c.min_fee_bps, c.max_fee_bps, c.default_fee_bps), (50, 300, 200));
    assert_eq!(c.platform_fraction_bps, 1_250);
    assert!(!c.launches_paused);
    // A second time fails: the account exists.
    assert!(w.initialize().is_err());
}

#[test]
fn initialise_refuses_empty_addresses() {
    let mut w = World::bare();
    let k = w.upgrade_key.insecure_clone();
    let i = w.initialize_ix(&k.pubkey(), solana_pubkey::Pubkey::default(), w.platform_receiver);
    let r = w.b.send_as(Some(&k), &[i], &[]);
    assert_eq!(custom_code(&r), Some(err(E::EmptyAddress)), "empty administrator");
    let i = w.initialize_ix(&k.pubkey(), w.admin.pubkey(), solana_pubkey::Pubkey::default());
    let r = w.b.send_as(Some(&k), &[i], &[]);
    assert_eq!(custom_code(&r), Some(err(E::EmptyAddress)), "empty platform receiver");
}

#[test]
fn naming_a_key_is_not_signing_with_it() {
    // The right address is passed each time, but its owner has not signed.
    let mut w = World::bare();
    let s = w.b.wallet();
    let mut i = w.initialize_ix(&w.upgrade_key.pubkey(), s.pubkey(), s.pubkey());
    i.accounts[0].is_signer = false;
    assert!(w.b.send_as(Some(&s), &[i], &[]).is_err(), "the upgrade key's address without its signature");
    assert!(w.b.account(&kit::config()).is_none());

    let mut w = World::new();
    let s = w.b.wallet();
    let mut i = w.admin_ix(&w.admin.pubkey(), burr::instruction::SetLaunchesPaused { paused: true });
    i.accounts[0].is_signer = false;
    assert!(w.b.send_as(Some(&s), &[i], &[]).is_err(), "the administrator's address without its signature");
    assert!(!w.config().launches_paused);

    let next = w.b.wallet();
    assert!(w.as_admin(burr::instruction::ProposeAdmin { new_admin: next.pubkey() }).is_ok());
    let mut i = kit::raw(
        burr::accounts::AcceptAdmin { new_admin: next.pubkey(), config: kit::config() },
        burr::instruction::AcceptAdmin {},
    );
    i.accounts[0].is_signer = false;
    assert!(w.b.send_as(Some(&s), &[i], &[]).is_err(), "the proposed administrator's address without its signature");
    assert_eq!(w.config().admin, w.admin.pubkey());
}

// ---- the administrator ----

#[test]
fn a_stranger_cannot_use_any_admin_instruction() {
    let mut w = World::new();
    let s = w.b.wallet();
    let tries: Vec<solana_instruction::Instruction> = vec![
        w.admin_ix(&s.pubkey(), burr::instruction::SetPlatformFraction { fraction_bps: 0 }),
        w.admin_ix(&s.pubkey(), burr::instruction::SetFeeRange { min_bps: 50, max_bps: 300, default_bps: 200 }),
        w.admin_ix(&s.pubkey(), burr::instruction::SetPlatformReceiver { receiver: s.pubkey() }),
        w.admin_ix(&s.pubkey(), burr::instruction::SetLaunchesPaused { paused: true }),
        w.admin_ix(&s.pubkey(), burr::instruction::ProposeAdmin { new_admin: s.pubkey() }),
    ];
    for i in tries {
        let r = w.b.send_as(Some(&s), &[i], &[]);
        assert_eq!(custom_code(&r), Some(err(E::NotAdmin)), "{}", logs(&r));
    }
}

#[test]
fn admin_settings_stay_inside_the_hard_limits() {
    let mut w = World::new();
    let r = w.as_admin(burr::instruction::SetPlatformFraction { fraction_bps: 5_001 });
    assert_eq!(custom_code(&r), Some(err(E::PlatformFractionTooHigh)));
    assert!(w.as_admin(burr::instruction::SetPlatformFraction { fraction_bps: 5_000 }).is_ok());

    for (min, max, def) in [(9, 300, 200), (50, 501, 200), (50, 300, 301), (50, 300, 49), (301, 300, 300)] {
        let r = w.as_admin(burr::instruction::SetFeeRange { min_bps: min, max_bps: max, default_bps: def });
        assert_eq!(custom_code(&r), Some(err(E::FeeSettingOutOfRange)), "{min} {max} {def}");
    }
    assert!(w.as_admin(burr::instruction::SetFeeRange { min_bps: 10, max_bps: 500, default_bps: 10 }).is_ok());

    let r = w.as_admin(burr::instruction::SetPlatformReceiver { receiver: solana_pubkey::Pubkey::default() });
    assert_eq!(custom_code(&r), Some(err(E::EmptyAddress)));
}

#[test]
fn admin_changes_touch_only_later_launches() {
    let mut w = World::new();
    let first = w.launch(&LaunchPlan::default());
    let new_receiver = solana_pubkey::Pubkey::new_unique();
    assert!(w.as_admin(burr::instruction::SetPlatformFraction { fraction_bps: 2_500 }).is_ok());
    assert!(w.as_admin(burr::instruction::SetPlatformReceiver { receiver: new_receiver }).is_ok());
    let second = w.launch(&LaunchPlan::default());

    let a = w.coin(&first);
    assert_eq!((a.platform_fraction_bps, a.platform_receiver), (1_250, w.platform_receiver));
    let b = w.coin(&second);
    assert_eq!((b.platform_fraction_bps, b.platform_receiver), (2_500, new_receiver));
}

#[test]
fn pausing_stops_new_launches_only() {
    let mut w = World::new();
    assert!(w.as_admin(burr::instruction::SetLaunchesPaused { paused: true }).is_ok());
    let (_, r) = w.try_launch(&LaunchPlan::default());
    assert_eq!(custom_code(&r), Some(err(E::LaunchesPaused)));
    assert!(w.as_admin(burr::instruction::SetLaunchesPaused { paused: false }).is_ok());
    w.launch(&LaunchPlan::default());
}

#[test]
fn the_admin_role_moves_in_two_steps() {
    let mut w = World::new();
    let next = w.b.wallet();
    let other = w.b.wallet();
    assert!(w.as_admin(burr::instruction::ProposeAdmin { new_admin: next.pubkey() }).is_ok());
    assert_eq!(w.config().admin, w.admin.pubkey(), "nothing changes until accepted");

    let accept = |who: &solana_pubkey::Pubkey| {
        kit::raw(
            burr::accounts::AcceptAdmin { new_admin: *who, config: kit::config() },
            burr::instruction::AcceptAdmin {},
        )
    };
    let r = w.b.send_as(Some(&other), &[accept(&other.pubkey())], &[]);
    assert_eq!(custom_code(&r), Some(err(E::NotPendingAdmin)));
    let r = w.b.send_as(Some(&next), &[accept(&next.pubkey())], &[]);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.config().admin, next.pubkey());
    // The old administrator has no power left.
    let r = w.as_admin(burr::instruction::SetLaunchesPaused { paused: true });
    assert_eq!(custom_code(&r), Some(err(E::NotAdmin)));
}

// ---- launch refusals ----

#[test]
fn launch_refuses_bad_plans() {
    let mut w = World::new();
    let d = LaunchPlan::default();
    let empty = solana_pubkey::Pubkey::default();
    let cases: Vec<(LaunchPlan, E)> = vec![
        (LaunchPlan { fee_bps: 49, ..d.clone() }, E::FeeOutOfRange),
        (LaunchPlan { fee_bps: 301, ..d.clone() }, E::FeeOutOfRange),
        (LaunchPlan { shares: [2_000, 2_000, 2_000, 2_000, 1_999], ..d.clone() }, E::SharesDoNotAddUp),
        (LaunchPlan { shares: [2_000, 2_000, 2_000, 2_000, 2_001], ..d.clone() }, E::SharesDoNotAddUp),
        (LaunchPlan { shares: [0; 5], ..d.clone() }, E::SharesDoNotAddUp),
        (LaunchPlan { shares: [u16::MAX, u16::MAX, u16::MAX, u16::MAX, u16::MAX], ..d.clone() }, E::SharesDoNotAddUp),
        (LaunchPlan { creator_receiver: empty, ..d.clone() }, E::MissingReceiver),
        (LaunchPlan { give_receiver: empty, ..d.clone() }, E::MissingReceiver),
        (LaunchPlan { king_round_secs: 299, ..d.clone() }, E::RoundLengthOutOfRange),
        (LaunchPlan { king_round_secs: 604_801, ..d.clone() }, E::RoundLengthOutOfRange),
        (LaunchPlan { name: "x".repeat(33), ..d.clone() }, E::TextTooLong),
    ];
    for (plan, want) in cases {
        let (_, r) = w.try_launch(&plan);
        assert_eq!(custom_code(&r), Some(err(want)), "{want:?}\n{}", logs(&r));
    }
}

#[test]
fn launch_accepts_the_edges_and_unused_settings() {
    let mut w = World::new();
    let empty = solana_pubkey::Pubkey::default();
    // Lowest and highest fee.
    w.launch(&LaunchPlan { fee_bps: 50, ..Default::default() });
    w.launch(&LaunchPlan { fee_bps: 300, ..Default::default() });
    // A hook that is not chosen needs no setting.
    w.launch(&LaunchPlan {
        shares: [0, 0, 10_000, 0, 0],
        creator_receiver: empty,
        give_receiver: empty,
        king_round_secs: 0,
        ..Default::default()
    });
}

#[test]
fn a_fee_above_what_pump_fun_takes_fails_cleanly() {
    let mut w = World::new();
    // The administrator may set up to 5.00%; pump.fun takes at most 3.00% today.
    assert!(w.as_admin(burr::instruction::SetFeeRange { min_bps: 50, max_bps: 500, default_bps: 200 }).is_ok());
    let (mint, r) = w.try_launch(&LaunchPlan { fee_bps: 400, ..Default::default() });
    assert_eq!(custom_code(&r), Some(6078), "pump.fun's CreatorFeeBpsOutOfRange\n{}", logs(&r));
    assert!(w.b.account(&kit::coin(&mint)).is_none());
}

#[test]
fn a_coin_cannot_be_launched_twice_or_adopted() {
    let mut w = World::new();
    // A coin made on pump.fun directly, with our authority as creator even.
    let mint = solana_keypair::Keypair::new();
    let payer = w.b.payer.pubkey();
    let direct = pump::create_v2(&mint.pubkey(), &payer, &kit::authority(&mint.pubkey()), 200);
    assert!(w.b.send(&[direct], &[&mint]).is_ok());
    // It can never get a record.
    let i = w.launch_ix(&payer, &mint.pubkey(), &LaunchPlan::default());
    let r = w.b.send(&[i], &[&mint]);
    assert!(r.is_err());
    assert!(w.b.account(&kit::coin(&mint.pubkey())).is_none());

    // And a launched coin cannot be launched again.
    let mint2 = solana_keypair::Keypair::new();
    let i = w.launch_ix(&payer, &mint2.pubkey(), &LaunchPlan::default());
    assert!(w.b.send(std::slice::from_ref(&i), &[&mint2]).is_ok());
    assert!(w.b.send(&[i], &[&mint2]).is_err());
}

#[test]
fn launch_refuses_substituted_pump_accounts() {
    let mut w = World::new();
    let payer = w.b.payer.pubkey();
    let fake = solana_pubkey::Pubkey::new_unique();
    // Each closure swaps one account for another address.
    type Swap = fn(&mut burr::accounts::Launch, solana_pubkey::Pubkey);
    let swaps: Vec<(&str, Swap)> = vec![
        ("pump_program", |a, f| a.pump_program = f),
        ("pump_global", |a, f| a.pump_global = f),
        ("bonding_curve", |a, f| a.bonding_curve = f),
        ("associated_bonding_curve", |a, f| a.associated_bonding_curve = f),
        ("associated_quote_bonding_curve", |a, f| a.associated_quote_bonding_curve = f),
        ("quote_control", |a, f| a.quote_control = f),
        ("pump_mint_authority", |a, f| a.pump_mint_authority = f),
        ("pump_event_authority", |a, f| a.pump_event_authority = f),
        ("mayhem_program", |a, f| a.mayhem_program = f),
        ("mayhem_global_params", |a, f| a.mayhem_global_params = f),
        ("mayhem_sol_vault", |a, f| a.mayhem_sol_vault = f),
        ("mayhem_state", |a, f| a.mayhem_state = f),
        ("mayhem_token_vault", |a, f| a.mayhem_token_vault = f),
        ("authority", |a, f| a.authority = f),
        ("vault", |a, f| a.vault = f),
        ("zec_mint", |a, f| a.zec_mint = f),
        ("config", |a, f| a.config = f),
        ("coin", |a, f| a.coin = f),
        ("ledger", |a, f| a.ledger = f),
        ("token_program", |a, f| a.token_program = f),
        ("token_2022_program", |a, f| a.token_2022_program = f),
    ];
    for (name, swap) in swaps {
        let mint = solana_keypair::Keypair::new();
        let mut accounts = w.launch_accounts(&payer, &mint.pubkey());
        swap(&mut accounts, fake);
        let good = w.launch_ix(&payer, &mint.pubkey(), &LaunchPlan::default());
        let i = solana_instruction::Instruction {
            program_id: kit::BURR,
            accounts: anchor_lang::ToAccountMetas::to_account_metas(&accounts, None),
            data: good.data,
        };
        let r = w.b.send(&[i], &[&mint]);
        assert!(r.is_err(), "swapping {name} was accepted");
        assert!(w.b.account(&kit::coin(&mint.pubkey())).is_none(), "{name}");
    }
    // Swapping the two token programs with each other is refused too.
    let mint = solana_keypair::Keypair::new();
    let mut accounts = w.launch_accounts(&payer, &mint.pubkey());
    std::mem::swap(&mut accounts.token_program, &mut accounts.token_2022_program);
    let good = w.launch_ix(&payer, &mint.pubkey(), &LaunchPlan::default());
    let i = solana_instruction::Instruction {
        program_id: kit::BURR,
        accounts: anchor_lang::ToAccountMetas::to_account_metas(&accounts, None),
        data: good.data,
    };
    assert!(w.b.send(&[i], &[&mint]).is_err());
}

#[test]
fn a_vault_made_early_by_someone_else_is_still_ours() {
    let mut w = World::new();
    let mint = solana_keypair::Keypair::new();
    let payer = w.b.payer.pubkey();
    // Someone sees the launch coming, makes the vault first and puts ZEC in it.
    let early = pump::create_ata(&payer, &kit::authority(&mint.pubkey()), &ZEC, &pump::TOKEN);
    assert!(w.b.send(&[early], &[]).is_ok());
    w.b.give_zec(&kit::authority(&mint.pubkey()), 1_000);
    let i = w.launch_ix(&payer, &mint.pubkey(), &LaunchPlan::default());
    let r = w.b.send(&[i], &[&mint]);
    assert!(r.is_ok(), "{}", logs(&r));
    // What was in it simply counts as fee.
    assert!(w.split(&mint.pubkey()).is_ok());
    assert_eq!(w.ledger(&mint.pubkey()).accounted, 1_000);
    w.assert_sound(&mint.pubkey());
}

#[test]
fn initialise_cannot_be_passed_another_programs_upgrade_record() {
    let mut w = World::bare();
    // The attacker really is the upgrade key of some other program, and
    // passes that program's record in place of Burr's.
    let attacker = w.b.wallet();
    let loader = kit::LOADER;
    let other_program = solana_pubkey::Pubkey::new_unique();
    let other_data = solana_pubkey::Pubkey::find_program_address(&[other_program.as_ref()], &loader).0;
    let mut data = vec![3, 0, 0, 0];
    data.extend_from_slice(&0u64.to_le_bytes());
    data.push(1);
    data.extend_from_slice(attacker.pubkey().as_ref());
    w.b.svm
        .set_account(
            other_data,
            solana_account::Account {
                lamports: 10_000_000,
                data,
                owner: loader,
                executable: false,
                rent_epoch: u64::MAX,
            },
        )
        .unwrap();
    let mut i = w.initialize_ix(&attacker.pubkey(), attacker.pubkey(), attacker.pubkey());
    i.accounts[3].pubkey = other_data;
    let r = w.b.send_as(Some(&attacker), &[i], &[]);
    assert_eq!(custom_code(&r), Some(err(E::NotUpgradeAuthority)), "{}", logs(&r));
    assert!(w.b.account(&kit::config()).is_none());
}
