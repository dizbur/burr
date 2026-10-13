# Burrpad

Fee hooks for ZEC-priced pump.fun coins, as one Solana program.

A coin launched through Burrpad is a real pump.fun coin, priced in ZEC, whose creator is an address only this program can sign for. Its creator fee can therefore only arrive in a vault the program controls. What happens to it is chosen once, at launch, from four hooks. Nothing in this program can change that choice afterwards: not the creator, not the administrator. pump.fun, whose programs this stands on, can still change a coin's fee or where it is paid.

| Hook | What it does with its share of the fee |
|---|---|
| Creator's share | Paid to an address named at launch. |
| Give | Paid to a second address named at launch. |
| Buy back and burn | Buys the coin and destroys what it bought. |
| Holders | Set aside for the coin's holders. **The program does not pay holders itself.** It releases the pot to a rewards wallet that the platform's server controls, and that server pays holders from a published snapshot. Who gets how much is worked out off the chain. |

The first three hooks pay by rule alone; no key decides. The holders hook is different: whoever holds the rewards wallet's key receives every coin's holder pot and is trusted to pay it on. The program limits that to the holder pot, to a stated amount no larger than the pot, at most once every 30 minutes per coin, and requires each release to name its snapshot by slot and fingerprint. The administrator can switch releases off at once, and can replace the rewards wallet only by a proposal made in the open that waits 24 hours.

Before the hooks, a part of each coin's fee goes to the platform. It starts at one half and can never be set above one half; the figure in force at launch is written into the coin's record and cannot be changed for that coin. The program sends that part to the platform's receiver and enforces nothing about what happens to it next. The platform's stated use of it (buying and burning the platform's own coin with four fifths, keeping one fifth) is done by a server outside this program. There is no launch fee.

**Status: deployed to Solana's main network on 13 Oct 2026, at `42w1jV45WeSmfM96vr6GrqSWRcyF5rVbPWjmQ89bvdhg`, from commit `e8f67d7` (an ordinary build, not yet a reproducible one). Not yet open to the public.** What checking it has had: the test suite here, run against copies of pump.fun's real programs; mutation tests that remove each protection in turn; and several independent read-throughs of the code. No outside security firm has examined it. Use it at your own risk.

## What is here

```
programs/burrpad/          the program
tests/                  the test bench: Burrpad run against real copies of pump.fun's programs
tests-standin/          a stand-in for pump.fun that misbehaves on purpose; tests only, never deployed
scripts/ground/         read-only checks against the main network (they send nothing)
scripts/check.sh        format, lint, advisories, build, tests
scripts/deploy/         deploy, set up, check and upgrade the program, with the deployer's own keys
scripts/protections.py  removes each protection in turn, in a throwaway copy, to prove a test notices
```

## Pinned tools

| Tool | Version |
|---|---|
| Rust | 1.99.0 (`rust-toolchain.toml`) |
| Solana tools (Agave) | 4.3.0, platform tools v1.57 |
| Anchor | 1.2.1 |
| Test runtime (litesvm) | 0.18.0 |
| cargo-audit | 0.22.1 |
| Node, for the read-only scripts | 24 |

Every crate is pinned by `Cargo.lock`. Direct dependencies are pinned exactly in each `Cargo.toml`.

Install, the standard way:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.99.0
sh -c "$(curl -sSfL https://release.anza.xyz/v4.3.0/install)"
cargo install anchor-cli --version 1.2.1 --locked
cargo install cargo-audit --version 0.22.1 --locked
```

## Build and test

The tests run pump.fun's real programs. Those are pump.fun's, so they are not in this repository: fetch copies first. This only reads from the main network.

```sh
cd scripts/ground && npm ci
mkdir -p ../../notes/pump-src   # pump.fun's interface files, for the other read-only scripts only
node 07-fetch-fixtures.mjs      # writes ../../fixtures (about 30 MB, not committed)
cd ../..
./scripts/check.sh
```

`check.sh` builds the program with `cargo build-sbf` and runs the whole suite. `anchor build` works too and also writes the interface file a site needs, `target/idl/burrpad.json`. The tests load `target/deploy/burrpad.so`, so the build must come first.

The tests do not use a validator. They run the program, and pump.fun's real programs, inside an in-process Solana runtime, which lets them move the clock.

To check that the tests have teeth (about half an hour, several GB of disk for throwaway copies):

```sh
python3 scripts/protections.py
```

## Using it from a site

`launch` passes about 27 accounts. With a long picture link that does not fit an old-style transaction: use a versioned transaction with an address table holding the fixed accounts. An address table holds no power.

## Deploying

Not done by this repository: whoever deploys runs the scripts in `scripts/deploy/` themselves, with their own keys, given as paths to key files. No script has a default network. The network's address is taken from the environment variable `BURRPAD_RPC_URL` and is never printed, and the main network needs `--mainnet yes` on every command. Every transaction is simulated first and is not sent if the simulation fails.

```
scripts/deploy/set-program-id.sh     writes the program's address into the code (declare_id! and Anchor.toml)
scripts/deploy/build.sh              a reproducible build in Docker (needs solana-verify); --plain for an ordinary one
scripts/deploy/deploy.sh             puts the program on a network; checks the wallet holds enough first
scripts/deploy/initialize.mjs        calls initialize, in the same sitting
scripts/deploy/address-table.mjs     makes and freezes the address table a launch needs
scripts/deploy/verify.mjs            reads everything back and prints PASS or FAIL per item; sends nothing
scripts/deploy/first-coin.mjs        takes one small coin through everything on the live network: launch with a first buy,
                                     buy, sell, collect, split, pay out, and thirty minutes later burn and the holders' release
scripts/deploy/upgrade.sh            replaces the deployed code and reads it back; or uploads it and offers it to a multi-signature wallet
scripts/deploy/admin.mjs             the administrator's switches and hand-overs; for a multi-signature wallet it prints the transaction to approve
scripts/deploy/hand-over-upgrade.mjs passes the upgrade key to the multi-signature wallet, once that wallet is the administrator
scripts/deploy/coin.mjs, lib.mjs     builders for every instruction a coin needs, from the interface file; a worked example for a site
scripts/deploy/rehearsal/            rehearses all of the above on a local validator running pump.fun's real programs
```

The order: set the address, build, deploy, initialize, address table, verify, first coin. One wallet can hold the upgrade key and the administrator role while testing. Before the public is invited both go to a multi-signature wallet: the administrator role is proposed by the deployer and accepted by the wallet, and only then can the upgrade key be handed over.

A program this size (about 663 KB) locks about 4.62 SOL on the network for good, and each upgrade needs the same amount again as a float, which is returned when the upgrade lands.

The rehearsal (`scripts/deploy/rehearsal/rehearse.sh <name>`) starts from a clean copy of the code and throwaway keys and runs every step above, including an upgrade, both pause switches and the hand-over. It takes about 40 minutes, most of it waiting for the first holder release to be allowed. A local validator switches on every planned network rule; the rehearsal switches one off (SIMD-0500, which refuses to deploy programs in this format and is not in force on the main network at the time of writing), and `deploy.sh` and `upgrade.sh` stop with a clear message if a network has it in force.

Two switches stop things: `admin.mjs pause-launches` (no new coins) and `admin.mjs pause-holder-releases` (no holder pot can leave). Nothing stops trading, collecting, the fixed payouts or buy back and burn.

## Licence

No licence has been chosen yet. All rights reserved.
