# Plan: Text-Layer-First Extraction (the change that actually drives cost down)

## Why previous changes didn't move the number (honest accounting)

Your $0.067 import of physics '24 splits roughly as:

| Component | Share of bill | What we did | Effect |
|---|---|---|---|
| **Base extraction image tokens** | ~85% | pixel cap 768→640 | ~26% cut on that slice (~$0.01) |
| Repairs (re-sending images) | ~10% | convergence guard | Q2/Q3 went 3→2 sends, but still sent images |
| Output tokens | ~5% | none | none |

Net: $0.0664 → $0.067. The ~85% base image slice was never attacked. The only change that attacks it is **text-layer-first extraction**: for a digital AQA paper the text layer is already reliable enough that the vision *structure pass* is skipped (`text_map_available`), so the question text is in the PDF text layer — we can transcribe most questions with **zero image tokens** and fall back to vision only when the question actually needs a figure.

Expected outcome for a digital paper: pure-text questions cost ~1–2k text tokens (~$0.0003) instead of ~10k+ image tokens (~$0.001+). Paper import drops from ~$0.067 toward **~$0.01–0.02**. This is the single lever that moves the bill.

## Design

### 1. Config flag — `src-tauri/src/pipeline.rs`
Add to `PipelineConfig`:
```rust
/// Try extracting each question from the PDF text layer FIRST (zero image
/// tokens). Only falls back to vision when the text attempt needs a figure
/// or fails validation. Off by default (tests keep the old behaviour);
/// enabled by the production import command when the text map is sufficient.
pub text_first: bool,
```
Default `false` in `PipelineConfig::new` so every existing test is untouched.

### 2. Enable in production — `src-tauri/src/commands.rs` (question import command, ~line 1406)
After `config.max_output_tokens = 32768;` add:
```rust
// Text-layer-first extraction: the PDF text layer is authoritative for
// digital papers (the vision structure pass is skipped for them). Transcribe
// from text, send images only when a figure is actually needed. Override:
// MERGEMARK_TEXT_FIRST=0 disables.
config.text_first = std::env::var("MERGEMARK_TEXT_FIRST")
    .map(|v| v != "0")
    .unwrap_or(true);
```

### 3. Thread the flag into extraction — `run_question_pipeline`
`run_question_pipeline` already computes `text_map_available`. Gate on it so scanned/garbled PDFs (where the structure pass runs) never take the text-first path:
- Add `text_first: bool` parameter to `extract_span` (and the two fallback call sites at ~3195 and ~3453).
- In the job dispatch closure (~1670) pass `text_first = config.text_first && text_map_available`.
- The same-page-batch path (`extract_same_page_batch`) stays vision-only: MCQ pages are image-heavy and batching already amortises the call.

### 4. The text-first attempt — inside `extract_span`
Insert **before** the vision chunk loop (after the collateral-cache fast path):
- If `!text_first` → skip.
- If the span's combined text layer is trivial (all pages nearly empty) → skip (vision will handle).
- Build `user_text` exactly like the vision path (TARGET/PAPER/MODULE, band notes, raw text) but WITHOUT the image reference, and call `llm::chat_body(&config.model, &system, &[], detail, Some(&user_text), ...)` — `images = []` forces a text-only request.
- Parse with `parse_llm_json::<AiQuestionPage>`.
- **Acceptance gates** (all must pass, otherwise fall through to the existing vision loop):
  1. parse is `Clean` or `Salvaged{ dropped_tail: false }`,
  2. exactly one item for the target question number,
  3. `content` non-empty,
  4. no `[DIAGRAM_PLACEHOLDER]` in `content` (a figure is needed → vision),
  5. `diagram_bboxes` empty,
  6. `validate_span_items` returns no errors,
  7. content isn't suspiciously short for `span.expected_marks` (> 1 mark requires ≥ 5 chars, same rule as validation).
- On success, run the SAME post-processing the vision path uses (`assemble_built_question` with contents/topics/marks), record a `[TEXT_FIRST] Question N extracted from text layer (0 image tokens)` diagnostic, add a `text_first` counter to `ImportReport`, and `return (Some(built_q), report)`.
- On any failure: do **not** increment `repairs` (it wasn't a failed repair; it was a cheaper first attempt). Fall through to the existing vision chunk loop, which then runs with its full repair budget.

### 5. Report field — `ImportReport`
Add `pub text_first: usize` and surface it in the `IngestionDropzone.tsx` report card ("Text-first: N" count), so you can see how many questions avoided images per import.

## Tests (all in `src-tauri/src/pipeline.rs` tests module)

1. `text_first_extracts_from_text_with_zero_images` — one image page with rich text, a `MockLlm` response for the text call; assert the built question is returned AND the single recorded body has an **empty `images` array** (assert `body["messages"][1]["content"]` contains no `image_url`).
2. `text_first_falls_back_to_vision_when_figure_needed` — text layer references a figure; first mock returns content containing `[DIAGRAM_PLACEHOLDER]`, second mock returns the vision answer; assert 2 bodies and the final question built from the second call.
3. `text_first_disabled_keeps_vision_path` — `config.text_first = false`; assert body still sends the image (guards the default).
4. Existing tests untouched because `PipelineConfig::new` defaults `text_first: false` and the test `config()` helper doesn't set it.

## Validation

1. `cd src-tauri && cargo test --lib -- --test-threads=4` — full suite green. (The full suite crashes at *default* thread count even on unmodified base code — pre-existing pdfium/rayon harness instability — so the 4-thread workaround remains.)
2. Manual A/B on `physics '24.pdf` with `MERGEMARK_LOG_USAGE=1`:
   - Sum the `[TOKENS] prompt=` lines before/after.
   - Expect prompt tokens to drop ~70–85% (text-only questions eliminate the ~10k-token image payloads).
   - Compare `ImportReport`: `questionsExtracted`, `quarantined.len()`, `repairs` must stay flat.
3. If any specific question regresses, the report card's `repairReasons` + `textFirst` counts identify it; `MERGEMARK_TEXT_FIRST=0` restores the old behaviour immediately.

## Risks & mitigations

- **Math fidelity from the text layer** (superscripts/θ as glyphs). The extraction prompt's few-shot examples are already text-based (e.g. `"4 (a) Solve 2x^2 - 5x + 2 = 0."`), and every text-first result still passes the full `validate_span_items` + `assemble_built_question` gates. Any question the model flags (placeholder/empty/short) falls back to vision automatically.
- **Scanned papers**: gated off entirely by `text_map_available == false`, so behaviour is unchanged for them.
- **One extra cheap call on fallback**: a text attempt that falls back costs ~1–2k tokens extra (~$0.0002) per figure-heavy question — negligible vs the ~$0.001+ image payload it replaces, and far cheaper than the current always-vision behaviour.
