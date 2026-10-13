//! pump.fun's real programs, run locally, with no program of ours.
//! This finishes the ground-proving: sweep and collect to a creator that is a
//! program-controlled address, which could not be simulated on the main network.

use crate::bench::{logs, Bench};
use crate::pump::{self, ata, TOKEN, TOKEN_2022, ZEC};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;

#[test]
fn create_trade_sweep_collect_to_a_program_controlled_creator() {
    let mut b = Bench::new();
    let trader = b.wallet();
    b.give_zec(&trader.pubkey(), 10_000_000_000);

    let mint = Keypair::new();
    let some_program = Pubkey::new_unique();
    let creator = pump::pda(&[b"coin", mint.pubkey().as_ref()], &some_program);
    assert!(!creator.is_on_curve());

    let r = b.send(&[pump::create_v2(&mint.pubkey(), &b.payer.pubkey(), &creator, 200)], &[&mint]);
    assert!(r.is_ok(), "{}", logs(&r));
    let c = b.curve(&mint.pubkey());
    assert_eq!(c.creator, creator);
    assert_eq!(c.quote_mint, ZEC);
    assert_eq!(c.creator_fee_bps, 200);

    let r = b.send_as(
        Some(&trader),
        &[
            pump::create_ata(&trader.pubkey(), &trader.pubkey(), &mint.pubkey(), &TOKEN_2022),
            pump::buy_exact_quote_in_v3(&mint.pubkey(), &trader.pubkey(), 100_000_000, 1),
        ],
        &[],
    );
    assert!(r.is_ok(), "{}", logs(&r));
    let c = b.curve(&mint.pubkey());
    // Same figures as the main-network simulation of 13 Oct 2026.
    assert_eq!(c.real_quote_reserves, 97_134_530);
    assert_eq!(c.creator_fee, 1_942_691);

    let bought = b.token_balance(&ata(&trader.pubkey(), &mint.pubkey(), &TOKEN_2022));
    let r = b.send_as(Some(&trader), &[pump::sell_v3(&mint.pubkey(), &trader.pubkey(), bought / 2, 0)], &[]);
    assert!(r.is_ok(), "{}", logs(&r));
    let waiting = b.curve(&mint.pubkey()).creator_fee;
    assert!(waiting > 1_942_691);

    // Anyone sweeps and collects. The creator signs nothing.
    let stranger = b.wallet();
    let r = b.send_as(
        Some(&stranger),
        &[
            pump::sweep_creator_fee(&mint.pubkey(), &stranger.pubkey(), &creator),
            pump::create_ata(&stranger.pubkey(), &creator, &ZEC, &TOKEN),
            pump::collect_creator_fee_v2(&creator),
        ],
        &[],
    );
    assert!(r.is_ok(), "{}", logs(&r));
    assert_eq!(b.token_balance(&ata(&creator, &ZEC, &TOKEN)), waiting);
    assert_eq!(b.curve(&mint.pubkey()).creator_fee, 0);
}
