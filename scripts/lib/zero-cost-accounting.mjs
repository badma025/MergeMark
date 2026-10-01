// Trustworthy zero-cost calibration accounting for the named 22-paper corpus.
//
// This module is the single source of truth for the C1 accounting dependency:
// it launches the REAL offline ingestion per paper into a unique per-run
// directory, captures the child exit code, requires freshly written artifacts
// from that exact run, and validates the offline `$0` / zero-attempt policy.
//
// The spawn function is injectable so rejection behaviour can be tested with
// controlled fake children (see scripts/tests/zero-cost-accounting.test.mjs).
//
// No network, no credentials, no production data. Offline ingestion never
// leaves the machine; a refused request is counted, never billed.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  statSync,
} from "node:fs";
import path from "node:path";

// Reuse the canonical freshness/run-failure/policy validators from the offline
// certification command so the calibration runner cannot silently drift from
// the e2e certification contract.
import {
  ARTIFACT_FRESHNESS_SLACK_MS,
  artifactIsFresh,
  certificationFailures as baseCertificationFailures,
  runFailureMessage,
} from "../verify-zero-cost-offline.mjs";

export { ARTIFACT_FRESHNESS_SLACK_MS, artifactIsFresh, runFailureMessage };

/// The named corpus under calibration. `name` doubles as the artifact slug
/// (`paper_name.replace(' ', '_').replace("'", "")`), and `file` is the exact
/// path passed to the ingestion harness.
export const PAPERS = [
  { name: "physics21", file: "past papers for mergemark/physics '21.pdf", subject: "Physics", module: "A-Level Physics", cohort: "historical" },
  { name: "physics24", file: "past papers for mergemark/physics '24.pdf", subject: "Physics", module: "A-Level Physics", cohort: "historical" },
  { name: "corepure21", file: "past papers for mergemark/core pure 1 '21.pdf", subject: "Mathematics", module: "Core Pure Mathematics", cohort: "historical" },
  { name: "corepure22", file: "past papers for mergemark/core pure 1 '22.pdf", subject: "Mathematics", module: "Core Pure Mathematics", cohort: "historical" },
  { name: "corepure23", file: "past papers for mergemark/core pure 1 '23.pdf", subject: "Mathematics", module: "Core Pure Mathematics", cohort: "historical" },
  { name: "fp224", file: "past papers for mergemark/fp2 '24.pdf", subject: "Mathematics", module: "Further Pure Mathematics 2", cohort: "historical" },
  { name: "fm121", file: "past papers for mergemark/further mechanics 1 '21.pdf", subject: "Mathematics", module: "Further Mechanics 1", cohort: "historical" },
  { name: "fm122", file: "past papers for mergemark/further mechanics 1 '22.pdf", subject: "Mathematics", module: "Further Mechanics 1", cohort: "historical" },
  { name: "fm123", file: "past papers for mergemark/further mechanics 1 '23.pdf", subject: "Mathematics", module: "Further Mechanics 1", cohort: "historical" },
  { name: "aqafm24", file: "past papers for mergemark/aqa gcse further maths '24.pdf", subject: "Mathematics", module: "GCSE Further Mathematics", cohort: "historical" },
  { name: "aea2024", file: "past papers for mergemark/aea2024.pdf", subject: "Mathematics", module: "AEA Mathematics", cohort: "historical" },
  { name: "cs222", file: "past papers for mergemark/computer science 2 '22.pdf", subject: "Computer Science", module: "Computer Science", cohort: "historical" },
  { name: "cs223", file: "past papers for mergemark/computer science 2 '23.pdf", subject: "Computer Science", module: "Computer Science", cohort: "historical" },
  { name: "cs224", file: "past papers for mergemark/computer science 2 '24.pdf", subject: "Computer Science", module: "Computer Science", cohort: "historical" },
  { name: "jan2025", file: "past papers for mergemark/January 2025 QP.pdf", subject: "Mathematics", module: "Decision Mathematics D1", cohort: "historical" },
  { name: "nov2024", file: "past papers for mergemark/Nov 2024 QP.pdf", subject: "Mathematics", module: "GCSE Mathematics", cohort: "historical" },
  { name: "naiker_m16", file: "past papers for mergemark/naikermaths_paper_m16_pure.pdf", subject: "Mathematics", module: "Pure Mathematics", cohort: "historical" },
  { name: "madas_p2", file: "past papers for mergemark/madas_paper_2_t.pdf", subject: "Mathematics", module: "Pure Mathematics", cohort: "historical" },
  { name: "tp01_edexcel_maths", file: "test_papers/01_edexcel_maths_p1_qp.pdf", subject: "Mathematics", module: "Pure Mathematics", cohort: "test_papers" },
  { name: "tp03_aqa_physics", file: "test_papers/03_aqa_physics_p1_qp.pdf", subject: "Physics", module: "A-Level Physics", cohort: "test_papers" },
  { name: "tp04_cie_maths", file: "test_papers/04_cie_maths_9709_p1_qp.pdf", subject: "Mathematics", module: "Pure Mathematics", cohort: "test_papers" },
  { name: "tp07_legacy_maths", file: "test_papers/07_legacy_c3_or_c4_qp.pdf", subject: "Mathematics", module: "Pure Mathematics", cohort: "test_papers" },
];

export function slugFor(paperName) {
  return paperName.replaceAll(" ", "_").replaceAll("'", "");
}

export function sha256File(filePath) {
  return createHash("sha256").update(readFileSync(filePath)).digest("hex").toUpperCase();
}

/// Counters that were not actually reported must be `null`, never silently 0.
export function knownNumber(value) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export function certificationArtifactPath(outDir, slug) {
  return path.join(outDir, `${slug}_offline_certification.json`);
}

export function cardsArtifactPaths(outDir, slug) {
  return {
    markdown: path.join(outDir, `${slug}_cards.md`),
    json: path.join(outDir, `${slug}_cards.json`),
  };
}

/// Empty array means the offline certification passed. Builds on the canonical
/// `verify-zero-cost-offline` gates (cloud attempts, zero-attempt policy, tokens,
/// cost) and adds the image/text attempt counters plus `tokensZero`. Unknown
/// counters fail (never treated as 0).
export function certificationFailures(cert) {
  const failures = baseCertificationFailures(cert);
  const gate = (name, value, expected) => {
    if (value !== expected) {
      failures.push(`${name}=${JSON.stringify(value)} (expected ${JSON.stringify(expected)})`);
    }
  };
  gate("imageAttempts", cert.imageAttempts, 0);
  gate("textOnlyAttempts", cert.textOnlyAttempts, 0);
  gate("tokensZero", cert.tokensZero, true);
  return failures;
}

function readJson(filePath) {
  return JSON.parse(readFileSync(filePath, "utf8"));
}

function artifactStat(filePath, startedAtMs, slackMs) {
  if (!existsSync(filePath)) {
    return { path: filePath, exists: false, fresh: false, mtimeMs: null };
  }
  const stat = statSync(filePath);
  return {
    path: filePath,
    exists: true,
    mtimeMs: stat.mtimeMs,
    fresh: artifactIsFresh(stat.mtimeMs, startedAtMs, slackMs),
  };
}

/// Spawn the real offline ingestion for one paper into a unique run directory
/// and evaluate the result. Any problem is reported as a `failures` entry; the
/// row's `status` becomes `"FAIL"` (or `"MISSING_FILE"` / `"UNKNOWN"`).
export function runPaper({
  paper,
  spawn,
  outRoot,
  runId,
  manifestPath = "src-tauri/Cargo.toml",
  cargo = "cargo",
  freshSlackMs = ARTIFACT_FRESHNESS_SLACK_MS,
  fixture = null,
  now = () => Date.now(),
}) {
  const slug = slugFor(paper.name);
  const runDir = path.join(outRoot, `${slug}-${runId}`);
  const failures = [];
  const row = {
    name: paper.name,
    cohort: paper.cohort,
    file: paper.file,
    subject: paper.subject,
    module: paper.module,
    runId,
    runDir,
    status: "OK",
    exitCode: null,
    signal: null,
    fileHash: null,
    expectedIds: null,
    extractedIds: null,
    idCoverage: "UNKNOWN",
    idDiscrepancies: null,
    counters: null,
    failures,
  };

  if (!existsSync(paper.file)) {
    row.status = "MISSING_FILE";
    failures.push(`source file missing: ${paper.file}`);
    return row;
  }
  row.fileHash = sha256File(paper.file);

  // Fixture is tied to the file hash; a hash mismatch invalidates the source
  // ID expectation rather than silently reusing a stale manifest.
  if (fixture && fixture.papers && fixture.papers[paper.name]) {
    const fx = fixture.papers[paper.name];
    if (fx.fileHash && fx.fileHash !== row.fileHash) {
      failures.push(`source-expected-ids hash mismatch (fixture ${fx.fileHash} != ${row.fileHash})`);
      row.idCoverage = "UNKNOWN";
    } else {
      row.expectedIds = fx.expectedIds ?? null;
      row.idCoverage = Array.isArray(fx.expectedIds) ? "RESOLVED" : "UNKNOWN";
    }
  }

  if (runId === null || runId === undefined) {
    // Discovery mode: identify the source without running ingestion.
    return row;
  }

  mkdirSync(runDir, { recursive: true });
  const paperOutDir = path.join(runDir, paper.name);
  mkdirSync(paperOutDir, { recursive: true });
  const certPath = certificationArtifactPath(paperOutDir, slug);
  const cards = cardsArtifactPaths(paperOutDir, slug);
  // Remove any earlier artifact FIRST so a stale file from a previous run can
  // never be mistaken for this run's result.
  rmSync(certPath, { force: true });
  rmSync(cards.markdown, { force: true });
  rmSync(cards.json, { force: true });

  const startedAt = now();
  const args = [
    "run", "--quiet",
    "--manifest-path", manifestPath,
    "--bin", "e2e_import",
    "--", paper.file, paper.name,
    "--offline",
    "--out", paperOutDir,
    "--subject", paper.subject,
    "--module", paper.module,
  ];
  const run = spawn(cargo, args, { stdio: "inherit" }) ?? {};
  row.exitCode = run.status ?? null;
  row.signal = run.signal ?? null;
  const endedAt = now();

  const runFailure = runFailureMessage(run.status ?? null, run.signal ?? null, run.error);
  if (runFailure) failures.push(runFailure);

  const certStat = artifactStat(certPath, startedAt, freshSlackMs);
  const mdStat = artifactStat(cards.markdown, startedAt, freshSlackMs);
  const jsonStat = artifactStat(cards.json, startedAt, freshSlackMs);
  row.artifacts = { certification: certStat, cardsMarkdown: mdStat, cardsJson: jsonStat };

  if (!certStat.exists) {
    failures.push(`no certification artifact written by this run: ${certPath}`);
  } else if (!certStat.fresh) {
    failures.push(`certification artifact is stale (mtime ${certStat.mtimeMs}, run started ${startedAt})`);
  }
  if (!mdStat.exists) failures.push(`no cards markdown written by this run: ${cards.markdown}`);
  else if (!mdStat.fresh) failures.push(`cards markdown is stale: ${cards.markdown}`);
  if (!jsonStat.exists) failures.push(`no cards json written by this run: ${cards.json}`);
  else if (!jsonStat.fresh) failures.push(`cards json is stale: ${cards.json}`);

  if (certStat.exists) {
    let cert;
    try {
      cert = readJson(certStat.path);
    } catch (error) {
      failures.push(`certification artifact unreadable: ${error.message}`);
      cert = null;
    }
    if (cert) {
      if (cert.mode !== "offline") failures.push(`mode=${JSON.stringify(cert.mode)} (expected "offline")`);
      failures.push(...certificationFailures(cert));
      row.counters = {
        strictTier0: knownNumber(cert.strictTier0),
        localRecoveries: knownNumber(cert.localRecoveries),
        quarantined: knownNumber(cert.quarantined),
        usableCards: knownNumber(cert.usableCards),
        placeholderFallbackCards: knownNumber(cert.placeholderFallbackCards),
        cloudAttempts: knownNumber(cert.cloudAttempts),
        imageAttempts: knownNumber(cert.imageAttempts),
        textOnlyAttempts: knownNumber(cert.textOnlyAttempts),
        promptTokens: knownNumber(cert.promptTokens),
        completionTokens: knownNumber(cert.completionTokens),
        costUsd: typeof cert.costUsd === "string" ? cert.costUsd : null,
        questionsExpected: knownNumber(cert.questionsExpected),
        questionsExtracted: knownNumber(cert.questionsExtracted),
      };
    }
  }

  // Compare extracted IDs against the independent source fixture when present.
  if (Array.isArray(row.expectedIds) && jsonStat.exists) {
    try {
      const extracted = readJson(jsonStat.path);
      const ids = [...new Set(extracted.map((c) => String(c.question_number)))].sort(
        (a, b) => Number(a) - Number(b),
      );
      row.extractedIds = ids;
      const expected = row.expectedIds.map(String);
      const missing = expected.filter((id) => !ids.includes(id));
      const extra = ids.filter((id) => !expected.includes(id));
      row.idDiscrepancies = { missing, extra };
      row.idCoverage = missing.length === 0 && extra.length === 0 ? "MATCH" : "MISMATCH";
    } catch (error) {
      failures.push(`cards json unreadable for ID comparison: ${error.message}`);
    }
  }

  row.endedAt = endedAt;
  if (failures.length > 0) row.status = "FAIL";
  return row;
}

export function runAll({
  papers = PAPERS,
  spawn,
  outRoot,
  runId,
  manifestPath = "src-tauri/Cargo.toml",
  cargo = "cargo",
  fixture = null,
  freshSlackMs = ARTIFACT_FRESHNESS_SLACK_MS,
  now = () => Date.now(),
  log = console.log,
}) {
  mkdirSync(outRoot, { recursive: true });
  const rows = [];
  for (const paper of papers) {
    log(`[accounting] ${paper.cohort}/${paper.name} <- ${paper.file}`);
    rows.push(
      runPaper({ paper, spawn, outRoot, runId, manifestPath, cargo, freshSlackMs, fixture, now }),
    );
  }
  return rows;
}

export function summarize(rows) {
  const buckets = {};
  for (const row of rows) {
    const key = row.cohort ?? "unknown";
    buckets[key] ??= { total: 0, failed: 0, strictTier0: 0, expectedIds: 0 };
    buckets[key].total += 1;
    if (row.status !== "OK") buckets[key].failed += 1;
    buckets[key].strictTier0 += row.counters?.strictTier0 ?? 0;
    if (Array.isArray(row.expectedIds)) buckets[key].expectedIds += row.expectedIds.length;
  }
  return buckets;
}

export function failingRows(rows) {
  return rows.filter((row) => row.status !== "OK");
}

export { spawnSync };
