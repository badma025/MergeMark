# R1 — parent mapping integrity (PARTIAL / rejected pending correction)

**STATUS: PARTIAL.** R1 was reviewed and NOT accepted. Parent *IDs* are
correct for the named papers, but the rendered bodies are placeholders for the
AQA Q5/Q22 and CIE Q5 cases — so the named "complete body / no quarantine"
checks FAIL. Elapsed this timebox ≈65 min, not 90. No new bundle started.

Timebox: 2026-09-28T16:01:06Z–17:31:06Z (stop-coding 17:16:06Z). Owner:
`/root/zero_cost_builder`. Scope: `doc_map.rs`, a directly necessary carve seam,
tests, source-fixture evidence, this report.

## What R1 actually achieved

**Parent ID mapping (real, verified):**
- Jan 2025 D1 → Q1..Q8, marks 9,13,5,6,7,13,14,8 = 75; no graph/page/answer-book
  parents; 3 diagrams written.
- AQA GCSE FM 2024 → IDs Q1..Q23 present (Q5, Q22 start lines recovered).
- CIE 2022 → IDs Q1..Q10; final Q10/Additional Page render-confirmed; fixture
  corrected 11→10.
- CIE 2014 → Q1..Q12 controls retained.

**What is NOT achieved (honest):** ID mapping ≠ complete body. The r1f AQA
Q5/Q22 cards are **marks 0, needs_review placeholders** (deterministic carver
`boundary_not_found`, and the Q5 fallback text includes Q6). The r1e CIE
Q4/Q5 cards are strict-false but the Q4 body is garbled control/math text; Q5 is
now `boundary_not_found`/quarantine. So the named no-quarantine/content checks
FAIL.

## Counts — actual (root-reviewed), not my earlier table

| Paper | Actual strict/rec/quarantine | Note |
|---|---|---|
| AQA GCSE FM 2024 | **8 / 11 / 4** | not "improved" |
| CIE 2022 (tp04) | **6 / 2 / 2** | my earlier 5/3/1 was stale |
| Jan 2025 | 5 / 3 / 0 | IDs correct, bodies mixed |
| CIE 2014 (tp07) | 3 / 6 / 3 | IDs correct |

Mapped vs placeholder: AQA Q5/Q22 are mapped IDs backed by placeholder cards
(marks 0). They must not be reported as recovered content.

## Implemented in `doc_map.rs` (general, source-driven)

1. AQA `*NN*` page-marker rejection.
2. Same-line marks-tag guard (previous question's `[N marks]` no longer
   suppresses the next heading on one-question pages).
3. Same-line unit check (diagram label "A" no longer read as amperes).
4. `is_printed_page_number` requires the number to be alone on its line.
5. Footer first-occurrence dedup (answer-booklet footer repeats no longer break
   monotonicity and discard all footers).
6. `resolve_lone_headings`: lone/ambiguous candidates accepted only when they
   fill a gap in the strong set; declared count constrains only ambiguous
   candidates, never strong headings; a final lone heading only at `max+1` and
   after the last strong heading.
7. Case-sensitive isotope guard (`22 f(x)` no longer matches fluorine).
8. **Narrowed `Q`-cross-reference rejection (correction).** Lowercase alone is
   not proof: it now fires only when the next non-empty line begins with a
   lower-case **prose connector** (`is/and/or/the/of/to/for/where/which/so/then/
   hence/when/with`). A genuine "Q1\nx = …" formula heading now survives; the
   spurious CIE "Q5\nis a. …" is still rejected. Covered by
   `q_prefixed_margin_label_is_not_a_heading` (negative + positive).

## Carve seam — NOT fixed (root priority)

`boundary_not_found` for AQA Q5 ("5 y = …"), AQA Q22 ("22 f(x) = +") and CIE
Q5 comes from `deterministic::carve_span`: `found_end` is false because the
doc-map span is clipped before the next heading / marks tag, and
`self_terminating_end` finds no boundary line in its ±4-line window. The carve
seam was not repaired inside the remaining timebox.

## CIE 2022 Q4 corruption — honestly flagged, not fixed

The r1e CIE Q4 body contains garbled control/math text and reports
needs_review. A narrow glyph-quality gate to prevent a false-strict was
authorized but not implemented in the remaining time; the card is flagged, not
claimed clean.

## Source renders (reviewable evidence paths)

* CIE 2022 final Q10 + Additional Page:
  `output/zero-cost-orchestration/r1-source-renders/tp04-p-19.png` (Q10(d))
  and `tp04-p-20.png` ("Additional Page").
* CIE 2022 Q5 source: `output/zero-cost-orchestration/r1-source-renders/tp04-p-08.png`.
* AQA FM Q5 source: `output/zero-cost-orchestration/r1-source-renders/aqafm-p-04.png`.
* AQA FM Q22/Q23 source: `aqafm-p-21.png`, `aqafm-p-22.png`
  (page 22 shows "END OF QUESTIONS" then Q23).
* Earlier: `output/zero-cost-orchestration/src-render-tp04/p-19.png`, `q-20.png`.

## Commands / exits

* `cargo build --bin e2e_import` → exit 0.
* `cargo test --lib doc_map::tests:: -- --test-threads=1` → 24 passed / 0 failed
  (`output/zero-cost-orchestration/r1-docmap-tests4.log`); plus the narrowed
  `q_prefixed_margin_label_is_not_a_heading` → 1 passed.
* Fresh offline imports (exit 0, cloud=0 image=0 tokens=0 $0.0000):
  jan2025 → IDs 1–8; aqafm24 → IDs 1–23; tp04 → IDs 1–10; tp07 → IDs 1–12;
  physics21 30/1/0, physics24 32/0/0, cs224 2/9/0 (no regression). Logs
  `r1b-*.log`, `r1e-*.log`, `r1f-*.log`; artifacts `r1b-*`, `r1e-*`, `r1f-*`.

## Missing work (explicit)

* Carve seam for AQA Q5/Q22 and CIE Q5 (placeholder bodies) — root priority.
* Narrow glyph-quality gate so corrupted CIE Q4 is not strict-clean.
* Full-corpus re-run (last complete corpus remains `run-20260928c1b`).
* Out of R1 scope: Nov 2024 mark provenance, CS structure/endings/tables,
  maths/symbol carving, Madas Q8/Q9.

## Revised estimate

ID mapping improved, but body/quarantine checks still fail. Call this a REJECTED
PARTIAL. The next bounded bundle should be the carve seam (Q5/Q22/CIE Q5), then
mark provenance, CS, then maths. At measured throughput, ≥3 more 90-minute
bundles remain; not committed.
