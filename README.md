# Burr

Fee hooks for ZEC-priced pump.fun coins, as one Solana program.

A coin launched through Burr is a real pump.fun coin, priced in ZEC, whose creator is an address only this program can sign for. Its creator fee can therefore only arrive in a vault the program controls. What happens to it is chosen once, at launch, from five hooks, and can never be changed.

| Hook | What it does with its share of the fee |
|---|---|
| Creator's share | Paid to an address named at launch. |
| Give | Paid to a second address named at launch. |
| Buy back and burn | Buys the coin and destroys what it bought. |
| Vault share | Shared among people who place their coins in the coin's vault, by amount and by time. |
| King of the hill | Each round, the largest single buy made through the program takes the pot. |

**Status: not audited. Not deployed. Do not use with real funds.**

## What is here

```
programs/burr/          the program
tests/                  the test bench: Burr run against real copies of pump.fun's programs
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

`check.sh` builds the program with `cargo build-sbf` and runs the whole suite. `anchor build` works too and also writes the interface file a site needs, `target/idl/burr.json`. The tests load `target/deploy/burr.so`, so the build must come first.

The tests do not use a validator. They run the program, and pump.fun's real programs, inside an in-process Solana runtime, which lets them move the clock.

To check that the tests have teeth (about half an hour, several GB of disk for throwaway copies):

```sh
python3 scripts/protections.py
```

## Using it from a site

`launch` passes about 27 accounts. With a long picture link that does not fit an old-style transaction: use a versioned transaction with an address table holding the fixed accounts. An address table holds no power.

## Deploying

Not yet, and not by this repository. Before any deployment: an outside audit, a program address chosen by the owner (the one in `declare_id!` is a local test address with no key kept), and a two-of-three wallet as upgrade key and administrator.

## Licence

Not yet chosen. All rights reserved until the owner picks one.
