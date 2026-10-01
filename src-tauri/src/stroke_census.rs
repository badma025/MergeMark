// ── Smart Scissors: typed-stroke figure segmentation ────────────────────────
//
// Replaces anonymous-bbox clustering (`cluster_boxes`) with a zero-AI
// pipeline per page:
//
//   1. CENSUS      every Path/Image/XObjectForm object is measured
//                  (segment counts, ink length, direction histogram) and
//                  typed: Bitmap · Rule · GridMember · Curve · Glyph.
//   2. FAMILIES    answer-line stacks (barriers, never seeds) and uniform-
//                  spacing gridline families (GridMember) are identified.
//   3. BARRIERS    positively-identified obstacles — body-text blocks,
//                  question headings, marks footers, answer-line stacks,
//                  header/footer bands — that region growth may not cross.
//   4. GROWING     strong primitives seed regions; decorative strokes join
//                  only alongside a region across a clear corridor.
//                  Over-clustering is impossible because growth cannot cross
//                  barriers; under-clustering dies because touching
//                  primitives merge regardless of type.
//   5. LABELS      short floating text inside a region (axis numbers,
//                  circuit labels) is captured and locked into the crop,
//                  expanding the bbox until it meets a barrier.
//   6. SCORING     deterministic confidence per figure, recorded as
//                  `DetectedFigure.seg_confidence`.
//
// All clustering happens in PDF point space (origin bottom-left, y up);
// rects are `[left, right, top, bottom]`. Conversion to the app's normalized
// [x, y, w, h] happens once at emission via `normalize_pdf_box`. Page
// rotation is normalized up front so boxes always match how
// `render_page_from_document` draws the page (crops are cut from renders).

use std::collections::HashSet;
use std::sync::OnceLock;

use pdfium_render::prelude::*;

use crate::geometry::{
    self, corridor_blocked, expand_clamped, rect_area, rect_center, rect_gap, rect_h, rect_pt,
    rect_union, rect_w, uniform_spacing_families, x_span_coverage, RectPt,
};

// ── Constants (single source; plan §Constants) ──────────────────────────────

/// Max centre-to-centre separation for any join during growth.
const JOIN_GAP_PT: f32 = 6.0;
/// Corridor sampling pitch for barrier checks.
const CORRIDOR_SAMPLE_PT: f32 = 6.0;
/// Barrier inflation applied during corridor sampling.
const CORRIDOR_PAD_PT: f32 = 1.0;
/// Minimum parallel strokes forming a gridline family.
const GRID_MIN_MEMBERS: usize = 4;
/// Max coefficient of variation of gridline spacing.
const GRID_SPACING_CV_MAX: f32 = 0.15;
/// Centre tolerance when binning projection-histogram peaks.
const GRID_TOL_PT: f32 = 3.0;
/// Pitch range of ruled answer lines (boards print them 18–30pt apart).
const ANSWER_LINE_PITCH_MIN_PT: f32 = 18.0;
const ANSWER_LINE_PITCH_MAX_PT: f32 = 30.0;
/// Stacked rules shorter than this are decoration, not answer space.
const ANSWER_LINE_MIN_COUNT: usize = 3;
/// A text block this wordy is prose (barrier), never a diagram label.
const BODY_MIN_WORDS: usize = 8;
/// …or this many characters (catches dense formula lines without spaces).
const BODY_MIN_CHARS: usize = 40;
/// Padding added around a captured internal label.
const LABEL_PAD_PT: f32 = 3.0;
/// Padding around body/heading barrier rects.
const BARRIER_PAD_PT: f32 = 2.0;
/// Padding around answer-line-stack barrier rects.
const STACK_PAD_PT: f32 = 4.0;
/// Header band (top fraction) excluded from figures — matches the legacy gate.
const HEADER_FRAC: f32 = 0.05;
/// Footer band (bottom fraction) excluded from figures.
const FOOTER_FRAC: f32 = 0.08;
/// Default ceiling for regions without a trusted standalone caption.
const MAX_FIGURE_AREA_FRAC: f32 = 0.5;
/// Aspect-ratio ceiling carried over from `is_probable_figure_box`.
const MAX_ASPECT: f32 = 8.0;
/// Ink length below which a small object is a glyph (dot, tick, arrowhead).
const MIN_SEED_INK_PT: f32 = 14.0;
/// Area fraction below which a small object is a glyph.
const MIN_SEED_AREA_FRAC: f32 = 0.0015;
/// A decoration may extend the region it joins by at most this factor along
/// its own long axis — stops page-wide divider rules welding themselves
/// onto a mid-page figure (extension ≈ 3×) while true axes/graph frames
/// barely extend anything (≈ 1.1×).
const DECORATION_EXTENSION_MAX: f32 = 2.0;
/// Board papers print "Figure N" one-to-two text lines BELOW the graphic.
/// Captions within this vertical reach of the bbox are bound to it (and
/// pulled into the crop); anything farther belongs to another element.
const CAPTION_SNAP_PT: f32 = 26.0;

/// Vertical window above a region within which a printed MCQ option letter
/// ("A".."E") is treated as that region's option label.
const OPTION_LABEL_SNAP_PT: f32 = 34.0;
/// Horizontal slack for the caption snap window.
const CAPTION_SNAP_SIDE_PT: f32 = 10.0;
/// Offset tag marking block indices inside `Region.labels` (which otherwise
/// stores primitive indices).
const BLOCK_TAG_OFFSET: usize = 100_000;

// ── Primitive typing ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Prim {
    /// Embedded bitmap (or opaque XObject form) — strongest seed.
    Bitmap,
    /// Long straight axis-aligned rule: borders, underlines, table rules.
    Rule,
    /// Member of a uniform-spacing parallel family (graph gridlines).
    GridMember,
    /// Multi-segment / curved drawing primitive — the heart of a figure.
    Curve,
    /// Tiny mark: arrowhead, dot, tick, small symbol group.
    Glyph,
}

impl Prim {
    /// Seed strength: decoration (Rule/GridMember) can never seed a region.
    pub(crate) fn strength(self) -> u8 {
        match self {
            Prim::Bitmap => 3,
            Prim::Curve => 2,
            Prim::Glyph => 1,
            Prim::Rule | Prim::GridMember => 0,
        }
    }
}

/// Measured properties of one vector path object.
#[derive(Debug, Clone)]
pub(crate) struct StrokeTelemetry {
    pub bbox: RectPt,
    pub line_count: u32,
    pub bezier_count: u32,
    /// Σ chord lengths across subpaths (MoveTo resets the pen).
    pub ink_len_pt: f32,
    /// Direction mass in eight 45° bins over the full circle.
    pub dir_hist: [f32; 8],
    pub closed: bool,
    pub stroked: bool,
}

pub(crate) struct TypedPrim {
    pub bbox: RectPt,
    pub prim: Prim,
    pub ink_len_pt: f32,
}

impl TypedPrim {
    fn strength(&self) -> u8 {
        self.prim.strength()
    }

    pub(crate) fn new(bbox: RectPt, prim: Prim, ink_len_pt: f32) -> Self {
        TypedPrim {
            bbox,
            prim,
            ink_len_pt,
        }
    }
}

/// Classify one measured stroke into its primitive type. Ordered rules;
/// the first hit wins.
pub(crate) fn classify_stroke(t: &StrokeTelemetry, page_area_pt: f32) -> Prim {
    let w = rect_w(&t.bbox);
    let h = rect_h(&t.bbox);
    let long = w.max(h);
    let short = w.min(h);

    // Rule: straight, few segments, overwhelmingly axis-aligned, needle-shaped.
    let total_mass: f32 = t.dir_hist.iter().sum();
    if total_mass > 0.0 && t.line_count <= 4 && t.bezier_count == 0 && t.stroked && long > 0.0 {
        let horiz = t.dir_hist[0] + t.dir_hist[4];
        let vert = t.dir_hist[2] + t.dir_hist[6];
        let dominance = horiz.max(vert) / total_mass;
        if dominance > 0.9 && short / long < 0.05 {
            return Prim::Rule;
        }
    }

    // Curve: beziers present, or enough direction changes to be drawing.
    if t.bezier_count > 0 {
        return Prim::Curve;
    }
    if t.line_count >= 3 && direction_entropy(&t.dir_hist) > 1.2 {
        return Prim::Curve;
    }

    // Glyph: specks, ticks, arrowheads, small symbols.
    if long > 0.0 {
        let area_frac = rect_area(&t.bbox) / page_area_pt.max(1.0);
        if area_frac < MIN_SEED_AREA_FRAC && t.ink_len_pt < MIN_SEED_INK_PT {
            return Prim::Glyph;
        }
    }

    // Remaining medium marks (diagonal rays, small symbol groups) seed weakly.
    Prim::Glyph
}

fn direction_entropy(hist: &[f32; 8]) -> f32 {
    let total: f32 = hist.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    hist.iter()
        .filter(|m| **m > 0.0)
        .map(|m| {
            let p = m / total;
            -p * p.log2()
        })
        .sum()
}

// ── Text blocks ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockClass {
    Body,
    Heading,
    Caption,
    LabelCandidate,
}

#[derive(Debug, Clone)]
pub(crate) struct TextBlock {
    pub rect: RectPt,
    pub text: String,
    pub class: BlockClass,
}

/// Greedy line assembly: segments sharing a baseline (within 60% of the
/// taller height) and horizontally adjacent merge into one line. Reading
/// order (top-to-bottom, left-to-right) keeps merging deterministic.
pub(crate) fn assemble_lines(mut segs: Vec<(RectPt, String)>) -> Vec<(RectPt, String)> {
    segs.sort_by(|a, b| {
        let (ca, cb) = (rect_center(&a.0), rect_center(&b.0));
        ca[1]
            .partial_cmp(&cb[1])
            .unwrap_or(std::cmp::Ordering::Equal)
            .reverse()
            .then(ca[0].partial_cmp(&cb[0]).unwrap_or(std::cmp::Ordering::Equal))
    });

    let mut lines: Vec<(RectPt, String)> = Vec::new();
    for (rect, text) in segs {
        let h = rect_h(&rect).max(1.0);
        let mut merged = false;
        for line in lines.iter_mut().rev() {
            let lh = rect_h(&line.0).max(1.0);
            let cy = rect_center(&rect)[1];
            let ly = rect_center(&line.0)[1];
            let same_baseline = (cy - ly).abs() <= 0.6 * lh.max(h);
            // Same-line gaps are small (word spaces); a generous reach here
            // welds AQA margin numerals onto captions across whitespace.
            let near = rect_gap(&line.0, &rect) <= (1.5 * lh.max(h)).max(12.0);
            if same_baseline && near {
                line.0 = rect_union(&line.0, &rect);
                if !line.1.is_empty() && !text.is_empty() {
                    line.1.push(' ');
                }
                line.1.push_str(&text);
                merged = true;
                break;
            }
        }
        if !merged {
            lines.push((rect, text));
        }
    }
    lines
}

/// Merge assembled lines into multi-line blocks (paragraphs, headings):
/// vertically close lines with substantial horizontal overlap coalesce.
pub(crate) fn lines_to_blocks(lines: &[(RectPt, String)]) -> Vec<(RectPt, String)> {
    let mut blocks: Vec<(RectPt, String)> = Vec::new();
    for (rect, text) in lines {
        let mut merged = false;
        for block in blocks.iter_mut().rev() {
            let bh = rect_h(&block.0).max(1.0);
            let gap = block.0[3] - rect[2]; // prev.bottom − next.top (positive = space)
            let v_close = gap >= -1.0 && gap <= 0.6 * bh.max(rect_h(rect));
            let overlaps =
                x_span_coverage(rect, &block.0) > 0.5 || x_span_coverage(&block.0, rect) > 0.5;
            if v_close && overlaps {
                block.0 = rect_union(&block.0, rect);
                block.1.push(' ');
                block.1.push_str(text);
                merged = true;
                break;
            }
        }
        if !merged {
            blocks.push((*rect, text.clone()));
        }
    }
    blocks
}

fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

/// Classify one assembled block. Order matters: positive evidence promotes a
/// block out of label-hood; labels remain the default.
pub(crate) fn classify_block(rect: &RectPt, text: &str) -> TextBlock {
    static TOTAL_RE: OnceLock<regex::Regex> = OnceLock::new();
    let total_re = TOTAL_RE
        .get_or_init(|| regex::Regex::new(r"(?i)\btotal\s+for\s+(?:this\s+)?question\b").unwrap());

    let class = if matches_caption(text) {
        BlockClass::Caption
    } else if total_re.is_match(text)
        || word_count(text) >= BODY_MIN_WORDS
        || text.trim().len() >= BODY_MIN_CHARS
    {
        BlockClass::Body
    } else if heading_block(text) {
        BlockClass::Heading
    } else {
        BlockClass::LabelCandidate
    };

    TextBlock {
        rect: *rect,
        text: text.to_string(),
        class,
    }
}

/// Heading-block test: the doc-map heading regex must match AND the block
/// must be essentially just the number (optionally punctuated). This rejects
/// quantity/margin labels like "10 N" / "2F" that the page-level regex
/// tolerates with surrounding prose but which are never headings on their own.
fn heading_block(text: &str) -> bool {
    let t = text.trim();
    if !crate::doc_map::QUESTION_HEADING_REGEX.is_match(t) {
        return false;
    }
    let bytes = t.as_bytes();
    let mut i = 0;
    while i < bytes.len() && !bytes[i].is_ascii_digit() {
        i += 1;
    }
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    // Everything after the digit run must be punctuation-only ("7.", "3)").
    t[i..]
        .trim()
        .trim_end_matches(['.', ')', ']', ':'])
        .trim()
        .is_empty()
}

/// True when a text segment looks like a "Figure N" / "Fig. N" caption.
pub(crate) fn matches_caption(text: &str) -> bool {
    static RE_CAPTION: OnceLock<regex::Regex> = OnceLock::new();
    RE_CAPTION
        .get_or_init(|| regex::Regex::new(r"(?i)^\s*fig(?:ure)?\.?\s*\d+[.:]?\s*$").unwrap())
        .is_match(text)
}

// ── Structural families ─────────────────────────────────────────────────────

/// Detect stacked horizontal answer-line runs and return their barrier rects
/// plus the indices of the consumed rules (excluded from figure absorption
/// and from grid-family retyping — an answer-line stack IS a uniform family;
/// it must never masquerade as graph gridlines).
pub(crate) fn detect_answer_line_stacks(
    prims: &[TypedPrim],
    column_width_pt: f32,
) -> (Vec<RectPt>, HashSet<usize>) {
    let mut candidates: Vec<usize> = prims
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            p.prim == Prim::Rule
                && rect_w(&p.bbox) > rect_h(&p.bbox) * 5.0
                && column_width_pt > 0.0
                && rect_w(&p.bbox) >= 0.45 * column_width_pt
                && rect_w(&p.bbox) <= 0.95 * column_width_pt
        })
        .map(|(i, _)| i)
        .collect();
    // Top-of-page first so pitch runs walk downward deterministically.
    candidates.sort_by(|&a, &b| {
        let ya = rect_center(&prims[a].bbox)[1];
        let yb = rect_center(&prims[b].bbox)[1];
        yb.partial_cmp(&ya).unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut stacks: Vec<RectPt> = Vec::new();
    let mut consumed: HashSet<usize> = HashSet::new();

    fn flush_stack(
        run: &[usize],
        prims: &[TypedPrim],
        stacks: &mut Vec<RectPt>,
        consumed: &mut HashSet<usize>,
    ) {
        if run.len() >= ANSWER_LINE_MIN_COUNT {
            let mut u = prims[run[0]].bbox;
            for &i in run {
                u = rect_union(&u, &prims[i].bbox);
                consumed.insert(i);
            }
            stacks.push(geometry::pad_rect(&u, STACK_PAD_PT));
        }
    }

    let mut run: Vec<usize> = Vec::new();
    for &i in &candidates {
        match run.last().copied() {
            Some(prev) => {
                let pitch = rect_center(&prims[prev].bbox)[1] - rect_center(&prims[i].bbox)[1];
                if (ANSWER_LINE_PITCH_MIN_PT..=ANSWER_LINE_PITCH_MAX_PT).contains(&pitch) {
                    run.push(i);
                } else {
                    flush_stack(&run, prims, &mut stacks, &mut consumed);
                    run.clear();
                    run.push(i);
                }
            }
            None => run.push(i),
        }
    }
    flush_stack(&run, prims, &mut stacks, &mut consumed);
    (stacks, consumed)
}

/// Retype uniform-spacing parallel rules as `GridMember` (they may join
/// figures but can neither seed regions nor bridge across text).
/// `skip` holds consumed answer-line rules.
pub(crate) fn retype_grid_families(prims: &mut [TypedPrim], skip: &HashSet<usize>) {
    let vertical: Vec<(f32, usize)> = prims
        .iter()
        .enumerate()
        .filter(|(i, p)| p.prim == Prim::Rule && !skip.contains(i) && rect_h(&p.bbox) > rect_w(&p.bbox))
        .map(|(i, p)| (rect_center(&p.bbox)[0], i))
        .collect();
    for family in
        uniform_spacing_families(vertical, GRID_TOL_PT, GRID_MIN_MEMBERS, GRID_SPACING_CV_MAX)
    {
        for idx in family {
            prims[idx].prim = Prim::GridMember;
        }
    }

    let horizontal: Vec<(f32, usize)> = prims
        .iter()
        .enumerate()
        .filter(|(i, p)| p.prim == Prim::Rule && !skip.contains(i) && rect_w(&p.bbox) >= rect_h(&p.bbox))
        .map(|(i, p)| (rect_center(&p.bbox)[1], i))
        .collect();
    for family in
        uniform_spacing_families(horizontal, GRID_TOL_PT, GRID_MIN_MEMBERS, GRID_SPACING_CV_MAX)
    {
        for idx in family {
            prims[idx].prim = Prim::GridMember;
        }
    }
}

// ── Regions ─────────────────────────────────────────────────────────────────

struct Region {
    bbox: RectPt,
    strongest: u8,
    has_grid: bool,
    /// Primitive indices plus (offset-tagged) captured block indices.
    labels: Vec<usize>,
    caption: Option<String>,
    ink_len_pt: f32,
}

impl Region {
    fn new(idx: usize, prim: &TypedPrim) -> Self {
        Region {
            bbox: prim.bbox,
            strongest: prim.strength(),
            has_grid: false,
            labels: vec![idx],
            caption: None,
            ink_len_pt: prim.ink_len_pt,
        }
    }

    fn absorb(&mut self, idx: usize, prim: &TypedPrim) {
        self.bbox = rect_union(&self.bbox, &prim.bbox);
        self.strongest = self.strongest.max(prim.strength());
        self.has_grid |= prim.prim == Prim::GridMember;
        self.labels.push(idx);
        self.ink_len_pt += prim.ink_len_pt;
    }
}

/// Seed-first, barrier-aware region growing (plan §Phase 5).
///
/// Phase A: seeds (strength ≥ 1) start regions or join the nearest region
/// within `JOIN_GAP_PT` across an unblocked corridor. Phase B: decorative
/// strokes join a region they lie ALONGSIDE (span coverage ≥ 25% of their
/// own long axis) across an unblocked corridor, provided the union stays
/// within the aspect/area ceilings. `excluded_decoration` holds indices
/// (consumed answer-line rules) that phase B must skip entirely.
fn grow_regions(
    prims: &[TypedPrim],
    barriers: &[RectPt],
    excluded_decoration: &HashSet<usize>,
) -> Vec<Region> {
    let mut order: Vec<usize> = (0..prims.len()).collect();
    order.sort_by(|&a, &b| {
        let (sa, sb) = (prims[a].strength(), prims[b].strength());
        sb.cmp(&sa).then_with(|| {
            rect_area(&prims[b].bbox)
                .partial_cmp(&rect_area(&prims[a].bbox))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });

    let mut regions: Vec<Region> = Vec::new();

    for &i in &order {
        if prims[i].strength() == 0 {
            continue;
        }
        let mut best: Option<(usize, f32)> = None;
        for (ri, region) in regions.iter().enumerate() {
            let g = rect_gap(&region.bbox, &prims[i].bbox);
            if g <= JOIN_GAP_PT
                && !corridor_blocked(
                    &region.bbox,
                    &prims[i].bbox,
                    barriers,
                    CORRIDOR_SAMPLE_PT,
                    CORRIDOR_PAD_PT,
                )
                && best.is_none_or(|(_, bg)| g < bg)
            {
                best = Some((ri, g));
            }
        }
        match best {
            Some((ri, _)) => regions[ri].absorb(i, &prims[i]),
            None => regions.push(Region::new(i, &prims[i])),
        }
    }

    for (i, prim) in prims.iter().enumerate() {
        if prim.strength() != 0 || excluded_decoration.contains(&i) {
            continue;
        }
        let horizontal = rect_w(&prim.bbox) >= rect_h(&prim.bbox);
        let mut best: Option<(usize, f32)> = None;
        for (ri, region) in regions.iter().enumerate() {
            let g = rect_gap(&region.bbox, &prim.bbox);
            if g > JOIN_GAP_PT {
                continue;
            }
            if corridor_blocked(
                &region.bbox,
                &prim.bbox,
                barriers,
                CORRIDOR_SAMPLE_PT,
                CORRIDOR_PAD_PT,
            ) {
                continue;
            }
            // Post-union ceilings: absorbing must not weld a page-wide rule
            // onto a mid-page figure (extension along the rule's long axis),
            // nor blow the aspect/area envelopes.
            let merged = rect_union(&region.bbox, &prim.bbox);
            let extension = if horizontal {
                rect_w(&merged) / rect_w(&region.bbox).max(1e-3)
            } else {
                rect_h(&merged) / rect_h(&region.bbox).max(1e-3)
            };
            if extension > DECORATION_EXTENSION_MAX {
                continue;
            }
            let mw = rect_w(&merged);
            let mh = rect_h(&merged);
            if mw.max(mh) / mw.min(mh).max(1e-6) > MAX_ASPECT {
                continue;
            }
            if best.is_none_or(|(_, bg)| g < bg) {
                best = Some((ri, g));
            }
        }
        if let Some((ri, _)) = best {
            regions[ri].absorb(i, prim);
        }
        // Unabsorbed decoration is dropped — it was never a figure.
    }

    regions
}

// ── Label capture, refinement, scoring ──────────────────────────────────────

/// Capture label/caption blocks and expand the bbox toward them (+pad),
/// clamping every edge at the first barrier met. Labels (axis numbers,
/// circuit tags) must be centred INSIDE the hull; captions may sit up to
/// `CAPTION_SNAP_PT` below/above it — board papers print them just under
/// the graphic, and the crop should include them.
fn capture_labels(region: &mut Region, blocks: &[TextBlock], barriers: &[RectPt]) {
    for (bi, block) in blocks.iter().enumerate() {
        let c = rect_center(&block.rect);
        let inside = c[0] >= region.bbox[0]
            && c[0] <= region.bbox[1]
            && c[1] >= region.bbox[3]
            && c[1] <= region.bbox[2];
        let is_caption = block.class == BlockClass::Caption;
        if !inside && !(is_caption && caption_in_snap_window(&c, &region.bbox)) {
            continue;
        }
        if !matches!(block.class, BlockClass::LabelCandidate | BlockClass::Caption) {
            continue;
        }
        let goal = geometry::pad_rect(&rect_union(&region.bbox, &block.rect), LABEL_PAD_PT);
        region.bbox = expand_clamped(&region.bbox, &goal, barriers, BARRIER_PAD_PT);
        region.labels.push(BLOCK_TAG_OFFSET + bi);
        if is_caption && region.caption.is_none() {
            region.caption = Some(block.text.clone());
        }
    }
}

/// Caption snap window: within `CAPTION_SNAP_PT` vertically (either side —
/// some boards print above) and `CAPTION_SNAP_SIDE_PT` horizontally beyond
/// the bbox edges.
fn caption_in_snap_window(c: &[f32; 2], bbox: &RectPt) -> bool {
    c[0] >= bbox[0] - CAPTION_SNAP_SIDE_PT
        && c[0] <= bbox[1] + CAPTION_SNAP_SIDE_PT
        && c[1] >= bbox[3] - CAPTION_SNAP_PT
        && c[1] <= bbox[2] + CAPTION_SNAP_PT
}

/// Positional capture of the printed MCQ option letter for a figure region.
///
/// Boards print per-option diagrams under a bare letter ("A" / "B" / "C" / "D",
/// or a `A B` / `C D` grid). The letter is a single-character `LabelCandidate`
/// text block sitting inside the region hull or just above its top edge with
/// real horizontal overlap. Returning the letter lets the caller bind the crop
/// to its printed option; if no letter qualifies, the region stays unlabelled
/// and the caller must not guess.
fn capture_option_label(region: &Region, blocks: &[TextBlock]) -> Option<String> {
    let bbox = region.bbox;
    let mut best: Option<(f32, String)> = None;
    for block in blocks {
        if block.class != BlockClass::LabelCandidate {
            continue;
        }
        let text = block.text.trim();
        if text.len() != 1 {
            continue;
        }
        let ch = text.chars().next().unwrap();
        if !matches!(ch, 'A'..='E') {
            continue;
        }
        let c = rect_center(&block.rect);
        let inside = c[0] >= bbox[0] && c[0] <= bbox[1] && c[1] >= bbox[3] && c[1] <= bbox[2];
        let above = c[1] >= bbox[2]
            && c[1] <= bbox[2] + OPTION_LABEL_SNAP_PT
            && x_span_coverage(&block.rect, &bbox) > 0.2;
        if !inside && !above {
            continue;
        }
        let d = (c[0] - (bbox[0] + bbox[1]) / 2.0).powi(2) + (c[1] - bbox[2]).powi(2);
        if best.as_ref().is_none_or(|(bd, _)| d < *bd) {
            best = Some((d, text.to_string()));
        }
    }
    best.map(|(_, text)| text)
}

/// Deterministic confidence score: seed strength, grid presence, label
/// richness, caption presence, and ink-density normality. Clamped to 0..1.
fn score_region(region: &Region, _page_area_pt: f32) -> f32 {
    let mut s = match region.strongest {
        3 => 0.5,
        2 => 0.35,
        _ => 0.2,
    };
    if region.has_grid {
        s += 0.15;
    }
    let label_count = region.labels.iter().filter(|&&i| i < BLOCK_TAG_OFFSET).count();
    s += 0.10 * (label_count.min(6) as f32 / 6.0);
    if region.caption.is_some() {
        s += 0.15;
    }
    let area_pt = rect_area(&region.bbox).max(1.0);
    let coverage = region.ink_len_pt / area_pt.sqrt();
    s += if coverage <= 0.0 {
        0.0
    } else if coverage < 0.02 {
        coverage / 0.02 * 0.10
    } else if coverage <= 4.0 {
        0.10
    } else {
        (0.4 / coverage).min(0.10)
    };
    s.clamp(0.0, 1.0)
}

/// Pick the nearest caption block within the snap window of the final bbox
/// — containment-scoped replacement for the legacy global-nearest heuristic.
fn contained_caption<'a>(blocks: &'a [TextBlock], bbox: &RectPt, center: [f32; 2]) -> Option<&'a str> {
    let mut best: Option<(f32, &'a str)> = None;
    for block in blocks.iter().filter(|b| b.class == BlockClass::Caption) {
        let c = rect_center(&block.rect);
        if !caption_in_snap_window(&c, bbox) {
            continue;
        }
        let d = (c[0] - center[0]) * (c[0] - center[0]) + (c[1] - center[1]) * (c[1] - center[1]);
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, block.text.as_str()));
        }
    }
    best.map(|(_, t)| t)
}

// ── Rotation normalization ──────────────────────────────────────────────────

/// Map an unrotated PDF-space rect into rendered-page space, mirroring how
/// pdfium applies `/Rotate` when drawing. Also returns the effective
/// (rendered) page dimensions.
pub(crate) fn rotate_rect_for_render(
    r: &RectPt,
    raw_w: f32,
    raw_h: f32,
    rotation: &PdfPageRenderRotation,
) -> (RectPt, f32, f32) {
    let mapped = match rotation {
        PdfPageRenderRotation::None => *r,
        // Clockwise 90°: point (x, y) → (H − y, x)
        PdfPageRenderRotation::Degrees90 => [raw_h - r[2], raw_h - r[3], r[1], r[0]],
        PdfPageRenderRotation::Degrees180 => {
            [raw_w - r[1], raw_w - r[0], raw_h - r[3], raw_h - r[2]]
        }
        // Clockwise 270° (≡ CCW 90°): point (x, y) → (y, W − x)
        PdfPageRenderRotation::Degrees270 => [r[3], r[2], raw_w - r[0], raw_w - r[1]],
    };
    let (w, h) = match rotation {
        PdfPageRenderRotation::Degrees90 | PdfPageRenderRotation::Degrees270 => (raw_h, raw_w),
        _ => (raw_w, raw_h),
    };
    (mapped, w, h)
}

// ── Pdfium collection ───────────────────────────────────────────────────────

struct Collected {
    prims: Vec<TypedPrim>,
    form_seeds: usize,
}

/// The visible page's lower-left corner in user space. Object coordinates
/// are user space while the page renders from its crop box (Edexcel papers
/// put it at (28.35, 28.35) inside a larger media box), so every collected
/// rectangle is shifted to crop-box coordinates.
fn crop_origin(page: &PdfPage) -> (f32, f32) {
    let boundaries = page.boundaries();
    match boundaries.crop().or_else(|_| boundaries.media()) {
        Ok(b) => (b.bounds.left().value, b.bounds.bottom().value),
        Err(_) => (0.0, 0.0),
    }
}

/// Walk every renderable object on the page, measure paths, and produce
/// typed primitives in rendered-point space.
fn collect_prims(page: &PdfPage, raw_w: f32, raw_h: f32, rotation: &PdfPageRenderRotation) -> Collected {
    let page_area_pt = (raw_w * raw_h).max(1.0);
    let (ox, oy) = crop_origin(page);
    let mut prims: Vec<TypedPrim> = Vec::new();
    let mut form_seeds = 0usize;

    for obj in page.objects().iter() {
        match &obj {
            PdfPageObject::Image(o) => {
                let Some(b) = o.bounds().ok() else { continue };
                push_prim(
                    &mut prims,
                    b.left().value - ox,
                    b.right().value - ox,
                    b.top().value - oy,
                    b.bottom().value - oy,
                    Prim::Bitmap,
                    0.0,
                    raw_w,
                    raw_h,
                    rotation,
                );
            }
            PdfPageObject::XObjectForm(f) => {
                // pdfium-render exposes only outer form bounds — no child
                // enumeration — so forms contribute opaque seed boxes.
                let Some(b) =
                    pdfium_render::prelude::PdfPageObjectCommon::bounds(f).ok()
                else {
                    continue;
                };
                form_seeds += 1;
                push_prim(
                    &mut prims,
                    b.left().value - ox,
                    b.right().value - ox,
                    b.top().value - oy,
                    b.bottom().value - oy,
                    Prim::Bitmap,
                    0.0,
                    raw_w,
                    raw_h,
                    rotation,
                );
            }
            PdfPageObject::Path(o) => {
                let Some(b) = o.bounds().ok() else { continue };
                let mut t = StrokeTelemetry {
                    bbox: rect_pt(b.left().value - ox, b.right().value - ox, b.top().value - oy, b.bottom().value - oy),
                    line_count: 0,
                    bezier_count: 0,
                    ink_len_pt: 0.0,
                    dir_hist: [0.0; 8],
                    closed: false,
                    stroked: o.is_stroked().unwrap_or(false),
                };
                let mut prev: Option<(f32, f32)> = None;
                for seg in o.segments().iter() {
                    match seg.segment_type() {
                        PdfPathSegmentType::MoveTo => {
                            let (x, y) = seg.point();
                            prev = Some((x.value, y.value));
                        }
                        PdfPathSegmentType::LineTo => {
                            let (px, py) = seg.point();
                            let (x, y) = (px.value, py.value);
                            t.line_count += 1;
                            if let Some((sx, sy)) = prev {
                                let dx = x - sx;
                                let dy = y - sy;
                                t.ink_len_pt += (dx * dx + dy * dy).sqrt();
                                let deg = dy.atan2(dx).to_degrees().rem_euclid(360.0);
                                let bin = ((deg / 45.0).round() as usize) % 8;
                                t.dir_hist[bin] += 1.0;
                            }
                            prev = Some((x, y));
                        }
                        PdfPathSegmentType::BezierTo => {
                            let (px, py) = seg.point();
                            let (x, y) = (px.value, py.value);
                            t.bezier_count += 1;
                            if let Some((sx, sy)) = prev {
                                t.ink_len_pt +=
                                    ((x - sx) * (x - sx) + (y - sy) * (y - sy)).sqrt();
                            }
                            prev = Some((x, y));
                        }
                        _ => {
                            if seg.is_close() {
                                t.closed = true;
                            }
                        }
                    }
                }
                let prim = classify_stroke(&t, page_area_pt);
                push_prim(
                    &mut prims,
                    t.bbox[0],
                    t.bbox[1],
                    t.bbox[2],
                    t.bbox[3],
                    prim,
                    t.ink_len_pt,
                    raw_w,
                    raw_h,
                    rotation,
                );
            }
            _ => {}
        }
    }

    Collected { prims, form_seeds }
}

#[allow(clippy::too_many_arguments)]
fn push_prim(
    out: &mut Vec<TypedPrim>,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    prim: Prim,
    ink_len_pt: f32,
    raw_w: f32,
    raw_h: f32,
    rotation: &PdfPageRenderRotation,
) {
    let raw = rect_pt(left, right, top, bottom);
    let (mapped, _, _) = rotate_rect_for_render(&raw, raw_w, raw_h, rotation);
    if rect_w(&mapped) <= 0.0 || rect_h(&mapped) <= 0.0 {
        return;
    }
    out.push(TypedPrim::new(mapped, prim, ink_len_pt));
}

/// Collect text segments as (rendered-space rect, text) pairs.
pub(crate) fn collect_text_segments(
    page: &PdfPage,
    raw_w: f32,
    raw_h: f32,
    rotation: &PdfPageRenderRotation,
) -> Vec<(RectPt, String)> {
    let mut segs: Vec<(RectPt, String)> = Vec::new();
    let (ox, oy) = crop_origin(page);
    if let Ok(text) = page.text() {
        for seg in text.segments().iter() {
            let b = seg.bounds();
            let txt = seg.text();
            if txt.trim().is_empty() {
                continue;
            }
            let raw = rect_pt(b.left().value - ox, b.right().value - ox, b.top().value - oy, b.bottom().value - oy);
            let (mapped, _, _) = rotate_rect_for_render(&raw, raw_w, raw_h, rotation);
            if rect_w(&mapped) <= 0.0 || rect_h(&mapped) <= 0.0 {
                continue;
            }
            segs.push((mapped, txt));
        }
    }
    segs
}

/// Local layout evidence for one page: the text runs and the long horizontal
/// rules (fraction bars, underlines, table rules) in rendered-point space.
/// Used by `pdf_render::pdf_layout_evidence` to expose position/font evidence
/// to the assembler without reloading the document per span.
pub(crate) fn collect_page_layout(
    page: &PdfPage,
    raw_w: f32,
    raw_h: f32,
    rotation: &PdfPageRenderRotation,
) -> (Vec<(RectPt, String)>, Vec<RectPt>) {
    let runs = collect_text_segments(page, raw_w, raw_h, rotation);
    let rules = collect_prims(page, raw_w, raw_h, rotation)
        .prims
        .into_iter()
        .filter(|p| p.prim == Prim::Rule)
        .map(|p| p.bbox)
        .collect();
    (runs, rules)
}

// ── Per-page entrypoint ─────────────────────────────────────────────────────

/// Detect figures on one page deterministically. Returns app-normalized
/// `DetectedFigure`s ready for span binding and cropping.
fn numeric_option_table_band(lines: &[(RectPt, String)]) -> Option<RectPt> {
    let mut rows = Vec::new();
    for letter in ["A", "B", "C", "D"] {
        let (label, _) = lines.iter().find(|(_, text)| text.trim() == letter)?;
        let cells: Vec<_> = lines.iter().filter(|(rect, text)| {
            rect[0] > label[1]
                && (rect_center(rect)[1] - rect_center(label)[1]).abs() < rect_h(label).max(8.0)
                && text.trim().chars().next().is_some_and(|c| c.is_ascii_digit())
                && text.split_whitespace().count() <= 2
        }).collect();
        // A column of ordinary numeric MCQ answers has one value per row.
        // Only a multi-column A-D data table is excluded from figure crops.
        if cells.len() < 2 { return None; }
        let row = cells.iter().fold(*label, |bounds, (rect, _)| rect_union(&bounds, rect));
        rows.push(row);
    }
    Some(rows[1..].iter().fold(rows[0], |bounds, row| rect_union(&bounds, row)))
}

pub(crate) fn detect_page_figures(page: &PdfPage) -> Vec<crate::pdf_render::DetectedFigure> {
    let raw_w = page.width().value;
    let raw_h = page.height().value;
    if raw_w <= 0.0 || raw_h <= 0.0 {
        return Vec::new();
    }
    let rotation = page.rotation().unwrap_or(PdfPageRenderRotation::None);
    let (_, eff_w, eff_h) =
        rotate_rect_for_render(&rect_pt(0.0, raw_w, raw_h, 0.0), raw_w, raw_h, &rotation);
    let page_area_pt = eff_w * eff_h;

    // 1. Census.
    let collected = collect_prims(page, raw_w, raw_h, &rotation);
    let mut prims = collected.prims;

    // 3. Text blocks (needed before answer-line detection: the column width
    //    comes from body prose).
    let segs = collect_text_segments(page, raw_w, raw_h, &rotation);
    let lines = assemble_lines(segs);
    let option_table = numeric_option_table_band(&lines);
    let blocks: Vec<TextBlock> = lines_to_blocks(&lines)
        .iter()
        .map(|(r, t)| classify_block(r, t))
        .collect();

    // 2a. Answer-line stacks first (they are themselves uniform families and
    //     must never be retyped as graph gridlines).
    let body_width = blocks
        .iter()
        .filter(|b| b.class == BlockClass::Body)
        .map(|b| rect_w(&b.rect))
        .fold(None::<f32>, |acc, w| Some(acc.map_or(w, |m| m.max(w))));
    let column_width_pt = body_width.unwrap_or_else(|| {
        let mut widths: Vec<f32> = prims
            .iter()
            .filter(|p| p.prim == Prim::Rule && rect_w(&p.bbox) > rect_h(&p.bbox))
            .map(|p| rect_w(&p.bbox))
            .collect();
        if widths.is_empty() {
            0.7 * eff_w
        } else {
            widths.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            widths[widths.len() / 2]
        }
    });
    let (stacks, stacked_rules) = detect_answer_line_stacks(&prims, column_width_pt);

    // 2b. Grid families (skipping consumed answer-line rules).
    retype_grid_families(&mut prims, &stacked_rules);

    // Barriers: header/footer bands + prose + headings + answer-line stacks.
    let mut barriers: Vec<RectPt> = vec![
        rect_pt(-1.0, eff_w + 1.0, eff_h, eff_h * (1.0 - HEADER_FRAC)),
        rect_pt(-1.0, eff_w + 1.0, eff_h * FOOTER_FRAC, -1.0),
    ];
    for block in &blocks {
        if matches!(block.class, BlockClass::Body | BlockClass::Heading) {
            barriers.push(geometry::pad_rect(&block.rect, BARRIER_PAD_PT));
        }
    }
    barriers.extend(stacks);

    // 4. Grow regions, then decoration absorption (consumed stack rules skip).
    let mut regions = grow_regions(&prims, &barriers, &stacked_rules);

    // 5. Labels.
    if let Some(table) = option_table {
        regions.retain(|region| {
            let center = rect_center(&region.bbox);
            !(center[0] >= table[0] - 12.0 && center[0] <= table[1] + 12.0
                && center[1] <= table[2] + 12.0 && center[1] >= table[3] - 12.0)
        });
    }
    for region in regions.iter_mut() {
        capture_labels(region, &blocks, &barriers);
    }

    if collected.form_seeds > 0 {
        eprintln!(
            "[STROKE_CENSUS] {} XObjectForm seed(s): forms contribute outer bounds only",
            collected.form_seeds
        );
    }

    // 6. Gates, scoring, emission. Density reference excludes blocks captured
    //    by ANY region so a figure's own axis numbers never trip the ceiling.
    let captured_blocks: HashSet<usize> = regions
        .iter()
        .flat_map(|r| {
            r.labels
                .iter()
                .filter(|&&t| t >= BLOCK_TAG_OFFSET)
                .map(|&t| t - BLOCK_TAG_OFFSET)
        })
        .collect();
    let density_norm: Vec<[f32; 4]> = blocks
        .iter()
        .enumerate()
        .filter(|(i, _)| !captured_blocks.contains(i))
        .map(|(_, b)| {
            geometry::normalize_pdf_box(b.rect[0], b.rect[1], b.rect[2], b.rect[3], eff_w, eff_h)
        })
        .collect();

    let mut figures = Vec::new();
    for region in regions.iter_mut() {
        let w = rect_w(&region.bbox);
        let h = rect_h(&region.bbox);
        // A labelled composite (apparatus plus graph) can exceed half a page.
        // Require a strong seed and caption; retain all prose/margin gates.
        let max_area = if region.strongest >= 2 && region.caption.is_some() {
            0.65
        } else {
            MAX_FIGURE_AREA_FRAC
        };
        if w <= 0.0 || h <= 0.0 || (w * h) / page_area_pt > max_area {
            continue;
        }
        if w.max(h) / w.min(h).max(1e-6) > MAX_ASPECT {
            continue;
        }
        let norm_bbox = geometry::normalize_pdf_box(
            region.bbox[0], region.bbox[1], region.bbox[2], region.bbox[3], eff_w, eff_h,
        );
        if !crate::geometry::is_probable_figure_box(
            &norm_bbox, &density_norm, 0.003, 0.015, MAX_ASPECT, 0.4,
        ) {
            continue;
        }
        // Decoration chains die here: no curve/bitmap seed, nothing captured,
        // negligible ink.
        if region.strongest < 2
            && region.labels.iter().filter(|&&i| i < BLOCK_TAG_OFFSET).count() < 2
            && region.caption.is_none()
            && region.ink_len_pt < MIN_SEED_INK_PT
        {
            continue;
        }

        let center = rect_center(&region.bbox);
        let caption = region
            .caption
            .clone()
            .or_else(|| contained_caption(&blocks, &region.bbox, center).map(str::to_string));
        let kind = caption
            .as_deref()
            .and_then(crate::geometry::caption_kind_from_text);
        let confidence = score_region(region, page_area_pt);
        let option_label = capture_option_label(region, &blocks);

        figures.push(crate::pdf_render::DetectedFigure {
            bbox: norm_bbox,
            caption,
            kind,
            seg_confidence: confidence,
            option_label,
        });
    }
    figures
}

// ── Unit tests (pure — synthetic strokes and rects, no PDFs) ────────────────

#[cfg(test)]
mod tests {
    #[test]
    fn physics24_large_captioned_composite_is_detected() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../physics '24.pdf");
        if !path.exists() { return; }
        let pdfium = crate::pdf_render::get_pdfium().unwrap();
        let document = pdfium.load_pdf_from_file(&path, None).unwrap();
        let page = document.pages().get(15).unwrap();
        let figures = detect_page_figures(&page);
        assert!(figures.iter().any(|f| f.caption.as_deref() == Some("Figure 9")
            && f.bbox[1] < 0.18 && f.bbox[1] + f.bbox[3] > 0.77),
            "apparatus and graph must remain together: {figures:?}");
    }

    #[test]
    fn physics24_diagram_mcq_regions_capture_option_letters() {
        // Physics '24 Q26 prints four charge diagrams under a `A B` / `C D`
        // grid. The deterministic detector must capture each printed option
        // letter from positional text evidence, so the assembler can bind each
        // crop to its own option rather than guessing an order.
        let _guard = crate::pdf_render::pdfium_test_lock();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../physics '24.pdf");
        if !path.exists() { return; }
        let pdfium = crate::pdf_render::get_pdfium().unwrap();
        let document = pdfium.load_pdf_from_file(&path, None).unwrap();
        let page = document.pages().get(31).unwrap();
        let figures = detect_page_figures(&page);
        let labels: Vec<String> = figures
            .iter()
            .filter_map(|f| f.option_label.clone())
            .collect();
        eprintln!("[OPTION_LABELS] {labels:?}");
        // The four charge diagrams sit in a 2x2 grid; the printed option
        // letters are captured from positional text evidence. The top-left
        // glyph is not present in the text layer for this paper, so three
        // regions carry a letter and exactly one stays anonymous — the
        // assembler resolves the forced missing letter downstream.
        let distinct: std::collections::BTreeSet<&String> = labels.iter().collect();
        assert!(
            labels.len() >= 3 && distinct.len() == labels.len(),
            "expected at least three distinct printed option letters, got {labels:?} from {figures:?}"
        );
        assert!(labels.iter().all(|l| matches!(l.as_str(), "A" | "B" | "C" | "D" | "E")));
    }

    use super::*;

    #[test]
    fn numeric_mcq_table_requires_two_columns() {
        let mut lines = Vec::new();
        for (i, label) in ["A", "B", "C", "D"].iter().enumerate() {
            let y = 200.0 - i as f32 * 30.0;
            lines.push((rect_pt(10.0, 20.0, y, y - 10.0), label.to_string()));
            lines.push((rect_pt(40.0, 60.0, y, y - 10.0), "50".into()));
        }
        assert!(numeric_option_table_band(&lines).is_none());
        for i in 0..4 {
            let y = 200.0 - i as f32 * 30.0;
            lines.push((rect_pt(90.0, 120.0, y, y - 10.0), "0.30π".into()));
        }
        assert!(numeric_option_table_band(&lines).is_some());
    }

    fn rule(l: f32, r: f32, t: f32, b: f32) -> TypedPrim {
        TypedPrim::new(rect_pt(l, r, t, b), Prim::Rule, (r - l).max(t - b))
    }
    fn curve(l: f32, r: f32, t: f32, b: f32) -> TypedPrim {
        TypedPrim::new(rect_pt(l, r, t, b), Prim::Curve, 60.0)
    }
    fn glyph(l: f32, r: f32, t: f32, b: f32) -> TypedPrim {
        TypedPrim::new(rect_pt(l, r, t, b), Prim::Glyph, 6.0)
    }
    fn bitmap(l: f32, r: f32, t: f32, b: f32) -> TypedPrim {
        TypedPrim::new(rect_pt(l, r, t, b), Prim::Bitmap, 0.0)
    }

    #[test]
    fn straight_axis_line_classifies_rule() {
        // A 200×0.8pt horizontal line drawn as move+line.
        let t = StrokeTelemetry {
            bbox: rect_pt(50.0, 250.0, 300.0, 299.2),
            line_count: 1,
            bezier_count: 0,
            ink_len_pt: 200.0,
            // two opposite-direction horizontal bins dominate
            dir_hist: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            closed: false,
            stroked: true,
        };
        assert_eq!(classify_stroke(&t, 500_000.0), Prim::Rule);
    }

    #[test]
    fn diagonal_ray_is_not_rule_but_seeds_weakly() {
        let t = StrokeTelemetry {
            bbox: rect_pt(100.0, 170.0, 170.0, 100.0),
            line_count: 1,
            bezier_count: 0,
            ink_len_pt: 99.0,
            dir_hist: [0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            closed: false,
            stroked: true,
        };
        assert_eq!(classify_stroke(&t, 500_000.0), Prim::Glyph);
    }

    #[test]
    fn bezier_curve_classifies_curve() {
        let t = StrokeTelemetry {
            bbox: rect_pt(100.0, 200.0, 200.0, 120.0),
            line_count: 0,
            bezier_count: 3,
            ink_len_pt: 140.0,
            dir_hist: [0.0; 8],
            closed: false,
            stroked: true,
        };
        assert_eq!(classify_stroke(&t, 500_000.0), Prim::Curve);
    }

    #[test]
    fn tiny_speck_classifies_glyph() {
        let t = StrokeTelemetry {
            bbox: rect_pt(150.0, 152.0, 152.0, 150.0),
            line_count: 1,
            bezier_count: 0,
            ink_len_pt: 2.8,
            dir_hist: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            closed: false,
            stroked: true,
        };
        assert_eq!(classify_stroke(&t, 500_000.0), Prim::Glyph);
    }

    #[test]
    fn answer_line_stack_detected_and_consumed() {
        // Three stacked 400pt-wide rules at ~24pt pitch + one stray wide rule.
        let prims = vec![
            rule(60.0, 460.0, 700.0, 699.0), // y-centre 699.5
            rule(60.0, 460.0, 676.0, 675.0), // pitch 24
            rule(60.0, 460.0, 652.0, 651.0), // pitch 24
            rule(60.0, 460.0, 300.0, 299.0), // far away
        ];
        let (stacks, consumed) = detect_answer_line_stacks(&prims, 430.0);
        assert_eq!(stacks.len(), 1, "one three-rule stack");
        assert_eq!(consumed.len(), 3, "stack rules are consumed");
        assert!(!consumed.contains(&3), "the distant rule is untouched");
    }

    #[test]
    fn grid_family_retyped_skipping_answer_lines() {
        // Four vertical rules at exact 30pt x-pitch → gridlines.
        let mut prims: Vec<TypedPrim> = (0..4)
            .map(|i| rule(100.0 + i as f32 * 30.0, 100.4 + i as f32 * 30.0, 500.0, 350.0))
            .collect();
        retype_grid_families(&mut prims, &HashSet::new());
        assert!(prims.iter().all(|p| p.prim == Prim::GridMember));

        // The same strokes are NOT retyped when consumed as answer lines.
        let mut prims2: Vec<TypedPrim> = (0..4)
            .map(|i| rule(100.0 + i as f32 * 30.0, 100.4 + i as f32 * 30.0, 500.0, 350.0))
            .collect();
        let skip: HashSet<usize> = (0..4).collect();
        retype_grid_families(&mut prims2, &skip);
        assert!(prims2.iter().all(|p| p.prim == Prim::Rule));
    }

    #[test]
    fn arrowhead_joins_nearby_curve_across_clear_corridor() {
        let shaft = curve(100.0, 200.0, 300.0, 280.0);   // strength 2 seed
        let head = glyph(202.0, 210.0, 305.0, 295.0);    // 2pt gap
        let regions = grow_regions(&[shaft, head], &[], &HashSet::new());
        assert_eq!(regions.len(), 1, "arrowhead welds to its shaft");
        assert_eq!(regions[0].strongest, 2);
    }

    #[test]
    fn text_barrier_prevents_merge_across_prose() {
        let left_fig = curve(40.0, 140.0, 300.0, 220.0);
        let right_fig = curve(260.0, 360.0, 300.0, 220.0);
        let prose = geometry::pad_rect(&rect_pt(150.0, 250.0, 290.0, 230.0), BARRIER_PAD_PT);
        let regions = grow_regions(&[left_fig, right_fig], &[prose], &HashSet::new());
        assert_eq!(regions.len(), 2, "body-text corridor keeps graphs separate");
    }

    #[test]
    fn page_wide_rule_never_welds_sideways() {
        // Mid-page figure + full-width divider rule 4pt below it. The divider
        // is rejected (3× extension), leaving the figure's own tight box.
        let divider = rule(0.0, 495.0, 216.0, 215.5);
        let regions =
            grow_regions(&[curve(80.0, 240.0, 300.0, 220.0), divider], &[], &HashSet::new());
        assert_eq!(regions.len(), 1, "figure stands alone");
        assert!(
            rect_w(&regions[0].bbox) < 250.0,
            "divider must not stretch the bbox (w = {})",
            rect_w(&regions[0].bbox)
        );

        // A genuinely accompanying axis (~1.1× extension) DOES join.
        let axis = rule(70.0, 250.0, 214.0, 213.5);
        let regions =
            grow_regions(&[curve(80.0, 240.0, 300.0, 220.0), axis], &[], &HashSet::new());
        assert_eq!(regions.len(), 1, "the true axis joins its graph");
        assert!(
            rect_w(&regions[0].bbox) >= 170.0,
            "axis widens the crop to include itself"
        );
    }

    #[test]
    fn decoration_never_forms_a_region_alone() {
        // Two parallel rules with no seed anywhere: rules are strength 0, so
        // growth produces NO region at all — decoration-only clusters die
        // before regions ever exist.
        let r1 = rule(100.0, 300.0, 400.0, 399.0);
        let r2 = rule(100.0, 300.0, 380.0, 379.0);
        let regions = grow_regions(&[r1, r2], &[], &HashSet::new());
        assert_eq!(regions.len(), 0);
    }

    #[test]
    fn label_capture_expands_and_clamps_at_barrier() {
        let mut region = Region::new(0, &curve(120.0, 240.0, 320.0, 260.0));
        // "10 N" label whose CENTER sits inside the region hull, poking out
        // on the left edge.
        let label = TextBlock {
            rect: rect_pt(110.0, 130.0, 310.0, 300.0),
            text: "10 N".into(),
            class: BlockClass::LabelCandidate,
        };
        // Barrier band just above the region's top edge (320): the top edge
        // must refuse to expand into it while the left edge expands freely.
        let barriers = vec![geometry::pad_rect(&rect_pt(0.0, 495.0, 322.0, 320.0), BARRIER_PAD_PT)];
        capture_labels(&mut region, &[label], &barriers);
        assert!(
            region.bbox[0] <= 107.0 + 1e-4,
            "left edge expanded toward the label"
        );
        assert!(
            (region.bbox[2] - 320.0).abs() < 1e-4,
            "top edge clamped before the barrier band"
        );
    }

    #[test]
    fn caption_below_figure_snaps_and_enters_crop() {
        let mut region = Region::new(0, &curve(120.0, 240.0, 300.0, 220.0));
        // "Figure 4" printed 14pt BELOW the figure's bottom edge — the
        // classic board layout. It must bind, and pull the bbox down over it.
        let caption = TextBlock {
            rect: rect_pt(140.0, 200.0, 206.0, 194.0),
            text: "Figure 4".into(),
            class: BlockClass::Caption,
        };
        // A distant caption belonging to another element must NOT bind.
        let far_caption = TextBlock {
            rect: rect_pt(140.0, 200.0, 100.0, 88.0),
            text: "Figure 9".into(),
            class: BlockClass::Caption,
        };
        capture_labels(&mut region, &[caption, far_caption], &[]);
        assert_eq!(region.caption.as_deref(), Some("Figure 4"));
        assert!(region.bbox[3] <= 191.0, "bbox extended down over the caption");
        assert!(region.bbox[3] > 120.0, "far caption never dragged in");
    }

    #[test]
    fn scoring_rewards_seeds_grids_captions() {
        let mut bare = Region::new(0, &glyph(0.0, 5.0, 5.0, 0.0));
        let mut rich = Region::new(0, &bitmap(0.0, 100.0, 100.0, 0.0));
        rich.has_grid = true;
        rich.labels.push(BLOCK_TAG_OFFSET); // one captured label
        rich.caption = Some("Figure 1".into());
        let s_bare = score_region(&bare, 500_000.0);
        let s_rich = score_region(&rich, 500_000.0);
        assert!(s_rich > s_bare + 0.35, "rich {} vs bare {}", s_rich, s_bare);
        bare.strongest = 9; // silence unused-mut lint while asserting bounds
        assert!(score_region(&bare, 500_000.0) <= 1.0);
    }

    #[test]
    fn rotation_maps_corners_consistently() {
        let whole = rect_pt(0.0, 595.0, 842.0, 0.0);
        let (r90, w90, h90) =
            rotate_rect_for_render(&whole, 595.0, 842.0, &PdfPageRenderRotation::Degrees90);
        assert_eq!((w90, h90), (842.0, 595.0));
        assert!((r90[0] - 0.0).abs() < 1e-4 && (r90[1] - 842.0).abs() < 1e-4);
        assert!((r90[2] - 595.0).abs() < 1e-4 && (r90[3] - 0.0).abs() < 1e-4);

        // A point near unrotated top-left lands near rotated top-right.
        let spot = rect_pt(0.0, 10.0, 20.0, 0.0);
        let (rs, _, _) =
            rotate_rect_for_render(&spot, 595.0, 842.0, &PdfPageRenderRotation::Degrees270);
        assert!(rs[0] < 15.0, "270° keeps top content on the left edge");
    }

    #[test]
    fn block_classification_prefers_positive_evidence() {
        let body = classify_block(&rect_pt(0.0, 300.0, 100.0, 88.0),
            "A student measures the current in the circuit and records the reading");
        assert_eq!(body.class, BlockClass::Body);

        let caption = classify_block(&rect_pt(0.0, 80.0, 100.0, 92.0), "Figure 3");
        assert_eq!(caption.class, BlockClass::Caption);
        let reference = classify_block(&rect_pt(0.0, 300.0, 100.0, 88.0),
            "The graph in Figure 9 shows how the electric potential varies with distance.");
        assert_eq!(reference.class, BlockClass::Body, "a reference is prose, not a crop label");

        let footer = classify_block(&rect_pt(0.0, 160.0, 30.0, 22.0),
            "(Total for Question 3 is 11 marks)");
        assert_eq!(footer.class, BlockClass::Body, "marks footer is structural");

        let label = classify_block(&rect_pt(0.0, 24.0, 100.0, 92.0), "10 N");
        assert_eq!(label.class, BlockClass::LabelCandidate);
    }

    #[test]
    fn lines_merge_into_paragraph_blocks() {
        let l1 = (
            rect_pt(50.0, 450.0, 300.0, 288.0),
            "The graph shows how the temperature".to_string(),
        );
        let l2 = (
            rect_pt(50.0, 420.0, 286.0, 274.0),
            "changes as the substance cools".to_string(),
        );
        let blocks = lines_to_blocks(&[l1, l2]);
        assert_eq!(blocks.len(), 1, "two close lines merge into one paragraph");
        assert_eq!(word_count(&blocks[0].1), 11);
    }

    // ── Golden-fixture gate ────────────────────────────────────────────────

    /// IoU of two normalized [x, y, w, h] boxes.
    fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
        let ix0 = a[0].max(b[0]);
        let iy0 = a[1].max(b[1]);
        let ix1 = (a[0] + a[2]).min(b[0] + b[2]);
        let iy1 = (a[1] + a[3]).min(b[1] + b[3]);
        let iw = (ix1 - ix0).max(0.0);
        let ih = (iy1 - iy0).max(0.0);
        let inter = iw * ih;
        let union = a[2] * a[3] + b[2] * b[3] - inter;
        if union <= 0.0 {
            0.0
        } else {
            inter / union
        }
    }

    const GOLDEN_IOU_MIN: f32 = 0.85;

    /// Curated-golden gate: every `fixtures/figure_golden/*.json` pins the
    /// exact figure count per page and requires each detected box to match
    /// its golden counterpart at IoU ≥ 0.85. Skips cleanly when the fixture
    /// PDFs or pdfium are unavailable (same pattern as the smoke test).
    #[test]
    fn golden_fixture_boxes_match_curated_ground_truth() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let golden_dir = manifest.join("fixtures").join("figure_golden");
        let Ok(entries) = std::fs::read_dir(&golden_dir) else {
            eprintln!("[GOLDEN] no fixtures/figure_golden directory — nothing to verify yet");
            return;
        };
        let mut checked_any = false;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            // Pre-curation dumps are authoring aids, never ground truth.
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("candidates_"))
            {
                continue;
            }
            let body = match std::fs::read_to_string(&path) {
                Ok(b) => b,
                Err(e) => panic!("golden {} unreadable: {e}", path.display()),
            };
            let doc: serde_json::Value =
                serde_json::from_str(&body).unwrap_or_else(|e| panic!("golden {} invalid: {e}", path.display()));
            let source = doc["source_pdf"].as_str().unwrap_or_default();
            if source.is_empty() {
                panic!("golden {} missing source_pdf", path.display());
            }
            let pdf_path = manifest.join("..").join(source);
            if !pdf_path.exists() {
                eprintln!("[GOLDEN] fixture missing: {} — skipping", pdf_path.display());
                continue;
            }
            let per_page = match crate::pdf_render::detect_pdf_figures(&pdf_path) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[GOLDEN] pdfium unavailable, skipping: {}", e);
                    return;
                }
            };
            let pages = doc["pages"].as_object().unwrap_or_else(|| {
                panic!("golden {} missing pages object", path.display())
            });
            for (page_key, goldens) in pages {
                let page_idx: usize = page_key.parse().unwrap_or_else(|e| {
                    panic!("golden {} has non-numeric page key {page_key:?}: {e}", path.display())
                });
                let goldens = goldens.as_array().unwrap_or_else(|| {
                    panic!("golden {}.pages[{page_key}] must be an array", path.display())
                });
                let detected: &Vec<crate::pdf_render::DetectedFigure> = per_page
                    .get(page_idx)
                    .unwrap_or_else(|| panic!("no page {page_idx} detected for {source}"));
                assert_eq!(
                    detected.len(),
                    goldens.len(),
                    "{source} page {page_idx}: figure count drifted from curated golden"
                );
                // Greedy best-IoU matching.
                let mut matched_golden = vec![false; goldens.len()];
                for det in detected {
                    let mut best = (0.0f32, None::<usize>);
                    for (gi, g) in goldens.iter().enumerate() {
                        if matched_golden[gi] {
                            continue;
                        }
                        let gbox: [f32; 4] = serde_json::from_value(g["bbox"].clone())
                            .unwrap_or_else(|e| panic!("golden bbox malformed: {e}"));
                        let v = iou(&det.bbox, &gbox);
                        if v > best.0 {
                            best = (v, Some(gi));
                        }
                    }
                    assert!(
                        best.0 >= GOLDEN_IOU_MIN,
                        "{source} page {page_idx}: detected {:?} matches no golden box at IoU ≥ {} (best {:.2})",
                        det.bbox, GOLDEN_IOU_MIN, best.0
                    );
                    matched_golden[best.1.unwrap()] = true;
                    assert!(
                        (0.0..=1.0).contains(&det.seg_confidence),
                        "seg_confidence out of range"
                    );
                }
            }
            checked_any = true;
        }
        let _ = checked_any; // zero goldens committed yet → silent no-op gate
    }

    /// Caption-binding diagnostics: for each page, print every
    /// Caption-class block (pt-space rect) next to the final figure bboxes,
    /// so a missed snap is immediately attributable — too far, wrong class,
    /// or the host region was rejected at the gates.
    #[test]
    #[ignore = "caption-binding diagnostics"]
    fn diagnose_caption_binding() {
        use pdfium_render::prelude::*;
        let _guard = crate::pdf_render::pdfium_test_lock();
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let path = manifest.join("..").join("physics '24.pdf");
        if !path.exists() {
            eprintln!("[CAPDIAG] fixture missing");
            return;
        }
        let Ok(pdfium) = crate::pdf_render::get_pdfium() else {
            eprintln!("[CAPDIAG] pdfium unavailable");
            return;
        };
        let Ok(document) = pdfium.load_pdf_from_file(&path, None) else {
            eprintln!("[CAPDIAG] load failed");
            return;
        };
        for (pi, page) in document.pages().iter().enumerate() {
            let (raw_w, raw_h) = (page.width().value, page.height().value);
            if raw_w <= 0.0 || raw_h <= 0.0 {
                continue;
            }
            let rotation = page.rotation().unwrap_or(PdfPageRenderRotation::None);
            let segs = collect_text_segments(&page, raw_w, raw_h, &rotation);
            eprintln!(
                "[CAPDIAG] page {pi}: {} collected segments, total_chars={}",
                segs.len(),
                segs.iter().map(|(_, t)| t.len()).sum::<usize>()
            );
            for (_, t) in segs.iter().take(3) {
                eprintln!("[CAPDIAG]   first-segs: {:?}", t.chars().take(40).collect::<String>());
            }
            let lines = assemble_lines(segs);
            // Stage probe 2: assembled lines mentioning Figure.
            for (r, t) in &lines {
                if t.contains("Figure") || t.contains("igure ") {
                    eprintln!(
                        "[CAPDIAG]   line {:?} rect=({:.0},{:.0},{:.0},{:.0})",
                        t.trim(),
                        r[0], r[2], r[1], r[3]
                    );
                }
            }
            let blocks: Vec<TextBlock> =
                lines_to_blocks(&lines).iter().map(|(r, t)| classify_block(r, t)).collect();
            // Stage probe 3: blocks whose text mentions Figure, with class.
            for b in &blocks {
                if b.text.contains("Figure") || b.text.contains("igure ") {
                    eprintln!(
                        "[CAPDIAG]   block(class={:?}) {:?}",
                        b.class,
                        b.text.trim().chars().take(60).collect::<String>()
                    );
                }
            }
            let captions: Vec<&TextBlock> =
                blocks.iter().filter(|b| b.class == BlockClass::Caption).collect();
            let figures = detect_page_figures(&page);
            // Decoder probe: does pdfium's own full-page text contain the
            // ASCII "Figure", or is the caption encoded via ligature
            // codepoints (ﬁ U+FB01) that defeat ASCII regexes?
            if let Ok(t) = page.text() {
                let all = t.all();
                let has_ascii = all.contains("Figure");
                let has_lig = all.contains('\u{FB01}');
                eprintln!(
                    "[CAPDIAG] page {pi} text probe: ascii_figure={} fi_ligature={}",
                    has_ascii, has_lig
                );
            }
            if captions.is_empty() && figures.is_empty() {
                continue;
            }
            eprintln!("[CAPDIAG] page {pi}: {} caption blocks, {} figures", captions.len(), figures.len());
            for c in &captions {
                let ctr = rect_center(&c.rect);
                let nearest = figures
                    .iter()
                    .map(|f| {
                        // normalized bbox → pt-space distance from center to bbox edges
                        let bl = f.bbox[3] * raw_h;
                        let br = f.bbox[2] * raw_h;
                        let bx0 = f.bbox[0] * raw_w;
                        let bx1 = f.bbox[1] * raw_w;
                        let dy = if ctr[1] < bl { bl - ctr[1] } else if ctr[1] > br { ctr[1] - br } else { 0.0 };
                        let dx = if ctr[0] < bx0 { bx0 - ctr[0] } else if ctr[0] > bx1 { ctr[0] - bx1 } else { 0.0 };
                        (dy.hypot(dx), f.caption.clone())
                    })
                    .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                let (dist, bound) = match &nearest {
                    Some((d, cap)) => (*d, cap.clone()),
                    None => (-1.0, None),
                };
                eprintln!(
                    "[CAPDIAG]   cap {:?} rect=({:.0},{:.0},{:.0},{:.0}) nearest_fig_dist={:.0}pt bound_into={:?}",
                    c.text.trim(),
                    c.rect[0], c.rect[2], c.rect[1], c.rect[3],
                    dist,
                    bound
                );
            }
            for f in &figures {
                eprintln!(
                    "[CAPDIAG]   fig bbox_norm={:?} cap={:?}",
                    f.bbox.iter().map(|v| (v * 1000.0).round() / 1000.0).collect::<Vec<_>>(),
                    f.caption
                );
            }
        }
    }

    /// Candidate generation for golden curation (run explicitly):
    ///   cargo test --lib generate_golden_candidates -- --ignored --nocapture
    ///
    /// Writes UNCURATED candidate JSONs into fixtures/figure_golden/. A human
    /// reviews them against rendered pages, corrects, renames to
    /// `<paper>.json` and commits. Never runs in CI.
    #[test]
    #[ignore = "manual golden-curation helper"]
    fn generate_golden_candidates() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let out_dir = manifest.join("fixtures").join("figure_golden");
        std::fs::create_dir_all(&out_dir).expect("create fixtures dir");
        for name in ["physics '21.pdf", "physics '24.pdf"] {
            let pdf_path = manifest.join("..").join(name);
            if !pdf_path.exists() {
                eprintln!("[CANDIDATE] fixture missing: {}", pdf_path.display());
                continue;
            }
            let stem: String = name
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '_' })
                .collect();
            let dump = out_dir.join(format!("candidates_{stem}.json"));
            std::env::set_var(
                "MERGEMARK_FIGURE_DEBUG_JSON",
                dump.to_string_lossy().to_string(),
            );
            match crate::pdf_render::detect_pdf_figures(&pdf_path) {
                Ok(per_page) => eprintln!(
                    "[CANDIDATE] {}: {} figures total → {}",
                    name,
                    per_page.iter().map(Vec::len).sum::<usize>(),
                    dump.display()
                ),
                Err(e) => {
                    eprintln!("[CANDIDATE] pdfium unavailable, skipping: {}", e);
                    return;
                }
            }
            std::env::remove_var("MERGEMARK_FIGURE_DEBUG_JSON");
        }
    }
}
