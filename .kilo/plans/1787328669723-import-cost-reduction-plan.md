# Plan: Cut import cost ~50% and make the meter accurate

## Goal
Reduce the physics '24-style import cost from ~$0.062 to ~$0.028-0.032 per import, and make cost reporting trustworthy so reductions are verifiable (the previous plan failed because the estimator under-reported by ~6x).

## Measured baseline (physics '24 import)
- Real bill: **~$0.062**. App estimate printed: $0.0101 (wrong).
- Routing: 29 text-first calls (0 images), 0 crop-first, 3 full-page vision, 32 questions, 40 pages. Figures: detection+crop fully local ($0).
- Cost split: text-first prompt ~59% (system prompt ~3,600 tok + schema ~900 tok + span text, re-paid **29 times**), text-first output ~31%, vision calls ~10%.
- Model: `google/gemini-2.5-flash` (billing.rs:44), real OpenRouter rate **$0.30/M in, $2.50/M out** — but `calculate_cost` prices all "gemini" at $0.10/$0.40 (cost.rs:49-50).

## Why "make sure it actually works" is a first-class deliverable
The estimate pipeline lies today:
1. Wrong rates (`cost.rs:49-50`, `UsageDashboard.tsx:45-47`).
2. Guessed token counts `prompt_est = pages*1400`, `comp_est = questions*350` (commands.rs:1674-1675) — no per-call system-prompt/schema overhead, no images.
3. No before/after OpenRouter usage delta — logged `usage_usd` is cumulative lifetime (commands.rs:1684-1693).
4. Real per-call token usage is already parsed (`llm::usage_from_response`, pipeline.rs:240-248) but only printed under `MERGEMARK_LOG_USAGE`, never accumulated.

## Changes (ordered — measurement first, then each saving is verifiable)

### 1. Accurate measurement foundation
1. **Fix pricing** in `cost.rs` `calculate_cost`: replace the catch-all `m.contains("gemini") → (0.10, 0.40)` with explicit `gemini-2.5-flash → (0.30, 2.50)` and `gemini-3.7-flash` (verify current OpenRouter rate before hardcoding; it is a reasoning model). Keep other entries. Mirror the same rates in `src/components/settings/UsageDashboard.tsx:45-47`.
2. **Accumulate real tokens**: add a shared `Arc<TokenTotals>` (`prompt_tokens`, `completion_tokens` as `AtomicU64`), created in `run_question_pipeline` and threaded into `chat_with_permit` (pipeline.rs:214) — the single choke point that already parses usage for the `[TOKENS]` log. Update all `chat_with_permit` call sites (text-first 2240, crop-first, vision 2980, same-page batch, markscheme window, structure pass). After each response: `token_totals.prompt += u.prompt_tokens; token_totals.completion += u.completion_tokens;`.
3. **Use real totals in the estimate**: in `commands.rs` (1674-1695) replace the guessed `prompt_est`/`comp_est` with the accumulated totals; print the new `[COST]` line and record them to `record_import_cost` (1703-1714). Keep the existing reasoning multiplier (3.0x for "3.7"/"o1"/"o3"; 2.5-flash gets 1.0x — correct, since `chat_body` sends `reasoning: { effort: "none" }` for it).
4. **Expected artifact**: after this task alone the `[COST]` line jumps from $0.0101 to ~$0.06 with zero behavior change. This is the meter becoming honest — not a regression.
5. **Baseline capture**: run one physics '24 import with `MERGEMARK_LOG_USAGE=1` and record: `[TOKENS]` totals, `[COST]` estimate, OpenRouter usage delta (~$0.062), `[PATH_SUMMARY]`, question quality. This file/baseline is the comparison target for every later task.

### 2. Slim the text-first prompt + schema (~$0.018)
The text-only call forbids `diagram_bboxes`/`visual_options` yet re-sends the 7-box-drawing few-shot examples (pipeline.rs:1027-1058, ~5,577 chars) plus the full bbox-bearing schema (pipeline.rs:931-976, ~2,341 chars).
1. Rewrite `text_first_system_prompt` (pipeline.rs:1126) to a compact prompt: keep the math-delimiter, adaptive-marks/difficulty, sub-parts-vs-MCQ, and question-isolation rule sections as plain text; **drop the FEW_SHOT block**; replace the bbox output rules with one line ("diagram_bboxes, diagram_captions, diagram_kinds, bbox_page_indexes MUST be empty arrays; visual_options MUST be null"). Add **one** compact few-shot example (multi-part question with marks + math only, no bboxes) to anchor formatting.
2. Add a slim JSON schema for text-first calls (`text_first_json_schema()`): `extraction_json_schema()` minus the 5 bbox/visual fields. `AiQuestion` uses `#[serde(default)]` (pipeline.rs:515-517), so omitted fields parse fine; `validate_span_items` and `attach_detected_figures` are unaffected.
3. Use the slim prompt+schema in `try_text_first_extraction` (pipeline.rs:2218, 2234). Leave the vision and crop-first prompts untouched (crop-first is rare; vision needs bboxes).
4. Regression net: the 8 existing text-first tests + `clean_marker_markdown`/`validate` post-processing must stay green.

### 3. Combined text-first batch call per shared page (~$0.011)
Today `extract_same_page_batch` (pipeline.rs:4062) loops `try_text_first_extraction` **per span** (4098-4137) — N calls, each re-sending the same page text and the full prompt overhead. Shared pages on physics '24: 23 (Q9-11), 24 (Q12-13), 26 (Q15-16), 28 (Q19-20), 33 (Q27-29), 35 (Q31-32) → 14 questions that can be one call each → 8 fewer calls.
1. **Refactor**: extract the acceptance-gates + assembly tail of `try_text_first_extraction` (pipeline.rs:2304-2403) into `fn build_question_from_parsed_page(page: &AiQuestionPage, span, config, available_figures) -> Option<BuiltQuestion>`. Both the single path and the new batch path call it (no duplicated gate logic).
2. **New `try_text_first_batch_extraction`**: one call whose user prompt lists all question numbers ("Transcribe Questions 9, 10, 11 from the RAW TEXT below"), using the slim prompt+schema. Parse once; group `items` by `question_number`; run `build_question_from_parsed_page` per span (validating the page-level needs_vision gate first, exactly as the current loop does at 4101).
3. **Fallback chain** (must not lose questions): (a) combined call parses → build what validates; (b) any missing/rejected span → individual `try_text_first_extraction`; (c) still missing → the existing shared-page vision call / per-span `extract_span` fallback (current behavior at 4141+, 4162-4181). Mirror the `same_page_batch_partial_fallback_recovers_missing_question` pattern.
4. Wire into `extract_same_page_batch` replacing the per-span loop (keep the text-map gate and the "any span needs vision → fall back to shared-page vision call" behavior).

### 4. Q24 unnumbered-figure band fix (~$0.003, one less vision call)
Q24 (page 30) references its figure by word only (`refs=[]`); the detector found one figure on the page at center y≈0.24, outside the span's y-band, so `span_figure_candidates` returns 0 → full-page vision (verified by the fixture gate diagnostic).
1. In `span_figure_candidates` (pipeline.rs:2018): when `referenced.is_empty()` AND the span is single-page AND the span's own pages contain **exactly one** detected figure, accept it regardless of the y-band. 
2. Hard guard: if a page has 2+ detected figures and no "Figure N" reference (e.g. MCQ visual-option pages), do **not** relax — stay on the current band rule so we never attach the wrong figure. This is the whole point of the band.
3. Unit-test both branches (relaxed when unique figure; not relaxed when multiple figures on page).

### 5. Exact before/after usage snapshot (stretch, small)
Store OpenRouter `usage_usd` at import start and log the delta at the end (the code comment at commands.rs:1685 already admits it can't do this). A tiny `usage_config`-style column or a `import_cost_snapshots` table. If it looks risky, skip — the real-token estimate from Task 1 is already accurate.

### 6. Build marker + final validation
Bump the marker in `commands.rs` `parse_pdf_vision` to `[BUILD] figure-fix-v6: accurate cost metering + slim text-first prompts + text-first batching`.

## Tests (new)
- `cost.rs`: `calculate_cost("google/gemini-2.5-flash", ...)` matches $0.30/$2.50 arithmetic; update existing gemini expectations.
- Prompt slimming: `text_first_system_prompt` does not contain "FEW-SHOT" or "diagram_bboxes"; slim schema has no bbox fields.
- `build_question_from_parsed_page`: split-response merge still works (existing `text_first_merges_multi_item_response` logic moves here).
- Combined batch: mock returns one response with items for 3 question numbers → assert 1 API call, 3 BuiltQuestions, correct contents, zero images.
- Batch partial fallback: combined response missing one question → that span gets an individual text-first call.
- Band relaxation: (a) single unnumbered figure outside band → candidate found; (b) two figures on page → not relaxed.
- Token accumulation: mock with a `usage` block → `TokenTotals` reflects prompt+completion across all calls.

## Validation protocol (how "it actually works" is proven)
1. `cd src-tauri && cargo test --lib -- --test-threads=1` (all tests, new + existing).
2. `cargo check --all-targets` (no warnings) and `npx tsc --noEmit` (frontend rate change).
3. Manual import of `physics '24.pdf` with `MERGEMARK_LOG_USAGE=1`, then compare against the Task-1 baseline:
   - `[PATH_SUMMARY]` text-first count must not drop below baseline; spot-check Q2 (merge), Q8, Q24, Q30 content.
   - `[TOKENS]` prompt totals must fall by ~62k (slimming) + ~36k (batching) ≈ −100k.
   - `[COST]` estimate must match the OpenRouter usage delta within ~15%, and the real bill must land ~$0.028-0.035.
   - `[BUILD] figure-fix-v6` marker present.
4. If prompt slimming regresses quality on the fixture, relax Task 2 (e.g. keep 2 few-shot examples) rather than the correctness gates.

## Out of scope
- Q8 (page 22): no figure detected at all — detector filter tuning is exploratory and carries false-positive crop risk (~$0.003). Separate follow-up.
- Replacing the hardcoded rate table with a live OpenRouter `/models` fetch (nice future hardening).
- Page-count/quality changes unrelated to the four levers.
