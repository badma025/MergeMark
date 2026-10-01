# Phase 2 report - local gate healing and boundary agreement (revision 2)

Author: native Flash implementation worker (`astra_flash_builder` consumer role).
Brief: `docs/agent-work/zero-cost-orchestration/PHASE-2.md`. Contracts: `PLAN.md` +
`docs/ZERO_COST_ORCHESTRATION_PROMPT.md`.
STATUS: **ready_for_review**.

Workspace: `C:/Users/alimb/Documents/trae_projects/MergeMark`.
Accepted phase-1 snapshot `output/zero-cost-orchestration/accepted-phase-1` was read and is
untouched. All prior dirty work preserved; nothing staged, committed, pushed, deployed, or run
with cloud calls.

Revision 2 closes the four batched-review findings (true code-aware delimiter scanning,
first-page/one-page support, conservative margin-allocation ambiguity, ignored corpus
diagnostic) with targeted tests only. Implementation and test evidence from revision 1 were
preserved; nothing was re-run merely because the host session stalled.

## Revision-2 corrections

### C1. Delimiter machinery is genuinely code-aware

* `validate::inline_code_spans` ([validate.rs:675](src-tauri/src/validate.rs:675)) implements
  CommonMark backtick-RUN rules: a span opens with a run of N backticks and closes at the next
  run of EXACTLY N backticks, so ``a ` b`` and ``$`` are opaque.
* `validate::fence_marker` ([validate.rs:718](src-tauri/src/validate.rs:718)) +
  `line_code_map` ([validate.rs:750](src-tauri/src/validate.rs:750)) track fenced blocks by
  MARKER CHAR and RUN LENGTH: a four-backtick fence is not closed by a three-backtick line, and
  a tilde fence is not closed by backticks (or vice versa).
* `normalize_dollar_runs_outside_code` ([validate.rs:844](src-tauri/src/validate.rs:844))
  replaces the old whole-content `normalize_dollar_runs`: `$$$` -> `$$` only outside fenced
  lines and inline code. The healer
  ([`balance_math_delimiters`](src-tauri/src/validate.rs:942)) and the validator
  ([`math_delimiter_balance_errors`](src-tauri/src/validate.rs:906)) both walk the shared code
  map, copy code verbatim, and scan only the non-code segments
  ([`push_balanced_segment`](src-tauri/src/validate.rs:869)).
* Regressions: `delimiter_machinery_treats_code_as_opaque` covers a fenced literal `$$$`, a
  double-backtick span containing a single dollar and a backtick, a four-backtick fence
  containing a three-backtick line and dollars, a mismatched tilde/backtick fence, escaped
  dollars and idempotence. The earlier `delimiter_balancer_preserves_code_and_is_idempotent`
  case still passes.

### C2. First-page / one-page papers are supported

* `doc_map::has_strong_first_page_question` ([doc_map.rs:392](src-tauri/src/doc_map.rs:392))
  requires a heading followed by at least 30 characters of same-line content, so cover
  furniture like Edexcel's `1 hour 45 minutes 9PH0/02` is not question evidence.
* `doc_map::is_front_matter_page` ([doc_map.rs:412](src-tauri/src/doc_map.rs:412)) replaces the
  unconditional `page == 0` skip ([doc_map.rs:492](src-tauri/src/doc_map.rs:492)): a page with a
  real question is content; a rubric-dominated cover (three or more cover markers, or any
  explicit rubric heading) is still skipped, and
  `append_text_only_short_answer_spans` no longer carries an unconditional page-0 guard.
* Regressions: `doc_map::tests::one_page_paper_keeps_question_one_and_real_covers_are_still_skipped`
  (one-page paper yields a Q1 span; a rubric-only cover is still NonQuestion and adds no spans)
  and the end-to-end `pipeline::tests::digital_one_page_paper_keeps_first_page_question`
  (one page, no synthetic cover: 0 cloud attempts, 1 local Tier-0 card, exact source content
  `enlargement ... scale factor`, 4 marks, `needs_review=false`).
* The real-corpus placement tests still pass (`doc_map::tests` 16 passed, including the
  AQA/Edexcel fixture integrity tests), so genuine covers are unaffected.

### C3. Detached margin allocations are ambiguous, not confidently strict

* New structural guard in `deterministic::check_gates`
  ([deterministic.rs:1754](src-tauri/src/deterministic.rs:1754)): a FOOTERLESS carve that
  carries more than one mark allocation while having no sub-part labels returns
  `ambiguous_mark_allocation`. This is source-layout evidence only - no paper name or question
  number is special-cased. The content is still retained by the recovery path, flagged for
  review, and the failed gate is recorded on the card and in the report.
* Regressions: `phase2_detached_margin_allocations_are_ambiguous_not_strict` (two detached
  allocations in one unlabelled question -> strict `Err`, recovery keeps the content flagged with
  the gate on the card) and `phase2_labelled_and_single_allocations_stay_strict` (labelled
  sub-parts with equal allocations and a single detached allocation both stay strict).
* Effect on madas p2: the seven cards that previously reached the strict gate with a
  neighbouring `(5)` tag plus figure labels now report
  `ambiguous_mark_allocation` and `needs_review=true`; no card reports the 11-mark sum as a
  confident Tier-0 accept. The remaining seven are `no_marks_signal`, and Q8/Q9 are
  glyph-blocked. Phase 3 reconstructs the layout and can remove the uncertainty.

### C4. The corpus diagnostic is ignored by default

* `deterministic::tests::phase2_boundary_agreement_diagnostic` now carries
  `#[ignore = "manual corpus diagnostic (presence-dependent PDFs); phase 4 runs the explicit
  corpus gate"]` ([deterministic.rs:2147](src-tauri/src/deterministic.rs:2147)), so a default
  test run no longer counts an unavailable-fixture skip as a pass. The synthetic behavioural
  tests remain normal (not ignored). No env switch was added.

### Terminal semantics: no further broadening

Per the review, `energy released = J` is answer debris for phase-3 cleanup, not proof of a
complete equation. The unit-only acceptance was removed from
`ends_with_complete_formula` ([validate.rs:266](src-tauri/src/validate.rs:266)): a written
equation now counts only when it ends on a VALUE, and a bare value+unit pair (`12 N`, `0.50 A`)
still counts. Tests assert `!ends_with_complete_formula("energy released = J")` and
`!has_terminal_ending("energy released = J")`.

## Revision-1 work (unchanged)

1. Terminal-ending semantics ([validate.rs:305](src-tauri/src/validate.rs:305)): bare bracket
   allocations (`[2]`, `**[1]**`), navigation endings (`END OF SECTION A`, `END OF QUESTIONS`,
   `Turn over`), complete formula endings; cut-off prose stays rejected.
2. Conservative pre-gate delimiter repair in `transcribe_span` and `try_local_recovery` (the
   same idempotent healer the assembler applies).
3. Carver/document-map boundary agreement: madas-style `Question N (*****)` decorators, glued
   nuclide headings (`box 3 1 27Mg 12 can decay ...`), and a nuclide data-row guard
   (`20 Ne 10 19.99244`) that stopped the Q6 truncation; the element list is shared with the
   map (`doc_map::NUCLIDE_ELEMENTS`).
4. `marker_client::fix_mark_allocation` single-part tag deletion fixed (no more `****`).

## Verification (targeted; revision-2 runs)

| Command | Exit | Result |
| --- | --- | --- |
| `cargo test --lib -- validate::tests` | 0 | 30 passed (incl. `delimiter_machinery_treats_code_as_opaque`, tightened terminal cases). `p2r-validate2.log` |
| `cargo test --lib -- validate::tests sanitize::tests marker_client::tests` | 0 | 104 passed. `p2r-validate-marker.log` |
| `cargo test --lib -- deterministic::tests` | 0 | 56 passed, 1 ignored (the diagnostic). `p2r-deterministic.log` |
| `cargo test --lib -- phase2_ digital_one_page` | 0 | 7 passed, 1 ignored. `p2r-phase2-tests2.log` |
| `cargo test --lib -- doc_map::tests` | 0 | 16 passed (real AQA/Edexcel placement unaffected by the page-0 change). `p2r-docmap2.log` |
| `cargo test --lib -- pipeline::tests` | 0 | 81 passed (incl. the one-page end-to-end regression). `p2r-pipeline.log` |
| `cargo test --lib -- mark_scheme_deterministic::tests` | 0 | 12 passed. `p2r-ms.log` |
| `cargo test --lib -- commands::tests llm::tests` | 0 | 11 passed. `p2r-misc.log` |
| `cargo build --bin e2e_import` (rebuild harness) | 0 | `p2r-build-e2e.log` |
| offline e2e: physics '21, physics '24, madas p2, core pure 1 '21 | 0 each | all four: `cloud_attempts=0`, 0 prompt tokens, 0 completion tokens, $0.0000 |

The full Cargo suite and full corpus remain phase 4 (per the brief and the review).

### Targeted paper results (offline, zero cloud)

| Paper | Before phase 2 | After phase 2 (revision 2) |
| --- | --- | --- |
| physics '21 | strict 24, recoveries 6, quarantined 1 | **strict 25, recoveries 6, quarantined 0** |
| physics '24 | strict 32, 0, 0 | **strict 32, 0, 0** (preserved) |
| madas p2 | strict 0, recoveries 0, quarantined 16 | **strict 0, recoveries 14, quarantined 2** (all 16 carve; ambiguity gate flags 7, no mark signal 7, glyphs 2) |
| core pure 1 '21 | strict 2, recoveries 0, quarantined 7 | **strict 2, recoveries 0, quarantined 7** (boundaries already correct) |

### Per-question identity evidence (physics '21)

* **Q6** (the brief's target): before, the carve stopped mid-table (`... 16.99913`) and the gate
  said `no_terminal_ending`, losing subparts 6.3/6.4. After, the carve reaches the next real
  heading, the terminal gate passes, and the retained card contains the final subpart
  (`(d) $^{3}_{2}\text{He}$ can undergo fusion reactions with either $^{34}_{16}\text{S}$ or
  $^{17}_{8}\text{O}$ ... These properties affect ...`). The remaining blocker is
  `table_columns_not_recovered` (Table 2 plain-text columns) - phase 3.
* **Q25**: the `unbalanced_math` defect (one stray `$` from
  `The transformer is 90% efficient.`) is cleared by the pre-gate balancer; the remaining
  blocker is the table-header wrap (`Secondary voltage $/ \text{V}$ Secondary current / ...`)
  - phase 3.
* **Q31** (final question before END OF QUESTIONS): now a strict card, `needs_review=false`,
  `issues=clean`, with the isotope heading (`$^{27}_{12}\text{Mg}$ can decay by beta minus
  emission ...`) and the diagram attached.
* Unchanged phase-3 recoveries: Q5 `table_columns_not_recovered`, Q9/Q14 MCQ option tagging,
  Q27 `isotope notation is scrambled in plaintext ("10 ... D 2")`.

### madas evidence (margin contamination now conservative)

Post-correction reasons: `ambiguous_mark_allocation` for Q2/Q5/Q7/Q10/Q12/Q14/Q16,
`no_marks_signal` for Q1/Q3/Q4/Q6/Q11/Q13/Q15, and explicit local failures for Q8/Q9
(symbol-font glyphs). Q2's card is now `needs_review=true` with the gate recorded rather than a
confident strict card; the underlying contamination is unchanged and is phase-3 layout work:

```
Show by a suitable algebraic method that
... 22 21 1620 - + - + + - = .
,
(5)
4 cm
A B
C D
E
(6)
```

### core pure 1 '21 (boundary already correct)

All 9 spans carve. Seven are blocked by Edexcel symbol-font glyphs (phase 3): page 2's matrix
line `M = -- -  44 3 43 4` carries the private-use bracket glyphs `U+F8EB`, `U+F8EC`, `U+F8ED`,
`U+F8F6`, `U+F8F7`, `U+F8F8`; pages 6/14/18/24/32 carry the same class. No carver change was
needed here.

## Remaining phase-3 items (with source examples)

* **MCQ option tagging** - physics '21 Q9/Q14 two-column options (`x y` /
  `A pressure in Pa temperature in ºC`).
* **Isotope notation** - physics '21 Q27 plaintext scramble (`"10 ... D 2"`).
* **Markdown tables** - physics '21 Q5/Q6 (`Cyclotron B / T R / m`, `Nucleus Mass / u`) and the
  madas right-margin columns/labels above.
* **Symbol-font glyphs** - core pure 1 '21 pages 2/6/14/18/24/32 (`U+F8Exx`/`U+F8Fxx`) and
  madas Q8/Q9.

## Risks and limits

1. Strict Tier-0 remains below the 95% target on the wider corpus; phase 2 fixes the mechanics
   (terminal, delimiters, boundaries, tag preservation, first-page papers, margin ambiguity) and
   lifts physics '21 to 25 strict with zero quarantine, but MCQ/isotope/table/font gates still
   cap the count.
2. madas cards are all recoveries/failures now; none claims a confident total while the
   margin-column association is unresolved. Phase 3 owns the layout reconstruction.
3. The corpus diagnostic is ignored by default; phase 4 must run it (or the explicit corpus
   gate) deliberately so an unavailable fixture is never counted as a pass.
4. Mark schemes, provider settings, dependencies and production data are untouched.

## Changed paths

* `src-tauri/src/validate.rs` - code-aware delimiter machinery (`inline_code_spans`,
  `fence_marker`, `line_code_map`, `normalize_dollar_runs_outside_code`,
  `push_balanced_segment`), terminal-ending semantics with the tightened formula rule
  (`ends_with_complete_formula`, `has_terminal_ending`), new tests.
* `src-tauri/src/doc_map.rs` - evidence-based page-0 handling
  (`has_strong_first_page_question`, `is_front_matter_page`), no unconditional page-0 guard in
  the heading-span builder, `NUCLIDE_ELEMENTS` shared with the carver, new tests.
* `src-tauri/src/deterministic.rs` - shared nuclide evidence, pre-gate delimiter repair,
  `ambiguous_mark_allocation` guard, ignored corpus diagnostic, phase-2 tests.
* `src-tauri/src/marker_client.rs` - single-part mark-tag preservation + tests.
* `docs/agent-work/zero-cost-orchestration/phase-2-report.md` - this report.
* `output/zero-cost-orchestration/` - phase-2 logs, source dumps and certification JSONs.