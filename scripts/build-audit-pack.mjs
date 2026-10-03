#!/usr/bin/env node
// Draw a stratified, seeded sample of imported cards from a triage run and
// render the matching source pages beside each one, for a card-versus-page audit.
//
//   node scripts/build-audit-pack.mjs --run output/triage/run-unused-001
//        [--out <dir>] [--seed 7] [--dpi 90] [--papers papers/unused]
//        [--ids <file>]   (exactly the cards listed, one "<slug>-q<n>" per line)
//
// Output: <out>/manifest.json plus <out>/<id>/{card.md,page-NN.png}. Nothing is
// changed in the run itself. The sample is reproducible for a given seed.

import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync, existsSync, readdirSync, rmSync } from "node:fs";
import path from "node:path";


const args = process.argv.slice(2);
const arg = (n, d = null) => (args.includes(n) ? args[args.indexOf(n) + 1] : d);
const runDir = arg("--run");
if (!runDir) throw new Error("usage: build-audit-pack.mjs --run <triage run dir>");
const out = arg("--out", path.join(runDir, "audit-pack"));
const seed = Number(arg("--seed", "7"));
const dpi = Number(arg("--dpi", "90"));
const papersDir = arg("--papers", "papers/unused");

const results = JSON.parse(readFileSync(path.join(runDir, "results.json"), "utf8"));

// mulberry32: tiny seeded RNG so the sample can be re-drawn exactly
let s = seed >>> 0;
const rnd = () => {
  s = (s + 0x6d2b79f5) >>> 0;
  let t = s;
  t = Math.imul(t ^ (t >>> 15), t | 1);
  t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
};
const shuffle = (a) => {
  const b = [...a];
  for (let i = b.length - 1; i > 0; i -= 1) {
    const j = Math.floor(rnd() * (i + 1));
    [b[i], b[j]] = [b[j], b[i]];
  }
  return b;
};

// every card with enough context to sample from
const pool = [];
for (const r of results) {
  if (!r.cardsJson || !existsSync(r.cardsJson)) continue;
  const cards = JSON.parse(readFileSync(r.cardsJson, "utf8"));
  const flaggedQs = new Set(r.findings.filter((f) => f.question != null).map((f) => Number(f.question)));
  for (const c of cards) {
    const q = Number(c.question_number);
    pool.push({
      r,
      c,
      q,
      cardFindings: r.findings.filter((f) => Number(f.question) === q).map((f) => f.code),
      hasGlyph: String(c.content).includes("�"),
      zeroMarks: !c.marks,
      needsReview: !!c.needs_review,
      cleanCard: !flaggedQs.has(q) && !c.needs_review,
    });
  }
}

const picked = [];
const perPaper = new Map();
function take(stratum, count, filter) {
  let n = 0;
  for (const p of shuffle(pool.filter(filter))) {
    if (n >= count) break;
    const id = `${p.r.slug}-q${p.q}`;
    if (picked.some((x) => x.id === id)) continue;
    if ((perPaper.get(p.r.slug) ?? 0) >= 3) continue;
    perPaper.set(p.r.slug, (perPaper.get(p.r.slug) ?? 0) + 1);
    picked.push({ id, stratum, ...p });
    n += 1;
  }
  return n;
}

// --ids <file>: exactly the cards listed (one "<slug>-q<n>" per line), for a
// targeted check of the cards a change touched, instead of the strata.
const idsFile = arg("--ids");
const passing = (p) => p.r.status === "PASS" || p.r.status === "REVIEW";
if (idsFile) {
  const wanted = new Set(readFileSync(idsFile, "utf8").split(/\r?\n/).map((l) => l.trim()).filter(Boolean));
  for (const p of pool) {
    const base = `${p.r.slug}-q${p.q}`;
    if (!wanted.has(base)) continue;
    // (A paper can carry two cards with one question number.)
    const n = picked.filter((x) => x.base === base).length;
    picked.push({ id: n ? `${base}-${n + 1}` : base, base, stratum: "T-targeted", ...p });
  }
  const found = new Set(picked.map((x) => x.base));
  for (const id of wanted) if (!found.has(id)) console.warn(`no card ${id} in this run`);
} else {
  take("A-clean-pass", 14, (p) => passing(p));
  take("B-unflagged-in-failing-paper", 10, (p) => !passing(p) && p.cleanCard);
  take("C-needs-review", 8, (p) => p.needsReview);
  take("D-unidentified-glyph", 5, (p) => p.hasGlyph);
  take("E-marks-missing", 6, (p) => p.zeroMarks || p.cardFindings.includes("marks-tag-undercount"));
  take("F-collapsed-or-gap", 5, (p) => p.r.findings.some((f) => ["id-gap", "low-text-coverage"].includes(f.code)) || p.r.stats.cards <= 2);
}

mkdirSync(out, { recursive: true });
for (const e of readdirSync(out)) rmSync(path.join(out, e), { recursive: true, force: true });

const tokens = (str) => new Set(String(str).toLowerCase().replace(/\\[a-z]+/g, " ").match(/[a-z]{5,}/g) ?? []);
const manifest = [];
// Each page's own text, read page by page: pdftotext splits its whole-file
// output at form feeds, and some pages carry a form feed in their text.
const pageCache = new Map();
function pageTexts(pdf) {
  const count = Number(/Pages:\s+(\d+)/.exec(execFileSync("pdfinfo", [pdf], { encoding: "utf8" }))?.[1] ?? 0);
  return Array.from({ length: count }, (_, i) => {
    try {
      return execFileSync("pdftotext", ["-layout", "-f", String(i + 1), "-l", String(i + 1), pdf, "-"], { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
    } catch {
      return "";
    }
  });
}
for (const p of picked) {
  // (A paper moved to papers/used/ as a fixture after the run is read there.)
  const pdf = [papersDir, "papers/used"].map((d) => path.join(d, p.r.file)).find((f) => existsSync(f)) ?? path.join(papersDir, p.r.file);
  if (!pageCache.has(pdf)) pageCache.set(pdf, pageTexts(pdf).map(tokens));
  const pages = pageCache.get(pdf);
  const want = tokens(p.c.content);
  const scores = pages.map((set) => {
    let hit = 0;
    for (const t of want) if (set.has(t)) hit += 1;
    return hit;
  });
  const best = scores.indexOf(Math.max(...scores));
  const chosen = [best];
  for (const dir of [-1, 1]) {
    for (let i = best + dir, k = 0; k < 2 && scores[i] !== undefined && scores[i] >= 0.35 * scores[best]; i += dir, k += 1) {
      chosen.push(i);
    }
  }
  chosen.sort((a, b) => a - b);
  const dir = path.join(out, p.id);
  mkdirSync(dir, { recursive: true });
  const pngs = [];
  for (const idx of chosen) {
    const base = path.join(dir, `page-${String(idx + 1).padStart(2, "0")}`);
    execFileSync("pdftoppm", ["-r", String(dpi), "-f", String(idx + 1), "-l", String(idx + 1), "-png", "-singlefile", pdf, base]);
    pngs.push(path.resolve(`${base}.png`));
  }
  writeFileSync(path.join(dir, "card.md"), String(p.c.content));
  const figures = [...String(p.c.content).matchAll(/!\[[^\]]*\]\(([^)]+)\)/g)].map((m) => path.resolve(m[1])).filter((f) => existsSync(f));
  manifest.push({
    id: p.id,
    stratum: p.stratum,
    paper: p.r.file,
    subject: p.r.subject,
    paperStatus: p.r.status,
    question: p.q,
    marks: p.c.marks,
    needsReview: p.needsReview,
    automaticFindings: p.cardFindings,
    pages: chosen.map((i) => i + 1),
    cardFile: path.resolve(path.join(dir, "card.md")),
    pagePngs: pngs,
    figurePngs: figures,
  });
  console.log(`${p.stratum.padEnd(30)} ${p.r.file.slice(0, 48).padEnd(50)} Q${p.q} pages ${chosen.map((i) => i + 1).join(",")}`);
}
writeFileSync(path.join(out, "manifest.json"), JSON.stringify({ seed, dpi, run: runDir, count: manifest.length, items: manifest }, null, 2));
console.log(`\n${manifest.length} cards in ${out}`);
