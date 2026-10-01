# Tier-0 Deterministic Extraction — Implementation Plan

Zero-cost import cascade, work item #1: replace the LLM text-first call with a deterministic Rust transcriber for text-reliable, figure-free question spans. LLM becomes the fallback, not the engine.

## Context (what exists today)

- Production text extraction feeds `pdf_extract` page texts into the PVRV pipeline (`src-tauri/src/pipeline.rs`, 9340 lines).
- Routing already computes per-span: `has_text`, `text_refs_figure`, `must_read` (`figure_read_required`), figure candidates, `needs_vision`.
  - Figure-free spans → `try_text_first_extraction` (pipeline.rs:3592) — ONE LLM call per span that sends RAW TEXT and expects JSON `{items:[...]}` (`AiQuestionPage`, pipeline.rs:683).
  - Shared pages hosting several questions → `extract_same_page_batch` (pipeline.rs:4990) with combined text-first call (pipeline.rs:5046).
- Acceptance seam: `build_question_from_parsed_page` (pipeline.rs:2869) filters target items, enforces placeholder/bbox gates, runs `validate_span_items` (pipeline.rs:5460), then `assemble_built_question` (pipeline.rs:4904) applies `clean_question_content`, `normalize_decimal_parts`, marker cleanup, delimiter balancing, marks reconciliation (printed footer authoritative), terminal-ending review flags.
- The LLM contract is fully specified by `text_first_system_prompt` (pipeline.rs:1541): math delimiters `$…$`/`$$…$$`, `\frac`, sub-part `(a)` paragraphs separated by `\n\n`, `**[X marks]**` placement, answer-line stripping, sentence rejoining, strict question isolation.
- Boundary evidence already computed deterministically in `doc_map.rs`: `scan_text_layer` (line 294) finds headings (`QuestionHeading`: page, number, y_frac-as-byte-proxy) and board-wide footers (`Footer`: page, question, marks); `text_layer_map_sufficient` (line 266) is calibrated against all AQA physics '17–'24 fixtures at repo root (`physics '17.pdf` … `'24.pdf`) plus `test_papers/`.
- Gating precedent: `commands.rs:1454` reads `MERGEMARK_TEXT_FIRST` env into `config.text_first`; `PipelineConfig::new()` defaults it false so old tests are unaffected.

## Decisions (confirmed with user)

1. **LaTeX depth**: token-level Unicode→LaTeX + simple patterns (`10⁻¹⁹`→`$10^{-19}$`, `x^2`→`$x^2$`, nuclear prescripts, nuclide decay arrows). NO structural parsing of stacked fractions/matrices/tables — those spans escalate.
2. **Rollout**: production default-ON via commands.rs wiring; kill switch `MERGEMARK_DETERMINISTIC=0`; `PipelineConfig::new()` field defaults `false` so every existing test keeps today's behavior.
3. **Surface**: both paths — single-span site AND shared-page batch path. All-spans-pass ⇒ zero API calls for the page; any failure ⇒ fall back to the existing combined call unchanged.

## Scope decisions (stated, no further sign-off needed)

- **Out of scope**: mark-scheme pipelines (`MsTextFirst`/`MsWindow`), crop-first/vision paths, corpus/packs (item #2), frontend changes.
- **Topics**: if `config.allowed_topics` is non-empty, Tier 0 declines that span (topic classification stays with the LLM). Empty allow-list ⇒ topics `[]` exactly like the prompt demands.
- **Tables**: row heuristic (≥2 column-gap runs on consecutive lines) ⇒ escalate, don't convert.
- **is_code**: `false` from Tier 0 (CS-subject code detection stays with the LLM; CS papers commonly escalate elsewhere anyway).
- **Escalation is invisible**: returning `None` flows into today's code path unchanged — no new failure modes for users.

## Architecture

New module `src-tauri/src/deterministic.rs`. It mimics the LLM's job: consume carved span text, emit an `AiQuestionPage`, then flow through the UNCHANGED acceptance seam (`build_question_from_parsed_page`). Validators are tier-invariant by construction.

```
extract_span:  needs_vision==false ──► try_deterministic_extraction()   [NEW]
                                        ├─ Some(built_q) → attach_detected_figures → done ($0)
                                        └─ None → try_text_first_extraction (unchanged)
extract_same_page_batch:            ──► carve ALL target spans locally  [NEW]
                                        ├─ all pass → skip API entirely ($0)
                                        └─ any fail → existing combined call (unchanged)
```

## Task List (ordered)

### 1. Module skeleton + config flag
- [ ] `PipelineConfig` gains `pub deterministic: bool` (default `false` in `new()`).
- [ ] `commands.rs` (~line 1454, next to `MERGEMARK_TEXT_FIRST`): `config.deterministic = std::env::var("MERGEMARK_DETERMINISTIC").map(|v| v != "0").unwrap_or(true);`
- [ ] Register `mod deterministic;` in `lib.rs`.

### 2. Span carving (`deterministic.rs`)
Line-based boundary parser over each span page's raw text (self-contained regexes; do NOT couple to doc_map internals):
- Start: first line matching whole-number heading for `span.number` (`^\s*N[\.\)]?\s`, `^Q\s*N\b`, `^Question\s+N\b`) on `span.start_page`; honor `start_y_frac` by slicing lines via the same byte-ratio trick `validate::slice_page_text_by_y` uses (reuse it for the y-bounds; it already applies safety margins).
- End: whichever comes first — footer line for N (`(Total for Question N …marks)`, reuse `doc_map`'s pattern family as local regex), next whole-number heading line, or page end. Interior pages taken whole; final page bounded by `end_y_frac`/footer.
- Strip page furniture: running headers (board name + paper-code line repeated at page top), bare page-number lines, barcode/registration noise. Keep a small const list per board prefix (`AQA`, `Pearson Edexcel`, `Cambridge`, `PMT`).
- Output struct `CarvedSpan { text, found_start, found_end, footer_marks: Option<u32>, subpart_numbers: Vec<u32>, answer_lines_seen: usize }`.

### 3. Text normalization pipeline (in-module, ordered stages)
Each stage returns enough info for the confidence gate:
1. Ligature/artifact cleanup — delegate to `validate::clean_ligatures`.
2. Answer-line stripping — underline runs, dotted leaders, `answer = ___`, trailing unit labels on dropped lines (mirror prompt rules 1575–1576). Count removals.
3. Sentence rejoining (conservative): merge a hard-wrapped line into the previous ONLY when previous line lacks terminal punctuation AND next line starts lowercase or a continuation word (`of`, `the`, `and`, `to`, `in`, `with`, `=`…). Never join across blank lines, sub-part labels, display math candidates, table-ish lines.
4. Sub-part formatting: ensure `(a)`, `(b)` labels begin paragraphs (`\n\n` separation); AQA decimal labels `03.1` normalized here or left for `normalize_decimal_parts` (which already handles it downstream — prefer downstream, don't duplicate).
5. Marks: parse `[n marks]`/`(n)` tags per sub-part with a `MARKS_RE`-style regex; append `**[X marks]**` at end of each marked sub-part; keep total for the item.
6. Unicode→LaTeX (token-level, decision #1):
   - Static map: √→`\sqrt`, ×→`\times`, ÷→`\div`, ±→`\pm`, ≤≥≠≈→LaTeX, Greek letters (θ α β λ Ω μ ρ σ π φ ω Δ…), ∘→`^{\circ}`, →(math ctx)→`\rightarrow`, ∝→`\propto`, ℓ, Å, ‰, superscripts ⁰¹²³⁴⁵⁶⁷⁸⁹⁻⁺ and subscripts ₀₁₂ₙ₋₊ → `^{…}`/`_{…}` runs.
   - Patterns: `N×10⁻¹⁹` / `Ne-p` scientific forms → `$N \times 10^{-19}$`; nuclear prescripts `²²⁶Ra` or `226/88 Ra` sequences → `$^{226}_{88}\text{Ra}$`; decay chains keep per-equation line structure (each own line; wrapping in `$$…$$` only if standalone line).
   - Wrapping: wrap contiguous math-token clusters (identifier+operators+digits+converted symbols) in `$…$`; NEVER wrap prose, units standing alone, marks tags, or whole sentences. Skip wrapping inside backticks (code).
7. MCQ options: lines matching `^[A-E][\.\)]` kept verbatim, one per line (no `\n\n` inflation).

### 4. Confidence gate (hard gates, all-or-nothing — return `None` on any miss)
1. `found_start && found_end` (or ran to document end legitimately for last question).
2. Marks checksum: parsed inline total equals `span.expected_marks` when known; else if unknown, require ≥1 inline tag when `footer_marks` absent.
3. Sub-part contiguity: decimal/sub-part sequence complete with no gaps.
4. Zero leftover answer-line artifacts (post-strip scan).
5. Terminal ending on final content (reuse `validate::has_terminal_ending`).
6. Length plausibility: ≥ 30 chars and ≥ 25 chars-per-mark.
7. Math delimiters balanced (reuse `validate::math_delimiter_balance_errors` — must be empty before assembly, since Tier 0 has no repair loop).
8. Table-like structure detected ⇒ decline.
9. `allowed_topics` non-empty ⇒ decline (scope decision).

### 5. Emission through the existing seam
- Build `AiQuestionPage { items: vec![AiQuestion { question_number: json!(span.number), content: Some(converted), marks: Some(json!(total)), topics: None, module: None, is_code: None, diagram_*: None/empty, math_snippet: None, visual_options: None }] }`.
- Call **`build_question_from_parsed_page`** directly — it re-checks placeholders-vs-figures, bboxes, and runs `validate_span_items`, then `assemble_built_question`. Tier 0 inherits every future validator improvement for free.

### 6. Pipeline integration — single-span path
At pipeline.rs:3591 block, before `try_text_first_extraction`:
```rust
if text_first && has_text && !needs_vision && config.deterministic {
    if let Some((mut built_q, mut t0_report)) = deterministic::try_deterministic_extraction(config, span, span_pages) {
        // mirror lines 3604-3624 exactly: log, attach_detected_figures,
        // pages_processed, push_mark_check, report.absorb
        t0_report.deterministic += 1;
        return (Some(built_q), report);
    }
}
```
Sync function (no client/semaphore/cancel params needed; still early-return on cancel flag for consistency).

### 7. Pipeline integration — shared-page batch path
In `extract_same_page_batch` (~line 5011–5090 text-first pre-check):
- When `config.deterministic`: attempt local carve for EVERY target span on the page.
- All succeed → assemble all, count `report.deterministic += n`, skip the combined API call entirely.
- Any failure → discard partial results, fall through to `try_text_first_batch_extraction` unchanged (fallback-for-all semantics; simplest correct behavior).

### 8. Reporting & telemetry
- `ImportReport`: add `pub deterministic: usize` (+ absorb in `absorb()` alongside `text_first` at pipeline.rs:594).
- `StageTag::Deterministic => "deterministic"` for timing entries (`record_timing("extraction","tier0",…)`, output_cap irrelevant — zero tokens).
- Log line: `[TIER0] Question {} carved locally (0 tokens)`; escalations: `[TIER0_FALLBACK] question={} reason=<gate_id>`.

### 9. Tests (all new tests opt-in via explicit config; existing suite untouched)
Unit tests in `deterministic.rs` (synthetic page texts, no pdfium):
- Carve boundaries: mid-page start under previous footer; shared page with next heading; multipage span; missing footer at paper end.
- Each normalizer stage: answer-line + trailing-unit strip; conservative rejoin (joins wrapped sentence, does NOT join across sub-part/display-math/table-ish lines); marks formatting/placement; unicode map cases incl. scientific notation and nuclear prescripts; MCQ options.
- Gate refusals: marks mismatch, sub-part gap, leftover blanks, unbalanced `$`, table detection, allowed_topics non-empty, min-length.
- Seam test: emitted `AiQuestionPage` passes `build_question_from_parsed_page` for a representative span.
Integration (mock client, follows existing `MockLlmClient` patterns):
- `tier0_resolves_span_without_api_calls`: text-reliable paper, `deterministic=true` → mock records ZERO calls; card contents correct.
- `tier0_escalates_to_llm_on_garbage`: mock called once via existing text-first path.
- `tier0_batch_skips_combined_call`: 5-MCQ shared page fully local; and partial-failure case falls back to exactly ONE combined call.
Fixture diagnostics (pdfium-guarded, skip-if-missing pattern like `diagnostic_gate_decisions_on_fixture` pipeline.rs:8840):
- Run physics '17–'24 + `test_papers/*_qp.pdf`: print per-paper Tier-0 accept rate and gate-refusal reasons. Manual-inspection harness, not assertions.

### 10. Calibration & rollout
- Tune gate thresholds against the physics '17–'24 fixture family using the diagnostic output; target ≥70% span acceptance with zero wrong-content accepts (precision over recall — misses only cost text-first tokens, which is today's status quo).
- No `CACHE_VERSION` bump: cached extractions remain valid; Tier 0 only affects fresh imports.

## Risks & Mitigations

| Risk | Mitigation |
|---|---|
| Over-conversion corrupts cards silently | Token-level scope (decision #1); balanced-delimiter hard gate; everything still passes `assemble_built_question` cleaners; needs_review flags preserved |
| Wrong question isolated (Frankenstein) | Hard boundary gates (start AND terminator required); validators unchanged; marks checksum must match footer |
| pdf_extract vs pdfium text differences | Carver is line/regex-based on whatever text arrives; diagnostics compare both sources (pattern at pipeline.rs:8867) |
| Board layout drift breaks boundaries | Gate refusal → LLM fallback (status quo), never a wrong card; per-board furniture lists isolated in consts |
| Silent regression on odd papers | Kill switch `MERGEMARK_DETERMINISTIC=0`; `deterministic` counter in report makes adoption visible |

## Validation Plan

1. `cargo test` in `src-tauri` — full suite green (existing tests prove non-regression since flag defaults false).
2. `cargo clippy` clean.
3. Fixture diagnostics: physics '17–'24 + test_papers — inspect accept rates and spot-check carved content against PDFs for at least 3 boards.
4. One manual production import of `physics '21.pdf` with flag on: confirm report shows `deterministic > 0`, OpenRouter usage delta ≈ 0, cards render correctly (KaTeX) in review screen.

## Explicitly Out of Scope

Mark-scheme pipelines; corpus/community packs (#2); free-model routing (#3/#4); crop-image optimization (#5); topic classification; frontend display of the new counter (backend report field only).
