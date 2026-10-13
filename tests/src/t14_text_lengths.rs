//! Every total length of name, ticker and link must launch.
//!
//! pump.fun's create leaves the new mint short of its rent when the three
//! texts together are exactly 37 bytes, and the network then refuses the
//! transaction ("insufficient funds for rent"). Launch makes up the
//! shortfall from the launcher. This test tries every length.

use crate::bench::logs;
use crate::kit::{self, LaunchPlan, World};
use solana_keypair::Keypair;
use solana_signer::Signer;

/// Tries a launch whose name is `name` bytes and link `link` bytes (the
/// ticker is 3). Returns the end of the log if it failed.
fn try_lengths(
    w: &mut World,
    table: &solana_message::AddressLookupTableAccount,
    name: usize,
    link: usize,
) -> Option<String> {
    let launcher = w.b.wallet();
    let mint = Keypair::new();
    let plan = LaunchPlan { name: "n".repeat(name), uri: "u".repeat(link), ..Default::default() };
    let i = w.launch_ix(&launcher.pubkey(), &mint.pubkey(), &plan);
    let (_, r) = w.b.send_with_table(&launcher, &[i], &[&mint], table);
    r.is_err().then(|| {
        format!(
            "{:?} {}",
            r.as_ref().err().map(|e| e.err.clone()),
            logs(&r).lines().rev().take(8).collect::<Vec<_>>().join(" | ")
        )
    })
}

#[test]
fn a_launch_works_at_every_text_length() {
    let mut w = World::new();
    let table = w.b.address_table(&kit::fixed_accounts());
    let mut failed = vec![];
    // name 1..=32 with a 1-byte link, then name 4 with links up to the limit:
    // totals (with the 3-byte ticker) from 5 to 107.
    let cases = (1..=32usize).map(|n| (n, 1usize)).chain((1..=100usize).map(|l| (4usize, l)));
    for (name, link) in cases {
        if let Some(why) = try_lengths(&mut w, &table, name, link) {
            failed.push((name + 3 + link, name, link, why));
        }
    }
    assert!(failed.is_empty(), "text lengths that failed (total, name, link, why): {failed:#?}");
    // The length that used to fail, checked on its own so the message is clear.
    assert!(try_lengths(&mut w, &table, 4, 30).is_none(), "the 37-byte launch is short of rent");
}
