#!/usr/bin/env node
// Build a manifest of public question-paper PDFs for the triage corpus.
//
//   node scripts/build-papers-manifest.mjs [--out papers/manifest.json] [--verify]
//
// Crawls the Physics & Maths Tutor listing pages (HTML only, rate limited),
// collects direct links to QUESTION papers (never mark schemes, examiner
// reports or data sheets) and, with --verify, confirms each one is a PDF and
// records its size using a HEAD request. It never saves a PDF; downloading is
// scripts/download-papers.mjs and needs the reviewed manifest.

import { mkdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import path from "node:path";

const UA = "Mozilla/5.0 (MergeMark corpus builder; personal testing)";
const DELAY_MS = 350;
const ROOT = "https://www.physicsandmathstutor.com";

const args = process.argv.slice(2);
const out = args.includes("--out") ? args[args.indexOf("--out") + 1] : "papers/manifest.json";
const verify = args.includes("--verify");

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const htmlCache = new Map();

async function getHtml(url) {
  if (htmlCache.has(url)) return htmlCache.get(url);
  await sleep(DELAY_MS);
  try {
    const res = await fetch(url, { headers: { "user-agent": UA }, redirect: "follow" });
    const text = res.ok ? await res.text() : "";
    htmlCache.set(url, text);
    return text;
  } catch {
    htmlCache.set(url, "");
    return "";
  }
}

function links(html) {
  return [...html.matchAll(/href="([^"]+)"/g)].map((m) => m[1].replace(/&amp;/g, "&"));
}

const isPdf = (u) => /^https:\/\/pmt\.physicsandmathstutor\.com\/download\/.+\.pdf$/i.test(u);

// A question paper: lives under a /QP/ folder, or is named "... QP.pdf".
// Reject anything that is a mark scheme, report, insert, data sheet or formula booklet.
function isQuestionPaper(u) {
  const rel = decodeURIComponent(u.split("/download/")[1]);
  if (/(^|\/)(MS|ER|Insert|Inserts|Source|Sources)(\/|$)/i.test(rel)) return false;
  if (/(mark ?scheme|examiner|data ?sheet|formula|insert|solution|\bMS\b|\bER\b)/i.test(rel)) return false;
  return /(^|\/)QP(\/|$)/i.test(rel) || /\bQP\b/i.test(rel);
}

// Subject hubs to crawl for sub-pages, then the sub-pages for PDF links.
const HUBS = [
  "/past-papers/a-level-physics/",
  "/past-papers/a-level-chemistry/",
  "/past-papers/a-level-biology/",
  "/past-papers/a-level-economics/",
  "/past-papers/a-level-computer-science/",
  "/past-papers/gcse-physics/",
  "/past-papers/gcse-chemistry/",
  "/past-papers/gcse-biology/",
  "/past-papers/gcse-maths/",
  "/past-papers/gcse-computer-science/",
  "/past-papers/gcse-economics/",
  "/maths-revision/a-level-edexcel/papers/",
  "/maths-revision/a-level-aqa/papers/",
  "/maths-revision/a-level-ocr/papers/",
  "/maths-revision/a-level-ocr-mei/papers/",
  "/maths-revision/a-level-cie/papers/",
  "/maths-revision/gcse-edexcel/papers/",
  "/maths-revision/gcse-aqa/papers/",
  "/maths-revision/gcse-ocr/papers/",
  "/a-level-maths-papers/",
];

const SUBPAGE = /^https:\/\/www\.physicsandmathstutor\.com\/(past-papers|maths-revision|a-level-maths-papers)\/[^#?]+\/$/i;

const found = new Map(); // url -> entry

function describe(u) {
  const rel = decodeURIComponent(u.split("/download/")[1]);
  const parts = rel.split("/");
  const file = parts[parts.length - 1].replace(/\.pdf$/i, "");
  const year = (file.match(/\b(19|20)\d{2}\b/) || [])[0] || null;
  return { url: u, rel, subject: parts[0], level: parts[1] || null, board: parts.find((p) => /^(AQA|Edexcel|OCR|OCR-MEI|MEI|CIE|WJEC|Eduqas|IAL|Cambridge)/i.test(p)) || null, file, year };
}

const seenPages = new Set();
for (const hub of HUBS) {
  const hubUrl = ROOT + hub;
  const html = await getHtml(hubUrl);
  if (!html) {
    console.error(`[skip] hub unreachable or missing: ${hub}`);
    continue;
  }
  const hubLinks = links(html);
  for (const l of hubLinks) if (isPdf(l) && isQuestionPaper(l)) found.set(l, describe(l));
  const subs = [...new Set(hubLinks.filter((l) => SUBPAGE.test(l)))];
  for (const sub of subs) {
    if (seenPages.has(sub)) continue;
    seenPages.add(sub);
    const subHtml = await getHtml(sub);
    for (const l of links(subHtml)) if (isPdf(l) && isQuestionPaper(l)) found.set(l, describe(l));
  }
  console.error(`[crawl] ${hub}: ${subs.length} sub-pages, ${found.size} question papers so far`);
}

let entries = [...found.values()].sort((a, b) => a.rel.localeCompare(b.rel));

if (verify) {
  let i = 0;
  for (const e of entries) {
    await sleep(DELAY_MS);
    i += 1;
    try {
      const res = await fetch(encodeURI(e.url), { method: "HEAD", headers: { "user-agent": UA }, redirect: "follow" });
      e.status = res.status;
      e.contentType = res.headers.get("content-type");
      e.bytes = Number(res.headers.get("content-length")) || null;
      e.ok = res.ok && /pdf/i.test(e.contentType || "");
    } catch (error) {
      e.ok = false;
      e.error = String(error.message || error);
    }
    if (i % 25 === 0) console.error(`[verify] ${i}/${entries.length}`);
  }
}

mkdirSync(path.dirname(out), { recursive: true });
writeFileSync(out, JSON.stringify({ generated: new Date().toISOString(), source: ROOT, verified: verify, count: entries.length, entries }, null, 2));
console.log(`wrote ${entries.length} entries to ${out}`);
