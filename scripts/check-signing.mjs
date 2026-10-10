// Checks that the updater signing secrets sign with the key the app trusts.
//
// A release build signs its update files with TAURI_SIGNING_PRIVATE_KEY,
// opened by TAURI_SIGNING_PRIVATE_KEY_PASSWORD, and installed copies accept
// an update only if it verifies against the public key in tauri.conf.json.
// This signs a scratch file with the two and verifies the signature against
// that public key, so a wrong secret fails in seconds instead of at the end of
// a release build.
//
// Its log is public (CI on a public repository): it prints key IDs, fixed
// messages, and a fingerprint of the key secret (the first 16 hex digits of
// the SHA-256 of its text) only once that text is known to be a minisign
// secret key, itself encrypted. Never the signer's own output, which can
// quote what it was given, and nothing derived from the password.
//
//   node scripts/check-signing.mjs    (with both variables set)
//
// CI runs it in .github/workflows/signing-check.yml and at the start of
// release.yml; scripts/set-signing-secrets.mjs runs it before it sets the two.

import { spawnSync } from "node:child_process";
import { createHash, createPublicKey, verify } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
class CheckFailed extends Error {}
const fail = (message) => {
  throw new CheckFailed(message);
};
// minisign stores a key ID as 8 bytes, least significant first.
const keyId = (bytes) => Buffer.from(bytes).reverse().toString("hex").toUpperCase();
// Tauri's form of a minisign key or signature: base64 of minisign's text
// file, an "untrusted comment:" line and then base64 of the binary record.
const record = (b64) => {
  const lines = Buffer.from(b64, "base64").toString("utf8").split("\n");
  return lines[0].startsWith("untrusted comment:") && lines[1] ? Buffer.from(lines[1], "base64") : null;
};

const dir = mkdtempSync(path.join(tmpdir(), "mergemark-signing-"));
try {
  // The public key: "Ed", the key ID, the 32-byte Ed25519 key.
  const conf = JSON.parse(readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8"));
  const pub = record(conf.plugins.updater.pubkey);
  if (pub?.length !== 42) fail("tauri.conf.json's updater public key is not a minisign public key");
  const trusted = keyId(pub.subarray(2, 10));
  console.log(`the app trusts key ${trusted}`);

  // Exactly as the release build gets it.
  const key = process.env.TAURI_SIGNING_PRIVATE_KEY ?? "";
  if (!key) fail("TAURI_SIGNING_PRIVATE_KEY is not set");
  // An encrypted secret key: "Ed", the KDF and checksum algorithms, the
  // salt, the KDF limits, the encrypted key: 158 bytes.
  const box = record(key);
  if (box?.length !== 158 || box.subarray(0, 2).toString("latin1") !== "Ed") {
    fail("TAURI_SIGNING_PRIVATE_KEY is not a minisign secret key in Tauri's form (are the two secrets swapped?)");
  }
  console.log(`the key secret's fingerprint is ${createHash("sha256").update(key).digest("hex").slice(0, 16)}`);
  // (In Actions a missing secret arrives empty; unset, which only a local
  // run can be, the signer would wait for a password typed at the console.)
  if (process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD === undefined) fail("TAURI_SIGNING_PRIVATE_KEY_PASSWORD is not set");

  const file = path.join(dir, "check.txt");
  writeFileSync(file, "MergeMark signing check\n");
  // The Tauri CLI reads the key and its password from the environment.
  const run = spawnSync(process.execPath, [path.join(root, "node_modules/@tauri-apps/cli/tauri.js"), "signer", "sign", file], { encoding: "utf8" });
  if (run.status !== 0) {
    fail(/Wrong password/.test(`${run.stderr ?? ""}${run.stdout ?? ""}`) ? "the password secret does not open the key secret" : `the signer could not sign with the key secret (exit ${run.status ?? run.signal ?? run.error?.code})`);
  }
  // The signature: the algorithm ("ED": over the file's BLAKE2b-512, "Ed":
  // over the file itself), the key ID, the 64-byte signature.
  const sig = record(readFileSync(`${file}.sig`, "utf8"));
  if (sig?.length !== 74) fail("the signer wrote no minisign signature");
  const signer = keyId(sig.subarray(2, 10));
  if (signer !== trusted) fail(`the secrets sign with key ${signer}, but the app trusts ${trusted}`);
  const content = readFileSync(file);
  const message = sig.subarray(0, 2).toString("latin1") === "ED" ? createHash("blake2b512").update(content).digest() : content;
  const publicKey = createPublicKey({ key: { kty: "OKP", crv: "Ed25519", x: pub.subarray(10, 42).toString("base64url") }, format: "jwk" });
  if (!verify(null, message, publicKey, sig.subarray(10, 74))) fail("the signature does not verify against the app's public key");
  console.log(`the secrets sign with key ${signer}, the key the app trusts`);
} catch (error) {
  if (!(error instanceof CheckFailed)) throw error;
  console.error(`signing check failed: ${error.message}`);
  process.exitCode = 1;
} finally {
  rmSync(dir, { recursive: true, force: true });
}
