#!/usr/bin/env node
// Offline $0.0000 ingestion certification.
//
// Runs the production question-ingestion pipeline against one PDF with a
// refusing, counting LLM client: no credentials, no network, no spend. The
// command FAILS unless the run:
//   * completed without an error or signal,
//   * produced a certification artifact from THIS run (stale files are removed
//     before the run and rejected afterwards),
//   * attempted exactly ZERO model requests, and
//   * reported zero prompt/completion tokens and $0.0000 actual spend.
//
// Usage:
//   node scripts/verify-zero-cost-offline.mjs "past papers for mergemark/fp2 '24.pdf" "fp2 24" [outDir]
//
// Exit codes: 0 = certified, 1 = certification failure, 2 = harness/artifact problem.

import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, rmSync, statSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

/// A freshly written artifact can predate the recorded start by filesystem
/// timestamp granularity; anything older than this is a stale file.
export const ARTIFACT_FRESHNESS_SLACK_MS = 2000;

export function artifactIsFresh(mtimeMs, startedAtMs, slackMs = ARTIFACT_FRESHNESS_SLACK_MS) {
  return Number.isFinite(mtimeMs) && mtimeMs >= startedAtMs - slackMs;
}

/// `null` when the child run itself succeeded, otherwise a reason string.
export function runFailureMessage(status, signal, error) {
  if (error) return `harness failed to start: ${error.message ?? String(error)}`;
  if (signal) return `harness killed by signal ${signal}`;
  if (status !== 0) return `harness exited with code ${status}`;
  return null;
}

/// Empty array means the certification passed. Cost, tokens and the
/// zero-attempt policy are ALL gates: a refused request still costs $0, but an
/// attempted request means the digital circuit breaker did not hold.
export function certificationFailures(cert) {
  const failures = [];
  if (cert.cloudAttempts !== 0) {
    failures.push(`cloudAttempts=${cert.cloudAttempts} (expected 0)`);
  }
  if (cert.zeroAttemptPolicyPassed !== true) {
    failures.push(
      `zeroAttemptPolicyPassed=${cert.zeroAttemptPolicyPassed} (expected true)`,
    );
  }
  if (cert.promptTokens !== 0) {
    failures.push(`promptTokens=${cert.promptTokens} (expected 0)`);
  }
  if (cert.completionTokens !== 0) {
    failures.push(`completionTokens=${cert.completionTokens} (expected 0)`);
  }
  if (cert.costUsd !== "0.0000") {
    failures.push(`costUsd=${cert.costUsd} (expected 0.0000)`);
  }
  return failures;
}

function main() {
  const [pdf, paper, outArg] = process.argv.slice(2);
  if (!pdf || !paper) {
    console.error(
      "usage: node scripts/verify-zero-cost-offline.mjs <pdf-path> <paper-name> [out-dir]",
    );
    process.exit(2);
  }

  const manifest = process.env.MERGEMARK_MANIFEST ?? "src-tauri/Cargo.toml";
  const out = outArg ?? path.join("output", "zero-cost-orchestration", "offline-check");
  mkdirSync(out, { recursive: true });

  const slug = paper.replaceAll(" ", "_").replaceAll("'", "");
  const certPath = path.join(out, `${slug}_offline_certification.json`);
  // Remove any earlier artifact FIRST: a stale file from a previous run can
  // never be mistaken for this run's result.
  rmSync(certPath, { force: true });

  console.log(`[zero-cost] offline import: ${pdf} -> ${out}`);
  const startedAt = Date.now();
  const run = spawnSync(
    "cargo",
    [
      "run",
      "--quiet",
      "--manifest-path",
      manifest,
      "--bin",
      "e2e_import",
      "--",
      pdf,
      paper,
      "--offline",
      "--out",
      out,
    ],
    // No shell: arguments (which may contain spaces, apostrophes and unicode)
    // are passed through as a real argv, so nothing needs quoting or escaping.
    { stdio: "inherit" },
  );

  const runFailure = runFailureMessage(run.status, run.signal, run.error);
  if (runFailure) {
    console.error(`[zero-cost] FAIL: ${runFailure}`);
    process.exit(2);
  }

  let stat;
  try {
    stat = statSync(certPath);
  } catch (error) {
    console.error(
      `[zero-cost] FAIL: no certification artifact for this run (${certPath}): ${error.message}`,
    );
    process.exit(2);
  }
  if (!artifactIsFresh(stat.mtimeMs, startedAt)) {
    console.error(
      `[zero-cost] FAIL: certification artifact is stale (mtime ${new Date(stat.mtimeMs).toISOString()}, run started ${new Date(startedAt).toISOString()})`,
    );
    process.exit(2);
  }

  let cert;
  try {
    cert = JSON.parse(readFileSync(certPath, "utf8"));
  } catch (error) {
    console.error(`[zero-cost] FAIL: certification artifact unreadable: ${error.message}`);
    process.exit(2);
  }

  // Offline spend is real spend: a refused request never leaves the machine.
  console.log(
    `[zero-cost] actual offline spend $${cert.costUsd} with ${cert.promptTokens} prompt / ${cert.completionTokens} completion tokens`,
  );
  console.log(
    `[zero-cost] zero-attempt-policy=${cert.zeroAttemptPolicyPassed === true ? "PASS" : "FAIL"} ` +
      `(cloudAttempts=${cert.cloudAttempts} text=${cert.textOnlyAttempts} image=${cert.imageAttempts})`,
  );
  console.log(
    `[zero-cost] coverage textLayer=${cert.textLayer} expected=${cert.questionsExpected} ` +
      `extracted=${cert.questionsExtracted} strictTier0=${cert.strictTier0} ` +
      `localRecoveries=${cert.localRecoveries} quarantined=${cert.quarantined} ` +
      `placeholderFallbacks=${cert.placeholderFallbackCards} usableCards=${cert.usableCards}`,
  );
  console.log(
    "[zero-cost] note: placeholder fallbacks are review stubs, not usable coverage; " +
      "strict Tier-0 plus local recoveries are the only cards that count as retained content.",
  );

  const failures = certificationFailures(cert);
  if (failures.length > 0) {
    console.error(`[zero-cost] FAIL: ${failures.join("; ")}`);
    process.exit(1);
  }
  console.log(
    "[zero-cost] PASS: 0 attempted requests, 0 tokens, $0.0000 actual offline spend",
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}
