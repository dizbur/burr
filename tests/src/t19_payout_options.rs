//! Payout choices made at launch and recorded in the coin's own record. The
//! first: private creator payouts, where the creator pot goes to Burrpad's one
//! creator payout wallet (fixed in Config once it confirms itself), which
//! Burrpad's server forwards to the creator's Zcash address.

use crate::bench::{custom_code, logs};
use crate::kit::{self, err, LaunchPlan, World};
use burrpad::error::BurrpadError as E;
use burrpad::state::Payee;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

const ZEC1: u64 = 100_000_000;

fn private(wallet: &Pubkey) -> LaunchPlan {
    LaunchPlan { payout_options: 1, creator_receiver: *wallet, ..Default::default() }
}

#[test]
fn a_private_launch_records_the_choice_and_its_creator_pot_goes_to_the_payout_wallet() {
    let mut w = World::new();
    let wallet = w.open_private_payouts().pubkey();
    let mint = w.launch(&private(&wallet));
    let c = w.coin(&mint);
    assert_eq!((c.payout_options, c.creator_receiver), (1, wallet));
    w.b.give_zec(&kit::authority(&mint), 10 * ZEC1);
    assert!(w.split(&mint).is_ok());
    let pot = w.ledger(&mint).pots[1];
    assert!(pot > 0);
    let r = w.pay_out(&mint, Payee::Creator);
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(w.zec_of(&wallet), pot, "the creator pot reached the payout wallet");
}

#[test]
fn a_private_launch_needs_a_confirmed_wallet_a_creator_share_and_that_wallet_as_receiver() {
    let mut w = World::new();
    let stranger = Pubkey::new_unique();
    let (_, r) = w.try_launch(&private(&stranger));
    assert_eq!(custom_code(&r), Some(err(E::PrivatePayoutsOff)), "no wallet set");
    let wallet = w.b.wallet();
    assert!(w.as_admin(burrpad::instruction::SetCreatorPayoutWallet { wallet: wallet.pubkey() }).is_ok());
    let (_, r) = w.try_launch(&private(&wallet.pubkey()));
    assert_eq!(custom_code(&r), Some(err(E::PrivatePayoutsOff)), "set but not confirmed");
    assert!(w.confirm_payout_wallet(&wallet).is_ok());
    let (_, r) = w.try_launch(&private(&stranger));
    assert_eq!(custom_code(&r), Some(err(E::WrongReceiver)), "the launcher's own receiver with the choice");
    let (_, r) = w.try_launch(&LaunchPlan { shares: [0, 5_000, 2_500, 2_500], ..private(&wallet.pubkey()) });
    assert_eq!(custom_code(&r), Some(err(E::MissingReceiver)), "the choice with no creator share");
    let (_, r) = w.try_launch(&private(&wallet.pubkey()));
    assert!(r.is_ok(), "{}", logs(&r));
}

#[test]
fn choices_not_yet_defined_are_refused() {
    let mut w = World::new();
    let wallet = w.open_private_payouts().pubkey();
    for bits in [8u8, 0x10, 0x40, 0x80, 0x0f, 0xff] {
        let (_, r) = w.try_launch(&LaunchPlan { payout_options: bits, ..private(&wallet) });
        assert_eq!(custom_code(&r), Some(err(E::PayoutOptionNotAllowed)), "bits {bits:#x}");
    }
}

#[test]
fn the_payout_wallet_receives_nothing_but_private_creator_pots() {
    let mut w = World::new();
    let wallet = w.open_private_payouts().pubkey();
    // As creator receiver without the choice, through every launch.
    for burr_share in [0u16, 2_000] {
        let shares = if burr_share > 0 { [2_000; 4] } else { [2_500; 4] };
        let (_, r) = w.try_launch(&LaunchPlan { creator_receiver: wallet, shares, burr_share, ..Default::default() });
        assert_eq!(custom_code(&r), Some(err(E::ReceiverNotAllowed)), "burr share {burr_share}");
    }
    let (_, r) = w.try_launch(&LaunchPlan {
        creator_receiver: wallet,
        shares: [2_000; 4],
        liquidity_share: 2_000,
        ..Default::default()
    });
    assert_eq!(custom_code(&r), Some(err(E::ReceiverNotAllowed)), "launch_v3 without the choice");
    // As give receiver, with the choice or without.
    let (_, r) = w.try_launch(&LaunchPlan { give_receiver: wallet, ..Default::default() });
    assert_eq!(custom_code(&r), Some(err(E::ReceiverNotAllowed)), "give, without the choice");
    let (_, r) = w.try_launch(&LaunchPlan { give_receiver: wallet, ..private(&wallet) });
    assert_eq!(custom_code(&r), Some(err(E::ReceiverNotAllowed)), "give, with the choice");
    // Nor can it become the platform receiver or the holders' wallet.
    assert_eq!(
        custom_code(&w.as_admin(burrpad::instruction::SetPlatformReceiver { receiver: wallet })),
        Some(err(E::ReceiverNotAllowed))
    );
    assert_eq!(
        custom_code(&w.as_admin(burrpad::instruction::ProposeRewardsWallet { wallet })),
        Some(err(E::ReceiverNotAllowed))
    );
}

#[test]
fn the_payout_wallet_is_fixed_once_it_confirms_itself() {
    let mut w = World::new();
    let cfg = w.config();
    for bad in [Pubkey::default(), cfg.platform_receiver, cfg.rewards_wallet] {
        let r = w.as_admin(burrpad::instruction::SetCreatorPayoutWallet { wallet: bad });
        assert!(r.is_err(), "{bad} was accepted");
    }
    // Before it confirms, the administrator may correct it, and only the wallet named can confirm.
    let (first, second) = (w.b.wallet(), w.b.wallet());
    assert!(w.as_admin(burrpad::instruction::SetCreatorPayoutWallet { wallet: first.pubkey() }).is_ok());
    assert!(w.as_admin(burrpad::instruction::SetCreatorPayoutWallet { wallet: second.pubkey() }).is_ok());
    assert_eq!(custom_code(&w.confirm_payout_wallet(&first)), Some(err(E::NotThePayoutWallet)), "the replaced one");
    let stranger = w.b.wallet();
    assert_eq!(custom_code(&w.confirm_payout_wallet(&stranger)), Some(err(E::NotThePayoutWallet)));
    // A stranger cannot set it either.
    let i = kit::raw(
        burrpad::accounts::AdminOnly { admin: stranger.pubkey(), config: kit::config() },
        burrpad::instruction::SetCreatorPayoutWallet { wallet: stranger.pubkey() },
    );
    assert_eq!(custom_code(&w.b.send_as(Some(&stranger), &[i], &[])), Some(err(E::NotAdmin)));
    assert!(w.confirm_payout_wallet(&second).is_ok());
    let c = w.config();
    assert_eq!((c.creator_payout_wallet, c.creator_payout_confirmed), (second.pubkey(), true));
    // Fixed for good.
    let r = w.as_admin(burrpad::instruction::SetCreatorPayoutWallet { wallet: first.pubkey() });
    assert_eq!(custom_code(&r), Some(err(E::PayoutWalletFixed)));
    assert_eq!(custom_code(&w.confirm_payout_wallet(&second)), Some(err(E::PayoutWalletFixed)));
    assert_eq!(w.config().creator_payout_wallet, second.pubkey());
}

#[test]
fn coins_launched_before_keep_their_receiver_and_no_choices() {
    let mut w = World::new();
    let old = w.launch(&LaunchPlan::default());
    let receiver = w.coin(&old).creator_receiver;
    let wallet = w.open_private_payouts().pubkey();
    assert_eq!(w.coin(&old).payout_options, 0);
    w.b.give_zec(&kit::authority(&old), 10 * ZEC1);
    assert!(w.split(&old).is_ok());
    let pot = w.ledger(&old).pots[1];
    assert!(w.pay_out(&old, Payee::Creator).is_ok());
    assert_eq!(w.zec_of(&receiver), pot, "paid to its own receiver, as before");
    assert_eq!(w.zec_of(&wallet), 0);
    // And plain launches go on as before.
    let m = w.launch(&LaunchPlan::default());
    assert_eq!(w.coin(&m).payout_options, 0);
}

#[test]
fn the_holders_choices_are_recorded_with_the_bonus_fixed_at_launch() {
    let mut w = World::new();
    // Holders paid in $BURR: recorded, nothing else.
    let m = w.launch(&LaunchPlan { payout_options: 2, ..Default::default() });
    let c = w.coin(&m);
    assert_eq!((c.payout_options, c.long_holder_step_bps, c.long_holder_max_bps), (2, 0, 0));
    // The long-holder bonus: its step and most are the coin's own, for good.
    let m = w.launch(&LaunchPlan {
        payout_options: 4,
        long_holder_step_bps: 100,
        long_holder_max_bps: 10_000,
        ..Default::default()
    });
    let c = w.coin(&m);
    assert_eq!((c.payout_options, c.long_holder_step_bps, c.long_holder_max_bps), (4, 100, 10_000));
    // All three choices together, at the edges allowed.
    let wallet = w.open_private_payouts().pubkey();
    let m = w.launch(&LaunchPlan {
        payout_options: 7,
        creator_receiver: wallet,
        long_holder_step_bps: 30_000,
        long_holder_max_bps: 30_000,
        ..Default::default()
    });
    assert_eq!(w.coin(&m).payout_options, 7);
}

#[test]
fn the_holders_choices_need_a_holders_share_and_a_sensible_bonus() {
    let mut w = World::new();
    let no_holders = [2_500u16, 2_500, 5_000, 0];
    for bits in [2u8, 4] {
        let (_, r) = w.try_launch(&LaunchPlan {
            shares: no_holders,
            payout_options: bits,
            long_holder_step_bps: if bits == 4 { 100 } else { 0 },
            long_holder_max_bps: if bits == 4 { 1_000 } else { 0 },
            ..Default::default()
        });
        assert_eq!(custom_code(&r), Some(err(E::HookNotChosen)), "bit {bits} with no holders' share");
    }
    // With the bonus: a step of at least 1, no more than the most, and the most at most 300%.
    for (step, most) in [(0u16, 1_000u16), (2_000, 1_000), (100, 30_001), (0, 0)] {
        let (_, r) = w.try_launch(&LaunchPlan {
            payout_options: 4,
            long_holder_step_bps: step,
            long_holder_max_bps: most,
            ..Default::default()
        });
        assert_eq!(custom_code(&r), Some(err(E::BonusOutOfRange)), "step {step}, most {most}");
    }
    // Without it, no step or most may be written.
    for (step, most) in [(100u16, 0u16), (0, 1_000), (100, 1_000)] {
        let (_, r) = w.try_launch(&LaunchPlan {
            payout_options: 2,
            long_holder_step_bps: step,
            long_holder_max_bps: most,
            ..Default::default()
        });
        assert_eq!(custom_code(&r), Some(err(E::BonusOutOfRange)), "without the bonus: step {step}, most {most}");
    }
}
