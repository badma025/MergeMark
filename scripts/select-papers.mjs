#!/usr/bin/env node
// Pick a balanced triage set from papers/manifest-raw.json.
//
//   node scripts/select-papers.mjs [--in papers/manifest-raw.json] [--out papers/manifest.json] [--verify]
//
// Deterministic: within each quota the candidates are ordered by year and taken
// at evenly spaced positions, so a re-run selects the same papers. Variants
// (v2-v4), specimens, mock and practice papers are skipped unless nothing else
// is left. With --verify each pick is checked with a HEAD request (PDF, size).

import { readFileSync, writeFileSync } from "node:fs";

const args = process.argv.slice(2);
const arg = (name, dflt) => (args.includes(name) ? args[args.indexOf(name) + 1] : dflt);
const inPath = arg("--in", "papers/manifest-raw.json");
const outPath = arg("--out", "papers/manifest.json");
const verify = args.includes("--verify");

const raw = JSON.parse(readFileSync(inPath, "utf8")).entries;

// [group, path prefix (matched on the folder, then "/"), how many]
const QUOTAS = [
  ["same-boards", "Maths/A-level/Papers/Edexcel/", 6],
  ["same-boards", "Maths/A-level/Papers/Edexcel-Further/", 6],
  ["same-boards", "Maths/A-level/Papers/AQA/", 4],
  ["same-boards", "Maths/A-level/Papers/AQA-Further/", 4],
  ["same-boards", "Maths/A-level/Papers/CIE/", 8],
  ["same-boards", "Physics/A-level/Past-Papers/AQA/", 6],
  ["same-boards", "Physics/A-level/Past-Papers/Edexcel/", 6],

  ["new-boards", "Maths/A-level/Papers/OCR-MEI/", 4],
  ["new-boards", "Maths/A-level/Papers/OCR-MEI-Further/", 4],
  ["new-boards", "Maths/A-level/Papers/OCR-Further/", 4],
  ["new-boards", "Maths/A-level/Papers/WJEC/", 3],
  ["new-boards", "Maths/A-level/Papers/WJEC-Further/", 2],
  ["new-boards", "Physics/A-level/Past-Papers/OCR-A/", 5],
  ["new-boards", "Physics/A-level/Past-Papers/OCR-B/", 3],
  ["new-boards", "Physics/A-level/Past-Papers/WJEC-Wales/", 3],
  ["new-boards", "Computer-Science/A-level/Past-Papers/Eduqas/", 2],

  ["other-subjects", "Chemistry/A-level/Past-Papers/AQA/", 4],
  ["other-subjects", "Chemistry/A-level/Past-Papers/Edexcel/", 4],
  ["other-subjects", "Chemistry/A-level/Past-Papers/OCR-A/", 4],
  ["other-subjects", "Biology/A-level/Past-Papers/AQA/", 4],
  ["other-subjects", "Biology/A-level/Past-Papers/Edexcel-A/", 3],
  ["other-subjects", "Biology/A-level/Past-Papers/OCR-A/", 3],
  ["other-subjects", "Economics/A-level/Past-Papers/AQA/", 3],
  ["other-subjects", "Economics/A-level/Past-Papers/Edexcel-A/", 3],
  ["other-subjects", "Economics/A-level/Past-Papers/OCR/", 2],

  ["gcse", "Maths/GCSE/Past-Papers/Edexcel/", 4],
  ["gcse", "Maths/GCSE/Past-Papers/AQA/", 4],
  ["gcse", "Maths/GCSE/Past-Papers/OCR/", 3],
  ["gcse", "Maths/GCSE/Past-Papers/Edexcel-IGCSE-A/", 3],
  ["gcse", "Maths/GCSE/Past-Papers/CIE/", 3],
  ["gcse", "Physics/GCSE/Past-Papers/AQA/", 3],
  ["gcse", "Physics/GCSE/Past-Papers/Edexcel/", 2],
  ["gcse", "Physics/GCSE/Past-Papers/OCR-A/", 2],
  ["gcse", "Chemistry/GCSE/Past-Papers/AQA/", 3],
  ["gcse", "Biology/GCSE/Past-Papers/AQA/", 3],
  ["gcse", "Biology/GCSE/Past-Papers/Edexcel/", 2],

  ["older-awkward", "Maths/A-level/C3/Papers-Edexcel/", 3],
  ["older-awkward", "Maths/A-level/C4/Papers-Edexcel/", 3],
  ["older-awkward", "Maths/A-level/M2/Papers-Edexcel/", 3],
  ["older-awkward", "Maths/A-level/M3/Papers-Edexcel/", 2],
  ["older-awkward", "Maths/A-level/FP2/Papers-Edexcel/", 2],
  ["older-awkward", "Maths/A-level/S1/Papers-Edexcel/", 2],
  ["older-awkward", "Physics/A-level/Past-Papers/AQA-Old/", 3],
  ["older-awkward", "Maths/GCSE/Past-Papers/Edexcel-A-Old/", 2],
];

const skippable = (e) => /\(v[2-9]\)|specimen|mock|practice|sample/i.test(e.file) || /Practice|Mock|Specimen/i.test(e.rel);

function spread(list, n) {
  if (list.length <= n) return list;
  const out = [];
  for (let i = 0; i < n; i += 1) out.push(list[Math.floor(((i + 0.5) * list.length) / n)]);
  return out;
}

const picked = [];
const shortfalls = [];
const seen = new Set();
for (const [group, prefix, count] of QUOTAS) {
  const pool = raw.filter((e) => e.rel.startsWith(prefix));
  let candidates = pool.filter((e) => !skippable(e));
  if (candidates.length < count) candidates = pool;
  candidates = candidates
    .map((e) => ({ ...e, _y: Number(e.year) || 0 }))
    .sort((a, b) => a._y - b._y || a.rel.localeCompare(b.rel));
  const chosen = spread(candidates, count);
  if (chosen.length < count) shortfalls.push(`${prefix}: wanted ${count}, have ${chosen.length}`);
  for (const e of chosen) {
    if (seen.has(e.url)) continue;
    seen.add(e.url);
    const { _y, ...entry } = e;
    const slug = prefix.replace(/\/$/, "").replace(/[\\/]+/g, "_");
    entry.group = group;
    entry.dest = `${slug}__${e.file.replace(/[^A-Za-z0-9._()-]+/g, "_")}.pdf`;
    picked.push(entry);
  }
}

if (verify) {
  for (const e of picked) {
    await new Promise((r) => setTimeout(r, 300));
    try {
      const res = await fetch(encodeURI(e.url), { method: "HEAD", headers: { "user-agent": "Mozilla/5.0 (MergeMark corpus builder; personal testing)" }, redirect: "follow" });
      e.status = res.status;
      e.contentType = res.headers.get("content-type");
      e.bytes = Number(res.headers.get("content-length")) || null;
      e.ok = res.ok && /pdf/i.test(e.contentType || "");
    } catch (error) {
      e.ok = false;
      e.error = String(error.message || error);
    }
  }
}

writeFileSync(outPath, JSON.stringify({ generated: new Date().toISOString(), verified: verify, count: picked.length, entries: picked }, null, 2));

const byGroup = {};
for (const e of picked) byGroup[e.group] = (byGroup[e.group] || 0) + 1;
console.log(`selected ${picked.length}`, byGroup);
if (shortfalls.length) console.log("shortfalls:\n  " + shortfalls.join("\n  "));
if (verify) {
  const ok = picked.filter((e) => e.ok);
  const bad = picked.filter((e) => !e.ok);
  const mb = ok.reduce((s, e) => s + (e.bytes || 0), 0) / 1e6;
  console.log(`verified ok=${ok.length} bad=${bad.length} total=${mb.toFixed(1)} MB (${ok.filter((e) => !e.bytes).length} without a size)`);
  for (const e of bad) console.log(`  BAD ${e.status ?? e.error} ${e.rel}`);
}
