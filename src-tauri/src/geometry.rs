// ── Deterministic diagram geometry ─────────────────────────────────────────
//
// The AI proposes bounding boxes; this module is the single authority that
// turns a proposal into a crop. It:
//   * accepts relative (0..1), percent (0..100), or absolute pixel values
//     and auto-detects which scale was used,
//   * tolerates both [x, y, w, h] and [x1, y1, x2, y2] semantics,
//   * clamps into image bounds with saturating math (never panics, never
//     underflows u32),
//   * rejects implausible boxes (degenerate specks, ~full-page misdetections),
// so the crash class and the wrong-crop class are retired regardless of what
// the model emits.

/// A pixel-space rectangle guaranteed to lie fully inside the source image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Caption location hint for crop expansion.
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub struct CaptionHint {
    /// Relative coordinates of caption text (0.0-1.0)
    pub x: f32,
    pub y: f32,
    /// Whether caption is above (true) or below (false) the figure
    pub above: bool,
}

/// Minimum crop edge in pixels — anything smaller is a meaningless speck.
const MIN_EDGE_PX: u32 = 12;
/// Reject boxes covering more than this fraction of the page — that's a
/// whole-page misdetection, not a diagram.
const MAX_AREA_FRAC: f64 = 0.92;
/// Reject boxes smaller than this fraction of the page — OCR noise.
const MIN_AREA_FRAC: f64 = 0.0005;

/// Normalize one proposed bbox of four values into an in-bounds pixel rect.
/// Returns `None` for garbage input or implausible geometry.
pub fn sanitize_bbox(b: &[f32], img_w: u32, img_h: u32) -> Option<PixelRect> {
    if b.len() != 4 || img_w < 2 * MIN_EDGE_PX || img_h < 2 * MIN_EDGE_PX {
        return None;
    }
    if b.iter().any(|v| !v.is_finite()) {
        return None;
    }
    // Negatives are never meaningful in any supported coordinate system.
    if b.iter().any(|v| *v < 0.0) {
        return None;
    }
    let max_val = b.iter().cloned().fold(0.0f32, f32::max);
    if max_val <= 0.0 {
        return None;
    }

    // ── 1. Detect the coordinate scale ─────────────────────────────────────
    #[derive(Clone, Copy)]
    enum Scale {
        Relative,
        Percent,
        Pixels,
    }
    let scale = if max_val <= 1.5 {
        Scale::Relative
    } else if max_val <= 100.0 {
        Scale::Percent
    } else if max_val <= img_w.max(img_h) as f32 * 1.05 {
        Scale::Pixels
    } else {
        return None; // beyond every plausible system
    };

    // Convert to fractional 0..1 coordinates so every downstream rule is
    // dimension-independent.
    let to_frac = |v: f32, dim: u32| -> f32 {
        match scale {
            Scale::Relative => v,
            Scale::Percent => v / 100.0,
            Scale::Pixels => v / dim as f32,
        }
    };
    let fx = to_frac(b[0], img_w);
    let fy = to_frac(b[1], img_h);
    let fv2 = to_frac(b[2], img_w);
    let fv3 = to_frac(b[3], img_h);

    // ── 2. Resolve [x, y, w, h] vs [x1, y1, x2, y2] ────────────────────────
    // Primary reading (what the prompt asks for): x, y, w, h.
    // Fall back to the corner reading only when the primary cannot apply.
    let mut readings: Vec<(f32, f32, f32, f32)> = Vec::with_capacity(2);
    if b[2] > 0.0 && b[3] > 0.0 {
        readings.push((fx, fy, fx + fv2, fy + fv3)); // (x, y, w, h)
    }
    if fv2 > fx && fv3 > fy {
        readings.push((fx, fy, fv2, fv3)); // (x1, y1, x2, y2)
    }

    for (x0, y0, x1, y1) in readings {
        if x1 - x0 <= 0.001 || y1 - y0 <= 0.001 {
            continue;
        }
        if x0 >= 1.0 || y0 >= 1.0 {
            continue; // starts entirely off-page
        }

        // Rounding (not floor/ceil) makes pixel-scale round-trips stable.
        let px = (x0 * img_w as f32).round().max(0.0) as u32;
        let py = (y0 * img_h as f32).round().max(0.0) as u32;
        let px = px.min(img_w.saturating_sub(1));
        let py = py.min(img_h.saturating_sub(1));

        // x0 < 1.0 / y0 < 1.0 were checked above, so the far edges are >= the
        // origin; saturating_sub is belt-and-braces.
        let far_x = (x1.min(1.0) * img_w as f32).round().max(0.0) as u32;
        let far_y = (y1.min(1.0) * img_h as f32).round().max(0.0) as u32;
        let pw = far_x.saturating_sub(px).min(img_w - px);
        let ph = far_y.saturating_sub(py).min(img_h - py);

        if pw < MIN_EDGE_PX || ph < MIN_EDGE_PX {
            continue;
        }
        let area_frac = (pw as f64 * ph as f64) / (img_w as f64 * img_h as f64);
        if area_frac > MAX_AREA_FRAC || area_frac < MIN_AREA_FRAC {
            continue;
        }
        return Some(PixelRect { x: px, y: py, w: pw, h: ph });
    }

    None
}

/// Expand a sanitized bbox to include nearby caption text, supporting
/// multi-component figures. The expansion is conservative and bounded:
/// - Expands by at most 15% of the figure dimension in the caption direction
/// - Clamps to page bounds and question region
/// - Rejects expansion into footer/margin/answer-area zones
/// - Returns the original bbox if no valid caption hint or expansion unsafe
#[allow(dead_code)]
pub fn expand_bbox_for_caption(
    bbox: PixelRect,
    img_w: u32,
    img_h: u32,
    caption_hint: Option<CaptionHint>,
    question_region: Option<PixelRect>, // Optional: the question's content region
) -> PixelRect {
    let mut expanded = bbox;
    
    // If no caption hint, return original with standard padding already applied upstream
    let Some(hint) = caption_hint else {
        return expanded;
    };
    
    // Convert hint to pixel coordinates
    let caption_x = (hint.x * img_w as f32).round() as u32;
    let _caption_y = (hint.y * img_h as f32).round() as u32;
    
    // Determine expansion direction and amount
    // Max expansion: 15% of figure height/width or 80px, whichever is smaller
    const MAX_EXPANSION_FRAC: f32 = 0.15;
    const MAX_EXPANSION_PX: u32 = 80;
    
    let vertical_expansion = ((bbox.h as f32 * MAX_EXPANSION_FRAC).round() as u32).min(MAX_EXPANSION_PX);
    let horizontal_expansion = ((bbox.w as f32 * MAX_EXPANSION_FRAC).round() as u32).min(MAX_EXPANSION_PX);
    
    if hint.above {
        // Caption is above the figure: expand upward
        let new_y = expanded.y.saturating_sub(vertical_expansion);
        let new_h = expanded.h + (expanded.y - new_y);
        
        // Check if expansion would enter forbidden zones
        if is_safe_expansion(new_y, new_h, expanded.x, expanded.w, img_w, img_h, question_region) {
            expanded.y = new_y;
            expanded.h = new_h;
        }
    } else {
        // Caption is below the figure: expand downward
        let new_h = (expanded.h + vertical_expansion).min(img_h - expanded.y);
        
        if is_safe_expansion(expanded.y, new_h, expanded.x, expanded.w, img_w, img_h, question_region) {
            expanded.h = new_h;
        }
    }
    
    // Also expand horizontally slightly to include caption width if caption
    // extends beyond figure bounds
    if caption_x < expanded.x {
        let expand_left = (expanded.x - caption_x).min(horizontal_expansion);
        let new_x = expanded.x.saturating_sub(expand_left);
        let new_w = expanded.w + (expanded.x - new_x);
        if is_safe_expansion(expanded.y, expanded.h, new_x, new_w, img_w, img_h, question_region) {
            expanded.x = new_x;
            expanded.w = new_w;
        }
    } else if caption_x > expanded.x + expanded.w {
        let expand_right = (caption_x - (expanded.x + expanded.w)).min(horizontal_expansion);
        let new_w = (expanded.w + expand_right).min(img_w - expanded.x);
        if is_safe_expansion(expanded.y, expanded.h, expanded.x, new_w, img_w, img_h, question_region) {
            expanded.w = new_w;
        }
    }
    
    // Final clamp to page bounds
    expanded.x = expanded.x.min(img_w.saturating_sub(1));
    expanded.y = expanded.y.min(img_h.saturating_sub(1));
    expanded.w = expanded.w.min(img_w - expanded.x);
    expanded.h = expanded.h.min(img_h - expanded.y);
    
    // Ensure minimum size
    if expanded.w < MIN_EDGE_PX || expanded.h < MIN_EDGE_PX {
        return bbox; // Revert to original if expansion broke minimum size
    }
    
    expanded
}

/// Check if a proposed expansion is safe (doesn't enter footer, margin, answer area, etc.)
#[allow(dead_code)]
fn is_safe_expansion(
    y: u32,
    h: u32,
    x: u32,
    w: u32,
    img_w: u32,
    img_h: u32,
    question_region: Option<PixelRect>,
) -> bool {
    // 1. Must stay within page bounds
    if x + w > img_w || y + h > img_h {
        return false;
    }
    
    // 2. Must not enter bottom margin (footer zone: bottom 8% of page)
    const FOOTER_ZONE_FRAC: f32 = 0.08;
    let footer_start = (img_h as f32 * (1.0 - FOOTER_ZONE_FRAC)).round() as u32;
    if y + h > footer_start && y < img_h {
        // Expansion enters footer zone
        return false;
    }
    
    // 3. Must not enter top margin (header zone: top 5% of page)
    const HEADER_ZONE_FRAC: f32 = 0.05;
    let header_end = (img_h as f32 * HEADER_ZONE_FRAC).round() as u32;
    if y < header_end {
        return false;
    }
    
    // 4. Must not enter side margins (left/right 5%)
    const SIDE_MARGIN_FRAC: f32 = 0.05;
    let left_margin = (img_w as f32 * SIDE_MARGIN_FRAC).round() as u32;
    let right_margin = img_w - left_margin;
    if x < left_margin || x + w > right_margin {
        return false;
    }
    
    // 5. If question region provided, expansion must stay within it (with small tolerance)
    if let Some(qr) = question_region {
        const TOLERANCE: u32 = 20;
        if x < qr.x.saturating_sub(TOLERANCE) {
            return false;
        }
        if y < qr.y.saturating_sub(TOLERANCE) {
            return false;
        }
        if x + w > qr.x + qr.w + TOLERANCE {
            return false;
        }
        if y + h > qr.y + qr.h + TOLERANCE {
            return false;
        }
    }
    
    // 6. Area fraction sanity check (don't become a full-page crop)
    let area_frac = (w as f64 * h as f64) / (img_w as f64 * img_h as f64);
    if area_frac > MAX_AREA_FRAC {
        return false;
    }
    
    true
}

/// Structural empty-answer-grid detector.
///
/// `is_blank_or_grid` misses ruled student answer grids (AQA trace tables,
/// working grids) because the *rules and header text* push variance and ink
/// above its thresholds. The invariant this checks instead:
///   * ≥ 4 long horizontal rules (> 55% of width) AND ≥ 2 long vertical
///     rules (> 55% of height), i.e. a table skeleton;
///   * after masking the rules themselves, ≥ 80% of the bands between
///     consecutive rules are EMPTY of ink.
/// Filled diagrams (Gantt figures, charts, populated tables) have ink
/// scattered through the bands and are kept.
pub fn looks_like_answer_grid(gray: &image::GrayImage) -> bool {
    const INK: u8 = 150;
    let (w, h) = gray.dimensions();
    if w < 40 || h < 40 {
        return false;
    }

    // Cheap early exit: an EMPTY grid is mostly white. Figures with real
    // content (charts, photos, shaded Gantt bars) blow straight past this —
    // one O(n) pass instead of the full structural scan below.
    // AQA trace tables can be pre-filled (first row completed) pushing ink
    // to ~20% — raise from 15% to 25% so they still enter the structural
    // check instead of being early-rejected as "not a grid" and saved as PNG.
    let mut ink = 0u64;
    for px in gray.pixels() {
        if px[0] < INK {
            ink += 1;
        }
    }
    if ink as f64 > 0.25 * (w as f64) * (h as f64) {
        return false;
    }

    let w_thresh = (w as f64 * 0.55) as u32;
    let h_thresh = (h as f64 * 0.55) as u32;

    let mut line_rows: Vec<u32> = Vec::new();
    for y in 0..h {
        let mut row_ink = 0u32;
        for x in 0..w {
            if gray.get_pixel(x, y)[0] < INK {
                row_ink += 1;
            }
        }
        if row_ink > w_thresh {
            line_rows.push(y);
        }
    }

    let mut line_cols: Vec<u32> = Vec::new();
    for x in 0..w {
        let mut col_ink = 0u32;
        for y in 0..h {
            if gray.get_pixel(x, y)[0] < INK {
                col_ink += 1;
            }
        }
        if col_ink > h_thresh {
            line_cols.push(x);
        }
    }

    if line_rows.len() < 4 || line_cols.len() < 2 {
        return false;
    }

    // Bands between consecutive horizontal rules (> 3px apart).
    let mut bands: Vec<(u32, u32)> = Vec::new();
    let mut prev: Option<u32> = None;
    for &y in &line_rows {
        if let Some(p) = prev {
            if y - p > 3 {
                bands.push((p, y));
            }
        }
        prev = Some(y);
    }
    if bands.is_empty() {
        return false;
    }

    let is_line_col = |x: u32| line_cols.binary_search(&x).is_ok();

    let mut empty = 0usize;
    for &(a, b) in &bands {
        let mut band_ink = 0u32;
        for y in (a + 1)..b {
            if line_rows.binary_search(&y).is_ok() {
                continue;
            }
            for x in 0..w {
                if is_line_col(x) {
                    continue;
                }
                if gray.get_pixel(x, y)[0] < INK {
                    band_ink += 1;
                }
            }
        }
        let band_area = (b - a - 1) as u64 * w as u64;
        if (band_ink as u64) < (band_area as f64 * 0.002) as u64 {
            empty += 1;
        }
    }

    (empty as f64) >= 0.8 * bands.len() as f64
}

/// 8×8 block-mean luma signature for duplicate-diagram detection.
/// Area-averaged (not point-sampled) so sparse line art survives.
pub fn tile_signature(crop: &image::RgbaImage) -> [u8; 64] {
    let gray = image::DynamicImage::ImageRgba8(crop.clone()).to_luma8();
    let (w, h) = gray.dimensions();
    let mut out = [0u8; 64];
    if w == 0 || h == 0 {
        return out;
    }
    for ty in 0..8u32 {
        for tx in 0..8u32 {
            let y0 = ty * h / 8;
            let y1 = ((ty + 1) * h / 8).max(y0 + 1).min(h);
            let x0 = tx * w / 8;
            let x1 = ((tx + 1) * w / 8).max(x0 + 1).min(w);
            let mut sum = 0u64;
            let mut n = 0u64;
            for y in y0..y1 {
                for x in x0..x1 {
                    sum += gray.get_pixel(x, y)[0] as u64;
                    n += 1;
                }
            }
            out[(ty * 8 + tx) as usize] = if n > 0 { (sum / n) as u8 } else { 255 };
        }
    }
    out
}

/// Mean per-tile L1 distance (0–255). Same diagram, slightly different
/// crop extent scores very low; distinct diagrams score high.
pub fn signature_distance(a: &[u8; 64], b: &[u8; 64]) -> u32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs())
        .sum::<u32>()
        / 64
}

/// Why a proposed crop was rejected — surfaced in the import report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropReject {
    /// bbox failed the sanitizer / out of bounds / implausible
    BadBox,
    /// ruled empty student answer grid (trace table, working grid)
    AnswerGrid,
}

/// Crop a proposed diagram bbox out of a decoded page image.
/// Applies the sanitizer, padding, the blank guard, and the structural
/// answer-grid guard. Returns `Err(CropReject)` (never panics) when the
/// crop is unusable.
pub fn crop_diagram(
    img: &image::DynamicImage,
    bbox: &[f32],
    padding: u32,
) -> Result<image::RgbaImage, CropReject> {
    use image::GenericImageView;
    let (img_w, img_h) = img.dimensions();
    let rect = sanitize_bbox(bbox, img_w, img_h).ok_or(CropReject::BadBox)?;

    // Footer / header / margin guards (AQA "1 | 1" false-positive was a
    // tiny table fragment touching the page footer IB/G/Jun24/7517/2).
    const FOOTER_FRAC: f32 = 0.08;
    const HEADER_FRAC: f32 = 0.05;
    const SIDE_FRAC: f32 = 0.05;
    let footer_start = (img_h as f32 * (1.0 - FOOTER_FRAC)).round() as u32;
    let header_end = (img_h as f32 * HEADER_FRAC).round() as u32;
    let left_margin = (img_w as f32 * SIDE_FRAC).round() as u32;
    let right_margin = img_w.saturating_sub(left_margin);
    // Any box entering the bottom 8% (exam footer) is invalid — never a figure.
    if rect.y + rect.h > footer_start {
        return Err(CropReject::BadBox);
    }
    if rect.y < header_end {
        return Err(CropReject::BadBox);
    }
    // Small boxes touching side margins are almost always barcode/QR or
    // marginalia like "1 | 1" fragments from a ruled grid, not a figure.
    let area_frac = (rect.w as f64 * rect.h as f64) / (img_w as f64 * img_h as f64);
    if (rect.x < left_margin || rect.x + rect.w > right_margin) && area_frac < 0.10 {
        return Err(CropReject::BadBox);
    }

    let safe_x = rect.x.saturating_sub(padding);
    let safe_y = rect.y.saturating_sub(padding);
    // img_w - safe_x / img_h - safe_y cannot underflow: the sanitizer
    // guarantees rect.x <= img_w - 1 and rect.y <= img_h - 1.
    let x_pad_left = rect.x - safe_x;
    let y_pad_top = rect.y - safe_y;
    let safe_w = (rect.w + x_pad_left + padding).min(img_w - safe_x);
    let safe_h = (rect.h + y_pad_top + padding).min(img_h - safe_y);

    if safe_w < MIN_EDGE_PX || safe_h < MIN_EDGE_PX {
        return Err(CropReject::BadBox);
    }

    let mut owned = img.clone();
    let cropped = image::imageops::crop(&mut owned, safe_x, safe_y, safe_w, safe_h).to_image();

    let gray = image::DynamicImage::ImageRgba8(cropped.clone()).to_luma8();
    if looks_like_answer_grid(&gray) {
        return Err(CropReject::AnswerGrid);
    }
    Ok(cropped)
}

/// Decode a base64 page image (with or without a data-URL prefix).
pub fn decode_page_image(b64: &str) -> Option<image::DynamicImage> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(strip_data_url(b64))
        .ok()?;
    image::load_from_memory(&bytes).ok()
}

// ── Vertical page band crop (Phase 1) ─────────────────────────────────────
//
// When a page contains multiple questions (MCQs, short-answer bands, or a
// footer that sits above the next question's heading), the document map
// records a vertical y_frac range for the question on that page. We do NOT
// physically crop in Phase 1 (we use prompt-level band hints + diagram-bbox
// y-range validation instead, which avoids coordinate-shift complexity
// across audit/save/dedupe). This helper is retained for a future Phase 2
// optimization and is currently unused.
#[allow(dead_code)]
pub struct PageBand {
    /// New base64 JPEG (no data-URL prefix) of the cropped region.
    pub b64: String,
    /// Pixel y-offset within the source image where the crop begins.
    pub y_offset_px: u32,
    /// Height of the cropped region in pixels.
    pub height_px: u32,
    /// Fractional y-offset within the source image (convenience).
    pub y_offset_frac: f32,
}

/// Crop a page image to a vertical band. Returns None if the band is
/// degenerate (empty or outside the page). `start_frac`/`end_frac` are
/// clamped to [0,1] and padded by a small margin to avoid chopping the
/// top/bottom lines of the question.
#[allow(dead_code)]
pub fn crop_page_vertical(b64: &str, start_frac: f32, end_frac: f32) -> Option<PageBand> {
    use base64::Engine;
    use image::GenericImageView;
    let img = decode_page_image(b64)?;
    let (w, h) = img.dimensions();
    if w < 2 || h < 2 {
        return None;
    }
    // Pad by ~0.005 of the page (a few lines) so descenders/headings aren't
    // clipped, and clamp into [0,1].
    let pad = 0.005_f32;
    let s = (start_frac - pad).clamp(0.0, 1.0);
    let e = (end_frac + pad).clamp(0.0, 1.0);
    if e - s < 0.01 {
        return None;
    }
    let y0 = (s * h as f32).round() as u32;
    let y1 = (e * h as f32).round() as u32;
    let y0 = y0.min(h.saturating_sub(1));
    let y1 = y1.min(h).max(y0 + 1);
    let band_h = y1 - y0;

    let mut owned = img;
    let cropped_rgba = image::imageops::crop(&mut owned, 0, y0, w, band_h).to_image();
    let cropped = image::DynamicImage::ImageRgba8(cropped_rgba).to_rgb8();

    // Re-encode as JPEG at high quality (matching what the frontend sends).
    // We use JPEG rather than PNG here because the result goes straight
    // to the vision API as an image_url payload and JPEG is ~4x smaller
    // at equivalent visual fidelity for text+line-art pages.
    let mut buf = std::io::Cursor::new(Vec::with_capacity((w as usize * band_h as usize) / 8));
    {
        use image::codecs::jpeg::JpegEncoder;
        use image::ImageEncoder;
        let enc = JpegEncoder::new_with_quality(&mut buf, 92);
        enc.write_image(&cropped, w, band_h, image::ExtendedColorType::Rgb8).ok()?;
    }
    let out_b64 = base64::engine::general_purpose::STANDARD.encode(buf.into_inner());

    Some(PageBand {
        b64: out_b64,
        y_offset_px: y0,
        height_px: band_h,
        y_offset_frac: s,
    })
}

/// Strip a data-URL prefix, if present.
pub fn strip_data_url(b64: &str) -> &str {
    if b64.starts_with("data:image") {
        b64.split(',').nth(1).unwrap_or(b64)
    } else {
        b64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 1654;
    const H: u32 = 2339;

    fn assert_close(a: u32, b: u32, tol: u32) {
        let d = (a as i64 - b as i64).abs();
        assert!(d <= tol as i64, "{a} not within {tol} of {b}");
    }

    #[test]
    fn relative_xywh() {
        let r = sanitize_bbox(&[0.1, 0.2, 0.4, 0.3], W, H).unwrap();
        assert_close(r.x, 165, 2);
        assert_close(r.y, 468, 2);
        assert!(r.w > 600 && r.w < 720);
        assert!(r.h > 650 && r.h < 760);
        assert!(r.x + r.w <= W && r.y + r.h <= H);
    }

    #[test]
    fn ambiguous_reading_stays_in_bounds() {
        // Could be (x,y,w,h) or (x1,y1,x2,y2) — either way: valid + in bounds.
        let r = sanitize_bbox(&[0.1, 0.1, 0.45, 0.45], W, H).unwrap();
        assert!(r.x + r.w <= W && r.y + r.h <= H);
    }

    #[test]
    fn pixel_coords_do_not_panic_and_round_trip() {
        // The killer case from the bug report: raw pixel integers.
        let r = sanitize_bbox(&[100.0, 150.0, 600.0, 400.0], W, H).unwrap();
        assert_close(r.x, 100, 2);
        assert_close(r.y, 150, 2);
        assert_close(r.w, 600, 2);
        assert_close(r.h, 400, 2);
        assert!(r.x + r.w <= W);
        assert!(r.y + r.h <= H);
    }

    #[test]
    fn percent_coords() {
        let r = sanitize_bbox(&[10.0, 20.0, 40.0, 30.0], W, H).unwrap();
        assert_close(r.x, 165, 3);
        assert!(r.x + r.w <= W && r.y + r.h <= H);
    }

    #[test]
    fn out_of_range_rejected() {
        assert!(sanitize_bbox(&[5000.0, 5000.0, 8000.0, 5000.0], W, H).is_none());
    }

    #[test]
    fn full_page_rejected() {
        assert!(sanitize_bbox(&[0.0, 0.0, 1.0, 1.0], W, H).is_none());
        assert!(sanitize_bbox(&[0.01, 0.01, 0.98, 0.98], W, H).is_none());
    }

    #[test]
    fn near_full_page_rejected_in_both_readings() {
        // >92% of the page whether read as (x,y,w,h) or (x1,y1,x2,y2).
        assert!(sanitize_bbox(&[0.0, 0.0, 0.97, 0.97], W, H).is_none());
        // Ambiguous large-but-legit box: as x1y1x2y2 it's ~88% (a big Gantt
        // chart is legal) — accepted, clamped, in bounds.
        let r = sanitize_bbox(&[0.02, 0.02, 0.96, 0.96], W, H).unwrap();
        assert!(r.x + r.w <= W && r.y + r.h <= H);
    }

    #[test]
    fn degenerate_rejected() {
        assert!(sanitize_bbox(&[0.5, 0.5, 0.0001, 0.0001], W, H).is_none());
    }

    #[test]
    fn nan_inf_negative_rejected() {
        assert!(sanitize_bbox(&[f32::NAN, 0.0, 0.5, 0.5], W, H).is_none());
        assert!(sanitize_bbox(&[f32::INFINITY, 0.0, 0.5, 0.5], W, H).is_none());
        assert!(sanitize_bbox(&[-0.1, 0.1, 0.5, 0.5], W, H).is_none());
    }

    #[test]
    fn off_page_start_rejected_not_panicky() {
        assert!(sanitize_bbox(&[2000.0, 2000.0, 100.0, 100.0], W, H).is_none());
    }

    #[test]
    fn wrong_len_rejected() {
        assert!(sanitize_bbox(&[0.1, 0.2], W, H).is_none());
        assert!(sanitize_bbox(&[0.1, 0.2, 0.3, 0.4, 0.5], W, H).is_none());
    }

    #[test]
    fn crop_diagram_never_panics_on_garbage() {
        let img = image::DynamicImage::new_rgba8(W, H);
        for b in [
            &[100.0, 150.0, 600.0, 400.0][..],
            &[5000.0, 1.0, 2.0, 3.0][..],
            &[f32::NAN, 0.0, 1.0, 1.0][..],
            &[0.1, 0.1, 0.3, 0.3][..],
        ] {
            let _ = crop_diagram(&img, b, 40); // must not panic
        }
    }

    // ── Synthetic fixtures for the answer-grid guard ────────────────────────

    fn blank(w: u32, h: u32) -> image::GrayImage {
        image::GrayImage::from_pixel(w, h, image::Luma([255u8]))
    }
    fn hline(g: &mut image::GrayImage, y: u32) {
        for x in 0..g.width() {
            g.put_pixel(x, y, image::Luma([40u8]));
        }
    }
    fn vline(g: &mut image::GrayImage, x: u32, y0: u32, y1: u32) {
        for y in y0..y1 {
            g.put_pixel(x, y, image::Luma([40u8]));
        }
    }
    fn text_blob(g: &mut image::GrayImage, y: u32, x0: u32, w: u32) {
        for x in x0..x0 + w {
            g.put_pixel(x, y, image::Luma([60u8]));
            g.put_pixel(x, y + 3, image::Luma([60u8]));
        }
    }

    /// The AQA trace table from the bug report: header text, 25 ruled rows,
    /// 6 column rules — an EMPTY answer grid.
    fn trace_table(w: u32, h: u32) -> image::GrayImage {
        let mut g = blank(w, h);
        let rows: Vec<u32> = (0..25).map(|i| 20 + i * 34).collect();
        for &r in &rows {
            if r < h {
                hline(&mut g, r);
            }
        }
        for c in [20, 215, 420, 470, 520, 570] {
            if c < w {
                vline(&mut g, c, 20, (*rows.last().unwrap()).min(h - 1));
            }
        }
        text_blob(&mut g, 40, 60, 220);
        text_blob(&mut g, 44, 260, 150);
        text_blob(&mut g, 100, 60, 120);
        g
    }

    /// A filled Gantt figure (legit diagram): same skeleton, but bars and
    /// marks scattered through most bands.
    fn filled_gantt() -> image::GrayImage {
        let mut g = blank(600, 500);
        let rows: Vec<u32> = (0..11).map(|i| 20 + i * 40).collect();
        for &r in &rows {
            hline(&mut g, r);
        }
        for c in [20, 120] {
            vline(&mut g, c, 20, *rows.last().unwrap());
        }
        vline(&mut g, 580, 0, 499);
        for (i, _r) in rows.iter().enumerate().take(10) {
            if i % 2 == 0 || i == 7 {
                let band_y = rows[i] + 15;
                for yy in band_y..band_y + 10 {
                    for x in 180..480u32 {
                        g.put_pixel(x, yy, image::Luma([80u8]));
                    }
                }
            }
        }
        g
    }

    /// A simple plotted curve (legit diagram): two axes + polyline, no grid.
    fn simple_chart() -> image::GrayImage {
        let mut g = blank(600, 400);
        hline(&mut g, 370);
        vline(&mut g, 40, 0, 399);
        for x in 40..580u32 {
            let y = (200.0 - 120.0 * ((x as f64 - 40.0) / 90.0).sin()) as i64;
            if y >= 0 {
                g.put_pixel(x, y.min(399) as u32, image::Luma([30u8]));
            }
        }
        g
    }

    #[test]
    fn answer_grid_rejected_figures_kept() {
        assert!(looks_like_answer_grid(&trace_table(602, 872)));
        assert!(!looks_like_answer_grid(&filled_gantt()));
        assert!(!looks_like_answer_grid(&simple_chart()));
        assert!(!looks_like_answer_grid(&blank(400, 400)));
    }

    #[test]
    fn crop_diagram_rejects_trace_table_keeps_chart() {
        // Page = white background with a trace table region and a chart region.
        let page_grid = trace_table(602, 872);
        let img = image::DynamicImage::ImageLuma8(page_grid);
        // Box around the whole trace table (relative coords) → AnswerGrid.
        assert_eq!(
            crop_diagram(&img, &[0.1, 0.1, 0.8, 0.8], 0),
            Err(CropReject::AnswerGrid)
        );

        let chart_page = image::DynamicImage::ImageLuma8(simple_chart());
        let r = crop_diagram(&chart_page, &[0.0, 0.1, 1.0, 0.7], 0);
        assert!(r.is_ok());
    }

    #[test]
    fn signature_dedupes_same_diagram_separates_distinct() {
        let t1 = image::DynamicImage::ImageLuma8(trace_table(602, 872)).to_rgba8();
        let t2 = image::DynamicImage::ImageLuma8(trace_table(600, 860)).to_rgba8();
        let chart = image::DynamicImage::ImageLuma8(simple_chart()).to_rgba8();
        let s1 = tile_signature(&t1);
        let s2 = tile_signature(&t2);
        let sc = tile_signature(&chart);
        assert!(signature_distance(&s1, &s2) < 4, "same table, resized → duplicate");
        assert!(signature_distance(&s1, &sc) >= 6, "table vs chart → distinct");
    }
}
