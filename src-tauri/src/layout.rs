//! Geometry-first page text reconstruction for born-digital exam papers.
//!
//! The PDF text layer (`FPDFText_GetText`) is emitted in CONTENT-STREAM order,
//! which for equation editors (MathType, Word equations, TeX) is not reading
//! order: `y = (4x^3 + x^7)/x^4` arrives as `y =`, `x x +`, `x`, `3 7`, `4`, `4`.
//! No amount of downstream regex repair can recover that faithfully, so this
//! module rebuilds each page from the actual glyph evidence instead:
//!
//! * every source character with its tight box, baseline origin, effective
//!   font size and font name;
//! * thin horizontal paths (fraction bars, radical overbars, rules);
//! * page furniture identified by position/font (rotated margin text,
//!   barcodes, running headers/footers, "do not write" margins).
//!
//! Glyphs are grouped into visual lines by baseline; stacked structures
//! (fractions, scripts, radicals, big delimiters, limits) are recognised from
//! their geometry and emitted as LaTeX. Every non-furniture glyph is consumed
//! exactly once; a glyph whose identity is unknown is emitted as U+FFFD so the
//! quality gates can refuse a strict accept instead of silently guessing.
//!
//! The module is self-contained (pdfium-render + std) so it can be unit
//! tested with synthetic glyphs.

use pdfium_render::prelude::*;

/// Marker emitted for a glyph whose identity cannot be established from the
/// source (unmapped code, control character, unknown private-use glyph).
pub const UNKNOWN_GLYPH: char = '\u{FFFD}';

#[derive(Debug, Clone)]
pub struct Glyph {
    /// pdfium char index on the page text (source accounting anchor).
    pub idx: usize,
    /// Resolved identity. `UNKNOWN_GLYPH` when it cannot be established.
    pub ch: char,
    /// Raw unicode value reported by the text layer.
    pub raw: u32,
    /// Base font name with any subset tag (`ABCDEF+`) removed.
    pub font: String,
    pub italic: bool,
    pub bold: bool,
    pub mono: bool,
    /// Effective (scaled) font size in points.
    pub size: f32,
    /// Tight glyph box in page points, y measured DOWN from the top edge.
    pub x0: f32,
    pub x1: f32,
    pub y0: f32,
    pub y1: f32,
    /// Baseline origin, y measured down from the top edge.
    pub ox: f32,
    pub oy: f32,
    /// Loose (advance) box x-range: horizontal adjacency is measured on the
    /// advance cell, not the ink, so a narrow "1" does not open a word gap.
    pub lx0: f32,
    pub lx1: f32,
    /// Rotation of the glyph in degrees (0 for horizontal text).
    pub angle: f32,
}

impl Glyph {
    pub fn cx(&self) -> f32 {
        (self.x0 + self.x1) * 0.5
    }
    pub fn cy(&self) -> f32 {
        (self.y0 + self.y1) * 0.5
    }
    pub fn w(&self) -> f32 {
        self.x1 - self.x0
    }
    pub fn h(&self) -> f32 {
        self.y1 - self.y0
    }
}

/// A thin horizontal rule (fraction bar, overbar, underline, table rule).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rule {
    pub x0: f32,
    pub x1: f32,
    pub y0: f32,
    pub y1: f32,
}

impl Rule {
    pub fn w(&self) -> f32 {
        self.x1 - self.x0
    }
    pub fn cy(&self) -> f32 {
        (self.y0 + self.y1) * 0.5
    }
}

/// A vector path (page space, y down) kept for shape recognition (radicals
/// drawn as paths, vertical table rules, big brackets).
#[derive(Debug, Clone, PartialEq)]
pub struct VPath {
    pub x0: f32,
    pub x1: f32,
    pub y0: f32,
    pub y1: f32,
    /// Points in drawing order; `true` marks a move-to (sub-path start).
    pub pts: Vec<(bool, [f32; 2])>,
    pub filled: bool,
    pub stroked: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PageGlyphs {
    pub page: usize,
    pub width: f32,
    pub height: f32,
    pub glyphs: Vec<Glyph>,
    pub rules: Vec<Rule>,
    pub paths: Vec<VPath>,
    /// Raster image boxes `[x0, y0, x1, y1]`.
    pub images: Vec<[f32; 4]>,
    /// Images set inside a line of text (a formula printed as a picture),
    /// each also carried in `glyphs` as an [`INLINE_IMAGE`] at its place.
    pub inline_images: Vec<[f32; 4]>,
    /// Real (non-generated) space glyphs as `[baseline, x0, x1]`, sorted by
    /// baseline: word-break evidence independent of glyph spacing.
    pub spaces: Vec<[f32; 3]>,
    /// Word breaks the text layer generated between two glyphs of a line
    /// (`[baseline, x0, x1]`), for documents that position words without
    /// space characters.
    pub generated_spaces: Vec<[f32; 3]>,
    /// Typed thin spaces (`[baseline, x0, x1]`): a kern in "f′", a word
    /// space between two letters ("r cm").
    pub thin_spaces: Vec<[f32; 3]>,
    /// Unidentified glyphs that draw no ink (invisible placeholders), removed.
    pub invisible_dropped: usize,
    /// Overprinted duplicate glyphs removed (same char, same box).
    pub duplicates_dropped: usize,
}

// ── Extraction ──────────────────────────────────────────────────────────────

fn strip_subset(name: &str) -> String {
    let b = name.as_bytes();
    if b.len() > 7 && b[6] == b'+' && b[..6].iter().all(|c| c.is_ascii_uppercase()) {
        name[7..].to_string()
    } else {
        name.to_string()
    }
}

fn font_is_bold(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.contains("bold") || l.contains("black") || l.contains("heavy") || l.contains("semibold") || l.contains("demi")
}

fn font_is_italic_name(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.contains("italic") || l.contains("oblique") || l.ends_with("-it") || l.contains("-it,") || l.contains("semiboldit") || l.contains("boldit")
}

fn font_is_mono_name(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.contains("courier") || l.contains("mono") || l.contains("consol") || l.contains("lucidaconsole")
}

/// Page coordinate frame: crop-box relative (the visible page), `/Rotate`
/// applied, y measured down from the top edge. Edexcel papers carry a
/// MediaBox larger than the CropBox, so raw user-space coordinates are offset
/// from the rendered page; every glyph, rule and path goes through this frame.
#[derive(Debug, Clone, Copy)]
struct Frame {
    left: f32,
    bottom: f32,
    raw_w: f32,
    raw_h: f32,
    /// 0 = none, 1 = 90°, 2 = 180°, 3 = 270° (clockwise, as pdfium renders).
    rot: u8,
}

impl Frame {
    fn new(page: &PdfPage) -> Frame {
        let rot = match page.rotation().unwrap_or(PdfPageRenderRotation::None) {
            PdfPageRenderRotation::None => 0,
            PdfPageRenderRotation::Degrees90 => 1,
            PdfPageRenderRotation::Degrees180 => 2,
            PdfPageRenderRotation::Degrees270 => 3,
        };
        let boundaries = page.boundaries();
        let rect = boundaries.crop().or_else(|_| boundaries.media()).map(|b| b.bounds).ok();
        match rect {
            Some(r) if r.width().value > 1.0 && r.height().value > 1.0 => Frame {
                left: r.left().value,
                bottom: r.bottom().value,
                raw_w: r.width().value,
                raw_h: r.height().value,
                rot,
            },
            _ => Frame { left: 0.0, bottom: 0.0, raw_w: page.width().value, raw_h: page.height().value, rot },
        }
    }
    /// Effective (rendered) page size.
    fn size(&self) -> (f32, f32) {
        if self.rot == 1 || self.rot == 3 {
            (self.raw_h, self.raw_w)
        } else {
            (self.raw_w, self.raw_h)
        }
    }
    /// Map a raw PDF user-space point (y up) into the frame (y down). Mirrors
    /// `stroke_census::rotate_rect_for_render`, then flips to y-down.
    fn pt(&self, x: f32, y: f32) -> [f32; 2] {
        let (x, y) = (x - self.left, y - self.bottom);
        match self.rot {
            0 => [x, self.raw_h - y],
            1 => [self.raw_h - y, self.raw_w - x],
            2 => [self.raw_w - x, y],
            _ => [y, x],
        }
    }
    fn rect(&self, l: f32, r: f32, t: f32, b: f32) -> [f32; 4] {
        let p = self.pt(l, t);
        let q = self.pt(r, b);
        [p[0].min(q[0]), p[1].min(q[1]), p[0].max(q[0]), p[1].max(q[1])]
    }
}

fn walk_objects(objects: &[PdfPageObject], parent: PdfMatrix, frame: &Frame, out: &mut PageGlyphs, depth: usize) {
    if depth > 8 {
        return;
    }
    for obj in objects {
        match obj {
            PdfPageObject::Path(p) => {
                let Ok(pm) = p.matrix() else { continue };
                let m = pm.multiply(parent);
                let segs = p.segments().transform(m);
                let mut pts: Vec<(bool, [f32; 2])> = Vec::new();
                let mut bad = false;
                for seg in segs.iter() {
                    let is_move = matches!(seg.segment_type(), PdfPathSegmentType::MoveTo);
                    let (x, y) = seg.point();
                    if !x.value.is_finite() || !y.value.is_finite() {
                        bad = true;
                        break;
                    }
                    pts.push((is_move, frame.pt(x.value, y.value)));
                }
                if bad || pts.is_empty() {
                    continue;
                }
                let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for (_, q) in &pts {
                    x0 = x0.min(q[0]);
                    x1 = x1.max(q[0]);
                    y0 = y0.min(q[1]);
                    y1 = y1.max(q[1]);
                }
                let filled = !matches!(p.fill_mode(), Ok(PdfPathFillMode::None));
                let stroked = p.is_stroked().unwrap_or(false);
                // One path object may draw several separate bars (MathType
                // draws every fraction bar of an equation as sub-paths of a
                // single path): each thin horizontal sub-path is its own rule.
                if filled || stroked {
                    let mut start = 0;
                    while start < pts.len() {
                        let mut end = start + 1;
                        while end < pts.len() && !pts[end].0 {
                            end += 1;
                        }
                        let sub = &pts[start..end];
                        let (mut sx0, mut sy0, mut sx1, mut sy1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                        for (_, q) in sub {
                            sx0 = sx0.min(q[0]);
                            sx1 = sx1.max(q[0]);
                            sy0 = sy0.min(q[1]);
                            sy1 = sy1.max(q[1]);
                        }
                        if sy1 - sy0 <= 1.6 && sx1 - sx0 >= 2.0 {
                            out.rules.push(Rule { x0: sx0, x1: sx1, y0: sy0, y1: sy1 });
                        }
                        start = end;
                    }
                }
                out.paths.push(VPath { x0, x1, y0, y1, pts, filled, stroked });
            }
            PdfPageObject::XObjectForm(f) => {
                let Ok(fm) = f.matrix() else { continue };
                let m = fm.multiply(parent);
                let children: Vec<PdfPageObject> = f.iter().collect();
                walk_objects(&children, m, frame, out, depth + 1);
            }
            PdfPageObject::Image(im) => {
                if let Ok(b) = im.bounds() {
                    // Bounds inside a form are in the form's space: place
                    // them on the page through the enclosing forms (a QR code
                    // drawn as an image in nested forms).
                    let corners = [(b.left(), b.top()), (b.right(), b.top()), (b.left(), b.bottom()), (b.right(), b.bottom())].map(|(x, y)| parent.apply_to_points(x, y));
                    let xs = corners.map(|c| c.0.value);
                    let ys = corners.map(|c| c.1.value);
                    let (left, right) = (xs.iter().copied().fold(f32::MAX, f32::min), xs.iter().copied().fold(f32::MIN, f32::max));
                    let (bottom, top) = (ys.iter().copied().fold(f32::MAX, f32::min), ys.iter().copied().fold(f32::MIN, f32::max));
                    let r = frame.rect(left, right, top, bottom);
                    // A hairline raster (some typesetters draw fraction bars
                    // as 1-pixel images) is a rule, not a figure.
                    if r[3] - r[1] <= 1.6 && r[2] - r[0] >= 2.0 {
                        out.rules.push(Rule { x0: r[0], x1: r[2], y0: r[1], y1: r[3] });
                    } else {
                        out.images.push(r);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Extract glyph + vector evidence for one page. Never fails: a page whose
/// text cannot be read yields an empty glyph list.
pub fn extract_page(page: &PdfPage, page_index: usize) -> PageGlyphs {
    extract_page_with(page, page_index, None)
}

/// As [`extract_page`], resolving simple-font codes that carry no Unicode
/// mapping through the page's font encoding catalog when one is supplied.
pub fn extract_page_with(page: &PdfPage, page_index: usize, fonts: Option<&PageFontCatalog>) -> PageGlyphs {
    let frame = Frame::new(page);
    let (width, height) = frame.size();
    let mut out = PageGlyphs { page: page_index, width, height, ..Default::default() };
    if let Ok(text) = page.text() {
        let text_chars = text.chars();
        let chars: Vec<PdfPageTextChar> = text_chars.iter().collect();
        let mut k = 0;
        // A word break the text layer infers from glyph spacing (a generated
        // space) between two glyphs of one line is word-break evidence too.
        let mut generated_space = false;
        while k < chars.len() {
            let c = &chars[k];
            k += 1;
            if c.is_generated().unwrap_or(false) {
                generated_space |= c.unicode_value() == 0x20;
                continue;
            }
            let mut raw = c.unicode_value();
            if raw == 0x0D || raw == 0x0A {
                continue;
            }
            // UTF-16 surrogate halves arrive as separate chars (Cambria Math
            // alphanumerics such as U+1D400): recombine them.
            if (0xD800..=0xDBFF).contains(&raw) && k < chars.len() {
                let lo = chars[k].unicode_value();
                if (0xDC00..=0xDFFF).contains(&lo) {
                    raw = 0x10000 + ((raw - 0xD800) << 10) + (lo - 0xDC00);
                    k += 1;
                }
            }
            if raw == 0x09 {
                raw = 0x20;
            }
            let Ok(tb) = c.tight_bounds() else { continue };
            let bx = frame.rect(tb.left().value, tb.right().value, tb.top().value, tb.bottom().value);
            let (ox, oy) = match c.origin() {
                Ok((x, y)) => {
                    let p = frame.pt(x.value, y.value);
                    (p[0], p[1])
                }
                Err(_) => (bx[0], bx[3]),
            };
            let (lx0, lx1) = match c.loose_bounds() {
                Ok(lb) => {
                    let r = frame.rect(lb.left().value, lb.right().value, lb.top().value, lb.bottom().value);
                    (r[0], r[2])
                }
                Err(_) => (bx[0], bx[2]),
            };
            let font = strip_subset(&c.font_name());
            let size = c.scaled_font_size().value;
            let size = if size.is_finite() && size > 0.0 { size } else { (bx[3] - bx[1]).max(1.0) };
            let italic = c.font_is_italic() || font_is_italic_name(&font);
            let bold = font_is_bold(&font)
                || c.font_weight().map(|w| matches!(w, PdfFontWeight::Weight600 | PdfFontWeight::Weight700Bold | PdfFontWeight::Weight800 | PdfFontWeight::Weight900)).unwrap_or(false);
            // Code fonts by name: the PDF fixed-pitch flag is also set on
            // symbol/maths fonts, whose glyphs are never code.
            let mono = font_is_mono_name(&font);
            let angle = c.angle_degrees().unwrap_or(0.0);
            let mut ch = resolve_identity(raw, &font);
            if ch == UNKNOWN_GLYPH {
                let advance = (lx1 - lx0).abs() / size.max(0.1) * 1000.0;
                if let Some(r) = resolve_symbol_fingerprint(raw, &font, advance) {
                    ch = r;
                }
            }
            if std::mem::take(&mut generated_space) {
                if let Some(prev) = out.glyphs.last() {
                    if (prev.oy - oy).abs() < 0.3 * size && bx[0] >= prev.x1 - 0.5 && bx[0] - prev.x1 < 1.5 * size {
                        out.generated_spaces.push([oy, prev.x1, bx[0]]);
                    }
                }
            }
            out.glyphs.push(Glyph {
                idx: c.index() as usize,
                ch,
                raw,
                font,
                italic,
                bold,
                mono,
                size,
                x0: bx[0],
                x1: bx[2],
                y0: bx[1],
                y1: bx[3],
                ox,
                oy,
                lx0,
                lx1,
                angle,
            });
        }
    }
    if let Some(cat) = fonts {
        resolve_control_codes(&mut out.glyphs, cat);
    }
    // Glyphs whose identity is still unknown: when they leave no ink at all
    // they are invisible placeholders (MathType spacing glyphs) and carry no
    // source content; visible unknowns stay as UNKNOWN_GLYPH for the gates.
    if out.glyphs.iter().any(|g| g.ch == UNKNOWN_GLYPH) {
        if let Some(ink) = render_ink(page) {
            let before = out.glyphs.len();
            out.glyphs.retain(|g| g.ch != UNKNOWN_GLYPH || ink.has_ink(g.x0, g.y0, g.x1, g.y1));
            out.invisible_dropped = before - out.glyphs.len();
        }
    }
    // A visible unidentified glyph whose ink is a thin horizontal bar (an
    // unmapped overline/macron accent, e.g. Cambria Math Boolean negation)
    // is geometrically a rule: hand it to the structure recogniser as one.
    let mut bars = Vec::new();
    out.glyphs.retain(|g| {
        let (w, h) = (g.x1 - g.x0, g.y1 - g.y0);
        let bar = g.ch == UNKNOWN_GLYPH && h > 0.0 && h <= 0.12 * g.size.max(1.0) && w >= 2.5 * h && w >= 2.0;
        if bar {
            bars.push(Rule { x0: g.x0, x1: g.x1, y0: g.y0, y1: g.y1 });
        }
        !bar
    });
    out.rules.extend(bars);
    // Overprinted duplicates (the same glyph drawn twice at one position,
    // e.g. Word equation text with an accessibility layer) are one glyph.
    // A ligature glyph ("ffi") is expanded by the text layer into
    // consecutive chars sharing the ligature's one box; its repeated
    // letters are source text, not an overprint. The shared advance cell is
    // wider than any single ligature letter, which an overprint never is.
    let ligature_part = |k: &Glyph, g: &Glyph| {
        g.idx == k.idx + 1 && matches!(k.ch, 'f' | 's') && matches!(g.ch, 'f' | 'i' | 'l' | 't') && (k.lx1 - k.lx0) >= 0.5 * k.size.max(1.0)
    };
    let before = out.glyphs.len();
    let mut kept: Vec<Glyph> = Vec::with_capacity(out.glyphs.len());
    for g in std::mem::take(&mut out.glyphs) {
        let dup = kept.iter().rev().take(8).any(|k| {
            k.ch == g.ch
                && !is_space_char(g.ch)
                && (k.x0 - g.x0).abs() < 0.3
                && (k.x1 - g.x1).abs() < 0.3
                && (k.y0 - g.y0).abs() < 0.3
                && (k.y1 - g.y1).abs() < 0.3
                && !ligature_part(k, &g)
        });
        if !dup {
            kept.push(g);
        }
    }
    out.duplicates_dropped = before - kept.len();
    out.glyphs = kept;
    for g in out.glyphs.iter_mut() {
        normalize_math_alnum(g);
    }
    identify_bracket_extensions(&mut out.glyphs);
    finalize_spaces(&mut out);
    let root: Vec<PdfPageObject> = page.objects().iter().collect();
    walk_objects(&root, PdfMatrix::IDENTITY, &frame, &mut out, 0);
    out.rules = merge_collinear_rules(std::mem::take(&mut out.rules));
    add_vector_bars(&mut out);
    add_vector_dots(&mut out);
    add_outline_glyphs(&mut out);
    add_inline_images(&mut out);
    out
}

/// The character standing for an image set inside a line of text.
pub const INLINE_IMAGE: char = '\u{FFFC}';

/// A picture inside a line of text (Naiker Maths prints dy/dx and other
/// formulas as embedded images): it keeps its place in the sentence as an
/// [`INLINE_IMAGE`] glyph, sized and based like the text beside it. Page
/// furniture images (logos in the top and bottom margins) and figure-sized
/// pictures are left to the figure regions.
fn add_inline_images(out: &mut PageGlyphs) {
    let (w, h) = (out.width, out.height);
    let mut next_idx = out.glyphs.iter().map(|g| g.idx + 1).max().unwrap_or(0);
    let mut keep: Vec<[f32; 4]> = Vec::new();
    let mut inline: Vec<Glyph> = Vec::new();
    for im in std::mem::take(&mut out.images) {
        let (iw, ih) = (im[2] - im[0], im[3] - im[1]);
        let margin = im[3] < 0.07 * h || im[1] > 0.93 * h;
        let host = out
            .glyphs
            .iter()
            .filter(|g| !is_space_char(g.ch) && g.cy() > im[1] && g.cy() < im[3])
            .filter(|g| (g.x0 - im[2]).max(im[0] - g.x1) >= -0.5 && (g.x0 - im[2]).max(im[0] - g.x1) <= 1.5 * g.size.max(4.0))
            .min_by(|a, b| {
                let da = (a.x0 - im[2]).max(im[0] - a.x1);
                let db = (b.x0 - im[2]).max(im[0] - b.x1);
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned();
        match host {
            Some(g) if !margin && iw >= 4.0 && ih >= 4.0 && ih <= 4.5 * g.size.max(4.0) && iw <= 0.6 * w => {
                inline.push(Glyph {
                    idx: next_idx,
                    ch: INLINE_IMAGE,
                    raw: INLINE_IMAGE as u32,
                    font: "InlineImage".to_string(),
                    italic: false,
                    bold: false,
                    mono: false,
                    size: g.size,
                    x0: im[0],
                    x1: im[2],
                    y0: im[1],
                    y1: im[3],
                    ox: im[0],
                    oy: g.oy,
                    lx0: im[0],
                    lx1: im[2],
                    angle: 0.0,
                });
                next_idx += 1;
                out.inline_images.push(im);
            }
            _ => keep.push(im),
        }
    }
    out.images = keep;
    out.glyphs.extend(inline);
}

/// A character drawn as filled outlines on a line of text (a symbol
/// converted to curves: physics '21 prints its ω so) has no identity the
/// source establishes. It is carried as an unidentified glyph, so the
/// question keeps its place in the text and can never pass as strict.
fn add_outline_glyphs(out: &mut PageGlyphs) {
    let mut next_idx = out.glyphs.iter().map(|g| g.idx + 1).max().unwrap_or(0);
    let mut outlined: Vec<Glyph> = Vec::new();
    for p in out.paths.iter().filter(|p| p.filled && p.pts.len() >= 20) {
        let (w, h) = (p.x1 - p.x0, p.y1 - p.y0);
        if !(3.0..=15.0).contains(&w) || !(3.0..=15.0).contains(&h) {
            continue;
        }
        let cy = (p.y0 + p.y1) * 0.5;
        let inked = out.glyphs.iter().any(|g| !is_space_char(g.ch) && g.x0 < p.x1 && g.x1 > p.x0 && g.y0 < p.y1 && g.y1 > p.y0);
        let host = out
            .glyphs
            .iter()
            .filter(|g| !is_space_char(g.ch) && g.font != "VectorOutline" && cy >= g.y0 - 1.0 && cy <= g.y1 + 1.0)
            .filter(|g| (g.x0 - p.x1).max(p.x0 - g.x1) <= 2.0 * g.size.max(4.0))
            .min_by(|a, b| (a.cx() - p.x0).abs().partial_cmp(&(b.cx() - p.x0).abs()).unwrap_or(std::cmp::Ordering::Equal));
        let (Some(host), false) = (host, inked) else { continue };
        outlined.push(Glyph {
            idx: next_idx,
            ch: UNKNOWN_GLYPH,
            raw: UNKNOWN_GLYPH as u32,
            font: "VectorOutline".to_string(),
            italic: false,
            bold: false,
            mono: false,
            size: host.size,
            x0: p.x0,
            x1: p.x1,
            y0: p.y0,
            y1: p.y1,
            ox: p.x0,
            oy: host.oy,
            lx0: p.x0,
            lx1: p.x1,
            angle: 0.0,
        });
        next_idx += 1;
    }
    out.glyphs.extend(outlined);
}

/// An unidentified glyph whose ink is a thin full-height stroke, standing in
/// the advance cell of a tall delimiter between that delimiter's top and
/// bottom pieces, is the delimiter's extension piece (SymbolMT subsets that
/// expose ⎜ under a control code with no usable name).
fn identify_bracket_extensions(glyphs: &mut [Glyph]) {
    let mut found: Vec<(usize, char)> = Vec::new();
    for (i, g) in glyphs.iter().enumerate() {
        let s = g.size.max(1.0);
        if g.ch != UNKNOWN_GLYPH || g.y1 - g.y0 < 0.6 * s {
            continue;
        }
        let column = |o: &Glyph| (o.lx0 - g.lx0).abs() <= 1.0 && (o.size - g.size).abs() <= 0.5;
        // A wide piece directly above (below) a delimiter's extension is
        // that delimiter's top (bottom).
        if g.x1 - g.x0 >= 0.25 * s {
            let touching = |above: bool| {
                glyphs.iter().find_map(|o| {
                    let (d, k) = piece_kind(o.ch)?;
                    let placed = if above { k != 2 && o.y1 <= g.y1 - 0.3 * s && o.y1 >= g.y0 - 2.0 } else { k != 0 && o.y0 >= g.y0 + 0.3 * s && o.y0 <= g.y1 + 2.0 };
                    (column(o) && placed).then_some(d)
                })
            };
            let piece = match (touching(true), touching(false)) {
                (None, Some(d)) => match d {
                    '(' => Some('⎛'),
                    ')' => Some('⎞'),
                    '[' => Some('⎡'),
                    ']' => Some('⎤'),
                    '{' => Some('⎧'),
                    '}' => Some('⎫'),
                    _ => None,
                },
                (Some(d), None) => match d {
                    '(' => Some('⎝'),
                    ')' => Some('⎠'),
                    '[' => Some('⎣'),
                    ']' => Some('⎦'),
                    '{' => Some('⎩'),
                    '}' => Some('⎭'),
                    _ => None,
                },
                _ => None,
            };
            if let Some(c) = piece {
                found.push((i, c));
            }
            continue;
        }
        if g.x1 - g.x0 > 0.15 * s {
            continue;
        }
        let delimiter = |kind: u8, above: bool| {
            glyphs.iter().find_map(|o| {
                let (d, k) = piece_kind(o.ch)?;
                // (Pieces overlap: the last extension runs into the bottom.)
                let placed = if above { o.y1 <= g.y1 - 0.3 * s && g.y0 - o.y1 <= 6.0 * s } else { o.y0 >= g.y0 + 0.3 * s && o.y0 - g.y1 <= 6.0 * s };
                (k == kind && column(o) && placed).then_some(d)
            })
        };
        if let (Some(top), Some(bottom)) = (delimiter(0, true), delimiter(2, false)) {
            if top == bottom {
                let ext = match top {
                    '(' => '⎜',
                    ')' => '⎟',
                    '[' => '⎢',
                    ']' => '⎥',
                    '{' | '}' => '⎪',
                    _ => continue,
                };
                found.push((i, ext));
            }
        }
    }
    for (i, c) in found {
        glyphs[i].ch = c;
    }
}

/// A small filled circle drawn level with a line's characters (the binary
/// point between the bits of a fixed point number) is a '●' of that line.
fn add_vector_dots(out: &mut PageGlyphs) {
    let mut next_idx = out.glyphs.iter().map(|g| g.idx + 1).max().unwrap_or(0);
    let mut dots: Vec<Glyph> = Vec::new();
    for p in out.paths.iter().filter(|p| p.filled) {
        let (w, h) = (p.x1 - p.x0, p.y1 - p.y0);
        if !(2.5..=12.0).contains(&w) || (w - h).abs() > 0.2 * w.max(h) || p.pts.len() < 9 {
            continue;
        }
        let (cx, cy, r) = ((p.x0 + p.x1) * 0.5, (p.y0 + p.y1) * 0.5, 0.25 * (w + h));
        let round = p.pts.iter().all(|(_, q)| {
            let d = (q[0] - cx).hypot(q[1] - cy);
            d >= 0.85 * r && d <= 1.2 * r
        });
        if !round || dots.iter().any(|d| (d.cx() - cx).abs() < 1.0 && (d.cy() - cy).abs() < 1.0) {
            continue;
        }
        let host = out
            .glyphs
            .iter()
            .filter(|g| g.ch.is_alphanumeric() && cy > g.y0 && cy < g.y1 && (g.cx() - cx).abs() <= 3.0 * g.size.max(4.0))
            .min_by(|a, b| (a.cx() - cx).abs().partial_cmp(&(b.cx() - cx).abs()).unwrap_or(std::cmp::Ordering::Equal));
        let Some(host) = host else { continue };
        dots.push(Glyph {
            idx: next_idx,
            ch: '●',
            raw: 0x25CF,
            font: "VectorDot".to_string(),
            italic: false,
            bold: false,
            mono: false,
            size: host.size,
            x0: p.x0,
            x1: p.x1,
            y0: p.y0,
            y1: p.y1,
            ox: p.x0,
            oy: host.oy,
            lx0: p.x0,
            lx1: p.x1,
            angle: 0.0,
        });
        next_idx += 1;
    }
    out.glyphs.extend(dots);
}

/// Delimiter bars drawn as strokes (MathType's |x| is two line segments, not
/// glyphs): a lone vertical segment about a line tall, set beside text and
/// not the edge of a box or table, is a bar glyph of the text it encloses.
fn add_vector_bars(out: &mut PageGlyphs) {
    let body = median(out.glyphs.iter().filter(|g| g.ch.is_alphanumeric()).map(|g| g.size).collect());
    if body <= 0.0 {
        return;
    }
    // Every stroked two-point vertical sub-path (one path may hold all the
    // bars of an expression: "|z − 3i| = 2|z|" is four move-to segments).
    let mut verticals: Vec<(f32, f32, f32)> = Vec::new();
    for p in out.paths.iter().filter(|p| p.stroked && !p.filled) {
        let mut sub: Vec<[f32; 2]> = Vec::new();
        let mut flush = |sub: &mut Vec<[f32; 2]>| {
            if let [a, b] = sub.as_slice() {
                if (a[0] - b[0]).abs() <= 1.0 {
                    verticals.push((0.5 * (a[0] + b[0]), a[1].min(b[1]), a[1].max(b[1])));
                }
            }
            sub.clear();
        };
        for &(moved, q) in &p.pts {
            if moved {
                flush(&mut sub);
            }
            sub.push(q);
        }
        flush(&mut sub);
    }
    let mut next_idx = out.glyphs.iter().map(|g| g.idx + 1).max().unwrap_or(0);
    let mut bars = Vec::new();
    for (k, &(x, y0, y1)) in verticals.iter().enumerate() {
        let h = y1 - y0;
        if h < 0.8 * body || h > 4.0 * body {
            continue;
        }
        // A box or table edge meets a horizontal rule at an end.
        // So does a side drawn in pieces (a collinear segment continues it).
        let boxed = out.rules.iter().any(|r| r.x0 - 1.5 <= x && x <= r.x1 + 1.5 && ((r.cy() - y0).abs() < 1.5 || (r.cy() - y1).abs() < 1.5))
            || verticals.iter().enumerate().any(|(j, &(qx, qy0, qy1))| {
                j != k && (qx - x).abs() < 0.8 && ((qy0 - y1).abs() < 1.5 || (qy1 - y0).abs() < 1.5)
            });
        let beside_text = out.glyphs.iter().any(|g| {
            !is_space_char(g.ch) && g.cy() > y0 && g.cy() < y1 && (g.x0 - x).abs().min((g.x1 - x).abs()) <= 1.5 * body
        });
        if boxed || !beside_text {
            continue;
        }
        bars.push(Glyph {
            idx: next_idx,
            ch: '|',
            raw: '|' as u32,
            font: "VectorBar".to_string(),
            italic: false,
            bold: false,
            mono: false,
            size: h / 1.2,
            x0: x - 0.4,
            x1: x + 0.4,
            y0,
            y1,
            ox: x - 0.1 * body,
            oy: y1 - 0.22 * h,
            lx0: x - 0.15 * body,
            lx1: x + 0.15 * body,
            angle: 0.0,
        });
        next_idx += 1;
    }
    out.glyphs.extend(bars);
}

/// Horizontal rules drawn as abutting/overlapping segments (stretched
/// overlines built from extender glyphs, table borders drawn per cell) are
/// one rule.
fn merge_collinear_rules(mut rules: Vec<Rule>) -> Vec<Rule> {
    rules.sort_by(|a, b| a.cy().partial_cmp(&b.cy()).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<Rule> = Vec::with_capacity(rules.len());
    let mut i = 0;
    while i < rules.len() {
        // One horizontal line: consecutive rules within 0.35pt of its centre.
        let mut j = i + 1;
        while j < rules.len() && (rules[j].cy() - rules[i].cy()).abs() <= 0.35 {
            j += 1;
        }
        let mut group: Vec<Rule> = rules[i..j].to_vec();
        group.sort_by(|a, b| a.x0.partial_cmp(&b.x0).unwrap_or(std::cmp::Ordering::Equal));
        let mut merged: Vec<Rule> = Vec::new();
        for r in group {
            if let Some(last) = merged.last_mut() {
                let same_weight = ((last.y1 - last.y0) - (r.y1 - r.y0)).abs() <= 0.5;
                if same_weight && r.x0 <= last.x1 + 0.3 {
                    last.x1 = last.x1.max(r.x1);
                    last.y0 = last.y0.min(r.y0);
                    last.y1 = last.y1.max(r.y1);
                    continue;
                }
            }
            merged.push(r);
        }
        out.extend(merged);
        i = j;
    }
    out
}

/// Mathematical alphanumeric symbols (U+1D400 block) are styled letters:
/// keep the base letter and carry the style in the glyph flags. Double-struck
/// letters keep their own code point (rendered with \mathbb).
fn normalize_math_alnum(g: &mut Glyph) {
    let c = g.ch as u32;
    if !(0x1D400..=0x1D7FF).contains(&c) {
        return;
    }
    // Letter blocks of 52 (A-Z, a-z) in style order.
    const STYLES: &[(u32, bool, bool)] = &[
        (0x1D400, true, false),  // bold
        (0x1D434, false, true),  // italic
        (0x1D468, true, true),   // bold italic
        (0x1D5A0, false, false), // sans-serif
        (0x1D5D4, true, false),  // sans-serif bold
        (0x1D608, false, true),  // sans-serif italic
        (0x1D63C, true, true),   // sans-serif bold italic
        (0x1D670, false, false), // monospace
    ];
    for &(start, bold, italic) in STYLES {
        if (start..start + 52).contains(&c) {
            let k = c - start;
            let base = if k < 26 { b'A' + k as u8 } else { b'a' + (k - 26) as u8 };
            g.ch = base as char;
            g.bold |= bold;
            g.italic = italic;
            return;
        }
    }
    // Bold / sans / monospace digits.
    if (0x1D7CE..=0x1D7FF).contains(&c) {
        let k = (c - 0x1D7CE) % 10;
        g.ch = (b'0' + k as u8) as char;
        g.bold |= c < 0x1D7D8 || (0x1D7EC..0x1D7F6).contains(&c);
    }
}

/// Grey-level page raster used only to decide whether an unidentified glyph
/// draws anything.
struct InkMap {
    gray: Vec<u8>,
    w: usize,
    h: usize,
    scale: f32,
}

impl InkMap {
    fn has_ink(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> bool {
        let px0 = ((x0 * self.scale).floor().max(0.0) as usize).min(self.w);
        let px1 = ((x1 * self.scale).ceil().max(0.0) as usize).min(self.w);
        let py0 = ((y0 * self.scale).floor().max(0.0) as usize).min(self.h);
        let py1 = ((y1 * self.scale).ceil().max(0.0) as usize).min(self.h);
        if px1 <= px0 || py1 <= py0 {
            // A zero-area box cannot be judged invisible from pixels.
            return true;
        }
        let mut dark = 0;
        for y in py0..py1 {
            for x in px0..px1 {
                if self.gray[y * self.w + x] < 170 {
                    dark += 1;
                    if dark >= 2 {
                        return true;
                    }
                }
            }
        }
        false
    }
}

fn render_ink(page: &PdfPage) -> Option<InkMap> {
    let scale = 2.0;
    let bitmap = page.render_with_config(&PdfRenderConfig::new().scale_page_by_factor(scale)).ok()?;
    let w = bitmap.width() as usize;
    let h = bitmap.height() as usize;
    let rgba = bitmap.as_rgba_bytes();
    if rgba.len() < w * h * 4 {
        return None;
    }
    let gray: Vec<u8> = rgba.chunks_exact(4).map(|p| ((p[0] as u32 + p[1] as u32 + p[2] as u32) / 3) as u8).collect();
    Some(InkMap { gray, w, h, scale })
}

/// Collect real space glyphs as word-break evidence (sorted by baseline).
pub fn finalize_spaces(pg: &mut PageGlyphs) {
    // Word spaces only: a hair, thin or zero-width space is a kern ("f′").
    pg.spaces = pg
        .glyphs
        .iter()
        .filter(|g| is_space_char(g.ch) && !matches!(g.ch, '\u{2009}' | '\u{200a}' | '\u{200b}'))
        .map(|g| [g.oy, g.lx0.min(g.x0), g.lx1.max(g.x1)])
        .collect();
    pg.thin_spaces = pg.glyphs.iter().filter(|g| g.ch == '\u{2009}').map(|g| [g.oy, g.lx0.min(g.x0), g.lx1.max(g.x1)]).collect();
    // A generated space spans from one glyph to the next in stream order:
    // with glyphs drawn inside that span ("(4 + 10e)" set as "(4", "10",
    // "+", "e"), it is no gap at all.
    let glyphs = &pg.glyphs;
    pg.generated_spaces.retain(|s| {
        !glyphs.iter().any(|g| !is_space_char(g.ch) && (g.oy - s[0]).abs() < 0.3 * g.size.max(1.0) && g.cx() > s[1] + 0.5 && g.cx() < s[2] - 0.5)
    });
    pg.generated_spaces.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));
    pg.spaces.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));
    pg.thin_spaces.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));
}

/// True when a real space glyph sits between `x_left` and `x_right` on
/// (about) the given baseline.
fn space_glyph_between(pg: &PageGlyphs, x_left: f32, x_right: f32, base: f32, size: f32) -> bool {
    space_between(&pg.spaces, x_left, x_right, base, size)
}

/// True when the text layer generated a word break between `x_left` and
/// `x_right` on (about) the given baseline.
fn generated_space_between(pg: &PageGlyphs, x_left: f32, x_right: f32, base: f32, size: f32) -> bool {
    space_between(&pg.generated_spaces, x_left, x_right, base, size)
}

fn space_between(spaces: &[[f32; 3]], x_left: f32, x_right: f32, base: f32, size: f32) -> bool {
    let tol = 0.3 * size.max(1.0);
    let start = spaces.partition_point(|s| s[0] < base - tol);
    for s in &spaces[start..] {
        if s[0] > base + tol {
            break;
        }
        let mid = (s[1] + s[2]) * 0.5;
        if mid >= x_left - 0.5 && mid <= x_right + 0.5 {
            return true;
        }
    }
    false
}

// ── Font encoding catalog ───────────────────────────────────────────────────
//
// Simple fonts embedded without a ToUnicode map expose raw codes through the
// text layer (MathType's SymbolMT subsets use codes 0x16–0x1F). Their
// identity is recorded in the font dictionary itself: `/Encoding
// /Differences` names each code's glyph and `/Widths` its advance. The
// catalog is read once per import with lopdf (already a dependency through
// pdf-extract); no glyph identity is ever guessed from context.

/// Glyph names of control-range codes (0x00–0x1F) shown with simple fonts
/// that define them through `/Encoding /Differences`, per
/// (base font, code) in content-stream order, with the advance width the
/// resource's `/Widths` declares for that code.
#[derive(Debug, Clone, Default)]
pub struct PageFontCatalog {
    pub control_glyphs: std::collections::HashMap<(String, u32), Vec<(String, f32)>>,
}

/// Read every page's control-glyph catalog by walking its content stream
/// (and nested form XObjects). Returns an empty vector when the document
/// cannot be parsed; resolution then simply does not apply.
pub fn font_catalog(path: &std::path::Path) -> Vec<PageFontCatalog> {
    let Ok(doc) = pdf_extract::Document::load(path) else { return Vec::new() };
    let mut out = Vec::new();
    for (_, page_id) in doc.get_pages() {
        let mut cat = PageFontCatalog::default();
        let res = page_resources(&doc, page_id);
        if let (Some(res), Ok(content)) = (res, doc.get_and_decode_page_content(page_id)) {
            let mut state = StreamState::default();
            walk_content(&doc, &content.operations, res, &mut state, &mut cat, 0);
        }
        out.push(cat);
    }
    out
}

fn page_resources(doc: &pdf_extract::Document, page_id: pdf_extract::ObjectId) -> Option<&pdf_extract::Dictionary> {
    let mut node = doc.get_dictionary(page_id).ok();
    for _ in 0..16 {
        let d = node?;
        if let Ok(res) = d.get(b"Resources") {
            if let Ok((_, res)) = doc.dereference(res) {
                if let Ok(res) = res.as_dict() {
                    return Some(res);
                }
            }
        }
        node = d.get(b"Parent").ok().and_then(|p| p.as_reference().ok()).and_then(|id| doc.get_dictionary(id).ok());
    }
    None
}

/// Current simple font: base name, differences, first char, widths.
#[derive(Debug, Clone, Default)]
struct StreamFont {
    base: String,
    simple: bool,
    first_char: u32,
    widths: Vec<f32>,
    differences: std::collections::HashMap<u32, String>,
}

#[derive(Debug, Clone, Default)]
struct StreamState {
    font: Option<StreamFont>,
    stack: Vec<Option<StreamFont>>,
}

fn font_from_resources(doc: &pdf_extract::Document, res: &pdf_extract::Dictionary, name: &[u8]) -> Option<StreamFont> {
    let fonts = res.get(b"Font").ok()?;
    let (_, fonts) = doc.dereference(fonts).ok()?;
    let fonts = fonts.as_dict().ok()?;
    let f = fonts.get(name).ok()?;
    let (_, f) = doc.dereference(f).ok()?;
    let font = f.as_dict().ok()?;
    let subtype = font.get(b"Subtype").ok().and_then(|s| s.as_name().ok()).unwrap_or(b"");
    let base = font.get(b"BaseFont").ok().and_then(|s| s.as_name().ok()).map(|b| strip_subset(&String::from_utf8_lossy(b))).unwrap_or_default();
    let simple = subtype != b"Type0";
    let first_char = font.get(b"FirstChar").ok().and_then(|o| o.as_i64().ok()).unwrap_or(0).max(0) as u32;
    let mut widths = Vec::new();
    if let Ok(w) = font.get(b"Widths") {
        if let Ok((_, w)) = doc.dereference(w) {
            if let Ok(arr) = w.as_array() {
                for o in arr {
                    let o = doc.dereference(o).map(|(_, o)| o).unwrap_or(o);
                    widths.push(o.as_float().or_else(|_| o.as_i64().map(|v| v as f32)).unwrap_or(0.0));
                }
            }
        }
    }
    let mut differences = std::collections::HashMap::new();
    if let Ok(enc) = font.get(b"Encoding") {
        if let Ok((_, enc)) = doc.dereference(enc) {
            if let Ok(ed) = enc.as_dict() {
                if let Ok(diffs) = ed.get(b"Differences") {
                    if let Ok((_, diffs)) = doc.dereference(diffs) {
                        if let Ok(arr) = diffs.as_array() {
                            let mut code: u32 = 0;
                            for o in arr {
                                if let Ok(n) = o.as_i64() {
                                    code = n.max(0) as u32;
                                } else if let Ok(name) = o.as_name() {
                                    differences.insert(code, String::from_utf8_lossy(name).to_string());
                                    code += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Some(StreamFont { base, simple, first_char, widths, differences })
}

fn record_codes(state: &StreamState, bytes: &[u8], cat: &mut PageFontCatalog) {
    let Some(f) = state.font.as_ref() else { return };
    if !f.simple {
        return;
    }
    for &b in bytes {
        let code = b as u32;
        if code >= 0x20 {
            continue;
        }
        let name = f.differences.get(&code).cloned().unwrap_or_default();
        let w = code.checked_sub(f.first_char).and_then(|i| f.widths.get(i as usize)).copied().unwrap_or(-1.0);
        cat.control_glyphs.entry((f.base.clone(), code)).or_default().push((name, w));
    }
}

fn walk_content(
    doc: &pdf_extract::Document,
    ops: &[pdf_extract::content::Operation],
    res: &pdf_extract::Dictionary,
    state: &mut StreamState,
    cat: &mut PageFontCatalog,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    for op in ops {
        match op.operator.as_str() {
            "q" => state.stack.push(state.font.clone()),
            "Q" => {
                if let Some(f) = state.stack.pop() {
                    state.font = f;
                }
            }
            "Tf" => {
                if let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) {
                    state.font = font_from_resources(doc, res, name);
                }
            }
            "Tj" | "'" => {
                if let Some(bytes) = op.operands.last().and_then(|o| o.as_str().ok()) {
                    record_codes(state, bytes, cat);
                }
            }
            "\"" => {
                if let Some(bytes) = op.operands.get(2).and_then(|o| o.as_str().ok()) {
                    record_codes(state, bytes, cat);
                }
            }
            "TJ" => {
                if let Some(arr) = op.operands.first().and_then(|o| o.as_array().ok()) {
                    for el in arr {
                        if let Ok(bytes) = el.as_str() {
                            record_codes(state, bytes, cat);
                        }
                    }
                }
            }
            "Do" => {
                let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) else { continue };
                let Some(xo) = res.get(b"XObject").ok().and_then(|x| doc.dereference(x).ok()).and_then(|(_, x)| x.as_dict().ok()) else { continue };
                let Some((_, obj)) = xo.get(name).ok().and_then(|o| doc.dereference(o).ok()) else { continue };
                let Ok(stream) = obj.as_stream() else { continue };
                if stream.dict.get(b"Subtype").ok().and_then(|s| s.as_name().ok()) != Some(b"Form".as_slice()) {
                    continue;
                }
                let form_res = stream
                    .dict
                    .get(b"Resources")
                    .ok()
                    .and_then(|r| doc.dereference(r).ok())
                    .and_then(|(_, r)| r.as_dict().ok())
                    .unwrap_or(res);
                let data = stream.decompressed_content().unwrap_or_else(|_| stream.content.clone());
                if let Ok(content) = pdf_extract::content::Content::decode(&data) {
                    let saved = state.font.clone();
                    walk_content(doc, &content.operations, form_res, state, cat, depth + 1);
                    state.font = saved;
                }
            }
            _ => {}
        }
    }
}

/// Resolve control-range codes (MathType SymbolMT subsets use 0x16–0x1F)
/// by aligning each (font, code) occurrence in text-layer order with the
/// same occurrence in content-stream order; applied only when the counts
/// agree exactly, so a misalignment can never assign a wrong glyph.
fn resolve_control_codes(glyphs: &mut [Glyph], cat: &PageFontCatalog) {
    let mut totals: std::collections::HashMap<(String, u32), usize> = std::collections::HashMap::new();
    for g in glyphs.iter() {
        if g.raw < 0x20 {
            *totals.entry((g.font.clone(), g.raw)).or_default() += 1;
        }
    }
    let mut seen: std::collections::HashMap<(String, u32), usize> = std::collections::HashMap::new();
    for g in glyphs.iter_mut() {
        if g.raw >= 0x20 {
            continue;
        }
        let key = (g.font.clone(), g.raw);
        let k = {
            let e = seen.entry(key.clone()).or_default();
            *e += 1;
            *e - 1
        };
        if g.ch != UNKNOWN_GLYPH {
            continue;
        }
        let Some(list) = cat.control_glyphs.get(&key) else { continue };
        // One name for every occurrence: unambiguous regardless of order.
        // Otherwise the occurrence counts must agree for positional pairing.
        let uniform = list.iter().all(|(n, _)| *n == list[0].0);
        let (name, w) = if uniform {
            &list[0]
        } else if list.len() == totals.get(&key).copied().unwrap_or(0) {
            &list[k]
        } else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        if let Some(c) = glyph_name_to_char(name, &g.font, if *w >= 0.0 { *w } else { -1.0 }) {
            g.ch = c;
        }
    }
}

/// Glyph name → character. `gNNN` names are glyph ids of the original
/// TrueType font: for Monotype SymbolMT (symbol.ttf) the glyph order follows
/// the Symbol encoding (ids 3..=97 are codes 0x20..=0x7E, ids 98.. are codes
/// 0xA1..), verified against the standard Symbol advance width.
fn glyph_name_to_char(name: &str, font: &str, width: f32) -> Option<char> {
    if let Some((code, _, w)) = SYMBOL_AFM.iter().find(|(_, n, _)| *n == name) {
        let _ = w;
        return adobe_symbol(*code);
    }
    // MathType double-struck glyph names ("Rbb", "Cbb", …).
    if name.len() == 3 && name.ends_with("bb") {
        let c = name.chars().next()?;
        if c.is_ascii_uppercase() {
            return Some(match c {
                'C' => 'ℂ',
                'H' => 'ℍ',
                'N' => 'ℕ',
                'P' => 'ℙ',
                'Q' => 'ℚ',
                'R' => 'ℝ',
                'Z' => 'ℤ',
                _ => char::from_u32(0x1D538 + (c as u32 - 'A' as u32))?,
            });
        }
    }
    if let Some(hex) = name.strip_prefix("uni") {
        if hex.len() == 4 {
            return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
        }
    }
    let lower = font.to_ascii_lowercase();
    if let Some(num) = name.strip_prefix('g') {
        if lower.contains("symbol") {
            let id: u32 = num.parse().ok()?;
            let candidates: Vec<u32> = if (3..=97).contains(&id) { vec![id + 29] } else { vec![id + 63, id + 62] };
            for code in candidates {
                if code > 0xFF {
                    continue;
                }
                if let Some((_, _, afm)) = SYMBOL_AFM.iter().find(|(c, _, _)| *c as u32 == code) {
                    if width < 0.0 || (*afm - width).abs() <= 25.0 {
                        return adobe_symbol(code as u8);
                    }
                }
            }
        }
        return None;
    }
    if name.chars().count() == 1 {
        return name.chars().next();
    }
    None
}

/// Symbol-encoded fonts with a generic subset name (Word's `TT…t00`) expose
/// codes U+F020..U+F0FF: accept the Symbol meaning only when the glyph's
/// measured advance matches the standard Symbol width for that code.
fn resolve_symbol_fingerprint(raw: u32, font: &str, advance: f32) -> Option<char> {
    if !(0xF020..=0xF0FF).contains(&raw) {
        return None;
    }
    let l = font.to_ascii_lowercase();
    if l.contains("wingding") || l.contains("dingbat") || l.contains("webding") || l.contains("zapf") {
        return None;
    }
    let code = (raw - 0xF000) as u8;
    let (_, _, afm) = SYMBOL_AFM.iter().find(|(c, _, _)| *c == code)?;
    if (*afm - advance).abs() <= 25.0 {
        adobe_symbol(code)
    } else {
        None
    }
}

/// Adobe Symbol encoding: (code, glyph name, advance width /1000 em).
const SYMBOL_AFM: &[(u8, &str, f32)] = &[
    (0x20, "space", 250.0), (0x21, "exclam", 333.0), (0x22, "universal", 713.0), (0x23, "numbersign", 500.0),
    (0x24, "existential", 549.0), (0x25, "percent", 833.0), (0x26, "ampersand", 778.0), (0x27, "suchthat", 439.0),
    (0x28, "parenleft", 333.0), (0x29, "parenright", 333.0), (0x2A, "asteriskmath", 500.0), (0x2B, "plus", 549.0),
    (0x2C, "comma", 250.0), (0x2D, "minus", 549.0), (0x2E, "period", 250.0), (0x2F, "slash", 278.0),
    (0x30, "zero", 500.0), (0x31, "one", 500.0), (0x32, "two", 500.0), (0x33, "three", 500.0), (0x34, "four", 500.0),
    (0x35, "five", 500.0), (0x36, "six", 500.0), (0x37, "seven", 500.0), (0x38, "eight", 500.0), (0x39, "nine", 500.0),
    (0x3A, "colon", 278.0), (0x3B, "semicolon", 278.0), (0x3C, "less", 549.0), (0x3D, "equal", 549.0),
    (0x3E, "greater", 549.0), (0x3F, "question", 444.0), (0x40, "congruent", 549.0), (0x41, "Alpha", 722.0),
    (0x42, "Beta", 667.0), (0x43, "Chi", 722.0), (0x44, "Delta", 612.0), (0x45, "Epsilon", 611.0), (0x46, "Phi", 763.0),
    (0x47, "Gamma", 603.0), (0x48, "Eta", 722.0), (0x49, "Iota", 333.0), (0x4A, "theta1", 631.0), (0x4B, "Kappa", 722.0),
    (0x4C, "Lambda", 686.0), (0x4D, "Mu", 889.0), (0x4E, "Nu", 722.0), (0x4F, "Omicron", 722.0), (0x50, "Pi", 768.0),
    (0x51, "Theta", 741.0), (0x52, "Rho", 556.0), (0x53, "Sigma", 592.0), (0x54, "Tau", 611.0), (0x55, "Upsilon", 690.0),
    (0x56, "sigma1", 439.0), (0x57, "Omega", 768.0), (0x58, "Xi", 645.0), (0x59, "Psi", 795.0), (0x5A, "Zeta", 611.0),
    (0x5B, "bracketleft", 333.0), (0x5C, "therefore", 863.0), (0x5D, "bracketright", 333.0), (0x5E, "perpendicular", 658.0),
    (0x5F, "underscore", 500.0), (0x60, "radicalex", 500.0), (0x61, "alpha", 631.0), (0x62, "beta", 549.0), (0x63, "chi", 549.0),
    (0x64, "delta", 494.0), (0x65, "epsilon", 439.0), (0x66, "phi", 521.0), (0x67, "gamma", 411.0), (0x68, "eta", 603.0),
    (0x69, "iota", 329.0), (0x6A, "phi1", 603.0), (0x6B, "kappa", 549.0), (0x6C, "lambda", 549.0), (0x6D, "mu", 576.0),
    (0x6E, "nu", 521.0), (0x6F, "omicron", 549.0), (0x70, "pi", 549.0), (0x71, "theta", 521.0), (0x72, "rho", 549.0),
    (0x73, "sigma", 603.0), (0x74, "tau", 439.0), (0x75, "upsilon", 576.0), (0x76, "omega1", 713.0), (0x77, "omega", 686.0),
    (0x78, "xi", 493.0), (0x79, "psi", 686.0), (0x7A, "zeta", 494.0), (0x7B, "braceleft", 480.0), (0x7C, "bar", 200.0),
    (0x7D, "braceright", 480.0), (0x7E, "similar", 549.0), (0xA1, "Upsilon1", 620.0), (0xA2, "minute", 247.0),
    (0xA3, "lessequal", 549.0), (0xA4, "fraction", 167.0), (0xA5, "infinity", 713.0), (0xA6, "florin", 500.0),
    (0xA7, "club", 753.0), (0xA8, "diamond", 753.0), (0xA9, "heart", 753.0), (0xAA, "spade", 753.0),
    (0xAB, "arrowboth", 1042.0), (0xAC, "arrowleft", 987.0), (0xAD, "arrowup", 603.0), (0xAE, "arrowright", 987.0),
    (0xAF, "arrowdown", 603.0), (0xB0, "degree", 400.0), (0xB1, "plusminus", 549.0), (0xB2, "second", 411.0),
    (0xB3, "greaterequal", 549.0), (0xB4, "multiply", 549.0), (0xB5, "proportional", 713.0), (0xB6, "partialdiff", 494.0),
    (0xB7, "bullet", 460.0), (0xB8, "divide", 549.0), (0xB9, "notequal", 549.0), (0xBA, "equivalence", 549.0),
    (0xBB, "approxequal", 549.0), (0xBC, "ellipsis", 1000.0), (0xBD, "arrowvertex", 603.0), (0xBE, "arrowhorizex", 1000.0),
    (0xBF, "carriagereturn", 658.0), (0xC0, "aleph", 823.0), (0xC1, "Ifraktur", 686.0), (0xC2, "Rfraktur", 795.0),
    (0xC3, "weierstrass", 987.0), (0xC4, "circlemultiply", 768.0), (0xC5, "circleplus", 768.0), (0xC6, "emptyset", 823.0),
    (0xC7, "intersection", 768.0), (0xC8, "union", 768.0), (0xC9, "propersuperset", 713.0), (0xCA, "reflexsuperset", 713.0),
    (0xCB, "notsubset", 713.0), (0xCC, "propersubset", 713.0), (0xCD, "reflexsubset", 713.0), (0xCE, "element", 713.0),
    (0xCF, "notelement", 713.0), (0xD0, "angle", 768.0), (0xD1, "gradient", 713.0), (0xD5, "product", 823.0),
    (0xD6, "radical", 549.0), (0xD7, "dotmath", 250.0), (0xD8, "logicalnot", 713.0), (0xD9, "logicaland", 603.0),
    (0xDA, "logicalor", 603.0), (0xDB, "arrowdblboth", 1042.0), (0xDC, "arrowdblleft", 987.0), (0xDD, "arrowdblup", 603.0),
    (0xDE, "arrowdblright", 987.0), (0xDF, "arrowdbldown", 603.0), (0xE0, "lozenge", 494.0), (0xE1, "angleleft", 329.0),
    (0xE5, "summation", 713.0), (0xE6, "parenlefttp", 384.0), (0xE7, "parenleftex", 384.0), (0xE8, "parenleftbt", 384.0),
    (0xE9, "bracketlefttp", 384.0), (0xEA, "bracketleftex", 384.0), (0xEB, "bracketleftbt", 384.0), (0xEC, "bracelefttp", 494.0),
    (0xED, "braceleftmid", 494.0), (0xEE, "braceleftbt", 494.0), (0xEF, "braceex", 494.0), (0xF1, "angleright", 329.0),
    (0xF2, "integral", 274.0), (0xF3, "integraltp", 686.0), (0xF4, "integralex", 686.0), (0xF5, "integralbt", 686.0),
    (0xF6, "parenrighttp", 384.0), (0xF7, "parenrightex", 384.0), (0xF8, "parenrightbt", 384.0), (0xF9, "bracketrighttp", 384.0),
    (0xFA, "bracketrightex", 384.0), (0xFB, "bracketrightbt", 384.0), (0xFC, "bracerighttp", 494.0), (0xFD, "bracerightmid", 494.0),
    (0xFE, "bracerightbt", 494.0),
];

// ── Glyph identity ──────────────────────────────────────────────────────────

/// Cambridge (CIE) `NewMathSymb` / `NewMathExtn` fonts carry a ToUnicode map
/// that is wrong for most maths glyphs (θ arrives as "1", π as "0", the big
/// parentheses as "P"/"Q"). Each entry below was established from the
/// rendered glyph shape in the source papers (glyph inventory of CIE 9709
/// June 2022 and the legacy CIE paper); codes not listed stay unknown.
fn cie_newmath(raw: u32, lower: &str) -> Option<char> {
    if lower.contains("newmathsymb") {
        return Some(match raw {
            0x21 => 'α',
            0x30 => 'π',
            0x31 => 'θ',
            0x3C => '<',
            0x3E => '>',
            0x8F => '≡',
            0xB3 => '•',
            0xC5 => '°',
            0x2264 => '⩽',
            0x2B | 0x3D | 0x2212 | 0x03C0 | 0x2192 => char::from_u32(raw)?,
            _ => UNKNOWN_GLYPH,
        });
    }
    if lower.contains("newmathextn") {
        return Some(match raw {
            0x00 | 0x10 | 0x40 | 0x50 | 0x60 => '(',
            0x01 | 0x11 | 0x41 | 0x51 | 0x61 => ')',
            0x0F => '√',
            0x74 => '{',
            _ => UNKNOWN_GLYPH,
        });
    }
    None
}

/// MathType's companion fonts (Euclid Math Two, Euclid Extra, MT Extra)
/// expose private-use codes through their ToUnicode maps. The codes are the
/// fonts' own fixed character assignments; each entry was established from
/// the rendered glyph in the Edexcel papers (glyph inventory).
fn mathtype_private(raw: u32, lower: &str) -> Option<char> {
    if lower.contains("euclidmathtwo") {
        return match raw {
            0xF052 => Some('ℝ'),
            0xF084 => Some('⩽'),
            0xF085 => Some('⩾'),
            _ => None,
        };
    }
    if lower.contains("euclidextra") {
        return match raw {
            0xF0A1 => Some('ℝ'),
            0xF0A5 => Some('ℕ'),
            _ => None,
        };
    }
    if lower.contains("mt-extra") || lower.contains("mtextra") {
        return match raw {
            0xF0A3 | 0x1F => Some('ℂ'),
            _ => None,
        };
    }
    None
}

/// Decorative dingbat fonts (page-corner markers, tier shapes, answer tick
/// boxes): never question text.
pub fn is_decoration_font(font: &str) -> bool {
    let l = font.to_ascii_lowercase();
    l.contains("wingding") || l.contains("webding") || l.contains("zapfdingbat") || l.contains("dingbats")
}

/// Resolve a glyph's identity from its raw text-layer value and font. Adobe
/// Symbol-encoding private-use code points (bracket pieces, extenders) map to
/// their standard Unicode equivalents; codes with no established identity
/// become `UNKNOWN_GLYPH`.
pub fn resolve_identity(raw: u32, font: &str) -> char {
    let lower = font.to_ascii_lowercase();
    if let Some(c) = cie_newmath(raw, &lower) {
        return c;
    }
    if let Some(c) = mathtype_private(raw, &lower) {
        return c;
    }
    if is_decoration_font(font) {
        // Word sets its "-->" / "<--" arrows in Wingdings (the light barb
        // arrows 0xDF–0xE2); every other dingbat is decoration.
        if lower.contains("wingdings") && !lower.contains("wingdings2") && !lower.contains("wingdings3") {
            match raw {
                0xF0DF => return '←',
                0xF0E0 => return '→',
                0xF0E1 => return '↑',
                0xF0E2 => return '↓',
                _ => {}
            }
        }
        return '■';
    }
    // Adobe Symbol encoding private-use assignments (Symbol, SymbolMT, …).
    if (0xF8E5..=0xF8FE).contains(&raw) {
        return match raw {
            0xF8E5 => '\u{203E}', // radical extender (overline)
            0xF8E6 => '\u{23D0}', // vertical arrow extender
            0xF8E7 => '\u{23AF}', // horizontal arrow extender
            0xF8EB => '\u{239B}', // left paren top
            0xF8EC => '\u{239C}', // left paren extension
            0xF8ED => '\u{239D}', // left paren bottom
            0xF8EE => '\u{23A1}', // left bracket top
            0xF8EF => '\u{23A2}', // left bracket extension
            0xF8F0 => '\u{23A3}', // left bracket bottom
            0xF8F1 => '\u{23A7}', // left brace top
            0xF8F2 => '\u{23A8}', // left brace middle
            0xF8F3 => '\u{23A9}', // left brace bottom
            0xF8F4 => '\u{23AA}', // brace extension
            0xF8F5 => '\u{23AE}', // integral extension
            0xF8F6 => '\u{239E}', // right paren top
            0xF8F7 => '\u{239F}', // right paren extension
            0xF8F8 => '\u{23A0}', // right paren bottom
            0xF8F9 => '\u{23A4}', // right bracket top
            0xF8FA => '\u{23A5}', // right bracket extension
            0xF8FB => '\u{23A6}', // right bracket bottom
            0xF8FC => '\u{23AB}', // right brace top
            0xF8FD => '\u{23AC}', // right brace middle
            0xF8FE => '\u{23AD}', // right brace bottom
            _ => UNKNOWN_GLYPH,
        };
    }
    // MathType's Euclid Symbol is Symbol-encoded: without a ToUnicode map
    // its upper codes surface as Latin-1 letters ("Ä" for ⊗, code 0xC4).
    if lower.contains("euclidsymbol") && (0xA0..=0xFF).contains(&raw) {
        return adobe_symbol(raw as u8).unwrap_or(UNKNOWN_GLYPH);
    }
    // Symbol-font codes surfaced as U+F0xx (no ToUnicode map): the low byte
    // is the Adobe Symbol encoding code.
    if (0xF020..=0xF0FF).contains(&raw) && lower.contains("symbol") {
        if let Some(c) = adobe_symbol((raw - 0xF000) as u8) {
            return c;
        }
        return UNKNOWN_GLYPH;
    }
    let Some(c) = char::from_u32(raw) else {
        return UNKNOWN_GLYPH;
    };
    if c == '\u{FFFD}' || (c as u32) < 0x20 || (0x7F..0xA0).contains(&(c as u32)) {
        return UNKNOWN_GLYPH;
    }
    if ('\u{E000}'..='\u{F8FF}').contains(&c) {
        return UNKNOWN_GLYPH;
    }
    c
}

/// Adobe Symbol font encoding (subset relevant to mathematics).
fn adobe_symbol(code: u8) -> Option<char> {
    Some(match code {
        0x20 => ' ',
        0x21 => '!',
        0x22 => '∀',
        0x23 => '#',
        0x24 => '∃',
        0x25 => '%',
        0x26 => '&',
        0x27 => '∋',
        0x28 => '(',
        0x29 => ')',
        0x2A => '∗',
        0x2B => '+',
        0x2C => ',',
        0x2D => '−',
        0x2E => '.',
        0x2F => '/',
        0x30..=0x39 => (b'0' + (code - 0x30)) as char,
        0x3A => ':',
        0x3B => ';',
        0x3C => '<',
        0x3D => '=',
        0x3E => '>',
        0x3F => '?',
        0x40 => '≅',
        0x41 => 'Α',
        0x42 => 'Β',
        0x43 => 'Χ',
        0x44 => 'Δ',
        0x45 => 'Ε',
        0x46 => 'Φ',
        0x47 => 'Γ',
        0x48 => 'Η',
        0x49 => 'Ι',
        0x4A => 'ϑ',
        0x4B => 'Κ',
        0x4C => 'Λ',
        0x4D => 'Μ',
        0x4E => 'Ν',
        0x4F => 'Ο',
        0x50 => 'Π',
        0x51 => 'Θ',
        0x52 => 'Ρ',
        0x53 => 'Σ',
        0x54 => 'Τ',
        0x55 => 'Υ',
        0x56 => 'ς',
        0x57 => 'Ω',
        0x58 => 'Ξ',
        0x59 => 'Ψ',
        0x5A => 'Ζ',
        0x5B => '[',
        0x5C => '∴',
        0x5D => ']',
        0x5E => '⊥',
        0x5F => '_',
        0x61 => 'α',
        0x62 => 'β',
        0x63 => 'χ',
        0x64 => 'δ',
        0x65 => 'ε',
        0x66 => 'φ',
        0x67 => 'γ',
        0x68 => 'η',
        0x69 => 'ι',
        0x6A => 'ϕ',
        0x6B => 'κ',
        0x6C => 'λ',
        0x6D => 'μ',
        0x6E => 'ν',
        0x6F => 'ο',
        0x70 => 'π',
        0x71 => 'θ',
        0x72 => 'ρ',
        0x73 => 'σ',
        0x74 => 'τ',
        0x75 => 'υ',
        0x76 => 'ϖ',
        0x77 => 'ω',
        0x78 => 'ξ',
        0x79 => 'ψ',
        0x7A => 'ζ',
        0x7B => '{',
        0x7C => '|',
        0x7D => '}',
        0x7E => '∼',
        0xA1 => 'ϒ',
        0xA2 => '′',
        0xA3 => '≤',
        0xA4 => '⁄',
        0xA5 => '∞',
        0xA6 => 'ƒ',
        0xA7 => '♣',
        0xA8 => '♦',
        0xA9 => '♥',
        0xAA => '♠',
        0xAB => '↔',
        0xAC => '←',
        0xAD => '↑',
        0xAE => '→',
        0xAF => '↓',
        0xB0 => '°',
        0xB1 => '±',
        0xB2 => '″',
        0xB3 => '≥',
        0xB4 => '×',
        0xB5 => '∝',
        0xB6 => '∂',
        0xB7 => '•',
        0xB8 => '÷',
        0xB9 => '≠',
        0xBA => '≡',
        0xBB => '≈',
        0xBC => '…',
        0xBD => '⏐',
        0xBE => '⎯',
        0xBF => '↵',
        0xC0 => 'ℵ',
        0xC1 => 'ℑ',
        0xC2 => 'ℜ',
        0xC3 => '℘',
        0xC4 => '⊗',
        0xC5 => '⊕',
        0xC6 => '∅',
        0xC7 => '∩',
        0xC8 => '∪',
        0xC9 => '⊃',
        0xCA => '⊇',
        0xCB => '⊄',
        0xCC => '⊂',
        0xCD => '⊆',
        0xCE => '∈',
        0xCF => '∉',
        0xD0 => '∠',
        0xD1 => '∇',
        0xD5 => '∏',
        0xD6 => '√',
        0xD7 => '⋅',
        0xD8 => '¬',
        0xD9 => '∧',
        0xDA => '∨',
        0xDB => '⇔',
        0xDC => '⇐',
        0xDD => '⇑',
        0xDE => '⇒',
        0xDF => '⇓',
        0xE0 => '◊',
        0xE1 => '〈',
        0xE5 => '∑',
        0xE6 => '⎛',
        0xE7 => '⎜',
        0xE8 => '⎝',
        0xE9 => '⎡',
        0xEA => '⎢',
        0xEB => '⎣',
        0xEC => '⎧',
        0xED => '⎨',
        0xEE => '⎩',
        0xEF => '⎪',
        0xF1 => '〉',
        0xF2 => '∫',
        0xF3 => '⌠',
        0xF4 => '⎮',
        0xF5 => '⌡',
        0xF6 => '⎞',
        0xF7 => '⎟',
        0xF8 => '⎠',
        0xF9 => '⎤',
        0xFA => '⎥',
        0xFB => '⎦',
        0xFC => '⎫',
        0xFD => '⎬',
        0xFE => '⎭',
        _ => return None,
    })
}

// ── Debug dump ──────────────────────────────────────────────────────────────

/// One-line-per-glyph diagnostic dump (development aid).
pub fn debug_glyphs(pg: &PageGlyphs) -> String {
    let mut s = String::new();
    for g in &pg.glyphs {
        s.push_str(&format!(
            "{:5} {:?} U+{:04X} o=({:.1},{:.1}) box=({:.1}-{:.1},{:.1}-{:.1}) lx=({:.1}-{:.1}) sz={:.1} {}{}{} {}\n",
            g.idx, g.ch, g.raw, g.ox, g.oy, g.x0, g.x1, g.y0, g.y1, g.lx0, g.lx1, g.size,
            if g.italic { "I" } else { "" }, if g.bold { "B" } else { "" }, if g.mono { "M" } else { "" },
            if g.angle.abs() > 0.01 { format!("{} ang={:.1}", g.font, g.angle) } else { g.font.clone() }
        ));
    }
    for r in &pg.rules {
        s.push_str(&format!("RULE x={:.1}-{:.1} y={:.1}-{:.1}\n", r.x0, r.x1, r.y0, r.y1));
    }
    for p in &pg.paths {
        if p.x1 - p.x0 > 0.5 * pg.width && p.y1 - p.y0 > 0.5 * pg.height {
            continue;
        }
        let pts: Vec<String> = p.pts.iter().take(12).map(|(m, q)| format!("{}{:.1},{:.1}", if *m { "M" } else { "" }, q[0], q[1])).collect();
        s.push_str(&format!(
            "PATH x={:.1}-{:.1} y={:.1}-{:.1} n={} fill={} stroke={} [{}]\n",
            p.x0, p.x1, p.y0, p.y1, p.pts.len(), p.filled, p.stroked, pts.join(" ")
        ));
    }
    for im in &pg.images {
        s.push_str(&format!("IMAGE x={:.1}-{:.1} y={:.1}-{:.1}\n", im[0], im[2], im[1], im[3]));
    }
    s
}

// ── Layout model ────────────────────────────────────────────────────────────

/// Why a line was withheld from the question text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FurnitureKind {
    Rotated,
    Barcode,
    Header,
    Footer,
    Margin,
    Watermark,
    AnswerLine,
    Continued,
    /// "Turn over for the next question" and similar navigation.
    Navigation,
    /// Answer prompt ("Answer", "x =") followed by an answer rule.
    AnswerPrompt,
    /// Text drawn inside a diagram (labels, axis values): part of the figure.
    Figure,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutLine {
    pub x0: f32,
    pub x1: f32,
    pub y0: f32,
    pub y1: f32,
    pub baseline: f32,
    pub size: f32,
    pub text: String,
    /// Glyph indices (into `PageGlyphs::glyphs`) consumed by this line.
    pub glyphs: Vec<usize>,
    pub furniture: Option<FurnitureKind>,
    /// First glyph of the line is bold (question/part heading evidence).
    pub bold_lead: bool,
    /// Glyphs the structure parser could not place confidently.
    pub unplaced: usize,
    /// A line of code set in a monospace font: `text` is verbatim and
    /// `char_width` is the font's advance (for indentation).
    pub code: bool,
    pub char_width: f32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayoutPage {
    pub page: usize,
    pub width: f32,
    pub height: f32,
    pub lines: Vec<LayoutLine>,
    pub furniture: Vec<LayoutLine>,
    /// Diagram regions `[x0, y0, x1, y1]` whose text was withheld as labels.
    pub figures: Vec<[f32; 4]>,
    /// Ruled tables `[x0, y0, x1, y1]` transcribed as Markdown tables.
    pub tables: Vec<[f32; 4]>,
    /// Pictures inside lines of text, in their lines as [`INLINE_IMAGE`].
    pub inline_images: Vec<[f32; 4]>,
    /// Ruled boxes whose text was transcribed as lines rather than a table
    /// (a row of bit cells): text, not a figure.
    pub text_grids: Vec<[f32; 4]>,
    /// Boxes of the page's drawn strokes and pictures outside any ruled
    /// grid: where something other than text and tables is drawn.
    pub drawings: Vec<[f32; 4]>,
    /// Pictures that are page furniture (printed at one place on many
    /// pages: a QR code, the margin strips), withheld from figures.
    pub furniture_images: Vec<[f32; 4]>,
}

impl LayoutPage {
    /// Body text: one line per visual line, furniture withheld.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for (i, l) in self.lines.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&l.text);
        }
        out
    }

    pub fn debug(&self) -> String {
        let mut s = String::new();
        let mut all: Vec<&LayoutLine> = self.lines.iter().chain(self.furniture.iter()).collect();
        all.sort_by(|a, b| a.baseline.partial_cmp(&b.baseline).unwrap_or(std::cmp::Ordering::Equal));
        for l in all {
            s.push_str(&format!(
                "{:6.1} [{:5.1}-{:5.1}] sz={:4.1}{}{}{} | {}\n",
                l.baseline,
                l.x0,
                l.x1,
                l.size,
                if l.bold_lead { " B" } else { "" },
                match l.furniture {
                    Some(k) => format!(" F:{:?}", k),
                    None => String::new(),
                },
                if l.unplaced > 0 { format!(" U{}", l.unplaced) } else { String::new() },
                l.text
            ));
        }
        s
    }
}

// ── Structure nodes ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccentKind {
    Bar,
    Overline,
    /// Text underlined in the source (a relation's key field).
    Underline,
    Hat,
    Tilde,
    Dot,
    DDot,
    Vec,
    OverArrow,
}

#[derive(Debug, Clone)]
enum Node {
    G(usize),
    Frac(Vec<Node>, Vec<Node>),
    Sqrt(Option<Vec<Node>>, Vec<Node>),
    Delim(char, char, Vec<Node>),
    Matrix(char, char, Vec<Vec<Vec<Node>>>),
    Cases(Vec<Vec<Node>>),
    Scripted {
        base: Box<Node>,
        sub: Option<Vec<Node>>,
        sup: Option<Vec<Node>>,
    },
    Pre {
        sub: Option<Vec<Node>>,
        sup: Option<Vec<Node>>,
    },
    BigOp {
        op: usize,
        lower: Option<Vec<Node>>,
        upper: Option<Vec<Node>>,
    },
    Accent(AccentKind, Vec<Node>),
    Space,
    /// A visibly wide gap (boxed terms, separated values) that must not
    /// collapse inside maths.
    WideSpace,
    Gap,
}

impl Node {
    fn is_struct(&self) -> bool {
        !matches!(self, Node::G(_) | Node::Space | Node::WideSpace | Node::Gap)
    }
}

/// A placed item during structure recognition: a glyph or a composite.
#[derive(Debug, Clone)]
struct Item {
    node: Node,
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
    /// Baseline (y down).
    base: f32,
    size: f32,
    glyphs: Vec<usize>,
    /// Must stay on the base line of whatever box holds it (big operators,
    /// tall delimiters).
    force_base: bool,
    /// Loose (advance) x-extent, for word-gap decisions.
    lx0: f32,
    lx1: f32,
    /// A single digit glyph (stacked nuclide numbers are full-size digits
    /// set off the baseline, so they cannot define it).
    numeral: bool,
    /// A single ordinary bracket glyph: an enlarged one (round a fraction)
    /// says nothing about the size of its line's text.
    bracket: bool,
}

impl Item {
    fn cx(&self) -> f32 {
        (self.x0 + self.x1) * 0.5
    }
    fn cy(&self) -> f32 {
        (self.y0 + self.y1) * 0.5
    }
    fn from_glyph(i: usize, g: &Glyph) -> Item {
        // A glyph whose bounds could not be measured still has an origin;
        // give it a nominal box so it keeps its reading position.
        let (x0, x1, y0, y1) = if g.x1 > g.x0 || g.y1 > g.y0 {
            (g.x0, g.x1.max(g.x0), g.y0, g.y1.max(g.y0))
        } else {
            (g.ox, g.ox + 0.5 * g.size, g.oy - 0.7 * g.size, g.oy)
        };
        let (lx0, lx1) = if g.lx1 > g.lx0 && g.lx0 <= x0 + 0.5 * g.size && g.lx1 >= x1 - 0.5 * g.size { (g.lx0, g.lx1) } else { (x0, x1) };
        // Enlarged delimiters are positioned by their extent, not their
        // origin (TeX/MathType big delimiters hang from the math axis): the
        // baseline they belong to sits ~0.25 em below their centre.
        let h = y1 - y0;
        let delim = matches!(g.ch, '(' | ')' | '[' | ']' | '{' | '}' | '|' | '‖' | '√');
        let origin_off = g.oy < y0 + 0.5 * h || g.oy > y1 + 0.5 * h;
        let base = if delim && (h > 1.3 * g.size.max(1.0) || origin_off) {
            (y0 + y1) * 0.5 + 0.25 * g.size
        } else {
            g.oy
        };
        // A list bullet is often set in a much larger font than its line (a
        // 28 pt bullet on 12 pt text, its origin lowered to centre it); its
        // nominal size and origin say nothing about the line, so it is placed
        // by its ink: about half an em tall, resting just above the baseline.
        // (A dot drawn as a path already carries its line's baseline.)
        let bullet = matches!(g.ch, '•' | '●' | '▪' | '■' | '◦') && h > 0.0 && g.font != "VectorDot";
        let size = if bullet { g.size.min((2.0 * h).max(4.0)) } else { g.size };
        let base = if bullet { y1 + 0.2 * h } else { base };
        Item { node: Node::G(i), x0, x1, y0, y1, base, size, glyphs: vec![i], force_base: bullet, lx0, lx1, numeral: g.ch.is_ascii_digit(), bracket: matches!(g.ch, '(' | ')' | '[' | ']' | '{' | '}') }
    }
}

fn union_box(items: &[Item]) -> (f32, f32, f32, f32) {
    let mut b = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for it in items {
        b.0 = b.0.min(it.x0);
        b.1 = b.1.max(it.x1);
        b.2 = b.2.min(it.y0);
        b.3 = b.3.max(it.y1);
    }
    b
}

fn median(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v[v.len() / 2]
}

fn hgap(a: &Item, b: &Item) -> f32 {
    if b.x0 >= a.x1 {
        b.x0 - a.x1
    } else if a.x0 >= b.x1 {
        a.x0 - b.x1
    } else {
        0.0
    }
}

// ── Furniture ───────────────────────────────────────────────────────────────

fn is_barcode_font(font: &str) -> bool {
    let l = font.to_ascii_lowercase();
    l.contains("barcode") || l.contains("bc39") || l.contains("precisionid") || l.contains("c39") || l.contains("code39") || l.contains("idautomation")
}

fn is_space_char(c: char) -> bool {
    c == ' ' || c == '\u{a0}' || c == '\t' || ('\u{2000}'..='\u{200b}').contains(&c) || c == '\u{3000}'
}

static WATERMARK_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)^(?:pmt|www\.[a-z0-9.\-]+|[a-z0-9\-]+\.(?:com|co\.uk|org|net)(?:/\S*)?)$").unwrap()
});

static HEADER_FOOTER_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(concat!(
        r"(?i)^(?:",
        r"\d{1,3}",
        r"|(?:\d{1,3}\s+)?\*[^*]{1,30}\*(?:\s*\[?turn over\]?\s*[►>]*)?(?:\s*\d{1,3})?",
        r"|\[?\s*turn over\s*\]?\s*[►>]*(?:\s*\d{1,3})?",
        r"|©.*|\(c\)\s*ucles.*|ucles\s+\d{4}.*",
        r"|[a-z0-9]{1,8}(?:/[a-z0-9]{1,8}){2,}(?:\s*\[?turn over\]?)?",
        // A paper reference, alone or beside the page number ("2  P76012A").
        r"|(?:\d{1,3}\s+)?[a-z]{1,3}\d{4,6}[a-z]\d{0,4}(?:\s+\d{1,3})?",
        r"|pmt|leave\s+blank",
        r"|do not write (?:in|outside) (?:this area|the box)",
        r")$"
    ))
    .unwrap()
});

static MARGIN_TEXT_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)^(?:do not write|outside the|box|do not write outside the box|do not write outside|the box|outside|leave blank|for examiner.?s use)\s*$").unwrap()
});

static NAVIGATION_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)^(?:do not write on this page|answer in the spaces provided|turn over for the next question|turn over for question \d+|question \d+ continues on the next page\.?|turn over\s*[►>]*|\(?\s*question \d+ continues? (?:on|over)(?: the)? (?:next|following) page\s*\)?\.?)$").unwrap()
});

/// "[Questions 10, 11 and 12 are printed on the next page.]"
static PRINTED_ON_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)^\[?\s*questions?\s+[\d,\sand]+\s+(?:is|are)\s+printed\s+on\s+the\s+(?:next|following)\s+pages?\.?\s*\]?$").unwrap()
});

/// Copyright and acknowledgement small print.
static BOILERPLATE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)permission to reproduce|copyright|\bucles\b|cambridge assessment|acknowledg").unwrap()
});

static CONTINUED_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)^\(?\s*question\s+\d{1,3}(?:\s*\([a-z]\))?\s+continued\s*\)?\s*$").unwrap()
});

static ANSWER_RUN_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"[.…]{5,}|_{3,}").unwrap());
static LATEX_WORD_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"\\[A-Za-z]+|[${}]").unwrap());

fn is_answer_line_text(t: &str) -> bool {
    let t = t.trim();
    // Dotted or ruled answer space, including a coordinate template
    // "( ………… , ………… )".
    if t.chars().count() >= 3
        && t.chars().all(|c| matches!(c, '_' | '.' | '…' | ' ' | '\t' | '-' | '‒' | '–' | '(' | ')' | ','))
        && t.chars().filter(|c| matches!(c, '_' | '.' | '…')).count() >= 3
    {
        return true;
    }
    // An answer template: dotted space around a short prompt or unit
    // ("……%", "£……", "$x =$ ……", "…… $\leqslant t <$ ……").
    let runs: usize = ANSWER_RUN_RE.find_iter(t).map(|m| m.as_str().chars().count()).sum();
    if runs == 0 {
        return false;
    }
    let rest = ANSWER_RUN_RE.replace_all(t, " ");
    let rest = LATEX_WORD_RE.replace_all(&rest, " ");
    let visible: String = rest.chars().filter(|c| !c.is_whitespace()).collect();
    runs >= 10 && visible.chars().count() <= 6 && rest.split_whitespace().count() <= 4
}

// ── Glyph classes used by structure recognition and emission ────────────────

fn is_math_font(font: &str) -> bool {
    let l = font.to_ascii_lowercase();
    l.contains("symbol") || l.contains("math") || l.contains("euclid") || l.contains("mt extra") || l.contains("mtextra") || l.starts_with("cmsy") || l.starts_with("cmmi") || l.starts_with("cmex") || l.starts_with("msam") || l.starts_with("msbm")
}

fn is_sans_font(font: &str) -> bool {
    let l = font.to_ascii_lowercase();
    l.contains("arial") || l.contains("helvetica") || l.contains("myriad") || l.contains("sans") || l.contains("verdana") || l.contains("calibri") || l.contains("chevin") || l.contains("frutiger") || l.contains("univers") || l.contains("gill")
}

fn is_operator_char(c: char) -> bool {
    matches!(
        c,
        '=' | '+' | '−' | '×' | '÷' | '<' | '>' | '≤' | '≥' | '⩽' | '⩾' | '±' | '∓' | '≠' | '≈' | '≡' | '∝' | '→' | '←' | '↔' | '∙'
            | '⇒' | '⇐' | '⇔' | '∞' | '∑' | '∏' | '∫' | '∂' | '∇' | '∈' | '∉' | '⊂' | '⊆' | '∪' | '∩' | '∠' | '√' | '∴' | '∵'
            | '⋅' | '·' | '∘' | '′' | '″' | '≅' | '∼' | '≃' | '⊥' | '∥' | '∅' | '∀' | '∃' | '⟹' | '⟺' | '↦' | '⊗' | '⊕' | '≪' | '≫'
    )
}

fn is_greek(c: char) -> bool {
    ('\u{0391}'..='\u{03A9}').contains(&c) || ('\u{03B1}'..='\u{03C9}').contains(&c) || matches!(c, 'ϑ' | 'ϕ' | 'ϖ' | 'ς' | 'ϵ')
}

fn is_open_delim(c: char) -> bool {
    matches!(c, '(' | '[' | '{' | '〈' | '⟨')
}

fn piece_kind(c: char) -> Option<(char, u8)> {
    // (delimiter, 0=top 1=extension 2=bottom 3=middle)
    Some(match c {
        '⎛' => ('(', 0),
        '⎜' => ('(', 1),
        '⎝' => ('(', 2),
        '⎞' => (')', 0),
        '⎟' => (')', 1),
        '⎠' => (')', 2),
        '⎡' => ('[', 0),
        '⎢' => ('[', 1),
        '⎣' => ('[', 2),
        '⎤' => (']', 0),
        '⎥' => (']', 1),
        '⎦' => (']', 2),
        '⎧' => ('{', 0),
        '⎨' => ('{', 3),
        '⎩' => ('{', 2),
        '⎪' => ('|', 1),
        '⎫' => ('}', 0),
        '⎬' => ('}', 3),
        '⎭' => ('}', 2),
        // A tall integral sign built from Symbol pieces.
        '⌠' => ('∫', 0),
        '⎮' => ('∫', 1),
        '⌡' => ('∫', 2),
        _ => return None,
    })
}

const FUNCTION_NAMES: &[&str] = &[
    "arcsin", "arccos", "arctan", "arsinh", "arcosh", "artanh", "sinh", "cosh", "tanh", "sech", "cosech", "coth", "cosec", "sin", "cos", "tan",
    "sec", "cot", "ln", "log", "exp", "lim", "max", "min", "det", "arg",
];

// ── Page layout ─────────────────────────────────────────────────────────────

/// Build the page layout: furniture separation, structure recognition, line
/// assembly and emission.
pub fn layout_page(pg: &PageGlyphs) -> LayoutPage {
    let mut lp = LayoutPage { page: pg.page, width: pg.width, height: pg.height, ..Default::default() };
    let n = pg.glyphs.len();
    let mut furniture: Vec<Option<FurnitureKind>> = vec![None; n];
    let mut is_space = vec![false; n];
    for (i, g) in pg.glyphs.iter().enumerate() {
        // The text layer reports a sheared (simulated-italic) glyph's slant
        // as an angle (a Symbol α at 18.9°); rotated margin text is at 90°.
        let a = g.angle.rem_euclid(360.0);
        if a > 25.0 && a < 335.0 {
            furniture[i] = Some(FurnitureKind::Rotated);
        } else if is_barcode_font(&g.font) {
            furniture[i] = Some(FurnitureKind::Barcode);
        } else if is_decoration_font(&g.font) && g.ch == '■' {
            furniture[i] = Some(FurnitureKind::Margin);
        }
        if is_space_char(g.ch) {
            is_space[i] = true;
        }
    }
    // Stream-order runs catch margin boilerplate and watermarks before they
    // can join a body line that happens to share their baseline.
    mark_stream_run_furniture(pg, &mut furniture);
    // A number in the right margin, standing clear of the line it shares a
    // baseline with, is an examiner's page-total box ("… TCP/IP stack.  15").
    for i in 0..n {
        let g = &pg.glyphs[i];
        if furniture[i].is_some() || !g.ch.is_ascii_digit() || g.x0 <= 0.92 * pg.width {
            continue;
        }
        let s = g.size.max(4.0);
        let same_row = |o: &Glyph| (o.oy - g.oy).abs() < 2.0 && !is_space_char(o.ch);
        let run: Vec<usize> = (0..n).filter(|&j| same_row(&pg.glyphs[j]) && pg.glyphs[j].x0 > 0.92 * pg.width).collect();
        if run.len() > 3 || !run.iter().all(|&j| pg.glyphs[j].ch.is_ascii_digit()) {
            continue;
        }
        let x0 = run.iter().map(|&j| pg.glyphs[j].x0).fold(f32::MAX, f32::min);
        let crowded = (0..n).any(|j| !run.contains(&j) && furniture[j].is_none() && same_row(&pg.glyphs[j]) && pg.glyphs[j].x1 > x0 - 3.0 * s && pg.glyphs[j].x1 <= x0);
        if !crowded {
            for j in run {
                furniture[j] = Some(FurnitureKind::Margin);
            }
        }
    }
    let shapes = radical_shapes(pg);
    let regions = figure_regions(pg, &shapes);
    for (i, g) in pg.glyphs.iter().enumerate() {
        if furniture[i].is_none() && !is_space[i] && regions.iter().any(|r| g.cx() >= r[0] - 2.0 && g.cx() <= r[2] + 2.0 && g.cy() >= r[1] - 2.0 && g.cy() <= r[3] + 2.0) {
            furniture[i] = Some(FurnitureKind::Figure);
        }
    }
    lp.figures = regions.clone();
    lp.inline_images = pg.inline_images.clone();

    let active: Vec<usize> = (0..n).filter(|&i| furniture[i].is_none() && !is_space[i]).collect();
    let mut items: Vec<Item> = active.iter().map(|&i| Item::from_glyph(i, &pg.glyphs[i])).collect();

    // ── Structure recognition (innermost first) ─────────────────────────
    let mut rule_used = vec![false; pg.rules.len()];
    mark_box_edges(pg, &items, &mut rule_used);
    // Table borders are never fraction bars, overlines or radical bars.
    let grids = table_grids(pg);
    for g in &grids {
        for &ri in &g.rules {
            rule_used[ri] = true;
        }
    }
    let in_grid = |b: [f32; 4]| {
        grids.iter().any(|g| b[0] >= g.xs[0] - 2.0 && b[2] <= g.xs[g.xs.len() - 1] + 2.0 && b[1] >= g.ys[0] - 2.0 && b[3] <= g.ys[g.ys.len() - 1] + 2.0)
    };
    // (A frame around the whole page is no drawing.)
    lp.drawings = pg
        .paths
        .iter()
        .map(|p| [p.x0, p.y0, p.x1, p.y1])
        .filter(|b| (b[2] - b[0]).max(b[3] - b[1]) >= 3.0 && !in_grid(*b) && (b[2] - b[0]) * (b[3] - b[1]) < 0.55 * pg.width * pg.height)
        .chain(pg.images.iter().copied())
        .collect();
    let mut shape_used = vec![false; shapes.len()];
    items = build_bracket_pieces(pg, items);
    loop {
        let before = items.len();
        let used_before = shape_used.iter().filter(|u| **u).count();
        items = build_vector_radicals(pg, items, &mut rule_used, &shapes, &mut shape_used);
        items = build_radicals(pg, items, &mut rule_used);
        items = build_fractions(pg, items, &mut rule_used);
        if items.len() == before && shape_used.iter().filter(|u| **u).count() == used_before {
            break;
        }
    }
    items = build_accents(pg, items, &mut rule_used);
    items = build_delimited(pg, items);
    items = build_big_op_limits(pg, items);

    // ── Tables ───────────────────────────────────────────────────────────
    // A ruled grid's cells are laid out one by one and the grid becomes a
    // single Markdown table block; rows of a table never interleave with
    // text beside it.
    let mut table_lines: Vec<LayoutLine> = Vec::new();
    // Blank axes to sketch on are answer space, like graph paper.
    let mut answer_grids: Vec<[f32; 4]> = blank_answer_axes(pg, &regions);
    for grid in &grids {
        // Row labels set just left of the grid, one per row ("R1", "15",
        // "R0" beside three rows of bit cells), are the table's first column.
        let labelled = with_row_labels(grid, &items);
        let grid = labelled.as_ref().unwrap_or(grid);
        let (inside, rest): (Vec<Item>, Vec<Item>) = items.into_iter().partition(|it| grid.contains(it.cx(), it.cy()));
        items = rest;
        let empty = inside.is_empty();
        match table_line(pg, grid, inside) {
            Ok(line) => {
                lp.tables.push([line.x0, line.y0, line.x1, line.y1]);
                table_lines.push(line);
            }
            Err(back) => {
                let rect = [grid.xs[0], grid.ys[0], grid.xs[grid.xs.len() - 1], grid.ys[grid.ys.len() - 1]];
                // An empty fine grid is graph paper to draw on; a grid with
                // text in it is that text.
                // (A curve drawn on it makes it a graph: a figure.)
                let drawn_on = regions.iter().any(|r| r[0] < rect[2] && r[2] > rect[0] && r[1] < rect[3] && r[3] > rect[1]);
                if empty && grid.xs.len() >= 7 && grid.ys.len() >= 7 && !drawn_on {
                    answer_grids.push(rect);
                } else if back.len() >= 2 {
                    lp.text_grids.push(rect);
                }
                items.extend(back)
            }
        }
    }

    // ── Lines ────────────────────────────────────────────────────────────
    let lines = group_lines(pg, items);
    let mut built: Vec<LayoutLine> = Vec::new();
    for line_items in lines {
        let glyphs: Vec<usize> = line_items.iter().flat_map(|it| it.glyphs.iter().copied()).collect();
        let (x0, x1, y0, y1) = union_box(&line_items);
        let first_glyph = line_items
            .iter()
            .min_by(|a, b| a.x0.partial_cmp(&b.x0).unwrap_or(std::cmp::Ordering::Equal))
            .and_then(|it| it.glyphs.first().copied());
        let bold_lead = first_glyph.map(|g| pg.glyphs[g].bold).unwrap_or(false);
        // Code (a monospace line: a program, an SQL statement) is copied
        // verbatim: no maths, no Markdown interpretation.
        if let Some((text, char_width)) = code_line_text(pg, &glyphs) {
            let base = median(glyphs.iter().map(|&g| pg.glyphs[g].oy).collect());
            let size = median(glyphs.iter().map(|&g| pg.glyphs[g].size).collect());
            built.push(LayoutLine { x0, x1, y0, y1, baseline: base, size, text, glyphs, furniture: None, bold_lead, unplaced: 0, code: true, char_width });
            continue;
        }
        let (nodes, base, size, unplaced) = parse_box(pg, line_items);
        let text = emit_nodes(pg, &nodes, false).trim().to_string();
        if text.is_empty() {
            continue;
        }
        built.push(LayoutLine { x0, x1, y0, y1, baseline: base, size, text, glyphs, furniture: None, bold_lead, unplaced, code: false, char_width: 0.0 });
    }
    attach_option_letters(pg, &mut built);
    let mut kinds: Vec<Option<FurnitureKind>> = (0..built.len()).map(|i| classify_line_furniture(pg, &built, i, &rule_used, &regions, &answer_grids)).collect();
    // A short line set against a figure's labels belongs to the figure too
    // (an axis title beyond its tick values, "Velocity (m/s)" left of the
    // scale).
    for _ in 0..2 {
        let labels: Vec<[f32; 4]> = (0..built.len()).filter(|&i| kinds[i] == Some(FurnitureKind::Figure)).map(|i| [built[i].x0, built[i].y0, built[i].x1, built[i].y1]).collect();
        for i in 0..built.len() {
            let l = &built[i];
            let printed = l.glyphs.iter().filter(|&&g| !is_space_char(pg.glyphs[g].ch)).count();
            if kinds[i].is_some() || l.code || printed > 24 || l.bold_lead || FIGURE_LABEL_KEEP_RE.is_match(plain_line(&l.text).trim()) {
                continue;
            }
            let s = l.size.max(4.0);
            let near = labels.iter().any(|b| l.x0 <= b[2] + 1.5 * s && l.x1 >= b[0] - 1.5 * s && l.y0 <= b[3] + 1.5 * s && l.y1 >= b[1] - 1.5 * s);
            if near {
                kinds[i] = Some(FurnitureKind::Figure);
            }
        }
    }
    for (mut line, kind) in built.into_iter().zip(kinds) {
        line.furniture = kind;
        if line.furniture.is_some() {
            lp.furniture.push(line);
        } else {
            lp.lines.push(line);
        }
    }
    // Glyph-level furniture, grouped per baseline for the debug view.
    let mut f_items: Vec<usize> = (0..n).filter(|&i| furniture[i].is_some() && !is_space[i]).collect();
    f_items.sort_by(|&a, &b| {
        (pg.glyphs[a].oy, pg.glyphs[a].x0).partial_cmp(&(pg.glyphs[b].oy, pg.glyphs[b].x0)).unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut k = 0;
    while k < f_items.len() {
        let kind = furniture[f_items[k]];
        let mut j = k;
        let mut text = String::new();
        while j < f_items.len() && furniture[f_items[j]] == kind && (pg.glyphs[f_items[j]].oy - pg.glyphs[f_items[k]].oy).abs() < 2.0 {
            text.push(pg.glyphs[f_items[j]].ch);
            j += 1;
        }
        let g = &pg.glyphs[f_items[k]];
        lp.furniture.push(LayoutLine {
            x0: g.x0,
            x1: g.x1,
            y0: g.y0,
            y1: g.y1,
            baseline: g.oy,
            size: g.size,
            text,
            glyphs: f_items[k..j].to_vec(),
            furniture: kind,
            bold_lead: false,
            unplaced: 0,
            code: false,
            char_width: 0.0,
        });
        k = j.max(k + 1);
    }
    lp.lines.extend(table_lines);
    lp.lines.sort_by(|a, b| a.baseline.partial_cmp(&b.baseline).unwrap_or(std::cmp::Ordering::Equal));
    lp
}

/// Pictures printed at one place on several pages (each page's QR code, a
/// logo, the "do not write" margin strips): page furniture, per page.
fn repeated_images(pages: &[PageGlyphs]) -> Vec<Vec<[f32; 4]>> {
    // The same place: centres within a few points, sizes within 15% (each
    // page's own QR code is placed a point or two apart).
    let same = |a: &[f32; 4], b: &[f32; 4]| {
        let (aw, ah, bw, bh) = (a[2] - a[0], a[3] - a[1], b[2] - b[0], b[3] - b[1]);
        ((a[0] + a[2]) - (b[0] + b[2])).abs() <= 12.0
            && ((a[1] + a[3]) - (b[1] + b[3])).abs() <= 12.0
            && (aw - bw).abs() <= 0.15 * aw.max(bw)
            && (ah - bh).abs() <= 0.15 * ah.max(bh)
    };
    // A small picture (a code, a logo) on three pages; a larger one on a
    // quarter of the paper too.
    let need = |im: &[f32; 4]| if (im[2] - im[0]).max(im[3] - im[1]) <= 90.0 { 3 } else { 3.max((pages.len() + 3) / 4) };
    pages
        .iter()
        .map(|pg| pg.images.iter().copied().filter(|im| pages.iter().filter(|o| o.images.iter().any(|b| same(im, b))).count() >= need(im)).collect())
        .collect()
}

/// Lay out every page of a document: pictures repeated as page furniture
/// are set aside first, so no page takes one for a figure.
pub fn layout_document(mut pages: Vec<PageGlyphs>) -> Vec<LayoutPage> {
    let furniture = repeated_images(&pages);
    pages
        .iter_mut()
        .zip(furniture)
        .map(|(pg, f)| {
            pg.images.retain(|im| !f.contains(im));
            let mut lp = layout_page(pg);
            lp.furniture_images = f;
            lp
        })
        .collect()
}

/// Stream-order phrase runs → margin boilerplate / watermark furniture.
fn mark_stream_run_furniture(pg: &PageGlyphs, furniture: &mut [Option<FurnitureKind>]) {
    let n = pg.glyphs.len();
    let mut i = 0;
    while i < n {
        if furniture[i].is_some() {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < n && furniture[j].is_none() {
            let a = &pg.glyphs[j - 1];
            let b = &pg.glyphs[j];
            let same_base = (a.oy - b.oy).abs() <= 0.2 * a.size.max(b.size);
            let same_size = (a.size - b.size).abs() <= 0.12 * a.size.max(b.size);
            let gap = b.x0 - a.x1;
            let adjacent = gap >= -0.3 * a.size && gap <= 0.9 * a.size;
            if !(same_base && same_size && adjacent) {
                break;
            }
            j += 1;
        }
        let text: String = pg.glyphs[i..j].iter().map(|g| g.ch).collect();
        let t = text.trim();
        let x0 = pg.glyphs[i..j].iter().filter(|g| !is_space_char(g.ch)).map(|g| g.x0).fold(f32::MAX, f32::min);
        let kind = if WATERMARK_RE.is_match(t) {
            Some(FurnitureKind::Watermark)
        } else if MARGIN_TEXT_RE.is_match(t) && x0 > 0.84 * pg.width {
            Some(FurnitureKind::Margin)
        } else {
            None
        };
        if let Some(k) = kind {
            for f in furniture.iter_mut().take(j).skip(i) {
                *f = Some(k);
            }
        }
        i = j;
    }
}

fn classify_line_furniture(pg: &PageGlyphs, all: &[LayoutLine], idx: usize, rule_used: &[bool], regions: &[[f32; 4]], answer_grids: &[[f32; 4]]) -> Option<FurnitureKind> {
    let line = &all[idx];
    let t = line.text.trim();
    // The scales and axis titles of graph paper to draw on belong to that
    // answer space ("Frequency", "120 140 ... 220" around the grid).
    let s = line.size.max(4.0);
    if line.glyphs.len() <= 30
        && answer_grids.iter().any(|g| line.x0 <= g[2] + 2.0 * s && line.x1 >= g[0] - 6.0 * s && line.y0 <= g[3] + 2.5 * s && line.y1 >= g[1] - 0.5 * s)
    {
        return Some(FurnitureKind::AnswerLine);
    }
    if is_answer_line_text(t) {
        return Some(FurnitureKind::AnswerLine);
    }
    if CONTINUED_RE.is_match(t) {
        return Some(FurnitureKind::Continued);
    }
    if NAVIGATION_RE.is_match(t) || PRINTED_ON_RE.is_match(plain_line(t).trim()) {
        return Some(FurnitureKind::Navigation);
    }
    // The publisher's small-print notices at the foot of a page (copyright
    // acknowledgements): a block of small lines one of which names it.
    let page_body = median(all.iter().map(|l| l.size).collect());
    let small_print = |l: &LayoutLine| l.y0 > 0.85 * pg.height && l.size <= 0.75 * page_body;
    if small_print(line) && all.iter().any(|o| small_print(o) && BOILERPLATE_RE.is_match(&o.text)) {
        return Some(FurnitureKind::Footer);
    }
    // Short text hugging a diagram is one of its labels (a label placed just
    // outside the drawn strokes, or the remainder of a label straddling the
    // region edge). Question headings above a diagram are kept.
    // Visible length: the characters printed, not the LaTeX that spells them
    // ("$2.8 \mathrm{m} \mathrm{s}^{-1}$" is seven characters).
    let printed = line.glyphs.iter().filter(|&&g| !is_space_char(pg.glyphs[g].ch)).count();
    let plain_len = if printed > 0 { printed } else { t.replace('$', "").chars().count() };
    let heading_like = line.bold_lead && t.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false);
    // (A caption, a mark allocation or a part label is never a label.)
    let keep = FIGURE_LABEL_KEEP_RE.is_match(plain_line(t).trim());
    if !heading_like && !keep && plain_len <= 24 {
        let (cx, cy) = ((line.x0 + line.x1) * 0.5, (line.y0 + line.y1) * 0.5);
        let _ = rule_used;
        if regions.iter().any(|r| cx >= r[0] - 18.0 && cx <= r[2] + 18.0 && cy >= r[1] - 18.0 && cy <= r[3] + 18.0) {
            return Some(FurnitureKind::Figure);
        }
        // A label set at the tip of a stroke, starting just past the
        // drawing's right edge ("does not win" at a branch end, "7.6 cm"
        // beside a side) or ending just before its left edge.
        let s = line.size.max(4.0);
        if regions.iter().any(|r| {
            line.y1 >= r[1] && line.y0 <= r[3] && (line.x0 >= r[2] - s && line.x0 <= r[2] + s || line.x1 <= r[0] + s && line.x1 >= r[0] - s)
        }) {
            return Some(FurnitureKind::Figure);
        }
        // The rest of a label that starts inside the drawing and runs out of
        // it ("2x² + 2xy + y² = 50" across the region's edge).
        let s = line.size.max(4.0);
        let straddles = regions.iter().any(|r| {
            pg.glyphs.iter().any(|g| {
                !is_space_char(g.ch)
                    && (g.oy - line.baseline).abs() < 1.0
                    && g.cx() >= r[0] - 2.0
                    && g.cx() <= r[2] + 2.0
                    && g.cy() >= r[1] - 2.0
                    && g.cy() <= r[3] + 2.0
                    && g.x1 <= line.x0 + 1.0
                    && g.x1 >= line.x0 - s
            })
        });
        if straddles {
            return Some(FurnitureKind::Figure);
        }
    }
    // Right-margin examiner boxes (page mark totals).
    if line.x0 > 0.9 * pg.width && t.chars().count() <= 4 && t.chars().all(|c| c.is_ascii_digit()) {
        return Some(FurnitureKind::Margin);
    }
    // "Answer" beside the answer boxes drawn for it is part of that space.
    if plain_line(t).trim().eq_ignore_ascii_case("answer")
        && regions.iter().any(|r| r[0] >= line.x1 - 1.0 && r[0] <= line.x1 + 6.0 * s && r[1] <= line.baseline && r[3] >= line.y0)
    {
        return Some(FurnitureKind::AnswerPrompt);
    }
    // A question/part heading line is never an answer prompt, and neither is
    // code (an SQL skeleton "CREATE TABLE Animal (" before its answer lines).
    let starts_heading = line.bold_lead && t.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false);
    if !starts_heading && !line.code && (is_answer_prompt(pg, line, rule_used) || is_answer_template(pg, line, rule_used)) {
        return Some(FurnitureKind::AnswerPrompt);
    }
    // An empty tall bracket pair is an answer grid ("M = ( __ __ )").
    if t.replace("\\left(\\right)", "").replace("\\left[\\right]", "").replace('$', "").trim().is_empty() {
        return Some(FurnitureKind::AnswerPrompt);
    }
    let plain: String = t.replace('$', "");
    let plain = plain.trim();
    let top_zone = line.y1 < 0.068 * pg.height;
    let bottom_zone = line.y0 > 0.915 * pg.height;
    // Running headers/footers stand apart from the body: a zone line that
    // touches a neighbouring body line (an exponent, a stacked limit) is not
    // page furniture.
    let isolated = !all.iter().enumerate().any(|(j, o)| {
        j != idx && {
            let s = line.size.max(o.size).max(4.0);
            let hov = o.x1 >= line.x0 - 2.0 * s && o.x0 <= line.x1 + 2.0 * s;
            let vgap = (o.y0.max(line.y0) - o.y1.min(line.y1)).max(0.0);
            hov && vgap < 0.9 * s
        }
    });
    if (top_zone || bottom_zone) && isolated && (HEADER_FOOTER_RE.is_match(plain) || WATERMARK_RE.is_match(plain)) {
        return Some(if top_zone { FurnitureKind::Header } else { FurnitureKind::Footer });
    }
    if line.x0 > 0.84 * pg.width && MARGIN_TEXT_RE.is_match(t) {
        return Some(FurnitureKind::Margin);
    }
    None
}

/// A short prompt line ("Answer", "x =", "dy/dx =", "a = … b = …") whose
/// answer is written on a rule directly to its right.
fn is_answer_prompt(pg: &PageGlyphs, line: &LayoutLine, rule_used: &[bool]) -> bool {
    let t = line.text.trim();
    if t.is_empty() || t.chars().count() > 60 {
        return false;
    }
    let plain = t.replace('$', "");
    let plain = plain.trim();
    let words = plain.split_whitespace().count();
    let lower = plain.to_ascii_lowercase();
    let looks_prompt = lower == "answer"
        || lower.starts_with("answer ")
        || lower.starts_with("answer\t")
        || plain.ends_with('=')
        || plain.ends_with("= ")
        || plain.contains("=\t")
        || t.contains("=$\t")
        || t.ends_with("=$");
    let s = line.size.max(6.0);
    // A short label followed on its own baseline by a long fill-in rule
    // ("Function 1 ______", "Working ______", "Advantage ______").
    let labelled_space = words <= 4
        && !plain.ends_with(['.', '?', '!', ':', ';'])
        && pg.rules.iter().enumerate().any(|(ri, r)| {
            !rule_used[ri]
                && r.w() >= 8.0 * s
                && r.x0 >= line.x1 - 2.0
                && r.x0 <= line.x1 + 1.5 * s
                && r.cy() >= line.baseline - 0.5 * s
                && r.cy() <= line.baseline + 0.6 * s
        });
    if labelled_space {
        return true;
    }
    if !looks_prompt || words > 8 {
        return false;
    }
    let ends_eq = plain.ends_with('=');
    // ("Answer" beside a row of answer boxes: their edges belong to a grid.)
    let label = lower == "answer";
    pg.rules.iter().enumerate().any(|(ri, r)| {
        (!rule_used[ri] || label)
            && r.w() >= 1.5 * s
            && r.x0 >= line.x0 - 1.0
            && r.x0 <= line.x1 + if ends_eq { 6.0 } else { 4.0 } * s
            && r.cy() >= line.baseline - if ends_eq { 2.5 } else { 0.5 } * s
            && r.cy() <= line.baseline + if ends_eq { 2.5 } else { 1.2 } * s
    })
}

/// A labels-and-blanks template line ("Stationary point ( , ) Nature") whose
/// blanks are answer rules laid within the line.
fn is_answer_template(pg: &PageGlyphs, line: &LayoutLine, rule_used: &[bool]) -> bool {
    let plain = line.text.replace('$', "");
    let plain = plain.trim();
    if plain.is_empty() || plain.split_whitespace().count() > 8 || plain.ends_with('.') || plain.ends_with('?') || plain.contains("mark") {
        return false;
    }
    let s = line.size.max(6.0);
    let blank: f32 = pg
        .rules
        .iter()
        .enumerate()
        .filter(|(ri, r)| {
            !rule_used[*ri] && r.x0 >= line.x0 - 0.5 * s && r.x0 <= line.x1 + 2.0 * s && r.cy() >= line.baseline - 0.3 * s && r.cy() <= line.baseline + 0.9 * s && r.w() < 0.6 * pg.width
        })
        .map(|(_, r)| r.w())
        .sum();
    blank >= 3.0 * s && blank >= 0.4 * (line.x1 - line.x0)
}

fn items_in(items: &[Item], f: impl Fn(&Item) -> bool) -> Vec<usize> {
    items.iter().enumerate().filter(|(_, it)| f(it)).map(|(i, _)| i).collect()
}

/// Remove `idx` from `items`; returns (taken in index order, rest).
fn take_items(items: Vec<Item>, idx: &[usize]) -> (Vec<Item>, Vec<Item>) {
    let set: std::collections::HashSet<usize> = idx.iter().copied().collect();
    let mut taken = Vec::new();
    let mut rest = Vec::new();
    for (i, it) in items.into_iter().enumerate() {
        if set.contains(&i) {
            taken.push((i, it));
        } else {
            rest.push(it);
        }
    }
    taken.sort_by_key(|(i, _)| *i);
    (taken.into_iter().map(|(_, it)| it).collect(), rest)
}

/// Split `taken` (in index order of `idx_sorted`) into the members listed in
/// `part`.
fn select(idx_sorted: &[usize], taken: &[Item], part: &[usize]) -> Vec<Item> {
    let set: std::collections::HashSet<usize> = part.iter().copied().collect();
    idx_sorted.iter().zip(taken.iter()).filter(|(i, _)| set.contains(i)).map(|(_, it)| it.clone()).collect()
}

fn composite(node: Node, members: &[Item], base: f32, size: f32, force_base: bool) -> Item {
    let (x0, x1, y0, y1) = union_box(members);
    let lx0 = members.iter().map(|m| m.lx0).fold(f32::MAX, f32::min).min(x0);
    let lx1 = members.iter().map(|m| m.lx1).fold(f32::MIN, f32::max).max(x1);
    Item { node, x0, x1, y0, y1, base, size, glyphs: members.iter().flat_map(|m| m.glyphs.iter().copied()).collect(), force_base, lx0, lx1, numeral: false, bracket: false }
}

fn bar_item(r: &Rule, size: f32) -> Item {
    Item { node: Node::Space, x0: r.x0, x1: r.x1, y0: r.y0, y1: r.y1, base: r.cy(), size, glyphs: vec![], force_base: false, lx0: r.x0, lx1: r.x1, numeral: false, bracket: false }
}

fn box_item(x0: f32, x1: f32, y0: f32, y1: f32, size: f32) -> Item {
    Item { node: Node::Space, x0, x1, y0, y1, base: y1, size, glyphs: vec![], force_base: false, lx0: x0, lx1: x1, numeral: false, bracket: false }
}

/// Rules that are the top/bottom edges of a box or table cell (a matching
/// rule of the same horizontal extent on the other side of the enclosed text)
/// are borders, never fraction bars or overlines.
fn mark_box_edges(pg: &PageGlyphs, items: &[Item], rule_used: &mut [bool]) {
    let n = pg.rules.len();
    for i in 0..n {
        for j in 0..n {
            if i == j {
                continue;
            }
            let (t, b) = (pg.rules[i], pg.rules[j]);
            let sep = b.cy() - t.cy();
            if !(5.0..=70.0).contains(&sep) || (t.x0 - b.x0).abs() > 1.5 || (t.x1 - b.x1).abs() > 1.5 {
                continue;
            }
            // Something must sit between them (a boxed number, a cell), and
            // a vertical side must join them: two equal fraction bars in
            // consecutive MCQ options are not a box.
            let enclosed = items.iter().any(|it| it.cy() > t.cy() && it.cy() < b.cy() && it.cx() >= t.x0 - 0.5 && it.cx() <= t.x1 + 0.5);
            let side = |x: f32| {
                pg.paths.iter().any(|p| {
                    p.x1 - p.x0 <= 1.6 && (p.x0 + p.x1) * 0.5 >= x - 1.5 && (p.x0 + p.x1) * 0.5 <= x + 1.5 && p.y0 <= t.cy() + 0.3 * sep && p.y1 >= b.cy() - 0.3 * sep
                })
            };
            if enclosed && (side(t.x0) || side(t.x1)) {
                rule_used[i] = true;
                rule_used[j] = true;
            }
        }
    }
}

/// True when a path has an oblique or curved stroke of useful length (drawing
/// evidence; rules, boxes and table grids are axis-aligned).
fn has_oblique(p: &VPath) -> bool {
    let mut prev: Option<[f32; 2]> = None;
    for (mv, q) in &p.pts {
        if let (Some(a), false) = (prev, *mv) {
            let dx = (q[0] - a[0]).abs();
            let dy = (q[1] - a[1]).abs();
            if dx > 0.6 && dy > 0.6 && dx.hypot(dy) >= 2.0 {
                return true;
            }
        }
        prev = Some(*q);
    }
    false
}

/// Diagram regions: clusters of oblique/curved vector strokes (plus the
/// axis-aligned strokes that touch them: axes, grid lines, polygon edges) and
/// raster images. Text inside a region is a figure label.
fn figure_regions(pg: &PageGlyphs, shapes: &[RadicalShape]) -> Vec<[f32; 4]> {
    let (w, h) = (pg.width, pg.height);
    let mut boxes: Vec<[f32; 4]> = Vec::new();
    for p in &pg.paths {
        let (pw, ph) = (p.x1 - p.x0, p.y1 - p.y0);
        if pw > 0.85 * w || ph > 0.85 * h {
            continue;
        }
        if pw < 6.0 && ph < 6.0 {
            continue; // glyph-sized symbol
        }
        if pw <= 8.0 && ph >= 2.5 * pw.max(0.5) {
            continue; // bracket-like tall thin stroke
        }
        if shapes.iter().any(|sh| p.x0 >= sh.x0 - 1.0 && p.x1 <= sh.x1 + 1.0 && p.y0 >= sh.y0 - 1.5 && p.y1 <= sh.y1 + 1.5) {
            continue;
        }
        if has_oblique(p) {
            boxes.push([p.x0, p.y0, p.x1, p.y1]);
        }
    }
    let near = |a: &[f32; 4], b: &[f32; 4], gap: f32| a[0] <= b[2] + gap && b[0] <= a[2] + gap && a[1] <= b[3] + gap && b[1] <= a[3] + gap;
    let merge = |v: &mut Vec<[f32; 4]>, gap: f32| loop {
        let mut changed = false;
        'outer: for i in 0..v.len() {
            for j in i + 1..v.len() {
                if near(&v[i], &v[j], gap) {
                    let b = v.remove(j);
                    let a = &mut v[i];
                    a[0] = a[0].min(b[0]);
                    a[1] = a[1].min(b[1]);
                    a[2] = a[2].max(b[2]);
                    a[3] = a[3].max(b[3]);
                    changed = true;
                    break 'outer;
                }
            }
        }
        if !changed {
            break;
        }
    };
    merge(&mut boxes, 12.0);
    // A long stroke ending in an arrowhead is an axis, however far it runs
    // (aea '24's x-axis spans the page under two small circles).
    let arrow_tipped = |p: &VPath| {
        let horizontal = p.x1 - p.x0 >= p.y1 - p.y0;
        pg.paths.iter().any(|a| {
            let (aw, ah) = (a.x1 - a.x0, a.y1 - a.y0);
            if aw > 12.0 || ah > 12.0 || aw < 1.5 && ah < 1.5 || !has_oblique(a) {
                return false;
            }
            if horizontal {
                a.y0 <= p.y1 + 1.0 && a.y1 >= p.y0 - 1.0 && ((a.x0 - p.x1).abs() <= 4.0 || (a.x1 - p.x1).abs() <= 4.0 || (a.x1 - p.x0).abs() <= 4.0 || (a.x0 - p.x0).abs() <= 4.0)
            } else {
                a.x0 <= p.x1 + 1.0 && a.x1 >= p.x0 - 1.0 && ((a.y0 - p.y1).abs() <= 4.0 || (a.y1 - p.y1).abs() <= 4.0 || (a.y1 - p.y0).abs() <= 4.0 || (a.y0 - p.y0).abs() <= 4.0)
            }
        })
    };
    // Grow with axis-aligned strokes that touch a drawing (axes, grids).
    loop {
        let mut grew = false;
        for r in boxes.iter_mut() {
            for p in &pg.paths {
                let (pw, ph) = (p.x1 - p.x0, p.y1 - p.y0);
                // A long line through the middle of the drawing (a dashed
                // line of centres across two circles) belongs to it too.
                let (pcx, pcy) = ((p.x0 + p.x1) * 0.5, (p.y0 + p.y1) * 0.5);
                let through = pw <= 0.95 * w
                    && ph <= 0.95 * h
                    && (ph <= 3.0 && pcy > r[1] + 0.15 * (r[3] - r[1]) && pcy < r[3] - 0.15 * (r[3] - r[1]) && p.x0 < r[0] + 2.0 && p.x1 > r[2] - 2.0
                        || pw <= 3.0 && pcx > r[0] + 0.15 * (r[2] - r[0]) && pcx < r[2] - 0.15 * (r[2] - r[0]) && p.y0 < r[1] + 2.0 && p.y1 > r[3] - 2.0);
                if (pw > 0.6 * w || ph > 0.6 * h) && !(pw <= 0.95 * w && ph <= 0.95 * h && arrow_tipped(p)) && !through {
                    continue;
                }
                let b = [p.x0, p.y0, p.x1, p.y1];
                if near(r, &b, 1.5) && (b[0] < r[0] - 0.5 || b[1] < r[1] - 0.5 || b[2] > r[2] + 0.5 || b[3] > r[3] + 0.5) {
                    // Only strokes that reach into the drawing, not a long
                    // answer line merely touching its corner.
                    let inside = (b[0].max(r[0]) <= b[2].min(r[2])) && (b[1].max(r[1]) <= b[3].min(r[3]));
                    if inside {
                        r[0] = r[0].min(b[0]);
                        r[1] = r[1].min(b[1]);
                        r[2] = r[2].max(b[2]);
                        r[3] = r[3].max(b[3]);
                        grew = true;
                    }
                }
            }
        }
        if !grew {
            break;
        }
    }
    // A small drawing just off a figure (a dimension line "<- 4 cm ->"
    // under it) is part of that figure.
    let big = |r: &[f32; 4]| r[2] - r[0] >= 40.0 && r[3] - r[1] >= 30.0;
    let (mut large, small): (Vec<[f32; 4]>, Vec<[f32; 4]>) = boxes.into_iter().partition(|r| big(r));
    for s in small {
        if let Some(l) = large.iter_mut().find(|l| near(l, &s, 24.0) && s[0] < l[2] && s[2] > l[0]) {
            l[0] = l[0].min(s[0]);
            l[1] = l[1].min(s[1]);
            l[2] = l[2].max(s[2]);
            l[3] = l[3].max(s[3]);
        }
    }
    let mut out: Vec<[f32; 4]> = large;
    for im in &pg.images {
        if im[2] - im[0] >= 20.0 && im[3] - im[1] >= 20.0 && (im[2] - im[0]) < 0.95 * w {
            out.push(*im);
        }
    }
    merge(&mut out, 4.0);
    // Page furniture drawn as graphics is not a figure: anything mostly off
    // the page, the narrow full-height strips of the outer margins ("DO
    // NOT WRITE IN THIS AREA" columns), and a page crossed through as not
    // for answers (a diagonal across most of the page).
    out.retain(|r| {
        let area = ((r[2] - r[0]) * (r[3] - r[1])).max(1.0);
        let on_page = (r[2].min(w) - r[0].max(0.0)).max(0.0) * (r[3].min(h) - r[1].max(0.0)).max(0.0);
        let strip = r[2] - r[0] < 0.06 * w && r[3] - r[1] > 0.5 * h && (r[2] < 0.08 * w || r[0] > 0.92 * w);
        let page_sized = area >= 0.55 * w * h;
        on_page >= 0.5 * area && !strip && !page_sized
    });
    out
}

/// A pair of long arrow-tipped axes crossing with nothing drawn between them
/// ("Sketch the graph of …" over empty axes): the box they span.
fn blank_answer_axes(pg: &PageGlyphs, regions: &[[f32; 4]]) -> Vec<[f32; 4]> {
    let line = |p: &VPath| p.pts.len() == 2 && p.stroked;
    let tipped = |p: &VPath, horizontal: bool| {
        pg.paths.iter().any(|a| {
            let (aw, ah) = (a.x1 - a.x0, a.y1 - a.y0);
            aw <= 12.0 && ah <= 12.0 && (aw >= 1.5 || ah >= 1.5) && has_oblique(a)
                && if horizontal { a.y0 <= p.y1 + 1.0 && a.y1 >= p.y0 - 1.0 && ((a.x0 - p.x1).abs() <= 4.0 || (a.x1 - p.x1).abs() <= 4.0) }
                   else { a.x0 <= p.x1 + 1.0 && a.x1 >= p.x0 - 1.0 && ((a.y0 - p.y0).abs() <= 4.0 || (a.y1 - p.y0).abs() <= 4.0) }
        })
    };
    let mut out = Vec::new();
    for hz in pg.paths.iter().filter(|p| line(p) && p.x1 - p.x0 >= 60.0 && p.y1 - p.y0 <= 2.0) {
        for vt in pg.paths.iter().filter(|p| line(p) && p.y1 - p.y0 >= 60.0 && p.x1 - p.x0 <= 2.0) {
            let crosses = vt.x0 >= hz.x0 && vt.x1 <= hz.x1 && hz.y0 >= vt.y0 && hz.y1 <= vt.y1;
            if !crosses || !tipped(hz, true) || !tipped(vt, false) {
                continue;
            }
            let b = [hz.x0, vt.y0, hz.x1, vt.y1];
            // A drawing between the axes makes them a graph, not answer space.
            let drawn = regions.iter().any(|r| r[0] < b[2] && r[2] > b[0] && r[1] < b[3] && r[3] > b[1]);
            if !drawn {
                out.push(b);
            }
        }
    }
    out
}

/// Merge vertically stacked bracket pieces (⎛⎜⎝ …) into one tall delimiter.
fn build_bracket_pieces(pg: &PageGlyphs, items: Vec<Item>) -> Vec<Item> {
    let pieces: Vec<usize> = items_in(&items, |it| matches!(it.node, Node::G(g) if piece_kind(pg.glyphs[g].ch).is_some()));
    if pieces.is_empty() {
        return items;
    }
    let mut order = pieces.clone();
    order.sort_by(|&a, &b| items[a].y0.partial_cmp(&items[b].y0).unwrap_or(std::cmp::Ordering::Equal));
    let mut used = vec![false; items.len()];
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for &p in &order {
        if used[p] {
            continue;
        }
        used[p] = true;
        let mut group = vec![p];
        let mut bottom = items[p].y1;
        let cx = items[p].cx();
        let lx = items[p].lx0;
        let kind = |q: usize| match items[q].node {
            Node::G(g) => piece_kind(pg.glyphs[g].ch).map(|k| k.0),
            _ => None,
        };
        let integral = kind(p) == Some('∫');
        let part = |q: usize| match items[q].node {
            Node::G(g) => piece_kind(pg.glyphs[g].ch).map(|k| k.1),
            _ => None,
        };
        let top = items[p].y0;
        loop {
            // Pieces are drawn from one origin; an integral's hooks curve
            // away from its stem, so its pieces' centres differ. A short
            // bracket's bottom may overlap its extension almost entirely.
            let next = order.iter().copied().find(|&q| {
                !used[q]
                    && ((items[q].cx() - cx).abs() <= 2.5 || (items[q].lx0 - lx).abs() <= 0.8)
                    && (kind(q) == Some('∫')) == integral
                    && items[q].y0 <= bottom + 2.5
                    && (items[q].y0 >= bottom - 0.6 * items[q].size || kind(q) == kind(p) && part(q) != Some(0) && items[q].y0 > top)
            });
            match next {
                Some(q) => {
                    used[q] = true;
                    bottom = bottom.max(items[q].y1);
                    group.push(q);
                }
                None => break,
            }
        }
        groups.push(group);
    }
    let mut out_extra: Vec<Item> = Vec::new();
    for group in &groups {
        let members: Vec<Item> = group.iter().map(|&q| items[q].clone()).collect();
        let (x0, x1, y0, y1) = union_box(&members);
        let glyphs: Vec<usize> = members.iter().flat_map(|m| m.glyphs.iter().copied()).collect();
        let size = members.iter().map(|m| m.size).fold(0.0, f32::max);
        out_extra.push(Item { node: Node::G(glyphs[0]), x0, x1, y0, y1, base: (y0 + y1) * 0.5 + 0.3 * size, size, glyphs, force_base: true, lx0: x0, lx1: x1, numeral: false, bracket: false });
    }
    let mut out: Vec<Item> = items.into_iter().enumerate().filter(|(i, _)| !used[*i]).map(|(_, it)| it).collect();
    out.extend(out_extra);
    out
}

/// Tall delimiter character represented by an item (assembled pieces or a
/// single enlarged glyph), if any.
fn delimiter_of(pg: &PageGlyphs, it: &Item, ref_size: f32) -> Option<char> {
    if !matches!(it.node, Node::G(_)) {
        return None;
    }
    if it.glyphs.iter().any(|&g| piece_kind(pg.glyphs[g].ch).is_some_and(|k| k.0 == '∫')) {
        return None;
    }
    if it.glyphs.len() > 1 {
        for &g in &it.glyphs {
            if let Some((k, part)) = piece_kind(pg.glyphs[g].ch) {
                if part != 1 || k != '|' {
                    return Some(k);
                }
            }
        }
        return Some('|');
    }
    let Node::G(g) = it.node else { return None };
    let gl = &pg.glyphs[g];
    if let Some((k, _)) = piece_kind(gl.ch) {
        return Some(k);
    }
    let tall = gl.h() > 1.35 * ref_size.max(1.0) || gl.size > 1.3 * ref_size;
    if tall && matches!(gl.ch, '(' | ')' | '[' | ']' | '{' | '}' | '|' | '‖' | '〈' | '〉' | '⟨' | '⟩') {
        return Some(gl.ch);
    }
    None
}

/// A radical sign drawn as vector paths (no glyph in the text layer): the
/// check-mark stroke plus an overbar running right from its top.
#[derive(Debug, Clone, Copy)]
struct RadicalShape {
    /// Left edge of the sign (tick start).
    x0: f32,
    /// Where the overbar starts (top of the long up-stroke).
    xa: f32,
    /// Right end of the overbar.
    x1: f32,
    /// Overbar y (top).
    y0: f32,
    /// Lowest point of the sign.
    y1: f32,
    /// x of the lowest point.
    xb: f32,
}

/// Polylines of a page: each path's sub-paths, plus 2-point stroked segments
/// chained end-to-end (some producers draw the sign as separate strokes).
fn page_polylines(pg: &PageGlyphs) -> Vec<Vec<[f32; 2]>> {
    let mut lines: Vec<Vec<[f32; 2]>> = Vec::new();
    for p in &pg.paths {
        let mut cur: Vec<[f32; 2]> = Vec::new();
        for (mv, q) in &p.pts {
            if *mv && !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            cur.push(*q);
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
    }
    // Chain short segments whose endpoints meet.
    let close = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() <= 0.6 && (a[1] - b[1]).abs() <= 0.6;
    let mut segs: Vec<Vec<[f32; 2]>> = lines.iter().filter(|l| l.len() == 2).cloned().collect();
    let mut chains: Vec<Vec<[f32; 2]>> = Vec::new();
    while let Some(mut chain) = segs.pop() {
        loop {
            let last = *chain.last().unwrap();
            let first = chain[0];
            if let Some(pos) = segs.iter().position(|sg| close(sg[0], last) || close(sg[1], last)) {
                let sg = segs.remove(pos);
                chain.push(if close(sg[0], last) { sg[1] } else { sg[0] });
                continue;
            }
            if let Some(pos) = segs.iter().position(|sg| close(sg[0], first) || close(sg[1], first)) {
                let sg = segs.remove(pos);
                chain.insert(0, if close(sg[0], first) { sg[1] } else { sg[0] });
                continue;
            }
            break;
        }
        if chain.len() >= 3 {
            chains.push(chain);
        }
    }
    lines.retain(|l| l.len() >= 3);
    lines.extend(chains);
    lines
}

fn radical_shapes(pg: &PageGlyphs) -> Vec<RadicalShape> {
    let mut out: Vec<RadicalShape> = Vec::new();
    for pl in page_polylines(pg) {
        let x0 = pl.iter().map(|q| q[0]).fold(f32::MAX, f32::min);
        let x1 = pl.iter().map(|q| q[0]).fold(f32::MIN, f32::max);
        let y0 = pl.iter().map(|q| q[1]).fold(f32::MAX, f32::min);
        let y1 = pl.iter().map(|q| q[1]).fold(f32::MIN, f32::max);
        let (w, h) = (x1 - x0, y1 - y0);
        if !(4.0..=90.0).contains(&h) || w < 0.4 * h {
            continue;
        }
        // The sign's tick and strokes are oblique; chained table borders are
        // all horizontal or vertical.
        // (A joint between two borders, offset by half a point, is not a
        // slant: it must lean at least ~7° from both axes.)
        let oblique = pl.windows(2).filter(|s| {
            let (dx, dy) = ((s[1][0] - s[0][0]).abs(), (s[1][1] - s[0][1]).abs());
            let len = dx.hypot(dy);
            len >= 1.0 && dx >= 0.12 * len && dy >= 0.12 * len
        }).count();
        if oblique < 2 {
            continue;
        }
        let top: Vec<f32> = pl.iter().filter(|q| q[1] <= y0 + 0.9).map(|q| q[0]).collect();
        if top.is_empty() {
            continue;
        }
        let top_left = top.iter().copied().fold(f32::MAX, f32::min);
        let top_right = top.iter().copied().fold(f32::MIN, f32::max);
        if top_right < x1 - 0.8 || top_right - top_left < 1.5 {
            continue;
        }
        let bottom = pl.iter().copied().max_by(|a, b| a[1].partial_cmp(&b[1]).unwrap_or(std::cmp::Ordering::Equal)).unwrap();
        let xb = bottom[0];
        if xb > top_left + 1.2 || xb < x0 + 0.3 {
            continue;
        }
        // The small tick left of the lowest point, mid-height.
        let tick = pl.iter().any(|q| q[0] <= xb - 0.8 && q[1] >= y0 + 0.25 * h && q[1] <= y1 - 0.1 * h);
        if !tick {
            continue;
        }
        let shape = RadicalShape { x0, xa: top_left, x1: top_right, y0, y1, xb };
        // Producers often draw the sign twice (fill + stroke): dedupe.
        if out.iter().any(|o| (o.x0 - shape.x0).abs() < 1.5 && (o.x1 - shape.x1).abs() < 1.5 && (o.y0 - shape.y0).abs() < 1.5) {
            continue;
        }
        out.push(shape);
    }
    out
}

/// A script-size item set flush after another script-size item on its level
/// continues that script run (the "x" of e^{x/9} before a radical): an index
/// stands alone in the radical's crook.
fn continues_script_run(items: &[Item], i: usize) -> bool {
    let o = &items[i];
    items.iter().enumerate().any(|(k, q)| {
        k != i && q.x1 <= o.x0 + 0.5 && o.x0 - q.x1 <= 0.35 * o.size.max(1.0) && q.y0 < o.y1 && q.y1 > o.y0 && q.size <= 1.15 * o.size
    })
}

fn build_vector_radicals(pg: &PageGlyphs, mut items: Vec<Item>, rule_used: &mut [bool], shapes: &[RadicalShape], shape_used: &mut [bool]) -> Vec<Item> {
    loop {
        // Innermost unused shape: no other unused shape inside its radicand.
        let pick = (0..shapes.len()).find(|&k| {
            !shape_used[k] && {
                let a = shapes[k];
                !(0..shapes.len()).any(|j| j != k && !shape_used[j] && shapes[j].x0 >= a.xa - 0.5 && shapes[j].x1 <= a.x1 + 0.5 && shapes[j].y0 > a.y0 + 0.5)
                    && !pg.rules.iter().enumerate().any(|(ri, r)| {
                        !rule_used[ri] && r.x0 >= a.xa - 0.5 && r.x1 <= a.x1 + 0.5 && r.cy() > a.y0 + 1.5 && r.cy() < a.y1 - 0.5
                    })
            }
        });
        let Some(k) = pick else { return items };
        shape_used[k] = true;
        let sh = shapes[k];
        let h = sh.y1 - sh.y0;
        let radicand_idx = items_in(&items, |o| o.cx() > sh.xa - 0.5 && o.cx() < sh.x1 + 0.5 && o.y0 >= sh.y0 - 1.0 && o.y1 <= sh.y1 + 0.35 * o.size.max(1.0));
        if radicand_idx.is_empty() {
            continue;
        }
        let s_rad = radicand_idx.iter().map(|&i| items[i].size).fold(0.0, f32::max).max(1.0);
        let index_idx: Vec<usize> = items_in(&items, |o| {
            // The index sits in the crook of the sign: its foot between the
            // overbar and mid-height, never a line above the radical.
            o.size < 0.85 * s_rad
                && o.cx() >= sh.x0 - 0.6 * s_rad
                && o.cx() <= sh.xb
                && o.y1 <= sh.y0 + 0.6 * h
                && o.y1 >= sh.y0 - 0.2 * s_rad
                && o.y0 >= sh.y0 - 1.0 * s_rad
        })
        .into_iter()
        .filter(|i| !radicand_idx.contains(i) && !continues_script_run(&items, *i))
        .collect();
        // Overbar strokes are part of the sign, never fraction bars/accents.
        for (ri, r) in pg.rules.iter().enumerate() {
            if (r.cy() - sh.y0).abs() <= 1.2 && r.x0 >= sh.x0 - 1.5 && r.x1 <= sh.x1 + 1.5 {
                rule_used[ri] = true;
            }
        }
        let mut all = radicand_idx.clone();
        all.extend(index_idx.iter().copied());
        all.sort();
        all.dedup();
        let (taken, rest) = take_items(items, &all);
        let radicand = select(&all, &taken, &radicand_idx);
        let index = select(&all, &taken, &index_idx);
        let (base, size) = main_baseline(&radicand);
        let body = parse_box(pg, build_delimited(pg, radicand)).0;
        let idx_nodes = if index.is_empty() { None } else { Some(parse_box(pg, index).0) };
        let mut members = taken;
        members.push(box_item(sh.x0, sh.x1, sh.y0, sh.y1, size));
        items = rest;
        items.push(composite(Node::Sqrt(idx_nodes, body), &members, base, size, false));
    }
}

/// Radical sign followed by an overbar (rule or extender glyphs): the
/// radicand is everything under the bar.
fn build_radicals(pg: &PageGlyphs, mut items: Vec<Item>, rule_used: &mut [bool]) -> Vec<Item> {
    loop {
        // (sqrt item, rule, bar right end or NaN for a bare sign)
        let mut found: Option<(usize, Option<usize>, f32)> = None;
        for (si, it) in items.iter().enumerate() {
            let Node::G(g) = it.node else { continue };
            if pg.glyphs[g].ch != '√' || it.glyphs.len() != 1 {
                continue;
            }
            let top = it.y0;
            let rule = pg.rules.iter().enumerate().find(|(ri, r)| {
                !rule_used[*ri] && r.x0 >= it.x0 + 0.2 * (it.x1 - it.x0) - 1.0 && r.x0 <= it.x1 + 1.5 && (r.cy() - top).abs() <= 2.0 && r.w() >= 2.0
            });
            if let Some((ri, r)) = rule {
                // Innermost: no unused bar under this one.
                let inner_bar = pg.rules.iter().enumerate().any(|(rj, q)| {
                    rj != ri && !rule_used[rj] && q.x0 >= r.x0 - 0.5 && q.x1 <= r.x1 + 0.5 && q.y0 > r.y1 && q.y1 < it.y1
                });
                if inner_bar {
                    continue;
                }
                found = Some((si, Some(ri), r.x1));
                break;
            }
            let ext_right = items
                .iter()
                .filter(|o| matches!(o.node, Node::G(h) if pg.glyphs[h].ch == '\u{203E}') && (o.y0 - top).abs() <= 2.5 && o.x0 >= it.x1 - 1.5)
                .map(|e| e.x1)
                .fold(f32::MIN, f32::max);
            if ext_right > f32::MIN {
                found = Some((si, None, ext_right));
                break;
            }
            found = Some((si, None, f32::NAN));
            break;
        }
        let Some((si, rule, right)) = found else { return items };
        let sq = items[si].clone();
        let size = sq.size;
        let radicand_idx: Vec<usize> = if right.is_nan() {
            let mut ordered = items_in(&items, |o| {
                o.x0 >= sq.x1 - 1.0 && (o.base - sq.base).abs() < 0.35 * size.max(o.size) && o.x0 - sq.x1 < 0.6 * size && o.glyphs != sq.glyphs
            });
            // A sign printed without a bar before a bracket ("√(125)") roots
            // the whole bracketed group: its members run past the sign.
            let opens_group = |k: usize| matches!(items[k].node, Node::G(h) if matches!(pg.glyphs[h].ch, '(' | '['));
            ordered.sort_by(|&a, &b| items[a].x0.partial_cmp(&items[b].x0).unwrap_or(std::cmp::Ordering::Equal));
            if ordered.first().is_some_and(|&k| opens_group(k)) {
                ordered = items_in(&items, |o| o.x0 >= sq.x1 - 1.0 && (o.base - sq.base).abs() < 0.35 * size.max(o.size) && o.glyphs != sq.glyphs);
                ordered.sort_by(|&a, &b| items[a].x0.partial_cmp(&items[b].x0).unwrap_or(std::cmp::Ordering::Equal));
            }
            let mut take = Vec::new();
            let mut last = sq.x1;
            let mut depth = 0i32;
            for (n, k) in ordered.into_iter().enumerate() {
                let o = &items[k];
                if o.x0 - last > 0.25 * size {
                    break;
                }
                let ch = match o.node {
                    Node::G(h) => Some(pg.glyphs[h].ch),
                    _ => None,
                };
                if n == 0 && opens_group(k) || depth > 0 {
                    match ch {
                        Some('(' | '[') => depth += 1,
                        Some(')' | ']') => depth -= 1,
                        _ => {}
                    }
                    last = o.x1;
                    take.push(k);
                    if depth == 0 {
                        break;
                    }
                    continue;
                }
                let alnum = ch.map(|c| c.is_alphanumeric() || c == '.').unwrap_or(true);
                if !alnum {
                    break;
                }
                last = o.x1;
                take.push(k);
            }
            // (An unclosed group is not a radicand.)
            if depth != 0 {
                take.clear();
            }
            take
        } else {
            let bar_y = match rule {
                Some(ri) => pg.rules[ri].y1,
                None => sq.y0,
            };
            items_in(&items, |o| {
                o.cx() > sq.x1 - 0.5
                    && o.cx() < right + 0.5
                    && o.y0 >= bar_y - 1.0
                    && o.y1 <= sq.y1 + 0.35 * size
                    && o.glyphs != sq.glyphs
                    && !matches!(o.node, Node::G(h) if pg.glyphs[h].ch == '\u{203E}')
            })
        };
        let ext_idx: Vec<usize> = items_in(&items, |o| {
            matches!(o.node, Node::G(h) if pg.glyphs[h].ch == '\u{203E}') && (o.y0 - sq.y0).abs() <= 2.5 && o.x0 >= sq.x1 - 1.5 && (right.is_nan() || o.x1 <= right + 0.5)
        });
        let index_idx: Vec<usize> = items_in(&items, |o| {
            o.size < 0.85 * size
                && o.cx() >= sq.x0 - 0.6 * size
                && o.cx() <= sq.x0 + 0.55 * (sq.x1 - sq.x0)
                && o.y1 <= sq.cy() + 0.5
                && o.y1 >= sq.y0 - 0.2 * size
                && o.y0 >= sq.y0 - 1.0 * size
                && o.glyphs != sq.glyphs
        })
        .into_iter()
        .filter(|k| !radicand_idx.contains(k) && !continues_script_run(&items, *k))
        .collect();
        let mut all: Vec<usize> = vec![si];
        all.extend(radicand_idx.iter().copied());
        all.extend(ext_idx.iter().copied());
        all.extend(index_idx.iter().copied());
        all.sort();
        all.dedup();
        let (taken, rest) = take_items(items, &all);
        let radicand = select(&all, &taken, &radicand_idx);
        let index = select(&all, &taken, &index_idx);
        let mut members = taken;
        if let Some(ri) = rule {
            rule_used[ri] = true;
            members.push(bar_item(&pg.rules[ri], size));
        }
        let base = if radicand.is_empty() { sq.base } else { main_baseline(&radicand).0 };
        let body = parse_box(pg, build_delimited(pg, radicand)).0;
        let idx_nodes = if index.is_empty() { None } else { Some(parse_box(pg, index).0) };
        items = rest;
        items.push(composite(Node::Sqrt(idx_nodes, body), &members, base, size, false));
    }
}

/// Fraction bars: a thin rule with a compact group directly above AND below.
fn build_fractions(pg: &PageGlyphs, mut items: Vec<Item>, rule_used: &mut [bool]) -> Vec<Item> {
    loop {
        let mut made = false;
        for ri in 0..pg.rules.len() {
            if rule_used[ri] {
                continue;
            }
            let r = pg.rules[ri];
            if r.w() > 0.6 * pg.width {
                continue;
            }
            let span = |o: &Item| o.cx() >= r.x0 - 1.0 && o.cx() <= r.x1 + 1.0;
            let near: Vec<f32> = items
                .iter()
                .filter(|o| span(o) && ((o.y1 - r.y0).abs() < 14.0 || (o.y0 - r.y1).abs() < 14.0))
                .map(|o| o.size)
                .collect();
            if near.is_empty() {
                continue;
            }
            let s = median(near).max(4.0);
            if r.w() > 25.0 * s {
                continue;
            }
            let above = items_in(&items, |o| span(o) && o.y1 <= r.y0 + 1.0 && o.y1 >= r.y0 - 1.15 * s);
            let below = items_in(&items, |o| span(o) && o.y0 >= r.y1 - 1.0 && o.y0 <= r.y1 + 1.15 * s);
            // A tall part (brackets, a radical) lifts some of its glyphs out
            // of the band next to the bar: a superscript inside the part's
            // own extent still belongs to it.
            let grow = |mut set: Vec<usize>, upper: bool| -> Vec<usize> {
                loop {
                    let ext = set.iter().fold((f32::MAX, f32::MIN), |e, &k| (e.0.min(items[k].y0), e.1.max(items[k].y1)));
                    let more: Vec<usize> = items_in(&items, |o| {
                        span(o)
                            && (if upper { o.y1 <= r.y0 + 1.0 } else { o.y0 >= r.y1 - 1.0 })
                            && o.y1 >= ext.0 - 0.5
                            && o.y0 <= ext.1 + 0.5
                    })
                    .into_iter()
                    .filter(|k| !set.contains(k))
                    .collect();
                    if more.is_empty() {
                        return set;
                    }
                    set.extend(more);
                }
            };
            let above = if above.is_empty() { above } else { grow(above, true) };
            let below = if below.is_empty() { below } else { grow(below, false) };
            if above.is_empty() || below.is_empty() {
                continue;
            }
            // A fraction's numerator stands clear of its bar and its
            // denominator hugs it; text resting on a rule with the next line
            // of text below is underlined text ("Zoo(ZooName, …)").
            // (The numerator's own baseline, not a subscript's: r₁ − r₂.)
            let num_items: Vec<Item> = above.iter().map(|&k| items[k].clone()).collect();
            let num_base = main_baseline(&num_items).0;
            let den_top = below.iter().map(|&k| items[k].y0).fold(f32::MAX, f32::min);
            let resting = r.cy() - num_base < 0.15 * s;
            let next_line_below = den_top - r.y1 > 0.45 * s;
            if resting && next_line_below || den_top - r.y1 > 0.7 * s {
                continue;
            }
            // A numerator stands clear of other text on its baseline: letters
            // touching it there make it part of a word on the line above
            // (the "c" of "decays" over the bar of an antineutrino's v̄).
            if inside_word(pg, &items, &above) {
                continue;
            }
            let a_items: Vec<Item> = above.iter().map(|&k| items[k].clone()).collect();
            let b_items: Vec<Item> = below.iter().map(|&k| items[k].clone()).collect();
            let a_box = union_box(&a_items);
            let b_box = union_box(&b_items);
            // Innermost: no other unused bar inside either part's region.
            let inner = pg.rules.iter().enumerate().any(|(rj, q)| {
                rj != ri
                    && !rule_used[rj]
                    && q.w() >= 2.0
                    && q.x0 >= r.x0 - 1.0
                    && q.x1 <= r.x1 + 1.0
                    && ((q.cy() > a_box.2 && q.cy() < r.y0) || (q.cy() > r.y1 && q.cy() < b_box.3))
            });
            if inner {
                continue;
            }
            // A real fraction's parts fill the bar and have no column-sized
            // internal gaps (table rules fail this).
            let fill = (a_box.1 - a_box.0).max(b_box.1 - b_box.0);
            if fill < 0.4 * r.w() {
                continue;
            }
            let compact = |v: &[Item]| {
                let mut xs: Vec<(f32, f32)> = v.iter().map(|o| (o.x0, o.x1)).collect();
                xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                let mut right = f32::MIN;
                for (x0, x1) in xs {
                    if right > f32::MIN && x0 - right > 2.0 * s {
                        return false;
                    }
                    right = right.max(x1);
                }
                true
            };
            if !compact(&a_items) || !compact(&b_items) {
                continue;
            }
            let mut all: Vec<usize> = above.iter().chain(below.iter()).copied().collect();
            all.sort();
            all.dedup();
            let (taken, rest) = take_items(items, &all);
            let num = select(&all, &taken, &above);
            let den = select(&all, &taken, &below);
            rule_used[ri] = true;
            let fsize = num.iter().chain(den.iter()).map(|o| o.size).fold(0.0, f32::max);
            // Brackets inside a part ("[ln(x+1) - 2 ln x]" over x^4) pair
            // within that part: the page-level pass runs after fractions.
            let node = Node::Frac(parse_box(pg, build_delimited(pg, num)).0, parse_box(pg, build_delimited(pg, den)).0);
            let mut members = taken;
            members.push(bar_item(&r, fsize));
            // The math axis sits ~0.27 em above the baseline.
            let base = r.cy() + 0.27 * fsize;
            items = rest;
            items.push(composite(node, &members, base, fsize, false));
            made = true;
            break;
        }
        if !made {
            return items;
        }
    }
}

/// Accent glyphs/rules directly above a glyph group: bars, hats, dots, arrows.
fn build_accents(pg: &PageGlyphs, mut items: Vec<Item>, rule_used: &mut [bool]) -> Vec<Item> {
    // A vector arrow typed as dashes ending in an arrowhead ("−−→") over the
    // letters it names (OA with an arrow above) is an over-arrow accent.
    loop {
        let is = |it: &Item, f: &dyn Fn(char) -> bool| matches!(it.node, Node::G(g) if f(pg.glyphs[g].ch));
        let mut found: Option<(Vec<usize>, Vec<usize>)> = None;
        for (ai, a) in items.iter().enumerate() {
            if !is(a, &|c| c == '→') {
                continue;
            }
            // The dashes leading into the arrowhead, touching it on its line.
            let mut shaft = vec![ai];
            let mut left = a.x0;
            loop {
                let next = items.iter().enumerate().find(|(k, o)| {
                    !shaft.contains(k) && is(o, &|c| matches!(c, '−' | '-' | '–')) && (o.base - a.base).abs() < 0.5 && o.x1 >= left - 0.8 && o.x0 < a.x0
                });
                match next {
                    Some((k, o)) => {
                        left = left.min(o.x0);
                        shaft.push(k);
                    }
                    None => break,
                }
            }
            if shaft.len() < 2 {
                continue;
            }
            let (x0, x1, y1) = (left, a.x1, shaft.iter().map(|&k| items[k].y1).fold(f32::MIN, f32::max));
            let body: Vec<usize> = items_in(&items, |b| b.cx() >= x0 - 1.5 && b.cx() <= x1 + 1.5 && b.y0 >= y1 - 1.5 && b.y0 <= y1 + 0.45 * b.size && !shaft.iter().any(|&k| items[k].glyphs == b.glyphs));
            if !body.is_empty() {
                found = Some((shaft, body));
                break;
            }
        }
        let Some((shaft, body_idx)) = found else { break };
        let mut all: Vec<usize> = shaft.iter().chain(body_idx.iter()).copied().collect();
        all.sort();
        all.dedup();
        let (taken, rest) = take_items(items, &all);
        let body = select(&all, &taken, &body_idx);
        let (base, size) = main_baseline(&body);
        items = rest;
        items.push(composite(Node::Accent(AccentKind::OverArrow, parse_box(pg, body).0), &taken, base, size, false));
    }
    loop {
        let mut hit: Option<(usize, AccentKind)> = None;
        for (ai, a) in items.iter().enumerate() {
            let Node::G(g) = a.node else { continue };
            if a.glyphs.len() != 1 {
                continue;
            }
            let kind = match pg.glyphs[g].ch {
                '¯' | 'ˉ' => AccentKind::Bar,
                'ˆ' | '\u{0302}' => AccentKind::Hat,
                '˜' | '\u{0303}' => AccentKind::Tilde,
                '˙' | '\u{0307}' => AccentKind::Dot,
                // A recurring-decimal dot set as a tiny bullet over a digit
                // (checked below: raised over its base, never on a line).
                '•' | '∙' if a.y1 - a.y0 <= 0.3 * a.size.max(4.0) || a.y1 - a.y0 <= 2.5 => AccentKind::Dot,
                '¨' | '\u{0308}' => AccentKind::DDot,
                '→' | '⃗' => AccentKind::Vec,
                '^' | '~' => {
                    if a.y1 - a.y0 > 0.35 * a.size {
                        continue;
                    }
                    if pg.glyphs[g].ch == '^' { AccentKind::Hat } else { AccentKind::Tilde }
                }
                _ => continue,
            };
            let has_base = items.iter().enumerate().any(|(bi, b)| {
                bi != ai && b.cx() >= a.x0 - 1.5 && b.cx() <= a.x1 + 1.5 && b.y0 >= a.y1 - 1.5 && b.y0 <= a.y1 + 0.45 * b.size
            });
            // An arrow (or a list bullet) on its own baseline is an
            // operator (or punctuation), not an accent.
            let on_line = items.iter().enumerate().any(|(bi, b)| bi != ai && (b.base - a.base).abs() < 0.2 * b.size && hgap(a, b) < 0.8 * b.size && b.size >= a.size * 0.9);
            let bullet = matches!(pg.glyphs[g].ch, '•' | '∙');
            if has_base && !((kind == AccentKind::Vec || bullet) && on_line) {
                hit = Some((ai, kind));
                break;
            }
        }
        let Some((ai, kind)) = hit else { break };
        let acc = items[ai].clone();
        let body_idx = items_in(&items, |b| {
            b.cx() >= acc.x0 - 1.5 && b.cx() <= acc.x1 + 1.5 && b.y0 >= acc.y1 - 1.5 && b.y0 <= acc.y1 + 0.5 * b.size && b.glyphs != acc.glyphs
        });
        let mut all = body_idx.clone();
        all.push(ai);
        all.sort();
        all.dedup();
        let (taken, rest) = take_items(items, &all);
        let body = select(&all, &taken, &body_idx);
        let kind = match kind {
            AccentKind::Vec if body.len() > 1 => AccentKind::OverArrow,
            AccentKind::Bar if body.len() > 1 => AccentKind::Overline,
            k => k,
        };
        let (base, size) = main_baseline(&body);
        items = rest;
        items.push(composite(Node::Accent(kind, parse_box(pg, body).0), &taken, base, size, false));
    }
    // Rules hugging a glyph group from above, with nothing directly above.
    // Innermost (lowest) first, so a stretched overline over already-barred
    // letters (Boolean negation of a product) wraps the inner accents.
    let mut order: Vec<usize> = (0..pg.rules.len()).collect();
    order.sort_by(|&a, &b| pg.rules[b].cy().partial_cmp(&pg.rules[a].cy()).unwrap_or(std::cmp::Ordering::Equal));
    for ri in order {
        if rule_used[ri] {
            continue;
        }
        let r = pg.rules[ri];
        let span = |o: &Item| o.cx() >= r.x0 - 0.5 && o.cx() <= r.x1 + 0.5;
        let touching = items_in(&items, |o| span(o) && o.y0 >= r.y1 - 0.8 && o.y0 <= r.y1 + 0.45 * o.size && o.x0 >= r.x0 - 2.0 && o.x1 <= r.x1 + 2.0);
        let above = items_in(&items, |o| span(o) && o.y1 <= r.y0 + 0.8 && o.y1 >= r.y0 - 0.6 * o.size);
        // (Letters of a word on the line above do not count.)
        if touching.is_empty() || !above.is_empty() && !inside_word(pg, &items, &above) {
            continue;
        }
        // Everything under the bar within the touching items' band belongs
        // to the decorated group (an operator between two barred letters).
        let band_top = touching.iter().map(|&k| items[k].y0).fold(f32::MAX, f32::min);
        let band_bot = touching.iter().map(|&k| items[k].y1).fold(f32::MIN, f32::max);
        let below = items_in(&items, |o| span(o) && o.cy() >= band_top - 0.5 && o.cy() <= band_bot + 0.5 && o.x0 >= r.x0 - 2.0 && o.x1 <= r.x1 + 2.0);
        let parts: Vec<Item> = below.iter().map(|&k| items[k].clone()).collect();
        let (bx0, bx1, _, _) = union_box(&parts);
        let s = parts.iter().map(|p| p.size).fold(0.0, f32::max);
        if r.w() > (bx1 - bx0) + 1.2 * s || r.w() < 0.5 * (bx1 - bx0) || r.w() > 8.0 * s {
            continue;
        }
        rule_used[ri] = true;
        let (taken, rest) = take_items(items, &below);
        let kind = if taken.len() == 1 && taken[0].glyphs.len() == 1 { AccentKind::Bar } else { AccentKind::Overline };
        let (base, size) = main_baseline(&taken);
        let node = Node::Accent(kind, parse_box(pg, taken.clone()).0);
        items = rest;
        let mut members = taken;
        members.push(bar_item(&r, size));
        items.push(composite(node, &members, base, size, false));
    }
    // Rules a word rests on, with nothing directly below: underlined text.
    for ri in 0..pg.rules.len() {
        if rule_used[ri] {
            continue;
        }
        let r = pg.rules[ri];
        let span = |o: &Item| o.cx() >= r.x0 - 0.5 && o.cx() <= r.x1 + 0.5;
        let resting = items_in(&items, |o| {
            span(o) && o.base <= r.cy() + 0.05 * o.size && o.base >= r.cy() - 0.3 * o.size && o.x0 >= r.x0 - 2.0 && o.x1 <= r.x1 + 2.0
        });
        if resting.is_empty() {
            continue;
        }
        let parts: Vec<Item> = resting.iter().map(|&k| items[k].clone()).collect();
        let (bx0, bx1, _, _) = union_box(&parts);
        let s = parts.iter().map(|p| p.size).fold(0.0, f32::max).max(4.0);
        let below = items_in(&items, |o| span(o) && o.y0 >= r.y1 - 0.5 && o.y0 <= r.y1 + 0.35 * s);
        if !below.is_empty() || r.w() > (bx1 - bx0) + 1.0 * s || r.w() < 0.8 * (bx1 - bx0) {
            continue;
        }
        rule_used[ri] = true;
        let (taken, rest) = take_items(items, &resting);
        let (base, size) = main_baseline(&taken);
        let node = Node::Accent(AccentKind::Underline, parse_box(pg, taken.clone()).0);
        items = rest;
        let mut members = taken;
        members.push(bar_item(&r, size));
        items.push(composite(node, &members, base, size, false));
    }
    items
}

/// Pair tall delimiters and wrap their contents (groups, matrices, cases).
fn build_delimited(pg: &PageGlyphs, mut items: Vec<Item>) -> Vec<Item> {
    let ref_size = median(items.iter().filter(|i| i.glyphs.len() == 1).map(|i| i.size).collect());
    let mut guard = 0;
    loop {
        guard += 1;
        if guard > 200 {
            return items;
        }
        let delims: Vec<(usize, char)> = items.iter().enumerate().filter_map(|(i, it)| delimiter_of(pg, it, ref_size).map(|c| (i, c))).collect();
        if delims.is_empty() {
            return items;
        }
        let mut lefts: Vec<(usize, char)> = delims.iter().copied().filter(|(_, c)| is_open_delim(*c) || *c == '|').collect();
        // Innermost first: rightmost left delimiter.
        lefts.sort_by(|a, b| items[b.0].x0.partial_cmp(&items[a.0].x0).unwrap_or(std::cmp::Ordering::Equal));
        let mut pair: Option<(usize, Option<usize>, char, char)> = None;
        for &(li, lc) in &lefts {
            let l = &items[li];
            let lh = l.y1 - l.y0;
            let partner = delims
                .iter()
                .filter(|(ri, rc)| {
                    let r = &items[*ri];
                    *ri != li && r.x0 >= l.x1 - 0.5 && (!is_open_delim(*rc)) && {
                        let ov = (l.y1.min(r.y1) - l.y0.max(r.y0)).max(0.0);
                        ov >= 0.6 * lh.min(r.y1 - r.y0)
                    }
                })
                .min_by(|a, b| items[a.0].x0.partial_cmp(&items[b.0].x0).unwrap_or(std::cmp::Ordering::Equal))
                .copied();
            match partner {
                Some((ri, rc)) => {
                    pair = Some((li, Some(ri), lc, rc));
                    break;
                }
                None if lc == '{' => {
                    pair = Some((li, None, lc, '.'));
                    break;
                }
                None => continue,
            }
        }
        let Some((li, ri, lc, rc)) = pair else {
            for (i, _) in &delims {
                items[*i].force_base = true;
            }
            return items;
        };
        let l = items[li].clone();
        let right_x = ri.map(|r| items[r].x0).unwrap_or(f32::MAX);
        let (top, bottom) = match ri {
            Some(r) => (l.y0.min(items[r].y0), l.y1.max(items[r].y1)),
            None => (l.y0, l.y1),
        };
        let content_idx: Vec<usize> = items_in(&items, |o| o.cx() > l.x1 - 0.5 && o.cx() < right_x + 0.5 && o.cy() >= top - 1.0 && o.cy() <= bottom + 1.0)
            .into_iter()
            .filter(|&k| k != li && Some(k) != ri)
            .collect();
        let mut all = content_idx.clone();
        all.push(li);
        if let Some(r) = ri {
            all.push(r);
        }
        all.sort();
        all.dedup();
        let (taken, rest) = take_items(items, &all);
        let content = select(&all, &taken, &content_idx);
        let size = content.iter().map(|c| c.size).fold(0.0, f32::max).max(ref_size.min(l.size));
        // A bracketed nuclide (13 over 6 before C) is one row of symbols with
        // full-size numbers stacked within half an em of it, not a matrix.
        let stacked_symbol = {
            let smax = content.iter().map(|c| c.size).fold(0.0, f32::max).max(1.0);
            let letters: Vec<&Item> = content.iter().filter(|c| !c.numeral && c.size >= 0.8 * smax).collect();
            !letters.is_empty()
                && letters.iter().all(|c| (c.base - letters[0].base).abs() <= 0.15 * smax)
                && content.iter().filter(|c| c.numeral).all(|c| (c.base - letters[0].base).abs() <= 0.6 * smax)
        };
        let rows = if stacked_symbol { vec![content.clone()] } else { split_rows(&content) };
        let multi = rows.len() >= 2;
        let node = if multi && lc == '{' && rc == '.' {
            Node::Cases(cases_rows(pg, rows))
        } else if multi {
            Node::Matrix(lc, rc, split_matrix(pg, rows))
        } else {
            Node::Delim(lc, rc, parse_box(pg, content.clone()).0)
        };
        let base = if content.is_empty() || multi { (top + bottom) * 0.5 + 0.28 * size } else { main_baseline(&content).0 };
        items = rest;
        items.push(composite(node, &taken, base, size, true));
    }
}

/// Split a content box into rows of ordinary-size items by baseline.
fn split_rows(items: &[Item]) -> Vec<Vec<Item>> {
    if items.is_empty() {
        return Vec::new();
    }
    let smax = items.iter().map(|i| i.size).fold(0.0, f32::max);
    let mut main: Vec<&Item> = items.iter().filter(|i| i.size >= 0.8 * smax).collect();
    main.sort_by(|a, b| a.base.partial_cmp(&b.base).unwrap_or(std::cmp::Ordering::Equal));
    let mut row_bases: Vec<f32> = Vec::new();
    for it in &main {
        match row_bases.last() {
            Some(&b) if (it.base - b).abs() <= 0.45 * smax => {}
            _ => row_bases.push(it.base),
        }
    }
    if row_bases.len() < 2 || !row_bases.windows(2).all(|w| w[1] - w[0] >= 0.8 * smax) {
        return vec![items.to_vec()];
    }
    let mut rows: Vec<Vec<Item>> = vec![Vec::new(); row_bases.len()];
    for it in items {
        let (k, _) = row_bases
            .iter()
            .enumerate()
            .map(|(k, b)| (k, (it.base - b).abs()))
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap();
        rows[k].push(it.clone());
    }
    rows.into_iter().filter(|r| !r.is_empty()).collect()
}

/// Columns for a matrix: gaps in the union x-projection of all rows.
/// Rows of a cases block. The conditions form a column starting at one x in
/// every row ("3x − 2   for −1 ⩽ x ⩽ 1" over "4/(5 − x)   for 1 < x ⩽ 4"):
/// each row is divided there by a gap node, however close its value comes.
fn cases_rows(pg: &PageGlyphs, rows: Vec<Vec<Item>>) -> Vec<Vec<Node>> {
    let smax = rows.iter().flatten().map(|i| i.size).fold(0.0, f32::max).max(1.0);
    // Where each row's clear gaps (wider than a word space) end.
    let starts: Vec<Vec<f32>> = rows
        .iter()
        .map(|r| {
            let mut xs: Vec<(f32, f32)> = r.iter().map(|i| (i.x0, i.x1)).collect();
            xs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut out = Vec::new();
            let mut reach = f32::MIN;
            for (x0, x1) in xs {
                if reach > f32::MIN && x0 - reach >= 0.6 * smax {
                    out.push(x0);
                }
                reach = reach.max(x1);
            }
            out
        })
        .collect();
    let column = starts.first().into_iter().flatten().copied().find(|&c| starts.iter().all(|s| s.iter().any(|&x| (x - c).abs() <= 0.5 * smax)));
    rows.into_iter()
        .zip(&starts)
        .map(|(r, s)| {
            let Some(c) = column else { return parse_box(pg, r).0 };
            let split = s.iter().copied().find(|&x| (x - c).abs() <= 0.5 * smax).unwrap_or(c);
            let (value, condition): (Vec<Item>, Vec<Item>) = r.into_iter().partition(|i| i.x0 < split - 0.1);
            let mut nodes = parse_box(pg, value).0;
            nodes.push(Node::Gap);
            nodes.extend(parse_box(pg, condition).0);
            nodes
        })
        .collect()
}

fn split_matrix(pg: &PageGlyphs, rows: Vec<Vec<Item>>) -> Vec<Vec<Vec<Node>>> {
    let smax = rows.iter().flatten().map(|i| i.size).fold(0.0, f32::max).max(1.0);
    let mut spans: Vec<(f32, f32)> = rows.iter().flatten().map(|i| (i.x0, i.x1)).collect();
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut cols: Vec<(f32, f32)> = Vec::new();
    for (x0, x1) in spans {
        match cols.last_mut() {
            Some(c) if x0 <= c.1 + 0.6 * smax => c.1 = c.1.max(x1),
            _ => cols.push((x0, x1)),
        }
    }
    rows.into_iter()
        .map(|row| {
            let mut cells: Vec<Vec<Item>> = vec![Vec::new(); cols.len().max(1)];
            for it in row {
                let k = cols.iter().position(|c| it.cx() >= c.0 - 0.1 && it.cx() <= c.1 + 0.1).unwrap_or(0);
                cells[k].push(it);
            }
            cells.into_iter().map(|c| parse_box(pg, c).0).collect()
        })
        .collect()
}

/// Limits stacked directly above/below ∑ ∏ ∫.
fn build_big_op_limits(pg: &PageGlyphs, mut items: Vec<Item>) -> Vec<Item> {
    let mut done: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let body = median(pg.glyphs.iter().filter(|g| g.ch.is_alphanumeric()).map(|g| g.size).collect()).max(4.0);
    loop {
        let pos = items.iter().position(|it| {
            matches!(it.node, Node::G(g) if matches!(pg.glyphs[g].ch, '∑' | '∏' | '∫' | '∮' | '⌠' | '⌡' | '⎮') && !done.contains(&g))
        });
        let Some(i) = pos else { return items };
        let Node::G(g) = items[i].node else { return items };
        done.insert(g);
        let op = items[i].clone();
        let s = op.size;
        let w = op.x1 - op.x0;
        let h = op.y1 - op.y0;
        // Limits are script-size: measured against the page's text size, not
        // the (enlarged) operator glyph, so a neighbouring line of prose is
        // never taken for a limit. Sums/products carry them centred above and
        // below; integrals usually beside the top and bottom of the sign.
        let integral = matches!(pg.glyphs[g].ch, '∫' | '∮' | '⌠' | '⌡' | '⎮');
        let small = |o: &Item| o.size <= 0.85 * body;
        let above = items_in(&items, |o| {
            o.glyphs != op.glyphs
                && small(o)
                && ((o.cx() >= op.x0 - 0.4 * w && o.cx() <= op.x1 + 0.4 * w && o.y1 <= op.y0 + 1.0 && o.y1 >= op.y0 - 0.9 * body)
                    || integral && o.x0 >= op.x0 + 0.3 * w && o.x0 <= op.x1 + 0.6 * body && o.cy() >= op.y0 - 0.5 * body && o.cy() <= op.y0 + 0.35 * h)
        });
        let below = items_in(&items, |o| {
            o.glyphs != op.glyphs
                && small(o)
                && !above.iter().any(|&a| items[a].glyphs == o.glyphs)
                && ((o.cx() >= op.x0 - 0.4 * w && o.cx() <= op.x1 + 0.4 * w && o.y0 >= op.y1 - 1.0 && o.y0 <= op.y1 + 0.9 * body)
                    || integral && o.x0 >= op.x0 && o.x0 <= op.x1 + 0.6 * body && o.cy() >= op.y1 - 0.35 * h && o.cy() <= op.y1 + 0.5 * body)
        });
        if above.is_empty() && below.is_empty() {
            items[i].force_base = true;
            continue;
        }
        let mut all = vec![i];
        all.extend(above.iter().copied());
        all.extend(below.iter().copied());
        all.sort();
        all.dedup();
        let (taken, rest) = take_items(items, &all);
        let up = select(&all, &taken, &above);
        let lo = select(&all, &taken, &below);
        let node = Node::BigOp {
            op: g,
            lower: if lo.is_empty() { None } else { Some(parse_box(pg, lo).0) },
            upper: if up.is_empty() { None } else { Some(parse_box(pg, up).0) },
        };
        items = rest;
        items.push(composite(node, &taken, op.base, s, true));
    }
}

/// The dominant baseline and size of a set of items.
fn main_baseline(items: &[Item]) -> (f32, f32) {
    if items.is_empty() {
        return (0.0, 0.0);
    }
    // Enlarged single brackets do not set the line's size (the text does).
    let text_max = items.iter().filter(|i| !i.force_base && !i.bracket).map(|i| i.size).fold(0.0, f32::max);
    let mut smax = if text_max > 0.0 { text_max } else { items.iter().filter(|i| !i.force_base).map(|i| i.size).fold(0.0, f32::max) };
    // A bracketed group carries its content's size (a big operator's glyph
    // size is inflated and says nothing about the line).
    let forced_max = items
        .iter()
        .filter(|i| i.force_base && matches!(i.node, Node::Delim(..) | Node::Matrix(..) | Node::Cases(..)))
        .map(|i| i.size)
        .fold(0.0, f32::max);
    // When everything else is script-size (a bracketed group and its power,
    // "(1 - 5x)^2" alone in a denominator), the bracketed group defines the
    // line.
    let bracketed = |i: &Item| matches!(i.node, Node::Delim(..) | Node::Matrix(..) | Node::Cases(..));
    let use_forced = smax < 0.88 * forced_max;
    if use_forced {
        smax = forced_max;
    } else if smax <= 0.0 {
        smax = items.iter().map(|i| i.size).fold(0.0, f32::max);
    }
    // Letters and operators define the baseline when present: full-size
    // digits may be stacked above or below a symbol (nuclide notation).
    let full = |i: &&Item| i.size >= 0.88 * smax && (!i.force_base || use_forced && bracketed(i));
    let symbols = items.iter().filter(full).any(|i| !i.numeral);
    let mut cands: Vec<(f32, f32)> = items.iter().filter(full).filter(|i| !symbols || !i.numeral).map(|i| (i.base, (i.x1 - i.x0).max(0.5))).collect();
    if cands.is_empty() {
        cands = items.iter().map(|i| (i.base, (i.x1 - i.x0).max(0.5))).collect();
    }
    cands.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut best = (cands[0].0, 0.0f32);
    for k in 0..cands.len() {
        let mut cover = 0.0;
        let mut sum = 0.0;
        let mut j = k;
        while j < cands.len() && cands[j].0 - cands[k].0 <= 0.15 * smax {
            cover += cands[j].1;
            sum += cands[j].0 * cands[j].1;
            j += 1;
        }
        if cover > best.1 {
            best = (sum / cover, cover);
        }
    }
    (best.0, smax)
}

/// A digit or sign glyph: what a stacked nuclide/charge number is made of.
fn is_stack_numeral(pg: &PageGlyphs, it: &Item) -> bool {
    match it.node {
        Node::G(i) => {
            let c = pg.glyphs[i].ch;
            c.is_ascii_digit() || matches!(c, '-' | '−' | '–' | '+')
        }
        _ => false,
    }
}

/// The verbatim text of a line set (almost) entirely in a monospace font,
/// with the spacing its glyph positions show, and the font's advance.
fn code_line_text(pg: &PageGlyphs, glyphs: &[usize]) -> Option<(String, f32)> {
    let mut gl: Vec<&Glyph> = glyphs.iter().map(|&g| &pg.glyphs[g]).filter(|g| !is_space_char(g.ch)).collect();
    if gl.is_empty() {
        return None;
    }
    let mono = gl.iter().filter(|g| g.mono).count();
    if mono * 5 < gl.len() * 4 {
        return None;
    }
    gl.sort_by(|a, b| a.lx0.partial_cmp(&b.lx0).unwrap_or(std::cmp::Ordering::Equal));
    let advance = median(gl.iter().filter(|g| g.mono).map(|g| g.lx1 - g.lx0).collect()).max(1.0);
    let mut text = String::new();
    let mut right: Option<(f32, bool)> = None;
    for g in gl {
        // A drawn mark (a binary point) stands apart from its neighbours.
        let drawn = g.font == "VectorDot";
        if let Some((r, prev_drawn)) = right {
            let gap = g.lx0 - r;
            if gap > 0.5 * advance || drawn || prev_drawn {
                text.push_str(&" ".repeat(((gap / advance).round() as usize).max(1)));
            }
        }
        text.push(g.ch);
        right = Some((g.lx1, drawn));
    }
    Some((text, advance))
}

/// Multiple-choice letters set in their own column are vertically centred on
/// a wrapped option: a two-line option's "A" sits between its lines, a
/// three-line option's on the middle line. In reading order the letter
/// heads the option's first line, so it is moved there. Only a column of
/// three or more letters in sequence (A, B, C…) is treated this way, and a
/// letter on a text line moves only in a column whose other letters show the
/// centred style (one stands alone between lines).
fn attach_option_letters(pg: &PageGlyphs, built: &mut Vec<LayoutLine>) {
    use std::cmp::Ordering::Equal;
    struct Head {
        idx: usize,
        letter: char,
        glyph: usize,
        x0: f32,
        x1: f32,
        alone: bool,
        rest_x0: f32,
    }
    let mut heads: Vec<Head> = Vec::new();
    for (i, l) in built.iter().enumerate() {
        if l.code {
            continue;
        }
        let mut gl: Vec<usize> = l.glyphs.clone();
        gl.sort_by(|&a, &b| pg.glyphs[a].x0.partial_cmp(&pg.glyphs[b].x0).unwrap_or(Equal));
        let Some(&first) = gl.first() else { continue };
        let c = pg.glyphs[first].ch;
        if !matches!(c, 'A'..='E') || !l.text.starts_with(c) {
            continue;
        }
        let s = l.size.max(4.0);
        let (x0, x1) = (pg.glyphs[first].x0, pg.glyphs[first].x1);
        if gl.len() == 1 && l.text.trim().len() == 1 {
            heads.push(Head { idx: i, letter: c, glyph: first, x0, x1, alone: true, rest_x0: f32::NAN });
        } else if gl.len() > 1 && l.text[1..].starts_with(' ') && pg.glyphs[gl[1]].x0 - x1 >= 0.4 * s {
            heads.push(Head { idx: i, letter: c, glyph: first, x0, x1, alone: false, rest_x0: pg.glyphs[gl[1]].x0 });
        }
    }
    if heads.len() < 3 {
        return;
    }
    let mut order: Vec<usize> = (0..built.len()).collect();
    order.sort_by(|&a, &b| built[a].baseline.partial_cmp(&built[b].baseline).unwrap_or(Equal));
    let mut pos = vec![0usize; built.len()];
    for (i, &j) in order.iter().enumerate() {
        pos[j] = i;
    }
    // (head index, line that starts the option)
    let mut moves: Vec<(usize, usize)> = Vec::new();
    let mut done = vec![false; heads.len()];
    for h in 0..heads.len() {
        if done[h] {
            continue;
        }
        let mut col: Vec<usize> = (0..heads.len()).filter(|&k| !done[k] && (heads[k].x0 - heads[h].x0).abs() <= 2.0).collect();
        for &k in &col {
            done[k] = true;
        }
        col.sort_by(|&a, &b| built[heads[a].idx].baseline.partial_cmp(&built[heads[b].idx].baseline).unwrap_or(Equal));
        if col.len() < 3 || !col.windows(2).all(|w| heads[w[1]].letter as u32 == heads[w[0]].letter as u32 + 1) {
            continue;
        }
        let centred = col.iter().any(|&k| heads[k].alone);
        if !centred {
            continue;
        }
        let head_lines: Vec<usize> = col.iter().map(|&k| heads[k].idx).collect();
        for &k in &col {
            let hd = &heads[k];
            let hl = &built[hd.idx];
            let s = hl.size.max(4.0);
            // The option's text column: right of the letter column.
            let xt = if hd.alone {
                order
                    .iter()
                    .copied()
                    .filter(|&j| {
                        j != hd.idx
                            && !built[j].code
                            && built[j].x0 > hd.x1 + 0.15 * s
                            && built[j].x0 < hd.x1 + 4.0 * s
                            && (built[j].baseline - hl.baseline).abs() <= 1.5 * s
                    })
                    .min_by(|&a, &b| (built[a].baseline - hl.baseline).abs().partial_cmp(&(built[b].baseline - hl.baseline).abs()).unwrap_or(Equal))
                    .map(|j| built[j].x0)
            } else {
                Some(hd.rest_x0)
            };
            let Some(xt) = xt else { continue };
            // The option's lines above / below the letter, up to a gap or
            // another option's letter.
            let walk = |dir: isize| -> Vec<usize> {
                let mut got = Vec::new();
                let mut cur = hl.baseline;
                let mut i = pos[hd.idx] as isize + dir;
                while i >= 0 && (i as usize) < order.len() {
                    let j = order[i as usize];
                    if (built[j].baseline - cur).abs() > 1.5 * s || head_lines.contains(&j) {
                        break;
                    }
                    if (built[j].x0 - xt).abs() <= 2.5 && !built[j].code {
                        got.push(j);
                        cur = built[j].baseline;
                    }
                    i += dir;
                }
                got
            };
            let above = walk(-1);
            let below = walk(1);
            let m = below.len();
            if m == 0 || above.len() < m {
                continue;
            }
            let first = above[m - 1];
            let mid = (built[first].baseline + built[below[m - 1]].baseline) * 0.5;
            if (mid - hl.baseline).abs() > 0.35 * s {
                continue;
            }
            moves.push((k, first));
        }
    }
    if moves.is_empty() {
        return;
    }
    let mut remove: Vec<usize> = Vec::new();
    for (k, first) in moves {
        let hd = &heads[k];
        let bold = pg.glyphs[hd.glyph].bold;
        let f = &mut built[first];
        f.text = format!("{} {}", hd.letter, f.text);
        f.x0 = f.x0.min(hd.x0);
        f.glyphs.insert(0, hd.glyph);
        f.bold_lead = bold;
        if hd.alone {
            remove.push(hd.idx);
        } else {
            let l = &mut built[hd.idx];
            l.text = l.text[1..].trim_start().to_string();
            l.glyphs.retain(|&g| g != hd.glyph);
            l.x0 = hd.rest_x0;
            l.bold_lead = l.glyphs.iter().min_by(|&&a, &&b| pg.glyphs[a].x0.partial_cmp(&pg.glyphs[b].x0).unwrap_or(Equal)).map(|&g| pg.glyphs[g].bold).unwrap_or(false);
        }
    }
    remove.sort_unstable();
    for i in remove.into_iter().rev() {
        built.remove(i);
    }
}

/// The items `set` are letters inside a longer word on their own baseline:
/// another letter or digit of their size touches them there.
fn inside_word(pg: &PageGlyphs, items: &[Item], set: &[usize]) -> bool {
    if set.is_empty() {
        return false;
    }
    let members: Vec<Item> = set.iter().map(|&k| items[k].clone()).collect();
    let base = main_baseline(&members).0;
    let size = members.iter().map(|o| o.size).fold(0.0, f32::max).max(1.0);
    let (l, r) = members.iter().fold((f32::MAX, f32::MIN), |e, o| (e.0.min(o.lx0), e.1.max(o.lx1)));
    items.iter().enumerate().any(|(k, o)| {
        !set.contains(&k)
            && matches!(o.node, Node::G(g) if pg.glyphs[g].ch.is_alphanumeric())
            && (o.base - base).abs() < 0.1 * size
            && (o.size - size).abs() < 0.15 * size
            && ((o.lx1 - l).abs() < 0.08 * size || (o.lx0 - r).abs() < 0.08 * size)
    })
}

/// A single ordinary bracket glyph (not an assembled tall delimiter).
fn is_bracket_glyph(pg: &PageGlyphs, it: &Item) -> bool {
    matches!(it.node, Node::G(i) if matches!(pg.glyphs[i].ch, '(' | ')' | '[' | ']'))
}

/// A letter glyph (a symbol a stacked number can belong to).
fn is_letter_item(pg: &PageGlyphs, it: &Item) -> bool {
    match &it.node {
        Node::G(i) => pg.glyphs[*i].ch.is_alphabetic(),
        Node::Accent(_, inner) => inner.iter().any(|n| matches!(n, Node::G(i) if pg.glyphs[*i].ch.is_alphabetic())),
        _ => false,
    }
}

/// Structures positioned on the math axis rather than a glyph baseline: a
/// stacked fraction, a matrix, cases, or delimiters around one of them.
fn axis_hung(n: &Node) -> bool {
    match n {
        Node::Frac(..) | Node::Matrix(..) | Node::Cases(..) => true,
        Node::BigOp { .. } => true,
        Node::Delim(_, _, inner) => inner.iter().any(axis_hung),
        _ => false,
    }
}

/// Group page items into visual lines: baseline rows, with small raised /
/// lowered items attached to the adjacent larger row as scripts.
fn group_lines(pg: &PageGlyphs, items: Vec<Item>) -> Vec<Vec<Item>> {
    if items.is_empty() {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| items[a].base.partial_cmp(&items[b].base).unwrap_or(std::cmp::Ordering::Equal));
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut row_base: Vec<f32> = Vec::new();
    // A stacked fraction hangs from the math axis, and how far its bar sits
    // above the text baseline depends on its parts (a radical denominator
    // lifts it): its estimated baseline is not exact, so fractions are placed
    // after the glyph rows exist.
    let on_axis = |it: &Item| axis_hung(&it.node);
    for &i in &order {
        let it = &items[i];
        if on_axis(it) {
            continue;
        }
        let tol = 0.18 * it.size.max(4.0);
        match row_base.last() {
            Some(&b) if (it.base - b).abs() <= tol => {
                rows.last_mut().unwrap().push(i);
                let r = rows.last().unwrap();
                *row_base.last_mut().unwrap() = r.iter().map(|&k| items[k].base).sum::<f32>() / r.len() as f32;
            }
            _ => {
                rows.push(vec![i]);
                row_base.push(it.base);
            }
        }
    }
    // A fraction joins the row whose baseline lies near its axis and whose
    // glyphs sit beside it within its height; otherwise it starts its own row
    // (or shares one with fractions on the same axis).
    let glyph_rows = rows.len();
    // A big operator is enlarged: the text it is set in is full size, never
    // a row of its own limits' script ("lim" over "δx→0" beside Σ).
    let body = median(items.iter().filter(|it| matches!(it.node, Node::G(g) if pg.glyphs[g].ch.is_alphanumeric())).map(|it| it.size).collect()).max(4.0);
    let text_rows: Vec<bool> = rows.iter().map(|r| r.iter().any(|&k| items[k].size >= 0.8 * body)).collect();
    for &i in &order {
        let it = &items[i];
        if !on_axis(it) {
            continue;
        }
        let s = it.size.max(4.0);
        let big_op = matches!(it.node, Node::BigOp { .. });
        let text_beside = big_op
            && (0..glyph_rows).any(|ri| {
                let b = row_base[ri];
                text_rows[ri] && b >= it.y0 && b <= it.y1 && rows[ri].iter().any(|&k| hgap(&items[k], it) <= 3.0 * s && items[k].cy() >= it.y0 && items[k].cy() <= it.y1)
            });
        let mut best: Option<(usize, f32)> = None;
        for ri in 0..glyph_rows {
            let b = row_base[ri];
            let d = b - it.base;
            if d < -0.35 * s || d > 0.45 * s || b < it.y0 || b > it.y1 {
                continue;
            }
            if text_beside && !text_rows[ri] {
                continue;
            }
            let beside = rows[ri].iter().any(|&k| {
                let o = &items[k];
                hgap(o, it) <= 3.0 * s && o.cy() >= it.y0 && o.cy() <= it.y1
            });
            if beside && best.map(|(_, bd)| d.abs() < bd).unwrap_or(true) {
                best = Some((ri, d.abs()));
            }
        }
        // No neighbour beside it: the plain baseline rule (table cells, a
        // display line of fractions).
        if best.is_none() {
            for ri in 0..rows.len() {
                let d = (row_base[ri] - it.base).abs();
                if d <= 0.18 * s && best.map(|(_, bd)| d < bd).unwrap_or(true) {
                    best = Some((ri, d));
                }
            }
        }
        match best {
            Some((ri, _)) => rows[ri].push(i),
            None => {
                rows.push(vec![i]);
                row_base.push(it.base);
            }
        }
    }
    // Full-size rows side by side whose baselines differ slightly are one
    // line: an embedded equation object is vertically centred on its text
    // line, not baseline-aligned ("C" beside a taller r(x/y)^(1/3)). A big
    // operator likewise spans the rows set beside it within its height
    // ("K ∫₀⁹ e^{x/9} …" with K centred on the sign, the integrand on its own
    // baseline).
    loop {
        let ext = |r: &Vec<usize>| {
            let mut e = (f32::MAX, f32::MIN, 0.0f32);
            for &k in r {
                e.0 = e.0.min(items[k].x0);
                e.1 = e.1.max(items[k].x1);
                if !matches!(items[k].node, Node::BigOp { .. }) {
                    e.2 = e.2.max(items[k].size);
                }
            }
            e
        };
        let spans = |a: usize, b: usize, s: f32| {
            rows[a].iter().any(|&t| {
                let op = &items[t];
                matches!(op.node, Node::BigOp { .. })
                    && rows[b].iter().any(|&k| {
                        let o = &items[k];
                        o.size >= 0.8 * s && o.cy() > op.y0 && o.cy() < op.y1 && hgap(o, op) <= 1.5 * s
                    })
            })
        };
        let mut pair: Option<(usize, usize)> = None;
        'find: for a in 0..rows.len() {
            for b in (a + 1)..rows.len() {
                let (ea, eb) = (ext(&rows[a]), ext(&rows[b]));
                let s = ea.2.max(eb.2).max(4.0);
                if ea.2 > 0.0 && eb.2 > 0.0 && ea.2.min(eb.2) < 0.8 * s {
                    continue;
                }
                let gap = if ea.1 <= eb.0 {
                    eb.0 - ea.1
                } else if eb.1 <= ea.0 {
                    ea.0 - eb.1
                } else {
                    continue;
                };
                let near = (row_base[a] - row_base[b]).abs() <= 0.25 * s && gap <= 3.0 * s;
                if near || spans(a, b, s) || spans(b, a, s) {
                    pair = Some((a, b));
                    break 'find;
                }
            }
        }
        let Some((a, b)) = pair else { break };
        let moved = rows.remove(b);
        row_base.remove(b);
        rows[a].extend(moved);
        let r = &rows[a];
        let full: Vec<f32> = r.iter().filter(|&&k| !items[k].force_base).map(|&k| items[k].base).collect();
        if !full.is_empty() {
            row_base[a] = full.iter().sum::<f32>() / full.len() as f32;
        }
    }
    let row_size: Vec<f32> = rows.iter().map(|r| r.iter().map(|&k| items[k].size).fold(0.0, f32::max)).collect();
    let mut target = vec![0usize; items.len()];
    for (ri, r) in rows.iter().enumerate() {
        for &k in r {
            target[k] = ri;
        }
    }
    // Script clusters: horizontally contiguous small items within one row
    // move together to the adjacent larger row they decorate.
    for (own, r) in rows.iter().enumerate() {
        let mut xs: Vec<usize> = r.clone();
        xs.sort_by(|&a, &b| items[a].x0.partial_cmp(&items[b].x0).unwrap_or(std::cmp::Ordering::Equal));
        let mut clusters: Vec<Vec<usize>> = Vec::new();
        for &k in &xs {
            match clusters.last_mut() {
                Some(c) if hgap(&items[*c.last().unwrap()], &items[k]) <= 0.7 * items[k].size.max(items[*c.last().unwrap()].size) => c.push(k),
                _ => clusters.push(vec![k]),
            }
        }
        for c in clusters {
            // A script's size is its text's: brackets around a script are
            // often set at the body size (e^{f(x_n)} with Symbol parentheses).
            let text_size = c.iter().filter(|&&k| !is_bracket_glyph(pg, &items[k])).map(|&k| items[k].size).fold(0.0, f32::max);
            let csize = if text_size > 0.0 { text_size } else { c.iter().map(|&k| items[k].size).fold(0.0, f32::max) };
            let cbase = c.iter().map(|&k| items[k].base).sum::<f32>() / c.len() as f32;
            let first = c[0];
            let last = *c.last().unwrap();
            let numeric = c.iter().all(|&k| is_stack_numeral(pg, &items[k]));
            let mut best: Option<(usize, f32)> = None;
            for (hi, hr) in rows.iter().enumerate() {
                if hi == own {
                    continue;
                }
                let hs = row_size[hi];
                let d = cbase - row_base[hi];
                // A full-size number stacked within about two thirds of an
                // em above or below a symbol (Word nuclide notation: 235
                // over 92 before U) is a prescript; rows that close are
                // never separate lines of text.
                let stacked = numeric && csize > 0.88 * hs && d.abs() >= 0.3 * hs && d.abs() <= 0.7 * hs;
                if stacked {
                    let host = hr.iter().any(|&k| {
                        let h = &items[k];
                        is_letter_item(pg, h) && hgap(&items[last], h) <= 0.5 * hs && h.x0 >= items[last].x1 - 0.5 * hs
                    });
                    if host && best.map(|(_, bd)| d.abs() < bd).unwrap_or(true) {
                        best = Some((hi, d.abs()));
                    }
                    continue;
                }
                if csize > 0.88 * hs {
                    continue;
                }
                if d.abs() < 0.1 * hs {
                    continue;
                }
                // Scripts follow their base (left end adjacent); prescripts
                // precede it (right end adjacent). A tall host (big
                // delimiter, stacked fraction) raises/lowers its scripts in
                // proportion to its own extent.
                let adjacent = hr.iter().any(|&k| {
                    let h = &items[k];
                    let up = (0.85 * hs).max(h.base - h.y0 + 0.35 * csize);
                    let down = (0.6 * hs).max(h.y1 - h.base + 0.5 * csize);
                    h.size > csize
                        && d >= -up
                        && d <= down
                        && (hgap(&items[first], h) <= 0.9 * hs && h.x1 <= items[first].x0 + 0.5 * hs || hgap(&items[last], h) <= 0.5 * hs && h.x0 >= items[last].x1 - 0.5 * hs)
                });
                if !adjacent {
                    continue;
                }
                if best.map(|(_, bd)| d.abs() < bd).unwrap_or(true) {
                    best = Some((hi, d.abs()));
                }
            }
            if let Some((hi, _)) = best {
                for &k in &c {
                    target[k] = hi;
                }
            }
        }
    }
    // A script attached to a row that itself moved (the subscript n of an
    // exponent f(x_n)) follows that row to its final line.
    let row_dest: Vec<usize> = rows
        .iter()
        .enumerate()
        .map(|(ri, r)| {
            let first = r.first().map(|&k| target[k]).unwrap_or(ri);
            if first != ri && r.iter().all(|&k| target[k] == first) { first } else { ri }
        })
        .collect();
    for t in target.iter_mut() {
        for _ in 0..4 {
            if row_dest[*t] == *t {
                break;
            }
            *t = row_dest[*t];
        }
    }
    let mut out: Vec<Vec<Item>> = vec![Vec::new(); rows.len()];
    for (i, it) in items.into_iter().enumerate() {
        out[target[i]].push(it);
    }
    out.into_iter().filter(|r| !r.is_empty()).collect()
}

/// Parse a box of items into nodes: the base sequence plus attached scripts.
/// Returns (nodes, baseline, size, unplaced count).
fn parse_box(pg: &PageGlyphs, items: Vec<Item>) -> (Vec<Node>, f32, f32, usize) {
    if items.is_empty() {
        return (Vec::new(), 0.0, 0.0, 0);
    }
    let (b, s) = main_baseline(&items);
    let s = s.max(1.0);
    #[derive(PartialEq, Clone, Copy)]
    enum Role {
        Base,
        Sup,
        Sub,
    }
    let roles: Vec<Role> = items
        .iter()
        .map(|it| {
            if it.force_base {
                return Role::Base;
            }
            let d = it.base - b;
            let small = it.size < 0.9 * s;
            // Full-size stacked numbers (see group_lines) sit at least
            // 0.3 em off the line; rows nearer than that were merged as one.
            let stacked = !small && d.abs() >= 0.3 * s && (is_stack_numeral(pg, it) || is_bracket_glyph(pg, it));
            // A reduced-size character set tight against the letter before
            // it, (almost) on that letter's baseline, is its subscript: Word
            // drops E_k's k by under half a point.
            let tight_sub = it.size < 0.8 * s
                && d > -0.05 * s
                && items.iter().any(|o| {
                    o.size >= 0.9 * s
                        && (o.base - b).abs() < 0.1 * s
                        && it.x0 - o.x1 >= -0.1 * s
                        && it.x0 - o.x1 <= 0.12 * s
                        && matches!(o.node, Node::G(g) if pg.glyphs[g].ch.is_alphabetic())
                });
            if small && d < -0.14 * s || stacked && d < 0.0 {
                Role::Sup
            } else if small && d > 0.07 * s || stacked && d > 0.0 || tight_sub {
                Role::Sub
            } else {
                Role::Base
            }
        })
        .collect();
    let mut base_idx: Vec<usize> = (0..items.len()).filter(|&i| roles[i] == Role::Base).collect();
    base_idx.sort_by(|&x, &y| items[x].x0.partial_cmp(&items[y].x0).unwrap_or(std::cmp::Ordering::Equal));
    let mut script_idx: Vec<usize> = (0..items.len()).filter(|&i| roles[i] != Role::Base).collect();
    script_idx.sort_by(|&x, &y| items[x].x0.partial_cmp(&items[y].x0).unwrap_or(std::cmp::Ordering::Equal));
    let mut groups: Vec<(Role, Vec<usize>)> = Vec::new();
    for &k in &script_idx {
        let r = roles[k];
        if let Some((_, gv)) = groups.iter_mut().rev().find(|(gr, _)| *gr == r) {
            let last = *gv.last().unwrap();
            let gap = items[k].x0 - items[last].x1;
            let between = base_idx.iter().any(|&bi| items[bi].x0 >= items[last].x1 - 0.3 && items[bi].x1 <= items[k].x0 + 0.3);
            if gap <= 0.55 * s && !between {
                gv.push(k);
                continue;
            }
        }
        groups.push((r, vec![k]));
    }
    let mut subs: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    let mut sups: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    let mut pre_sub: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    let mut pre_sup: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    let mut unplaced: Vec<usize> = Vec::new();
    for (role, members) in &groups {
        let gx0 = members.iter().map(|&k| items[k].x0).fold(f32::MAX, f32::min);
        let gx1 = members.iter().map(|&k| items[k].x1).fold(f32::MIN, f32::max);
        let left = base_idx
            .iter()
            .copied()
            .filter(|&bi| items[bi].x1 <= gx0 + 0.35 * s && items[bi].x0 < gx0)
            .max_by(|&x, &y| items[x].x1.partial_cmp(&items[y].x1).unwrap_or(std::cmp::Ordering::Equal));
        let right = base_idx
            .iter()
            .copied()
            .filter(|&bi| items[bi].x0 >= gx1 - 0.35 * s)
            .min_by(|&x, &y| items[x].x0.partial_cmp(&items[y].x0).unwrap_or(std::cmp::Ordering::Equal));
        // Measured from the base's own scripts placed so far (E_k^{1.5}: the
        // exponent follows the subscript, not the E).
        let left_gap = left
            .map(|bi| {
                let reach = subs.get(&bi).into_iter().chain(sups.get(&bi)).flatten().fold(items[bi].x1, |x, &m| x.max(items[m].x1));
                gx0 - reach
            })
            .unwrap_or(f32::MAX);
        let right_gap = right.map(|bi| items[bi].x0 - gx1).unwrap_or(f32::MAX);
        // A full-size stacked number belongs to the symbol after it.
        let stacked_pre = members.iter().all(|&k| items[k].size >= 0.9 * s && is_stack_numeral(pg, &items[k]))
            && right.is_some_and(|bi| is_letter_item(pg, &items[bi]))
            && right_gap <= 0.5 * s;
        // A limit set centred under an operator name ("δx→0" under "lim")
        // is that operator's subscript.
        let under_op = if *role == Role::Sub { limit_operator_over(pg, &items, &base_idx, 0.5 * (gx0 + gx1), s) } else { None };
        if let Some(last) = under_op {
            subs.entry(last).or_default().extend(members.iter().copied());
        } else if stacked_pre {
            let map = if *role == Role::Sub { &mut pre_sub } else { &mut pre_sup };
            map.entry(right.unwrap()).or_default().extend(members.iter().copied());
        } else if left.is_some() && left_gap <= 0.7 * s && !(right_gap < 0.15 * s && left_gap > 0.35 * s) {
            let map = if *role == Role::Sub { &mut subs } else { &mut sups };
            map.entry(left.unwrap()).or_default().extend(members.iter().copied());
        } else if right.is_some() && right_gap <= 0.4 * s {
            let map = if *role == Role::Sub { &mut pre_sub } else { &mut pre_sup };
            map.entry(right.unwrap()).or_default().extend(members.iter().copied());
        } else {
            unplaced.extend(members.iter().copied());
        }
    }
    let mut seq = base_idx.clone();
    seq.extend(unplaced.iter().copied());
    seq.sort_by(|&x, &y| items[x].x0.partial_cmp(&items[y].x0).unwrap_or(std::cmp::Ordering::Equal));
    let sub_nodes = |v: &Vec<usize>| -> Vec<Node> { parse_box(pg, v.iter().map(|&k| items[k].clone()).collect()).0 };
    let mut nodes = Vec::new();
    let mut prev_right: Option<(f32, f32, f32)> = None; // (tight right, loose right, centre)
    // An upright letter whose following letters spell a function name
    // exactly ("sin" in "i sin θ"): letters of one word abut, so any clear
    // advance gap before the name is a word space, however thin.
    let upright_letter = |k: usize| matches!(items[k].node, Node::G(g) if pg.glyphs[g].ch.is_ascii_alphabetic() && !pg.glyphs[g].italic);
    let letter_item = |k: usize| matches!(items[k].node, Node::G(g) if pg.glyphs[g].ch.is_alphabetic());
    // (Glyphs of one word abut; a number set a clear gap before a word is a
    // quantity and its unit: "200 N".)
    let digit_item = |k: usize| matches!(items[k].node, Node::G(g) if pg.glyphs[g].ch.is_ascii_digit());
    let starts_function_name = |at: usize| {
        let mut name = String::new();
        let mut j = at;
        while j < seq.len() && upright_letter(seq[j]) && (j == at || items[seq[j]].lx0 - items[seq[j - 1]].lx1 <= 0.03 * s) {
            if let Node::G(g) = items[seq[j]].node {
                name.push(pg.glyphs[g].ch);
            }
            j += 1;
        }
        FUNCTION_NAMES.contains(&name.as_str())
    };
    for (pos, &k) in seq.iter().enumerate() {
        let it = &items[k];
        // A prescript stands before its base: the gap before them is the
        // gap before the prescript (¹₁p after "+", not a wide space).
        let (ix0, ilx0) = pre_sub
            .get(&k)
            .into_iter()
            .chain(pre_sup.get(&k))
            .flatten()
            .fold((it.x0, it.lx0), |(a, l), &m| (a.min(items[m].x0), l.min(items[m].lx0)));
        // Closing punctuation takes a clear word space, not a kern (a ratio
        // colon is spaced on both sides, so it is not one).
        let closing = matches!(it.node, Node::G(g) if matches!(pg.glyphs[g].ch, ',' | '.' | ';' | ')' | ']'));
        if let Some((pr, plr, pc)) = prev_right {
            let gap = ix0 - pr;
            let loose_gap = ilx0 - plr;
            // (A typed space may sit under an italic overhang: "of the", or
            // the swash of an italic f reaching back over it: "is f".)
            let origin = if pre_sub.contains_key(&k) || pre_sup.contains_key(&k) || it.glyphs.is_empty() {
                ix0
            } else {
                it.glyphs.iter().map(|&g| pg.glyphs[g].ox).fold(f32::MAX, f32::min).max(ix0)
            };
            if gap > 2.2 * s {
                nodes.push(Node::Gap);
            } else if loose_gap > 0.9 * s {
                nodes.push(Node::WideSpace);
            } else if space_glyph_between(pg, pr.min(pc.max(pr - 0.3 * s)), origin, b, s)
                || !closing && generated_space_between(pg, pr, ix0, b, s)
                || loose_gap > if closing { 0.25 } else { 0.12 } * s
                || loose_gap > 0.06 * s && pos > 0 && upright_letter(seq[pos - 1]) && upright_letter(k) && starts_function_name(pos)
                || pos > 0 && letter_item(seq[pos - 1]) && letter_item(k) && space_between(&pg.thin_spaces, pr - 0.3, ix0 + 0.3, b, s)
                || loose_gap > 0.08 * s && pos > 0 && digit_item(seq[pos - 1]) && letter_item(k)
            {
                nodes.push(Node::Space);
            }
        }
        if pre_sub.contains_key(&k) || pre_sup.contains_key(&k) {
            nodes.push(Node::Pre { sub: pre_sub.get(&k).map(&sub_nodes), sup: pre_sup.get(&k).map(&sub_nodes) });
        }
        let mut right = it.x1;
        let mut lright = it.lx1;
        if subs.contains_key(&k) || sups.contains_key(&k) {
            for v in subs.get(&k).into_iter().chain(sups.get(&k)) {
                for &m in v {
                    right = right.max(items[m].x1);
                    lright = lright.max(items[m].lx1);
                }
            }
            nodes.push(Node::Scripted { base: Box::new(it.node.clone()), sub: subs.get(&k).map(&sub_nodes), sup: sups.get(&k).map(&sub_nodes) });
        } else {
            nodes.push(it.node.clone());
        }
        prev_right = Some((right, lright, (it.x0 + it.x1) * 0.5));
    }
    (nodes, b, s, unplaced.len())
}

/// The last letter of an upright limit operator name ("lim", "max", "min",
/// "sup", "inf") whose extent covers `cx`.
fn limit_operator_over(pg: &PageGlyphs, items: &[Item], base_idx: &[usize], cx: f32, s: f32) -> Option<usize> {
    let letter = |k: usize| matches!(items[k].node, Node::G(g) if pg.glyphs[g].ch.is_ascii_alphabetic() && !pg.glyphs[g].italic);
    let mut runs: Vec<Vec<usize>> = Vec::new();
    for &k in base_idx {
        if !letter(k) {
            runs.push(Vec::new());
            continue;
        }
        match runs.last_mut() {
            Some(r) if r.last().is_some_and(|&p| items[k].x0 - items[p].x1 <= 0.2 * s) => r.push(k),
            _ => runs.push(vec![k]),
        }
    }
    runs.into_iter().find_map(|r| {
        let word: String = r.iter().filter_map(|&k| if let Node::G(g) = items[k].node { Some(pg.glyphs[g].ch) } else { None }).collect();
        let (x0, x1) = (items[*r.first()?].x0, items[*r.last()?].x1);
        (matches!(word.as_str(), "lim" | "max" | "min" | "sup" | "inf") && cx >= x0 && cx <= x1).then(|| *r.last().unwrap())
    })
}

// ── Tables ──────────────────────────────────────────────────────────────────
//
// A table is a connected set of border strokes: horizontal rules plus
// vertical strokes (line segments or thin filled rectangles, as Word draws
// cell borders). Its distinct rule levels are the row and column boundaries.

#[derive(Debug, Clone, Copy)]
struct BorderSeg {
    horizontal: bool,
    /// y of a horizontal segment, x of a vertical one.
    at: f32,
    from: f32,
    to: f32,
    /// Index into `PageGlyphs::rules` for a horizontal rule.
    rule: Option<usize>,
}

#[derive(Debug, Clone)]
struct TableGrid {
    /// Column boundaries, ascending.
    xs: Vec<f32>,
    /// Row boundaries, ascending (y down).
    ys: Vec<f32>,
    /// The horizontal rules that are this grid's borders.
    rules: Vec<usize>,
    /// Vertical border strokes (x, y0, y1): an interior column boundary
    /// missing in a row band marks merged cells.
    verticals: Vec<(f32, f32, f32)>,
}

impl TableGrid {
    fn contains(&self, x: f32, y: f32) -> bool {
        x > self.xs[0] && x < self.xs[self.xs.len() - 1] && y > self.ys[0] && y < self.ys[self.ys.len() - 1]
    }
    fn cell_of(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        let mut c = self.xs.windows(2).position(|w| x >= w[0] && x < w[1])?;
        let r = self.ys.windows(2).position(|w| y >= w[0] && y < w[1])?;
        // A merged cell ("Memory locations" over three columns) belongs to
        // the first column of its span.
        while c > 0 && !self.boundary_in_band(c, r) {
            c -= 1;
        }
        Some((r, c))
    }
    /// Whether the column boundary `xs[c]` is drawn across row band `r`.
    fn boundary_in_band(&self, c: usize, r: usize) -> bool {
        let (x, y0, y1) = (self.xs[c], self.ys[r], self.ys[r + 1]);
        let covered: f32 = self
            .verticals
            .iter()
            .filter(|v| (v.0 - x).abs() <= 1.5)
            .map(|v| (v.2.min(y1) - v.1.max(y0)).max(0.0))
            .sum();
        covered >= 0.5 * (y1 - y0)
    }
}

fn border_segments(pg: &PageGlyphs) -> Vec<BorderSeg> {
    let mut out: Vec<BorderSeg> = pg
        .rules
        .iter()
        .enumerate()
        .filter(|(_, r)| r.w() >= 3.0)
        .map(|(ri, r)| BorderSeg { horizontal: true, at: r.cy(), from: r.x0, to: r.x1, rule: Some(ri) })
        .collect();
    for p in &pg.paths {
        let (w, h) = (p.x1 - p.x0, p.y1 - p.y0);
        if p.filled && w <= 1.6 && h >= 3.0 {
            out.push(BorderSeg { horizontal: false, at: 0.5 * (p.x0 + p.x1), from: p.y0, to: p.y1, rule: None });
            continue;
        }
        if !p.stroked {
            continue;
        }
        let mut prev: Option<[f32; 2]> = None;
        for &(moved, q) in &p.pts {
            if let (false, Some(a)) = (moved, prev) {
                if (a[0] - q[0]).abs() <= 0.8 && (a[1] - q[1]).abs() >= 3.0 {
                    out.push(BorderSeg { horizontal: false, at: 0.5 * (a[0] + q[0]), from: a[1].min(q[1]), to: a[1].max(q[1]), rule: None });
                }
            }
            prev = Some(q);
        }
    }
    out
}

fn segments_touch(a: &BorderSeg, b: &BorderSeg) -> bool {
    const T: f32 = 1.5;
    match (a.horizontal, b.horizontal) {
        (true, true) | (false, false) => (a.at - b.at).abs() <= 1.0 && a.from <= b.to + T && b.from <= a.to + T,
        (true, false) => b.at >= a.from - T && b.at <= a.to + T && a.at >= b.from - T && a.at <= b.to + T,
        (false, true) => segments_touch(b, a),
    }
}

/// Distinct coordinates (within 1.5 pt), ascending.
fn levels(mut v: Vec<f32>) -> Vec<f32> {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<f32> = Vec::new();
    for x in v {
        match out.last() {
            Some(&l) if x - l <= 1.5 => {}
            _ => out.push(x),
        }
    }
    out
}

fn table_grids(pg: &PageGlyphs) -> Vec<TableGrid> {
    let segs = border_segments(pg);
    let n = segs.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while p[r] != r {
            r = p[r];
        }
        let mut c = i;
        while p[c] != r {
            let next = p[c];
            p[c] = r;
            c = next;
        }
        r
    }
    for i in 0..n {
        for j in (i + 1)..n {
            if segments_touch(&segs[i], &segs[j]) {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                if a != b {
                    parent[a] = b;
                }
            }
        }
    }
    let mut groups: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let mut grids = Vec::new();
    for members in groups.values() {
        let hs: Vec<&BorderSeg> = members.iter().map(|&i| &segs[i]).filter(|s| s.horizontal).collect();
        let vs: Vec<&BorderSeg> = members.iter().map(|&i| &segs[i]).filter(|s| !s.horizontal).collect();
        if hs.len() < 2 || vs.is_empty() {
            continue;
        }
        let hx0 = hs.iter().map(|s| s.from).fold(f32::MAX, f32::min);
        let hx1 = hs.iter().map(|s| s.to).fold(f32::MIN, f32::max);
        let vy0 = vs.iter().map(|s| s.from).fold(f32::MAX, f32::min);
        let vy1 = vs.iter().map(|s| s.to).fold(f32::MIN, f32::max);
        let mut xs = levels(vs.iter().map(|s| s.at).collect());
        let mut ys = levels(hs.iter().map(|s| s.at).collect());
        // Open sides (a stem-and-leaf diagram has no outer border): the
        // strokes' own extent bounds the outer cells.
        if hx0 < xs[0] - 2.0 {
            xs.insert(0, hx0);
        }
        if hx1 > xs[xs.len() - 1] + 2.0 {
            xs.push(hx1);
        }
        if vy0 < ys[0] - 2.0 {
            ys.insert(0, vy0);
        }
        if vy1 > ys[ys.len() - 1] + 2.0 {
            ys.push(vy1);
        }
        if xs.len() < 3 || ys.len() < 2 || xs[xs.len() - 1] - xs[0] > 0.95 * pg.width {
            continue;
        }
        let rules = hs.iter().filter_map(|s| s.rule).collect();
        let verticals = vs.iter().map(|s| (s.at, s.from, s.to)).collect();
        grids.push(TableGrid { xs, ys, rules, verticals });
    }
    grids.sort_by(|a, b| (a.ys[0], a.xs[0]).partial_cmp(&(b.ys[0], b.xs[0])).unwrap_or(std::cmp::Ordering::Equal));
    grids
}

/// The grid widened by a label column when short labels sit just left of
/// it, each within one row band, in at least two rows.
fn with_row_labels(grid: &TableGrid, items: &[Item]) -> Option<TableGrid> {
    let (left, top, bottom) = (grid.xs[0], grid.ys[0], *grid.ys.last()?);
    let mut rows_labelled = std::collections::BTreeSet::new();
    let mut x_min = f32::MAX;
    for it in items {
        let s = it.size.max(4.0);
        if it.x1 > left + 1.0 || it.x1 < left - 3.0 * s || it.x0 < left - 6.0 * s || it.cy() <= top || it.cy() >= bottom {
            continue;
        }
        let r = grid.ys.windows(2).position(|w| it.y0 >= w[0] - 1.0 && it.y1 <= w[1] + 1.0)?;
        rows_labelled.insert(r);
        x_min = x_min.min(it.x0);
    }
    if rows_labelled.len() < 2 {
        return None;
    }
    let mut g = grid.clone();
    g.xs.insert(0, x_min - 1.0);
    Some(g)
}

/// The Markdown table for one grid, or the items back when the grid is not
/// a table of text (graph paper, an empty answer grid).
fn table_line(pg: &PageGlyphs, grid: &TableGrid, inside: Vec<Item>) -> Result<LayoutLine, Vec<Item>> {
    let (nr, nc) = (grid.ys.len() - 1, grid.xs.len() - 1);
    let mut cells: Vec<Vec<Vec<Item>>> = vec![vec![Vec::new(); nc]; nr];
    let mut stray = Vec::new();
    for it in inside {
        match grid.cell_of(it.cx(), it.cy()) {
            Some((r, c)) => cells[r][c].push(it),
            None => stray.push(it),
        }
    }
    let filled = cells.iter().flatten().filter(|c| !c.is_empty()).count();
    let filled_rows = cells.iter().filter(|row| row.iter().any(|c| !c.is_empty())).count();
    // An answer table to complete (a trace table) is mostly empty under a
    // full header row; graph paper and empty answer grids carry no text.
    let header_row = cells.iter().take(2).any(|row| row.iter().filter(|c| !c.is_empty()).count() * 5 >= row.len() * 4);
    // A single boxed row is a table only as a row of named fields
    // ("Destination Address | Source Address | Payload (data) | Checksum"),
    // never a row of answer boxes or bits.
    let word_cells = cells
        .first()
        .map(|row| {
            row.iter()
                .filter(|c| c.iter().flat_map(|it| it.glyphs.iter()).filter(|&&g| pg.glyphs[g].ch.is_alphabetic()).count() >= 3)
                .count()
        })
        .unwrap_or(0);
    let single_row_ok = nr == 1 && word_cells >= 2 && word_cells * 2 >= nc;
    if !stray.is_empty() || filled < 2 || filled_rows < 2 && !single_row_ok || ((filled as f32) < 0.3 * (nr * nc) as f32 && !header_row) {
        let mut back = stray;
        back.extend(cells.into_iter().flatten().flatten());
        return Err(back);
    }
    let glyphs: Vec<usize> = cells.iter().flatten().flatten().flat_map(|it| it.glyphs.iter().copied()).collect();
    let sizes: Vec<f32> = cells.iter().flatten().flatten().map(|it| it.size).collect();
    let render = |items: Vec<Item>| -> String {
        let mut rows: Vec<Vec<Item>> = group_lines(pg, items);
        rows.sort_by(|a, b| {
            let ba = a.iter().map(|i| i.base).sum::<f32>() / a.len().max(1) as f32;
            let bb = b.iter().map(|i| i.base).sum::<f32>() / b.len().max(1) as f32;
            ba.partial_cmp(&bb).unwrap_or(std::cmp::Ordering::Equal)
        });
        rows.into_iter()
            .map(|row| emit_nodes(pg, &parse_box(pg, row).0, false).replace('\t', " ").trim().to_string())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
            .replace('|', "\\|")
    };
    // A header row is printed bold, or heads a two-way table from an empty
    // corner cell; otherwise the table has no header and gets an empty one
    // (Markdown requires a header row).
    let first_bold = {
        let g: Vec<usize> = cells[0].iter().flatten().flat_map(|it| it.glyphs.iter().copied()).collect();
        !g.is_empty() && g.iter().all(|&i| pg.glyphs[i].bold || !pg.glyphs[i].ch.is_alphanumeric())
    };
    let corner_empty = cells[0][0].is_empty() && cells[0].iter().skip(1).any(|c| !c.is_empty());
    let text: Vec<Vec<String>> = cells.into_iter().map(|row| row.into_iter().map(render).collect()).collect();
    // A table whose rows are the answer options A, B, C, D… (first column)
    // under column headings: each option becomes one option line carrying
    // its headings, so the card keeps one tagged line per option.
    let option_rows = nr >= 4
        && corner_empty
        && text.iter().skip(1).enumerate().all(|(i, row)| row[0] == ((b'A' + i as u8) as char).to_string());
    let body = if option_rows {
        text.iter()
            .skip(1)
            .map(|row| {
                let parts: Vec<String> = (1..nc)
                    .filter(|&c| !row[c].is_empty())
                    .map(|c| if text[0][c].is_empty() { row[c].clone() } else { format!("{}: {}", text[0][c], row[c]) })
                    .collect();
                format!("{} {}", row[0], parts.join("; "))
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        let mut text_rows: Vec<String> = Vec::with_capacity(nr + 2);
        let header_in_source = first_bold || corner_empty;
        if !header_in_source {
            text_rows.push(format!("|{}", "  |".repeat(nc)));
            text_rows.push(format!("|{}", " --- |".repeat(nc)));
        }
        for (r, row) in text.iter().enumerate() {
            text_rows.push(format!("| {} |", row.join(" | ")));
            if r == 0 && header_in_source {
                text_rows.push(format!("|{}", " --- |".repeat(nc)));
            }
        }
        format!("\n{}\n", text_rows.join("\n"))
    };
    Ok(LayoutLine {
        x0: grid.xs[0],
        x1: grid.xs[nc],
        y0: grid.ys[0],
        y1: grid.ys[nr],
        baseline: grid.ys[0] + 0.1,
        size: median(sizes).max(1.0),
        text: body,
        glyphs,
        furniture: None,
        bold_lead: false,
        unplaced: 0,
        code: false,
        char_width: 0.0,
    })
}

// ── Emission ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WordClass {
    /// Prose (multi-letter words in a text font, labels, mark tags).
    Text,
    /// Mathematics (structures, math fonts, operators, italic variables).
    Math,
    /// Numbers, brackets, punctuation, single upright letters: follows context.
    Neutral,
    /// Function name (sin, cos, ln …).
    Func,
}

static PART_LABEL_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^\((?:[a-h]|i{1,3}|iv|vi{0,3}|ix|x)\)[.,:]?$").unwrap()
});
static MARK_TOKEN_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    // Includes AEA style-and-clarity allocations "(+S1)".
    regex::Regex::new(r"^(?:\[\d{1,2}\]|\(\d{1,2}\)|\(\+S\d\)|\[\d{1,2}|marks?\]|\[Total:?|\d{1,2}\])[.,]?$").unwrap()
});


fn node_glyphs<'a>(pg: &'a PageGlyphs, word: &[Node]) -> Vec<&'a Glyph> {
    word.iter()
        .filter_map(|n| match n {
            Node::G(i) => Some(&pg.glyphs[*i]),
            _ => None,
        })
        .collect()
}

fn word_string(pg: &PageGlyphs, word: &[Node]) -> String {
    node_glyphs(pg, word).iter().map(|g| g.ch).collect()
}

fn is_minus_like(c: char) -> bool {
    matches!(c, '−' | '–' | '‒' | '-')
}

fn classify_word(pg: &PageGlyphs, word: &[Node]) -> WordClass {
    if word.iter().any(|n| n.is_struct()) {
        return WordClass::Math;
    }
    let gl = node_glyphs(pg, word);
    let s: String = gl.iter().map(|g| g.ch).collect();
    if PART_LABEL_RE.is_match(&s) || MARK_TOKEN_RE.is_match(&s) {
        return WordClass::Text;
    }
    // A picture in the line stands alone.
    if s.contains(INLINE_IMAGE) {
        return WordClass::Text;
    }
    // A list bullet is prose punctuation whatever font draws it.
    if !s.is_empty() && s.chars().all(|c| matches!(c, '•' | '●' | '▪' | '■' | '◦')) {
        return WordClass::Text;
    }
    let letters: Vec<&&Glyph> = gl.iter().filter(|g| g.ch.is_alphabetic() && !is_greek(g.ch)).collect();
    let trimmed: String = s.trim_matches(|c: char| !c.is_alphanumeric()).to_string();
    if FUNCTION_NAMES.contains(&trimmed.as_str()) && letters.iter().all(|g| !g.italic) {
        return WordClass::Func;
    }
    if gl.iter().any(|g| is_greek(g.ch) || g.ch == UNKNOWN_GLYPH) {
        return WordClass::Math;
    }
    // Operators; a dash is a minus only when it binds to what follows.
    let chars: Vec<char> = s.chars().collect();
    for (k, &c) in chars.iter().enumerate() {
        if is_operator_char(c) {
            return WordClass::Math;
        }
        if is_minus_like(c) && c != '-' {
            // A dash joining two upright words ("east–west") is prose, and so
            // is an en dash between two numbers (the range "0–12").
            let joins_words = k > 0
                && k + 1 < chars.len()
                && (gl.get(k - 1).is_some_and(|g| g.ch.is_alphabetic() && !g.italic) && gl.get(k + 1).is_some_and(|g| g.ch.is_alphabetic() && !g.italic)
                    || c == '–' && gl.get(k - 1).is_some_and(|g| g.ch.is_ascii_digit()) && gl.get(k + 1).is_some_and(|g| g.ch.is_ascii_digit()));
            if !joins_words && chars.len() > 1 && k + 1 < chars.len() && (chars[k + 1].is_alphanumeric() || chars[k + 1] == '(') {
                return WordClass::Math;
            }
        }
    }
    // Upright letter runs that are all function names ("4sinx", "cos2x").
    {
        let mut runs: Vec<String> = Vec::new();
        let mut cur = String::new();
        for g in &gl {
            if g.ch.is_ascii_alphabetic() && !g.italic {
                cur.push(g.ch);
            } else if !cur.is_empty() {
                runs.push(std::mem::take(&mut cur));
            }
        }
        if !cur.is_empty() {
            runs.push(cur);
        }
        if !runs.is_empty() && runs.iter().all(|r| FUNCTION_NAMES.contains(&r.as_str())) {
            return WordClass::Math;
        }
    }
    // A plain number times a bold serif letter ("4k", "2i") is a vector
    // term.
    if letters.len() == 1
        && letters[0].bold
        && !is_sans_font(&letters[0].font)
        && gl.len() >= 2
        && gl.last().is_some_and(|g| g.ch == letters[0].ch)
        && gl[..gl.len() - 1].iter().all(|g| g.ch.is_ascii_digit() && !g.bold)
    {
        return WordClass::Math;
    }
    // (Punctuation or brackets around it do not change that: "i.", "j,",
    // "{I,".)
    // ("a)" alone is a part label.)
    let core = s.trim_matches(|c: char| matches!(c, '(' | ')' | '[' | ']' | '{' | '}' | ',' | '.' | ';' | ':'));
    let label = s.ends_with(')') && !s.contains('(');
    if letters.len() == 1 && letters[0].bold && core.chars().count() == 1 && !label {
        // A lone bold serif letter is a vector/matrix name; a bold sans
        // letter is a label in prose ("Section B", "capacitor C").
        if is_sans_font(&letters[0].font) {
            return WordClass::Neutral;
        }
        return WordClass::Math;
    }
    if letters.len() >= 2 {
        let all_italic = letters.iter().all(|g| g.italic);
        if all_italic {
            let has_vowel = letters.iter().any(|g| "aeiouyAEIOUY".contains(g.ch));
            let sans = letters.iter().any(|g| is_sans_font(&g.font));
            // Italic serif capitals name points ("triangle ABC", "angle
            // AOB"), whatever their vowels.
            if !sans && letters.iter().all(|g| g.ch.is_ascii_uppercase()) && gl.iter().all(|g| g.ch.is_ascii_uppercase() || matches!(g.ch, ',' | '.' | ';' | ':')) {
                return WordClass::Math;
            }
            if letters.len() >= 3 && has_vowel || sans && letters.len() >= 3 {
                return WordClass::Text;
            }
            return WordClass::Math;
        }
        // Mixed: a number glued to an italic variable ("2x") is maths.
        if letters.iter().any(|g| g.italic) && letters.iter().filter(|g| !g.italic).count() <= 1 {
            return WordClass::Math;
        }
        return WordClass::Text;
    }
    if letters.len() == 1 {
        let l = letters[0];
        if l.italic || is_math_font(&l.font) {
            return WordClass::Math;
        }
        return WordClass::Neutral;
    }
    if gl.iter().any(|g| is_math_font(&g.font) && !g.ch.is_ascii_digit() && !g.ch.is_ascii_punctuation()) {
        return WordClass::Math;
    }
    WordClass::Neutral
}

fn is_numeric_word(s: &str) -> bool {
    let t = s.trim_matches(|c: char| matches!(c, '(' | ')' | '[' | ']' | ',' | ';' | ':'));
    !t.is_empty() && t.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ',' || c == '°') && t.chars().any(|c| c.is_ascii_digit())
}

/// Emit a node sequence. `in_math` = the caller is already inside math mode
/// (a numerator, script, radicand …): prose words become `\text{…}`.
fn emit_nodes(pg: &PageGlyphs, nodes: &[Node], in_math: bool) -> String {
    // Split into words separated by Space / Gap.
    let mut words: Vec<(Vec<Node>, Option<Node>)> = Vec::new(); // (word, following separator)
    let mut cur: Vec<Node> = Vec::new();
    for n in nodes {
        match n {
            Node::Space | Node::WideSpace | Node::Gap => {
                if !cur.is_empty() {
                    words.push((std::mem::take(&mut cur), Some(n.clone())));
                } else if let Some(last) = words.last_mut() {
                    if matches!(n, Node::Gap) || matches!(n, Node::WideSpace) && !matches!(last.1, Some(Node::Gap)) {
                        last.1 = Some(n.clone());
                    }
                }
            }
            _ => cur.push(n.clone()),
        }
    }
    if !cur.is_empty() {
        words.push((cur, None));
    }
    if words.is_empty() {
        return String::new();
    }
    let mut classes: Vec<WordClass> = words.iter().map(|(w, _)| classify_word(pg, w)).collect();
    let mut texts: Vec<String> = words.iter().map(|(w, _)| word_string(pg, w)).collect();
    // An italic letter hyphenated to a prose word ("the x-axis", "the
    // y-coordinate"): the letter stays maths inside the prose word (marked
    // here, set as maths when the prose is written).
    for (k, (w, _)) in words.iter().enumerate() {
        let gl = node_glyphs(pg, w);
        if classes[k] != WordClass::Text || gl.len() < 4 || !w.iter().all(|n| matches!(n, Node::G(_))) {
            continue;
        }
        let head = gl.iter().take_while(|g| g.italic && g.ch.is_ascii_alphabetic()).count();
        let rest = &gl[head..];
        if head == 1 && matches!(rest[0].ch, '-' | '‐' | '‑' | '–') && rest.len() >= 3 && rest[1..].iter().all(|g| !g.italic && (g.ch.is_ascii_alphabetic() || matches!(g.ch, ',' | '.' | ';' | ':' | ')'))) {
            texts[k] = format!("{MATH_OPEN}{}{MATH_CLOSE}-{}", gl[0].ch, rest[1..].iter().map(|g| g.ch).collect::<String>());
        }
    }
    // Enlarged brackets (Symbol parentheses) around prose are punctuation,
    // not a delimited formula: a mark allocation "(10)" around bold digits,
    // a part label "(a)", a footer "(Total for Question 6 is 4 marks)".
    for (k, (w, _)) in words.iter().enumerate() {
        if let [Node::Delim('(', ')', inner)] = w.as_slice() {
            if let Some(t) = bracketed_prose(pg, inner) {
                classes[k] = WordClass::Text;
                texts[k] = format!("({})", t);
            }
        }
    }
    // Italic prose keeps its short words: a two-letter italic word with a
    // vowel next to an italic prose word ("(There is no need to estimate
    // …)") is a word, not a product of two variables.
    if !in_math {
        // An italic word: italic ASCII letters, with at most bracket or
        // punctuation marks at its ends ("[In", "need,").
        let italic_letters = |k: usize| -> Option<usize> {
            let gl = node_glyphs(pg, &words[k].0);
            if gl.is_empty() || !words[k].0.iter().all(|n| matches!(n, Node::G(_))) {
                return None;
            }
            let first = gl.iter().position(|g| g.ch.is_alphabetic())?;
            let last = gl.iter().rposition(|g| g.ch.is_alphabetic())?;
            let core = &gl[first..=last];
            let ends_ok = gl[..first].iter().chain(&gl[last + 1..]).all(|g| matches!(g.ch, '(' | '[' | ')' | ']' | ',' | '.' | ';' | ':'));
            (ends_ok && core.iter().all(|g| g.italic && g.ch.is_ascii_alphabetic())).then_some(core.len())
        };
        loop {
            let mut changed = false;
            for k in 0..words.len() {
                let Some(n) = italic_letters(k) else { continue };
                if classes[k] != WordClass::Math {
                    continue;
                }
                let prose = |j: usize| italic_letters(j).is_some() && classes[j] == WordClass::Text;
                let before = k > 0 && prose(k - 1);
                let after = k + 1 < words.len() && prose(k + 1);
                let letters: String = texts[k].chars().filter(|c| c.is_alphabetic()).collect();
                let short_word = n == 2 && letters.chars().any(|c| "aeiouyAEIOUY".contains(c)) && (before || after);
                let article = n == 1 && matches!(letters.as_str(), "a" | "A" | "I") && before && after;
                if short_word || article {
                    classes[k] = WordClass::Text;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        // The italic tail of an italic sentence ("… network is 1960 m]") is
        // prose too, numbers and units included.
        let italic_tail = |k: usize| {
            let gl = node_glyphs(pg, &words[k].0);
            !gl.is_empty() && words[k].0.iter().all(|n| matches!(n, Node::G(_))) && gl.iter().all(|g| g.italic || matches!(g.ch, '(' | '[' | ')' | ']' | ',' | '.' | ';' | ':'))
        };
        if let Some(start) = (1..words.len()).find(|&k| (k..words.len()).all(|j| italic_tail(j)) && italic_letters(k - 1).is_some() && classes[k - 1] == WordClass::Text) {
            for c in classes.iter_mut().skip(start) {
                if *c == WordClass::Math {
                    *c = WordClass::Text;
                }
            }
        }
    }
    // Monospace words are code: verbatim, in an inline code span, never
    // maths ("the instruction `ADD R3, R3, R0`").
    if !in_math {
        // (Prose punctuation set after a code word, "register `d`.", is not
        // part of the code.)
        let code: Vec<Option<usize>> = words
            .iter()
            .map(|(w, _)| {
                let gl = node_glyphs(pg, w);
                let tail = gl.iter().rev().take_while(|g| !g.mono && matches!(g.ch, '.' | ',' | ';' | ':' | '?' | '!' | ')')).count();
                let core = &gl[..gl.len() - tail];
                (!core.is_empty() && w.iter().all(|n| matches!(n, Node::G(_))) && core.iter().all(|g| g.mono)).then_some(tail)
            })
            .collect();
        let mut k = 0;
        while k < words.len() {
            if code[k].is_none() {
                k += 1;
                continue;
            }
            let mut j = k;
            while j + 1 < words.len() && code[j] == Some(0) && code[j + 1].is_some() && !matches!(words[j].1, Some(Node::Gap)) {
                j += 1;
            }
            for m in k..=j {
                classes[m] = WordClass::Text;
            }
            texts[k] = format!("`{}", texts[k]);
            let tail = code[j].unwrap_or(0);
            let cut = texts[j].char_indices().rev().nth(tail.saturating_sub(1)).map(|(i, _)| i).filter(|_| tail > 0).unwrap_or(texts[j].len());
            texts[j] = format!("{}`{}", &texts[j][..cut], &texts[j][cut..]);
            k = j + 1;
        }
    }
    // Line-leading labels are prose: a bold question number ("1 9", "5",
    // "12."), part labels, and a bold MCQ option letter ("A an attractive
    // force …", "A 10^20") never join the maths that follows them.
    if !in_math {
        let is_bold_glyphs = |w: &[Node]| w.iter().all(|n| matches!(n, Node::G(i) if pg.glyphs[*i].bold));
        let bold_number = |k: usize| {
            let t = texts[k].trim_end_matches('.');
            !t.is_empty() && t.len() <= 2 && t.chars().all(|c| c.is_ascii_digit()) && is_bold_glyphs(&words[k].0)
        };
        let mut k = 0;
        while k < words.len() {
            if bold_number(k) {
                classes[k] = WordClass::Text;
                k += 1;
            } else if k > 0 && texts[k] == "." && k + 1 < words.len() && bold_number(k + 1) {
                // The boxed AQA part number after the question number
                // ("0 1 . 2").
                classes[k] = WordClass::Text;
                classes[k + 1] = WordClass::Text;
                k += 2;
            } else {
                break;
            }
        }
        while k < words.len() && PART_LABEL_RE.is_match(&texts[k]) {
            k += 1;
        }
        if k + 1 < words.len() {
            let t = texts[k].as_str();
            let next_is_op = texts[k + 1].chars().next().map(|c| is_operator_char(c) || c == '=').unwrap_or(false);
            if t.len() == 1 && matches!(t, "A" | "B" | "C" | "D" | "E") && is_bold_glyphs(&words[k].0) && !next_is_op {
                classes[k] = WordClass::Text;
            }
        }
    }
    // A standalone dash between maths is a minus sign.
    for k in 0..words.len() {
        let t = texts[k].as_str();
        if t.chars().count() == 1 && t.chars().all(is_minus_like) && !words[k].0.iter().any(|n| n.is_struct()) {
            let prev_m = k > 0 && matches!(classes[k - 1], WordClass::Math | WordClass::Neutral) && !matches!(words[k - 1].1, Some(Node::Gap));
            let next_m = k + 1 < words.len() && matches!(classes[k + 1], WordClass::Math | WordClass::Neutral | WordClass::Func);
            // Inside a script or fraction part a dash is always a minus.
            if prev_m && next_m || in_math {
                classes[k] = WordClass::Math;
            } else {
                classes[k] = WordClass::Text;
            }
        }
    }
    // Math runs: maximal Math/Neutral/Func sequences containing Math or Func,
    // not crossing a Gap; neutral edge words that are not numeric are trimmed.
    let mut in_run = vec![false; words.len()];
    if in_math {
        for (k, c) in classes.iter().enumerate() {
            in_run[k] = *c != WordClass::Text;
        }
    }
    let mut k = if in_math { words.len() } else { 0 };
    while k < words.len() {
        if classes[k] == WordClass::Text {
            k += 1;
            continue;
        }
        let mut j = k;
        while j < words.len() && classes[j] != WordClass::Text {
            let gap_after = matches!(words[j].1, Some(Node::Gap));
            j += 1;
            if gap_after {
                break;
            }
        }
        // [k, j) is a candidate run.
        let has_math = (k..j).any(|m| matches!(classes[m], WordClass::Math | WordClass::Func));
        if has_math {
            let mut a = k;
            let mut b = j;
            let keep_edge = |m: usize| -> bool {
                matches!(classes[m], WordClass::Math | WordClass::Func)
                    || is_numeric_word(&texts[m])
                    || texts[m].chars().any(|c| matches!(c, '(' | ')' | '[' | ']' | '|' | '‖'))
            };
            // An operator binds the word on its other side ("2 − 3i", "−4 + 4i"),
            // and an upright i after maths is its imaginary unit ("…/9 i").
            let binary = |c: char| matches!(c, '+' | '-' | '−' | '×' | '÷' | '=' | '<' | '>' | '≤' | '≥' | '⩽' | '⩾' | '±');
            // (A bold sans letter is a label in prose: "from S to A = …".)
            let sans_label = |m: usize| {
                let gl = node_glyphs(pg, &words[m].0);
                gl.len() == 1 && gl[0].bold && is_sans_font(&gl[0].font)
            };
            let operand_after = |m: usize| m > k && !sans_label(m) && texts[m - 1].trim_end().chars().last().is_some_and(binary);
            let operand_before = |m: usize| m + 1 < j && !sans_label(m) && texts[m + 1].trim_start().chars().next().is_some_and(binary);
            let unit_i = |m: usize| m > k && texts[m].trim_end_matches(|c: char| matches!(c, '.' | ',' | ';' | ':')) == "i" && matches!(classes[m - 1], WordClass::Math);
            while a < b && !keep_edge(a) && !operand_before(a) {
                a += 1;
            }
            while b > a && !keep_edge(b - 1) && !operand_after(b - 1) && !unit_i(b - 1) {
                b -= 1;
            }
            for m in a..b {
                in_run[m] = true;
            }
        }
        k = j.max(k + 1);
    }
    let mut out = String::new();
    let mut m = 0;
    while m < words.len() {
        if in_run[m] {
            let mut j = m;
            let mut latex = String::new();
            while j < words.len() && in_run[j] {
                let piece = latex_word(pg, &words[j].0);
                if j > m {
                    if matches!(words[j - 1].1, Some(Node::WideSpace)) {
                        latex.push_str("\\quad ");
                    } else if DIFFERENTIAL_RE.is_match(&piece) && !latex.trim_end().ends_with(['=', '+', '-', '(', '{']) {
                        // The integrand and its differential are set apart
                        // ("ln x dx", not "ln xdx").
                        latex.push_str(" \\, ");
                    } else {
                        latex.push(' ');
                    }
                }
                latex.push_str(&piece);
                let gap_after = matches!(words[j].1, Some(Node::Gap));
                j += 1;
                if gap_after {
                    break;
                }
            }
            // Sentence punctuation after the maths stays outside it.
            let mut trail = String::new();
            while latex.ends_with('.') || latex.ends_with(',') || latex.ends_with(';') || latex.ends_with(':') || latex.ends_with('?') {
                let c = latex.pop().unwrap();
                trail.insert(0, c);
            }
            let latex = latex.trim().to_string();
            if !out.is_empty() && !out.ends_with(' ') && !out.ends_with('\t') {
                out.push(' ');
            }
            if in_math {
                out.push_str(&latex);
            } else if !latex.is_empty() {
                out.push('$');
                out.push_str(&latex);
                out.push('$');
            }
            out.push_str(&trail);
            if j < words.len() {
                out.push(if matches!(words[j - 1].1, Some(Node::Gap)) { '\t' } else { ' ' });
            }
            m = j;
        } else {
            // Text run.
            let mut j = m;
            let mut text = String::new();
            while j < words.len() && !in_run[j] {
                text.push_str(&texts[j]);
                if j + 1 < words.len() && !in_run[j + 1] {
                    text.push(if matches!(words[j].1, Some(Node::Gap)) { '\t' } else { ' ' });
                }
                j += 1;
            }
            if in_math {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
                // Maths mode ignores spaces outside the text: the words keep
                // theirs inside it ("\text{for }1 < x").
                let lead = if out.is_empty() { "" } else { " " };
                let tail = if j < words.len() { " " } else { "" };
                let text = text.replace([MATH_OPEN, MATH_CLOSE], "");
                out.push_str(&format!("\\text{{{lead}{}{tail}}}", escape_text_in_math(&text)));
            } else {
                out.push_str(&escape_prose(&text).replace([MATH_OPEN, MATH_CLOSE], "$"));
            }
            if j < words.len() {
                out.push(if matches!(words[j - 1].1, Some(Node::Gap)) { '\t' } else { ' ' });
            }
            m = j;
        }
    }
    out.trim_end().to_string()
}

/// The plain text of a bracketed group when it is prose rather than maths:
/// bold digits (a mark allocation), an upright part label, or at least two
/// upright words. Any structure (fraction, script, radical) keeps it maths.
fn bracketed_prose(pg: &PageGlyphs, inner: &[Node]) -> Option<String> {
    let mut text = String::new();
    for n in inner {
        match n {
            Node::G(i) => text.push(pg.glyphs[*i].ch),
            Node::Space | Node::WideSpace | Node::Gap => text.push(' '),
            _ => return None,
        }
    }
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let glyphs = node_glyphs(pg, inner);
    if glyphs.is_empty() {
        return None;
    }
    let marks = glyphs.len() <= 3 && glyphs.iter().all(|g| g.ch.is_ascii_digit() && g.bold);
    let label = PART_LABEL_RE.is_match(&format!("({})", text)) && glyphs.iter().all(|g| !g.italic);
    let words = text
        .split(' ')
        .filter(|w| w.chars().filter(|c| c.is_alphabetic()).count() >= 2)
        .filter(|w| node_glyphs_for_word(pg, inner, w).iter().all(|g| !g.italic))
        .count();
    (marks || label || words >= 2).then_some(text)
}

/// Glyphs of the first occurrence of `word` among a node list's glyphs.
fn node_glyphs_for_word<'a>(pg: &'a PageGlyphs, nodes: &[Node], word: &str) -> Vec<&'a Glyph> {
    let gl = node_glyphs(pg, nodes);
    let chars: Vec<char> = word.chars().collect();
    (0..gl.len())
        .find(|&s| s + chars.len() <= gl.len() && (0..chars.len()).all(|k| gl[s + k].ch == chars[k]))
        .map(|s| gl[s..s + chars.len()].to_vec())
        .unwrap_or_default()
}

fn escape_text_in_math(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\backslash "),
            '{' => o.push_str("\\{"),
            '}' => o.push_str("\\}"),
            '$' => o.push_str("\\$"),
            '%' => o.push_str("\\%"),
            '&' => o.push_str("\\&"),
            '#' => o.push_str("\\#"),
            '_' => o.push_str("\\_"),
            '\t' => o.push(' '),
            _ => o.push(c),
        }
    }
    o
}

/// A differential standing as its own word: "\mathrm{d}x", "\mathrm{d}\theta".
static DIFFERENTIAL_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"^\\mathrm\{d\}(?:[A-Za-z]|\\[a-z]+)$").unwrap());

/// Marks around a maths letter inside a prose word (see [`emit_nodes`]):
/// Unicode noncharacters, which no source text contains.
const MATH_OPEN: char = '\u{FDD0}';
const MATH_CLOSE: char = '\u{FDD1}';

fn escape_prose(s: &str) -> String {
    s.replace('$', "\\$")
}

/// LaTeX for one word (a sequence of nodes without spaces) in math mode.
fn latex_word(pg: &PageGlyphs, word: &[Node]) -> String {
    let mut out = String::new();
    let mut k = 0;
    while k < word.len() {
        // Group consecutive upright letters into one token (function names,
        // units, differentials).
        if let Node::G(i) = word[k] {
            let g = &pg.glyphs[i];
            if g.ch.is_ascii_alphabetic() && !g.italic && !g.bold && !is_math_font(&g.font) {
                let mut j = k;
                let mut run = String::new();
                let mut scripts: Option<(&Option<Vec<Node>>, &Option<Vec<Node>>)> = None;
                while j < word.len() {
                    match &word[j] {
                        Node::G(h) if pg.glyphs[*h].ch.is_ascii_alphabetic() && !pg.glyphs[*h].italic && !pg.glyphs[*h].bold => {
                            run.push(pg.glyphs[*h].ch);
                            j += 1;
                        }
                        Node::Scripted { base, sub, sup } => {
                            if let Node::G(h) = base.as_ref() {
                                let bg = &pg.glyphs[*h];
                                if bg.ch.is_ascii_alphabetic() && !bg.italic && !bg.bold {
                                    run.push(bg.ch);
                                    scripts = Some((sub, sup));
                                    j += 1;
                                }
                            }
                            break;
                        }
                        _ => break,
                    }
                }
                let mut piece = upright_run(&run);
                if let Some((sub, sup)) = scripts {
                    piece.push_str(&script_suffix(pg, sub, sup));
                }
                push_latex(&mut out, &piece);
                k = j;
                continue;
            }
        }
        let piece = latex_node(pg, &word[k]);
        push_latex(&mut out, &piece);
        k += 1;
    }
    out
}

/// Append a LaTeX fragment, separating a control word from a following letter.
fn push_latex(out: &mut String, piece: &str) {
    if piece.is_empty() {
        return;
    }
    let ends_control_word = {
        let bytes = out.as_bytes();
        let mut i = bytes.len();
        while i > 0 && bytes[i - 1].is_ascii_alphabetic() {
            i -= 1;
        }
        i > 0 && i < bytes.len() && bytes[i - 1] == b'\\'
    };
    if ends_control_word && piece.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false) {
        out.push(' ');
    }
    out.push_str(piece);
}

fn script_suffix(pg: &PageGlyphs, sub: &Option<Vec<Node>>, sup: &Option<Vec<Node>>) -> String {
    let mut s = String::new();
    if let Some(sb) = sub {
        let t = emit_nodes(pg, sb, true);
        // A one-letter subscript label ("m_e", "V_C") is written bare, as is
        // conventional; its upright style carries no meaning of its own.
        let t = match t.strip_prefix("\\mathrm{").and_then(|r| r.strip_suffix('}')) {
            Some(l) if l.chars().count() == 1 && l.chars().all(|c| c.is_ascii_alphabetic()) => l.to_string(),
            _ => t,
        };
        if t.chars().count() == 1 && t.chars().all(|c| c.is_ascii_alphanumeric()) {
            s.push('_');
            s.push_str(&t);
        } else {
            s.push_str(&format!("_{{{}}}", t));
        }
    }
    if let Some(sp) = sup {
        s.push_str(&format!("^{{{}}}", emit_nodes(pg, sp, true)));
    }
    s
}

fn upright_run(run: &str) -> String {
    if FUNCTION_NAMES.contains(&run) {
        return match run {
            "cosec" | "sech" | "cosech" | "arsinh" | "arcosh" | "artanh" => format!("\\operatorname{{{}}}", run),
            _ => format!("\\{}", run),
        };
    }
    if run.chars().count() == 1 {
        return format!("\\mathrm{{{}}}", run);
    }
    format!("\\text{{{}}}", run)
}

fn latex_node(pg: &PageGlyphs, n: &Node) -> String {
    match n {
        Node::G(i) => latex_glyph(&pg.glyphs[*i]),
        Node::Space => " ".to_string(),
        Node::WideSpace => "\\quad ".to_string(),
        Node::Gap => "\\quad ".to_string(),
        Node::Frac(a, b) => format!("\\frac{{{}}}{{{}}}", emit_nodes(pg, a, true), emit_nodes(pg, b, true)),
        Node::Sqrt(idx, body) => match idx {
            Some(ix) => format!("\\sqrt[{}]{{{}}}", emit_nodes(pg, ix, true), emit_nodes(pg, body, true)),
            None => format!("\\sqrt{{{}}}", emit_nodes(pg, body, true)),
        },
        Node::Delim(l, r, body) => format!("\\left{}{}\\right{}", delim_latex(*l), emit_nodes(pg, body, true), delim_latex(*r)),
        Node::Matrix(l, r, rows) => {
            let env = match (l, r) {
                ('(', ')') => "pmatrix",
                ('[', ']') => "bmatrix",
                ('|', '|') => "vmatrix",
                ('{', '}') => "Bmatrix",
                _ => "matrix",
            };
            let body = rows
                .iter()
                .map(|row| row.iter().map(|c| emit_nodes(pg, c, true)).collect::<Vec<_>>().join(" & "))
                .collect::<Vec<_>>()
                .join(" \\\\ ");
            if env == "matrix" {
                format!("\\left{}\\begin{{matrix}}{}\\end{{matrix}}\\right{}", delim_latex(*l), body, delim_latex(*r))
            } else {
                format!("\\begin{{{env}}}{body}\\end{{{env}}}")
            }
        }
        Node::Cases(rows) => {
            let body = rows
                .iter()
                .map(|r| {
                    // A wide gap inside a row separates the condition column.
                    let mut parts: Vec<Vec<Node>> = vec![Vec::new()];
                    for n in r {
                        if matches!(n, Node::Gap) {
                            parts.push(Vec::new());
                        } else {
                            parts.last_mut().unwrap().push(n.clone());
                        }
                    }
                    parts.iter().filter(|p| !p.is_empty()).map(|p| emit_nodes(pg, p, true)).collect::<Vec<_>>().join(" & ")
                })
                .collect::<Vec<_>>()
                .join(" \\\\ ");
            format!("\\begin{{cases}}{}\\end{{cases}}", body)
        }
        Node::Scripted { base, sub, sup } => {
            let mut s = match base.as_ref() {
                Node::G(i) => {
                    let g = &pg.glyphs[*i];
                    if g.ch.is_ascii_alphabetic() && !g.italic && !g.bold && !is_math_font(&g.font) {
                        format!("\\mathrm{{{}}}", g.ch)
                    } else {
                        latex_glyph(g)
                    }
                }
                other => latex_node(pg, other),
            };
            s.push_str(&script_suffix(pg, sub, sup));
            s
        }
        Node::Pre { sub, sup } => {
            let mut s = String::from("{}");
            if let Some(sp) = sup {
                s.push_str(&format!("^{{{}}}", emit_nodes(pg, sp, true)));
            }
            if let Some(sb) = sub {
                s.push_str(&format!("_{{{}}}", emit_nodes(pg, sb, true)));
            }
            s
        }
        Node::BigOp { op, lower, upper } => {
            let mut s = latex_glyph(&pg.glyphs[*op]);
            if let Some(lo) = lower {
                s.push_str(&format!("_{{{}}}", emit_nodes(pg, lo, true)));
            }
            if let Some(up) = upper {
                s.push_str(&format!("^{{{}}}", emit_nodes(pg, up, true)));
            }
            s
        }
        Node::Accent(kind, body) => {
            let cmd = match kind {
                AccentKind::Bar => "bar",
                AccentKind::Overline => "overline",
                AccentKind::Underline => "underline",
                AccentKind::Hat => "hat",
                AccentKind::Tilde => "tilde",
                AccentKind::Dot => "dot",
                AccentKind::DDot => "ddot",
                AccentKind::Vec => "vec",
                AccentKind::OverArrow => "overrightarrow",
            };
            format!("\\{}{{{}}}", cmd, emit_nodes(pg, body, true))
        }
    }
}

fn delim_latex(c: char) -> &'static str {
    match c {
        '(' => "(",
        ')' => ")",
        '[' => "[",
        ']' => "]",
        '{' => "\\{",
        '}' => "\\}",
        '|' => "|",
        '‖' => "\\|",
        '〈' | '⟨' => "\\langle",
        '〉' | '⟩' => "\\rangle",
        _ => ".",
    }
}

fn latex_glyph(g: &Glyph) -> String {
    let c = g.ch;
    if c.is_ascii_alphabetic() {
        return if g.bold && g.italic {
            format!("\\boldsymbol{{{}}}", c)
        } else if g.bold {
            format!("\\mathbf{{{}}}", c)
        } else {
            c.to_string()
        };
    }
    if c.is_ascii_digit() {
        return if g.bold { format!("\\mathbf{{{}}}", c) } else { c.to_string() };
    }
    let s: &str = match c {
        '−' | '–' | '‒' | '-' => "-",
        '×' => "\\times",
        '÷' => "\\div",
        '±' => "\\pm",
        '∓' => "\\mp",
        '≤' => "\\leq",
        '≥' => "\\geq",
        '⩽' => "\\leqslant",
        '⩾' => "\\geqslant",
        '≠' => "\\neq",
        '≈' => "\\approx",
        '≡' => "\\equiv",
        '∝' => "\\propto",
        '∼' | '~' => "\\sim",
        '≅' => "\\cong",
        '≃' => "\\simeq",
        '→' => "\\rightarrow",
        '←' => "\\leftarrow",
        '↔' => "\\leftrightarrow",
        '⇒' | '⟹' => "\\Rightarrow",
        '⇐' => "\\Leftarrow",
        '⇔' | '⟺' => "\\Leftrightarrow",
        '↦' => "\\mapsto",
        '∞' => "\\infty",
        '∑' => "\\sum",
        '∏' => "\\prod",
        '∫' | '⌠' | '⌡' | '⎮' => "\\int",
        // A bracket extension piece standing alone (never assembled with a
        // top and bottom) is drawn as a plain vertical bar ("3 ⎜ 0" key).
        '⎜' | '⎟' | '⎢' | '⎥' | '⎪' => "|",
        '∮' => "\\oint",
        '∂' => "\\partial",
        '∇' => "\\nabla",
        '∈' => "\\in",
        '∉' => "\\notin",
        '⊂' => "\\subset",
        '⊆' => "\\subseteq",
        '∪' => "\\cup",
        '∩' => "\\cap",
        '∅' => "\\emptyset",
        '∠' => "\\angle",
        '°' => "^{\\circ}",
        '′' => "'",
        '″' => "''",
        '·' | '⋅' | '∙' => "\\cdot",
        '∘' => "\\circ",
        '∴' => "\\therefore",
        '∵' => "\\because",
        '⊥' => "\\perp",
        '∥' => "\\parallel",
        '∀' => "\\forall",
        '∃' => "\\exists",
        '⊗' => "\\otimes",
        '⊕' => "\\oplus",
        '≪' => "\\ll",
        '≫' => "\\gg",
        '‖' => "\\|",
        '%' => "\\%",
        '#' => "\\#",
        '&' => "\\&",
        '$' => "\\$",
        '_' => "\\_",
        '{' => "\\{",
        '}' => "\\}",
        '…' | '⋯' => "\\ldots",
        '√' => "\\sqrt{}",
        'ℝ' => "\\mathbb{R}",
        'ℍ' => "\\mathbb{H}",
        'ℙ' => "\\mathbb{P}",
        'ℕ' => "\\mathbb{N}",
        'ℤ' => "\\mathbb{Z}",
        'ℚ' => "\\mathbb{Q}",
        'ℂ' => "\\mathbb{C}",
        '∗' => "*",
        '⁄' => "/",
        '\u{203E}' => "",
        'α' => "\\alpha",
        'β' => "\\beta",
        'γ' => "\\gamma",
        'δ' => "\\delta",
        'ε' | 'ϵ' => "\\varepsilon",
        'ζ' => "\\zeta",
        'η' => "\\eta",
        'θ' => "\\theta",
        'ϑ' => "\\vartheta",
        'ι' => "\\iota",
        'κ' => "\\kappa",
        'λ' => "\\lambda",
        'μ' | 'µ' => "\\mu",
        'ν' => "\\nu",
        'ξ' => "\\xi",
        'ο' => "o",
        'π' => "\\pi",
        'ϖ' => "\\varpi",
        'ρ' => "\\rho",
        'σ' => "\\sigma",
        'ς' => "\\varsigma",
        'τ' => "\\tau",
        'υ' => "\\upsilon",
        'φ' => "\\phi",
        'ϕ' => "\\phi",
        'χ' => "\\chi",
        'ψ' => "\\psi",
        'ω' => "\\omega",
        'Γ' => "\\Gamma",
        'Δ' => "\\Delta",
        'Θ' => "\\Theta",
        'Λ' => "\\Lambda",
        'Ξ' => "\\Xi",
        'Π' => "\\Pi",
        'Σ' => "\\Sigma",
        'Υ' => "\\Upsilon",
        'Φ' => "\\Phi",
        'Ψ' => "\\Psi",
        'Ω' => "\\Omega",
        'Α' => "A",
        'Β' => "B",
        'Ε' => "E",
        'Ζ' => "Z",
        'Η' => "H",
        'Ι' => "I",
        'Κ' => "K",
        'Μ' => "M",
        'Ν' => "N",
        'Ο' => "O",
        'Ρ' => "P",
        'Τ' => "T",
        'Χ' => "X",
        _ => "",
    };
    if !s.is_empty() {
        return s.to_string();
    }
    if c == UNKNOWN_GLYPH {
        return c.to_string();
    }
    if c.is_ascii() {
        return c.to_string();
    }
    format!("\\text{{{}}}", c)
}

// ── Question map from geometric headings ────────────────────────────────────
//
// A question heading is a question number printed in the page's heading
// column (the left margin): "1." (Edexcel), "1" (CIE, GCSE), boxed "0 1"
// (AQA), "Question 1" (Madas). The column is measured per document from the
// candidates themselves, so body lines that happen to start with a number
// (stem-and-leaf rows, "3 significant figures.") never qualify. A question
// owns every body line from its heading to the next question's heading (or
// an end-of-paper marker) — exact line boundaries, no page-fraction proxies.

/// One question located on the layout pages.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutQuestion {
    pub number: u32,
    /// Body lines in reading order as (page, line index in `LayoutPage::lines`).
    pub lines: Vec<(usize, usize)>,
    pub start_page: usize,
    pub end_page: usize,
    /// Heading line top (points) on the start page.
    pub start_y: f32,
    /// Top of the next question's heading when it shares the end page.
    pub end_y: Option<f32>,
    /// Index in `lines` where reference material printed elsewhere for
    /// this question (a data table "included so that you can answer …")
    /// was appended: the body ends with that material, complete as printed.
    pub reference_from: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayoutMap {
    pub questions: Vec<LayoutQuestion>,
    pub anomalies: Vec<String>,
}

/// Parsed leading question-number token of a line.
#[derive(Debug, Clone, Copy, PartialEq)]
struct HeadToken {
    number: u32,
    /// AQA decimal part ("0 1 . 2" → 2) or a lettered part follows.
    has_part: bool,
    /// Zero-padded two-box AQA form ("0 1").
    zero_padded: bool,
    /// Two single digits in separate boxes ("1 0").
    spaced_pair: bool,
    /// Nothing but the number on the line.
    alone: bool,
}

static HEAD_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(concat!(
        r"^(?:(?P<qword>Question|QUESTION|Q)\s*)?",
        r"(?P<num>\d(?:[ \t]\d)?|\d{1,2})",
        r"(?P<dot>\.)?",
        r"(?P<part>[ \t]*\.[ \t]*\d{1,2}|[ \t]+\((?:[a-h]|i{1,3}|iv|vi{0,3})\))?",
        r"(?P<rest>[ \t].*|\(.*)?$"
    ))
    .unwrap()
});

static END_MARKER_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)^\**\s*(?:end of (?:questions?|paper|examination|test)|total for paper is\s*\d+\s*marks?|total marks for (?:this )?paper\s*[:=]?\s*\d+)\s*\**\.?$").unwrap()
});

/// A section boundary: lines from here to the next question heading (section
/// title, "Answer all questions in this section", response instructions)
/// belong to no question.
static SECTION_BREAK_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)^\**\s*(?:end of section\s+[a-z0-9]{1,2}|section\s+[a-z0-9]{1,2})\s*\**\.?$").unwrap()
});

/// "This table is included so that you can answer Questions 09.1 and 09.2".
/// "Question parts 06.1 and 06.2 use …", "Questions 05.3, 05.4 and 05.5 use …".
static LEAD_IN_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)^(?:for\s+)?questions?(?:\s+parts?)?\s+0?(\d{1,2})\s*\.\s*\d").unwrap());

static REFERENCE_FOR_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)\bthis (?:table|figure|page|insert|information|data)\b.{0,40}?\bincluded\b.{0,40}?\banswer\b.{0,20}?\bquestions?(?:\s+parts?)?\s+0?(\d{1,2})").unwrap()
});

static TRAILING_PAGE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)^(?:additional page|additional answer space|blank page|there are no questions printed on this page|copyright information|question number|for examiner.?s use only)").unwrap()
});

fn plain_line(t: &str) -> String {
    t.replace('$', "").replace("\\mathbf{", "").replace('}', "").trim().to_string()
}

fn head_token(line: &LayoutLine) -> Option<HeadToken> {
    let t = plain_line(&line.text);
    let caps = HEAD_RE.captures(&t)?;
    let num_s = caps.name("num")?.as_str();
    let digits: String = num_s.chars().filter(|c| c.is_ascii_digit()).collect();
    let spaced_pair = num_s.contains([' ', '\t']);
    let zero_padded = spaced_pair && digits.starts_with('0');
    let number: u32 = digits.parse().ok()?;
    if number == 0 || number > 99 {
        return None;
    }
    let rest = caps.name("rest").map(|m| m.as_str().trim()).unwrap_or("");
    // "3 significant figures", "1 hour 45 minutes": a bare number followed by
    // lowercase prose is a quantity, not a heading (the column check is the
    // primary guard; this only removes the obvious cases).
    if caps.name("qword").is_none() && caps.name("dot").is_none() && caps.name("part").is_none() {
        if let Some(c) = rest.chars().next() {
            if c.is_ascii_lowercase() && !rest.starts_with("(") && rest.split_whitespace().next().map(|w| w.len() > 1).unwrap_or(false) {
                return None;
            }
        }
    }
    Some(HeadToken {
        number,
        has_part: caps.name("part").is_some() || rest.starts_with('('),
        zero_padded,
        spaced_pair,
        alone: rest.is_empty(),
    })
}

/// Build the question map from laid-out pages.
pub fn build_layout_map(pages: &[LayoutPage]) -> LayoutMap {
    let mut map = LayoutMap::default();
    // 1. Candidates with their x position.
    let mut cands: Vec<(usize, usize, HeadToken, f32, bool)> = Vec::new();
    for (pi, page) in pages.iter().enumerate() {
        for (li, line) in page.lines.iter().enumerate() {
            if let Some(h) = head_token(line) {
                cands.push((pi, li, h, line.x0, line.bold_lead));
            }
        }
    }
    if cands.is_empty() {
        map.anomalies.push("layout map: no question-number candidates".to_string());
        return map;
    }
    // 2. Heading column: numbers printed in the left margin, i.e. clearly left
    // of where body text lines start on the same document (the modal line
    // start). Right-aligned two-digit numbers ("22" vs "5") both qualify.
    // "Question N" headings are recognised by their wording instead.
    let mut starts: std::collections::HashMap<i32, usize> = std::collections::HashMap::new();
    for page in pages {
        for line in &page.lines {
            *starts.entry((line.x0 / 2.0).round() as i32).or_default() += 1;
        }
    }
    let body_x = starts.iter().max_by_key(|(_, n)| **n).map(|(k, _)| *k as f32 * 2.0).unwrap_or(0.0);
    let min_cand = cands.iter().map(|c| c.3).fold(f32::MAX, f32::min);
    let margin_limit = if body_x > min_cand + 12.0 { body_x - 8.0 } else { min_cand + 4.0 };
    let in_col: Vec<&(usize, usize, HeadToken, f32, bool)> = cands
        .iter()
        .filter(|c| c.3 <= margin_limit || plain_line(&pages[c.0].lines[c.1].text).to_ascii_lowercase().starts_with("question"))
        .collect();
    if in_col.is_empty() {
        map.anomalies.push("layout map: no heading column".to_string());
        return map;
    }
    let bold_share = in_col.iter().filter(|c| c.4).count() as f32 / in_col.len().max(1) as f32;
    let aqa_boxed = in_col.iter().filter(|c| c.2.zero_padded).count() >= 2;
    // 3. Walk candidates in reading order, keeping the running sequence.
    let mut heads: Vec<(usize, usize, u32)> = Vec::new();
    let mut end_marker: Option<(usize, usize)> = None;
    'pages: for (pi, page) in pages.iter().enumerate() {
        for (li, line) in page.lines.iter().enumerate() {
            let t = plain_line(&line.text);
            if END_MARKER_RE.is_match(&t) && !heads.is_empty() {
                end_marker = Some((pi, li));
                break 'pages;
            }
            let Some(&&(_, _, h, x0, bold)) = in_col.iter().find(|c| c.0 == pi && c.1 == li) else { continue };
            let _ = x0;
            if bold_share >= 0.6 && !bold {
                continue;
            }
            // Spaced digit pairs are two-digit numbers only in AQA's boxed
            // style; elsewhere the first digit is the number.
            let number = if h.spaced_pair && !aqa_boxed {
                t.chars().next().and_then(|c| c.to_digit(10)).unwrap_or(h.number)
            } else {
                h.number
            };
            let current = heads.last().map(|x| x.2).unwrap_or(0);
            if number == current {
                continue; // a part of the running question
            }
            if number > current && number <= current + 3 {
                if number > current + 1 {
                    map.anomalies.push(format!("layout map: question numbers jump from {} to {}", current, number));
                }
                heads.push((pi, li, number));
            } else if !heads.is_empty() && number < current {
                // A restarted sequence (answer booklet) ends the paper.
                if number == 1 {
                    end_marker = Some((pi, li));
                    break 'pages;
                }
                map.anomalies.push(format!("layout map: out-of-sequence number {} after {} on page {}", number, current, pi + 1));
            }
        }
    }
    // Running text: a line repeated at the top or bottom edge of many pages
    // ("Created by T. Madas", a publisher banner) is furniture of no question.
    let running: std::collections::HashSet<String> = {
        let mut seen: std::collections::HashMap<String, std::collections::HashSet<usize>> = std::collections::HashMap::new();
        for (pi, page) in pages.iter().enumerate() {
            let h = page.height.max(1.0);
            for line in &page.lines {
                let t = plain_line(&line.text);
                if (line.y1 < 0.12 * h || line.y0 > 0.88 * h) && t.chars().filter(|c| c.is_alphabetic()).count() >= 4 {
                    seen.entry(t).or_default().insert(pi);
                }
            }
        }
        seen.into_iter().filter(|(_, ps)| ps.len() >= 3 && ps.len() * 3 >= pages.len()).map(|(t, _)| t).collect()
    };
    // 4. Trailing non-question pages end the last question; the same page
    // between two questions ("There are no questions printed on this page"
    // before a reference table) is skipped instead.
    let last_heading_page = heads.iter().map(|h| h.0).max().unwrap_or(0);
    let non_question_page = |pi: usize| {
        let first = pages[pi].lines.first().map(|l| plain_line(&l.text)).unwrap_or_default();
        TRAILING_PAGE_RE.is_match(&first)
    };
    let stop_at = |pi: usize, li: usize| -> bool {
        if let Some((ep, el)) = end_marker {
            if pi > ep || (pi == ep && li >= el) {
                return true;
            }
        }
        non_question_page(pi) && pi > last_heading_page
    };
    let skip_page = |pi: usize| non_question_page(pi) && pi <= last_heading_page;
    // Reference material printed ahead of the question it serves ("This table
    // is included so that you can answer Questions 09.1 and 09.2"): it ends
    // the question before it and is appended to the referenced question.
    let mut reference_runs: Vec<(u32, Vec<(usize, usize)>)> = Vec::new();
    let mut lead_in_runs: Vec<(u32, Vec<(usize, usize)>)> = Vec::new();
    for (k, &(pi, li, number)) in heads.iter().enumerate() {
        let next = heads.get(k + 1).copied();
        let mut lines = Vec::new();
        // Reference material for this question's own parts printed at its
        // end ("This table is included so that you can answer question parts
        // 07.1, 07.2 and 07.3"), after its last mark allocation.
        let mut own_reference: Option<usize> = None;
        let mut end_page = pi;
        let mut p = pi;
        let mut l = li;
        'collect: loop {
            if p >= pages.len() {
                break;
            }
            if skip_page(p) && !next.is_some_and(|(np, _, _)| np == p) {
                l = pages[p].lines.len();
            }
            while l < pages[p].lines.len() {
                if let Some((np, nl, _)) = next {
                    if p == np && l == nl {
                        break 'collect;
                    }
                }
                if stop_at(p, l) {
                    break 'collect;
                }
                if (p, l) != (pi, li) && SECTION_BREAK_RE.is_match(&plain_line(&pages[p].lines[l].text)) {
                    break 'collect;
                }
                if running.contains(&plain_line(&pages[p].lines[l].text)) {
                    l += 1;
                    continue;
                }
                if let Some(target) = REFERENCE_FOR_RE
                    .captures(&plain_line(&pages[p].lines[l].text))
                    .and_then(|c| c[1].parse::<u32>().ok())
                    .filter(|&t| t > number && heads.iter().any(|h| h.2 == t))
                {
                    // Everything from here to the next heading belongs to
                    // the referenced question.
                    let mut run = Vec::new();
                    let (mut rp, mut rl) = (p, l);
                    'reference: while rp < pages.len() {
                        while rl < pages[rp].lines.len() {
                            if next.is_some_and(|(np, nl, _)| rp == np && rl == nl) || stop_at(rp, rl) {
                                break 'reference;
                            }
                            if !running.contains(&plain_line(&pages[rp].lines[rl].text)) {
                                run.push((rp, rl));
                            }
                            rl += 1;
                        }
                        rp += 1;
                        rl = 0;
                    }
                    reference_runs.push((target, run));
                    break 'collect;
                }
                // A lead-in for the next question's parts ("Question parts
                // 06.1 and 06.2 use a normalised floating point
                // representation ...") opens that question.
                if let Some(target) = LEAD_IN_RE
                    .captures(&plain_line(&pages[p].lines[l].text))
                    .and_then(|c| c[1].parse::<u32>().ok())
                    .filter(|&t| t > number && next.is_some_and(|(_, _, nn)| nn == t))
                {
                    let mut run = Vec::new();
                    let (mut rp, mut rl) = (p, l);
                    'lead: while rp < pages.len() {
                        while rl < pages[rp].lines.len() {
                            if next.is_some_and(|(np, nl, _)| rp == np && rl == nl) || stop_at(rp, rl) {
                                break 'lead;
                            }
                            if !running.contains(&plain_line(&pages[rp].lines[rl].text)) {
                                run.push((rp, rl));
                            }
                            rl += 1;
                        }
                        rp += 1;
                        rl = 0;
                    }
                    lead_in_runs.push((target, run));
                    break 'collect;
                }
                if own_reference.is_none()
                    && REFERENCE_FOR_RE.captures(&plain_line(&pages[p].lines[l].text)).and_then(|c| c[1].parse::<u32>().ok()) == Some(number)
                {
                    own_reference = Some(lines.len());
                }
                lines.push((p, l));
                end_page = p;
                l += 1;
            }
            p += 1;
            l = 0;
            if let Some((np, _, _)) = next {
                if p > np {
                    break;
                }
            }
        }
        let end_y = next.and_then(|(np, nl, _)| if np == end_page { pages[np].lines.get(nl).map(|x| x.y0) } else { None });
        let marked = |&(p, l): &(usize, usize)| plain_line(&pages[p].lines[l].text).split_whitespace().any(|w| MARK_TOKEN_RE.is_match(w));
        let reference_from = own_reference.filter(|&i| i > 0 && lines[..i].iter().any(marked) && !lines[i..].iter().any(marked));
        map.questions.push(LayoutQuestion {
            number,
            start_y: pages[pi].lines[li].y0,
            lines,
            start_page: pi,
            end_page,
            end_y,
            reference_from,
        });
    }
    for (target, run) in reference_runs {
        if let Some(q) = map.questions.iter_mut().find(|q| q.number == target) {
            if !run.is_empty() {
                q.reference_from = Some(q.lines.len());
            }
            q.lines.extend(run);
        }
    }
    for (target, run) in lead_in_runs {
        if let (Some(q), Some(&(fp, fl))) = (map.questions.iter_mut().find(|q| q.number == target), run.first()) {
            if fp < q.start_page || fp == q.start_page && pages[fp].lines[fl].y0 < q.start_y {
                q.start_page = fp;
                q.start_y = pages[fp].lines[fl].y0;
            }
            let mut lines = run;
            lines.extend(std::mem::take(&mut q.lines));
            q.lines = lines;
        }
    }
    map
}

/// The body text of one mapped question (lines joined with newlines; a page
/// turn is just another line break).
pub fn question_text(pages: &[LayoutPage], q: &LayoutQuestion) -> String {
    q.lines.iter().map(|&(p, l)| pages[p].lines[l].text.as_str()).collect::<Vec<_>>().join("\n")
}

static QUESTION_FOOTER_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    // The question number may be omitted ("Total for question = 11 marks").
    regex::Regex::new(r"(?i)^\(?\s*total\s+for\s+question\s*(\d{1,3})?\s*(?:is|=)\s*(\d{1,3})\s*marks?\s*\)?\.?$").unwrap()
});

/// The body of one mapped question, ready for normalisation: the question's
/// own lines with its printed "(Total for Question N is M marks)" footer
/// removed and recorded.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QuestionBody {
    pub text: String,
    pub footer_marks: Option<u32>,
    /// Pages the body spans (first, last).
    pub pages: (usize, usize),
    /// "Figure N" captions beside an empty drawing space with no figure
    /// found in it: a diagram the figure detection missed.
    pub undetected_figures: usize,
    /// The body closes with appended reference material (see
    /// [`LayoutQuestion::reference_from`]), whose last line may carry no
    /// closing punctuation.
    pub ends_with_reference: bool,
}

static OPTION_HEAD_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"^([A-E])[ \t]+\S").unwrap());

/// Lines that wrap a multiple-choice option: for each bold option head
/// ("B the charge stored on …"), the following lines set at the option's
/// text indent and line spacing continue that option, up to the next head.
/// Requires the A–D heads in order, so a stem sentence starting "A planet"
/// is never mistaken for an option.
fn option_continuations(pages: &[LayoutPage], lines: &[(usize, usize)]) -> std::collections::HashSet<usize> {
    let mut joined = std::collections::HashSet::new();
    let heads: Vec<(usize, char)> = lines
        .iter()
        .enumerate()
        .filter_map(|(k, &(p, l))| {
            let line = &pages[p].lines[l];
            let c = OPTION_HEAD_RE.captures(&line.text)?;
            line.bold_lead.then(|| (k, c[1].chars().next().unwrap()))
        })
        .collect();
    let Some(start) = heads.iter().rposition(|h| h.1 == 'A') else { return joined };
    let run: Vec<(usize, char)> = heads[start..].to_vec();
    if run.len() < 4 || !run.iter().enumerate().all(|(i, h)| h.1 == (b'A' + i as u8) as char) {
        return joined;
    }
    for (i, &(hk, _)) in run.iter().enumerate() {
        let (hp, hl) = lines[hk];
        let head = &pages[hp].lines[hl];
        let stop = run.get(i + 1).map(|h| h.0).unwrap_or(lines.len());
        let s = head.size.max(4.0);
        let mut prev = head;
        for k in (hk + 1)..stop {
            let (p, l) = lines[k];
            let line = &pages[p].lines[l];
            let indented = p == hp && line.x0 > head.x0 + 0.5 * s && line.x0 < head.x0 + 8.0 * s;
            let step = line.baseline - prev.baseline;
            if !indented || step <= 0.0 || step > 1.7 * s || line.text.starts_with('[') {
                break;
            }
            joined.insert(k);
            prev = line;
        }
    }
    joined
}

/// Where a figure is spliced into a question body.
pub const FIGURE_PLACEHOLDER: &str = "[DIAGRAM_PLACEHOLDER]";

static FIGURE_LABEL_KEEP_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)^(?:fig(?:ure|\.)?\s*\d+|table\s+\d+|\[?\(?\d{1,2}\s*marks?\)?\]?|\(\d{1,2}\)|\(?[a-h]\)|\(?(?:i|ii|iii|iv|v|vi)\))").unwrap()
});

pub fn question_body(pages: &[LayoutPage], q: &LayoutQuestion) -> QuestionBody {
    question_body_with_figures(pages, q, &[])
}

/// The question's text with a [`FIGURE_PLACEHOLDER`] line where each of
/// `figures` — (page, `[x0, y0, x1, y1]` in points), in reading order — is
/// drawn: before the first line below the figure's top. A short line lying
/// wholly inside a figure is one of its labels and stays in the figure.
pub fn question_body_with_figures(pages: &[LayoutPage], q: &LayoutQuestion, figures: &[(usize, [f32; 4])]) -> QuestionBody {
    let mut kept: Vec<String> = Vec::new();
    let mut footer_marks = None;
    let continuations = option_continuations(pages, &q.lines);
    let mut next_figure = 0usize;
    let inline = |p: usize, r: &[f32; 4]| pages[p].inline_images.iter().any(|im| (im[0] - r[0]).abs() < 0.5 && (im[1] - r[1]).abs() < 0.5);
    let in_figure = |p: usize, line: &LayoutLine| {
        let plain = plain_line(&line.text);
        !line.code
            && plain.chars().count() <= 40
            && !FIGURE_LABEL_KEEP_RE.is_match(plain.trim())
            && figures.iter().any(|&(fp, r)| fp == p && line.x0 >= r[0] - 1.0 && line.x1 <= r[2] + 1.0 && line.y0 >= r[1] - 1.0 && line.y1 <= r[3] + 1.0)
    };
    // Consecutive code lines become one fenced block, indented from the
    // block's own left edge in the font's advance.
    let mut code_run: Vec<&LayoutLine> = Vec::new();
    fn flush_code(kept: &mut Vec<String>, run: &mut Vec<&LayoutLine>) {
        if run.is_empty() {
            return;
        }
        let left = run.iter().map(|l| l.x0).fold(f32::MAX, f32::min);
        // A gap of an extra line or more separates groups (blank lines in
        // the listing); a page turn (baseline going back up) does not.
        let pitch = run.windows(2).map(|w| w[1].baseline - w[0].baseline).filter(|d| *d > 0.0).fold(f32::MAX, f32::min);
        let mut block = String::from("```");
        for (i, l) in run.iter().enumerate() {
            if i > 0 && l.baseline - run[i - 1].baseline > (1.9 * l.size).max(1.5 * pitch) {
                block.push('\n');
            }
            let indent = ((l.x0 - left) / l.char_width.max(1.0)).round().max(0.0) as usize;
            block.push('\n');
            block.push_str(&" ".repeat(indent));
            block.push_str(&l.text);
        }
        block.push_str("\n```");
        kept.push(block);
        run.clear();
    }
    for (k, &(p, l)) in q.lines.iter().enumerate() {
        let line = &pages[p].lines[l];
        // Figures drawn above this line come first (never ahead of the
        // heading: a figure beside "5." follows it).
        let cy = (line.y0 + line.y1) * 0.5;
        while let Some(&(fp, r)) = figures.get(next_figure).filter(|_| k > 0) {
            if fp > p || fp == p && r[1] >= cy {
                break;
            }
            next_figure += 1;
            if inline(fp, &r) {
                continue; // spliced at its character below
            }
            flush_code(&mut kept, &mut code_run);
            kept.push(FIGURE_PLACEHOLDER.to_string());
        }
        if in_figure(p, line) {
            continue;
        }
        let t = line.text.as_str();
        if let Some(c) = QUESTION_FOOTER_RE.captures(plain_line(t).as_str()) {
            if c.get(1).map_or(true, |n| n.as_str().parse::<u32>().ok() == Some(q.number)) {
                footer_marks = c[2].parse().ok();
                continue;
            }
        }
        // A lone punctuation mark on its own line is typesetting debris.
        let bare = t.replace('$', "");
        let bare = bare.trim();
        if bare.chars().count() == 1 && bare.chars().all(|c| matches!(c, ',' | '.' | ';' | ':')) {
            continue;
        }
        if line.code {
            code_run.push(line);
            continue;
        }
        flush_code(&mut kept, &mut code_run);
        match kept.last_mut() {
            Some(last) if continuations.contains(&k) && last != FIGURE_PLACEHOLDER => {
                last.push(' ');
                last.push_str(t);
            }
            _ => kept.push(t.to_string()),
        }
    }
    flush_code(&mut kept, &mut code_run);
    for &(fp, r) in &figures[next_figure.min(figures.len())..] {
        if !inline(fp, &r) {
            kept.push(FIGURE_PLACEHOLDER.to_string());
        }
    }
    // Inline pictures become placeholders where they stand in the text.
    for k in kept.iter_mut() {
        if k.contains(INLINE_IMAGE) {
            *k = k.replace(INLINE_IMAGE, FIGURE_PLACEHOLDER);
        }
    }
    let undetected_figures = undetected_figure_captions(pages, q, figures);
    QuestionBody { text: kept.join("\n"), footer_marks, pages: (q.start_page, q.end_page), undetected_figures, ends_with_reference: q.reference_from.is_some() }
}

static FIGURE_CAPTION_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)^(?:figure|fig\.?)\s*\d+[a-z]?$").unwrap());

/// Captions ("Figure 3") with no figure next to them and a drawing-sized
/// empty space above or below them on the page: the space holds a diagram
/// that was not found (a figure printed as text - a program, relations, a
/// table - sits close under its caption instead).
fn undetected_figure_captions(pages: &[LayoutPage], q: &LayoutQuestion, figures: &[(usize, [f32; 4])]) -> usize {
    let mut missing = 0;
    for &(p, l) in &q.lines {
        let cap = &pages[p].lines[l];
        if !FIGURE_CAPTION_RE.is_match(plain_line(&cap.text).trim()) {
            continue;
        }
        let s = cap.size.max(6.0);
        let near_figure = figures.iter().any(|&(fp, r)| fp == p && r[1] <= cap.y1 + 3.0 * s && r[3] >= cap.y0 - 3.0 * s);
        if near_figure {
            continue;
        }
        // Text or a table directly above / below on the page.
        let lp = &pages[p];
        let below = lp.lines.iter().filter(|o| o.y0 >= cap.y1 - 0.5).map(|o| o.y0 - cap.y1).fold(f32::MAX, f32::min);
        let above = lp.lines.iter().filter(|o| o.y1 <= cap.y0 + 0.5).map(|o| cap.y0 - o.y1).fold(f32::MAX, f32::min);
        let bottom_room = lp.height - cap.y1;
        if below > 3.0 * s && bottom_room > 3.0 * s || above > 3.0 * s && cap.y0 > 0.15 * lp.height {
            missing += 1;
        }
    }
    missing
}

/// The figures a layout question owns: on its pages, with their centre
/// between its heading and the next question's heading.
pub fn question_figure_window(pages: &[LayoutPage], q: &LayoutQuestion, page: usize, cy: f32) -> bool {
    if page < q.start_page || page > q.end_page {
        return false;
    }
    let h = pages.get(page).map(|lp| lp.height).unwrap_or(f32::MAX);
    let top = if page == q.start_page { q.start_y - 2.0 } else { 0.0 };
    let bottom = if page == q.end_page { q.end_y.unwrap_or(h) } else { h };
    cy >= top && cy <= bottom
}

/// A layout figure region widened to take in the label lines set just
/// outside its strokes (axis names, vertex letters), plus a small margin.
pub fn figure_crop_rect(lp: &LayoutPage, r: [f32; 4]) -> [f32; 4] {
    let dist = |f: &LayoutLine, b: &[f32; 4]| (b[0] - f.x1).max(f.x0 - b[2]).max(b[1] - f.y1).max(f.y0 - b[3]).max(0.0);
    let others: Vec<[f32; 4]> = lp.figures.iter().copied().filter(|o| *o != r).collect();
    // Labels reach out from the drawing in steps (an axis title beyond its
    // tick values); a label nearer another figure on the page is that one's.
    let mut out = r;
    let mut taken = vec![false; lp.furniture.len()];
    for _ in 0..4 {
        let mut grew = false;
        for (i, f) in lp.furniture.iter().enumerate() {
            if taken[i] || f.furniture != Some(FurnitureKind::Figure) || dist(f, &out) > 18.0 || others.iter().any(|o| dist(f, o) < dist(f, &r)) {
                continue;
            }
            taken[i] = true;
            grew = true;
            out[0] = out[0].min(f.x0);
            out[1] = out[1].min(f.y0);
            out[2] = out[2].max(f.x1);
            out[3] = out[3].max(f.y1);
        }
        if !grew {
            break;
        }
    }
    [(out[0] - 3.0).max(0.0), (out[1] - 3.0).max(0.0), (out[2] + 3.0).min(lp.width), (out[3] + 3.0).min(lp.height)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Laid-out pages of a corpus paper; `None` when the fixture or PDFium is
    /// unavailable on this machine.
    fn corpus_pages(rel: &str) -> Option<Vec<LayoutPage>> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(rel);
        if !path.exists() {
            eprintln!("[LAYOUT] fixture missing: {rel}");
            return None;
        }
        crate::pdf_render::load_layout_pages(&path)
    }

    fn body(pages: &[LayoutPage], map: &LayoutMap, n: u32) -> String {
        let q = map.questions.iter().find(|q| q.number == n).unwrap_or_else(|| panic!("Q{n} not mapped"));
        question_body(pages, q).text
    }

    #[test]
    fn aqa_gcse_further_maths_2024_bodies_are_isolated_and_source_faithful() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        let Some(pages) = corpus_pages("past papers for mergemark/aqa gcse further maths '24.pdf") else { return };
        let map = build_layout_map(&pages);
        assert_eq!(map.questions.iter().map(|q| q.number).collect::<Vec<_>>(), (1..=23).collect::<Vec<_>>());
        // Q5's heading "5  y = …" has no capitalised word after the number.
        let q5 = body(&pages, &map, 5);
        assert!(q5.contains(r"$y = \frac{4x^{3} + x^{7}}{x^{4}}$"), "{q5}");
        assert!(q5.contains(r"Work out $\frac{\mathrm{d}y}{\mathrm{d}x}$"), "{q5}");
        assert!(q5.contains("[3 marks]"), "{q5}");
        assert!(!q5.contains("lie on a circle"), "Q6 leaked into Q5: {q5}");
        let q22 = body(&pages, &map, 22);
        assert!(q22.contains(r"\frac{3 \sin x \cos x + \sin^{2}x}{12\cos^{2}x + 4 \sin x \cos x}"), "{q22}");
        assert!(q22.contains(r"- \frac{\sqrt{3}}{4}"), "{q22}");
        assert!(q22.contains("[4 marks]"), "{q22}");
    }

    #[test]
    fn cie_2022_bodies_rebuild_maths_from_font_identities() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        let Some(pages) = corpus_pages("test_papers/04_cie_maths_9709_p1_qp.pdf") else { return };
        let map = build_layout_map(&pages);
        assert_eq!(map.questions.len(), 10);
        // The "ffi" ligature is one glyph expanded into three chars.
        assert!(body(&pages, &map, 3).contains("The coefficient of"));
        // NewMathSymb/NewMathExtn glyphs carry no usable unicode.
        let q4 = body(&pages, &map, 4);
        assert!(
            q4.contains(r"\frac{\sin^{3} \theta}{\sin \theta - 1} - \frac{\sin^{2} \theta}{1 + \sin \theta} \equiv - \tan^{2} \theta(1 + \sin^{2} \theta)"),
            "{q4}"
        );
        // The strict signs are the font's own "<"/">" codes; only its
        // U+2264 code is printed as ⩽.
        assert!(q4.contains(r"for $0 < \theta < 2\pi$."), "{q4}");
        let q6 = body(&pages, &map, 6);
        assert!(q6.contains(r"for $x > 2$."), "{q6}");
        let q8 = body(&pages, &map, 8);
        assert!(q8.contains(r"for $0^{\circ} \leqslant x \leqslant 360^{\circ}$."), "{q8}");
        assert!(!q4.contains(UNKNOWN_GLYPH) && !q4.chars().any(|c| c.is_control() && c != '\n' && c != '\t'), "{q4:?}");
        let q5 = body(&pages, &map, 5);
        assert!(q5.contains(r"(a) Given that $\theta = \frac{1}{6}\pi$"), "{q5}");
        assert!(q5.contains(r"(b) Given instead that the length of $BD$ is $\frac{\sqrt{3}}{2} r$"), "{q5}");
        assert!(!q5.contains("The function f is defined"), "Q6 leaked into Q5: {q5}");
    }

    #[test]
    fn physics_2024_nuclides_options_and_section_breaks() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        let Some(pages) = corpus_pages("past papers for mergemark/physics '24.pdf") else { return };
        let map = build_layout_map(&pages);
        assert_eq!(map.questions.len(), 32);
        // Full-size stacked numbers before a symbol are its prescripts.
        let q18 = body(&pages, &map, 18);
        assert!(q18.contains(r"{}^{235}_{92}\mathrm{U}"), "{q18}");
        assert!(q18.contains(r"{}^{146}_{57}\text{La}"), "{q18}");
        // Section B's instructions belong to no question.
        let q7 = body(&pages, &map, 7);
        assert!(q7.contains("Suggest why."), "{q7}");
        assert!(!q7.contains("END OF SECTION") && !q7.contains("best response"), "{q7}");
        // Wrapped option text stays on its option's line.
        let q24 = body(&pages, &map, 24);
        assert!(q24.lines().any(|l| l.starts_with("B the charge stored") && l.ends_with("between the plates is 1 V")), "{q24}");
        // An option letter beside a tall fraction shares its line.
        let q21 = body(&pages, &map, 21);
        assert!(q21.contains(r"A $\frac{R}{\sqrt[3]{2}}$"), "{q21}");
    }

    #[test]
    fn display_integral_limits_never_take_neighbouring_prose() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        let Some(pages) = corpus_pages("past papers for mergemark/core pure 1 '21.pdf") else { return };
        let map = build_layout_map(&pages);
        let q2 = body(&pages, &map, 2);
        assert!(q2.contains("(c) Use the integration function on your calculator to evaluate\n"), "{q2}");
        assert!(q2.contains(r"$\int_{\frac{\pi}{6}}^{\frac{\pi}{2}} \left(\frac{1}{x} \cos^{2} \left(\frac{x}{3}\right)\right) \, \mathrm{d}x$"), "{q2}");
        assert!(q2.contains("Give your answer to 5 decimal places."), "{q2}");
    }

    #[test]
    fn cs_options_bit_patterns_arrows_and_labelled_tables_are_source_faithful() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        let Some(pages) = corpus_pages("past papers for mergemark/computer science 2 '22.pdf") else { return };
        let map = build_layout_map(&pages);
        // A letter centred on its two-line option heads the option.
        let q2 = body(&pages, &map, 2);
        assert!(q2.contains("A For a particular communications channel, the bit rate can be higher than the baud rate."), "{q2}");
        assert!(q2.contains("C The bandwidth of a transmission medium is the range of signal frequencies"), "{q2}");
        // The binary point is a drawn dot between the bits.
        let q5 = body(&pages, &map, 5);
        assert!(q5.contains("1  0  1  1  0  0 ● 1  0  1  1"), "{q5}");
        // "Answer" beside its answer boxes is answer space.
        assert!(!q5.lines().any(|l| l.trim() == "Answer"), "{q5}");
        // Code words keep their code span before prose punctuation, and an
        // en dash between numbers is a range, not a minus.
        let q9_table = body(&pages, &map, 9);
        assert!(q9_table.contains("into register `d`."), "{q9_table}");
        assert!(q9_table.contains("are numbered 0–12"), "{q9_table}");
        // A program listing keeps its indentation and its blank lines.
        let q9 = body(&pages, &map, 9);
        assert!(q9.contains("```\n  LDR R0, 120\n  LDR R1, 121\n  MOV R3, #0\nloop:\n"), "{q9}");
        let q12 = body(&pages, &map, 12);
        assert!(q12.contains("temps = [50, 68, 95, 86]\n\nfu a = (a - 32) * 5 / 9\n"), "{q12}");

        let Some(pages) = corpus_pages("past papers for mergemark/computer science 2 '23.pdf") else { return };
        let map = build_layout_map(&pages);
        // Word's "-->" arrows are Wingdings glyphs.
        let q5 = body(&pages, &map, 5);
        assert!(q5.contains(r"\text{GET}\rightarrow\text{FETCH}"), "{q5}");
        // Question 6's lead-in opens Question 6, not the end of Question 5.
        assert!(!q5.contains("Question parts 06.1"), "{q5}");
        assert!(body(&pages, &map, 6).starts_with("Question parts 06.1 and 06.2 use"), "{}", body(&pages, &map, 6));
        // Row labels left of a grid are its first column.
        let q7 = body(&pages, &map, 7);
        assert!(q7.contains("| `R1` | `0` | `1` | `0` | `0` | `0` | `1` | `1` | `0` |"), "{q7}");
        assert!(q7.contains("| `15` | `0` | `0` | `0` | `0` | `1` | `1` | `1` | `1` |"), "{q7}");

        let Some(pages) = corpus_pages("past papers for mergemark/computer science 2 '24.pdf") else { return };
        let map = build_layout_map(&pages);
        // The examiner's page total in the margin is not option text.
        let q7 = body(&pages, &map, 7);
        assert!(q7.contains("E The protocol operates at the network layer of the TCP/IP stack.") && !q7.contains("stack.\t15") && !q7.ends_with("15"), "{q7}");
    }

    #[test]
    fn subscripts_brackets_and_outlined_symbols_follow_the_source() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        let Some(pages) = corpus_pages("past papers for mergemark/physics '21.pdf") else { return };
        let map = build_layout_map(&pages);
        // A reduced k set on E's baseline is its subscript, then raised.
        let q5 = body(&pages, &map, 5);
        assert!(q5.contains(r"$E_k^{1.5}$") && !q5.contains(r"E\mathrm{k}"), "{q5}");
        assert!(body(&pages, &map, 19).contains(r"a distance $r_2$ from O."));
        // ω is drawn as outlines: no identity, so it is carried unidentified.
        let q2 = body(&pages, &map, 2);
        assert_eq!(q2.matches(UNKNOWN_GLYPH).count(), 2, "{q2}");

        let Some(pages) = corpus_pages("past papers for mergemark/core pure 1 '23.pdf") else { return };
        let map = build_layout_map(&pages);
        // SymbolMT pieces under control codes assemble the matrix brackets.
        let q8 = body(&pages, &map, 8);
        assert!(q8.contains(r"\end{pmatrix}^{-1}") && !q8.contains(UNKNOWN_GLYPH) && !q8.contains('⎝'), "{q8}");
        // Italic prose keeps its short words as words.
        assert!(q8.contains("(There is no need to estimate"), "{q8}");
        // Enlarged parentheses do not set the line's size.
        let q6 = body(&pages, &map, 6);
        assert!(q6.contains(r"$\frac{\mathrm{d}}{\mathrm{d}t} (\arctan \mathrm{e}^{0.4t})$"), "{q6}");
        // "i sin" is two words though set only a thin space apart.
        let q3 = body(&pages, &map, 3);
        assert!(q3.contains(r"+ \mathrm{i} \sin \frac{17\pi}{12}\right)$"), "{q3}");

        let Some(pages) = corpus_pages("past papers for mergemark/core pure 1 '21.pdf") else { return };
        let map = build_layout_map(&pages);
        // A short bracket's bottom piece overlapping its extension is one bracket.
        let q9 = body(&pages, &map, 9);
        assert!(q9.contains(r"\frac{1}{2} \left[x\sqrt{x^{2} - 1} + \operatorname{arcosh} x\right] + k"), "{q9}");
    }

    #[test]
    fn legacy_cie_word_breaks_vector_arrows_and_small_print() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        let Some(pages) = corpus_pages("test_papers/07_legacy_c3_or_c4_qp.pdf") else { return };
        let map = build_layout_map(&pages);
        // Words positioned apart with no space character are still words.
        assert!(body(&pages, &map, 1).contains(r"graph of $y = a + b \sin x$"), "{}", body(&pages, &map, 1));
        // "−−→" typed over OA is the vector's arrow.
        let q8 = body(&pages, &map, 8);
        assert!(q8.contains(r"$\overrightarrow{OA} = ") && q8.contains(r"$\overrightarrow{BA}$"), "{q8}");
        assert!(!q8.lines().any(|l| l.trim().chars().all(|c| matches!(c, '−' | '-' | '\t' | ' '))), "{q8}");
        // Navigation and the publisher's small print are page furniture.
        assert!(!body(&pages, &map, 9).contains("printed on the next page"));
        assert!(!body(&pages, &map, 12).contains("Permission to reproduce"));
        // A sign printed without a bar roots the bracketed group after it.
        let q7 = body(&pages, &map, 7);
        assert!(q7.contains(r"distance $AB$ is $\sqrt{(125)}$ units"), "{q7}");
        // Both rows' conditions start in one column, the first row's gap
        // being narrower; the words keep their spaces in maths mode.
        let q10 = body(&pages, &map, 10);
        assert!(
            q10.contains(r"\begin{cases}3x - 2 & \text{for } -1 \leqslant x \leqslant 1, \\ \frac{4}{5 - x} & \text{for } 1 < x \leqslant 4.\end{cases}"),
            "{q10}"
        );
    }

    #[test]
    fn spacing_and_vector_terms_follow_the_print() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        // A generated space spanning glyphs drawn inside it is no gap.
        if let Some(pages) = corpus_pages("past papers for mergemark/further mechanics 1 '23.pdf") {
            let map = build_layout_map(&pages);
            assert!(body(&pages, &map, 3).contains(r"$\frac{(4 + 10e)u}{3}$"), "{}", body(&pages, &map, 3));
        }
        // A number set a clear gap before its unit; a bold vector before a
        // full stop or comma.
        if let Some(pages) = corpus_pages("past papers for mergemark/further mechanics 1 '22.pdf") {
            let map = build_layout_map(&pages);
            let q2 = body(&pages, &map, 2);
            assert!(q2.contains("constant force of magnitude 200 N."), "{q2}");
            let q8 = body(&pages, &map, 8);
            assert!(q8.contains(r"in terms of $\mathbf{i}$ and $\mathbf{j}$, the velocity"), "{q8}");
        }
        // An operator binds its operand; an upright i after maths is the
        // imaginary unit.
        if let Some(pages) = corpus_pages("past papers for mergemark/core pure 1 '23.pdf") {
            let map = build_layout_map(&pages);
            assert!(body(&pages, &map, 3).contains(r"$z_1 = -4 + 4\mathrm{i}$"), "{}", body(&pages, &map, 3));
        }
        // The typed space under an italic f's swash is a word space.
        if let Some(pages) = corpus_pages("test_papers/03_aqa_physics_p1_qp.pdf") {
            let map = build_layout_map(&pages);
            assert!(body(&pages, &map, 19).contains("on a string is $f$."), "{}", body(&pages, &map, 19));
        }
    }

    #[test]
    fn axis_titles_beyond_the_tick_values_are_in_the_figure_crop() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        let Some(pages) = corpus_pages("past papers for mergemark/Nov 2024 QP.pdf") else { return };
        // Q22's velocity-time graph: "Velocity (m/s)" left of the speed
        // values and "Time (s)" under the time values.
        let lp = &pages[19];
        assert_eq!(lp.figures.len(), 1, "{:?}", lp.figures);
        let crop = figure_crop_rect(lp, lp.figures[0]);
        let title = |t: &str| lp.furniture.iter().find(|l| l.text == t).unwrap_or_else(|| panic!("{t} not found"));
        for t in ["Velocity (m/s)", "Time (s)"] {
            let l = title(t);
            assert!(l.x0 >= crop[0] && l.x1 <= crop[2] && l.y0 >= crop[1] && l.y1 <= crop[3], "{t} {:?} outside {crop:?}", [l.x0, l.y0, l.x1, l.y1]);
        }
    }

    #[test]
    fn a_question_closing_on_its_own_reference_table_ends_there() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        let Some(pages) = corpus_pages("past papers for mergemark/computer science 2 '23.pdf") else { return };
        let map = build_layout_map(&pages);
        // Table 3 "is included so that you can answer question parts 07.1,
        // 07.2 and 07.3" after 07.3's marks; its last line ends unpunctuated.
        let q7 = map.questions.iter().find(|q| q.number == 7).unwrap();
        let b7 = question_body(&pages, q7);
        assert!(b7.ends_with_reference && b7.text.trim_end().ends_with("are numbered 0–12"), "{}", b7.text);
        // A question without such material does not.
        let q8 = map.questions.iter().find(|q| q.number == 8).unwrap();
        assert!(!question_body(&pages, q8).ends_with_reference);
    }

    #[test]
    fn page_qr_codes_are_furniture_not_figures() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        let Some(pages) = corpus_pages("past papers for mergemark/aea2024.pdf") else { return };
        // Each question's first page carries a QR code (an image inside
        // nested forms) at the top right, a point or two from the others.
        let qr = |lp: &LayoutPage| lp.furniture_images.iter().copied().find(|r| r[0] > 480.0 && r[3] < 100.0 && (r[2] - r[0] - 50.0).abs() < 5.0);
        for p in [1usize, 3, 5, 9, 13, 19, 25] {
            assert!(qr(&pages[p]).is_some(), "page {} QR: {:?}", p + 1, pages[p].furniture_images);
        }
        // Figure 1 is the only figure on its page; Figure 4's region does not
        // grow to the code above its right end.
        assert_eq!(pages[3].figures.len(), 1, "{:?}", pages[3].figures);
        assert!(pages[25].figures.iter().all(|r| r[1] > 70.0), "{:?}", pages[25].figures);
    }

    #[test]
    fn big_operators_stay_in_the_sentence_they_are_printed_in() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        let Some(pages) = corpus_pages("test_papers/01_edexcel_maths_p1_qp.pdf") else { return };
        let map = build_layout_map(&pages);
        // The enlarged Σ (with "lim" over "δx→0" before it) is set inside
        // the heading line of Q4, not on a line of its own above it.
        let q4 = body(&pages, &map, 4);
        assert!(q4.contains(r"(a) Express $\lim_{\delta x\rightarrow0}\sum_{x = 2.1}^{6.3} \frac{2}{x} \delta x$ as an integral."), "{q4}");
        let q3 = body(&pages, &map, 3);
        assert!(!q3.contains(r"\sum"), "Q4's formula leaked into Q3: {q3}");
        // The differential is set apart from its integrand.
        let q12 = body(&pages, &map, 12);
        assert!(q12.contains(r"x^{3} \ln x \, \mathrm{d}x = a\mathrm{e}^{8} + b$"), "{q12}");
    }

    #[test]
    fn formula_pictures_stay_where_they_are_printed() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        let Some(pages) = corpus_pages("past papers for mergemark/naikermaths_paper_m16_pure.pdf") else { return };
        let map = build_layout_map(&pages);
        let q3 = body(&pages, &map, 3);
        assert!(q3.contains(&format!("(a) Find {}, writing your answer", FIGURE_PLACEHOLDER)), "{q3}");
        let q5 = body(&pages, &map, 5);
        assert!(q5.contains(&format!("The point $P$ with coordinates {} lies on $C$.", FIGURE_PLACEHOLDER)), "{q5}");
    }

    #[test]
    fn sheared_greek_and_stroked_bars_are_source_text() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        let Some(pages) = corpus_pages("past papers for mergemark/madas_paper_2_t.pdf") else { return };
        let map = build_layout_map(&pages);
        let q3 = body(&pages, &map, 3);
        // α is drawn with a slant matrix (reported as an 18.9° "angle").
        assert!(q3.contains(r"has a solution $\alpha$, which is numerically small."), "{q3}");
        // |x| is two vector strokes around the x.
        assert!(q3.contains("|x|"), "{q3}");
        // The running banner is page furniture.
        assert!(!q3.contains("Created by T. Madas"), "{q3}");
    }
}
