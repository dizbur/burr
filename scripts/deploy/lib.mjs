// Shared helpers for the deploy kit.
//
// Keys: every script takes paths to key files from whoever runs it. A key is
// read only to sign, and is never printed or written anywhere.
// Network: no script has a default network. The network's address comes from
// the environment variable BURRPAD_RPC_URL (or `--url`), is never printed,
// and the main network also needs `--mainnet yes`.
import {
  AddressLookupTableAccount, ComputeBudgetProgram, Connection, Keypair, PublicKey, Transaction,
  TransactionInstruction, TransactionMessage, VersionedTransaction,
} from "@solana/web3.js";
import bs58 from "bs58";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
export const ROOT = path.resolve(here, "../..");
const MAINNET_GENESIS = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";

export const PUMP = new PublicKey("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");
export const PUMP_AMM = new PublicKey("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA");
export const PUMP_FEES = new PublicKey("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ");
export const MAYHEM = new PublicKey("MAyhSmzXzV1pTf7LsNkrNwkWKTo4ougAJ1PPg47MD4e");
export const ZEC = new PublicKey("A7bdiYdS5GjqGFtxf17ppRHtDKPkkRqbKtR27dxvQXaS");
export const TOKEN = new PublicKey("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
export const TOKEN_2022 = new PublicKey("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
export const ATA_PROGRAM = new PublicKey("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
export const SYSTEM = new PublicKey("11111111111111111111111111111111");
export const LOADER = new PublicKey("BPFLoaderUpgradeab1e11111111111111111111111");
export const COMPUTE_BUDGET = new PublicKey("ComputeBudget111111111111111111111111111111");
// One of pump.fun's buy-back receivers; its ZEC account is passed on every buy.
export const BUYBACK = new PublicKey("9M4giFFMxmFGXtc3feFzRai56WbBqehoSeRE5GK7gf7");

export const pda = (seeds, program) =>
  PublicKey.findProgramAddressSync(
    seeds.map((s) => (typeof s === "string" ? Buffer.from(s) : s.toBuffer ? s.toBuffer() : s)),
    program,
  )[0];
export const ata = (owner, mint, tokenProgram) => pda([owner, tokenProgram, mint], ATA_PROGRAM);

// A failure prints what went wrong and stops, without a page of internals.
const plain = (e) => {
  const m = e?.message ?? String(e);
  return /fetch failed|ECONNREFUSED|ENOTFOUND|ETIMEDOUT/.test(m) ? "could not reach the network. Check BURRPAD_RPC_URL and your internet connection." : m;
};
process.on("uncaughtException", (e) => fail(plain(e)));
process.on("unhandledRejection", (e) => fail(plain(e)));

// ---- command line ----

// `--name value` pairs after any plain words. Unknown names are refused.
export function options(allowed, argv = process.argv.slice(2)) {
  const out = { _: [] };
  for (let i = 0; i < argv.length; i++) {
    if (!argv[i].startsWith("--")) { out._.push(argv[i]); continue; }
    const name = argv[i].slice(2);
    if (!allowed.includes(name)) fail(`Unknown option --${name}. Allowed: ${allowed.map((a) => "--" + a).join(" ")}`);
    if (i + 1 >= argv.length) fail(`--${name} needs a value`);
    out[name] = argv[++i];
  }
  return out;
}
export function need(opts, name) {
  if (!opts[name]) fail(`Missing --${name}`);
  return opts[name];
}
let secretUrl = null;
export function fail(message) {
  // The network's address is the owner's own; it is never shown, even inside an error.
  message = String(message);
  if (secretUrl) {
    message = message.split(secretUrl).join("<the network address>");
    try {
      const u = new URL(secretUrl);
      message = message.split(u.host).join("<the network>");
      if (u.search.length > 1) message = message.split(u.search.slice(1)).join("<hidden>");
      if (u.pathname.length > 8) message = message.split(u.pathname).join("<hidden>");
    } catch { /* not a URL: nothing more to hide */ }
  }
  console.error("STOPPED: " + message);
  process.exit(1);
}
export function address(text, what) {
  // The text is not echoed back in full: something pasted by mistake might be a secret.
  try { return new PublicKey(text); } catch { fail(`${what} is not an address (it starts "${String(text).slice(0, 4)}…", ${String(text).length} characters).`); }
}
// Reads a key file to sign with. Never prints it.
export function loadKey(file, what) {
  if (!existsSync(file)) fail(`${what}: no key file at the path given`);
  try { return Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(file, "utf8")))); }
  catch { fail(`${what}: the file is not a key file`); }
}

// ---- network ----

export async function connect(opts) {
  const url = opts.url ?? process.env.BURRPAD_RPC_URL;
  if (!url) fail("No network given. Set BURRPAD_RPC_URL in this terminal (export BURRPAD_RPC_URL=...), or pass --url.");
  secretUrl = url;
  const conn = new Connection(url, "confirmed");
  const genesis = await conn.getGenesisHash();
  const mainnet = genesis === MAINNET_GENESIS;
  if (mainnet && opts.mainnet !== "yes") {
    fail("This is the main network. Add  --mainnet yes  if that is what you mean.");
  }
  // Asked for the main network, got another: a wrong connection address. Stop before anything is done.
  if (!mainnet && opts.mainnet === "yes") fail("--mainnet yes was given, but this connection is NOT the main network. Check BURRPAD_RPC_URL in this window. Nothing was done.");
  console.log(mainnet ? "Network: MAIN NETWORK" : "Network: a test network");
  conn.isMainnet = mainnet;
  // Each network keeps its own notes, so a test network's addresses never mix with another's.
  conn.outName = mainnet ? "mainnet" : "test-" + genesis.slice(0, 8);
  return conn;
}

// ---- the program's interface file ----

export function loadIdl() {
  // The interface file kept with the release build (see build.sh).
  const file = path.join(ROOT, "target/release-kit/burrpad.json");
  if (!existsSync(file)) fail("No release build here yet. Run scripts/deploy/build.sh first.");
  const idl = JSON.parse(readFileSync(file, "utf8"));
  idl.programId = new PublicKey(idl.address);
  return idl;
}
export const configAddress = (idl) => pda(["config"], idl.programId);
export const programDataAddress = (programId) => pda([programId], LOADER);
export const coinAddresses = (idl, mint) => {
  const authority = pda(["authority", mint], idl.programId);
  const burner = pda(["burner", mint], idl.programId);
  return {
    mint, authority, burner,
    coin: pda(["coin", mint], idl.programId),
    ledger: pda(["ledger", mint], idl.programId),
    vault: ata(authority, ZEC, TOKEN),
  };
};

const typeOf = (idl, name) => {
  const t = idl.types.find((x) => x.name === name);
  if (!t) throw new Error("no type " + name);
  return t.type;
};

function encodeValue(idl, t, v) {
  if (typeof t === "string") {
    switch (t) {
      case "bool": return Buffer.from([v ? 1 : 0]);
      case "u8": return Buffer.from([v]);
      case "u16": { const b = Buffer.alloc(2); b.writeUInt16LE(v); return b; }
      case "u32": { const b = Buffer.alloc(4); b.writeUInt32LE(v); return b; }
      case "u64": { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(v)); return b; }
      case "i64": { const b = Buffer.alloc(8); b.writeBigInt64LE(BigInt(v)); return b; }
      case "pubkey": return new PublicKey(v).toBuffer();
      case "string": { const s = Buffer.from(v, "utf8"); const n = Buffer.alloc(4); n.writeUInt32LE(s.length); return Buffer.concat([n, s]); }
    }
    throw new Error("cannot encode " + t);
  }
  if (t.vec) {
    const n = Buffer.alloc(4); n.writeUInt32LE(v.length);
    return Buffer.concat([n, ...Array.from(v, (x) => encodeValue(idl, t.vec, x))]);
  }
  if (t.array) {
    if (v.length !== t.array[1]) throw new Error(`expected ${t.array[1]} items`);
    return Buffer.concat(Array.from(v, (x) => encodeValue(idl, t.array[0], x)));
  }
  if (t.defined) {
    const def = typeOf(idl, t.defined.name);
    if (def.kind === "enum") {
      const i = def.variants.findIndex((x) => x.name === v);
      if (i < 0) throw new Error(`${v} is not one of ${def.variants.map((x) => x.name)}`);
      return Buffer.from([i]);
    }
    return Buffer.concat(def.fields.map((f) => {
      if (!(f.name in v)) throw new Error("missing field " + f.name);
      return encodeValue(idl, f.type, v[f.name]);
    }));
  }
  throw new Error("cannot encode " + JSON.stringify(t));
}

// Builds one Burrpad instruction from the interface file: the accounts in the
// file's order with its signer and writable marks, then the arguments.
export function instruction(idl, name, accounts, args = {}) {
  const def = idl.instructions.find((x) => x.name === name);
  if (!def) throw new Error("no instruction " + name);
  // A group of accounts nested in another (the shadow burn takes a $BURR buy's whole list) is listed flat, in order.
  const flat = (list) => list.flatMap((a) => (a.accounts ? flat(a.accounts) : [a]));
  const keys = flat(def.accounts).map((a) => {
    const key = accounts[a.name] ?? (a.address ? new PublicKey(a.address) : undefined);
    if (!key) throw new Error(`${name}: account ${a.name} not given`);
    return { pubkey: key, isSigner: !!a.signer, isWritable: !!a.writable };
  });
  const data = Buffer.concat([
    Buffer.from(def.discriminator),
    ...def.args.map((a) => {
      if (!(a.name in args)) throw new Error(`${name}: argument ${a.name} not given`);
      return encodeValue(idl, a.type, args[a.name]);
    }),
  ]);
  return new TransactionInstruction({ programId: idl.programId, keys, data });
}

// Decodes one of the program's accounts by the interface file.
export function decodeAccount(idl, typeName, data) {
  const buf = Buffer.from(data);
  const disc = idl.accounts.find((a) => a.name === typeName).discriminator;
  if (!buf.subarray(0, 8).equals(Buffer.from(disc))) throw new Error(`not a ${typeName} account`);
  let o = 8;
  const rd = (t) => {
    if (typeof t === "string") {
      switch (t) {
        case "bool": return buf[o++] !== 0;
        case "u8": return buf[o++];
        case "u16": { const v = buf.readUInt16LE(o); o += 2; return v; }
        case "u32": { const v = buf.readUInt32LE(o); o += 4; return v; }
        case "u64": { const v = buf.readBigUInt64LE(o); o += 8; return v; }
        case "i64": { const v = buf.readBigInt64LE(o); o += 8; return v; }
        case "u128": { const lo = buf.readBigUInt64LE(o), hi = buf.readBigUInt64LE(o + 8); o += 16; return (hi << 64n) | lo; }
        case "pubkey": { const v = new PublicKey(buf.subarray(o, o + 32)); o += 32; return v.toBase58(); }
      }
      throw new Error("cannot decode " + t);
    }
    if (t.array) return Array.from({ length: t.array[1] }, () => rd(t.array[0]));
    if (t.defined) return struct(typeOf(idl, t.defined.name));
    throw new Error("cannot decode " + JSON.stringify(t));
  };
  const struct = (def) => Object.fromEntries(def.fields.map((f) => [f.name, rd(f.type)]));
  return struct(typeOf(idl, typeName));
}

export async function readAccount(conn, idl, typeName, addr) {
  const info = await conn.getAccountInfo(addr);
  if (!info) return null;
  if (!info.owner.equals(idl.programId)) throw new Error(`${typeName} account is not owned by the program`);
  return decodeAccount(idl, typeName, info.data);
}

// ---- sending ----

// What is paid on top of the base fee so a transaction is picked up, in
// millionths of a lamport per compute unit. BURRPAD_PRIORITY_FEE changes it.
const priorityFee = () => {
  const text = process.env.BURRPAD_PRIORITY_FEE || "50000";
  if (!/^[0-9]+$/.test(text)) fail("BURRPAD_PRIORITY_FEE must be a whole number.");
  if (Number(text) > 2_000_000) fail(`BURRPAD_PRIORITY_FEE is ${text}, far above anything sensible (the usual is 50000). It is in millionths of a lamport.`);
  return Number(text);
};

export const link = (conn, signature) => (conn.isMainnet ? `https://solscan.io/tx/${signature}` : signature);

// Sends one transaction and waits for it. `signers[0]` pays. The transaction
// is simulated first and is not sent if the simulation fails. Throws with
// the program's log if it fails.
export async function send(conn, instructions, signers, { table, units = 400_000 } = {}) {
  let tx, blockhash, lastValidBlockHeight;
  for (let attempt = 0; ; attempt++) {
    ({ blockhash, lastValidBlockHeight } = await conn.getLatestBlockhash());
    const message = new TransactionMessage({
      payerKey: signers[0].publicKey,
      recentBlockhash: blockhash,
      instructions: [
        ComputeBudgetProgram.setComputeUnitLimit({ units }),
        ComputeBudgetProgram.setComputeUnitPrice({ microLamports: priorityFee() }),
        ...instructions,
      ],
    }).compileToV0Message(table ? [table] : []);
    tx = new VersionedTransaction(message);
    tx.sign(signers);
    const sim = await conn.simulateTransaction(tx, { sigVerify: true, commitment: "confirmed" });
    if (!sim.value.err) break;
    // A fresh blockhash the answering server has not seen yet: ask again.
    if (JSON.stringify(sim.value.err).includes("BlockhashNotFound") && attempt < 5) { await sleep(800); continue; }
    const logs = (sim.value.logs ?? []).join("\n");
    // A program can be called from the slot after it was deployed, not before.
    if (/Program is not deployed/.test(logs) && attempt < 20) { await sleep(1000); continue; }
    throw new Error(`not sent: the simulation failed (${JSON.stringify(sim.value.err)}).\n${logs}`);
  }
  const raw = tx.serialize();
  const size = raw.length;
  // The transaction's name is known before it is sent, so a connection that
  // drops at the very moment of sending does not lose track of it: the wait
  // below finds it if it landed, and sends it again if it did not.
  const signature = bs58.encode(tx.signatures[0]);
  await conn.sendRawTransaction(raw, { skipPreflight: true, maxRetries: 0 }).catch(() => {});
  // Wait for it by asking the network directly, sending the very same signed
  // transaction again every couple of seconds (it can only ever run once),
  // until well after it could no longer land. Asking, not listening: a dropped
  // listening connection must not turn a landed transaction into "not seen".
  let status = null, expiredAt = 0, history = true, unreachable = 0;
  const started = Date.now();
  for (let turn = 0; ; turn++) {
    let answer;
    try {
      answer = await conn.getSignatureStatuses([signature], history ? { searchTransactionHistory: true } : undefined);
      unreachable = 0;
    } catch {
      history = false; // some providers refuse the history search; ask plainly next time
      unreachable++;
    }
    status = answer?.value?.[0] ?? null;
    // Only a result the network has settled on counts, good or bad.
    if (status && (status.confirmationStatus === "confirmed" || status.confirmationStatus === "finalized")) break;
    status = null;
    const height = await conn.getBlockHeight("confirmed").catch(() => { unreachable++; return 0; });
    if (height > lastValidBlockHeight) {
      if (!expiredAt) expiredAt = Date.now();
      else if (answer && Date.now() - expiredAt > 30_000) break; // only on a turn where the network really answered "not there"
    } else if (turn % 2 === 1) {
      await conn.sendRawTransaction(raw, { skipPreflight: true, maxRetries: 0 }).catch(() => {});
    }
    if (unreachable > 20 || Date.now() - started > 240_000) {
      throw new Error(`the network stopped answering while waiting for ${link(conn, signature)}\nIt may or may not have landed. Running the same command again is safe: each script looks at what is already there before sending anything.`);
    }
    await sleep(1000);
  }
  if (!status) {
    throw new Error(`sent, but it did not land: ${link(conn, signature)}\nIt can no longer land. Check the link once to be sure, then run the command again.`);
  }
  if (status.err) {
    const done = await conn.getTransaction(signature, { maxSupportedTransactionVersion: 0, commitment: "confirmed" }).catch(() => null);
    throw new Error(`failed on the network: ${link(conn, signature)} ${JSON.stringify(status.err)}\n${(done?.meta?.logMessages || []).join("\n")}`);
  }
  // Reads made right after this must see it, whichever server answers them.
  for (let i = 0; i < 20 && (await conn.getSlot("confirmed").catch(() => 0)) < status.slot; i++) await sleep(500);
  return { signature, size, link: link(conn, signature) };
}

// A transaction for a multi-signature wallet to sign: unsigned, with that
// wallet as payer, as base58 text to paste into the wallet's app.
export async function unsignedForWallet(conn, instructions, payer) {
  const tx = new Transaction({ feePayer: payer, recentBlockhash: (await conn.getLatestBlockhash()).blockhash });
  tx.add(...instructions);
  return bs58.encode(tx.serialize({ requireAllSignatures: false, verifySignatures: false }));
}

export async function loadTable(conn, addr) {
  const got = await conn.getAddressLookupTable(addr);
  if (!got.value) throw new Error("no address table at " + addr.toBase58());
  return got.value;
}
export { AddressLookupTableAccount };

// The accounts that are the same for every coin. They go in the address
// table so that a launch fits one transaction. Same list as
// `fixed_accounts()` in tests/src/kit.rs.
export function fixedAccounts(idl) {
  const solVault = pda(["sol-vault"], MAYHEM);
  return [
    idl.programId,
    configAddress(idl),
    ZEC, PUMP, PUMP_AMM, PUMP_FEES, MAYHEM,
    pda(["global"], PUMP),
    pda(["__event_authority"], PUMP),
    pda(["quote-control"], PUMP),
    pda(["fee_config", PUMP], PUMP_FEES),
    pda(["global_config"], PUMP_AMM),
    pda(["__event_authority"], PUMP_AMM),
    pda(["fee_config", PUMP_AMM], PUMP_FEES),
    pda(["mint-authority"], PUMP),
    pda(["global-params"], MAYHEM),
    solVault,
    ata(BUYBACK, ZEC, TOKEN),
    TOKEN, TOKEN_2022, ATA_PROGRAM, SYSTEM, COMPUTE_BUDGET,
  ];
}

// ---- what a run leaves behind: addresses only, never keys ----

const outFile = (name) => path.join(here, "out", name + ".json");
export function readOut(name) {
  return existsSync(outFile(name)) ? JSON.parse(readFileSync(outFile(name), "utf8")) : {};
}
export function writeOut(name, values) {
  mkdirSync(path.join(here, "out"), { recursive: true });
  writeFileSync(outFile(name), JSON.stringify({ ...readOut(name), ...values }, null, 2) + "\n");
}

export const sha256 = (data) => createHash("sha256").update(data).digest();
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// PASS / FAIL lines, and a count at the end.
export function checker() {
  let failed = 0;
  return {
    check(what, ok, detail = "") {
      if (!ok) failed++;
      console.log(`${ok ? "PASS" : "FAIL"}  ${what}${detail ? "  (" + detail + ")" : ""}`);
      return ok;
    },
    eq(what, got, want) {
      const same = String(got) === String(want);
      const short = (v) => (String(v).length > 60 ? `${String(v).slice(0, 44)}… (${Array.isArray(v) ? v.length + " items" : "long"})` : String(v));
      return this.check(what, same, same ? short(got) : `found ${got}, expected ${want}`);
    },
    finish() {
      console.log(failed ? `\n${failed} item(s) FAILED.` : "\nAll items passed.");
      process.exit(failed ? 1 : 0);
    },
  };
}

// A receiver must be an ordinary wallet that has been used. An address has no
// built-in check for typing mistakes, and coins copy the platform receiver for good.
export async function ordinaryWallet(conn, who, what) {
  const info = await conn.getAccountInfo(who);
  if (!info) fail(`${what} ${who.toBase58()} has never been used on this network. Send it a little SOL first; if it still fails, the address is wrong.`);
  if (!info.owner.equals(SYSTEM) || info.executable) fail(`${what} ${who.toBase58()} is not an ordinary wallet (it is a program's account or a token account).`);
}
