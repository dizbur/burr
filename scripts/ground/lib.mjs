// Shared helpers for read-only checks against the main network.
// Nothing here signs or sends a transaction.
import { Connection, PublicKey } from "@solana/web3.js";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));

// A public endpoint. Used gently: a few calls, never in a loop.
export const RPC = "https://api.mainnet-beta.solana.com";
export const conn = new Connection(RPC, "confirmed");

export const PUMP = new PublicKey("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");
export const PUMP_AMM = new PublicKey("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA");
export const PUMP_FEES = new PublicKey("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ");
export const MAYHEM = new PublicKey("MAyhSmzXzV1pTf7LsNkrNwkWKTo4ougAJ1PPg47MD4e");
export const ZEC = new PublicKey("A7bdiYdS5GjqGFtxf17ppRHtDKPkkRqbKtR27dxvQXaS");
export const TOKEN = new PublicKey("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
export const TOKEN_2022 = new PublicKey("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
export const ATA_PROGRAM = new PublicKey("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

export const pda = (seeds, program) =>
  PublicKey.findProgramAddressSync(
    seeds.map((s) => (typeof s === "string" ? Buffer.from(s) : s.toBuffer ? s.toBuffer() : s)),
    program,
  )[0];

export const ata = (owner, mint, tokenProgram) =>
  pda([owner, tokenProgram, mint], ATA_PROGRAM);

// The interface files are pump.fun's, kept outside git in notes/pump-src.
export const loadIdl = (name) =>
  JSON.parse(readFileSync(path.join(here, "../../notes/pump-src", name), "utf8"));

// Decodes an account by its interface description. Short accounts are
// padded with zeros, as pump.fun's docs say to read them.
export function decode(idl, typeName, data, skipDiscriminator = true) {
  const buf = Buffer.concat([Buffer.from(data), Buffer.alloc(4096)]);
  let o = skipDiscriminator ? 8 : 0;
  const types = Object.fromEntries(idl.types.map((t) => [t.name, t.type]));
  const rd = (t) => {
    if (typeof t === "string") {
      switch (t) {
        case "bool": return buf[o++] !== 0;
        case "u8": return buf[o++];
        case "u16": { const v = buf.readUInt16LE(o); o += 2; return v; }
        case "u32": { const v = buf.readUInt32LE(o); o += 4; return v; }
        case "u64": { const v = buf.readBigUInt64LE(o); o += 8; return v; }
        case "i64": { const v = buf.readBigInt64LE(o); o += 8; return v; }
        case "u128": case "i128": {
          const lo = buf.readBigUInt64LE(o), hi = buf.readBigUInt64LE(o + 8); o += 16;
          const v = (hi << 64n) | lo;
          return t === "i128" ? BigInt.asIntN(128, v) : v;
        }
        case "pubkey": { const v = new PublicKey(buf.subarray(o, o + 32)); o += 32; return v.toBase58(); }
        case "string": { const n = buf.readUInt32LE(o); o += 4; const v = buf.subarray(o, o + n).toString(); o += n; return v; }
      }
      throw new Error("type " + t);
    }
    if (t.array) return Array.from({ length: t.array[1] }, () => rd(t.array[0]));
    if (t.vec) { const n = buf.readUInt32LE(o); o += 4; return Array.from({ length: n }, () => rd(t.vec)); }
    if (t.option) return buf[o++] ? rd(t.option) : null;
    if (t.defined) return struct(types[t.defined.name]);
    throw new Error("type " + JSON.stringify(t));
  };
  const struct = (def) => {
    if (def.kind === "enum") return def.variants[buf[o++]]?.name;
    const out = {};
    def.fields.forEach((f, i) => {
      if (f.name) out[f.name] = rd(f.type);
      else out[i] = rd(f);
    });
    return out;
  };
  return struct(types[typeName]);
}

export const show = (x) =>
  JSON.stringify(x, (_, v) => (typeof v === "bigint" ? v.toString() : v), 2);
