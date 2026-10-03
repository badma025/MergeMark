// ── Deterministic diagram geometry ─────────────────────────────────────────
//
// Figure boxes come from the local detector and the layout map; this module
// is the single authority that turns a box into a crop. It:
//   * accepts relative (0..1), percent (0..100), or absolute pixel values
//     and auto-detects which scale was used,
//   * tolerates both [x, y, w, h] and [x1, y1, x2, y2] semantics,
//   * clamps into image bounds with saturating math (never panics, never
//     underflows u32),
//   * rejects implausible boxes (degenerate specks, ~full-page misdetections),
// so the crash class and the wrong-crop class are retired whatever box it is
// handed.

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

/// Heuristic: detect if a crop contains MCQ option letters (A, B, C, D) that would be clipped.
#[allow(dead_code)]
fn contains_mcq_options(_crop: &image::RgbaImage) -> bool {
    // This is a cheap check - we'd need OCR to be certain, but we can
    // use the caption text from the model to detect this case
    false  // Placeholder - actual detection happens at a higher level
}

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
    let primary = (fx, fy, fx + fv2, fy + fv3); // (x, y, w, h)
    let corners = (fx, fy, fv2, fv3); // (x1, y1, x2, y2)
    let corner_is_in_bounds = fv2 <= 1.0 && fv3 <= 1.0 && fv2 > fx && fv3 > fy;
    // A box may come as corner coordinates instead of width/height. If the width/height reading crosses the page
    // edge while the corner reading is fully in bounds, prefer the latter.
    // This preserves figures whose lower edge would otherwise be mistaken for
    // an exam footer and rejected after clamping.
    let prefer_corners = corner_is_in_bounds && (primary.2 > 1.0 || primary.3 > 1.0);
    let mut readings: Vec<(f32, f32, f32, f32)> = Vec::with_capacity(2);
    if prefer_corners {
        readings.push(corners);
        readings.push(primary);
    } else {
        if b[2] > 0.0 && b[3] > 0.0 {
            readings.push(primary);
        }
        if corner_is_in_bounds || (fv2 > fx && fv3 > fy) {
            readings.push(corners);
        }
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
        let far_x = (x1 * img_w as f32).round().max(0.0) as u32;
        let far_y = (y1 * img_h as f32).round().max(0.0) as u32;
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

    let w_thresh = (w as f64 * 0.75) as u32;
    let h_thresh = (h as f64 * 0.65) as u32;

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

    // Grids have many rows. Most physics diagrams have few full-width wires.
    // Raise from 4 to 6 rows to protect complex circuits/schematics.
    if line_rows.len() < 6 || line_cols.len() < 2 {
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
    if bands.len() < 4 {
        return false;
    }

    // Check for regularity: grids have uniform row heights.
    // Physics diagrams with "wires" (line_rows) usually have varying spacing.
    let heights: Vec<u32> = bands.iter().map(|(a, b)| b - a).collect();
    let avg = heights.iter().sum::<u32>() as f32 / heights.len() as f32;
    let mut variance = 0.0;
    for &h in &heights {
        variance += (h as f32 - avg).powi(2);
    }
    let std_dev = (variance / heights.len() as f32).sqrt();
    // A regular grid will have very low std_dev relative to average height.
    // If std_dev is > 15% of average height, it's likely a varied diagram, not a grid.
    if std_dev > 0.15 * avg {
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
    /// nothing printed in the box (an exact crop must show its figure)
    Blank,
}

/// Crop a proposed diagram bbox out of a decoded page image.
/// Applies the sanitizer, padding, the blank guard, and the structural
/// answer-grid guard. Returns `Err(CropReject)` (never panics) when the
/// crop is unusable.
#[allow(dead_code)]
pub fn crop_diagram(
    img: &image::DynamicImage,
    bbox: &[f32],
    padding: u32,
    ignore_grid: bool,
) -> Result<image::RgbaImage, CropReject> {
    crop_diagram_with_options(img, bbox, padding, ignore_grid, false)
}

// ── Deterministic figure detection (PDF content stream) ────────────────────
//
// An on-device pass over the PDF's vector content: every Image/Path object
// has exact page-space bounds from pdfium.
// This module contains the pure geometry: normalization, clustering, and the
// filter predicates. `pdf_render::detect_pdf_figures` is the thin wrapper that
// feeds it raw object bounds; these functions are unit-testable without pdfium.
// Box convention matches the rest of the codebase: normalized [x, y, w, h] in
// 0..1 with y measured from the top of the page.

/// Convert PDF-space bounds (origin bottom-left, in points) into the app's
/// normalized [x, y, w, h] (y from top). Clamped to the page.
pub fn normalize_pdf_box(
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    page_w: f32,
    page_h: f32,
) -> [f32; 4] {
    let x = (left / page_w).clamp(0.0, 1.0);
    let y = (1.0 - top / page_h).clamp(0.0, 1.0);
    let w = ((right - left) / page_w).clamp(0.0, 1.0);
    let h = ((top - bottom) / page_h).clamp(0.0, 1.0);
    [x, y, w, h]
}

/// Reject degenerate figure boxes: area below the floor or either dimension
/// below the floor. Kills specks, tick marks, borders, and underlines.
pub fn is_tiny_figure(b: &[f32; 4], min_area_frac: f32, min_dim_frac: f32) -> bool {
    let area = b[2] * b[3];
    area < min_area_frac || b[2] < min_dim_frac || b[3] < min_dim_frac
}

/// Reject "rule lines": extremely thin boxes (ruled answer lines, borders,
/// continuation rules). Aspect ratio beyond `max_aspect` in either direction.
pub fn is_rule_line(b: &[f32; 4], max_aspect: f32) -> bool {
    let (w, h) = (b[2], b[3]);
    if w <= 0.0 || h <= 0.0 {
        return true;
    }
    let aspect = w / h;
    aspect > max_aspect || aspect < 1.0 / max_aspect
}

/// Fraction of `b`'s area covered by the given text-segment rectangles
/// (normalized [x, y, w, h], y from top). Overlaps are counted once per
/// segment (not unioned), capped at 1.0. A high share means the region is
/// question text or a table, not a figure.
pub fn text_density_in_box(b: &[f32; 4], text_rects: &[[f32; 4]]) -> f32 {
    let box_area = b[2] * b[3];
    if box_area <= 0.0 {
        return 1.0;
    }
    let mut covered = 0.0f32;
    for tr in text_rects {
        let ix0 = b[0].max(tr[0]);
        let ix1 = (b[0] + b[2]).min(tr[0] + tr[2]);
        let iy0 = b[1].max(tr[1]);
        let iy1 = (b[1] + b[3]).min(tr[1] + tr[3]);
        if ix0 < ix1 && iy0 < iy1 {
            covered += (ix1 - ix0) * (iy1 - iy0);
        }
    }
    (covered / box_area).min(1.0)
}

/// One-stop predicate for a candidate figure region: rejects specks, rule
/// lines, header (top 5%) / footer (bottom 8%) bands, and text-dense regions.
pub fn is_probable_figure_box(
    b: &[f32; 4],
    text_rects: &[[f32; 4]],
    min_area_frac: f32,
    min_dim_frac: f32,
    max_aspect: f32,
    max_text_density: f32,
) -> bool {
    if is_tiny_figure(b, min_area_frac, min_dim_frac) || is_rule_line(b, max_aspect) {
        return false;
    }
    if b[1] < 0.05 || b[1] + b[3] > 0.92 {
        return false;
    }
    text_density_in_box(b, text_rects) <= max_text_density
}

/// Infer a semantic kind from a figure caption (or nearby text), for the
/// `graph_like` crop flag and the report card.
pub fn caption_kind_from_text(caption: &str) -> Option<String> {
    let lower = caption.to_ascii_lowercase();
    if lower.contains("flow") {
        Some("flowchart".to_string())
    } else if lower.contains("graph")
        || lower.contains("chart")
        || lower.contains("plot")
        || lower.contains("curve")
    {
        Some("graph".to_string())
    } else if lower.contains("circuit") {
        Some("circuit".to_string())
    } else if lower.contains("diagram") || lower.contains("sketch") {
        Some("diagram".to_string())
    } else {
        None
    }
}

/// Expand a pixel-space rectangle without changing the coordinate-system
/// meaning of its edges. `left` and `top` move the origin toward zero;
/// `right` and `bottom` move the far edge toward the page bounds.
fn expand_rect(
    rect: PixelRect,
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
    img_w: u32,
    img_h: u32,
) -> PixelRect {
    let x0 = rect.x.saturating_sub(left);
    let y0 = rect.y.saturating_sub(top);
    let x1 = rect
        .x
        .saturating_add(rect.w)
        .saturating_add(right)
        .min(img_w);
    let y1 = rect
        .y
        .saturating_add(rect.h)
        .saturating_add(bottom)
        .min(img_h);
    PixelRect {
        x: x0,
        y: y0,
        w: x1.saturating_sub(x0),
        h: y1.saturating_sub(y0),
    }
}

// ── Point-space geometry for Smart Scissors (stroke census) ────────────────
//
// All stroke-census clustering happens in PDF point space (origin
// bottom-left, y up). A `RectPt` is `[left, right, top, bottom]` with
// `top > bottom`. Conversion to the app's normalized [x, y, w, h] happens
// exactly once at emission via `normalize_pdf_box`.

pub type RectPt = [f32; 4];

/// Construct a point-space rect from its four edges (y-up: top > bottom).
#[inline]
pub fn rect_pt(left: f32, right: f32, top: f32, bottom: f32) -> RectPt {
    [left, right, top, bottom]
}

#[inline]
pub fn rect_w(r: &RectPt) -> f32 {
    (r[1] - r[0]).max(0.0)
}

#[inline]
pub fn rect_h(r: &RectPt) -> f32 {
    (r[2] - r[3]).max(0.0)
}

#[inline]
pub fn rect_area(r: &RectPt) -> f32 {
    rect_w(r) * rect_h(r)
}

#[inline]
pub fn rect_center(r: &RectPt) -> [f32; 2] {
    [(r[0] + r[1]) / 2.0, (r[2] + r[3]) / 2.0]
}

pub fn rect_union(a: &RectPt, b: &RectPt) -> RectPt {
    [
        a[0].min(b[0]),
        a[1].max(b[1]),
        a[2].max(b[2]),
        a[3].min(b[3]),
    ]
}

/// Axis-aligned separation distance between two rects. Zero when they
/// overlap or touch. This is the join metric used by region growing.
pub fn rect_gap(a: &RectPt, b: &RectPt) -> f32 {
    let dx = (a[0] - b[1]).max(b[0] - a[1]).max(0.0);
    let dy = (a[3] - b[2]).max(b[3] - a[2]).max(0.0);
    (dx * dx + dy * dy).sqrt()
}

/// Overlap length of two 1-D intervals [a0, a1] and [b0, b1].
#[inline]
fn interval_overlap(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

/// Fraction of `inner`'s x-span overlapped by `outer`'s x-span (0..1).
/// Used by line/block assembly: two lines coalesce only if they genuinely
/// sit side-by-side, not merely near each other.
pub fn x_span_coverage(inner: &RectPt, outer: &RectPt) -> f32 {
    let w = rect_w(inner);
    if w <= 0.0 {
        return 0.0;
    }
    interval_overlap(inner[0], inner[1], outer[0], outer[1]) / w
}

/// Detect uniform-spacing families among 1-D stroke centers.
///
/// Two-level: coordinates are first collapsed into "line positions"
/// (centers within `tol_pt` belong to the same printed line), then every
/// window of `min_members` consecutive lines whose gaps have coefficient of
/// variation ≤ `cv_max` is reported as a family. This mirrors reality:
/// gridline members sit far apart (their pitch IS the signal), while
/// near-duplicate centers are fragments of one line. Windows may overlap on
/// long uniform grids; consumers retype idempotently. Returns index groups.
pub fn uniform_spacing_families(
    mut coords: Vec<(f32, usize)>,
    tol_pt: f32,
    min_members: usize,
    cv_max: f32,
) -> Vec<Vec<usize>> {
    if coords.len() < min_members {
        return Vec::new();
    }
    coords.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    // Collapse near-identical centers into line positions (running mean).
    let mut lines: Vec<(f32, Vec<usize>)> = Vec::new();
    for (coord, idx) in coords {
        match lines.last_mut() {
            Some((rep, members)) if coord - *rep <= tol_pt => {
                let n = members.len() as f32;
                *rep = (*rep * n + coord) / (n + 1.0);
                members.push(idx);
            }
            _ => lines.push((coord, vec![idx])),
        }
    }
    if lines.len() < min_members {
        return Vec::new();
    }

    let mut families = Vec::new();
    for start in 0..=lines.len() - min_members {
        let window = &lines[start..start + min_members];
        let gaps: Vec<f32> = window.windows(2).map(|w| w[1].0 - w[0].0).collect();
        let n = gaps.len() as f32;
        let mean = gaps.iter().sum::<f32>() / n;
        if mean <= 1e-6 {
            continue;
        }
        let var = gaps.iter().map(|g| (g - mean) * (g - mean)).sum::<f32>() / n;
        if var.sqrt() / mean <= cv_max {
            families.push(window.iter().flat_map(|(_, m)| m.iter().copied()).collect());
        }
    }
    families
}

/// True when the straight corridor between the centers of `a` and `b`
/// passes through any barrier (sampled every `step_pt`, barriers expanded
/// by `pad_pt`). This is the ONLY geometric predicate region growth uses —
/// strokes separated by question text can never weld into one figure.
pub fn corridor_blocked(a: &RectPt, b: &RectPt, barriers: &[RectPt], step_pt: f32, pad_pt: f32) -> bool {
    if barriers.is_empty() {
        return false;
    }
    let ca = rect_center(a);
    let cb = rect_center(b);
    let dx = cb[0] - ca[0];
    let dy = cb[1] - ca[1];
    let dist = (dx * dx + dy * dy).sqrt();
    let steps = ((dist / step_pt.max(1.0)).ceil() as usize).max(1);
    for k in 0..=steps {
        let t = k as f32 / steps as f32;
        let px = ca[0] + dx * t;
        let py = ca[1] + dy * t;
        for bar in barriers {
            if px >= bar[0] - pad_pt
                && px <= bar[1] + pad_pt
                && py >= bar[3] - pad_pt
                && py <= bar[2] + pad_pt
            {
                return true;
            }
        }
    }
    false
}

/// Expand a point-space rect outward by `pad` on every edge.
pub fn pad_rect(r: &RectPt, pad: f32) -> RectPt {
    [r[0] - pad, r[1] + pad, r[2] + pad, r[3] - pad]
}

/// Minimum fraction of an edge's span a barrier must cover to block that
/// edge's movement. Corner grazes by unrelated bands never freeze lateral
/// growth; genuine text walls spanning most of the edge do.
const EDGE_COVER_MIN: f32 = 0.15;

/// Expand `cur` toward `goal` edge-by-edge. Each moving edge stops at the
/// first barrier face it would cross; a barrier counts only when it lies in
/// FRONT of the movement AND spans ≥ `EDGE_COVER_MIN` of the edge. Monotone
/// and deterministic — label capture can pull a crop toward its labels but
/// never swallow neighbouring body text.
pub fn expand_clamped(cur: &RectPt, goal: &RectPt, barriers: &[RectPt], pad_pt: f32) -> RectPt {
    let covers = |edge_lo: f32, edge_hi: f32, bar_lo: f32, bar_hi: f32| -> bool {
        let span = (edge_hi - edge_lo).max(1e-3);
        interval_overlap(edge_lo, edge_hi, bar_lo, bar_hi) / span >= EDGE_COVER_MIN
    };
    let mut r = *cur;

    // LEFT: stopping faces are barrier right-faces strictly ahead (< r[0]).
    if goal[0] < r[0] {
        let mut limit = goal[0];
        for bar in barriers {
            if !covers(r[3], r[2], bar[3], bar[2]) {
                continue;
            }
            let stop = bar[1] + pad_pt;
            if stop >= r[0] {
                // Zone reaches across the edge plane — movement is frozen…
                if bar[0] - pad_pt < r[0] {
                    limit = r[0];
                    break;
                }
                // …but only when the barrier is ahead of, not behind, the edge.
                continue;
            }
            if stop > limit {
                limit = stop;
            }
        }
        r[0] = limit.min(r[0]);
    }
    // RIGHT
    if goal[1] > r[1] {
        let mut limit = goal[1];
        for bar in barriers {
            if !covers(r[3], r[2], bar[3], bar[2]) {
                continue;
            }
            let stop = bar[0] - pad_pt;
            if stop <= r[1] {
                if bar[1] + pad_pt > r[1] {
                    limit = r[1];
                    break;
                }
                continue;
            }
            if stop < limit {
                limit = stop;
            }
        }
        r[1] = limit.max(r[1]);
    }
    // TOP: moving up, barrier bottom-faces are the stopping faces.
    if goal[2] > r[2] {
        let mut limit = goal[2];
        for bar in barriers {
            if !covers(r[0], r[1], bar[0], bar[1]) {
                continue;
            }
            let stop = bar[3] - pad_pt;
            if stop <= r[2] {
                if bar[2] + pad_pt > r[2] {
                    limit = r[2];
                    break;
                }
                continue;
            }
            if stop < limit {
                limit = stop;
            }
        }
        r[2] = limit.max(r[2]);
    }
    // BOTTOM
    if goal[3] < r[3] {
        let mut limit = goal[3];
        for bar in barriers {
            if !covers(r[0], r[1], bar[0], bar[1]) {
                continue;
            }
            let stop = bar[2] + pad_pt;
            if stop >= r[3] {
                if bar[3] - pad_pt < r[3] {
                    limit = r[3];
                    break;
                }
                continue;
            }
            if stop > limit {
                limit = stop;
            }
        }
        r[3] = limit.min(r[3]);
    }
    r
}

/// Crop a diagram with optional graph-canvas margins. Graphs need asymmetric
/// room outside the plotted rectangle for vertical axis titles/units on the
/// left and tick labels/axis titles below the x-axis.
/// Crop a figure whose box is already its exact extent (drawn strokes and
/// their labels, from the page layout): a hairline margin, and none of the
/// padding, edge-text trimming or margin heuristics meant for proposed
/// boxes — those cut away labels set at a figure's edge. Answer grids are
/// still refused unless the question asks to read one.
pub fn crop_exact(img: &image::DynamicImage, bbox: &[f32], ignore_grid: bool, pad_frac: f32) -> Result<image::RgbaImage, CropReject> {
    use image::GenericImageView;
    let (img_w, img_h) = img.dimensions();
    let rect = sanitize_bbox(bbox, img_w, img_h).ok_or(CropReject::BadBox)?;
    let pad = (img_w.min(img_h) as f32 * pad_frac.max(0.0)).round() as u32;
    let expanded = expand_rect(rect, pad, pad, pad, pad, img_w, img_h);
    if expanded.w < MIN_EDGE_PX || expanded.h < MIN_EDGE_PX {
        return Err(CropReject::BadBox);
    }
    let mut owned = img.clone();
    let cropped = image::imageops::crop(&mut owned, expanded.x, expanded.y, expanded.w, expanded.h).to_image();
    let gray = image::DynamicImage::ImageRgba8(cropped.clone()).to_luma8();
    // A figure's box shows its strokes: an (almost) inkless crop is no
    // picture of it.
    let ink = gray.pixels().filter(|p| p.0[0] < 160).count();
    if ink < 20.max((gray.width() * gray.height()) as usize / 2000) {
        return Err(CropReject::Blank);
    }
    if !ignore_grid && looks_like_answer_grid(&gray) {
        return Err(CropReject::AnswerGrid);
    }
    Ok(cropped)
}

pub fn crop_diagram_with_options(
    img: &image::DynamicImage,
    bbox: &[f32],
    padding: u32,
    ignore_grid: bool,
    graph_like: bool,
) -> Result<image::RgbaImage, CropReject> {
    let primary = crop_diagram_reading(img, bbox, padding, ignore_grid, graph_like);
    if primary.is_ok() {
        return primary;
    }

    // A box given as [x1, y1, x2, y2] rather than [x, y, width, height]: if
    // the primary crop is rejected, retry the other reading before discarding
    // the figure.
    // This fallback is deliberately limited to rejected crops so valid
    // width/height proposals retain their existing interpretation.
    if bbox.len() == 4 && bbox[2] > bbox[0] && bbox[3] > bbox[1] {
        let alternate = [
            bbox[0],
            bbox[1],
            bbox[2] - bbox[0],
            bbox[3] - bbox[1],
        ];
        if alternate
            .iter()
            .zip(bbox.iter())
            .any(|(alternate, original)| alternate != original)
        {
            if let Ok(crop) = crop_diagram_reading(img, &alternate, padding, ignore_grid, graph_like) {
                return Ok(crop);
            }
        }
    }

    primary
}

fn crop_diagram_reading(
    img: &image::DynamicImage,
    bbox: &[f32],
    padding: u32,
    ignore_grid: bool,
    graph_like: bool,
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

    // Scale padding proportionally to image resolution: we crop from the
    // full 300-DPI page, where a fixed 40px padding is too small, causing
    // truncation of labels and borders.
    let scale_factor = (img_w.max(img_h) as f32 / 1024.0).max(1.0);
    let scaled_padding = (padding as f32 * scale_factor).round() as u32;

    // Anti-clipping safety: ensure we have enough padding to protect
    // MCQ option letters (A/B/C/D) at edges, graph axis titles, and
    // diagram borders. Use a minimum padding that scales with image size.
    let min_padding = (img_w.min(img_h) as f32 * 0.02).round() as u32;  // 2% of min dimension
    let base_padding = scaled_padding.max(min_padding);

    let left_padding = if graph_like {
        base_padding.max((img_w as f32 * 0.10).round() as u32)
    } else {
        base_padding
    };
    let right_padding = base_padding;
    let top_padding = if graph_like {
        base_padding.max((img_h as f32 * 0.06).round() as u32)
    } else {
        base_padding
    };
    let bottom_padding = if graph_like {
        base_padding.max((img_h as f32 * 0.10).round() as u32)
    } else {
        base_padding
    };

    // For composit visual options (MCQ option boxes), add extra padding
    // on all sides to prevent clipping option letters
    let is_composite_options = graph_like; // graph_like is also true for composite_visual_options
    let (left_padding, right_padding, top_padding, bottom_padding) = if is_composite_options {
        // Extra padding for MCQ options
        (left_padding + 10, right_padding + 10, top_padding + 10, bottom_padding + 10)
    } else {
        (left_padding, right_padding, top_padding, bottom_padding)
    };

    let expanded = expand_rect(
        rect,
        left_padding,
        top_padding,
        right_padding,
        bottom_padding,
        img_w,
        img_h,
    );
    let safe_x = expanded.x;
    let safe_y = expanded.y;
    let safe_w = expanded.w;
    let safe_h = expanded.h;

    if safe_w < MIN_EDGE_PX || safe_h < MIN_EDGE_PX {
        return Err(CropReject::BadBox);
    }

    let mut owned = img.clone();
    let raw_cropped = image::imageops::crop(&mut owned, safe_x, safe_y, safe_w, safe_h).to_image();
    let cropped = trim_residual_text_edges(raw_cropped);

    if !ignore_grid {
        let gray = image::DynamicImage::ImageRgba8(cropped.clone()).to_luma8();
        if looks_like_answer_grid(&gray) {
            return Err(CropReject::AnswerGrid);
        }
    }
    Ok(cropped)
}

/// Check horizontal row densities to detect and trim single-line question text/headers
/// accidentally caught at the very top or bottom edge of a diagram crop.
pub fn trim_residual_text_edges(img: image::RgbaImage) -> image::RgbaImage {
    let (w, h) = (img.width(), img.height());
    if w < 50 || h < 60 {
        return img;
    }

    let gray = image::DynamicImage::ImageRgba8(img.clone()).to_luma8();
    
    // Row darkness profile: count of dark pixels (< 200) per row
    let mut row_dark_counts: Vec<u32> = Vec::with_capacity(h as usize);
    for y in 0..h {
        let mut dark = 0u32;
        for x in 0..w {
            if gray.get_pixel(x, y)[0] < 200 {
                dark += 1;
            }
        }
        row_dark_counts.push(dark);
    }

    let mut top_trim = 0u32;
    let max_search_h = (h as f32 * 0.18).round() as u32; // check up to top 18%

    let mut text_band_found = false;
    let mut gap_start = 0;
    for y in 0..max_search_h {
        let dark = row_dark_counts[y as usize];
        let frac = dark as f32 / w as f32;
        if frac > 0.03 && frac < 0.40 {
            text_band_found = true;
        } else if text_band_found && frac <= 0.005 {
            gap_start = y;
            break;
        }
    }

    if gap_start > 0 {
        let mut gap_len = 0;
        for y in gap_start..max_search_h {
            if row_dark_counts[y as usize] as f32 / w as f32 <= 0.005 {
                gap_len += 1;
            } else {
                break;
            }
        }
        if gap_len >= 6 {
            top_trim = gap_start + gap_len;
        }
    }

    let mut bottom_trim = 0u32;
    let mut bottom_text_found = false;
    let mut b_gap_start = 0;
    let search_bottom_start = h.saturating_sub(max_search_h);

    for y in (search_bottom_start..h).rev() {
        let dark = row_dark_counts[y as usize];
        let frac = dark as f32 / w as f32;
        if frac > 0.03 && frac < 0.40 {
            bottom_text_found = true;
        } else if bottom_text_found && frac <= 0.005 {
            b_gap_start = y;
            break;
        }
    }

    if b_gap_start > search_bottom_start {
        let mut gap_len = 0;
        for y in (search_bottom_start..=b_gap_start).rev() {
            if row_dark_counts[y as usize] as f32 / w as f32 <= 0.005 {
                gap_len += 1;
            } else {
                break;
            }
        }
        if gap_len >= 6 {
            bottom_trim = h.saturating_sub(b_gap_start.saturating_sub(gap_len));
        }
    }

    if top_trim > 0 || bottom_trim > 0 {
        let new_y = top_trim;
        let new_h = h.saturating_sub(top_trim).saturating_sub(bottom_trim);
        if new_h >= MIN_EDGE_PX {
            let mut owned = image::DynamicImage::ImageRgba8(img);
            return image::imageops::crop(&mut owned, 0, new_y, w, new_h).to_image();
        }
    }

    img
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
    fn an_exact_crop_with_no_ink_is_rejected() {
        let white = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(400, 400, image::Rgb([255, 255, 255])));
        assert!(matches!(crop_exact(&white, &[0.2, 0.2, 0.5, 0.5], true, 0.004), Err(CropReject::Blank)));
        let mut drawn = image::RgbImage::from_pixel(400, 400, image::Rgb([255, 255, 255]));
        for x in 100..300 {
            drawn.put_pixel(x, 200, image::Rgb([0, 0, 0]));
        }
        assert!(crop_exact(&image::DynamicImage::ImageRgb8(drawn), &[0.2, 0.2, 0.5, 0.5], true, 0.004).is_ok());
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
    fn corner_coordinates_are_preferred_when_width_height_crosses_page_edge() {
        let r = sanitize_bbox(&[0.17, 0.399, 0.77, 0.864], W, H).unwrap();
        assert_close(r.x, (0.17 * W as f32).round() as u32, 2);
        assert_close(r.y, (0.399 * H as f32).round() as u32, 2);
        assert_close(r.w, ((0.77 - 0.17) * W as f32).round() as u32, 2);
        assert_close(r.h, ((0.864 - 0.399) * H as f32).round() as u32, 2);

        let r = sanitize_bbox(&[0.17, 0.463, 0.78, 0.84], W, H).unwrap();
        assert_close(r.w, ((0.78 - 0.17) * W as f32).round() as u32, 2);
        assert_close(r.h, ((0.84 - 0.463) * H as f32).round() as u32, 2);
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
            let _ = crop_diagram(&img, b, 40, false); // must not panic
        }
    }

    /// Regression test: padding must scale with image resolution.
    /// A 300-DPI page (~3000x4000px) needs ~3x more padding than a 1024px
    /// downsampled image to capture the same labels/borders.
    #[test]
    fn crop_diagram_padding_scales_with_resolution() {
        // Low-res image (1024px longest edge) - should use base 40px padding
        let low_res = image::DynamicImage::new_rgba8(1024, 1024);
        let bbox = &[0.2, 0.2, 0.6, 0.6]; // 60% of image = 614px
        let crop_low = crop_diagram(&low_res, bbox, 40, true).unwrap();
        // With 40px padding on each side: 614 + 80 = 694px
        assert!(crop_low.width() >= 690 && crop_low.width() <= 700);

        // High-res image (3072px longest edge, 3x scale) - should use ~120px padding
        let high_res = image::DynamicImage::new_rgba8(3072, 3072);
        let crop_high = crop_diagram(&high_res, bbox, 40, true).unwrap();
        // With ~120px padding on each side: 1843 + 240 = 2083px
        assert!(crop_high.width() >= 2070 && crop_high.width() <= 2100);

        // Verify high-res crop is proportionally larger
        let scale_ratio = crop_high.width() as f32 / crop_low.width() as f32;
        assert!(
            scale_ratio >= 2.8 && scale_ratio <= 3.2,
            "High-res crop should be ~3x larger than low-res crop, got {:.2}x",
            scale_ratio
        );
    }

    #[test]
    fn graph_crops_reserve_margin_for_axis_labels() {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(W, H, |x, y| {
            image::Rgb([(x % 251) as u8, (y % 251) as u8, ((x + y) % 251) as u8])
        }));
        let bbox = &[0.2, 0.2, 0.6, 0.6];
        let regular = crop_diagram(&image, bbox, 40, true).unwrap();
        let graph = crop_diagram_with_options(&image, bbox, 40, true, true).unwrap();

        assert!(graph.width() > regular.width());
        assert!(graph.height() > regular.height());
    }

    #[test]
    fn graph_margin_expansion_moves_all_edges_outward_and_clamps() {
        let rect = PixelRect {
            x: 300,
            y: 400,
            w: 500,
            h: 600,
        };
        let expanded = expand_rect(rect, 100, 120, 140, 160, 1000, 1200);
        assert_eq!(expanded.x, 200);
        assert_eq!(expanded.y, 280);
        assert_eq!(expanded.x + expanded.w, 940);
        assert_eq!(expanded.y + expanded.h, 1160);

        let edge = expand_rect(rect, 500, 500, 500, 500, 700, 800);
        assert_eq!(edge.x, 0);
        assert_eq!(edge.y, 0);
        assert_eq!(edge.x + edge.w, 700);
        assert_eq!(edge.y + edge.h, 800);
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
            crop_diagram(&img, &[0.1, 0.1, 0.8, 0.8], 0, false),
            Err(CropReject::AnswerGrid)
        );

        let chart_page = image::DynamicImage::ImageLuma8(simple_chart());
        let r = crop_diagram(&chart_page, &[0.0, 0.1, 1.0, 0.7], 0, false);
        assert!(r.is_ok());

        // Test the bypass
        let r_bypass = crop_diagram(&img, &[0.1, 0.1, 0.8, 0.8], 0, true);
        assert!(r_bypass.is_ok(), "bypass must allow grid through");
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

    // ── Deterministic figure detection geometry ────────────────────────────

    #[test]
    fn normalize_pdf_box_flips_y_origin() {
        // PDF origin is bottom-left; y must flip to top-down, x unchanged.
        let b = normalize_pdf_box(100.0, 300.0, 700.0, 400.0, 800.0, 1000.0);
        assert!((b[0] - 0.125).abs() < 1e-5);
        assert!((b[1] - 0.30).abs() < 1e-5);
        assert!((b[2] - 0.25).abs() < 1e-5);
        assert!((b[3] - 0.30).abs() < 1e-5);
    }

    #[test]
    fn normalize_pdf_box_clamps_out_of_page_bounds() {
        let b = normalize_pdf_box(-50.0, 900.0, 1100.0, -50.0, 800.0, 1000.0);
        assert!(b[0] >= 0.0 && b[0] <= 1.0);
        assert!(b[1] >= 0.0 && b[1] <= 1.0);
        assert!(b[2] >= 0.0 && b[2] <= 1.0);
        assert!(b[3] >= 0.0 && b[3] <= 1.0);
    }

    #[test]
    fn rule_line_and_speck_are_rejected() {
        let text: [[f32; 4]; 0] = [];
        let rule = [0.20, 0.70, 0.60, 0.008]; // wide, thin → ruled answer line
        let speck = [0.50, 0.50, 0.005, 0.005]; // tiny → OCR noise
        let graph = [0.30, 0.35, 0.30, 0.25]; // legit figure
        assert!(!is_probable_figure_box(&rule, &text, 0.003, 0.015, 8.0, 0.4));
        assert!(!is_probable_figure_box(&speck, &text, 0.003, 0.015, 8.0, 0.4));
        assert!(is_probable_figure_box(&graph, &text, 0.003, 0.015, 8.0, 0.4));
    }

    #[test]
    fn text_dense_region_is_rejected() {
        // A box almost fully covered by text segments is question text, not a figure.
        let text_rects: [[f32; 4]; 3] = [
            [0.10, 0.10, 0.60, 0.05],
            [0.10, 0.16, 0.60, 0.05],
            [0.10, 0.22, 0.60, 0.05],
        ];
        assert!(text_density_in_box(&[0.10, 0.10, 0.60, 0.17], &text_rects) > 0.4);
        assert!(text_density_in_box(&[0.30, 0.40, 0.10, 0.10], &text_rects) < 0.4);
    }

    #[test]
    fn header_and_footer_boxes_are_rejected() {
        let text: [[f32; 4]; 0] = [];
        let header = [0.20, 0.02, 0.30, 0.02];
        let footer = [0.20, 0.93, 0.30, 0.05];
        let middle = [0.20, 0.30, 0.30, 0.20];
        assert!(!is_probable_figure_box(&header, &text, 0.003, 0.015, 8.0, 0.4));
        assert!(!is_probable_figure_box(&footer, &text, 0.003, 0.015, 8.0, 0.4));
        assert!(is_probable_figure_box(&middle, &text, 0.003, 0.015, 8.0, 0.4));
    }

    #[test]
    fn caption_kind_keywords() {
        assert_eq!(caption_kind_from_text("Figure 1: circuit diagram"), Some("circuit".into()));
        assert_eq!(caption_kind_from_text("Graph of y against x"), Some("graph".into()));
        assert_eq!(caption_kind_from_text("Flow chart of the process"), Some("flowchart".into()));
        assert_eq!(caption_kind_from_text("Figure 4"), None);
    }

    #[test]
    fn rect_metrics_and_gap() {
        let a = rect_pt(0.0, 10.0, 20.0, 0.0);
        let b = rect_pt(14.0, 24.0, 8.0, 4.0);
        assert!((rect_w(&a) - 10.0).abs() < 1e-5);
        assert!((rect_h(&a) - 20.0).abs() < 1e-5);
        // horizontal separation 4pt (a.right=10 → b.left=14), vertical overlap
        let g = rect_gap(&a, &b);
        assert!((g - 4.0).abs() < 1e-4, "gap {g}");
        // overlapping rects are gap-zero
        let c = rect_pt(5.0, 12.0, 15.0, 2.0);
        assert!(rect_gap(&a, &c) < 1e-6);
    }

    #[test]
    fn uniform_family_needs_regular_spacing() {
        // Four gridlines at exact 10pt pitch: every 3-line window is a
        // regular family (windows may overlap by design); their union must
        // cover all members.
        let regular: Vec<(f32, usize)> =
            [50.0, 60.0, 70.0, 80.0].iter().enumerate().map(|(i, &c)| (c, i)).collect();
        let fams = uniform_spacing_families(regular.clone(), 3.0, 3, 0.15);
        assert!(!fams.is_empty(), "regular pitch must form a family");
        let mut covered: Vec<usize> = fams.iter().flatten().copied().collect();
        covered.sort_unstable();
        covered.dedup();
        assert_eq!(covered, vec![0, 1, 2, 3]);

        // Irregular spacings fail the CV gate in every window.
        let irregular: Vec<(f32, usize)> =
            [50.0, 56.0, 74.0, 80.0].iter().enumerate().map(|(i, &c)| (c, i)).collect();
        let fams = uniform_spacing_families(irregular, 3.0, 3, 0.15);
        assert!(fams.is_empty(), "irregular gaps must not form a family");

        // Two members can never be a family.
        let pair: Vec<(f32, usize)> = vec![(50.0, 0), (60.0, 1)];
        assert!(uniform_spacing_families(pair, 3.0, 3, 0.15).is_empty());
    }

    #[test]
    fn corridor_blocked_crosses_text_barrier_only() {
        let a = rect_pt(0.0, 10.0, 100.0, 90.0);   // figure left
        let b = rect_pt(200.0, 210.0, 100.0, 90.0); // figure right
        let text_between = vec![rect_pt(90.0, 130.0, 95.0, 85.0)];
        assert!(corridor_blocked(&a, &b, &text_between, 6.0, 1.0));

        // No barrier in between → clear.
        assert!(!corridor_blocked(&a, &b, &[rect_pt(300.0, 310.0, 0.0, -10.0)], 6.0, 1.0));
        // No barriers at all → trivially clear.
        assert!(!corridor_blocked(&a, &b, &[], 6.0, 1.0));
    }

    #[test]
    fn expand_clamped_stops_at_body_text() {
        let barriers = vec![rect_pt(120.0, 180.0, 100.0, 90.0)];
        let cur = rect_pt(40.0, 110.0, 100.0, 60.0);
        // Goal wants to swallow the barrier entirely; the right edge must
        // stop before it, while the free top edge still expands.
        let goal = rect_pt(30.0, 200.0, 140.0, 50.0);
        let out = expand_clamped(&cur, &goal, &barriers, 2.0);
        assert!(out[1] <= barriers[0][0] + 1e-4, "right edge {} must stop at barrier left {}", out[1], barriers[0][0]);
        assert!((out[2] - goal[2]).abs() < 1e-4, "top edge expands freely");
        assert!((out[0] - goal[0]).abs() < 1e-4, "left edge expands freely");
    }
}
