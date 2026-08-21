# Plan: Vision image cost optimisation — per-image `detail` policy + resolution cap

## Goal

Reduce the ~$0.07/40-page-paper ingestion cost without sacrificing extraction quality, by:

1. **Threading an explicit `detail` (low/high) choice through every vision API call** instead of the current hardcoded `"detail": "high"` in `llm::chat_body`.
2. **Capping the pixel resolution** of the two call sites that currently send full ~1158px 140-DPI page renders (vision structure pass, mark-scheme sliding windows) down to ≤768px, matching what the extraction path already sends.

User-confirmed decisions:
- **Scope:** both the detail knob AND the resolution cap (the cap is what actually moves cost on the default Gemini model, which ignores `detail`).
- **Detail policy:** band crops → `low`; any call containing a full page → `high`.

## Context (from codebase deep dive)

- `llm::chat_body` (src-tauri/src/llm.rs:68-152) hardcodes `"detail": "high"` for every image (line 107-113). Gemini/Anthropic ignore the field entirely; OpenAI-style providers honour it and price image tokens via 512px tiles (high) vs a single 512px tile (low). So the field is a free win on OpenAI-compatible models and a no-op on the default `google/gemini-2.5-flash`.
- For Gemini/Claude, image cost scales with **pixels**. The extraction path (`prepare_chunk_images`, pipeline.rs:307-397 and `geometry::crop_page_vertical_from_image`, geometry.rs:1182-1241) already downscales every API-bound image to ≤768px (WebP). Two call sites do **not**:
  - Vision structure pass — `pipeline.rs:1188-1199` pushes the raw `page.kind` b64 (140-DPI ~1158px JPEG).
  - Mark-scheme windows — `pipeline.rs:4113-4116` pushes raw page b64s (~1158px).
- `chat_body` call sites (all in pipeline.rs): structure pass (1206), `extract_span` main (1999), `extract_span` reduced EOF retry (2072), `extract_same_page_batch` (3097), `extract_fallback_page` (3812), `read_markscheme_window` (4163).
- Band-crop vs full-page is deterministically known per image from `PreparedChunk.page_crop_offsets` (parallel to `images`; a crop sets the offset away from the `(0.0, 1.0)` default).
- Existing unit tests use text-only pages (`pipeline.rs:4449-4456`), so the image paths are mostly covered by tests that pass tiny PNG b64s (`grid_page`/`chart_page`, pipeline.rs:4748-4759) and do not assert on `detail`. The 6 `chat_body` calls in `llm.rs` tests (434-470) will need the new argument.

## Design

### 1. `ImageDetail` enum + `chat_body` signature (src-tauri/src/llm.rs)

Add a small enum near the top of llm.rs:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageDetail {
    Low,
    High,
}
impl ImageDetail {
    pub fn as_str(&self) -> &'static str {
        match self { ImageDetail::Low => "low", ImageDetail::High => "high" }
    }
}
```

Change `chat_body` to accept the detail for the whole call and emit it per image:

```rust
pub fn chat_body<S: AsRef<str>>(
    model: &str,
    system: &str,
    images: &[S],
    detail: ImageDetail,
    text: Option<&str>,
    max_tokens: u32,
    response_format: Option<ResponseFormat>,
) -> serde_json::Value
```

- Replace `"detail": "high"` (llm.rs:111) with `"detail": detail.as_str()`.
- Update the stale comment (llm.rs:101-106) that describes a "~200 DPI render / 2048-px long edge" — the actual default render is 140 DPI and API images are capped at 768px.
- Sentinel filtering (`__SKIP__`, `TEXT_ONLY`, etc.) is unchanged and still happens before the `detail` is attached.

### 2. Resolution-cap helper (src-tauri/src/geometry.rs)

Add a shared constant and two helpers. Keep output format identical to the existing paths (raw base64 WebP, no `data:` prefix — `chat_body` already wraps such values as `data:image/webp;base64,`):

```rust
pub const API_IMAGE_MAX_DIM: u32 = 768;

/// Downscale so the longest edge <= max_dim, then WebP-encode.
/// Returns raw base64 (no data-URL prefix). None on encode failure.
pub fn encode_webp_resized(img: &image::DynamicImage, max_dim: u32) -> Option<String>;

/// Decode (data-URL or raw) + encode_webp_resized. None if undecodable/encode fails.
pub fn resize_b64_to_max_dim(b64: &str, max_dim: u32) -> Option<String>;
```

- `encode_webp_resized` mirrors the resize+WebP logic already in `prepare_chunk_images` (pipeline.rs:345-365) and `crop_page_vertical_from_image` (geometry.rs:1210-1231). Optionally refactor those two to call the new helper to eliminate duplication (low risk, same output format).
- `resize_b64_to_max_dim` decodes via the existing `geometry::decode_page_image` (geometry.rs:1138-1144), which strips any `data:` prefix.

### 3. Call-site threading (src-tauri/src/pipeline.rs)

| Site | Images sent | Detail |
|---|---|---|
| Structure pass (1206) | full page, **now ≤768px** | `High` |
| `extract_span` main (1999) | band crops and/or full pages (≤768px) | `Low` iff **all** images in the chunk are real band crops, else `High` |
| `extract_span` reduced EOF retry (2072) | first `reduced_image_count` images | `Low` iff all of those are band crops, else `High` |
| `extract_same_page_batch` (3097) | full page (≤768px) | `High` |
| `extract_fallback_page` (3812) | full page (≤768px) | `High` |
| `read_markscheme_window` (4163) | full pages, **now ≤768px** | `High` |

Band-crop detection uses `page_crop_offsets` (parallel to `images`): an image is a genuine band crop iff its offset is not the default `(0.0, 1.0)`:

```rust
let all_band_crops = !page_crop_offsets.is_empty()
    && page_crop_offsets.iter().all(|(s, e)| *s != 0.0 || *e != 1.0);
let detail = if all_band_crops { ImageDetail::Low } else { ImageDetail::High };
```

- Main call: compute from the in-scope `page_crop_offsets` immediately before the `chat_body` call at 1999.
- Reduced call: compute from `page_crop_offsets[..reduced_image_count]` before building `reduced_body` at 2072 (the truncation at 2115-2118 happens after the body is built; the slice is safe to read at that point).
- Same-page batch / fallback / structure pass / mark-scheme: pass `ImageDetail::High` directly.

### 4. Structure pass downscale (pipeline.rs:1188-1199)

The structure pass already decodes the page image for blank detection. Reuse that decode:

```rust
if let PageInputKind::Image { b64, .. } = &page.kind {
    if let Some(decoded) = geometry::decode_page_image(b64) {
        if geometry::is_image_blank(&decoded) { /* existing BLANK early-return */ }
        images.push(
            geometry::encode_webp_resized(&decoded, geometry::API_IMAGE_MAX_DIM)
                .unwrap_or_else(|| b64.clone()),
        );
    } else {
        images.push(b64.clone());
    }
}
```

### 5. Mark-scheme windows downscale (pipeline.rs:4113-4116)

```rust
let images: Vec<String> = pages[start..end]
    .iter()
    .filter_map(|p| p.get_b64().cloned())
    .map(|b64| geometry::resize_b64_to_max_dim(&b64, geometry::API_IMAGE_MAX_DIM).unwrap_or(b64))
    .collect();
```

### 6. Optional env overrides

Add two optional, codebase-consistent knobs (read once at call sites; do not read env inside `chat_body` so tests stay deterministic):
- `MERGEMARK_VISION_DETAIL=low|high` — force all vision calls to one detail (escape hatch if a user sees a quality regression and prefers max savings/quality).
- `MERGEMARK_VISION_MAX_DIM=<px>` — override `API_IMAGE_MAX_DIM` (default 768) for the resolution cap.

These are optional. Core behaviour (section 3-5 defaults) does not depend on them.

### 7. Validation instrumentation (small, env-gated)

`usage_from_response` (llm.rs:182-196) is currently `#[allow(dead_code)]`. To make the before/after token comparison measurable without DB plumbing, add an env-gated one-liner in `chat_with_permit` (pipeline.rs:178-209):

```rust
if std::env::var_os("MERGEMARK_LOG_USAGE").is_some() {
    let u = crate::llm::usage_from_response(&resp);
    eprintln!("[TOKENS] prompt={} completion={} total={}", u.prompt_tokens, u.completion_tokens, u.total_tokens);
}
```

This lets an A/B run with `MERGEMARK_LOG_USAGE=1` sum prompt tokens per import before/after.

## Implementation steps (ordered)

1. **llm.rs** — add `ImageDetail` enum + `as_str`; extend `chat_body` signature with `detail`; emit `detail.as_str()`; fix the stale comment; update the 6 test calls in `mod tests` to pass `ImageDetail::High` (or Low) and add new assertions (below).
2. **geometry.rs** — add `API_IMAGE_MAX_DIM`, `encode_webp_resized`, `resize_b64_to_max_dim`; optionally dedupe `prepare_chunk_images` / `crop_page_vertical_from_image` to use `encode_webp_resized`.
3. **pipeline.rs structure pass** — reuse the blank-check decode to push a ≤768px WebP render.
4. **pipeline.rs extract_span** — compute `all_band_crops` detail for the main call (1999) and the reduced call (2072).
5. **pipeline.rs same-page batch + fallback** — pass `ImageDetail::High`.
6. **pipeline.rs mark-scheme windows** — downscale each page b64 to ≤768px, pass `ImageDetail::High`.
7. **chat_with_permit** — add the env-gated `[TOKENS]` usage log (validation aid).
8. **Tests** (see next section).
9. **cargo test + manual A/B validation** (see validation section).

## Tests

- **llm.rs**
  - Update all 6 existing `chat_body` test calls for the new signature.
  - `chat_body_emits_low_detail` — assert `body["messages"][1]["content"][0]["image_url"]["detail"] == "low"`.
  - `chat_body_emits_high_detail` — assert `"high"`.
  - Keep the existing data-URL MIME preservation and reasoning-effort tests green.

- **geometry.rs** (new tests)
  - `encode_webp_resized_caps_long_edge` — build a ~1600×2000 image, encode with max_dim 768, decode the result and assert longest edge ≤768.
  - `resize_b64_to_max_dim_handles_data_url` — pass a `data:image/png;base64,...` of a large image, assert the returned value decodes to ≤768px and is raw base64 (no prefix).

- **pipeline.rs**
  - `band_crop_requested_at_low_detail` — construct a PageInput with a large PNG (like existing `png_b64` helper), a span with `start_y_frac`/`end_y_frac`, run `extract_span` with `MockLlm`, assert `mock.bodies()[0]` image detail is `"low"`.
  - `full_page_requested_at_high_detail` — same setup without y-bands, assert `"high"`.
  - `markscheme_window_sends_downscaled_images` — run `run_markscheme_pipeline` with image pages, decode the first image from `mock.bodies()[0]`, assert longest edge ≤768 (guards the cap).
  - Existing tests stay green (they don't assert on `detail`).

## Validation

1. `cd src-tauri && cargo test` — full unit suite (this repo's documented verification, per docs/INGESTION_RELIABILITY_PLAN.md).
2. Manual A/B on a real 40-page paper (e.g. `physics '21.pdf` in the repo root) against the current default model:
   - Run one import before the change and one after, with `MERGEMARK_LOG_USAGE=1`.
   - Compare summed `[TOKENS] prompt=` lines (expected: structure-pass/mark-scheme pages fall from ~1158px to ~768px ≈ 2.3x fewer pixels; OpenAI-model users additionally see band crops go `high`→`low`).
   - Compare `ImportReport` quality signals: `questions_extracted`, `quarantined.len()`, `repairs` — must be flat.
3. If a quality regression appears on any paper, use `MERGEMARK_VISION_DETAIL=high` (or raise `MERGEMARK_VISION_MAX_DIM`) to confirm the cause and tune.

## Risks

- **Structure pass at 768px on scanned PDFs** — the structure pass is the fallback when the text layer is corrupt; small printed question numbers could theoretically be harder to read. Mitigation: the same 768px ceiling is already what extraction uses successfully for full transcription; if a specific scan regresses, `MERGEMARK_VISION_MAX_DIM` (or `MERGEMARK_VISION_DETAIL`) is the escape hatch.
- **Mark-scheme windows re-decode/encode overlapping pages** (step=2 windows overlap by 1 page) — a few extra ms of CPU per import, negligible vs API cost.
- **Signature change churn** — only 6 production call sites + 6 test calls touch `chat_body`; no external callers (commands.rs goes through the pipeline).
- **Env override determinism** — env vars are read at call sites, never inside `chat_body`, so unit tests are unaffected by the ambient environment.

## Out of scope (deliberately not in this plan)

- Input-side image-blob caching across questions sharing a page (recommendation #2) — the band crops are already computed once per question; deduping identical blobs is a separate change.
- Mark-scheme text-layer-first path (recommendation #3).
- Raising `MERGEMARK_PARALLELISM` defaults (recommendation #4).
- Lowering the default `MERGEMARK_RENDER_DPI` (recommendation #5).
- WebP for the full-page render path (recommendation #6) and per-page extraction cache (recommendation #7) — noted for follow-up.
