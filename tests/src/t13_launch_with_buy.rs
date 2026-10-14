//! Launch and the launcher's first buy in one transaction, the way a site
//! would send it: a versioned transaction with an address table, and a
//! picture link of realistic length.

use crate::bench::{custom_code, logs, MAX_TRANSACTION};
use crate::kit::{self, LaunchPlan, World};
use crate::pump::{self, TOKEN_2022};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

const ZEC1: u64 = 100_000_000;

/// Launches with a link of `link_len` characters and buys `first_buy` ZEC
/// units in the same transaction. Returns (bytes, compute units, world, mint).
fn launch_and_buy(link_len: usize, first_buy: u64) -> (usize, u64, World, Pubkey) {
    let mut w = World::new();
    let launcher = w.trader(100 * ZEC1);
    let table = w.b.address_table(&kit::fixed_accounts());
    let mint = Keypair::new();
    let plan = LaunchPlan {
        name: "A coin with a long enough name ok".chars().take(32).collect(),
        uri: "https://example.com/".chars().chain(std::iter::repeat('x')).take(link_len).collect(),
        ..Default::default()
    };
    let mut ixs = vec![w.launch_ix(&launcher.pubkey(), &mint.pubkey(), &plan)];
    if first_buy > 0 {
        ixs.push(pump::create_ata(&launcher.pubkey(), &launcher.pubkey(), &mint.pubkey(), &TOKEN_2022));
        ixs.push(pump::buy_exact_quote_in_v3(&mint.pubkey(), &launcher.pubkey(), first_buy, 1));
    }
    let (size, r) = w.b.send_with_table(&launcher, &ixs, &[&mint], &table);
    assert!(r.is_ok(), "link of {link_len}: {} bytes\n{}", size, logs(&r));
    let units = r.unwrap().compute_units_consumed;
    assert_eq!(w.coin(&mint.pubkey()).launcher, launcher.pubkey());
    if first_buy > 0 {
        assert!(w.coins_of(&launcher.pubkey(), &mint.pubkey()) > 0, "the first buy went through");
        assert!(w.b.curve(&mint.pubkey()).creator_fee > 0, "and already owes the coin its fee");
    }
    (size, units, w, mint.pubkey())
}

#[test]
fn launch_with_a_first_buy_fits_one_transaction_at_every_allowed_link_length() {
    for link in [14usize, 60, 100] {
        let (size, units, _, _) = launch_and_buy(link, ZEC1 / 2);
        println!("link {link:>3} characters, with first buy: {size} bytes, {units} compute units");
        // Room is left for the priority-fee instruction a wallet adds.
        assert!(size + 16 <= MAX_TRANSACTION, "link {link}: {size} bytes");
        assert!(units < 400_000);
        let (size, units, _, _) = launch_and_buy(link, 0);
        println!("link {link:>3} characters, launch only:    {size} bytes, {units} compute units");
        assert!(size <= MAX_TRANSACTION);
    }
}

#[test]
fn a_link_longer_than_fits_is_refused() {
    // pump.fun itself allows 200 characters. Measured: with a 200-character
    // link, launch plus first buy is 1,247 bytes, over the network's 1,232.
    // So Burrpad stops at 100.
    let mut w = World::new();
    let table = w.b.address_table(&kit::fixed_accounts());
    let payer = w.b.payer.insecure_clone();
    for (len, ok) in [(100usize, true), (101, false), (200, false)] {
        let mint = Keypair::new();
        let plan = LaunchPlan { uri: "x".repeat(len), ..Default::default() };
        let i = w.launch_ix(&payer.pubkey(), &mint.pubkey(), &plan);
        let (_, r) = w.b.send_with_table(&payer, &[i], &[&mint], &table);
        if ok {
            assert!(r.is_ok(), "{}", logs(&r));
        } else {
            assert_eq!(custom_code(&r), Some(kit::err(burrpad::error::BurrpadError::TextTooLong)), "{len}");
        }
    }
}

#[test]
fn a_first_buy_does_not_delay_the_first_burn() {
    // The launcher's own buy lifts the price above the launch price. Buy back
    // and burn buys on its first call all the same.
    println!("first buy (ZEC), price rise, minutes until buy back and burn first buys");
    for first_buy in [ZEC1 / 10, ZEC1 / 2, ZEC1, 3 * ZEC1] {
        let (_, _, mut w, mint) = launch_and_buy(100, first_buy);
        let c = w.b.curve(&mint);
        let rise_pct = (c.virtual_quote_reserves as u128 * 1_073_000_000_000_000u128 * 100)
            / (c.virtual_token_reserves as u128 * 275_548_581u128)
            - 100;
        // A burn-only coin would have its pot from the same trades; here fee is added by hand.
        w.b.give_zec(&kit::authority(&mint), 100 * ZEC1);
        assert!(w.split(&mint).is_ok());
        w.b.wait(60);
        let calls = w.burn_until_it_buys(&mint);
        println!("{:.2}, +{rise_pct}%, {calls}", first_buy as f64 / ZEC1 as f64);
        assert_eq!(calls, 1, "first buy {first_buy}: {calls} calls");
    }
}
