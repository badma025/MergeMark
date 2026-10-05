// The exam papers the Rust tests read, kept encrypted in the repository.
//
// The paper corpus (papers/) is not in git: the papers are the exam boards'
// copyright, and the repository is public. CI and other machines still need
// the papers the tests read, so an encrypted copy of exactly those is
// committed under src-tauri/fixtures/corpus/ (AES-256-GCM, one file each).
// The key is never committed: it is the CI secret MERGEMARK_FIXTURES_KEY, or
// the file ~/.mergemark/fixtures.key on a developer machine.
//
//   node scripts/fixtures.mjs record <log>   set the list from a test run's log
//   node scripts/fixtures.mjs pack           encrypt listed papers not yet stored
//   node scripts/fixtures.mjs unpack         decrypt every listed paper into papers/
//   node scripts/fixtures.mjs check [<log>]  every listed paper is stored (and, when
//                                            present in papers/, unchanged); with a
//                                            log, every paper the run read is listed
//   node scripts/fixtures.mjs keygen         write a new key to ~/.mergemark/fixtures.key
//
// The list comes from the tests themselves: with MERGEMARK_FIXTURE_LOG=<file>
// set, every paper a test asks for (test_support::corpus_paper) is appended to
// <file>. scripts/verify.sh checks its run's log, and a CI run, which has only
// the listed papers, fails under MERGEMARK_REQUIRE_FIXTURES=1 on any other, so
// the list cannot fall behind the tests. After adding a test on a new paper:
// `record` its run's log, then `pack`, and commit the manifest and new files.

import { createCipheriv, createDecipheriv, createHash, randomBytes } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { gunzipSync, gzipSync } from "node:zlib";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const STORE = path.join(ROOT, "src-tauri", "fixtures", "corpus");
const MANIFEST = path.join(STORE, "manifest.json");
const KEY_FILE = path.join(os.homedir(), ".mergemark", "fixtures.key");
const MAGIC = Buffer.from("MMFX1\0");

export function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

/// The stored file of a corpus path: papers/used/x.pdf -> used/x.pdf.enc
/// (the store must not contain a folder named "papers": .gitignore drops those).
export function storedName(corpusPath) {
  const rel = corpusPath.replace(/\\/g, "/");
  if (!/^papers\/(used|unused)\/[^/]+\.pdf$/.test(rel)) throw new Error(`not a corpus paper: ${corpusPath}`);
  return rel.replace(/^papers\//, "") + ".enc";
}

export function encrypt(key, corpusPath, plain) {
  const iv = randomBytes(12);
  const cipher = createCipheriv("aes-256-gcm", key, iv);
  cipher.setAAD(Buffer.from(corpusPath));
  const body = Buffer.concat([cipher.update(gzipSync(plain, { level: 9 })), cipher.final()]);
  return Buffer.concat([MAGIC, iv, cipher.getAuthTag(), body]);
}

export function decrypt(key, corpusPath, sealed) {
  if (!sealed.subarray(0, MAGIC.length).equals(MAGIC)) throw new Error(`${corpusPath}: not a MergeMark fixture file`);
  const at = MAGIC.length;
  const decipher = createDecipheriv("aes-256-gcm", key, sealed.subarray(at, at + 12));
  decipher.setAAD(Buffer.from(corpusPath));
  decipher.setAuthTag(sealed.subarray(at + 12, at + 28));
  return gunzipSync(Buffer.concat([decipher.update(sealed.subarray(at + 28)), decipher.final()]));
}

function loadKey() {
  const text = process.env.MERGEMARK_FIXTURES_KEY?.trim() || (existsSync(KEY_FILE) ? readFileSync(KEY_FILE, "utf8").trim() : "");
  if (!text) throw new Error(`no key: set MERGEMARK_FIXTURES_KEY or write ${KEY_FILE} (node scripts/fixtures.mjs keygen)`);
  const key = Buffer.from(text, "base64");
  if (key.length !== 32) throw new Error("the fixtures key must be 32 bytes, base64-encoded");
  return key;
}

function loadManifest() {
  return existsSync(MANIFEST) ? JSON.parse(readFileSync(MANIFEST, "utf8")) : { papers: [] };
}

function saveManifest(manifest) {
  manifest.papers.sort((a, b) => a.path.localeCompare(b.path));
  writeFileSync(MANIFEST, JSON.stringify(manifest, null, 2) + "\n");
}

function main([cmd, arg]) {
  if (cmd === "keygen") {
    if (existsSync(KEY_FILE)) throw new Error(`${KEY_FILE} exists; remove it first to replace the key`);
    mkdirSync(path.dirname(KEY_FILE), { recursive: true });
    writeFileSync(KEY_FILE, randomBytes(32).toString("base64") + "\n", { mode: 0o600 });
    console.log(`wrote ${KEY_FILE}`);
  } else if (cmd === "record") {
    const asked = [...new Set(readFileSync(arg, "utf8").split(/\r?\n/).map((l) => l.trim()).filter(Boolean))];
    const manifest = loadManifest();
    const known = new Map(manifest.papers.map((p) => [p.path, p]));
    manifest.papers = asked.map((rel) => {
      storedName(rel);
      const bytes = readFileSync(path.join(ROOT, rel));
      return { path: rel, sha256: sha256(bytes), bytes: bytes.length };
    });
    saveManifest(manifest);
    const dropped = [...known.keys()].filter((rel) => !asked.includes(rel));
    console.log(`${manifest.papers.length} papers listed${dropped.length ? `; no longer read: ${dropped.join(", ")}` : ""}`);
  } else if (cmd === "pack") {
    const key = loadKey();
    let written = 0;
    for (const p of loadManifest().papers) {
      const out = path.join(STORE, storedName(p.path));
      if (existsSync(out)) {
        try {
          if (sha256(decrypt(key, p.path, readFileSync(out))) === p.sha256) continue;
        } catch {
          // Sealed with another key or damaged: seal it again.
        }
      }
      const plain = readFileSync(path.join(ROOT, p.path));
      if (sha256(plain) !== p.sha256) throw new Error(`${p.path} changed since it was listed; run record again`);
      mkdirSync(path.dirname(out), { recursive: true });
      writeFileSync(out, encrypt(key, p.path, plain));
      written += 1;
    }
    console.log(`${written} sealed`);
  } else if (cmd === "unpack") {
    const key = loadKey();
    let restored = 0;
    for (const p of loadManifest().papers) {
      const dest = path.join(ROOT, p.path);
      if (existsSync(dest)) {
        if (sha256(readFileSync(dest)) !== p.sha256) throw new Error(`${p.path} differs from the listed paper; not overwritten`);
        continue;
      }
      const plain = decrypt(key, p.path, readFileSync(path.join(STORE, storedName(p.path))));
      if (sha256(plain) !== p.sha256) throw new Error(`${p.path}: decrypted paper does not match its hash`);
      mkdirSync(path.dirname(dest), { recursive: true });
      writeFileSync(dest, plain);
      restored += 1;
    }
    console.log(`${restored} restored into papers/`);
  } else if (cmd === "check") {
    const problems = [];
    const listed = new Set(loadManifest().papers.map((p) => p.path));
    if (arg) {
      for (const rel of new Set(readFileSync(arg, "utf8").split(/\r?\n/).map((l) => l.trim()).filter(Boolean))) {
        if (!listed.has(rel)) problems.push(`read by a test but not listed: ${rel} (record, then pack)`);
      }
    }
    for (const p of loadManifest().papers) {
      if (!existsSync(path.join(STORE, storedName(p.path)))) problems.push(`not sealed: ${p.path}`);
      const local = path.join(ROOT, p.path);
      if (existsSync(local) && sha256(readFileSync(local)) !== p.sha256) problems.push(`changed: ${p.path}`);
    }
    if (problems.length) {
      console.error(problems.join("\n"));
      process.exit(1);
    }
    console.log(`${loadManifest().papers.length} papers listed and sealed`);
  } else {
    console.error("usage: node scripts/fixtures.mjs record <log> | pack | unpack | check | keygen");
    process.exit(2);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2));
}
