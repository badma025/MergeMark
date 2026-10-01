# R1 progress checkpoint — PARTIAL, not accepted

Started:2026-09-28T16:01:06Z. Review recorded:2026-09-28T17:15:39Z (approximately 75 minutes into the90-minute timebox). Worker stopped and interrupted; no next bundle started. The deadline was17:31:06Z; review is delivered early because the worker returned partial work and the named body checks remain unsatisfied.

Preserved source/scripts/patches: `output/zero-cost-orchestration/r1-checkpoint-20260928-1715utc/`. No commits, pushes, deployments, cloud ingestion, or new dependencies. Original dirty work and prior snapshots remain.

## Cost verification

Last full accepted cost baseline:22/22 named digital papers recorded zero cloud/image/text attempts, zero prompt/completion tokens, and$0.0000. Fresh affected R1 imports also record zero attempts/tokens/cost. This is separate from extraction quality. Final changed code still requires full fresh certification.

## Before/after evidence

Last complete corpus remains `run-20260928c1b`: historical rawstrict141/244; test rawstrict49/69. **No full after-corpus run was completed**, so these subset results are not a new corpus percentage.

Counts below are strict/recovered/quarantined from saved artifacts, not accepted source-quality scores.

| Paper | Before: complete c1b run | R1 saved affected run | Review result |
|---|---|---|---|
| January2025 D1 |8 parentIDs;5/3/0 |8 parentIDs;5/3/0 |Parent map retained; no new strict gain |
| AQA GCSEFM2024 |21 parentIDs;8/11/2 |23 parentIDs;8/11/4 |Q5/Q22 IDs restored, but both are zero-mark placeholders |
| CIE2022 |9 parentIDs;5/3/1 |10 parentIDs;6/2/2 |Q4 mapped; Q5 now placeholder; Q4's rawstrict classification is invalid because maths is visibly garbled |
| CIE2014 |12 parentIDs;3/6/3 |12 parentIDs;3/6/3 |Controls retained; existing quarantines remain |
| Physics2021 |30/1/0 |30/1/0 |No observed regression |
| Physics2024 |32/0/0 |32/0/0 |No observed regression; final renderer still pending |
| CS2024 |2/9/0 |2/9/0 |No observed regression |

The affected imports precede the final narrow Q-prefix correction. That last correction has a passing targeted test and build, but no fresh import after it. Do not claim final-source corpus certification.

## Implemented and tested

- Improved page-number/heading distinction and case-sensitive isotope matching restore AQA formula-heading IDs.
- Declared-count filtering no longer discards strong headings above the count; a final lone-heading candidate is handled.
- Broad lowercase Q-prefix rejection was narrowed and a genuine formula-heading regression added. It remains a lexical heuristic; source-context proof and legitimate prose-heading negatives still need review.
- Earlier preserved changes include same-line mark/unit checks and repeated-footer deduplication. They are not all new work in this timebox.
- Saved doc_map suite:24passed/0failed (`r1-docmap-tests4.log`).
- Root verified final Q-prefix targeted test:1passed/0failed, exit0 (`r1-root-final-targeted.log`).
- Final build passed. Full Cargo, final Physics24 renderer, and all-corpus rendering have not passed their final gates.

## Remaining named failures

1. AQA Q5/Q22 and CIE Q5: document-map starts exist, but `deterministic::carve_span` fails its end-boundary check and emits placeholders. AQA Q5 fallback contains Q6. Body isolation is not complete.
2. CIE Q4: corrupt control/math text is still marked strict with `needs_review=false`. The worker's report caveat is not an implemented quality flag. No narrow glyph-quality gate was added.
3. No regression evidence yet demonstrates correct complete bodies for these three carving failures. IDs alone cannot satisfy R1.
4. Out of this bundle: November marks; CS structure/endings/tables and the unaccepted global(i) exclusion; complex maths/symbols; MadasQ8/Q9; parallel PDFium integration; final corpus/tests/rendering.

## Revised next assignment and estimate

**Next bundle, only after review/resumption: R1b carving seam and truthful quality flags.** Owner remains the same native Flash worker. Scope: the doc-map-to-carver boundary contract for AQA Q5/Q22 and CIE Q5, isolated fallback text, narrow corrupt-glyph review flag for CIE Q4, and focused regression tests. No matrix reconstruction, marks overhaul, or CS refactor.

Completion checks: no boundary-failure placeholder on the three named source-readable questions; each body contains only its own source text/subparts/marks; Q5 must not include Q6; genuine missing boundaries still fail; CIE Q4 cannot count as strict while its expression is corrupt; fresh affected imports have zero attempts/tokens/cost; tests and root source review pass.

Planning estimate: **60–90 minutes for one focused R1b attempt**, with another hard progress review and no automatic extension. This is not a completion guarantee. A reliable overall finish ETA is not supported by the current throughput: usable/accepted quality did not improve in this review, and at least four further work families remain (marks, CS, maths, final integration). Do not reuse the worker's “three more bundles” as a supported finish estimate. Re-estimate after an accepted carving repair with measured corpus impact.

Original acceptance criteria remain unchanged, including>=95% historical strict acceptance and all final tests/render checks. R1 is PARTIAL; work is stopped for this progress review.
