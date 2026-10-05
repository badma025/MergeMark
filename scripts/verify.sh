#!/usr/bin/env bash
# One verify chain: cargo test, calibration, triage of papers/unused.
#   bash scripts/verify.sh <stamp>
# Logs go to output/fix-log/<stamp>-*.log and the status to <stamp>-status.txt.
# It takes 1.5-2 hours: run it in the background and do not edit Rust in this
# tree meanwhile (calibration and triage `cargo run` once per paper).
#
# No `set -e`: a failed or stopped stage does not stop the next one; read the
# status file. A test whose paper is missing fails (MERGEMARK_REQUIRE_FIXTURES),
# and a test that reads a paper the encrypted fixture list does not hold fails
# the cargo-test stage too (see scripts/fixtures.mjs).
s=$1
if [ -z "$s" ]; then echo "usage: bash scripts/verify.sh <stamp>" >&2; exit 2; fi
L=output/fix-log
mkdir -p "$L"
rm -f "$L/$s-fixtures.log"
MERGEMARK_REQUIRE_FIXTURES=1 MERGEMARK_FIXTURE_LOG="$L/$s-fixtures.log" \
  cargo test --manifest-path src-tauri/Cargo.toml > "$L/$s-cargo-test.log" 2>&1
t=$?
node --test "scripts/tests/*.test.mjs" "scripts/*.test.mjs" "src/lib/*.test.mjs" >> "$L/$s-cargo-test.log" 2>&1 || t=1
node scripts/fixtures.mjs check "$L/$s-fixtures.log" >> "$L/$s-cargo-test.log" 2>&1 || t=1
echo "cargo-test exit=$t" | tee "$L/$s-status.txt"
node scripts/run-zero-cost-calibration.mjs --stamp "$s" > "$L/$s-calibration.log" 2>&1; echo "calibration exit=$?" | tee -a "$L/$s-status.txt"
node scripts/triage-papers.mjs --stamp "unused-$s" > "$L/$s-triage.log" 2>&1; echo "triage exit=$?" | tee -a "$L/$s-status.txt"
echo DONE >> "$L/$s-status.txt"
