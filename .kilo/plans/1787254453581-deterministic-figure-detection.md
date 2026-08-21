# Plan: Deterministic Figure Detection from the PDF Content Stream (free figure boxing)

## Goal

Make figure detection/boxing **effectively free** while keeping it accurate, on the existing `google/gemini-2.5-flash` stack, by reading the PDF's own vector content instead of asking the vision model to find figures in a raster. Target: a digital AQA paper imports with text-first transcription + deterministic figure crops and **no vision call for the common case** — only questions that genuinely need to *read* figure content fall back to vision (and then only the small cropped region).

## Why this works (verified against the codebase)

- `pdf_render.rs:107-109` already walks `page.objects()` and inspects `PdfPageObjectType::Image` / `::Path`. **pdfium gives exact bounds for every object** (`PdfPageObject::bounds() -> PdfQuadPoints` with `left/right/top/bottom`; `PdfPage::width()/height()` in points). Figure regions are computable deterministically, on-device, zero AI calls.
- Text is a separate object type (`PdfPageObjectType::Text`) and `page.text().segments()` exposes per-segment `bounds()` + `text()` — so "Figure 1" captions can be matched to their actual figure boxes.
- The pipeline already has a rigorous **deterministic guard chain** the proposed boxes can be pushed through unchanged: `audit_diagram_boxes` (pipeline.rs:3860) rejects degenerate / out-of-page / boilerplate / answer-space / duplicate boxes, and `crop_diagram_with_options` (geometry.rs:900) + `sanitize_bbox` (geometry.rs:391) apply footer/header/margin guards, grid rejection, and answer-space detection at save time. **The detector proposes; the existing Rust guards dispose** — same trust model as today's vision boxes, minus the vision cost.
- `save_diagram` (pipeline.rs:4088) already crops from the 300-DPI `PageRenderCache` and dedups via `tile_signature` — the deterministic boxes plug straight into it.

## Design

### 1. Deterministic detector — `src-tauri/src/pdf_render.rs`

New API (mirrors existing render helpers):

```rust
/// A figure region detected from the PDF content stream, in normalized
/// 0.0–1.0 page coordinates: [x, y, w, h] (y from top, matching the
/// vision schema).
pub struct DetectedFigure {
    pub bbox: [f32; 4],
    /// "Figure 1" caption text if a matching text segment was found.
    pub caption: Option<String>,
    /// Semantic kind inferred from the caption ("graph", "circuit", …).
    pub kind: Option<String>,
}

pub fn detect_page_figures(path: &Path, page_idx: usize)
    -> Result<Vec<DetectedFigure>, String>;
```

Algorithm:
1. `page.size()` → page width/height in points.
2. Iterate `page.objects()`; for every object of type `Image` or `Path`, get `bounds()` → axis-aligned `left/right/top/bottom` → normalize to 0–1 (remember PDF origin is bottom-left: `y_top_normalized = 1.0 - top/height`, `h = (top-bottom)/height`).
3. **Cluster** objects into figure regions (greedy union by overlap/gap tolerance, e.g. boxes whose 1-D projections overlap after expanding by ~2% of page) — a circuit or graph is usually many paths.
4. **Filter** (this is the accuracy core):
   - reject boxes smaller than a floor (e.g. area < 0.3% of page or min dimension < 1.5%) — removes underlines, tick marks, borders;
   - reject very thin boxes (aspect ratio > ~8:1) — ruled answer lines / continuation rules;
   - reject boxes whose region is **text-dense** (count text-segment bounds intersecting the region; if a large share of the region is covered by `Text` objects, it's question text/table, not a figure);
   - reject boxes in the header (top 5%) / footer (bottom 8%) bands — matches existing `crop_diagram_reading` guards.
5. **Caption association**: scan `page.text().segments()`; find segments matching `Figure\s+\d+` (or `Fig.?\s*\d+`); for each, pick the detected box whose center is nearest; store the caption. Infer `kind` from the caption text with a small keyword map ("graph"/"chart"/"plot" → graph, "circuit" → circuit, "flow" → flowchart, "diagram"/"sketch" → diagram, fallback None).

Pure functions (clustering/filtering) go in `geometry.rs` so they are unit-testable without pdfium; `detect_page_figures` is a thin pdfium wrapper.

### 2. Compute once per import — `commands.rs` + `run_question_pipeline`

- In the question-import command (`parse_pdf_vision`), after `render_pdf_pages`, run `detect_page_figures` for every page in a `spawn_blocking` pass (parallel with Rayon like the existing JPEG encode) → `Vec<Vec<DetectedFigure>>`.
- Thread it into `run_question_pipeline` → `extract_span` as a new `page_figures: &[Vec<DetectedFigure>]` argument (empty `vec![]` in tests keeps existing behaviour).
- Keep it **free at runtime**: one CPU pass, ~ms per page, no API cost.

### 3. Text-first for ALL questions + deterministic figure attach

Remove the figure gate so figure questions also transcribe from text:

- In `extract_span`, `text_first` now attempts regardless of `text_references_figure` (keep the gate only as a *vision-fallback trigger*, see §4).
- After a successful text-first transcription, **attach deterministic figures**:
  1. For each page in the span, take `page_figures[page]` boxes whose center-y lies within the span's band on that page (same band logic already used for vision: `start_y_frac`/`end_y_frac`; interior pages use full height).
  2. For each such figure, run it through the existing `audit_diagram_boxes`-style checks and `save_diagram` (300-DPI crop via `PageRenderCache`, signature dedup, guard chain) — reusing `persist_diagrams`.
  3. Replace `[DIAGRAM_PLACEHOLDER]` tokens in the transcribed content with the crop links (one placeholder per figure, in order); if the content references "Figure N", insert the link after that reference; append remaining figures at the end.
  4. Populate `diagram_captions`/`diagram_kinds` from the `DetectedFigure` metadata.
- Text-first prompt: tell the model it may emit `[DIAGRAM_PLACEHOLDER]` where the question references a figure (revert the "no placeholders" line), so link placement matches reading order.

### 4. Vision fallback (only when genuinely needed)

Fall back to the existing vision path per-question only when:
- the text-first attempt fails validation/empty/truncated, **or**
- the question's text signals the **answer must be read from the figure** (heuristic on "write down the value from", "read the coordinates", "state the value of the resistor", "what is the reading on", "use the graph to determine") AND fewer deterministic figures were found than figure references, **or**
- the deterministic detector found **no** figures on the span's pages but the text references one (safety net for unusual PDF encodings).

This keeps quality: genuinely figure-reading questions still get vision. Everything else is free. (A later phase can send *only* the cropped figure region at reduced size for those rare questions instead of the full page.)

### 5. Reporting

- Add `figures_detected: usize` (deterministic) to `ImportReport`; surface "Figures detected: N (free)" in the `IngestionDropzone` report card next to the existing `diagramsSaved`.

## Implementation steps (ordered)

1. **geometry.rs** — add pure helpers: `normalize_pdf_box(left,right,top,bottom,page_w,page_h) -> [f32;4]`, `cluster_boxes(Vec<[f32;4]>) -> Vec<[f32;4]>`, `is_rule_line`, `text_density_in_box` (uses text-segment rects), with unit tests.
2. **pdf_render.rs** — `DetectedFigure` + `detect_page_figures` using the pdfium APIs verified above (`page.objects()`, `obj.bounds()`, `page.text().segments()`, `page.width()/height()`). Guard against `bounds()` errors per-object (skip, don't fail the page).
3. **commands.rs** — run detection per page in parallel; pass `page_figures` into the pipeline.
4. **pipeline.rs** — thread `page_figures` through `run_question_pipeline` → `extract_span`; relax the `text_first` gate; implement figure attach after text-first (reuse `persist_diagrams` / `save_diagram` / `audit_diagram_boxes`); wire the vision-fallback triggers; add `figures_detected` to the report.
5. **IngestionDropzone.tsx** — display `figuresDetected`.
6. **Tests** (see below).
7. **Validation** (see below).

## Tests

- **geometry.rs**: normalize/cluster/filter unit tests (synthetic boxes: two overlapping paths → one figure; thin rule line rejected; tiny mark rejected).
- **pdf_render.rs**: on the repo's real fixture (`physics '24.pdf`, `physics '21.pdf`): assert detection returns non-empty figure boxes on pages that contain figures, empty on pure-text pages, and that every detected box passes `sanitize_bbox` after normalization.
- **pipeline.rs**: `detected_figures_attach_to_text_first_question` — feed `page_figures` with 2 boxes on the span page, mock text-first response with 2 `[DIAGRAM_PLACEHOLDER]`, assert 2 crops saved (`diagrams_saved == 2`) and links in content.
- **pipeline.rs**: `vision_fallback_when_figure_read_required` — text says "read the coordinates from the graph", detector returns 0 boxes → assert the vision path runs (body has image).
- **pipeline.rs**: existing 146 tests stay green (empty `page_figures` = old behaviour).

## Validation

1. `cd src-tauri && cargo test --lib -- --test-threads=1` (the parallel crash is a pre-existing harness issue; single-threaded is deterministic).
2. Manual A/B on `physics '24.pdf` with `MERGEMARK_LOG_USAGE=1`:
   - Sum `[TOKENS] prompt=` lines; expect the figure questions to drop their ~10k-token image payloads.
   - Compare `ImportReport`: `diagramsSaved` must stay ~flat vs today (figures still extracted), `figuresDetected` > 0, `quarantined`/`repairs` flat.
   - Visually inspect a few saved `.png` crops to confirm they frame the figure tightly (the existing `crop_diagram_with_options` padding handles this).
3. If any paper loses figures, the fallback triggers + `figuresDetected` counter isolate whether the detector missed them (tune clustering/filter thresholds) or the question genuinely needs vision.

## Risks & mitigations

- **Boxing question text/tables instead of figures** — mitigated by text-density filtering + the existing answer-space/boilerplate/blank guards that run before anything is saved.
- **Two adjacent figures merged into one box** — caption association disambiguates; if a single detected box contains two captions, keep it (same behaviour as today's compound-figure handling).
- **Non-vector figures (raster scans embedded as images)** — `Image` objects are included in detection, so embedded photos/bitmaps are still caught; if the whole PDF is a scan (no objects), detection returns empty and the vision fallback triggers as today.
- **QA calibration effort** — detection thresholds must be tuned against the repo's real AQA physics files (the repo already has `physics '21.pdf` / `'24.pdf` fixtures and a `doc_map` test harness calibrated on AQA '17–'24). This is the main engineering cost; the API surface is proven.
- **Caption/kinds loss vs today** — captions come from "Figure N" text segments (present in AQA papers); kinds from a keyword map. Slightly less rich than model-generated captions; acceptable for the cost win, and vision fallback covers the figure-reading cases.

## Out of scope (future)

- Sending only the cropped figure region (small image) for the rare vision-fallback questions instead of the full page — a follow-up after this lands.
- Detecting figures for the **mark-scheme** import (same detector applies; separate change).
- Raster/layout-model approaches (DocLayout-YOLO etc.) if content-stream detection proves insufficient on scanned inputs.
