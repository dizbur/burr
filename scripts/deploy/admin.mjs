// The administrator's switches and hand-overs.
//
//   node scripts/deploy/admin.mjs <action> [address] --url <network> (--admin-key <file> | --admin <address>) [--mainnet yes]
//
// With --admin-key the transaction is signed and sent. With --admin (the
// address of a multi-signature wallet) nothing is sent: the unsigned
// transaction is printed as text, to paste into that wallet's app and approve there.
//
// Actions:
//   pause-launches | resume-launches                 new coins off / on
//   pause-holder-releases | resume-holder-releases   every coin's holder payout off / on
//   propose-admin <address>                          first step of handing the administrator role on
//   accept-admin                                     second step, by the proposed administrator
//   propose-rewards-wallet <address>                 starts the 24-hour wait
//   cancel-rewards-wallet | apply-rewards-wallet
//   set-platform-receiver <address>                  for coins launched from now on
//   set-platform-fraction <bps>                      the platform's part of each fee, 2500 to 5000, for coins launched from now on
//   set-fee-range --min <bps> --max <bps> --default <bps>
//                                                    the fees a launcher may pick, in hundredths of a percent, for coins launched from now on
import { address, configAddress, connect, fail, ordinaryWallet, instruction, loadIdl, loadKey, options, readAccount, send, unsignedForWallet } from "./lib.mjs";

const opts = options(["url", "admin-key", "admin", "min", "max", "default", "mainnet"]);
const [action, value] = opts._;
const conn = await connect(opts);
const idl = loadIdl();
if (opts["admin-key"] && opts.admin) fail("Give --admin-key or --admin, not both.");
if (opts._.length > 2) fail("Too many words. The form is: admin.mjs <action> [address] --options");
const key = opts["admin-key"] ? loadKey(opts["admin-key"], "--admin-key") : null;
const admin = key ? key.publicKey : opts.admin ? address(opts.admin, "--admin") : fail("Give --admin-key <file> or --admin <address>.");
const config = configAddress(idl);
const asAdmin = (name, args = {}) => instruction(idl, name, { admin, config }, args);
const target = () => address(value ?? fail(`${action} needs an address`), "the address");
const whole = (text, what) => (/^[0-9]+$/.test(text ?? "") ? Number(text) : fail(`${what} must be a whole number of hundredths of a percent (200 is 2.00%). Nothing was sent.`));
const pct = (bps) => `${(bps / 100).toFixed(2)}%`;
// The program's own limits are 10 to 500; pump.fun accepts a creator fee of 1 to 300. A launch above 300 would fail at pump.fun.
const feeRange = () => {
  const [min, max, def] = [whole(opts.min, "--min"), whole(opts.max, "--max"), whole(opts.default, "--default")];
  if (min < 10) fail("--min cannot be below 10 (0.10%), the program's hard limit. Nothing was sent.");
  if (max > 300) fail("--max cannot be above 300 (3.00%): pump.fun refuses a higher creator fee. Nothing was sent.");
  if (!(min <= def && def <= max)) fail("the fees must be in order: --min, then --default, then --max. Nothing was sent.");
  return { min_bps: min, max_bps: max, default_bps: def };
};
const fraction = () => {
  const bps = whole(value, "the fraction");
  if (bps < 2500 || bps > 5000) fail("the platform's part must be between 2500 (a quarter) and 5000 (a half), the program's hard limits. Nothing was sent.");
  return { fraction_bps: bps };
};

const actions = {
  "pause-launches": () => asAdmin("set_launches_paused", { paused: true }),
  "resume-launches": () => asAdmin("set_launches_paused", { paused: false }),
  "pause-holder-releases": () => asAdmin("set_holder_releases_paused", { paused: true }),
  "resume-holder-releases": () => asAdmin("set_holder_releases_paused", { paused: false }),
  "propose-admin": () => asAdmin("propose_admin", { new_admin: target() }),
  "accept-admin": () => instruction(idl, "accept_admin", { new_admin: admin, config }),
  "propose-rewards-wallet": () => asAdmin("propose_rewards_wallet", { wallet: target() }),
  "cancel-rewards-wallet": () => asAdmin("cancel_rewards_wallet"),
  "apply-rewards-wallet": () => asAdmin("apply_rewards_wallet"),
  "set-platform-receiver": () => asAdmin("set_platform_receiver", { receiver: target() }),
  "set-platform-fraction": () => asAdmin("set_platform_fraction", fraction()),
  "set-fee-range": () => asAdmin("set_fee_range", feeRange()),
};
if (!actions[action]) fail("Unknown action. One of: " + Object.keys(actions).join(", "));
if (action === "set-platform-receiver" || action === "propose-rewards-wallet") await ordinaryWallet(conn, target(), "the new address");
const ix = actions[action]();
const settings = async () => {
  const c = await readAccount(conn, idl, "Config", config);
  return `launchers may pick a fee from ${pct(c.min_fee_bps)} to ${pct(c.max_fee_bps)}, first offered ${pct(c.default_fee_bps)}; the platform's part is ${pct(c.platform_fraction_bps)} of each fee, paid to ${c.platform_receiver}`;
};
const forFuture = ["set-fee-range", "set-platform-fraction", "set-platform-receiver"].includes(action);
if (forFuture) console.log("Before: " + (await settings()));

if (!key) {
  console.log(`Nothing was sent. Paste this into the multi-signature wallet's app as a transaction to approve (${action}):\n`);
  console.log(await unsignedForWallet(conn, [ix], admin));
  console.log("\nIt goes stale after about a minute if the app needs a fresh one; run this again then.");
  process.exit(0);
}
const { signature } = await send(conn, [ix], [key]);
console.log(`PASS  ${action} sent: ${signature}`);
if (forFuture) console.log("After:  " + (await settings()) + "\n        This applies to coins launched from now on. Coins already launched keep the terms they were launched with.");
const c = await readAccount(conn, idl, "Config", config);
console.log(`      administrator ${c.admin}` + (c.pending_admin !== "11111111111111111111111111111111" ? `, proposed ${c.pending_admin}` : ""));
console.log(`      launches ${c.launches_paused ? "PAUSED" : "on"}, holder releases ${c.holder_releases_paused ? "PAUSED" : "on"}`);
console.log(`      rewards wallet ${c.rewards_wallet}` + (c.pending_rewards_wallet !== "11111111111111111111111111111111" ? `, proposed ${c.pending_rewards_wallet}, can be applied at ${new Date(Number(c.rewards_wallet_ready_at) * 1000).toISOString()}` : ""));
