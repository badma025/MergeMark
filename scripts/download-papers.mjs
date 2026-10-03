#!/usr/bin/env node
// Download the reviewed manifest into papers/unused/.
//
//   node scripts/download-papers.mjs [--manifest papers/manifest.json] [--dest papers/unused]
//
// Skips files that already exist, rejects anything that is not a PDF (%PDF
// magic), and drops a download whose bytes are identical to a paper already in
// papers/used or papers/unused. Sequential and rate limited.

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync, unlinkSync } from "node:fs";
import path from "node:path";

const args = process.argv.slice(2);
const arg = (name, dflt) => (args.includes(name) ? args[args.indexOf(name) + 1] : dflt);
const manifestPath = arg("--manifest", "papers/manifest.json");
const dest = arg("--dest", "papers/unused");
const UA = "Mozilla/5.0 (MergeMark corpus builder; personal testing)";

const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
if (!manifest.verified) {
  console.error("manifest is not verified; run scripts/select-papers.mjs --verify first");
  process.exit(2);
}
mkdirSync(dest, { recursive: true });

const sha = (buf) => createHash("sha256").update(buf).digest("hex");
const known = new Map();
for (const dir of ["papers/used", "papers/unused"]) {
  if (!existsSync(dir)) continue;
  for (const f of readdirSync(dir)) {
    if (f.toLowerCase().endsWith(".pdf")) known.set(sha(readFileSync(path.join(dir, f))), path.join(dir, f));
  }
}

const result = { downloaded: 0, skippedExisting: 0, duplicates: 0, failed: [] };
const log = [];
for (const e of manifest.entries.filter((x) => x.ok)) {
  const target = path.join(dest, e.dest);
  if (existsSync(target)) {
    result.skippedExisting += 1;
    continue;
  }
  await new Promise((r) => setTimeout(r, 400));
  try {
    const res = await fetch(encodeURI(e.url), { headers: { "user-agent": UA }, redirect: "follow" });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const buf = Buffer.from(await res.arrayBuffer());
    if (buf.subarray(0, 5).toString("latin1") !== "%PDF-") throw new Error("not a PDF");
    const hash = sha(buf);
    if (known.has(hash)) {
      result.duplicates += 1;
      log.push({ rel: e.rel, duplicateOf: known.get(hash) });
      continue;
    }
    writeFileSync(target, buf);
    known.set(hash, target);
    result.downloaded += 1;
    log.push({ rel: e.rel, dest: target, bytes: buf.length, sha256: hash, group: e.group, url: e.url });
  } catch (error) {
    result.failed.push({ rel: e.rel, error: String(error.message || error) });
  }
}

writeFileSync(path.join(path.dirname(manifestPath), "download-log.json"), JSON.stringify({ at: new Date().toISOString(), ...result, log }, null, 2));
console.log(JSON.stringify({ ...result, failed: result.failed.length }, null, 2));
for (const f of result.failed) console.log(`  FAILED ${f.rel}: ${f.error}`);
