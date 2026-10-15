// Rehearsal only: compares what the same calls did to the copied coins'
// books before and after the upgrade. Times and slots are left out (the two
// runs are minutes apart); every amount must match to the unit.
//   node dress-compare.mjs <before.json> <after.json>
import { readFileSync } from "node:fs";

const [a, b] = process.argv.slice(2).map((f) => JSON.parse(readFileSync(f, "utf8")));
const flat = (o, p = "", out = {}) => {
  if (o && typeof o === "object") for (const [k, v] of Object.entries(o)) flat(v, p ? `${p}.${k}` : k, out);
  else if (!/time|slot|hash|release|bump/.test(p)) out[p] = o;
  return out;
};
const delta = (r) => {
  const x = flat(r.before), y = flat(r.after), d = {};
  for (const k of new Set([...Object.keys(x), ...Object.keys(y)])) {
    if (/^\d+$|^-?\d+$/.test(String(x[k] ?? "0")) && /^-?\d+$/.test(String(y[k] ?? "0"))) {
      const v = BigInt(y[k] ?? 0) - BigInt(x[k] ?? 0);
      if (v !== 0n) d[k] = String(v);
    }
  }
  return d;
};
const key = (r) => (r.name.startsWith("launch") ? r.name : `${r.name} ${r.mint}`);
const after = new Map(b.map((r) => [key(r), r]));
let differ = 0;
for (const r of a) {
  const s = after.get(key(r));
  if (!s) { console.log(`MISSING after: ${key(r)}`); differ++; continue; }
  const [d1, d2] = [delta(r), delta(s)];
  const same = r.ok === s.ok && r.err === s.err && JSON.stringify(d1) === JSON.stringify(d2) && (r.bought ?? "") === (s.bought ?? "");
  if (!same) differ++;
  const what = r.ok ? Object.entries(d1).map(([k, v]) => `${k} ${v}`).join(", ") || "no change to the books" : r.err;
  console.log(`${same ? "same    " : "DIFFERS "} ${key(r).slice(0, 48).padEnd(48)} ${r.ok ? "ok" : "refused"}: ${what.slice(0, 140)}`);
  if (!same) console.log(`         after: ${s.ok ? "ok" : "refused " + s.err}: ${JSON.stringify(d2).slice(0, 300)}`);
}
for (const s of b) if (!a.some((r) => key(r) === key(s))) console.log(`new     ${key(s).slice(0, 48).padEnd(48)} ${s.ok ? "ok" : "refused " + s.err}${s.size ? `, ${s.size} bytes` : ""}${s.bought ? `, bought ${s.bought}` : ""}`);
console.log(differ ? `\n${differ} DIFFER` : "\nAll the same calls did exactly the same to the books, before and after.");
process.exit(differ ? 1 : 0);
