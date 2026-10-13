// Copies pump.fun's programs and the accounts they need from the main network
// into fixtures/, for the local test bench. Reading only. A handful of calls.
// Run:  node 07-fetch-fixtures.mjs
import { PublicKey } from "@solana/web3.js";
import { writeFileSync, mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { conn, PUMP, PUMP_AMM, PUMP_FEES, MAYHEM, ZEC, TOKEN, pda, ata } from "./lib.mjs";

const out = path.join(path.dirname(fileURLToPath(import.meta.url)), "../../fixtures");
mkdirSync(out, { recursive: true });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const programs = { pump: PUMP, pump_amm: PUMP_AMM, pump_fees: PUMP_FEES, mayhem: MAYHEM };
const BUYBACK = [
  "5YxQFdt3Tr9zJLvkFccqXVUwhdTWJQc1fFg2YPbxvxeD", "9M4giFFMxmFGXtc3feFzRai56WbBqehoSeRE5GK7gf7",
].map((s) => new PublicKey(s));
const accounts = {
  pump_global: pda(["global"], PUMP),
  pump_quote_control: pda(["quote-control"], PUMP),
  pump_global_volume: pda(["global_volume_accumulator"], PUMP),
  pump_fee_config: pda(["fee_config", PUMP], PUMP_FEES),
  amm_fee_config: pda(["fee_config", PUMP_AMM], PUMP_FEES),
  amm_global_config: pda(["global_config"], PUMP_AMM),
  amm_global_volume: pda(["global_volume_accumulator"], PUMP_AMM),
  mayhem_global_params: pda(["global-params"], MAYHEM),
  mayhem_sol_vault: pda(["sol-vault"], MAYHEM),
  zec_mint: ZEC,
  buyback0_zec: ata(BUYBACK[0], ZEC, TOKEN),
  buyback1_zec: ata(BUYBACK[1], ZEC, TOKEN),
};

const dump = (name, key, a) => {
  writeFileSync(path.join(out, `${name}.json`), JSON.stringify({
    name, pubkey: key.toBase58(), owner: a.owner.toBase58(), lamports: a.lamports,
    executable: a.executable, data: a.data.toString("base64"),
  }));
  console.log(name.padEnd(22), key.toBase58(), "len", a.data.length);
};

const slot = await conn.getSlot();
const names = Object.keys(accounts);
const infos = await conn.getMultipleAccountsInfo(names.map((n) => accounts[n]));
names.forEach((n, i) => (infos[i] ? dump(n, accounts[n], infos[i]) : console.log(n.padEnd(22), "NOT ON CHAIN (skipped)")));

for (const [name, id] of Object.entries(programs)) {
  await sleep(1500);
  const p = await conn.getAccountInfo(id);
  dump(`program_${name}`, id, p);
  const dataKey = new PublicKey(p.data.subarray(4, 36));
  await sleep(1500);
  dump(`programdata_${name}`, dataKey, await conn.getAccountInfo(dataKey));
}
writeFileSync(path.join(out, "README.txt"),
  `Copied from the main network at slot ${slot} by scripts/ground/07-fetch-fixtures.mjs.\nThese are pump.fun's programs and accounts. Not committed.\n`);
console.log("done at slot", slot);
