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

**Status: not deployed.** What checking it has had: the test suite here, run against copies of pump.fun's real programs; mutation tests that remove each protection in turn; and several independent read-throughs of the code. No outside security firm has examined it. Use it at your own risk.

## What is here

```
programs/burrpad/          the program
tests/                  the test bench: Burrpad run against real copies of pump.fun's programs
tests-standin/          a stand-in for pump.fun that misbehaves on purpose; tests only, never deployed
scripts/ground/         read-only checks against the main network (they send nothing)
scripts/check.sh        format, lint, advisories, build, tests
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

Not yet, and not by this repository. Before any deployment: a program address chosen at deployment (the one in `declare_id!` is a local test address with no key kept), and a two-of-three wallet as upgrade key and administrator.

## Licence

No licence has been chosen yet. All rights reserved.
