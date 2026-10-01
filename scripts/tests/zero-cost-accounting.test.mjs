// Certification-safety regressions for the zero-cost calibration accounting.
//
//   node --test scripts/tests/zero-cost-accounting.test.mjs
//
// Every test uses a controlled fake child (the injectable `spawn`) so we can
// prove the runner accepts a genuinely fresh, policy-clean run and REJECTS
// stale output, a non-zero child, absent/malformed artifacts, and an attempted
// cloud request even when tokens and reported cost are zero.

import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync, utimesSync } from "node:fs";
import os from "node:os";
import path from "node:path";

import {
  runPaper,
  certificationFailures,
  knownNumber,
  failingRows,
} from "../lib/zero-cost-accounting.mjs";

function tempDir() {
  return mkdtempSync(path.join(os.tmpdir(), "zc-accounting-"));
}

function makePaper(root, name = "fake21") {
  const file = path.join(root, "fake.pdf");
  writeFileSync(file, "%PDF-1.4\n% fake\n");
  return { name, file, subject: "Physics", module: "A-Level Physics", cohort: "historical" };
}

function cleanCert(over = {}) {
  return {
    mode: "offline",
    cloudAttempts: 0,
    imageAttempts: 0,
    textOnlyAttempts: 0,
    promptTokens: 0,
    completionTokens: 0,
    costUsd: "0.0000",
    tokensZero: true,
    zeroAttemptPolicyPassed: true,
    questionsExpected: 3,
    questionsExtracted: 3,
    strictTier0: 3,
    localRecoveries: 0,
    quarantined: 0,
    usableCards: 3,
    placeholderFallbackCards: 0,
    ...over,
  };
}

/// Builds a fake `spawn` that locates `--out <dir>` and invokes `produce`.
/// `produce` may return `{ status, signal }` to control the child result.
function fakeSpawn(produce) {
  return (cmd, args) => {
    const outIdx = args.indexOf("--out");
    const outDir = args[outIdx + 1];
    const paperName = args[args.indexOf("--") + 2];
    const slug = paperName.replaceAll(" ", "_").replaceAll("'", "");
    return produce({ outDir, paperName, slug }) ?? { status: 0 };
  };
}

function writeArtifacts(outDir, slug, cert) {
  mkdirSync(outDir, { recursive: true });
  writeFileSync(path.join(outDir, `${slug}_offline_certification.json`), JSON.stringify(cert, null, 2));
  writeFileSync(path.join(outDir, `${slug}_cards.md`), "===== Q1 =====\ncontent\n");
  writeFileSync(
    path.join(outDir, `${slug}_cards.json`),
    JSON.stringify([{ question_number: 1 }, { question_number: 2 }, { question_number: 3 }]),
  );
}

test("a fresh, clean, zero-attempt run is accepted", () => {
  const root = tempDir();
  const paper = makePaper(root);
  const spawn = fakeSpawn(({ outDir, slug }) => writeArtifacts(outDir, slug, cleanCert()));
  const row = runPaper({ paper, spawn, outRoot: path.join(root, "out"), runId: "t1" });
  assert.equal(row.status, "OK", JSON.stringify(row.failures));
  assert.deepEqual(row.failures, []);
  assert.equal(row.counters.cloudAttempts, 0);
  assert.equal(row.counters.costUsd, "0.0000");
});

test("stale previous output is rejected, never mistaken for this run", () => {
  const root = tempDir();
  const paper = makePaper(root);
  const spawn = fakeSpawn(({ outDir, slug }) => {
    writeArtifacts(outDir, slug, cleanCert());
    const past = new Date(Date.now() - 60 * 60 * 1000);
    utimesSync(path.join(outDir, `${slug}_offline_certification.json`), past, past);
  });
  const row = runPaper({ paper, spawn, outRoot: path.join(root, "out"), runId: "t2" });
  assert.equal(row.status, "FAIL");
  assert.ok(row.failures.some((f) => f.includes("stale")), row.failures.join("; "));
});

test("a pre-seeded artifact from a previous run cannot satisfy a silent child", () => {
  const root = tempDir();
  const outRoot = path.join(root, "out");
  const paper = makePaper(root);
  writeArtifacts(path.join(outRoot, "fake21-t3", "fake21"), "fake21", cleanCert());
  const spawn = fakeSpawn(() => {}); // child writes nothing
  const row = runPaper({ paper, spawn, outRoot, runId: "t3" });
  assert.equal(row.status, "FAIL");
  assert.ok(
    row.failures.some((f) => f.includes("no certification artifact")),
    row.failures.join("; "),
  );
});

test("a non-zero child is rejected even when plausible artifacts exist", () => {
  const root = tempDir();
  const paper = makePaper(root);
  const spawn = fakeSpawn(({ outDir, slug }) => {
    writeArtifacts(outDir, slug, cleanCert());
    return { status: 7 };
  });
  const row = runPaper({ paper, spawn, outRoot: path.join(root, "out"), runId: "t4" });
  assert.equal(row.status, "FAIL");
  assert.ok(row.failures.some((f) => f.includes("exited with code 7")), row.failures.join("; "));
});

test("a child killed by a signal is rejected", () => {
  const root = tempDir();
  const paper = makePaper(root);
  const spawn = fakeSpawn(({ outDir, slug }) => {
    writeArtifacts(outDir, slug, cleanCert());
    return { signal: "SIGTERM" };
  });
  const row = runPaper({ paper, spawn, outRoot: path.join(root, "out"), runId: "t5" });
  assert.equal(row.status, "FAIL");
  assert.ok(row.failures.some((f) => f.includes("signal SIGTERM")), row.failures.join("; "));
});

test("absent artifact is rejected", () => {
  const root = tempDir();
  const paper = makePaper(root);
  const spawn = fakeSpawn(() => {});
  const row = runPaper({ paper, spawn, outRoot: path.join(root, "out"), runId: "t6" });
  assert.equal(row.status, "FAIL");
  assert.ok(
    row.failures.some((f) => f.includes("no certification artifact")),
    row.failures.join("; "),
  );
});

test("malformed certification artifact is rejected", () => {
  const root = tempDir();
  const paper = makePaper(root);
  const spawn = fakeSpawn(({ outDir, slug }) => {
    mkdirSync(outDir, { recursive: true });
    writeFileSync(path.join(outDir, `${slug}_offline_certification.json`), "{ not json");
    writeFileSync(path.join(outDir, `${slug}_cards.md`), "x");
    writeFileSync(path.join(outDir, `${slug}_cards.json`), "[]");
  });
  const row = runPaper({ paper, spawn, outRoot: path.join(root, "out"), runId: "t7" });
  assert.equal(row.status, "FAIL");
  assert.ok(row.failures.some((f) => f.includes("unreadable")), row.failures.join("; "));
});

test("an attempted cloud request fails even with zero tokens and $0 cost", () => {
  const root = tempDir();
  const paper = makePaper(root);
  const spawn = fakeSpawn(({ outDir, slug }) =>
    writeArtifacts(outDir, slug, cleanCert({ cloudAttempts: 2, zeroAttemptPolicyPassed: false })),
  );
  const row = runPaper({ paper, spawn, outRoot: path.join(root, "out"), runId: "t8" });
  assert.equal(row.status, "FAIL");
  assert.ok(row.failures.some((f) => f.includes("cloudAttempts=2")), row.failures.join("; "));
  assert.equal(row.counters.cloudAttempts, 2, "attempt count retained");
  assert.equal(row.counters.costUsd, "0.0000", "reported offline cost stays $0");
});

test("unknown counters are never silently treated as zero", () => {
  const root = tempDir();
  const paper = makePaper(root);
  const spawn = fakeSpawn(({ outDir, slug }) => {
    const cert = cleanCert();
    delete cert.cloudAttempts;
    delete cert.promptTokens;
    writeArtifacts(outDir, slug, cert);
  });
  const row = runPaper({ paper, spawn, outRoot: path.join(root, "out"), runId: "t9" });
  assert.equal(row.status, "FAIL");
  assert.equal(row.counters.cloudAttempts, null);
  assert.equal(row.counters.promptTokens, null);
  assert.ok(row.failures.some((f) => f.includes("cloudAttempts")), row.failures.join("; "));
});

test("a missing source file is rejected before any child runs", () => {
  const root = tempDir();
  const paper = {
    name: "ghost",
    file: path.join(root, "does_not_exist.pdf"),
    subject: "Physics",
    module: "A-Level Physics",
    cohort: "historical",
  };
  let spawnCalled = false;
  const spawn = () => {
    spawnCalled = true;
    return { status: 0 };
  };
  const row = runPaper({ paper, spawn, outRoot: path.join(root, "out"), runId: "t10" });
  assert.equal(row.status, "MISSING_FILE");
  assert.equal(spawnCalled, false);
});

test("a source-ID fixture hash mismatch is reported and cannot verify coverage", () => {
  const root = tempDir();
  const paper = makePaper(root, "fake21");
  const spawn = fakeSpawn(({ outDir, slug }) => writeArtifacts(outDir, slug, cleanCert()));
  const fixture = { papers: { fake21: { fileHash: "DEADBEEF", expectedIds: [1, 2, 3] } } };
  const row = runPaper({ paper, spawn, outRoot: path.join(root, "out"), runId: "t11", fixture });
  assert.equal(row.status, "FAIL");
  assert.ok(row.failures.some((f) => f.includes("hash mismatch")), row.failures.join("; "));
  assert.equal(row.idCoverage, "UNKNOWN");
});

test("a matching fixture verifies extracted IDs", () => {
  const root = tempDir();
  const paper = makePaper(root, "fake21");
  const spawn = fakeSpawn(({ outDir, slug }) => writeArtifacts(outDir, slug, cleanCert()));
  const fixture = { papers: { fake21: { expectedIds: [1, 2, 3] } } };
  const row = runPaper({ paper, spawn, outRoot: path.join(root, "out"), runId: "t12", fixture });
  assert.equal(row.status, "OK", JSON.stringify(row.failures));
  assert.equal(row.idCoverage, "MATCH");
});

test("certification gate helper rejects every attempted-request counter", () => {
  assert.deepEqual(certificationFailures(cleanCert()), []);
  for (const over of [
    { cloudAttempts: 1 },
    { imageAttempts: 1 },
    { textOnlyAttempts: 1 },
    { zeroAttemptPolicyPassed: false },
    { tokensZero: false },
    { promptTokens: 1 },
    { completionTokens: 1 },
    { costUsd: "0.0001" },
    { cloudAttempts: undefined },
  ]) {
    assert.ok(certificationFailures(cleanCert(over)).length > 0, JSON.stringify(over));
  }
  assert.equal(knownNumber(undefined), null);
  assert.equal(knownNumber(0), 0);
});

test("failingRows surfaces a rejected row so the runner exits non-zero", () => {
  const root = tempDir();
  const paper = makePaper(root);
  const spawn = fakeSpawn(({ outDir, slug }) => {
    writeArtifacts(outDir, slug, cleanCert({ cloudAttempts: 1, zeroAttemptPolicyPassed: false }));
  });
  const bad = runPaper({ paper, spawn, outRoot: path.join(root, "out"), runId: "t13" });
  const okPaper = { ...paper, name: "ok21" };
  const okSpawn = fakeSpawn(({ outDir, slug }) => writeArtifacts(outDir, slug, cleanCert()));
  const good = runPaper({ paper: okPaper, spawn: okSpawn, outRoot: path.join(root, "out"), runId: "t13" });
  const failing = failingRows([bad, good]);
  assert.equal(failing.length, 1);
  assert.equal(failing[0].name, "fake21");
});
