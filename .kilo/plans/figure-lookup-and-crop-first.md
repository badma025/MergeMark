# Plan: Close the $0.055 → ~$0.02 gap — Adjacent-page figure lookup + crop-first vision fallback

## Problem (measured)

The deterministic figure detector works (physics '24 → 32 figures, '21 → 48, all "Figure N" captions correct), but an import still costs $0.0557 with only 11/28 questions going text-first. The remaining ~17 figure questions fall back to the full-page vision path because:

1. **The figure is usually not on the span's own pages.** `span_eligible_figures` only searches `start_page..=end_page`. In AQA the figure frequently sits on the page after the question text (or a facing page) → `fig_count == 0` → vision.
2. **Read-from-figure questions** ("use the graph to determine…", "state the value of the resistor…") are intentionally kept on vision for correctness, and they currently send full pages (~10k image tokens each).

Both are fixable without a local vision model.

---

## #1 — Adjacent-page / caption-matched figure lookup

**Where:** `src-tauri/src/pipeline.rs`.

### Current state

- `count_figure_references(text) -> usize` (dead after this change — remove it).
- `span_eligible_figures(span, span_pages, page_figures) -> Vec<(usize, &DetectedFigure)>` — band-eligible figures on the span's own pages only. Used by the gate and by `attach_detected_figures`.
- Gate in `extract_span` (~line 2363): `needs_vision = (text_refs_figure && fig_count == 0) || (text_refs_figure && figure_read_required && fig_count < fig_refs)`.

### Changes

1. **Add `figure_reference_numbers(text: &str) -> Vec<u32>`** — extract numbers from `(?i)\bfigure\s*(\d+)|\bfig\.?\s*(\d+)`. Replaces `count_figure_references`.

2. **Rename `span_eligible_figures` → `span_band_figures`** (same body, band-only, span's own pages).

3. **Add `span_figure_candidates(span, span_pages, page_figures, referenced: &[u32]) -> Vec<(usize, &DetectedFigure)>`**:
   - Step 1: band-eligible figures on span pages (`span_band_figures`).
   - Step 2: if a referenced "Figure N" is not yet found, search the **adjacent pages** `start_page-1` and `end_page+1` (bounds-checked, skip pages already in `span_pages`), full height, accepting figures whose caption matches a referenced number.
   - Step 3: for referenced numbers still missing, scan **the whole paper** and accept any figure whose caption matches (captions are the identity of the figure; AQA figure numbers are unique per paper).
   - Dedup via `(page_idx, bbox)` identity (helper `add_unique_figure`); caption match via `caption_num_matches` (uses existing `fig_number_from_caption`).

4. **New gate in `extract_span`** (this is also a correctness fix):
   ```rust
   let must_read = figure_read_required(&combined_text);
   let referenced = figure_reference_numbers(&combined_text);
   let fig_count = span_figure_candidates(span, span_pages, page_figures, &referenced).len();
   let needs_vision = must_read || (text_refs_figure && fig_count == 0);
   ```
   - Read-required questions **always** go vision (crop-first now, see #2) — a text-first answer for "read the value off the graph" is wrong, even with the figure attached afterwards.
   - Non-read-required figure questions go text-first **whenever a figure is findable** (span page, adjacent page, or anywhere by caption). This is the cost win.
   - The existing placeholder safety net still holds: if the model emits more `[DIAGRAM_PLACEHOLDER]` than `available_figures`, `try_text_first_extraction` rejects → full-page vision.

5. **`attach_detected_figures`** — change line 2010 from `span_eligible_figures(...)` to
   ```rust
   let referenced = figure_reference_numbers(content);
   let eligible = span_figure_candidates(span, span_pages, page_figures, &referenced);
   ```
   (page_b64 lookup for `pdf_path == None` stays span-only — fine for tests; production uses the 300-DPI render cache keyed by `global_page_idx`.)

6. **Remove `count_figure_references`** (no longer referenced).

### Expected effect
Most of the ~17 vision questions become text-first with a deterministic crop attached (the figure now found via adjacent/caption lookup). `figure_read_required` questions stop producing wrong text-first answers.

---

## #2 — Crop-first vision fallback (tiny figure crops for the residual vision questions)

**Where:** `src-tauri/src/pipeline.rs`.

The residual vision questions are the read-required ones. Instead of sending 1–2 full pages (~10–20k image tokens), send **only the detected figure crop(s)** (~512px, ~4k tokens). The question wording comes from the text layer in the user message; the crop carries the values.

### Changes

1. **`crop_first_system_prompt(config)`** — mirror `text_first_system_prompt`: base = `extraction_system_prompt(config)`, then override:
   - "The attached image(s) are the question's figure(s), already cropped by the system. Read any required values/coordinates/readings from them."
   - "Transcribe the question exactly; the RAW TEXT is authoritative for wording."
   - "`diagram_bboxes`/`diagram_captions`/`diagram_kinds`/`bbox_page_indexes` MUST be empty arrays and `visual_options` MUST be null — the figure content is already provided as an image, do not box it."
   - "Do NOT insert [DIAGRAM_PLACEHOLDER]."

2. **`const CROP_FIRST_MAX_DIM: u32 = 512;`** — 512px long edge = 4×4 tiles ≈ 16 × 258 ≈ 4.1k tokens (vs ~10.3k for a 640px page). This is the 2.5× per-image / ~5× per-call win.

3. **`async fn try_crop_first_extraction<C: LlmClient>(...)`** → `Option<(BuiltQuestion, ImportReport)>`:
   - Params: `client`, `config`, `span`, `combined_text`, `candidates: &[(usize, &DetectedFigure)]`, `page_render_cache`, `request_semaphore`, `cancel`.
   - For each candidate: get the page image (`page_render_cache.get_or_render(pdf_path, page_idx)` when `config.pdf_path` is `Some`; else look the b64 up in `span_pages` and `geometry::decode_page_image`; return `None` if neither source exists), `geometry::crop_diagram_with_options(img.as_ref(), &fig.bbox, 8, true, false)` (small padding, `ignore_grid=true` — a graph grid is exhibit, not an answer grid), then `geometry::encode_webp_resized(&DynamicImage::ImageRgba8(crop), CROP_FIRST_MAX_DIM)`. If every crop fails → `None`.
   - One API call: `llm::chat_body(&config.model, &crop_first_system_prompt(config), &crop_b64s, llm::ImageDetail::High, Some(&user_text), config.max_output_tokens, Some(ResponseFormat::JsonSchema { schema: extraction_json_schema() }))`, where `user_text` = `"TARGET: Question {n}\nPAPER: …\nMODULE: …\n\nRead the values needed to answer Question {n} from the attached figure image(s), and transcribe the question exactly. RAW TEXT (authoritative for wording):\n{combined_text}"`.
   - Acceptance (mirror the text-first gates): parse `Clean` (or `Salvaged` with `dropped_tail == false`), exactly **one** item for `span.number`, non-empty content, `validate_span_items` passes. Strip `[DIAGRAM_PLACEHOLDER]`. No repair loop (single attempt; a failure just falls through to the existing full-page vision path).
   - Build via the same `assemble_built_question` tail used by `try_text_first_extraction`.
   - Log `[CROP_FIRST] Question {n} answered from figure crops ({m} small images)`.

4. **Wire into `extract_span`**, between the text-first block and the chunk loop (~line 2413):
   ```rust
   // Figure-reading questions: show the detected figure crops instead of full pages.
   if needs_vision && must_read && !combined_text.trim().is_empty() {
       let candidates = span_figure_candidates(span, span_pages, page_figures,
           &figure_reference_numbers(&combined_text));
       if !candidates.is_empty() {
           if let Some((mut built_q, mut crop_report)) = try_crop_first_extraction(
               client, config, span, &combined_text, &candidates,
               page_render_cache, request_semaphore, cancel,
           ).await {
               attach_detected_figures(config, span, span_pages, page_figures,
                   page_render_cache, &mut built_q.content, &mut crop_report).await;
               crop_report.pages_processed += (span.start_page..=span.end_page).count().max(1);
               push_mark_check(span, &built_q, &mut crop_report);
               report.absorb(crop_report);
               return (Some(built_q), report);
           }
       }
   }
   ```
   - Requires `text_first` enabled and the text layer present (the wording comes from the text; scanned pages fall to full-page vision as today).
   - Restructure the gate block so `combined_text`, `must_read`, `referenced`, `fig_count`, `needs_vision` are computed **outside** the `if text_first` block and reused by both the text-first attempt and the crop-first attempt.
   - If crop-first fails (validation/parse/crop), execution falls through to the existing chunk loop → full-page vision, unchanged.

### Expected effect
The ~5–10 genuinely figure-reading questions per paper drop from ~$0.003 each to ~$0.0004–0.001 each. Combined with #1, a digital AQA import should land at **~$0.012–0.02/paper**.

---

## Tests

- **`adjacent_page_figure_enables_text_first`** (pipeline): span = page 0 only, text references "Figure 2", `page_figures` has Figure 2 detected on page 1 (adjacent). Mock text-first response (valid, no placeholders). Assert `report.text_first == 1`, `mock.bodies().len() == 1`, no image sent. This pins #1.
- **`whole_paper_caption_match_finds_distant_figure`** (pipeline): span = page 0, references "Figure 5" detected only on page 12. Assert text-first is attempted (1 text-only call).
- **`crop_first_reads_figure_crop_not_full_page`** (pipeline): read-required text ("Use the graph to determine the value of $x$. Figure 3 shows the graph."), one detected figure on the span page, `config.pdf_path = None` (b64 source), `diagrams_dir` set. Mock response with the value. Assert: exactly **one** call, the body image decodes to ≤512px long edge (not the full 1200×1600 test page), `text_first == 0`, content contains the value. Pins #2.
- **`crop_first_falls_back_to_full_page_when_parse_fails`** (pipeline): crop-first mock returns malformed JSON → assert the second (full-page) call happens and the question still builds.
- Existing tests must stay green: `text_first_skipped_when_text_references_figure` (empty `page_figures` → `fig_count == 0` → vision ✓), `vision_fallback_when_figure_read_required` (read-required, empty figures → crop-first skipped → full-page vision ✓), `detected_figures_attach_to_text_first_question` (not read-required, 2 figures → text-first + attach ✓).

## Validation

1. `cd src-tauri && cargo test --lib -- --test-threads=1` (all green).
2. Manual A/B on `physics '24.pdf` with `MERGEMARK_LOG_USAGE=1`:
   - Sum `[TOKENS] prompt=` per call; expect read-required questions to show ~4k-token crops instead of ~10k-token pages.
   - `[TEXT_FIRST]` should now cover the majority of questions; `[CROP_FIRST]` covers the read-required few; the residual full-page vision path should be near-zero for digital papers.
   - Compare `ImportReport`: `diagramsSaved` flat vs today, `figuresDetected` > 0, `cropRejections` flat.

## Out of scope (unchanged)

- Local vision model (separate project).
- Mark-scheme import detection/crop-first.
- `extract_same_page_batch` / `extract_fallback_page` paths (still full-page vision; only `extract_span` gains crop-first).
