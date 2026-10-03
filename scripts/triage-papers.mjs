#!/usr/bin/env node
// Offline triage of a folder of question papers that have no hand-made
// expected-IDs entry. Imports every paper with the real offline pipeline
// (zero network, zero tokens), then applies the source-independent invariants
// in scripts/lib/triage-invariants.mjs and the app's Markdown/KaTeX renderer.
//
//   node scripts/triage-papers.mjs [--dir papers/unused] [--stamp <id>]
//        [--out output/triage] [--only substr,substr] [--limit N]
//        [--manifest papers/manifest.json] [--no-render] [--timeout-min 20]
//
// Resumable: finished papers are appended to results.jsonl as they complete
// and are skipped when the same --stamp is run again. This script changes no
// pipeline code. Exit code is 1 when any paper is FAIL.

import { spawnSync } from "node:child_process";
import { appendFileSync, closeSync, existsSync, mkdirSync, openSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import path from "node:path";

import { runPaper, slugFor } from "./lib/zero-cost-accounting.mjs";
import { checkPaper, classify } from "./lib/triage-invariants.mjs";

const args = process.argv.slice(2);
const arg = (name, dflt = null) => (args.includes(name) ? args[args.indexOf(name) + 1] : dflt);
const dir = arg("--dir", "papers/unused");
const stamp = arg("--stamp", new Date().toISOString().replace(/[-:T]/g, "").slice(0, 12));
const outBase = arg("--out", "output/triage");
const only = arg("--only") ? arg("--only").split(",").map((s) => s.trim().toLowerCase()).filter(Boolean) : null;
const limit = arg("--limit") ? Number(arg("--limit")) : Infinity;
const manifestPath = arg("--manifest", "papers/manifest.json");
const timeoutMs = Number(arg("--timeout-min", "20")) * 60_000;
const skipRender = args.includes("--no-render");
const rescore = args.includes("--rescore");

const outRoot = path.join(outBase, `run-${stamp}`);
const logsDir = path.join(outRoot, "logs");
const resultsPath = path.join(outRoot, "results.jsonl");
mkdirSync(logsDir, { recursive: true });

// Mark schemes, solutions and examiner reports are not question papers.
const notQuestionPaper = (f) => /(_ms\b|\bms\b|mark ?scheme|solution|examiner|_er\b)/i.test(f);

// group/board for papers that came from the downloaded manifest
const meta = new Map();
if (existsSync(manifestPath)) {
  for (const e of JSON.parse(readFileSync(manifestPath, "utf8")).entries) meta.set(e.dest, e);
}

function inferSubject(pdf, name) {
  const m = meta.get(name);
  const fromPath = m?.subject ?? null;
  const text = (() => {
    try {
      return spawnSync("pdftotext", ["-l", "2", pdf, "-"], { encoding: "utf8", maxBuffer: 32 * 1024 * 1024 }).stdout ?? "";
    } catch {
      return "";
    }
  })().toLowerCase();
  const guess =
    fromPath ??
    (/computer science/.test(text) ? "Computer Science" :
     /\bphysics\b/.test(text) ? "Physics" :
     /\bchemistry\b/.test(text) ? "Chemistry" :
     /\bbiology\b/.test(text) ? "Biology" :
     /\beconomics\b/.test(text) ? "Economics" : "Mathematics");
  const subject = { Maths: "Mathematics", "Computer-Science": "Computer Science" }[guess] ?? guess;
  return { subject, module: m ? `${subject} (${m.level ?? "triage"})` : subject };
}

let files = readdirSync(dir)
  .filter((f) => f.toLowerCase().endsWith(".pdf") && !notQuestionPaper(f))
  .sort();
if (only) files = files.filter((f) => only.some((o) => f.toLowerCase().includes(o)));
files = files.slice(0, limit);

const done = new Map();
if (existsSync(resultsPath)) {
  for (const line of readFileSync(resultsPath, "utf8").split("\n")) {
    if (!line.trim()) continue;
    const r = JSON.parse(line);
    done.set(r.slug, r);
  }
}

function paperSpawn(logFile) {
  return (cmd, cargoArgs) => {
    const fd = openSync(logFile, "a");
    try {
      return spawnSync(cmd, cargoArgs, { stdio: ["ignore", fd, fd], timeout: timeoutMs, killSignal: "SIGKILL" });
    } finally {
      closeSync(fd);
    }
  };
}

console.log(`[triage] ${files.length} question papers in ${dir}; run ${stamp}; ${done.size} already done`);
let n = 0;
for (const f of files) {
  n += 1;
  const name = f.replace(/\.pdf$/i, "").replace(/[^A-Za-z0-9_-]+/g, "_").slice(0, 80);
  const slug = slugFor(name);
  if (done.has(slug) && rescore) {
    // Re-apply the current invariants to the saved import without re-running it.
    const old = done.get(slug);
    const cj = old.cardsJson;
    let cards = null;
    try { cards = JSON.parse(readFileSync(cj, "utf8")); } catch { /* none */ }
    const certPath = cj.replace(/_cards\.json$/, "_offline_certification.json");
    let cert = null;
    try { cert = JSON.parse(readFileSync(certPath, "utf8")); } catch { /* none */ }
    const inv = checkPaper({ cards, sourcePdf: path.join(dir, f), cardsDir: path.dirname(cj), cert });
    old.stats = inv.stats;
    old.findings = inv.findings;
    old.status = classify(inv.findings, old.runFailures);
    continue;
  }
  if (done.has(slug)) continue;
  const file = path.join(dir, f).replaceAll("\\", "/");
  const { subject, module } = inferSubject(file, f);
  const started = Date.now();
  const logFile = path.join(logsDir, `${slug}.log`);
  console.log(`[triage] (${n}/${files.length}) ${f}  [${subject}]`);
  const row = runPaper({
    paper: { name, file, subject, module, cohort: "triage" },
    spawn: paperSpawn(logFile),
    outRoot,
    runId: stamp,
    fixture: null,
  });
  const paperOut = path.join(row.runDir, name);
  const cardsJson = path.join(paperOut, `${slug}_cards.json`);
  const certJson = path.join(paperOut, `${slug}_offline_certification.json`);
  let cards = null;
  let cert = null;
  try { cards = JSON.parse(readFileSync(cardsJson, "utf8")); } catch { /* reported below */ }
  try { cert = JSON.parse(readFileSync(certJson, "utf8")); } catch { /* reported below */ }
  const inv = checkPaper({ cards, sourcePdf: file, cardsDir: paperOut, cert });
  const m = meta.get(f);
  const result = {
    slug,
    name,
    file: f,
    group: m?.group ?? "original",
    board: m?.board ?? null,
    subject,
    seconds: Math.round((Date.now() - started) / 1000),
    runStatus: row.status,
    runFailures: row.failures,
    counters: row.counters,
    stats: inv.stats,
    findings: inv.findings,
    cardsJson: cardsJson.replaceAll("\\", "/"),
    status: classify(inv.findings, row.failures),
  };
  appendFileSync(resultsPath, JSON.stringify(result) + "\n");
  done.set(slug, result);
  console.log(`         -> ${result.status} in ${result.seconds}s ${JSON.stringify(inv.stats.marksTotal ?? "")} cards=${inv.stats.cards ?? 0}`);
}

// Render every card through the app's Markdown + KaTeX pipeline.
const rows = [...done.values()];
if (!skipRender && rows.length) {
  console.log("[triage] render check (Markdown + KaTeX strict)");
  const r = spawnSync(process.execPath, ["scripts/verify-corpus-render.mjs", outRoot], { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 });
  const auditPath = path.join(outRoot, "render-audit.json");
  if (existsSync(auditPath)) {
    const audit = JSON.parse(readFileSync(auditPath, "utf8"));
    for (const p of audit.papers) {
      const row = rows.find((x) => x.slug === p.paper || slugFor(x.name) === p.paper);
      if (!row) continue;
      row.stats.renderFailing = p.failing;
      row.findings = row.findings.filter((x) => x.code !== "render-error" && x.code !== "katex-no-metrics");
      for (const bad of p.rows) {
        for (const err of bad.errors) {
          // U+FFFD is already reported as unidentified-glyph; do not double count.
          if (/U\+FFFD|private-use/i.test(err) || /No character metrics for '�'/.test(err)) continue;
          // "No character metrics" means KaTeX falls back to another font for a
          // text-mode symbol. It displays, so it is a warning, not a failure.
          const soft = /No character metrics/.test(err);
          row.findings.push({ severity: soft ? "warn" : "fail", code: soft ? "katex-no-metrics" : "render-error", detail: err, question: bad.question });
        }
      }
      row.status = classify(row.findings, row.runFailures);
    }
  } else {
    console.error("[triage] render audit produced no report:\n" + (r.stderr || r.stdout || "").slice(-600));
  }
}

// ---- reports -------------------------------------------------------------
const tally = (key) => rows.reduce((a, r) => ((a[r[key] ?? "n/a"] = (a[r[key] ?? "n/a"] ?? 0) + 1), a), {});
const byStatus = tally("status");
const codes = new Map();
for (const r of rows) {
  for (const f of r.findings) {
    const k = `${f.severity}:${f.code}`;
    if (!codes.has(k)) codes.set(k, { papers: new Set(), cards: 0, example: f.detail });
    const c = codes.get(k);
    c.papers.add(r.name);
    c.cards += 1;
  }
}

const tsv = ["status\tgroup\tsubject\tpaper\tcards\tsumMarks\tprinted\tmarksTotal\ttextCov\tsec\tfailCodes"];
for (const r of rows.sort((a, b) => a.status.localeCompare(b.status) || a.file.localeCompare(b.file))) {
  const fails = [...new Set(r.findings.filter((f) => f.severity === "fail").map((f) => f.code))].join(",");
  tsv.push([r.status, r.group, r.subject, r.file, r.stats.cards ?? 0, r.stats.marksSum ?? "", (r.stats.printedTotals ?? []).join("/"), r.stats.marksTotal ?? "", r.stats.textCoverage ?? "", r.seconds, fails].join("\t"));
}
writeFileSync(path.join(outRoot, "results.tsv"), tsv.join("\n") + "\n");
writeFileSync(path.join(outRoot, "results.json"), JSON.stringify(rows, null, 2));

const pct = (k) => `${byStatus[k] ?? 0}`;
const md = [];
md.push(`# Triage run ${stamp}`, "", `${rows.length} papers from \`${dir}\`. All imports offline: zero network attempts, zero tokens.`, "");
md.push(`| PASS | REVIEW | FLAGGED | FAIL |`, `|---|---|---|---|`, `| ${pct("PASS")} | ${pct("REVIEW")} | ${pct("FLAGGED")} | ${pct("FAIL")} |`, "");
md.push("PASS = every invariant holds. REVIEW = only heuristic warnings. FLAGGED = the pipeline itself flagged cards for review. FAIL = a hard invariant broke.", "");
md.push("## By group", "", "| group | papers | PASS | REVIEW | FLAGGED | FAIL |", "|---|---|---|---|---|---|");
for (const g of [...new Set(rows.map((r) => r.group))].sort()) {
  const rs = rows.filter((r) => r.group === g);
  const c = (s) => rs.filter((r) => r.status === s).length;
  md.push(`| ${g} | ${rs.length} | ${c("PASS")} | ${c("REVIEW")} | ${c("FLAGGED")} | ${c("FAIL")} |`);
}
md.push("", "## Findings by code", "", "| severity:code | papers | cards | example |", "|---|---|---|---|");
for (const [k, v] of [...codes.entries()].sort((a, b) => (a[0].startsWith("fail") ? 0 : 1) - (b[0].startsWith("fail") ? 0 : 1) || b[1].papers.size - a[1].papers.size)) {
  md.push(`| ${k} | ${v.papers.size} | ${v.cards} | ${String(v.example).replace(/\|/g, "/").slice(0, 110)} |`);
}
md.push("", "## Papers that did not PASS", "");
for (const r of rows.filter((x) => x.status !== "PASS")) {
  md.push(`- **${r.status}** \`${r.file}\` (${r.group}, ${r.subject}, ${r.stats.cards ?? 0} cards)`);
  const seen = new Set();
  for (const f of r.findings.filter((x) => x.severity !== "warn" || r.status === "REVIEW")) {
    const k = `${f.code}:${f.question ?? ""}`;
    if (seen.has(k)) continue;
    seen.add(k);
    if (seen.size > 6) { md.push("  - ..."); break; }
    md.push(`  - ${f.severity} ${f.code}${f.question != null ? ` Q${f.question}` : ""}: ${String(f.detail).slice(0, 140)}`);
  }
  for (const rf of r.runFailures.slice(0, 3)) md.push(`  - run: ${rf}`);
}
writeFileSync(path.join(outRoot, "summary.md"), md.join("\n") + "\n");
console.log(`\n[triage] ${JSON.stringify(byStatus)}\n[triage] report: ${path.join(outRoot, "summary.md")}`);
if ((byStatus.FAIL ?? 0) > 0) process.exitCode = 1;
