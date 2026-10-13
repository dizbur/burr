// Calls `initialize` once, right after the deploy, with the upgrade key.
//   node scripts/deploy/initialize.mjs --url <network> --upgrade-key <file> \
//        --platform-receiver <address> --rewards-wallet <address> [--mainnet yes]
// The upgrade key becomes the administrator for now; admin.mjs hands that on.
// The platform's share (one half) and the fee range (0.50% to 3.00%, first
// offered 2.00%) are fixed in the program and are checked here afterwards.
import { address, checker, fail, configAddress, connect, instruction, loadIdl, loadKey, need, options, programDataAddress,
  readAccount, send, SYSTEM, writeOut } from "./lib.mjs";

const opts = options(["url", "upgrade-key", "platform-receiver", "rewards-wallet", "mainnet"]);
const conn = await connect(opts);
const idl = loadIdl();
const key = loadKey(need(opts, "upgrade-key"), "--upgrade-key");
const platformReceiver = address(need(opts, "platform-receiver"), "--platform-receiver");
const rewardsWallet = address(need(opts, "rewards-wallet"), "--rewards-wallet");
const config = configAddress(idl);

// A receiver must be an ordinary funded wallet. An address has no built-in
// check for typing mistakes, and every coin copies the platform receiver for good.
for (const [what, who] of [["--platform-receiver", platformReceiver], ["--rewards-wallet", rewardsWallet]]) {
  const info = await conn.getAccountInfo(who);
  if (!info) fail(`${what} ${who.toBase58()} has never been used on this network. Send it a little SOL first; if it still fails, the address is wrong.`);
  if (!info.owner.equals(SYSTEM) || info.executable) fail(`${what} ${who.toBase58()} is not an ordinary wallet (it is a program's account or a token account).`);
}
if (new Set([key.publicKey.toBase58(), platformReceiver.toBase58(), rewardsWallet.toBase58()]).size !== 3) {
  fail("the upgrade key, the platform receiver and the rewards wallet must be three different addresses.");
}

// "Already there" means an account this program owns and can read, not merely an address someone sent SOL to.
const existing = await readAccount(conn, idl, "Config", config).catch(() => null);
if (existing) {
  console.log("Already initialised. Nothing sent. Checking it against what was given:");
} else {
  const { signature } = await send(conn, [
    instruction(idl, "initialize", {
      upgrade_authority: key.publicKey,
      config,
      program: idl.programId,
      program_data: programDataAddress(idl.programId),
      system_program: SYSTEM,
    }, { admin: key.publicKey, platform_receiver: platformReceiver, rewards_wallet: rewardsWallet }),
  ], [key]);
  console.log("Sent: " + signature);
}

const c = await readAccount(conn, idl, "Config", config);
const t = checker();
if (!existing) t.eq("administrator is the upgrade key, for now", c.admin, key.publicKey.toBase58());
t.eq("platform receiver", c.platform_receiver, platformReceiver.toBase58());
t.eq("rewards wallet", c.rewards_wallet, rewardsWallet.toBase58());
t.eq("platform's share is one half", c.platform_fraction_bps, 5000);
t.eq("fee range", [c.min_fee_bps, c.max_fee_bps, c.default_fee_bps], [50, 300, 200]);
t.eq("launches on", c.launches_paused, false);
t.eq("holder releases on", c.holder_releases_paused, false);
writeOut(conn.outName, { program: idl.programId.toBase58(), config: config.toBase58() });
t.finish();
