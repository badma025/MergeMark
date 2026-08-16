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

use std::sync::LazyLock;

/// Minimum crop edge in pixels — anything smaller is a meaningless speck.
const MIN_EDGE_PX: u32 = 12;
/// Reject boxes covering more than this fraction of the page — that's a
/// whole-page misdetection, not a diagram.
const MAX_AREA_FRAC: f64 = 0.92;
/// Reject boxes smaller than this fraction of the page — OCR noise.
const MIN_AREA_FRAC: f64 = 0.0005;

/// Boilerplate patterns that should never be included in image bounding boxes.
/// These are matched against OCR text within or near the proposed crop region.
const BOILERPLATE_PATTERNS: &[&str] = &[
    r"(?i)do\s+not\s+write\s+(?:outside\s+the\s+box|in\s+this\s+area)",
    r"(?i)turn\s+over",
    r"(?i)question\s+\d+\s+continues\s+on\s+(?:the\s+)?next\s+page",
    r"(?i)do\s+not\s+write\s+on\s+this\s+page",
    r"(?i)blank\s+page",
    r"(?i)end\s+of\s+questions?",
    r"(?i)\[\s*\d+\s+marks?\s*\]",  // Mark allocations like [2 marks]
    r"(?i)total\s+for\s+question\s+\d+\s+is\s+\d+\s+marks?",
    r"(?i)page\s+\d+(\s*/\s*\d+)?",  // Page numbers
    r"(?i)ib\s*/\s*[a-z]\s*/\s*[a-z]{3}\d{2}\s*/\s*\d+\s*/\s*\d+",  // Footer codes like IB/M/Jun21/7408/2
];

static BOILERPLATE_REGEXES: LazyLock<Vec<regex::Regex>> = LazyLock::new(|| {
    BOILERPLATE_PATTERNS
        .iter()
        .map(|p| regex::Regex::new(p).unwrap())
        .collect()
});

#[allow(dead_code)]
static RE_MARKS: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\[\s*\d+\s+marks?\s*\]").unwrap()
});

#[allow(dead_code)]
static RE_CONT: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)question\s+\d+\s+continues\s+on\s+(?:the\s+)?next\s+page").unwrap()
});

static RE_MULTI_FIG: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)(?:fig(?:ure)?\.?\s*\d+[\s,]*(?:and|,|-)\s*)+fig(?:ure)?\.?\s*\d+").unwrap()
});

static RE_FIG_NUM: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)fig(?:ure)?\.?\s*(\d+)").unwrap()
});

/// Detect if a text region contains boilerplate that should be excluded from image crops.
fn contains_boilerplate(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    for re in BOILERPLATE_REGEXES.iter() {
        if re.is_match(&lower) {
            return true;
        }
    }
    false
}

/// Conservatively clamp a proposed bounding box into the safe printable area,
/// preventing accidental bleed into page headers, footers (Turn over, page numbers),
/// and side margins without triggering expensive LLM repair loops.
pub fn clamp_bbox_safe(bbox: &mut [f32]) {
    if bbox.len() != 4 {
        return;
    }
    const MIN_Y: f32 = 0.03;
    const MAX_Y: f32 = 0.93;
    const MIN_X: f32 = 0.03;
    const MAX_X: f32 = 0.97;

    let mut x = bbox[0].clamp(0.0, 1.0);
    let mut y = bbox[1].clamp(0.0, 1.0);
    let mut w = bbox[2].clamp(0.01, 1.0);
    let mut h = bbox[3].clamp(0.01, 1.0);

    // If y extends into header
    if y < MIN_Y {
        let diff = MIN_Y - y;
        y = MIN_Y;
        h = (h - diff).max(0.02);
    }
    // If y + h extends into footer
    if y + h > MAX_Y {
        h = (MAX_Y - y).max(0.02);
    }

    // If x extends into left margin
    if x < MIN_X {
        let diff = MIN_X - x;
        x = MIN_X;
        w = (w - diff).max(0.02);
    }
    // If x + w extends into right margin
    if x + w > MAX_X {
        w = (MAX_X - x).max(0.02);
    }

    bbox[0] = x;
    bbox[1] = y;
    bbox[2] = w;
    bbox[3] = h;
}

/// Check if a proposed bounding box (in relative coordinates 0..1) lies entirely
/// in a forbidden boilerplate zone (e.g. top header or bottom footer with page codes).
pub fn bbox_contains_boilerplate(page_text: &str, bbox: &[f32]) -> bool {
    if bbox.len() != 4 || page_text.trim().is_empty() {
        return false;
    }

    let bbox_y = bbox[1];
    let bbox_h = bbox[3];

    // Check if the entire bbox sits inside the top margin header zone (< 0.04)
    if bbox_y + bbox_h <= 0.04 && contains_boilerplate(page_text) {
        return true;
    }

    // Check if the entire bbox sits inside the bottom footer zone (>= 0.94)
    if bbox_y >= 0.94 && contains_boilerplate(page_text) {
        return true;
    }

    false
}

/// Heuristic: detect student answer lines / blank working space within a proposed crop.
/// Returns true if the crop appears to contain primarily answer lines (underscores,
/// horizontal rules, dotted lines, empty grids) rather than a diagram.
pub fn looks_like_answer_space(crop: &image::RgbaImage) -> bool {
    let gray = image::DynamicImage::ImageRgba8(crop.clone()).to_luma8();
    let (w, h) = gray.dimensions();
    if w < 40 || h < 40 {
        return false;
    }

    // Check for horizontal line patterns (answer lines)
    let mut _horizontal_lines = 0;
    let mut long_horizontal_lines = 0;
    for y in 0..h {
        let mut run = 0;
        let mut max_run = 0;
        for x in 0..w {
            if gray.get_pixel(x, y)[0] < 128 {
                run += 1;
                max_run = max_run.max(run);
            } else {
                run = 0;
            }
        }
        if max_run as f32 > w as f32 * 0.5 {
            _horizontal_lines += 1;
            if max_run as f32 > w as f32 * 0.8 {
                long_horizontal_lines += 1;
            }
        }
    }

    // If we have multiple long horizontal lines spanning most of the width,
    // it's likely answer lines / working space
    long_horizontal_lines >= 3
}

/// Heuristic: detect if a crop contains MCQ option letters (A, B, C, D) that would be clipped.
#[allow(dead_code)]
fn contains_mcq_options(_crop: &image::RgbaImage) -> bool {
    // This is a cheap check - we'd need OCR to be certain, but we can
    // use the caption text from the model to detect this case
    false  // Placeholder - actual detection happens at a higher level
}

/// Result of splitting a compound figure into separate visual assets.
#[derive(Debug, Clone)]
pub struct SplitFigure {
    pub bbox: Vec<f32>,           // [x, y, w, h] in relative coordinates
    pub caption: Option<String>,  // Associated caption (e.g., "Figure 4")
    pub kind: String,             // "graph", "schema", "diagram", etc.
}

/// Attempt to split a compound bounding box into separate visual assets.
/// Detects multiple distinct captions (e.g., "Figure 4" and "Figure 5") and
/// uses whitespace gutters to determine split boundaries.
pub fn split_compound_figure(
    bboxes: &[Vec<f32>],
    captions: &[String],
    kinds: &[String],
    _page_texts: &[String],  // OCR text from each page for caption alignment
) -> Vec<SplitFigure> {
    if bboxes.is_empty() {
        return Vec::new();
    }

    // If there's only one bbox and one caption, check if the caption indicates
    // multiple figures (e.g., "Figure 4 and Figure 5" or "Figures 4-5")
    if bboxes.len() == 1 && captions.len() == 1 {
        let caption = &captions[0];
        let bbox = &bboxes[0];

        // Check for multiple figure references in caption
        if let Some(split) = try_split_by_caption(bbox, caption) {
            return split;
        }
    }

    // If multiple bboxes already exist, try to merge very close ones that
    // belong to the same figure, and split ones that have distinct captions
    let mut figures = Vec::new();
    for (i, bbox) in bboxes.iter().enumerate() {
        let cap = captions.get(i).cloned();
        let kind = kinds.get(i).cloned().unwrap_or_else(|| "diagram".to_string());
        if let Some(split) = try_split_by_caption(bbox, cap.as_deref().unwrap_or("")) {
            figures.extend(split);
        } else {
            figures.push(SplitFigure {
                bbox: bbox.clone(),
                caption: cap,
                kind,
            });
        }
    }

    // Now try to merge figures that are close and have related captions
    merge_related_figures(figures)
}

/// Try to split a single bbox by detecting multiple figure references in the caption.
fn try_split_by_caption(bbox: &[f32], caption: &str) -> Option<Vec<SplitFigure>> {
    // Pattern: "Figure 4 and Figure 5", "Figures 4-5", "Figure 4, Figure 5", etc.
    if !RE_MULTI_FIG.is_match(caption) {
        return None;
    }

    // Extract all figure numbers mentioned
    let fig_nums: Vec<u32> = RE_FIG_NUM.captures_iter(caption)
        .filter_map(|c| c[1].parse::<u32>().ok())
        .collect();

    if fig_nums.len() < 2 {
        return None;
    }

    // Heuristic: if the bbox is tall, likely figures are stacked vertically
    // If wide, likely side by side. Split proportionally.
    let is_tall = bbox[3] > bbox[2];  // height > width
    let num = fig_nums.len();

    let mut result = Vec::new();
    for (idx, &fig_num) in fig_nums.iter().enumerate() {
        let (x, y, w, h) = if is_tall {
            // Vertical split: each figure gets equal height portion
            let x = bbox[0];
            let y = bbox[1] + bbox[3] * (idx as f32 / num as f32);
            let w = bbox[2];
            let h = bbox[3] / num as f32;
            (x, y, w, h)
        } else {
            // Horizontal split: side by side
            let x = bbox[0] + bbox[2] * (idx as f32 / num as f32);
            let y = bbox[1];
            let w = bbox[2] / num as f32;
            let h = bbox[3];
            (x, y, w, h)
        };
        result.push(SplitFigure {
            bbox: vec![x, y, w, h],
            caption: Some(format!("Figure {}", fig_num)),
            kind: "diagram".to_string(),
        });
    }

    Some(result)
}

/// Merge related figures that are close together and likely part of the same visual asset.
fn merge_related_figures(mut figures: Vec<SplitFigure>) -> Vec<SplitFigure> {
    // Simple greedy merge: if two figures overlap significantly or touch,
    // and have the same kind, merge them
    let mut merged = true;
    while merged {
        merged = false;
        for i in 0..figures.len() {
            for j in (i + 1)..figures.len() {
                if should_merge(&figures[i], &figures[j]) {
                    figures[i] = merge_two(&figures[i], &figures[j]);
                    figures.remove(j);
                    merged = true;
                    break;
                }
            }
            if merged { break; }
        }
    }
    figures
}

fn should_merge(a: &SplitFigure, b: &SplitFigure) -> bool {
    if a.kind != b.kind {
        return false;
    }

    // Check if bboxes overlap or are very close
    let (ax, ay, aw, ah) = (a.bbox[0], a.bbox[1], a.bbox[2], a.bbox[3]);
    let (bx, by, bw, bh) = (b.bbox[0], b.bbox[1], b.bbox[2], b.bbox[3]);

    let a_right = ax + aw;
    let a_bottom = ay + ah;
    let b_right = bx + bw;
    let b_bottom = by + bh;

    // Check overlap
    let overlap_x = (ax.min(b_right) - a_right.max(bx)).max(0.0);
    let overlap_y = (ay.min(b_bottom) - a_bottom.max(by)).max(0.0);
    let overlap_area = overlap_x * overlap_y;

    let a_area = aw * ah;
    let b_area = bw * bh;

    // If significant overlap, merge
    if overlap_area > 0.1 * a_area.min(b_area) {
        return true;
    }

    // If touching (very close edges) and same caption prefix
    let close_x = (ax - b_right).abs().min((bx - a_right).abs());
    let close_y = (ay - b_bottom).abs().min((by - a_bottom).abs());

    close_x < 0.02 && close_y < 0.02
}

fn merge_two(a: &SplitFigure, b: &SplitFigure) -> SplitFigure {
    let (ax, ay, aw, ah) = (a.bbox[0], a.bbox[1], a.bbox[2], a.bbox[3]);
    let (bx, by, bw, bh) = (b.bbox[0], b.bbox[1], b.bbox[2], b.bbox[3]);

    let x = ax.min(bx);
    let y = ay.min(by);
    let right = (ax + aw).max(bx + bw);
    let bottom = (ay + ah).max(by + bh);

    let caption = match (&a.caption, &b.caption) {
        (Some(ca), Some(cb)) if ca == cb => Some(ca.clone()),
        (Some(ca), None) => Some(ca.clone()),
        (None, Some(cb)) => Some(cb.clone()),
        _ => a.caption.clone().or(b.caption.clone()),
    };

    SplitFigure {
        bbox: vec![x, y, right - x, bottom - y],
        caption,
        kind: a.kind.clone(),
    }
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
    // Some vision providers emit corner coordinates even though the schema
    // requests width/height. If the width/height reading crosses the page
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

        // Apply conservative 2% safety padding so axis ticks, bounds, and labels are never clipped
        let pad_x = ((x1 - x0) * 0.02).max(0.003);
        let pad_y = ((y1 - y0) * 0.02).max(0.003);
        let padded_x0 = (x0 - pad_x).max(0.0);
        let padded_y0 = (y0 - pad_y).max(0.0);
        let padded_x1 = (x1 + pad_x).min(1.0);
        let padded_y1 = (y1 + pad_y).min(1.0);

        // Rounding (not floor/ceil) makes pixel-scale round-trips stable.
        let px = (padded_x0 * img_w as f32).round().max(0.0) as u32;
        let py = (padded_y0 * img_h as f32).round().max(0.0) as u32;
        let px = px.min(img_w.saturating_sub(1));
        let py = py.min(img_h.saturating_sub(1));

        // x0 < 1.0 / y0 < 1.0 were checked above, so the far edges are >= the
        // origin; saturating_sub is belt-and-braces.
        let far_x = (padded_x1 * img_w as f32).round().max(0.0) as u32;
        let far_y = (padded_y1 * img_h as f32).round().max(0.0) as u32;
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

/// Return one normalized [x, y, width, height] box enclosing visual option
/// boxes. Inputs are clamped to the normalized page and malformed entries are
/// ignored; callers can therefore use this at the JSON boundary safely.
pub fn union_relative_bboxes(boxes: &[Vec<f32>]) -> Option<Vec<f32>> {
    let mut min_x = 1.0_f32;
    let mut min_y = 1.0_f32;
    let mut max_x = 0.0_f32;
    let mut max_y = 0.0_f32;
    let mut valid = false;

    for bbox in boxes {
        if bbox.len() != 4 || bbox.iter().any(|value| !value.is_finite()) {
            continue;
        }
        let x = bbox[0].clamp(0.0, 1.0);
        let y = bbox[1].clamp(0.0, 1.0);
        let right = (bbox[0] + bbox[2]).clamp(0.0, 1.0);
        let bottom = (bbox[1] + bbox[3]).clamp(0.0, 1.0);
        if right <= x || bottom <= y {
            continue;
        }
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(right);
        max_y = max_y.max(bottom);
        valid = true;
    }

    valid.then(|| vec![min_x, min_y, max_x - min_x, max_y - min_y])
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

/// Crop a diagram with optional graph-canvas margins. Graphs need asymmetric
/// room outside the plotted rectangle for vertical axis titles/units on the
/// left and tick labels/axis titles below the x-axis.
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

    // Vision providers sometimes return [x1, y1, x2, y2] despite the
    // requested [x, y, width, height] schema. If the primary crop is rejected,
    // retry the equivalent width/height reading before discarding the figure.
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

    // Scale padding proportionally to image resolution. The vision model
    // returns bboxes based on a downsampled 1024px image, but we crop from
    // the full 300-DPI page. A fixed 40px padding is too small for high-res
    // images, causing truncation of labels and borders.
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
    /// Fractional height of the cropped region relative to source image.
    pub height_frac: f32,
}

/// Crop a page image to a vertical band. Returns None if the band is
/// degenerate (empty or outside the page). `start_frac`/`end_frac` are
/// clamped to [0,1] and padded by a small margin to avoid chopping the
/// top/bottom lines of the question.
#[allow(dead_code)]
pub fn crop_page_vertical(b64: &str, start_frac: f32, end_frac: f32) -> Option<PageBand> {
    let img = decode_page_image(b64)?;
    crop_page_vertical_from_image(&img, start_frac, end_frac)
}

/// Crop an already-decoded page image to a vertical band. This is the
/// allocation-heavy half of `crop_page_vertical`; callers that also need the
/// decoded full page can now decode once and retain it for diagram auditing.
pub fn crop_page_vertical_from_image(
    img: &image::DynamicImage,
    start_frac: f32,
    end_frac: f32,
) -> Option<PageBand> {
    use base64::Engine;
    use image::GenericImageView;
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

    let cropped_rgba = image::imageops::crop_imm(img, 0, y0, w, band_h).to_image();
    let cropped = image::DynamicImage::ImageRgba8(cropped_rgba).to_rgb8();

    // Scale to optimal vision tile dimension (max_dim: 768) to prevent tile explosions
    let max_dim: u32 = 768;
    let (cw, ch) = (cropped.width(), cropped.height());
    let final_img = if cw > max_dim || ch > max_dim {
        let scale = max_dim as f32 / (cw.max(ch) as f32);
        let new_w = (cw as f32 * scale).round().max(1.0) as u32;
        let new_h = (ch as f32 * scale).round().max(1.0) as u32;
        image::DynamicImage::ImageRgb8(image::imageops::resize(
            &cropped,
            new_w,
            new_h,
            image::imageops::FilterType::Triangle,
        ))
    } else {
        image::DynamicImage::ImageRgb8(cropped)
    };

    // Re-encode as WebP format.
    // WebP is ~30% smaller than JPEG at equivalent fidelity, reducing payload
    // size and upload time to vision APIs.
    let mut buf = std::io::Cursor::new(Vec::with_capacity((final_img.width() as usize * final_img.height() as usize) / 8));
    final_img.write_to(&mut buf, image::ImageFormat::WebP).ok()?;
    let out_b64 = base64::engine::general_purpose::STANDARD.encode(buf.into_inner());

    Some(PageBand {
        b64: out_b64,
        y_offset_px: y0,
        height_px: band_h,
        y_offset_frac: y0 as f32 / h as f32,
        height_frac: band_h as f32 / h as f32,
    })
}

/// Fast heuristic to check if a page image is completely blank or near-blank (e.g., blank exam page).
/// Samples pixels on a regular grid; returns true if almost all pixels are white / background.
pub fn is_image_blank(img: &image::DynamicImage) -> bool {
    use image::GenericImageView;
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return true;
    }
    let step_x = (w / 100).max(4);
    let step_y = (h / 100).max(4);
    let mut total_samples = 0usize;
    let mut dark_pixels = 0usize;

    for y in (0..h).step_by(step_y as usize) {
        for x in (0..w).step_by(step_x as usize) {
            total_samples += 1;
            let pixel = img.get_pixel(x, y);
            let r = pixel[0] as u32;
            let g = pixel[1] as u32;
            let b = pixel[2] as u32;
            let a = pixel[3];
            if a > 32 {
                let lum = (299 * r + 587 * g + 114 * b) / 1000;
                if lum < 235 {
                    dark_pixels += 1;
                }
            }
        }
    }

    if total_samples == 0 {
        return true;
    }

    let dark_ratio = dark_pixels as f32 / total_samples as f32;
    dark_ratio < 0.0015
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
    use base64::Engine;

    const W: u32 = 1654;
    const H: u32 = 2339;

    fn assert_close(a: u32, b: u32, tol: u32) {
        let d = (a as i64 - b as i64).abs();
        assert!(d <= tol as i64, "{a} not within {tol} of {b}");
    }

    #[test]
    fn decoded_vertical_crop_matches_base64_entry_point() {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(320, 480, |x, y| {
            image::Rgb([(x % 255) as u8, (y % 255) as u8, 180])
        }));
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(png.into_inner());

        let from_b64 = crop_page_vertical(&b64, 0.2, 0.7).unwrap();
        let decoded = decode_page_image(&b64).unwrap();
        let from_decoded = crop_page_vertical_from_image(&decoded, 0.2, 0.7).unwrap();

        assert_eq!(from_b64.y_offset_px, from_decoded.y_offset_px);
        assert_eq!(from_b64.height_px, from_decoded.height_px);
        assert_eq!(from_b64.y_offset_frac, from_decoded.y_offset_frac);
        assert_eq!(from_b64.height_frac, from_decoded.height_frac);
        assert_eq!(from_b64.b64, from_decoded.b64);
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

    #[test]
    fn union_relative_bboxes_encloses_visual_options_and_clamps() {
        let union = union_relative_bboxes(&[
            vec![0.10, 0.20, 0.30, 0.15],
            vec![0.15, 0.40, 0.70, 0.20],
            vec![0.05, 0.65, 0.90, 0.40],
        ])
        .unwrap();
        assert_eq!(union, vec![0.05, 0.20, 0.90, 0.80]);
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
    fn conservative_clamping_protects_margins_and_footers() {
        let mut box_overshooting_footer = vec![0.1, 0.5, 0.8, 0.48]; // reaches y=0.98 (footer)
        clamp_bbox_safe(&mut box_overshooting_footer);
        assert!(box_overshooting_footer[1] + box_overshooting_footer[3] <= 0.93 + 0.001);
        assert!(box_overshooting_footer[3] > 0.02);

        let mut box_overshooting_header = vec![0.1, 0.01, 0.8, 0.4]; // reaches y=0.01 (header)
        clamp_bbox_safe(&mut box_overshooting_header);
        assert!(box_overshooting_header[1] >= 0.03 - 0.001);

        let mut box_overshooting_left_margin = vec![0.005, 0.2, 0.5, 0.4];
        clamp_bbox_safe(&mut box_overshooting_left_margin);
        assert!(box_overshooting_left_margin[0] >= 0.03 - 0.001);
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
