# Plan: Speed up PDF ingestion (figure papers ~40s → ~12-15s)

## Goal
Cut ingestion wall-time for figure-bearing PDFs from ~40s to ~12-15s (API-bound) with **zero quality change** — crop resolution and output identical. The bottleneck is local PDF rendering, not API calls.

## Measured problem
For physics '24 (29 text-first / 3 vision, ~40s total, API calls only ~5-8s):
- `PageRenderCache::get_or_render` (pdf_render.rs:39) renders figure-crop pages at **300 DPI (2480px)** via `render_pdf_page_at_300dpi` (pdf_render.rs:363), which calls `pdfium.load_pdf_from_file(path)` on **every page render** — re-parsing the whole 40-page PDF each time.
- ~20 distinct figure pages (Q2, Q3, Q5, Q6, Q7, Q14, Q23, Q26, Q30) → **~20 full PDF parses + 2480px renders**.
- The state `Mutex` (pdf_render.rs:44) is held **during** the render → all parallel figure-attach jobs serialize.
- Cache capacity is **4** (`PAGE_RENDER_CACHE_CAPACITY`, pipeline.rs:215) → 20 pages churn through 4 slots → evictions → re-renders.
- Renders run synchronously on the async executor (crop-first pipeline.rs:2750, persist helper pipeline.rs:5146-5150).

Result: ~20 × (0.5-2s parse + render) ≈ 14-40s of local CPU, serialized.

## Changes

### 1. PageRenderCache: load the PDF document once (pdf_render.rs) — the core fix
- Add a `document: Mutex<Option<(PathBuf, Arc<PdfDocument>)>>` field, lazily loaded on first `get_or_render`.
- New `fn render_page_from_document(doc: &PdfDocument, page_idx: usize, dpi: u32) -> Result<image::DynamicImage, String>` (the body of `render_pdf_page_at_300dpi` minus the per-call `load_pdf_from_file`).
- Rewrite `get_or_render`:
  1. Lock state briefly → cache hit: touch LRU, return (unchanged fast path).
  2. Load/obtain `Arc<PdfDocument>` (lock `document` only for the load).
  3. Render the page **outside the state lock** (shared `&PdfDocument`, so parallel jobs no longer serialize).
  4. Re-lock state, double-check the cache (another job may have won the race), insert, touch LRU.
- Verify `PdfDocument: Send + Sync` compiles (pdfium-render 0.9.3 advertises thread-safety). If it does not, fall back to serialized renders inside the lock — load-once still captures ~80% of the win.
- Keep `render_pdf_page_at_300dpi` as a thin delegate (`get_pdfium()?.load_pdf_from_file(...)` + `render_page_from_document`) so any external/test callers are unaffected (its only current caller is `get_or_render`).

### 2. Raise the render cache capacity 4 → 32 (pipeline.rs:215)
The ~20 figure pages must stay resident — no eviction churn. 32 covers the largest multi-page questions plus headroom; memory cost is ~20 × 30MB at 300 DPI, acceptable for an import; consider 16 if memory is a concern.

### 3. DPI knob, default unchanged (pdf_render.rs)
Keep 300 DPI by default (zero quality change). Add `MERGEMARK_FIGURE_RENDER_DPI` env override (u32, clamped 96-300) applied in `render_page_from_document` (`set_target_width((8.27 * dpi).round() as u16)`). Users can opt into 150 DPI (~4× faster renders) without a code change; the default preserves today's crop legibility.

### 4. Move the figure-crop render loops off the async executor (pipeline.rs)
Wrap the render+crop loops in `tokio::task::spawn_blocking` at the two heavy call sites:
- `try_crop_first_extraction` (pipeline.rs:2750) — the crop loop.
- `attach_detected_figures`' save helper / `persist_diagrams` (pipeline.rs:5146-5150) — the per-request get_or_render + crop.
Pattern to mirror: `prepare_chunk_images` (pipeline.rs:394) already does `spawn_blocking` + rayon. This stops a render from stalling an async worker mid-extraction stream. Low priority if Task 1 already makes renders fast — keep it simple.

### 5. Run text extraction + figure detection concurrently (commands.rs:1278, 1527-1540)
`extract_page_texts` (pdf_extract, spawn_blocking) and `detect_pdf_figures` (pdfium, spawn_blocking) are independent one-time PDF reads currently run sequentially. Launch both up-front and `tokio::join!` them. Saves ~1-3s.

### 6. Per-stage timing summary in import logs (pipeline.rs)
Print a `[TIMING]` line at the end of `run_question_pipeline` (near the `[PATH_SUMMARY]` at pipeline.rs:1952): `document_map`, `structure`, `extraction span_stream`, `extraction fallback_stream`, `total_elapsed_ms` from `report.timings`/`total_elapsed_ms`. Without it the speedup is unmeasurable in import logs.

## Tests
- **Cache-reuse regression** (fixture-gated, like `detect_figures_on_real_fixture`): instrument `PageRenderCache` with a `load_count: AtomicUsize` (incremented on document load). Call `get_or_render` for pages 0 and 1; assert 2 image results and `load_count == 1`. Skips cleanly when pdfium/fixture absent.
- **Render-from-document smoke**: `render_page_from_document` returns a `DynamicImage` with nonzero dims on a real fixture page (skips when unavailable).
- **DPI knob**: `render_page_from_document(doc, i, 300).dimensions()` > `(doc, i, 150).dimensions()` on a fixture.
- Existing suites must stay green: all 176 lib tests (figure-attach, crop-first, vision tests exercise these paths), `detect_figures_on_real_fixture`.

## Validation protocol (how the speedup is proven)
1. `cd src-tauri && cargo test --lib -- --test-threads=1` (all tests, new + existing).
2. `cargo check --all-targets` (no warnings); `npx tsc --noEmit` (frontend untouched, but run for safety).
3. Import `physics '24.pdf`; compare against baseline:
   - `[TIMING]` shows `document_map`/`structure` near-zero and `extraction span_stream` dominant.
   - Wall time ~40s → **~12-15s**.
   - `[PATH_SUMMARY]` identical (29 text-first / 0 crop-first / 3 full-page vision) and all 32 questions present.
   - Spot-check Q2's saved figure crop renders identically (300 DPI default) — save one crop before/after and compare dimensions/hash.
4. If wall time is still >20s, re-check Task 1's fallback path (document not shared / renders still serialized) and Task 4.

## Out of scope / risks
- **Frontend page rendering** (the browser renders `pdf_base64_pages` before the Rust call) is outside this plan; if the frontend render is slow it is a separate Tauri/JS task.
- **Switching crops to the frontend base64 images** instead of pdfium rendering — rejected: changes crop resolution/quality characteristics; the load-once fix keeps pixels identical.
- Memory: 300-DPI page images are ~30MB each; capacity 32 caps worst case ~1GB. If memory pressure appears, drop Task 2 to 16 (covers the fixture's ~20 pages with minor re-render) — do not go below 12.
- pdfium thread-safety: if `PdfDocument` is not `Sync`, renders stay serialized (Task 1 fallback) — still removes the 20× re-parse.
