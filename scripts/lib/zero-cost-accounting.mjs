// Trustworthy zero-cost calibration accounting for the named corpus: the 22
// original papers (historical, test_papers) plus triage_fixtures, papers moved
// from papers/unused after being fixed against.
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
  { name: "physics21", file: "papers/used/physics '21.pdf", subject: "Physics", module: "A-Level Physics", cohort: "historical" },
  { name: "physics24", file: "papers/used/physics '24.pdf", subject: "Physics", module: "A-Level Physics", cohort: "historical" },
  { name: "corepure21", file: "papers/used/core pure 1 '21.pdf", subject: "Mathematics", module: "Core Pure Mathematics", cohort: "historical" },
  { name: "corepure22", file: "papers/used/core pure 1 '22.pdf", subject: "Mathematics", module: "Core Pure Mathematics", cohort: "historical" },
  { name: "corepure23", file: "papers/used/core pure 1 '23.pdf", subject: "Mathematics", module: "Core Pure Mathematics", cohort: "historical" },
  { name: "fp224", file: "papers/used/fp2 '24.pdf", subject: "Mathematics", module: "Further Pure Mathematics 2", cohort: "historical" },
  { name: "fm121", file: "papers/used/further mechanics 1 '21.pdf", subject: "Mathematics", module: "Further Mechanics 1", cohort: "historical" },
  { name: "fm122", file: "papers/used/further mechanics 1 '22.pdf", subject: "Mathematics", module: "Further Mechanics 1", cohort: "historical" },
  { name: "fm123", file: "papers/used/further mechanics 1 '23.pdf", subject: "Mathematics", module: "Further Mechanics 1", cohort: "historical" },
  { name: "aqafm24", file: "papers/used/aqa gcse further maths '24.pdf", subject: "Mathematics", module: "GCSE Further Mathematics", cohort: "historical" },
  { name: "aea2024", file: "papers/used/aea2024.pdf", subject: "Mathematics", module: "AEA Mathematics", cohort: "historical" },
  { name: "cs222", file: "papers/used/computer science 2 '22.pdf", subject: "Computer Science", module: "Computer Science", cohort: "historical" },
  { name: "cs223", file: "papers/used/computer science 2 '23.pdf", subject: "Computer Science", module: "Computer Science", cohort: "historical" },
  { name: "cs224", file: "papers/used/computer science 2 '24.pdf", subject: "Computer Science", module: "Computer Science", cohort: "historical" },
  { name: "jan2025", file: "papers/used/January 2025 QP.pdf", subject: "Mathematics", module: "Decision Mathematics D1", cohort: "historical" },
  { name: "nov2024", file: "papers/used/Nov 2024 QP.pdf", subject: "Mathematics", module: "GCSE Mathematics", cohort: "historical" },
  { name: "naiker_m16", file: "papers/used/naikermaths_paper_m16_pure.pdf", subject: "Mathematics", module: "Pure Mathematics", cohort: "historical" },
  { name: "madas_p2", file: "papers/used/madas_paper_2_t.pdf", subject: "Mathematics", module: "Pure Mathematics", cohort: "historical" },
  { name: "tp01_edexcel_maths", file: "papers/used/01_edexcel_maths_p1_qp.pdf", subject: "Mathematics", module: "Pure Mathematics", cohort: "test_papers" },
  { name: "tp03_aqa_physics", file: "papers/used/03_aqa_physics_p1_qp.pdf", subject: "Physics", module: "A-Level Physics", cohort: "test_papers" },
  { name: "tp04_cie_maths", file: "papers/used/04_cie_maths_9709_p1_qp.pdf", subject: "Mathematics", module: "Pure Mathematics", cohort: "test_papers" },
  { name: "tp07_legacy_maths", file: "papers/used/07_legacy_c3_or_c4_qp.pdf", subject: "Mathematics", module: "Pure Mathematics", cohort: "test_papers" },
  // Papers moved out of papers/unused after being fixed against (triage
  // clusters); regression fixtures for the conventions they carry.
  { name: "ocr_gcse_jun24", file: "papers/used/Maths_GCSE_Past-Papers_OCR__June_2024_QP.pdf", subject: "Mathematics", module: "GCSE Mathematics", cohort: "triage_fixtures" },
  { name: "ocra_phys_jun17", file: "papers/used/Physics_A-level_Past-Papers_OCR-A__June_2017_QP.pdf", subject: "Physics", module: "A-Level Physics", cohort: "triage_fixtures" },
  { name: "cie_igcse_nov22", file: "papers/used/Maths_GCSE_Past-Papers_CIE__November_2022_(v1)_QP.pdf", subject: "Mathematics", module: "GCSE Mathematics", cohort: "triage_fixtures" },
  { name: "wjec_as_jun25", file: "papers/used/Maths_A-level_Papers_WJEC__June_2025_QP_-_Unit_1_WJEC_Maths_AS-level.pdf", subject: "Mathematics", module: "Pure Mathematics", cohort: "triage_fixtures" },
  { name: "ocrmei_jun25", file: "papers/used/Maths_A-level_Papers_OCR-MEI__June_2025_QP.pdf", subject: "Mathematics", module: "Pure Mathematics", cohort: "triage_fixtures" },
  { name: "aqa_alevel_jun19", file: "papers/used/Maths_A-level_Papers_AQA__June_2019_QP.pdf", subject: "Mathematics", module: "Pure Mathematics", cohort: "triage_fixtures" },
  { name: "edx_chem_jun22", file: "papers/used/Chemistry_A-level_Past-Papers_Edexcel__June_2022_QP_-_Paper_1_Edexcel_Chemistry_A-level.pdf", subject: "Chemistry", module: "A-Level Chemistry", cohort: "triage_fixtures" },
  { name: "edexcel18", file: "papers/used/edexcel '18 edexcel.pdf", subject: "Physics", module: "A-Level Physics", cohort: "triage_fixtures" },
  { name: "aqa_bio_gcse_jun22", file: "papers/used/Biology_GCSE_Past-Papers_AQA__June_2022_QP.pdf", subject: "Biology", module: "GCSE Biology", cohort: "triage_fixtures" },
  { name: "edx_fp2_jun05", file: "papers/used/Maths_A-level_FP2_Papers-Edexcel__June_2005_QP.pdf", subject: "Mathematics", module: "Further Pure Mathematics 2", cohort: "triage_fixtures" },
  { name: "aqa_gcse_nov17", file: "papers/used/Maths_GCSE_Past-Papers_AQA__November_2017_QP.pdf", subject: "Mathematics", module: "GCSE Mathematics", cohort: "triage_fixtures" },
  { name: "cie_m2_jun15", file: "papers/used/Maths_A-level_Papers_CIE__June_2015_(v1)_QP_-_M2.pdf", subject: "Mathematics", module: "Mechanics 2", cohort: "triage_fixtures" },
  { name: "ocr_econ_jun24", file: "papers/used/Economics_A-level_Past-Papers_OCR__June_2024_QP_-_Paper_1_OCR_Economics_A-level.pdf", subject: "Economics", module: "A-Level Economics", cohort: "triage_fixtures" },
  { name: "edx_m3_jun04_scan", file: "papers/used/Maths_A-level_M3_Papers-Edexcel__June_2004_QP.pdf", subject: "Mathematics", module: "Mechanics 3", cohort: "triage_fixtures" },
  { name: "edexcel19_outlined", file: "papers/used/edexcel '19 edexcel.pdf", subject: "Physics", module: "A-Level Physics", cohort: "triage_fixtures" },
  { name: "stats_tp10_scan", file: "papers/used/old ahh stats paper.pdf", subject: "Mathematics", module: "Statistics 1", cohort: "triage_fixtures" },
  { name: "edfm_jun25", file: "papers/used/Maths_A-level_Papers_Edexcel-Further__June_2025_QP.pdf", subject: "Mathematics", module: "Further Mathematics", cohort: "triage_fixtures" },
  { name: "physics18", file: "papers/used/physics '18.pdf", subject: "Physics", module: "A-Level Physics", cohort: "triage_fixtures" },
  { name: "edxa_bio_jun21", file: "papers/used/Biology_A-level_Past-Papers_Edexcel-A__June_2021_QP_-_Paper_1_Edexcel_(A)_Biology_AS-level.pdf", subject: "Biology", module: "AS Biology", cohort: "triage_fixtures" },
  { name: "cie_gcse_nov16", file: "papers/used/Maths_GCSE_Past-Papers_CIE__November_2016_(v1)_QP.pdf", subject: "Mathematics", module: "GCSE Mathematics", cohort: "triage_fixtures" },
  { name: "aqafm_jun25", file: "papers/used/Maths_A-level_Papers_AQA-Further__June_2025_QP.pdf", subject: "Mathematics", module: "Further Mathematics", cohort: "triage_fixtures" },
  { name: "edx_phys_jun22_p1", file: "papers/used/Physics_A-level_Past-Papers_Edexcel__June_2022_QP_-_Paper_1_Edexcel_Physics_AS-level.pdf", subject: "Physics", module: "AS Physics", cohort: "triage_fixtures" },
  { name: "ocra_chem_jun24", file: "papers/used/Chemistry_A-level_Past-Papers_OCR-A__June_2024_QP.pdf", subject: "Chemistry", module: "A-Level Chemistry", cohort: "triage_fixtures" },
  { name: "ocra_phys_gcse_nov20", file: "papers/used/Physics_GCSE_Past-Papers_OCR-A__November_2020_QP_-_Paper_1_OCR_(A)_Physics_GCSE.pdf", subject: "Physics", module: "GCSE Physics", cohort: "triage_fixtures" },
  { name: "edx_gcse_jun22", file: "papers/used/Maths_GCSE_Past-Papers_Edexcel__June_2022_QP.pdf", subject: "Mathematics", module: "GCSE Mathematics", cohort: "triage_fixtures" },
  { name: "edxa_econ_jun18", file: "papers/used/Economics_A-level_Past-Papers_Edexcel-A__June_2018_QP_-_Paper_2_Edexcel_(A)_Economics_AS-level.pdf", subject: "Economics", module: "AS Economics", cohort: "triage_fixtures" },
  { name: "aqa_chem_jun17", file: "papers/used/Chemistry_A-level_Past-Papers_AQA__June_2017_QP.pdf", subject: "Chemistry", module: "A-Level Chemistry", cohort: "triage_fixtures" },
  { name: "aqa_chem_gcse_jun22", file: "papers/used/Chemistry_GCSE_Past-Papers_AQA__June_2022_QP.pdf", subject: "Chemistry", module: "GCSE Chemistry", cohort: "triage_fixtures" },
  { name: "wjecf_jun24", file: "papers/used/Maths_A-level_Papers_WJEC-Further__June_2024_QP_-_Unit_2_WJEC_Further_Maths_AS-level.pdf", subject: "Mathematics", module: "Further Mathematics", cohort: "triage_fixtures" },
  { name: "edx_s1_nov03", file: "papers/used/Maths_A-level_S1_Papers-Edexcel__November_2003_QP.pdf", subject: "Mathematics", module: "Statistics 1", cohort: "triage_fixtures" },
  { name: "edx_c4_jan11", file: "papers/used/Maths_A-level_C4_Papers-Edexcel__January_2011_QP.pdf", subject: "Mathematics", module: "Core Mathematics 4", cohort: "triage_fixtures" },
  { name: "edx_phys_jun17_p2", file: "papers/used/Physics_A-level_Past-Papers_Edexcel__June_2017_QP_-_Paper_2_Edexcel_Physics_AS-level.pdf", subject: "Physics", module: "AS Physics", cohort: "triage_fixtures" },
  { name: "edx_gcse_old_jun16_p1f", file: "papers/used/Maths_GCSE_Past-Papers_Edexcel-A-Old__June_2016_QP_-_Paper_1_(F)_Edexcel_(A)_Maths_GCSE.pdf", subject: "Mathematics", module: "GCSE Mathematics", cohort: "triage_fixtures" },
  { name: "edexcel17", file: "papers/used/edexcel '17 edexcel.pdf", subject: "Physics", module: "A-Level Physics", cohort: "triage_fixtures" },
  { name: "ocra_phys_jun19", file: "papers/used/Physics_A-level_Past-Papers_OCR-A__June_2019_QP.pdf", subject: "Physics", module: "A-Level Physics", cohort: "triage_fixtures" },
  { name: "ocra_chem_jun22", file: "papers/used/Chemistry_A-level_Past-Papers_OCR-A__June_2022_QP.pdf", subject: "Chemistry", module: "A-Level Chemistry", cohort: "triage_fixtures" },
  { name: "aqa_econ_jun21", file: "papers/used/Economics_A-level_Past-Papers_AQA__June_2021_QP.pdf", subject: "Economics", module: "A-Level Economics", cohort: "triage_fixtures" },
  { name: "ocr_further_nov21", file: "papers/used/Maths_A-level_Papers_OCR-Further__Nov_2021_QP.pdf", subject: "Mathematics", module: "Further Mathematics", cohort: "triage_fixtures" },
  { name: "edx_c4_jan06", file: "papers/used/Maths_A-level_C4_Papers-Edexcel__January_2006_QP.pdf", subject: "Mathematics", module: "Core Mathematics 4", cohort: "triage_fixtures" },
  { name: "edx_fp2_jun14r", file: "papers/used/Maths_A-level_FP2_Papers-Edexcel__June_2014_(R)_QP.pdf", subject: "Mathematics", module: "Further Pure 2", cohort: "triage_fixtures" },
  { name: "edx_s1_jun12", file: "papers/used/Maths_A-level_S1_Papers-Edexcel__June_2012_QP.pdf", subject: "Mathematics", module: "Statistics 1", cohort: "triage_fixtures" },
  { name: "aqa_chem_gcse_jun24", file: "papers/used/Chemistry_GCSE_Past-Papers_AQA__June_2024_QP.pdf", subject: "Chemistry", module: "GCSE Chemistry", cohort: "triage_fixtures" },
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
