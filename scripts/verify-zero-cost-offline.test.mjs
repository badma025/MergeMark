// Pure-validator regressions for the offline certification command.
// Run with: node --test scripts/verify-zero-cost-offline.test.mjs

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  artifactIsFresh,
  certificationFailures,
  runFailureMessage,
} from "./verify-zero-cost-offline.mjs";

const cleanCert = {
  costUsd: "0.0000",
  promptTokens: 0,
  completionTokens: 0,
  cloudAttempts: 0,
  zeroAttemptPolicyPassed: true,
};

test("a clean offline run certifies", () => {
  assert.deepEqual(certificationFailures(cleanCert), []);
});

test("an attempted request fails certification even at zero cost", () => {
  const failures = certificationFailures({
    ...cleanCert,
    cloudAttempts: 3,
    zeroAttemptPolicyPassed: false,
  });
  assert.ok(failures.some((f) => f.includes("cloudAttempts=3")), failures.join("; "));
  assert.ok(
    failures.some((f) => f.includes("zeroAttemptPolicyPassed")),
    failures.join("; "),
  );
});

test("nonzero tokens fail certification", () => {
  const failures = certificationFailures({ ...cleanCert, promptTokens: 12 });
  assert.ok(failures.some((f) => f.includes("promptTokens=12")), failures.join("; "));
});

test("a missing or false zero-attempt policy flag fails certification", () => {
  assert.ok(
    certificationFailures({ ...cleanCert, zeroAttemptPolicyPassed: undefined }).some((f) =>
      f.includes("zeroAttemptPolicyPassed"),
    ),
  );
  assert.ok(
    certificationFailures({ ...cleanCert, zeroAttemptPolicyPassed: false }).some((f) =>
      f.includes("zeroAttemptPolicyPassed"),
    ),
  );
});

test("a non-zero reported cost fails certification", () => {
  const failures = certificationFailures({ ...cleanCert, costUsd: "0.0100" });
  assert.ok(failures.some((f) => f.includes("costUsd=0.0100")), failures.join("; "));
});

test("child error, signal and non-zero status are detected", () => {
  assert.equal(runFailureMessage(0, null, undefined), null);
  assert.match(runFailureMessage(1, null, undefined), /exited with code 1/);
  assert.match(runFailureMessage(null, "SIGTERM", undefined), /signal SIGTERM/);
  assert.match(
    runFailureMessage(null, null, new Error("spawn cargo ENOENT")),
    /failed to start/,
  );
});

test("stale or missing artifacts are rejected", () => {
  assert.equal(artifactIsFresh(9_000, 5_000), true);
  assert.equal(artifactIsFresh(1_000, 5_000), false, "old file must be stale");
  assert.equal(artifactIsFresh(Number.NaN, 5_000), false, "missing mtime is stale");
});
