# Recovery plan

**Status: stopped for R1 progress review; PARTIAL, not accepted.** Review recorded 2026-09-28T17:15:39Z, within the16:01:06–17:31:06UTC timebox. See R1-PROGRESS-CHECKPOINT.md. No next bundle dispatched.

The root interrupted `/root/zero_cost_builder` on 2026-09-28 at 14:24 UTC and stopped its remaining `run-20260928c1c` calibration process tree. The final process check found no cargo, rustc, or e2e_import process. The incomplete c1c artifacts are not certification evidence.

Current source, scripts, tracked/staged patches, and status inventory are preserved in `output/zero-cost-orchestration/paused-20260928-1425utc/`. Earlier snapshots and the original dirty baseline remain intact. Unrelated workspace files remain in place. Nothing was reverted, committed, pushed, or deployed.

This plan supersedes the broad C1 boundary assignment. Reuse the same native Flash worker after resumption, with one bounded implementation bundle at a time. Root retains acceptance review.

## Original acceptance criteria remain binding

- Every digital question-paper import attempts zero cloud, text, and image requests; prompt/completion tokens are zero and cloud cost is exactly $0.0000. Any non-whitespace extracted text prohibits cloud fallback.
- Historical strict Tier-0 acceptance is at least 95%, targeting 100%. Recoveries, placeholders, quarantines, missing questions, and extra questions remain separate.
- Physics 2021 and 2024 have complete coverage, zero quarantine, and zero vision fallback. Physics 2024 rendering passes 32/32.
- Corpus output preserves source maths, code, tables, and marks, with no KaTeX crashes, math-wrapped table delimiters, or prompt-margin debris.
- The full default `cargo test --manifest-path src-tauri/Cargo.toml` passes without regressions. Ignored/environment checks are disclosed. Targeted or serialized tests do not replace this gate.

## Verified cost and unfinished quality

**Cost baseline accepted:** fresh `run-20260928fresh` recorded zero cloud/image/text attempts, zero tokens, and $0.0000 for all 22 named papers. Fourteen accounting rejection tests and seven canonical verifier tests passed. The latest complete `run-20260928c1b` also records 22/22 cost passes. Final changed code still needs fresh certification.

**Extraction quality is not accepted.** These are raw parser counts, not accepted content-quality percentages:

| Cohort | Earlier complete run | Latest complete c1b run | Interpretation |
|---|---|---|---|
| Historical, 18 papers | 140/244 strict (57.4%) | 141/244 strict (57.8%) | Only one additional raw strict result; at least 232/244 would be needed if this denominator is confirmed |
| Test papers, 4 papers | 48/70 strict | 49/69 strict | Not directly comparable: a CIE fixture changed from 11 to 10 source questions and needs evidence review |

Wrong/extra parent IDs cannot count toward accepted strict coverage. Source-fixture changes must be justified independently of parser output. Cost PASS and ID MATCH do not establish accurate, usable content.

## Preserved work since the last accepted dependency

The worker changed `doc_map.rs`, `validate.rs`, and the source-ID fixture/generator. These changes are **unaccepted**, not discarded.

- January 2025 D1 now maps parent IDs 1-8 without extra IDs; raw strict count rose from 4 to 5 and quarantine fell from 6 to 0. Source body/mark comparison remains necessary.
- AQA GCSE Further Maths now misses Q5 and Q22; earlier missing IDs were Q2, Q6, Q14, Q19, Q21, and Q22. Q5 is a new mapping loss. Strict count remains 8.
- CIE 2022 still misses Q4. Removing Q11 is a claimed fixture correction, not a parser repair. CIE 2014 now maps all 12 parent IDs but retains three quarantines.
- CS 2022 raw strict count regressed from 5 to 4. CS 2023/2024 remain at 3/2 strict.
- Saved tests: doc_map 20 passed; validate 31 passed. The earlier boundary-test command failed because its Cargo arguments were invalid. Full-suite and final-render gates remain pending.
- Root review risks: heading resolution drops all headings above a declared count despite a comment promising only bare-number filtering; gap filling may mistake ordinary numbers for headings; the final lone heading can be rejected above the largest strong heading. The validator globally ignores lettered part `(i)` instead of distinguishing Roman subparts by context.

## Highest-impact failure groups

Counts below are first recovery reasons in c1b logs; they are diagnostic frequencies, not additive guaranteed gains.

| Group | Observed examples | Completion evidence |
|---|---|---|
| Parent mapping integrity | January D1 Q1-8; AQA FM Q5/Q22; CIE 2022 Q4; CIE 2014 Q2/Q3 as controls | Correct source IDs and complete carved content; graph/page numbers rejected; final and split headings retained |
| Mark provenance | 20 checksum mismatches and 7 ambiguous allocations; November 2024 Q5/Q7/Q8/Q10/Q11/Q13/Q14/Q16/Q17/Q19/Q20/Q22/Q23 | Source allocations and totals reconcile exactly once; continuation/END regions bounded; ambiguity still flagged |
| CS structure, endings, tables | 15 subpart gaps and 16 terminal-ending failures across the corpus; CS 2024 Q2/Q3/Q6/Q7/Q8; CS 2022 Q5/Q7/Q12 | Actual labels and tables recovered; hierarchical Roman handling; code preserved; genuine missing parts still fail |
| Maths, symbols, and carving | Core Pure 2021 Q2/Q4/Q5/Q6/Q7/Q9; Core Pure 2022/2023 and FP2; Madas Q8/Q9; Physics 2021 Q5 exponent | Source-derived glyph/path repairs with exact consumption and actual card/render comparisons; no guessed expressions |
| Integration | Prior parallel-PDFium heap failure; full Cargo and all-card rendering pending | Real concurrency diagnosis/fix, full suite green, final fresh cost and quality certification |

## Only next bundle: R1 — parent mapping integrity

**Owner:** existing `zero_cost_builder`, native `astra_flash_builder` / DeepSeek V4.1 Flash. Authorized scope is R1 only during this timebox.

**Scope:** doc_map, directly necessary carving seams/tests, source-fixture evidence, and an R1 report. Preserve the pending patch. Do not expand into matrices, CS validators, or mark summation. Any necessary scope expansion must be reported to root before implementation.

**Named completion checks:**

1. January 2025 D1: retain exactly source parents 1-8, with their text, subparts, and diagrams. Repeated answer-booklet footers and page/graph/table numbers must not create parents. A declared count alone must not erase strong headings or nonconsecutive sections.
2. AQA GCSE FM 2024: recover Q5 and Q22 and retain all parents 1-23. Cover short pages, split headings, and preceding mark tags.
3. CIE 2022: recover Q4 and inspect the final question/Additional Page to settle the source count independently. Keep CIE 2014 Q2/Q3 as regression controls.
4. Add meaningful failing-example and negative tests. Assert exact IDs and retained body/parts, not just the existence of a card. Cover graph labels, quantities, page numbers, final lone headings, and sections starting above one.
5. Run affected tests and fresh imports. Require zero attempted requests/tokens, no new quarantine or content loss, and source comparisons. Existing doc_map tests must pass.
6. Root reviews code, sources, and saved results together. R1 is complete only when every named check passes. Remaining maths/marks/recovery failures are listed explicitly; R1 completion is not Phase 3 completion.

After R1 review, choose only one next bundle, initially November mark provenance. Then CS structure, then one maths/carving family at a time. Each receives named examples and concrete checks before dispatch. No unchecked chain of bundles.

## Mandatory 90-minute progress checkpoint

The clock does not run while implementation is paused. Before resuming, root records the start and deadline (start plus 90 minutes) in `RECOVERY-CHECKPOINT.json`.

- **0-15 minutes:** inspect the preserved patch and named source failures; confirm the baseline.
- **15-60:** implement R1 and run focused tests.
- **60-75:** fresh affected imports and source comparisons.
- **75-90:** no new scope; prepare evidence and root review. Do not start a full-corpus run unless it can finish before the deadline.
- **By minute 90:** stop and deliver a progress review even if incomplete. Interrupt any running worker/task-owned background validation, preserve changes, and label partial artifacts incomplete. No automatic extension or next bundle before the review; overruns require explicit user direction.

The review must include before/after corpus results, fixed defects and test evidence, remaining named failures, regressions, elapsed time, and a revised estimate. If a full after-run is unavailable, show the last complete corpus and fresh affected-paper rows separately; never blend them into a fresh corpus claim.

**Estimate:** the next bundle has one 90-minute timebox, not a completion promise. At least five repair/verification families remain. A defensible overall finish estimate is not established given repeated partial deliveries and minimal strict-count gain. The first timed review must provide a revised range based on accepted progress and measured throughput.

## Completion and communication rules

- `COMPLETE`: implemented code, meaningful passing tests with commands/exits, fresh source-compared output, and root acceptance.
- `PARTIAL`: explicitly separate implemented, tested, unreviewed, and missing work. Ordinary unfinished coding is not an external blocker.
- `BLOCKED`: identify the external dependency and evidence.
- Put current status at the top of reports. Do not label an unfinished bundle complete or imply acceptance through `ready_for_review`.
- User updates only for meaningful results, blockers, decisions, and the mandatory checkpoint. No periodic waiting messages.
- Preserve all existing work. No commits, pushes, or deployments.
