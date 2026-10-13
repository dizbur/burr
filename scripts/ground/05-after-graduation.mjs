// Reads the graduated Sapling coin: its PumpSwap pool, where its creator fee
// waits, and how it has really been collected. A few calls.
import { PublicKey } from "@solana/web3.js";
import { conn, PUMP, PUMP_AMM, ZEC, TOKEN, pda, ata, loadIdl, decode, show } from "./lib.mjs";

const amm = loadIdl("pump_amm.json");
const mint = new PublicKey("8TUn4HHBybSR4oatWshpeM7i1keT5esWCZE6zc8EB1uC");
const creator = new PublicKey("8gM6Sp1wRsiLjQyKuCLnFRWVNmHt3Az63eYkJdeFKcc9");
const poolAuthority = pda(["pool-authority", mint], PUMP);
const pool = pda(["pool", Buffer.from([0, 0]), poolAuthority, mint, ZEC], PUMP_AMM);
const curveVault = pda(["creator-vault", creator], PUMP);
const ammVault = pda(["creator_vault", creator], PUMP_AMM);
const addrs = { pool, creator, creatorZec: ata(creator, ZEC, TOKEN), curveVaultZec: ata(curveVault, ZEC, TOKEN),
  ammVaultZec: ata(ammVault, ZEC, TOKEN), ammGlobal: pda(["global_config"], PUMP_AMM), mint,
  sharingConfig: pda(["sharing-config", mint], new PublicKey("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ")) };
const infos = await conn.getMultipleAccountsInfo(Object.values(addrs));
Object.keys(addrs).forEach((k, i) => console.log(k.padEnd(14), addrs[k].toBase58(), infos[i]
  ? `owner ${infos[i].owner.toBase58()} len ${infos[i].data.length}` + (infos[i].data.length === 165 ? ` amount ${infos[i].data.readBigUInt64LE(64)} tokenOwner ${new PublicKey(infos[i].data.subarray(32, 64)).toBase58()}` : "")
  : "no account"));
if (infos[0]) console.log("pool", show(decode(amm, "Pool", infos[0].data)));
if (infos[5]) console.log("amm global", show(decode(amm, "GlobalConfig", infos[5].data)));
for (const [name, a] of [["creator's ZEC account", addrs.creatorZec], ["amm creator vault", addrs.ammVaultZec]]) {
  await new Promise((r) => setTimeout(r, 2000));
  const sigs = await conn.getSignaturesForAddress(a, { limit: 6 });
  console.log(`\nrecent on ${name}:`, sigs.length);
  const s = sigs.find((x) => !x.err);
  if (!s) continue;
  await new Promise((r) => setTimeout(r, 2000));
  const tx = await conn.getTransaction(s.signature, { maxSupportedTransactionVersion: 0 });
  console.log("tx", s.signature, "slot", s.slot);
  console.log(tx.meta.logMessages.filter((l) => /Instruction:|invoke \[1\]/.test(l)).join("\n"));
}
