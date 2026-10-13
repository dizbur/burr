// Reads who can upgrade pump.fun's three programs. Two calls.
import { PublicKey } from "@solana/web3.js";
import { conn, PUMP, PUMP_AMM, PUMP_FEES } from "./lib.mjs";
const progs = { pump: PUMP, pump_amm: PUMP_AMM, pump_fees: PUMP_FEES };
const a = await conn.getMultipleAccountsInfo(Object.values(progs));
const pd = a.map((x) => new PublicKey(x.data.subarray(4, 36)));
const b = await conn.getMultipleAccountsInfo(pd, { dataSlice: { offset: 0, length: 45 } });
Object.keys(progs).forEach((k, i) => {
  const d = b[i].data;
  console.log(k.padEnd(10), progs[k].toBase58(), "| last deployed slot", d.readBigUInt64LE(4),
    "| upgrade authority", d[12] ? new PublicKey(d.subarray(13, 45)).toBase58() : "none (frozen)");
});
