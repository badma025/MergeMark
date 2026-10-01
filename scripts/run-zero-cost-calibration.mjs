#!/usr/bin/env node
// Fresh, trustworthy zero-cost calibration over the named 22-paper corpus.
//
//   node scripts/run-zero-cost-calibration.mjs [--out <dir>] [--stamp <id>]
//                                              [--only a,b] [--manifest <path>]
//
// For every paper this launches the REAL offline ingestion
// (`cargo run --bin e2e_import -- <pdf> <name> --offline`) into a unique
// per-run/per-paper directory, captures the child exit code, requires freshly
// written artifacts from that exact run, and validates the offline zero-attempt
// policy against an independent source-ID fixture. Any failure makes the
// runner exit non-zero. No network, no credentials, no production data.

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";

import {
  PAPERS,
  failingRows,
  runAll,
  spawnSync,
  summarize,
} from "./lib/zero-cost-accounting.mjs";

function parseArgs(argv) {
  const opts = {
    out: "output/zero-cost-orchestration/c-calibration",
    stamp: null,
    only: null,
    manifest: "src-tauri/Cargo.toml",
  };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === "--out") opts.out = argv[++i];
    else if (a === "--stamp") opts.stamp = argv[++i];
    else if (a === "--only") opts.only = argv[++i].split(",").map((s) => s.trim()).filter(Boolean);
    else if (a === "--manifest") opts.manifest = argv[++i];
    else if (a === "--help" || a === "-h") {
      console.log(
        "usage: node scripts/run-zero-cost-calibration.mjs [--out <dir>] [--stamp <id>] [--only a,b] [--manifest <path>]",
      );
      process.exit(0);
    } else {
      console.error(`unknown argument: ${a}`);
      process.exit(2);
    }
  }
  return opts;
}

function loadFixture() {
  try {
    return JSON.parse(readFileSync("scripts/fixtures/source-expected-ids.json", "utf8"));
  } catch (error) {
    console.error(`[accounting] WARNING: source-expected-ids fixture unreadable: ${error.message}`);
    return null;
  }
}

function main() {
  const opts = parseArgs(process.argv.slice(2));
  const runId = opts.stamp ?? new Date().toISOString().replace(/[^0-9]/g, "").slice(0, 14);
  const runRoot = path.join(opts.out, `run-${runId}`);
  mkdirSync(runRoot, { recursive: true });

  const papers = opts.only ? PAPERS.filter((p) => opts.only.includes(p.name)) : PAPERS;
  if (papers.length === 0) {
    console.error(`[accounting] no papers matched --only ${opts.only.join(",")}`);
    process.exit(2);
  }
  const fixture = loadFixture();

  console.log(`[accounting] run=${runId} out=${runRoot} papers=${papers.length}`);
  const rows = runAll({
    papers,
    spawn: spawnSync,
    outRoot: runRoot,
    runId,
    manifestPath: opts.manifest,
    fixture,
  });

  const manifest = {
    runId,
    generatedAt: new Date().toISOString(),
    manifestPath: opts.manifest,
    fixture: fixture
      ? { path: "scripts/fixtures/source-expected-ids.json", generatedBy: fixture.generatedBy }
      : null,
    rows,
  };
  const manifestPath = path.join(runRoot, `manifest-${runId}.json`);
  writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");

  let report = `# zero-cost calibration run ${runId}\n\n`;
  report += "cohort\tname\tstatus\tstrict\trecovered\tquarantined\tusable\tcloud\timage\ttokens\tcost\tidCoverage\n";
  for (const row of rows) {
    const c = row.counters ?? {};
    report += [
      row.cohort,
      row.name,
      row.status,
      c.strictTier0 ?? "?",
      c.localRecoveries ?? "?",
      c.quarantined ?? "?",
      c.usableCards ?? "?",
      c.cloudAttempts ?? "?",
      c.imageAttempts ?? "?",
      (c.promptTokens ?? 0) + (c.completionTokens ?? 0),
      c.costUsd ?? "?",
      row.idCoverage,
    ].join("\t");
    report += "\n";
    if (row.failures.length > 0) report += `\tfailures: ${row.failures.join("; ")}\n`;
    if (row.idDiscrepancies) report += `\tidDiscrepancies: ${JSON.stringify(row.idDiscrepancies)}\n`;
  }
  const buckets = summarize(rows);
  report += `\nsummary ${JSON.stringify(buckets)}\n`;
  const bad = failingRows(rows);
  report += `failingRows=${bad.length}/${rows.length}\n`;
  const reportPath = path.join(runRoot, `summary-${runId}.txt`);
  writeFileSync(reportPath, report, "utf8");
  console.log(report);
  console.log(`[accounting] manifest: ${manifestPath}`);
  console.log(`[accounting] summary:  ${reportPath}`);

  if (bad.length > 0) {
    console.error(`[accounting] FAIL: ${bad.length}/${rows.length} rows failed`);
    process.exit(1);
  }
  const unmatched = rows.filter((r) => r.idCoverage !== "MATCH");
  if (unmatched.length > 0) {
    console.error(
      `[accounting] NOTE: ${unmatched.length} rows not ID-matched (${unmatched
        .map((r) => `${r.name}:${r.idCoverage}`)
        .join(", ")}); weighted rate is not fully verified`,
    );
  }
  console.log("[accounting] PASS: all rows fresh, offline, zero-attempt, zero-cost");
}

main();
