// Checks that the updater signing secrets sign with the key the app trusts.
//
// A release build signs its update files with TAURI_SIGNING_PRIVATE_KEY,
// opened by TAURI_SIGNING_PRIVATE_KEY_PASSWORD, and installed copies accept
// an update only if it verifies against the public key in tauri.conf.json.
// This signs a scratch file with the two and verifies the signature against
// that public key, so a wrong secret fails in seconds instead of at the end of
// a release build. It prints key IDs and a fingerprint of the key as stored
// (the first 16 hex digits of the SHA-256 of the key file's text), never a
// secret.
//
//   node scripts/check-signing.mjs    (with both variables set)
//
// CI runs it in .github/workflows/signing-check.yml and at the start of
// release.yml.

import { spawnSync } from "node:child_process";
import { createHash, createPublicKey, verify } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const fail = (message) => {
  console.error(`signing check failed: ${message}`);
  process.exit(1);
};
// minisign stores a key ID as 8 bytes, least significant first.
const keyId = (bytes) => Buffer.from(bytes).reverse().toString("hex").toUpperCase();
// A minisign public key or signature in Tauri's form: base64 of minisign's
// text file, whose second line is base64 of the binary record.
const record = (b64) => Buffer.from(Buffer.from(b64.trim(), "base64").toString("utf8").split("\n")[1], "base64");

// The public key: "Ed", the key ID, the 32-byte Ed25519 key.
const conf = JSON.parse(readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8"));
const pub = record(conf.plugins.updater.pubkey);
const trusted = keyId(pub.subarray(2, 10));
console.log(`the app trusts key ${trusted}`);

const key = (process.env.TAURI_SIGNING_PRIVATE_KEY ?? "").trim();
if (!key) fail("TAURI_SIGNING_PRIVATE_KEY is not set");
// Unset, the signer would wait for a password typed at the console.
if (process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD === undefined) fail("TAURI_SIGNING_PRIVATE_KEY_PASSWORD is not set");
console.log(`the key secret's fingerprint is ${createHash("sha256").update(key).digest("hex").slice(0, 16)}`);

const dir = mkdtempSync(path.join(tmpdir(), "mergemark-signing-"));
try {
  const file = path.join(dir, "check.txt");
  writeFileSync(file, "MergeMark signing check\n");
  // The Tauri CLI reads the key and its password from the environment.
  const run = spawnSync(process.execPath, [path.join(root, "node_modules/@tauri-apps/cli/tauri.js"), "signer", "sign", file], {
    encoding: "utf8",
    env: { ...process.env, TAURI_SIGNING_PRIVATE_KEY: key },
  });
  const said = `${run.stderr ?? ""}${run.stdout ?? ""}`;
  if (run.status !== 0) {
    fail(/Wrong password/.test(said) ? "the password secret does not open the key secret" : `the signer failed: ${said.trim() || run.error}`);
  }
  // The signature: the algorithm ("ED": over the file's BLAKE2b-512, "Ed":
  // over the file itself), the key ID, the 64-byte signature.
  const sig = record(readFileSync(`${file}.sig`, "utf8"));
  const signer = keyId(sig.subarray(2, 10));
  if (signer !== trusted) fail(`the secrets sign with key ${signer}, but the app trusts ${trusted}`);
  const content = readFileSync(file);
  const message = sig.subarray(0, 2).toString("latin1") === "ED" ? createHash("blake2b512").update(content).digest() : content;
  const publicKey = createPublicKey({ key: { kty: "OKP", crv: "Ed25519", x: pub.subarray(10, 42).toString("base64url") }, format: "jwk" });
  if (!verify(null, message, publicKey, sig.subarray(10, 74))) fail("the signature does not verify against the app's public key");
  console.log(`the secrets sign with key ${signer}, the key the app trusts`);
} finally {
  rmSync(dir, { recursive: true, force: true });
}
