#!/usr/bin/env node
// One-off generator for the human-reviewable source-expected-IDs fixture.
//
//   node scripts/generate-source-ids.mjs [--dumps output/.../c-calibration]
//
// Reads the raw `--inspect-source` page dumps, derives each paper's parent
// question IDs with an independent scan (scripts/lib/source-ids.mjs), and
// writes scripts/fixtures/source-expected-ids.json tied to each PDF's SHA256.
// The production parser never reads this file.

import { existsSync, mkdirSync, readdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";

import { PAPERS, sha256File } from "./lib/zero-cost-accounting.mjs";
import { deriveExpectedIds } from "./lib/source-ids.mjs";

const STYLE_BY_PAPER = {
  physics21: "aqa",
  physics24: "aqa",
  cs222: "aqa",
  cs223: "aqa",
  cs224: "aqa",
  tp03_aqa_physics: "aqa",
  naiker_m16: "heading",
  madas_p2: "heading",
};
const DEFAULT_STYLE = "numbered";

function range(lo, hi) {
  return Array.from({ length: hi - lo + 1 }, (_, i) => lo + i);
}

/// Papers whose text-dump heading scan is lossy (question headings split across
/// runs by layout artefacts) but whose ID set was confirmed by direct source
/// page review. Expected IDs are frozen here so the accounting comparison is
/// still independent of the production parser.
const REVIEWED_OVERRIDES = {
  aqafm24: {
    expectedIds: range(1, 23),
    confidence: "MEDIUM",
    note: "AQA 8365/1 Jun24: Q1..Q23; Q8/Q20 headings are split by the 'box' instruction run (page 18 shows 'box 20 ...').",
  },
  tp04_cie_maths: {
    // Render-verified (pdftoppm): page 19 ends at Q10(d) and page 20 is the
    // "Additional Page"; the only "11" tokens are the printed page number and
    // math. Source has exactly 10 questions (the paper code is 9709/11).
    expectedIds: range(1, 10),
    confidence: "MEDIUM",
    note: "CIE 9709/11 M/J 22 Paper 1: Q1..Q10 (render-verified last page is Additional Page; '11' is the paper code, not a question).",
  },
  tp07_legacy_maths: {
    expectedIds: range(1, 12),
    confidence: "MEDIUM",
    note: "CIE 9709/11 M/J 14: headings 1-12 visible; Q6/Q10 headings split across runs.",
  },
};

function latestInspectDir(dumpsRoot, name) {
  if (!existsSync(dumpsRoot)) return null;
  const hits = readdirSync(dumpsRoot, { withFileTypes: true })
    .filter((d) => d.isDirectory() && d.name.startsWith(`${name}-inspect-`))
    .map((d) => d.name)
    .sort();
  return hits.length ? path.join(dumpsRoot, hits[hits.length - 1]) : null;
}

function main() {
  const argv = process.argv.slice(2);
  const dumpArgIdx = argv.indexOf("--dumps");
  const dumpsRoot =
    dumpArgIdx >= 0 ? argv[dumpArgIdx + 1] : "output/zero-cost-orchestration/c-calibration";
  const outPath = "scripts/fixtures/source-expected-ids.json";
  mkdirSync(path.dirname(outPath), { recursive: true });

  const fixture = {
    _comment:
      "Human-reviewable independent source question IDs. Derived from raw --inspect-source page dumps, NOT from the production parser. Tied to each PDF SHA256; a hash mismatch invalidates the expectation.",
    generatedBy: "scripts/generate-source-ids.mjs",
    papers: {},
  };

  const report = [];
  for (const paper of PAPERS) {
    if (!existsSync(paper.file)) {
      report.push({ name: paper.name, status: "MISSING_FILE" });
      continue;
    }
    const dumpDir = latestInspectDir(dumpsRoot, paper.name);
    if (!dumpDir) {
      report.push({ name: paper.name, status: "NO_DUMP" });
      continue;
    }
    const style = STYLE_BY_PAPER[paper.name] ?? DEFAULT_STYLE;
    const derived = deriveExpectedIds(dumpDir, style);
    const override = REVIEWED_OVERRIDES[paper.name];
    const confidence = override ? override.confidence : derived.confidence;
    const expectedIds = override ? override.expectedIds : derived.ids;
    const hash = sha256File(paper.file);
    fixture.papers[paper.name] = {
      file: paper.file,
      fileHash: hash,
      cohort: paper.cohort,
      style,
      confidence,
      expectedIds,
      candidateIds: derived.candidates,
      sourcePages: derived.pages,
      declaredCount: derived.declaredCount,
      declaredPage: derived.declaredPage,
      ...(override ? { reviewNote: override.note } : {}),
    };
    report.push({
      name: paper.name,
      status: "OK",
      style,
      confidence,
      expected: expectedIds ? expectedIds.length : null,
      candidates: derived.candidates.length,
      declared: derived.declaredCount,
    });
  }

  writeFileSync(outPath, `${JSON.stringify(fixture, null, 2)}\n`, "utf8");
  for (const r of report) console.log(JSON.stringify(r));
  console.log(`wrote ${outPath}`);
}

main();
