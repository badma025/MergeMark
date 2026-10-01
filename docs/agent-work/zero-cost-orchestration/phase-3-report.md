# Phase 3 report — local transcription generalization

> **Current root status (2026-09-28): PAUSED at user request. Phase 3 is incomplete.** The latest boundary/validator edits are preserved but unaccepted. `run-20260928c1b` is the last complete diagnostic run; `c1c` was interrupted and is not certification. See `RECOVERY-PLAN.md` for the bounded next bundle and mandatory 90-minute review. Status labels and results below describe historical submissions, not current overall acceptance.

Author: native Flash implementation worker (`astra_flash_builder` consumer role).
Brief: `docs/agent-work/zero-cost-orchestration/PHASE-3.md`. Contracts: `PLAN.md` +
`docs/ZERO_COST_ORCHESTRATION_PROMPT.md`.
STATUS: **ready_for_review**.

> Revision 3 (PHASE-3-REVIEW.md corrections) — see the section below. STATUS is
> now **checkpoint**: bounded corruption fixes are implemented and tested, the
> local layout-evidence architecture is added, and the remaining matrix / margin
> / nuclide / equation reconstruction plus full-corpus calibration is the
> outstanding scope. This is not accepted work.

Workspace: `C:/Users/alimb/Documents/trae_projects/MergeMark`.
Accepted snapshots `output/zero-cost-orchestration/accepted-phase-1` and
`accepted-phase-2` are untouched. All prior dirty work preserved; nothing staged,
committed, pushed, deployed, or run with cloud calls. No new dependencies.

> Revision 4 (PHASE-3C1-ACCOUNTING): the calibration accounting dependency is
> corrected — stale-artifact reading is gone, ingestion now runs fresh per paper
> with child-exit/artifact-freshness/policy validation, an independent
> source-ID/hash fixture is added, malformed CLI args are rejected, and rejection
> tests pass. The fresh 22-paper baseline is recorded below. C1 boundary/mark
> repairs and overall Phase 3 remain outstanding.
## Corrections completed in this pass

### Layout foundation A (phase-3 sub-bundle) — ready_for_review

Delivered the live local-evidence foundation plus real source-evidence repairs:

1. **Character-level evidence.** `pdf_render::LocalChar { index, ch, bbox, font_size }`
   plus `PageLayoutEvidence { text, chars, runs, rules, fully_mapped }`. `text` is
   the character-indexed page string (`chars[i].index == i`), built from pdfium's
   `PdfPageTextChar::{index, unicode_char, tight_bounds, scaled_font_size}` — an
   explicit source-index → text-offset map (index == offset). An unmapped Unicode
   value or a failed `tight_bounds` keeps the char as `U+FFFD` and sets
   `fully_mapped = false`; `repair_page_text_with_evidence` then returns the
   **original** page text, so unknown source is never silently erased.
2. **Per-import immutable cache.** `PipelineConfig.layout_evidence:
   Option<Arc<ImportEvidence>>`; `pdf_render::load_import_evidence(path)` builds it
   once. Wired into production (`commands.rs` question import) and the e2e harness
   (`src-tauri/src/bin/e2e_import.rs`). No global path-only cache; text-only
   callers keep `None`. Consumed by `deterministic::repaired_span_texts`, which
   both the strict and recovery paths use before carving.
3. **Source-evidence repairs (bounded, no hardcoded equations).**
   * Q14: `stacked_fraction_at` recognizes a numerator digit with its
     denominator bounded to ~0.3–2.5 glyph heights directly below and the
     variable within ~3 glyph widths to the right. Physics '21 Q14 yields the
     four distinct source options `9/2 F`, `9/4 F`, `3/2 F`, `3/4 F` — the
     previously dropped denominators are restored.
   * Q5: `reconstruct_equations` works from a thin fraction-bar **rule** and the
     glyphs around it (symbols, exponents, subscripts, operators are all read
     from the real glyphs; no fixed equation string). The replacement targets the
     exact source character span of the local cluster, and is rejected when any
     non-whitespace glyph in that span is not consumed exactly once — so
     surrounding prose and multiple equations are preserved. Physics '21 Q5c becomes
     `$E_{k}=\frac{e^{2}\text{B}^{2}\text{R}^{2}}{2m_{p}}$`, matching the source
     render (`output/zero-cost-orchestration/q5-page18.png`).

   Source-loss invariants (foundation review): `render_group` returns success
   only when every glyph is rendered exactly once (unassigned or ambiguously
   attached small glyphs reject); `reconstruct_equations` requires every
   non-whitespace glyph in the replacement span to be consumed exactly once (no
   bbox-only exception); overlapping replacement intervals are dropped before
   descending splices; and equations + stacked fractions compose on one page.
   `repair_page_text_with_evidence` verifies the source-index ↔ text-offset map
   (`chars[i].index == i`, full coverage) and otherwise returns the original.

Fresh offline results after layout foundation A (zero cloud, zero tokens, $0.0000):

| Paper | Strict | Recoveries | Quarantine | Log |
| --- | --- | --- | --- | --- |
| physics '21 | **31** | 0 | 0 | `physics21-p3fb.log` |
| physics '24 | 32 | 0 | 0 | `physics24-p3fb.log` |
| madas paper 2 | 0 | 14 | 2 | `madas-p3fa.log` |
| core pure 1 '21 | 2 | 0 | 7 | `corepure-p3fa.log` |

Regressions (targeted): `pdf_render::tests` synthetic set 4 passed
(`p3fb-synth2.log`), including the stacked-fraction negatives, the local
equation-cluster negatives, and
`replacement_engine_rejects_ambiguity_overlap_and_index_gaps`
(ambiguous superscript, unconsumed operator, overlapping bars,
equation+stacked-fraction composition, gapped-index fallback). The pdfium
fixture test passes alone (`p3fb-fixture.log`, `--test-threads=1`); the full
`pdf_render::tests` run hits the pre-existing parallel-pdfium heap flakiness, so
it is run serialized. `deterministic::tests` + `sanitize::tests` 84 passed /
1 ignored (`p3fa-detsan.log`).

### Phase 3B2 — matrices and remaining source maths

* **Local vector/path evidence.** `pdf_render::PathEvidence` + `PageLayoutEvidence.paths`,
  built from `PdfPagePathObject::segments().transform(matrix)`. Object/form
  matrices are composed with `PdfMatrix::multiply`; the order is proven
  (`pdf_matrix_multiply_applies_child_then_parent`: `multiply(self,other)`
  applies self first, so the walker applies the path matrix before its container
  transform), page rotation + normalization applied once. Move/Line/Bezier and
  close are preserved; nested forms are walked depth-bounded; a failed
  object/form matrix or non-finite coordinate skips the object (no silent
  parent fallback).
* **Shape-based radical recognition.** `radical_overbar` requires an open,
  single-subpath, curve-free stroke with a long near-horizontal bar and a dip
  below it; closed polygons, disconnected MoveTo points and graph/line shapes are
  rejected (`radical_requires_check_shape_and_overbar`).
* **Matrix consumer with integrity checks.** `reconstruct_matrices` clusters
  bracket pieces by x AND vertical contiguity (two matrices sharing x no longer
  merge — `two_vertically_separate_matrices_do_not_merge`), pairs same-height
  left/right groups, groups entries into aligned rows/columns, and rebuilds each
  cell from captured glyphs + radical paths. `render_matrix_cell` consumes the
  radicand by the radical's overbar extent and preserves following operators/
  letters (exact consumption; unsupported symbols reject the candidate). The
  replacement interval is accepted only when every non-whitespace source glyph
  is consumed exactly once (same invariant as equations). Core Pure 1 '21 Q1
  reconstructs `\begin{pmatrix}-4 & -4\sqrt{3} \\ 4\sqrt{3} & -4\end{pmatrix}`
  and it appears in-card in math; Core Pure is **2 strict / 1 recovery / 6
  quarantine** (Q1 flagged for unrelated subpart/barcode issues, matrix intact).
* **Inline source scripts (physics '21 Q5).** `unrepresented_inline_superscript`
  detects the `E_k^{1.5}` run (0.85× max threshold). A strictly bounded
  `inline_script_edits` attach was added (adjacent single alphabetic subscript +
  short numeric superscript, exact consumption). It does not yet fire for Q5
  (page 19 is not `fully_mapped`, so the evidence repair falls back), so Q5
  remains an asserted flagged recovery (`source_superscript_unrecovered`) and
  physics '21 is **30 strict / 1 recovery**.
* **Madas Q8/Q9** remain quarantined (no viable local candidate); the matrix
  consumer needs its bracket/region constraints validated on that page.

### Phase 3C1 — trustworthy accounting (accepted-dependency candidate)

**Root-found defect fixed:** the previous calibration ran only `--inspect-source`
and then read whatever `*_offline_certification.json` already existed in a fixed
directory, so a stale or hand-written artifact could be reported as a fresh policy
pass. That path is removed (`scripts/calibrate-zero-cost.ps1` is now a thin shim).

**Fresh runner.** `scripts/run-zero-cost-calibration.mjs` +
`scripts/lib/zero-cost-accounting.mjs` launch the REAL offline ingestion for the 22
named papers via `cargo run --bin e2e_import -- <pdf> <name> --offline --subject
<...> --module <...>` into a unique per-run/per-paper directory
(`output/.../c-calibration/run-<stamp>/<slug>-<stamp>/<slug>/`). Any earlier
artifact at the exact path is deleted first; the child exit/signal is recorded;
and the row requires freshly written `*_offline_certification.json`, `*_cards.md`
and `*_cards.json` from that run (mtime ≥ run start − 2s slack).

Subject/module are declared per paper (`PAPERS`), so Jan 2025 → Mathematics /
Decision Mathematics D1 and Nov 2024 → Mathematics / GCSE Mathematics, and no
paper is forced through the Physics taxonomy.
Per row it fails on: missing source; missing/malformed/stale artifact; non-zero
child exit or signal; `mode != "offline"`; `cloudAttempts != 0`;
`imageAttempts != 0`; `textOnlyAttempts != 0`; `zeroAttemptPolicyPassed != true`;
`tokensZero != true`; non-zero prompt/completion tokens; `costUsd != "0.0000"`.
Unknown counters are stored as `null` and fail — never synthesised as 0. Any row
failure makes the runner exit non-zero; failures are listed and are never dropped
from the denominator.

**Independent source IDs.** `scripts/lib/source-ids.mjs` +
`scripts/generate-source-ids.mjs` derive expected question IDs from the raw
`--inspect-source` page dumps, not from the parser: the AQA "For Examiner's Use"
front table (physics '21/'24, tp03, CS2 '22/'23/'24), the paper's self-declared
"There are N questions" (core pure '21/'22/'23, fp2 '24, FM1 '21/'22/'23, AEA,
Jan 2025, TP01, Naiker, Madas), and heading scans for Nov 2024. The frozen fixture
`scripts/fixtures/source-expected-ids.json` is tied to each PDF's SHA256 (a hash
mismatch invalidates the expectation) and is never read by the production parser.
aqafm24/tp04/tp07 headings are split across text runs; their IDs were confirmed by
direct page review and are marked MEDIUM confidence.

**Fresh baseline — run `20260928fresh`, exit 0, all 22 rows OK, cloud=0, image=0,
tokens=0, cost $0.0000.** Historical weighted strict **140/244**; test weighted
strict **48/70** (overall 188/314). Per-paper counts:
`output/.../c-calibration/run-20260928fresh/summary-20260928fresh.txt`; the
machine-readable manifest with source SHA256, run identity, settings and child
exits is `manifest-20260928fresh.json`.

**Honest ID discrepancies retained (18 MATCH / 4 MISMATCH):**
* `jan2025` — source declares 8 questions; the parser emits 12 cards
  (missing 5,6; extra 10,12,13,16,18,19): over-split confirmed.
* `aqafm24` — parser cards lack 2,6,14,19,21,22 relative to source.
* `tp04_cie_maths` — parser cards lack 4 and 11.
* `tp07_legacy_maths` — parser cards lack 2 and 3.
These are reported, not hidden; resolving them is the C1 boundary/mark sub-bundle.

**Rejection tests (14/14 pass, `node --test` exit 0).**
`scripts/tests/zero-cost-accounting.test.mjs` drives an injectable fake child to
prove acceptance of a fresh clean run, that a rejected row drives the non-zero runner exit, and rejection of: stale output; a
pre-seeded artifact with a silent child; non-zero child with plausible artifacts;
signal kill; absent artifact; malformed artifact; attempted cloud request at zero
tokens/$0; unknown counters; missing source; fixture hash mismatch. The runner validates through the canonical `scripts/verify-zero-cost-offline.mjs` freshness/policy helpers, extended with the image/text attempt and `tokensZero` gates.

**Malformed CLI args (req 5).** `src-tauri/src/bin/e2e_import.rs` now rejects a
required-value flag whose next token is another option: `--subject --offline`
exits 1 with `--subject requires a value (got '--offline')`; `--out --subject` and
an end-of-argv `--module` also exit 1 (verified against the built binary).

Superscript detector repair (earlier pass) is retained: CS2 '24 `04.2`/`10.1` no
longer false-flag; physics '21 Q5 `1.5` stays gated (1 recovery).

**Evidence commands (this run):**
* `node scripts/generate-source-ids.mjs` → exit 0; wrote
  `scripts/fixtures/source-expected-ids.json` (22/22 papers: 18 HIGH, 4 MEDIUM).
* `node --test scripts/tests/zero-cost-accounting.test.mjs` → exit 0; 14/14 pass.
* `node --test scripts/verify-zero-cost-offline.test.mjs` → exit 0; 7/7 pass.
* `node scripts/run-zero-cost-calibration.mjs --stamp 20260928fresh` → exit 0;
  22/22 rows OK; log `output/zero-cost-orchestration/c1-accounting-fresh.log`.
* `e2e_import "past papers for mergemark/physics '21.pdf" physics21 --subject
  --offline` → exit 1, `--subject requires a value (got '--offline')`;
  `--out --subject Physics` → exit 1; trailing `--module` → exit 1.
Remaining C1 boundary/mark repairs (CS boxed parts/code, Nov 2024 checksum, Jan
2025 / AQAFM boundaries, Madas Q8/Q9 no-viable-carve) and C2 symbol/font/matrix
gaps are not implemented yet.
### Phase 3C1B — boundary / sub-part / heading repairs

General, source-driven fixes in `src-tauri/src/doc_map.rs` and `validate.rs`.
No paper-keyed ranges; all effects come from layout/sequence/declared-count
evidence.

1. **Jan 2025 D1 (12 fabricated card IDs → 8 true parents).** Root cause was
   two-fold: (a) the answer booklet repeats every "Total for Question N is M
   marks" footer, so the footer sequence ran 1..8,1..8 and failed the monotonic
   check, discarding *all* footers and forcing page-granular heading carving;
   (b) the answer-book printed page numbers ("10", "12", …) were accepted as
   bare question headings, while the real "5."/"6." headings were missed.
   Fixes: keep only the FIRST footer per question (restores 1..8), reject AQA
   `*NN*` page markers, and resolve *isolated* bare numbers against the strong
   heading set — a lone number is a heading only when it fills a gap in that set
   (`resolve_lone_headings`). Jan 2025 now yields exactly Q1..Q8 (marks
   9,13,5,6,7,13,14,8 = 75), `idCoverage=MATCH`.
2. **CIE 9709/11 M/J 2014 (missing 2,3 → Q1..Q12).** The CIE layout prints a
   question number alone above a diagram; those are genuine gap-fillers and are
   now accepted, while the printed page numbers (rejected by the index guard and
   the gap rule) are not. `tp07_legacy_maths` now yields Q1..Q12, `MATCH`.
3. **CIE 9709/11 M/J 2022.** Render-verified (pdftoppm): the paper has exactly
   **10** questions — page 19 ends at Q10(d) and page 20 is the "Additional
   Page"; the "11" is the paper code, not a question. The fixture override was
   corrected 11→10; Q7 is now recovered (the real heading was rejected because
   the diagram label "A" on the next line was misread as the unit "ampere" — the
   quantity check is now same-line only). Q4's span is still missing (see below).
4. **AQA GCSE Further Maths 2024 (17 → 19 of 23).** Fixes: reject `*NN*` page
   markers (they were strong headings), restrict the marks-tag proximity guard to
   the same line (a previous question's "[2 marks]" on a one-question page was
   suppressing the next heading), restrict unit detection to the same line, and
   route bare-rejected headings (e.g. "5 y = …", "22 f(x)") into the gap-filler
   resolver. Q2/Q6/Q14/Q19/Q21 now appear; Q5 and Q22 remain missing.
5. **CS 2022/23/24 sub-part gate.** `validate::card_structure_errors` treated
   `(i)` as lettered sub-part 9 (the `[a-i]` class), so every CS card with a
   Roman-numeral sub-sub-part failed the sequential gate. Restricted to `[a-h]`;
   CS papers now show no false sub-part failures, and a genuinely missing lettered
   part ("['b','c','d'] must be ['a','b','c']") is still flagged (cs223 Q6).

Regression tests added in `doc_map::tests`
(`lone_headings_only_fill_strong_set_gaps`, `aqa_page_marker_is_not_a_heading`,
`previous_line_marks_tag_does_not_suppress_next_heading`,
`duplicate_answer_book_footers_keep_monotone_sequence`; `doc_map::` 20 passed)
and `validate::tests::roman_numeral_subsubparts_are_not_lettered_parts`.

**Fresh 22-paper run `20260928c1b` (exit 0, all rows OK, cloud=0, image=0,
tokens=0, $0.0000):** 20/22 ID-MATCH. Weighted strict historical **141/244**,
test **49/69**. No regressions in physics '21/'24, core pure, FP2, FM1, CS2, AEA,
Nov 2024, Naiker, Madas, TP01, TP03. Summary:
`output/.../c-calibration/run-20260928c1b/summary-20260928c1b.txt`.

**Still open (reported, not hidden):**
* `aqafm24` — Q5 ("5 y = …") and Q22 ("22 f(x)") not yet carved
  (`idDiscrepancies.missing=["5","22"]`).
* `tp04_cie_maths` — Q4 span missing (`missing=["4"]`); its heading is detected
  but no span is carved.
* `nov2024` — 18 local recoveries are dominated by `unmapped_symbol_font_glyphs`
  (C2 symbol/font work); the marks_checksum cases (e.g. Q4/Q5 norm≠expected) are
  not yet reconciled.
* `madas_p2` — Q8/Q9 remain quarantined (no viable local carve).
These are the C2 / next sub-bundle scope.
### Phase 3C — corpus manifest + baseline calibration

Manifest per `docs/ZERO_COST_ORCHESTRATION_PROMPT.md`; harness extended with
backward-compatible `--subject`/`--module` so CS/maths runs are not forced through
the Physics taxonomy (`scripts/calibrate-zero-cost.ps1`, raw run
`output/zero-cost-orchestration/c-run.log`, artifacts in
`output/zero-cost-orchestration/c-calibration/`). Every run: offline refusing
client, `cloud_attempts=0`, `prompt_tokens=0`, `completion_tokens=0`, `$0.0000`.

Historical `past papers for mergemark/` (18 available named files):

| Paper | Expected | Extracted | Strict | Recov | Quar | Placeholder |
| --- | --- | --- | --- | --- | --- | --- |
| physics '21 | 31 | 31 | 30 | 1 | 0 | 0 |
| physics '24 | 32 | 32 | 31 | 1 | 0 | 0 |
| core pure 1 '21 | 9 | 9 | 2 | 1 | 6 | 6 |
| core pure 1 '22 | 10 | 10 | 5 | 2 | 3 | 3 |
| core pure 1 '23 | 8 | 8 | 2 | 1 | 5 | 5 |
| fp2 '24 | 8 | 8 | 4 | 2 | 2 | 2 |
| further mechanics 1 '21 | 7 | 7 | 4 | 1 | 2 | 2 |
| further mechanics 1 '22 | 8 | 8 | 6 | 2 | 0 | 0 |
| further mechanics 1 '23 | 7 | 7 | 6 | 1 | 0 | 0 |
| aqa gcse further maths '24 | 17 | 17 | 8 | 5 | 4 | 4 |
| aea2024 | 7 | 7 | 1 | 3 | 3 | 3 |
| computer science 2 '22 | 13 | 13 | 5 | 8 | 0 | 0 |
| computer science 2 '23 | 12 | 12 | 3 | 6 | 3 | 3 |
| computer science 2 '24 | 11 | 11 | 1 | 10 | 0 | 0 |
| January 2025 QP | 12 | 12 | 4 | 2 | 6 | 6 |
| Nov 2024 QP | 23 | 23 | 2 | 19 | 2 | 2 |
| naikermaths m16 pure | 11 | 11 | 10 | 1 | 0 | 0 |
| madas paper 2 | 16 | 16 | 13 | 1 | 2 | 2 |

Historical weighted strict = **137 / 242 = 56.6 %** — below the 95 % target.
Test-papers cohort (`test_papers/`, 4 QP files): tp01 11/16, tp03 29/31,
tp04 4/9, tp07 2/10 → weighted **46 / 66 = 69.7 %**.

Unlisted-but-present files (inventory note, not in the prompt manifest):
`m3_centre_of_mass.pdf`, `mp1_a.pdf`, `mp1_t.pdf`, `mp1_t_solutions.pdf`,
`old ahh stats paper.pdf`.

Largest strict gaps (general defects, no paper/question keys): Edexcel/Core Pure
and CIE/legacy maths PUA matrices and symbol fonts (corepure '21/'22/'23, tp04,
tp07), AQA GCSE FM and January/Nov 2024 boundary+MCQ+table handling, and CS
code/table questions (cs2 '23/'24 recoveries). Unknown/ambiguous items remain
flagged; nothing was converted to confident content to raise the number.

### Phase 3B2 integrity completion (PHASE-3B2-INTEGRITY.md)

Requirement → real test result (all `pdf_render::tests`, 19 passed serialized,
`p3b3-all2.log`):

1. **Exact source consumption for math replacements (incl. inline radical).**
   `inline_radical_keeps_operators_and_rejects_unsupported` — `2+3` under the
   overbar → `$\sqrt{2+3}$` (surrounding text preserved), unsupported glyph →
   unchanged; `inline_radical_requires_glyphs_under_the_overbar` — out-of-bar and
   out-of-band glyphs produce no edit.
2. **Consistent column grid / signed rectangular / column vectors.**
   `matrix_grid_supports_rectangular_and_column` asserts the full 2×3 signed
   LaTeX and a 3×1 column; `two_vertically_separate_matrices_do_not_merge`
   asserts exactly two independently correct matrices with the separator glyph
   preserved (no vacuous `len<=2`).
3. **Radical path identity + overbar/vertical radicand.** `reconstruct_matrices`
   records path ids per cell and rejects a reused path;
   `inline_radical_requires_glyphs_under_the_overbar` covers overbar-extent and
   vertical-band association; `radical_requires_check_shape_and_overbar` covers
   the closed-rectangle / line negatives.
4. **Inline script local vertical bounds.**
   `inline_script_requires_local_vertical_bounds` — real Q5 cluster attaches
   `$E_{k}^{1.5}$`; later-line same-x small text and a baseline small number do
   not attach. Bounds use the local base size and glyph-bottom raised/lowered
   geometry, not the whole-page max.
5. **Root-decision eligibility.**
   `repair_eligibility_separates_geometry_from_mapping` — unrelated bad bounds
   still allow the local fraction repair; bad bounds inside the candidate, an
   unknown Unicode page, and an index gap all leave the text unchanged. The
   geometry check tests all four bbox components for finiteness.
6. **Transform mapping.** `map_point_applies_page_rotation` exercises the
   collector's mapping under page rotation; `pdf_matrix_multiply_applies_child_then_parent`
   proves the composition order the walker relies on. The real Core Pure Q1
   fixture still passes.

Fresh sample imports (zero cloud/tokens/$0.0000): Core Pure 1 '21 2/1/6 with the
matrix in-card (`corepure-p3b3.log`); physics '21 30/1/0 (`physics21-p3b3.log`, Q5
honest recovery). Madas boundary recovery moves to C.

### Phase 3B2 revision — root decision + Madas inspection

* **Unicode/index vs geometry separated (root decision).** `pdf_layout_evidence`
  now leaves `fully_mapped` about Unicode presence + contiguous indices only; a
  per-glyph bounds error no longer flips it. `repair_page_text_with_evidence`
  rejects an individual edit whose source span touches a non-whitespace glyph
  with missing/non-finite/zero geometry, while unknown Unicode or index gaps
  still trigger the whole-page original-text fallback.
* **Madas Q8/Q9 inspected.** The matrix-region evidence is a **vector radical
  path** (11-segment check + overbar, overbar x≈0.5667–0.6127) — not symbol-font
  brackets. Added `inline_radical_edits` (radicand taken under the overbar).
  Q8/Q9 are still quarantined at the carve ("no viable local candidate"), so the
  next step is their boundary/heading evidence, not the consumer.
* **Physics '21 Q5** still cannot repair: page 19 carries an unknown-Unicode
  glyph, so the whole-page fallback applies per the root rule. Q5 remains an
  asserted `source_superscript_unrecovered` recovery.

### Phase 3B1 — MARK / BOUNDARY / FURNITURE

Delivered:

* **physics '21 Q5(d) recovered structurally.** AQA prints the part label as a
  boxed `0 5 . 4` (also `0 5 box . 2`). `deterministic::recover_aqa_part_labels`
  rebuilds `(d)` from the span's question number and the part index BEFORE
  normalisation — no paper/question literal. Q5 now renders all four subparts
  with source allocations: (a)2, (b)1, (c)3, (d)4 = **10 marks** (`10m`), and it
  is strict-clean. Source comparison (`q5d-page19.png`) confirms (d) is
  `[4 marks]`; the review's "9" was a misread of the source.
* **Global furniture deletion replaced with position/narrow evidence.**
  * The blanket x ≥ 0.90 deletion is gone. `pdf_render::margin_allocations`
    finds actual right-margin `(N)` allocations that are the rightmost glyph on
    their line, and deletes only those; `answer_prompt_edits` removes a printed
    label (`cyclotron =`, `cost =`) only when its y matches a long answer rule.
  * A legitimate right-edge table cell / equation containing `cost = 10`,
    fenced code containing it or `left right`, and a `left right` table header
    all survive (no global string deletion).
  * `recover_aqa_part_labels` now requires a leading zero pad or a box token and
    whitespace before the dot plus a part-token boundary, so `5.25 kg` and code
    can never become `(b)`.
  * The earlier over-broad regex that broke physics '21 Q13 is gone.
* **Madas right-margin association implemented with source provenance.**
  New `pdf_render::{MarginRecord, MarginModel}`: the model is built ONCE per
  import (`PipelineConfig.margin_model`, a `OnceLock`) from the measured heading
  y of each source `LocalTextRun`. Only **confirmed records** (exact source char
  ranges) drive removal before carving — the old global body regex is deleted, so
  unassigned `(N)`/code/values are preserved. Regions reject ambiguous or missing
  next headings and are bounded by the span's `end_page`; a body mark that
  disagrees with the measured association becomes `mark_allocation_mismatch`.
  `margin_allocations` requires true isolation (large left gap, no operator
  before), so `x=(5)`/table cells are not allocations. `answer_prompt_edits`
  now enforces exact consumption for both the label span and the answer-box
  digits.
  Fresh Madas import: **13 strict / 1 recovery / 2 quarantine**
  (`madas-p3b1g.log`), per-question output vs source margin tokens: Q1=6, Q2=5,
  Q3=10, Q4=8, Q5=7, Q6=12, Q7=8, Q10=12, Q11=15, Q12=14, Q13=12, Q14=10,
  Q15=16, Q16=16 (Q8/Q9 quarantined matrices). **Q12=14 is confirmed against the
  source render** (`madas-page6b.png`): parts a)(2) b)(8) c)(4).
* Fresh offline (zero cloud/tokens/$0.0000): physics '21 **31/31 strict**
  (`physics21-p3b1d.log`).
* Regressions: `aqa_padded_part_labels_become_human_labels`,
  `contextual_furniture_removed_without_touching_legitimate_content`.
* **physics '21 Q5 `E_k^{1.5}` — now actually gated.** Inspecting the source
  char region proved the run IS present: `E`(12.00), `k`(7.98 subscript),
  `1`/`.`/`5`(7.98 superscript) on page 19. The detector threshold was wrong
  (0.7× median); it is now 0.85× max size, so it fires and Q5 is a flagged local
  recovery (`source_superscript_unrecovered`) rather than a strict-clean card.
  physics '21 is **30 strict / 1 recovery**. A text-level inline-script repair
  was prototyped but removed: it corrupted physics '21 Q4
  (`m_{.0}^{2}`), so a bounded script-attach pass remains the honest next step
  before Q5 can return to strict.

Still open in Phase 3B:

* **Matrix reconstruction.** Core Pure 1 '21 Q1's source matrix is
  `M = (−4, −4√3; 4√3, −4)` (`mx-page2.png`). The bracket pieces
  (`U+F8EB/F8EC/F8ED`, `U+F8F6/F8F7/F8F8`) and entries are present, but the
  **radical glyph `√` is absent from the text layer** (only `-4`, `-4`, `3`
  appear), so entries cannot be reconstructed without guessing. This must stay
  a local failure/recovery unless a vector/primitive radical source is exposed.
  Madas Q8/Q9 need the bracket-piece row/column grouping pass.
* **Madas right-margin `(5)`/`(6)` association.** Needs span-level y-region
  integration — implemented (see above); remaining validation is per-question
  ID/mark audit across the paper (some cards still take marks from body stray
  tags, to be tightened in C).

Full named-corpus calibration remains bundle C; final full suite and
certification remain Phase 4.

### Revision 3 — PHASE-3-REVIEW.md corrections

Implemented and tested (Rust `cargo test` in the default sandbox):

* **F1 unsafe fraction guess removed.** `sanitize::recover_stacked_mcq_fractions`
  no longer turns a "number line + short token line" into `\frac{9}{F}`, and the
  same in-option guess inside `tighten_mcq_lists` is gone. The frontend
  `preprocess-mcq.ts::fuseSplitMcqLines` stacked rule is removed too. Physics
  '21 Q14 now drops to a flagged local recovery instead of a false strict card
  (physics '21 offline e2e: strict 30, recovery 1 — see `physics21-p3r.log`).
  Regression `stacked_fraction_is_not_guessed_from_two_lines` covers the
  `9\nF` / `3\nF` case and the `9 N` / `3 F` / prose negatives.
* **F2 table math-wrapping fixed.** `deterministic::wrap_table_math_cells`
  wraps each pipe-row cell independently and only when the cell carries explicit
  math evidence, so pipes are never inside `$...$`. Fresh physics '21 output is
  now `|Cyclotron|B / T|R / m|`, `|Nucleus|Mass / u|`, `|X|1.3|0.38|`
  (`physics21-p3r`). Regression in
  `phase3_table_recovery_and_glued_option_header`.
* **F4 code masking covers the whole frontend chain.** `protectCode` now runs at
  the very start of `preprocessExamMarkdown` (not only inside
  `healLatexDelimiters`), and the sentinel marker is chosen collision-safe
  (U+0001..U+0003/U+0000, repeated, never a marker already in the source). Node
  regression `scripts/verify-phase3-code-opacity.mjs` extended to 14 cases
  (fenced `$$ $$`, placeholder strings, mixed math+code, marker collision,
  idempotence). *This check needs an escalated `node` run — see the checkpoint
  request below.*
* **F5 diagram letters require MCQ context.** `stem_supports_mcq_options`
  requires a single-part question that asks the reader to choose and ends with
  `?`; `bind_detached_option_pairs` still requires a forced letter assignment.
  Multi-part or descriptive figures stay unbound. physics '24 Q26 keeps its
  correct option-image binding. Regression
  `detached_option_figures_bind_only_when_the_letter_is_forced`.
* **F3 local layout-evidence architecture added.** New public evidence model in
  `pdf_render`: `LocalTextRun`, `LocalRule`, `PageLayoutEvidence`,
  `pdf_layout_evidence(path)` (loads the PDF once via the existing pdfium-render
  dependency and reuses the stroke-census collectors) and
  `fraction_bar_between(upper, lower, rules)` — the geometric signature that
  finally distinguishes a stacked fraction from `9F`/`9 F`. Regression
  `fraction_bar_detection_requires_a_bar_between_the_runs`.
* **False-strict gate for flattened formula/nuclide debris.**
  `deterministic::flattened_math_debris` flags a flattened nuclide/reaction row
  (several bare small integers plus an element symbol) or a short variable span
  trailed by uppercase symbol debris. Physics '21 Q5 (`$k p 2$ eBR E`) is now an
  honest local recovery instead of a strict-clean card. Regression
  `flattened_formula_debris_is_gated_not_strict`.
* **Source-supported fusion-equation reconstruction.**
  `sanitize::reconstruct_flattened_nuclide_chain` rebuilds the column-flattened
  three-term chain from the real pdfium print order: physics '21 Q6 is now
  `$^{3}_{2}\text{He} + ^{17}_{8}\text{O} \rightarrow ^{20}_{10}\text{Ne}$`
  (was `3 17 20 He $+ O$ Ne $2 8 10 \rightarrow$`). Regression
  `flattened_fusion_chain_is_reconstructed_from_source_order`.

Fresh offline physics results after these changes:

| Paper | Strict | Recoveries | Quarantine | Log |
| --- | --- | --- | --- | --- |
| physics '21 | 29 | 2 (Q5 flattened symbol debris, Q14 stacked fraction) | 0 | `physics21-p3r4.log` |
| physics '24 | 32 | 0 | 0 | `physics24-p3r.log` |

Verification (this revision): `sanitize::tests` 24 passed; `deterministic::tests`
passed (incl. `flattened_formula_debris_is_gated_not_strict`); `pdf_render::tests::fraction_bar_detection…` passed;
`stacked_fraction…`, `phase3_table…`, `detached_option_figures…`,
`physics24_diagram_mcq_regions…` all passed.

### Outstanding (not done — the reason STATUS is checkpoint)

* **F3 end-to-end consumption (partial).** The Q6 fusion chain is now
  reconstructed from real print order, and physics '21 Q5 (`$k p 2$ eBR E`) is
  gated to recovery rather than claiming strict-clean. Still outstanding:
  reconstruct Q5's Ek formula from glyph evidence, and wire the evidence API
  into the Madas margin association.
* **Q14 evidence finding (validates the review).** `pdf_layout_evidence` on
  physics '21 page 27 shows Q14's option digits and `F` on the same visual line
  with **zero horizontal rules** on the page. There is no fraction-bar evidence,
  so `\frac{9}{F}` was indeed unfounded; the honest Q14 outcome is the current
  flagged recovery. Any future reconstruction must respect the raised-digit /
  baseline geometry (the `9`/`3` sit ~8 pt above `F`) rather than assume a bar.
* **Core Pure / Madas matrix brackets** (`U+F8EB…F8F8`) still flagged, not
  reconstructed.
* **Madas `(5)`/`(6)` margin association** still conservative-flagged.
* **Full named-corpus inventory and calibration** (only 7 papers calibrated;
  target ≥95% honest strict not yet reached).

### Checkpoint request to root

The frontend opacity regression needs esbuild, which reads `node_modules` above
the sandbox and stalls under child escalation. Please run once and return the
result:

```
node scripts/verify-phase3-code-opacity.mjs
```

(from `C:/Users/alimb/Documents/trae_projects/MergeMark`; expected `14/14` once
the outer `protectCode` masking is confirmed.)

### C1. Frontend delimiter machinery is now code-aware (integration finding)

The Rust assembler already copies code verbatim (`validate::line_code_map`,
`fence_closes`, `inline_code_spans`), but the **frontend** preprocessing path was
still code-blind: `stripOrphanedDollars`, `validateAndEnforceDelimiters`,
`balanceMathEnvironments` and `ensureDisplayMathLineBreaks` rewrote `$$$` and `$`
inside fenced blocks and inline code.

* New shared TS code map `lineCodeMap` ([preprocess-exam-markdown.ts:672](src/lib/preprocess-exam-markdown.ts:672))
  mirrors the Rust one: fence blocks tracked by MARKER CHAR + RUN LENGTH, a
  closing fence requires nothing but whitespace after the run, and inline spans
  use CommonMark backtick-RUN rules.
* `ensureDisplayMathLineBreaks` ([preprocess-exam-markdown.ts:760](src/lib/preprocess-exam-markdown.ts:760))
  now walks that map: fenced lines and wholly-inline-code lines never open, close
  or continue display math, and `$$` toggles are counted only in non-code
  segments.
* `healLatexDelimiters` masks fenced blocks and inline spans with opaque control
  sentinels via `protectCode` ([preprocess-exam-markdown.ts:702](src/lib/preprocess-exam-markdown.ts:702),
  applied at [:886](src/lib/preprocess-exam-markdown.ts:886)) for the whole
  delimiter/table pass chain, then restores the exact source text. Newline
  structure is preserved one sentinel per code line.
* Regression: `scripts/verify-phase3-code-opacity.mjs` bundles the real
  `preprocess-math.ts` + `preprocess-exam-markdown.ts` and asserts fenced `$$$`,
  double-backtick `$`, four-backtick/tilde fence mismatch, escaped dollars,
  info-string closing fences, display-break opacity and a real multi-line
  separator. **9/9 pass.**

### C2. Physics '24 Q26 diagram-MCQ options now bind from positional evidence

Q26 prints four charge diagrams under a `A B` / `C D` grid. The text layer is
plain text (no coordinates exposed), so the previous code appended the four
crops after the stem with no option letters and the render verifier failed
("Expected exactly A, B, C, D").

* `DetectedFigure` gains `option_label: Option<String>` ([pdf_render.rs:247](src-tauri/src/pdf_render.rs:247)),
  populated by `capture_option_label` ([stroke_census.rs:649](src-tauri/src/stroke_census.rs:649))
  from a single-character `A..E` `LabelCandidate` text block sitting inside the
  region hull or just above its top edge with real horizontal overlap. This is
  positional source evidence, not a paper-name or question-number rule.
* `bind_detached_option_pairs` ([pipeline.rs:3262](src-tauri/src/pipeline.rs:3262))
  binds detached crops to their printed letters only when the assignment is
  forced: every crop labelled, or the captured letters are exactly the
  contiguous run `A..N` minus exactly one letter with exactly one anonymous
  crop. Two anonymous crops, duplicate letters or a non-contiguous run stay
  unbound.
* For Q26 the text layer carries `B`, `C`, `D` (the top-left `A` glyph is not
  present), so the single anonymous crop is forced to `A` — a unique solution.
  The reconstructed card is `stem`, then `- [MCQ:A..D] ![Diagram](…)`, with the
  `**[1 mark]**` tag moved after option D (`split_trailing_mark_tag`,
  [pipeline.rs:3241](src-tauri/src/pipeline.rs:3241)).
* Regressions: `stroke_census::tests::physics24_diagram_mcq_regions_capture_option_letters`
  asserts ≥3 distinct positional letters on the real fixture;
  `pipeline::tests::detached_option_figures_bind_only_when_the_letter_is_forced`
  covers the forced case, two-anonymous, duplicate, non-contiguous, <3 crops and
  the all-labelled case.

### C3. physics '21 preserved

`physics '21` still extracts **31/31 strict**, `needs_review=false`, zero cloud
attempts after the stroke-census change.

## Verification (targeted; commands, exits, logs)

| Command | Exit | Result | Log |
| --- | --- | --- | --- |
| `node scripts/verify-phase3-code-opacity.mjs` | 0 | 9/9 code-opacity assertions pass | `output/zero-cost-orchestration/p3-code-opacity.log` |
| `cargo test --lib -- bind_detached_option_pairs …` (both new tests) | 0 | 2 passed | `output/zero-cost-orchestration/p3-q26-tests3.log` |
| `cargo test --lib -- pipeline::tests stroke_census::tests deterministic::tests` | 0 | 161 passed, 0 failed, 3 ignored | `output/zero-cost-orchestration/p3-suite-regress.log` |
| `cargo build --bin e2e_import` | 0 | rebuilt harness | `output/zero-cost-orchestration/p3-build-e2e4.log` |
| offline e2e physics '24 | 0 | strict_tier0=32, recoveries 0, quarantine 0, cloud_attempts=0, tokens 0, $0.0000 | `output/zero-cost-orchestration/physics24-phase3c.log` |
| offline e2e physics '21 | 0 | strict_tier0=31, recoveries 0, quarantine 0, cloud_attempts=0 | `output/zero-cost-orchestration/physics21-phase3c.log` |
| `node scripts/verify-physics24-render.mjs <phase3c cards>` | 0 | **complete, passed 32/32, failures []** | inline output |

## Per-paper calibration

Strict = `needs_review=false` Tier-0. Recoveries and quarantine are honest local
outcomes with zero cloud calls. Post-correction re-runs are marked ✔; the rest
are the phase-3 calibration runs **before** the C1/C2 corrections and are
re-certified in phase 4.

| Paper | Questions | Strict Tier-0 | Local recoveries | Quarantined | Source |
| --- | --- | --- | --- | --- | --- |
| physics '21 | 31 | 31 | 0 | 0 | ✔ `physics21-p3fa2.log` (layout foundation A) |
| physics '24 | 32 | 32 | 0 | 0 | ✔ `physics24-p3r.log` |
| AQA physics p1 | 31 | 29 | 2 | 0 | `aqaphys-phase3.log` (pre-revision) |
| Computer Science 2 '22 | 13 | 5 | 8 | 0 | ✔ `cs2-p3r.log` |
| FP2 '24 | 8 | 4 | 2 | 2 | ✔ `fp2-p3r.log` |
| Madas paper 2 | 16 | 0 | 14 | 2 | ✔ `madas-p3r.log` |
| Core Pure 1 '21 | 9 | 2 | 0 | 7 | ✔ `corepure-p3r.log` |
| **Total** | **140** | **103** | 26 | 11 | — |

Strict rate **103/140 = 73.6 %**, below the 95 % target. Layout foundation A
restored physics '21 to an honest 31/31 (Q14 stacked fractions and Q5 formula
reconstructed from glyph evidence). The remaining gap is owned by the matrix /
margin / contextual-debris / full-corpus items below; nothing was relabelled.

## Remaining phase-3 items (concrete, source-evidenced)

1. **Two-column plain-letter MCQ** — AQA/Edexcel answer keys still mis-form when
   the printed `x / y` column headers fuse with the option letters. Carver work
   (`deterministic::recover_markdown_data_tables`, `split_glued_header_option_run`)
   and the synthetic regressions are in place; live cases remain.
2. **Stacked/glued nuclides** — physics '21 Q27 plaintext `10 … D 2`; local
   reconstruction exists but not all decay chains are covered.
3. **Markdown pipe tables** — physics '21 Q5 (`Cyclotron B / T R / m`),
   Q6 (`Nucleus Mass / u`), Q25 (flattened unit header); the sanitizer repairs
   the flattened header (`repair_flattened_layout_artifacts`) but remaining
   rows/columns are not yet strict.
4. **Symbol-font (PUA) glyphs** — Edexcel Core Pure 1 '21 pages 2/6/14/18/24/32
   (`U+F8EB…F8F8` brackets) and Madas Q8/Q9. Known bracket glyphs alone do not
   establish matrix shape; these stay flagged until row/column layout is
   reconstructed.
5. **Madas margin association** — the phase-2 conservative
   `ambiguous_mark_allocation` guard is preserved; margin `(5)`/`(6)` labels and
   figure-label contamination keep cards in local recovery rather than claiming
   a confident total.
6. **Contextual debris** — margin barcodes (`*05*`), answer-area labels
   (`left right`, `= J 9`) and orphan units in their furniture context.

### Capability blocker for item 4 (exact)

The text layer is extracted as plain strings (`pdf_page_texts` returns
`Vec<String>`; `PageInput.text: String`). There is no exposed per-glyph
(char, bbox, font) stream outside `stroke_census::collect_text_segments`, which
is internal to figure detection. Reconstructing Edexcel matrix shape therefore
needs a glyph/font-position evidence path surfaced to the assembler; without it,
the correct behaviour is the current local review flag, not a guessed matrix.

## Risks and limits

1. The full Cargo suite and full corpus certification are phase 4 by contract.
   Only the affected suites and two papers were re-run here.
2. AQA/CS2/FP2/Madas/Core Pure counts above predate the C1/C2 corrections; the
   corrections only add option binding when positional letter evidence is
   forced, so a regression is unlikely but not proven until phase 4.
3. The Q26 forced-letter rule depends on the missing letter having exactly one
   possible home; papers that print an incomplete or duplicated letter grid stay
   unbound by design.

## Changed paths (this pass)

* `src/lib/preprocess-exam-markdown.ts` — `lineCodeMap`, `protectCode`, code-aware
  `ensureDisplayMathLineBreaks`, masked `healLatexDelimiters`.
* `src-tauri/src/pdf_render.rs` — `DetectedFigure::option_label`.
* `src-tauri/src/stroke_census.rs` — `capture_option_label`, `OPTION_LABEL_SNAP_PT`,
  emission + fixture regression.
* `src-tauri/src/pipeline.rs` — `bind_detached_option_pairs`,
  `split_trailing_mark_tag`, detached-image MCQ assembly, unit regression.
* `scripts/verify-phase3-code-opacity.mjs` — new Node regression.
* `output/zero-cost-orchestration/*` — logs and regenerated phase3c card sets.

Earlier phase-3 implementation from this session (sanitizer table/typography
repairs, deterministic table/option recovery, isotope reconstruction) is
unchanged and preserved.
