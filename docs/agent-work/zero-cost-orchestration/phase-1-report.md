# Phase 1 report - the digital cloud circuit breaker (revision 3)

Author: native Flash implementation worker (`astra_flash_builder` consumer role).
Plan: `docs/agent-work/zero-cost-orchestration/PLAN.md` (Astra-owned; not edited).
Authority: `docs/ZERO_COST_ORCHESTRATION_PROMPT.md`.
STATUS: **ready_for_review**.

Workspace: `C:/Users/alimb/Documents/trae_projects/MergeMark`
Baseline HEAD: `52560233abcf779bc8bd2473ef09c8b84de0e3d7` (dirty tree preserved; nothing
reverted, staged, committed, deployed, or switched to another model).

Revision 2 was one consolidated correction pass over six findings from root review. Revision 3
closes three narrow follow-ups (A: no text threshold may grant network permission; B: inline
marks vs repeated metadata, and the mislabeled-parent safeguard; C: the certification command's
hard gates). Earlier logs remain under `output/zero-cost-orchestration/phase1-*.log` and
`p1c-*.log`.

## Corrections in this pass

### C1 (P1 cost boundary) - the policy is content-based, not size-based

* `TextLayerClass` is a **struct**, not an enum with a `Mixed` variant
  ([doc_map.rs](src-tauri/src/doc_map.rs:315)). It answers one binary question: does ANY page
  carry extracted text?
* **Revision 3 (finding A): there is no threshold of any kind.**
  [`page_has_extracted_text`](src-tauri/src/doc_map.rs:354) is exactly
  `!text.trim().is_empty()`. A complete maths-only question with no prose ("1. Solve x=2. [1]")
  keeps the import local, and so do page numbers, watermarks and OCR noise. The old
  `MIN_PAGE_TEXT_CHARS = 24` / `MIN_PAGE_LETTER_CHARS = 8` gates are deleted, along with
  `MIN_DOCUMENT_TEXT_CHARS`. Cloud compatibility exists only when the extracted text is
  entirely absent (whitespace-only pages included in that test).
* Any page with extracted text therefore makes the WHOLE document local-only, including mixed
  and ambiguous inputs. `is_scanned_only()` is true only for a genuinely image-only input, and
  that is the only shape that keeps cloud compatibility.
* Pages with no extracted text are reported, not used as an escape hatch: the pipeline emits
  `digital document: N/M pages carry extracted text - cloud extraction disabled (zero requests
  permitted); pages [...] have no text layer and are reported locally`
  ([pipeline.rs](src-tauri/src/pipeline.rs:2131)).
* Old cloud-path unit tests were updated, not papered over: they now either model genuinely
  scanned input (`refusing_client_counts_attempts_on_the_cloud_path` uses image pages with no
  text) or bind an explicit internal cloud context through
  [`run_cloud_pipeline`](src-tauri/src/pipeline.rs:7904), which sets a `#[cfg(test)]`-only
  `force_scanned_context` field that does not exist in production builds. Production code
  cannot reach it.
* New regressions: real short paper (< 1000 characters, toggles off), a maths-only one-mark
  question with no prose (zero calls, toggles off), rich page plus blank page plus
  END/copyright page plus image-only page (zero calls, unresolved pages reported), furniture and
  noise pages that over-block rather than pay, and the scan-only control (refusing client
  counts > 0).

### C2 (P1 data loss) - equal subpart marks are allocations, not duplicate totals

* [`stitch_question_items`](src-tauri/src/pipeline.rs:865) now takes
  `expected_parent_marks` (the printed total from the span/footer) and uses it as the ONLY
  evidence for collapsing:
  * `expected == value` and `sum > expected` -> the model repeated the parent total: collapse
    to one mark value and one trailing tag.
  * `expected == sum` -> equal per-part allocations confirmed: keep every tag, keep the sum.
  * anything else -> keep everything exactly as reported and set `marks_ambiguous`, which the
    callers turn into `needs_review` with an explanatory note.
* **Revision 3 (finding B): inline tags are stripped only when the tag's own value is the
  confirmed parent total** ([`mark_tag_value`](src-tauri/src/pipeline.rs:833)). Metadata may
  repeat the parent total (9 on every item) while the content tags are the genuine per-part
  allocations ((a)(b)(c) each "**[3 marks]**"): only the metadata collapses, the three `[3]`
  tags stay, and the inline sum still matches the printed 9. Regression
  `stitch_keeps_inline_allocations_that_differ_from_repeated_metadata`.
* **Revision 3 (finding B): the historical mislabeled-parent safeguard now runs BEFORE the
  stitch** ([pipeline.rs](src-tauri/src/pipeline.rs:5220)). For an exactly-two-item response the
  existing `looks_like_new_question` check decides (so a part-label reset or a new question
  heading goes down the repair path); for any item that opens with its OWN different question
  number ([`leading_question_number`](src-tauri/src/pipeline.rs:1480)) the stitch is refused as
  well. End-to-end regression
  `separate_question_stems_mislabeled_as_one_parent_are_never_merged` proves two clearly
  separate stems sharing a number are re-asked, never silently merged.
* `(a)(b)(c)` worth 3 each with a printed total of 9 is now a 9-mark card with three tags.
  Heterogeneous allocations (2/3/4) always sum. Absent marks stay absent. Diagrams, captions,
  kinds, page indexes and topics are still merged in source order.
* Identity refusals are unchanged: a different parent number refuses
  (`ForeignQuestion`), an unreadable number refuses (`UnverifiableIdentity`), and an absent
  number is allowed only because the call is single-question.

### C3 (P2 metrics) - a batch failure no longer demotes its clean siblings

* In [`extract_same_page_batch`](src-tauri/src/pipeline.rs:5861) the digital branch now
  re-runs each span ALONE through the strict Tier-0 gates first, and only a genuinely failed
  span becomes a flagged local recovery. Regression
  `digital_batch_keeps_clean_sibling_strict_and_flags_only_the_failure` asserts
  `deterministic == 1`, `recovered == 1`, and that the clean sibling is NOT `needs_review`.
* This also improved the real corpus: `physics '21` strict Tier-0 rose from 20 to 24 with
  recoveries down from 10 to 6, same zero attempts.

### C4 (P1 production dependency) - digital imports never touch provider or quota

* [`import_requires_provider`](src-tauri/src/commands.rs:58) decides from page text alone.
  `parse_pdf_vision` now computes it BEFORE `resolve_llm_client`
  ([commands.rs](src-tauri/src/commands.rs:1515)): a local-only document gets a
  [`QuestionClient::Local`](src-tauri/src/commands.rs:29) (the offline `RefusingLlm`) and
  `route = None`, so no credentials are read, no free-tier exhaustion can block the import, and
  the free-upload debit is skipped ([commands.rs](src-tauri/src/commands.rs:1670)). The
  paid/scanned path is byte-for-byte the old behaviour (route resolved, debited on success).
* No billing redesign and no production DB change: the debit is the same call, gated on the
  same `FreeTier` match, now expressed over `Option<&BillingRoute>`.
* Tests: `commands::tests::digital_import_never_requires_a_provider` (rich page, short paper,
  blank + image-only pages) and `commands::tests::scanned_only_import_still_requires_a_provider`
  verify the decision boundary in isolation, without Tauri state.

### C5 (P2 stitching coverage) - ordinary single-span responses stitch too

* The aggregate multi-item path in `extract_span`
  ([pipeline.rs](src-tauri/src/pipeline.rs:5169)) previously discarded a two-item response or
  spent a repair round on three-plus items. It now stitches compatible single-parent items
  first (with `expected_parent_marks`), and only falls back to the old repair/truncate
  behaviour when identity cannot be validated.
* Regression `vision_single_question_subparts_stitch_without_repair_request` drives the normal
  full-page vision path with one mock response containing three subpart items and asserts one
  card, `repairs == 0`, `bodies().len() == 1`, and `needs_review == false` (the printed total
  was available). Not a helper test: it exercises `extract_span` end to end.

### C6 (certification honesty) - real spend vs policy vs coverage

* e2e `--offline` records `costUsd: "0.0000"` (a refused request never leaves the machine
  and costs nothing), plus separate booleans `tokensZero` and `zeroAttemptPolicyPassed`, plus
  `placeholderFallbackCards` and `usableCards`. It exits non-zero when attempts > 0 **or**
  tokens > 0 ([e2e_import.rs](src-tauri/src/bin/e2e_import.rs:236)).
* Never-lose-a-question fallback cards are counted and labelled as review stubs; they are
  excluded from `usableCards`, and the wrapper prints "placeholder fallbacks are review stubs,
  not usable coverage".
* **Revision 3 (finding C): the certification command is strict.**
  [`verify-zero-cost-offline.mjs`](scripts/verify-zero-cost-offline.mjs) now fails non-zero on a
  child error, signal or non-zero status (`runFailureMessage`,
  [line 33](scripts/verify-zero-cost-offline.mjs:33)); deletes any earlier artifact before the
  run and rejects a missing or stale one (`artifactIsFresh`,
  [line 28](scripts/verify-zero-cost-offline.mjs:28)); and requires zero `cloudAttempts`, zero
  prompt/completion tokens, `zeroAttemptPolicyPassed === true` and `costUsd === "0.0000"`
  (`certificationFailures`, [line 43](scripts/verify-zero-cost-offline.mjs:43)). Actual offline
  spend is still reported as `$0.0000` even when the policy gate fails. Pure-validator
  regression: [`verify-zero-cost-offline.test.mjs`](scripts/verify-zero-cost-offline.test.mjs).

## Verification (revision 2)

All commands run from the workspace root; logs are in `output/zero-cost-orchestration/`.
PowerShell prints `NativeCommandError` noise when cargo writes progress to stderr; the recorded
exit codes are cargo's own.

| Command | Exit | Result |
| --- | --- | --- |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib` | 0 | **333 passed, 0 failed, 2 ignored** on the final tree (256.96s). Log `p1c-full-lib-final.log` (an earlier identical run is `p1c-full-lib.log`) |
| `cargo build --manifest-path src-tauri/Cargo.toml --lib --bin e2e_import` | 0 | clean, no warnings. Log `p1c-build-lib.log` (a `--bins` build additionally compiles the Tauri app but hit an OS lock on a running `mergemark.exe`; environment, not code) |
| `cargo test ... --lib -- pipeline::tests` | 0 | 77 passed, 0 failed. Log `p1c-pipeline-tests-3.log` |
| `cargo test ... --lib -- doc_map::tests` | 0 | 15 passed, 0 failed (includes the real AQA/Edexcel corpus placement tests). Log `p1c-docmap.log` |
| `cargo test ... --lib -- commands::tests llm::tests` | 0 | 11 passed, 0 failed (2 new command-route tests + llm). Log `p1c-commands-llm.log` |
| `cargo test ... --lib -- deterministic::tests` | 0 | 51 passed, 0 failed. Log `p1c-deterministic.log` |
| `node scripts/verify-zero-cost-offline.mjs "past papers for mergemark/fp2 '24.pdf" "fp2 24" ...` | 0 | cost PASS ($0.0000, 0 tokens); zero-attempt policy PASS; coverage 4 strict + 2 recoveries, 2 placeholder fallbacks, 6 usable. Log `p1c-fp2-script.log` |
| `cargo run --bin e2e_import -- "physics '24.pdf" ... --offline` | 0 | 32/32 strict, 0 attempts, 0 tokens, `$0.0000`. Log `p1c-physics24.log` |
| `cargo run --bin e2e_import -- "physics '21.pdf" ... --offline` | 0 | 31 questions: 24 strict, 6 recoveries, 1 explicit local failure, 0 attempts. Log `p1c-physics21.log` |
| offline sweep: core pure 1 '21, cs2 '22, aqa physics p1, madas_p2 | 0 each | see the table below. Logs `corpus-offline/*.log` |

Revision-3 targeted runs (no full-suite or full-corpus repetition, per root instruction):

| Command | Exit | Result |
| --- | --- | --- |
| `cargo test ... --lib -- pipeline::tests` | 0 | 80 passed, 0 failed (was 77: + maths-only, + mislabeled stems, + inline-tag vs metadata). Log `p1d-pipeline-tests.log` |
| `cargo test ... --lib -- any_extracted_text_makes weak_heading_map_is_still_digital` | 0 | 2 passed (no-threshold classifier). Log `p1d-docmap-tests.log` |
| `cargo test ... --lib -- commands::tests` | 0 | 2 passed (provider-route decision). Log `p1d-commands-tests.log` |
| `node --test scripts/verify-zero-cost-offline.test.mjs` | 0 | 7 passed: clean cert, attempted requests, non-zero tokens, missing/false policy flag, non-zero cost, child error/signal/status, stale artifact. Log `p1d-wrapper-tests.log` |
| `node scripts/verify-zero-cost-offline.mjs no-such-paper.pdf "some paper" <dir>` with a planted stale artifact | 2 | FAIL: harness exited with code 1; the planted stale artifact was deleted before the run (`present after run: False`), so it could never satisfy the gates. Log `p1d-stale-probe.log` |
| `node scripts/verify-zero-cost-offline.mjs "past papers for mergemark/madas_paper_2_t.pdf" "madas p2" <dir>` | 0 | actual offline spend $0.0000, zero-attempt policy PASS, 0 attempts, 16 placeholder stubs / 0 usable cards. Log `p1d-madas-wrapper.log` |
| `cargo build --manifest-path src-tauri/Cargo.toml --lib --bin e2e_import` (forced rebuild) | 0 | `Compiling mergemark ... Finished`, zero warnings. Log `p1d-build-force.log` |

The 2 ignored tests are pre-existing manual helpers
(`stroke_census::tests::diagnose_caption_binding`,
`stroke_census::tests::generate_golden_candidates`).

### Offline corpus snapshot (real PDFs, refreshed, zero spend)

Every paper was classified **digital**, attempted **zero** requests, used zero tokens, and
reported `zeroAttemptPolicyPassed = true`. `placeholder` counts the never-lose-a-question
review stubs, which are not usable coverage.

| Paper | Pages | Questions (expected/extracted) | Strict Tier-0 | Recoveries | Quarantined | Placeholder stubs | Usable cards |
| --- | --- | --- | --- | --- | --- | --- | --- |
| physics '24 | 40 | 32/32 | 32 | 0 | 0 | 0 | 32 |
| physics '21 | 44 | 31/31 | 24 | 6 | 1 | 1 | 30 |
| aqa physics p1 (test_papers) | 36 | 31/31 | 28 | 3 | 0 | 0 | 31 |
| cs2 '22 | 32 | 13/13 | 5 | 8 | 0 | 0 | 13 |
| core pure 1 '21 | 36 | 9/9 | 2 | 0 | 7 | 7 | 2 |
| fp2 '24 | 32 | 8/8 | 4 | 2 | 2 | 2 | 6 |
| madas_p2 | 8 | 16/16 | 0 | 0 | 16 | 16 | 0 |
| **Total** | | **140/140** | **95 (67.9%)** | **19** | **26** | **26** | **114** |

Strict Tier-0 is 95/140; strict plus local recoveries retain real content for 114/140
questions. The remaining 26 are explicit local failures surfaced as review stubs. Dominant
gate reasons: `boundary_not_found` (core pure 1 '21, madas_p2), `marks_checksum_mismatch`,
`strict_seam_validation`, `unmapped_symbol_font_glyphs` (fp2), MCQ option tagging, isotope
notation, `unbalanced_math`, and physics '21 Q31's final-question boundary. Phases 2-3 own these.

The table was measured on revision 2. Revision 3 only loosens classification (every listed
paper was already digital, so its local path is unchanged) and touches LLM-response stitching
paths that these zero-attempt runs never exercise; `madas_p2` was re-run under revision 3
through the certification wrapper and produced identical counts (16/16, 0 strict, 16
quarantined, 16 placeholder stubs).

## Pre-existing defect found while testing (not fixed here)

`marker_client::fix_mark_allocation` ([marker_client.rs](src-tauri/src/marker_client.rs:534))
drops the mark tag, and leaves the surrounding asterisks, when a **single-part** question
carries exactly one mark tag: `"... increases. **[4 marks]**"` becomes `"... increases. ****"`.
`sum_inline_marks` then reads 0 and `has_terminal_ending` fails, so the card is flagged
`content lacks terminal punctuation` even though nothing is wrong. Footers still supply the
marks, so no total is lost, but it inflates review flags (and physics '21 strict acceptance).
It is a pre-existing baseline behaviour and phase 2 owns gate healing; fixing it here would
have widened this correction pass and risked the batch/aggregate paths I was changing. The
batch fixture in this pass was written with real subparts so it does not depend on the quirk.

## Contract check against PLAN.md

| # | Contract | Where satisfied |
| --- | --- | --- |
| 1 | Digital means local, independently of map sufficiency; zero structure/extraction/batch/crop/repair/classification calls; toggles must not re-enable cloud; scanned compat preserved; mixed reported | `doc_map.rs:315/354/361`, `pipeline.rs:151/423/2131`; tests `text_layer_classification_is_not_map_sufficiency`, `digital_short_paper_ingests_locally_with_toggles_off`, `digital_with_blank_and_backmatter_pages_stays_local_and_reports_gaps`, `digital_weak_map_ingests_with_zero_attempted_requests`, `digital_entry_points_attempt_zero_requests`, `refusing_client_counts_attempts_on_the_cloud_path` |
| 2 | Honest recovery: retain nonempty isolated candidates flagged, never suppress flags, explicit local failure when nothing is viable | `deterministic.rs:1949`, `pipeline.rs:3556`; test `digital_failed_gate_recovers_locally_without_cloud`, plus the quarantine counts in the corpus table |
| 3 | Reuse existing seams; explicit policy checked before network activity; additive serialization | `PipelineConfig`/`ImportReport` additions, `chat_with_permit` policy argument, `pipelines::is_local_only_document` + `commands::import_requires_provider` |
| 4 | Stitch only one question; preserve content/diagrams/topics/marks without duplicate totals; distinct parents never stitched; batch stays separate | `pipeline.rs:865` (`mark_tag_value` at 833, mislabeled-stem safeguard at 5220); tests `stitch_preserves_equal_subpart_allocations_and_their_tags`, `stitch_collapses_a_confirmed_repeated_parent_total`, `stitch_keeps_marks_and_flags_ambiguity_without_a_printed_total`, `stitch_refuses_distinct_parent_questions`, `stitch_refuses_missing_identity_when_the_number_is_unreadable`, `text_first_stitches_single_question_subparts_without_vision`, `vision_single_question_subparts_stitch_without_repair_request` |
| 5 | Conservative normalization | Untouched (phases 2-3); see the defect note above |
| 6 | Measured certification; offline refusing/counting client; no inferred $0 | `llm.rs:528`, `e2e_import --offline` (`costUsd` + `zeroAttemptPolicyPassed` + placeholder split), `scripts/verify-zero-cost-offline.mjs` plus its `node --test` regression, which enforce zero attempts, zero tokens and a fresh artifact |

## Outstanding risks and limits

1. **Strict Tier-0 acceptance is 67.9% on this sample, not the 95% target.** Coverage is intact
   (`expected == extracted` everywhere) and 114/140 questions retain real content, but
   `core pure 1 '21`, `cs2 '22`, `fp2 '24` and `madas_p2` are not import-ready. This is the
   phase-2/phase-3 work and the largest quality risk for Astra to gate phase 2 on.
2. **Over-blocking is now deliberate and can hurt a mixed scan.** Revision 3 removed every
   text threshold, so ANY page with a non-whitespace character - a digital cover, a watermark, a
   page number, OCR noise - makes the whole document local-only. A scanned body with any such
   page therefore falls back to flagged local recovery instead of the paid path. The root
   contract chooses this direction explicitly; it is listed here as the user-visible
   consequence.
3. **Mark schemes remain out of scope** (root confirmed). `run_markscheme_pipeline` never
   classifies its document, so a digital mark scheme still spends tokens.
4. **`madas_p2`-style documents degrade to review stubs.** Honest, flagged, zero cost - but the
   user gets 16 review stubs and no usable cards for that paper today.
5. **The legacy cloud-context flag is test-only by construction, and was disclosed here.** It
   exists only under `#[cfg(test)]`, so production binaries cannot compile a bypass.
6. The pre-existing `fix_mark_allocation` tag-dropping defect above inflates review flags.
7. No frontend consumer was changed or tested for the new `ImportReport` fields.
8. Phase-4 items not attempted: full corpus certification across both corpus directories,
   `node scripts/verify-physics24-render.mjs`, the KaTeX crash sweep, and the per-paper report.

## Decisions for Astra

* Accept revision 2 and start phase 2 (gate healing: terminal endings, delimiter balancing,
  final-question boundaries) - the sampled corpus needs it most.
* Decide whether `fix_mark_allocation`'s single-tag drop is folded into phase 2 (recommended;
  evidence is in this report) or left to phase 3.
* Confirm that over-blocking mixed scans (risk 2) is the intended trade.

## Changed paths

* `src-tauri/src/doc_map.rs` - `TextLayerClass` struct, threshold-free
  `classify_text_layer` / `page_has_extracted_text`, `scanned_only`, `is_scanned_only`,
  `build_text_only_map`, `finalize_spans`, updated/new tests.
* `src-tauri/src/pipeline.rs` - digital policy helpers, `chat_with_permit` cloud refusal,
  text-only map / structure-pass / fallback gating, digital branches in `extract_span` and
  `extract_same_page_batch` (strict-per-span retry), `stitch_question_items` with
  `expected_parent_marks` and `StitchedQuestion`, aggregate single-span stitching,
  `build_recovered_question`, `ImportReport::recovered`/`text_layer`, `force_scanned_context`
  (test only), `run_cloud_pipeline` helper, updated/new tests.
* `src-tauri/src/deterministic.rs` - `try_local_recovery`, `MIN_RECOVERED_CHARS`, digital-aware
  `try_deterministic_batch`.
* `src-tauri/src/commands.rs` - `QuestionClient`, `import_requires_provider`, local-only route
  before provider resolution, quota debit gated on a resolved route, new tests.
* `src-tauri/src/llm.rs` - `RefusingLlm`, `body_has_images`, test.
* `src-tauri/src/bin/e2e_import.rs` - `--offline`, honest certification fields, exit on
  attempts or tokens.
* `scripts/verify-zero-cost-offline.mjs` - strict certification: child status/signal, fresh
  artifact, zero attempts, zero tokens, $0.0000.
* `scripts/verify-zero-cost-offline.test.mjs` - pure-validator regressions (`node --test`).
* `src-tauri/fixtures/digital_zero_cost/{cover,blank_page,end_only_page,weak_map_page_1..3,failed_gate_page_1..3,batch_page}.txt`.
* `output/zero-cost-orchestration/` - logs and refreshed certification JSONs.
* `docs/agent-work/zero-cost-orchestration/phase-1-report.md` - this report.

No dependency, credential, production-DB, commit, staging, push, deploy, or model-route change
was made. `PLAN.md` and `CHECKPOINT.md` were not edited.
