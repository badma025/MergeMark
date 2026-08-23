# Phase 1 — Smart Scissors: Typed-Stroke Figure Segmentation

Replace the anonymous-bbox clustering in figure detection with a typed stroke census, barrier-aware region growing, and label capture. Pure Rust/pdfium — zero AI calls. Detection-only scope.

## Locked decisions

1. **Full replacement** — `stroke_census.rs` becomes the implementation behind `detect_pdf_figures`; legacy `cluster_boxes` path deleted. Safety = golden IoU fixtures.
2. **`DetectedFigure` gains `seg_confidence: f32`** now (recorded, no consumer yet). Update ~6 pipeline test literals.
3. **Ground truth = curated paper goldens** (debug-dump → human verify → committed JSON; CI asserts count + IoU ≥ 0.85 + barrier non-overlap). Plus ordinary pure-function unit tests on synthetic stroke data (no synthetic PDFs).
4. **Detection-only boundary** — no `pipeline.rs` behaviour changes beyond test literals; no Tier-0 gating; no escalation hooks. Rotation normalization **in**; XObjectForm recursion **out** (pdfium-render 0.9.3 exposes only outer form bounds).

## Verified pdfium-render 0.9.3 API (from local crate source)

- `PdfPageObject::{Text, Path, Image, Shading, XObjectForm, Unsupported}` — `object.rs:306`
- `PdfPagePathObject::segments()` → `PdfPathSegment { .segment_type() → MoveTo|LineTo|BezierTo|Unknown, .point() → (PdfPoints, PdfPoints), .is_close() }`; segment points already include the object matrix (page space) — `object/path.rs:882`, `path/segment.rs:65-112`
- `PdfPagePathObject::is_stroked()` / `is_filled()` — `object/path.rs:835`
- `PdfPage::rotation()` → `PdfPageRenderRotation` — `page.rs:292`
- Text segments + bounds already used in `detect_page_figures_inner`
- Limitation: `XObjectForm` children not enumerable → treat forms as opaque seed boxes; log when one contributes a figure.

## Coordinate system

Work in **PDF point space** (bottom-left origin) throughout the census. Convert once at emission via existing `geometry::normalize_pdf_box(left, right, top, bottom, page_w, page_h)` → `[x, y, w, h]` y-from-top. For rotated pages: if `page.rotation()` is 90/270, swap effective page dims and rotate object rects into rendered space **before** normalization (must match how `render_page_from_document` renders, since crops are cut from renders).

## Module layout

- **New** `src-tauri/src/stroke_census.rs` — census, classification, barriers, growing, label capture, scoring. Register `mod stroke_census;` in `lib.rs`.
- **Extend** `geometry.rs` — point-space rect ops (`RectPt` = `[f32;4]` l,r,t,b in pt), projection histogram + parallel-family detector, `corridor_blocked` sampler. Keep existing helpers (`is_probable_figure_box`, `text_density_in_box`, `is_rule_line`, `is_tiny_figure`, `matches_caption`, `nearest_caption`, `caption_kind_from_text`).
- **Rewire** `pdf_render.rs` — `detect_page_figures_inner` delegates to `stroke_census::detect_page_figures(page)`. Delete `cluster_boxes` + its unit tests (sole production call site is here). Delete the raw-bounds collection block it replaced.
- **Extend** `DetectedFigure` (`pdf_render.rs:230`) with `seg_confidence: f32`; fix literals in `pipeline.rs` tests (`~7641`, `~7762`, others surfaced by compiler).

## Pipeline (per page)

### 1. Stroke census & classification
Iterate `page.objects()`; for `Path` objects read `segments()` and build:

```rust
struct StrokeTelemetry {
    bbox_pt: RectPt,            // from obj.bounds()
    seg_count: u32,
    line_count: u32, bezier_count: u32,
    ink_len_pt: f32,            // Σ chord lengths within subpaths (reset at MoveTo)
    dir_hist: [f32; 8],         // LineTo delta angles, 45° bins
    closed: bool,               // any is_close()
    stroked: bool, filled: bool,
}
enum Prim { Bitmap, Rule, GridMember, Curve, Glyph }
```

Classification (ordered):
- `Rule`: stroked, ≤4 line segs, axis-aligned dominance > 0.9 (hist mass in the two horizontal or two vertical bins), min/max side < 0.05
- `Curve`: `bezier_count > 0` OR (≥3 segs AND direction entropy > 1.2)
- `Glyph`: area_frac < 0.0015 AND ink_len < 14pt; default fallback for other small marks
- `GridMember`: assigned in step 2 (never seeds)
- `Bitmap`: `PdfPageObject::Image` bounds; `XObjectForm` → treated as Bitmap-strength seed, counted in a `form_seeds` counter

### 2. Structural family detection (projection histograms)
- Bin x-centers of vertical Rules (h > 2% page, w < 0.5%); peaks with ≥4 members and spacing CV < 0.15 → retype as `GridMember`. Same for horizontal.
- **Answer-line stacks**: horizontal Rules with width ∈ [0.45, 0.95] × column width (column width = max body-block width, fallback median rule width), ≥3 members at pitch 18–30pt → emit barrier rect (stack union, padded 4pt).

### 3. Text-block assembly & barriers
- Assemble text segments → lines (same baseline ±1.5pt, gap < 1.5× char height) → blocks (vertical gap < 0.6× line height, horizontal overlap > 50%).
- Classify: `Caption` (existing `matches_caption`), `Heading` (reuse `doc_map`'s heading regex — make it `pub(crate)` if private), `Body` (≥8 words or char density above const), else `LabelCandidate`.
- Barriers = header band (top 5%) + footer band (bottom 8%) + Body blocks (+2pt) + answer-line stacks (+4pt) + Heading blocks (+2pt).
- `corridor_blocked(a, b)`: sample center-to-center segment every 6pt; blocked if any sample ∈ expanded barrier rect.

### 4. Seed-first region growing
- Seeds: Bitmap (strength 3) > Curve (2) > Glyph (1). Rules/GridMembers never seed.
- Sort seeds by (strength desc, area desc); each seed starts a region unless within 6pt of an existing region with clear corridor → absorb.
- Decoration pass: every Rule/GridMember/unassigned Glyph joins the nearest region within 6pt **iff** corridor clear (this is what keeps axes attached to their graph and arrowheads attached to shafts, while answer-line stacks — themselves barriers — never merge).
- Regions with zero seeds are discarded (decoration chains die).

### 5. Label capture, refinement, scoring
- `LabelCandidate`/`Caption` blocks whose center lies inside a region hull → capture; expand region bbox to include them (+3pt), clamping each edge at the first barrier met.
- Plausibility gates (existing): `MAX_FIGURE_AREA_FRAC` 0.5, header/footer band rejection, min area 0.003, min dim 0.015, aspect 8.0 — via `is_probable_figure_box`, but compute `text_density_in_box` **excluding captured internal labels** (labels legitimately sit inside figures).
- `seg_confidence` = clamp(seed_strength: Bitmap 0.5 / Curve 0.35 / Glyph 0.2 + 0.15·has_grid_family + 0.10·min(labels,6)/6 + 0.15·has_caption + 0.10·ink_density_normality, 0, 1).
- Emit via `normalize_pdf_box`; attach caption via existing `nearest_caption`, kind via `caption_kind_from_text`.

## Debug dump & goldens

- `MERGEMARK_FIGURE_DEBUG_JSON=<path>`: `detect_pdf_figures` (wrapper only) additionally writes `{ file, pages: [{ index, figures: [{bbox, caption, kind, seg_confidence}] }] }`.
- Goldens: `src-tauri/tests/fixtures/figure_golden/{physics21,physics24,edexcel_maths_pack}.json` — schema `{ source_pdf, pages: { "<idx>": [{ "bbox": [x,y,w,h], "caption": "Figure N" }] } }`.
- Golden test (fixture-gated skip pattern like `detect_figures_on_real_fixture`): for each page — detected count == golden count; greedy IoU matching ≥ 0.85 per pair; every detected box satisfies the density gate (labels excluded).

## Unit tests (pure, no PDFs)

Classifier on synthetic telemetry; family detection (grid family yes/no, answer-line stack yes/no); `corridor_blocked`; grower scenarios: arrowhead 2pt from shaft joins, axis 3pt from curve hull joins, answer-line stack blocks a merge, two side-by-side graphs stay separate, decoration-only cluster dies; label expansion clamped at barrier.

## Constants (module-level, single source)

`JOIN_GAP_PT = 6.0`, `CORRIDOR_SAMPLE_PT = 6.0`, `GRID_MIN_MEMBERS = 4`, `GRID_SPACING_CV_MAX = 0.15`, `ANSWER_LINE_PITCH_PT = 18.0..=30.0`, `ANSWER_LINE_MIN_COUNT = 3`, `BODY_MIN_WORDS = 8`, `LABEL_PAD_PT = 3.0`, `BARRIER_PAD_PT = 2.0`, `STACK_PAD_PT = 4.0`.

## Edge cases

| Case | Handling |
|---|---|
| Rotated page | Rotate rects + swap effective dims pre-normalization (match render) |
| XObjectForm figure | Opaque Bitmap-strength seed; counter logged |
| Shading objects | Ignored (rare in board papers); noted in code comment |
| Scanned page (no text objects) | No barriers → growth is gap-based; image-only pages = one Bitmap seed; downstream `is_blank_or_grid` guard unchanged |
| Two graphs side-by-side | Corridor crosses inter-column body-text barrier → stay separate |
| Table with populated cells | Rule-grid family + Body blocks, no Curve/Bitmap seed → rejected |

## Task order

1. `geometry.rs`: RectPt ops, projection histogram + family detector, `corridor_blocked` (+ unit tests)
2. `stroke_census.rs`: telemetry + classification (+ unit tests)
3. `stroke_census.rs`: block assembly + classification + barriers (+ unit tests)
4. `stroke_census.rs`: region growing + decoration pass (+ unit tests)
5. `stroke_census.rs`: label capture, refinement, scoring, emission
6. `pdf_render.rs`: delegate `detect_page_figures_inner`; rotation normalization; extend `DetectedFigure`; delete `cluster_boxes` path; fix test literals
7. Debug dump env var; generate candidate boxes for 3 fixtures → user curates → commit goldens
8. Golden integration test; full `cargo test`; manual import of one physics paper via the app; check ImportReport figure counts/dedupe metrics

## Validation

- `cargo test` green (existing suite + new units + goldens) on the dev machine (pdfium.dll present in `src-tauri/`); fixture-gated tests skip cleanly without it
- Golden IoU ≥ 0.85 and count-exact on all three fixtures
- Manual: import `physics '21.pdf`; ImportReport shows equal-or-fewer repairs, `diagrams_deduped` stable, no new quarantines

## Out of scope (later phases)

XObjectForm recursion; `TargetedVisionUnit` emission; Tier-0 gate consuming `seg_confidence`; pack builder; embedded-bitmap passthrough (`FigureSource::Embedded`).
