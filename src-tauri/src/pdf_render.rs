use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use pdfium_render::prelude::*;
use crate::pipeline::{PageInput, PageInputKind};
use base64::Engine;
use image::DynamicImage;
use std::io::Cursor;
use std::sync::OnceLock;

static PDFIUM_INSTANCE: OnceLock<Result<Pdfium, String>> = OnceLock::new();

struct PageRenderCacheState {
    pages: HashMap<usize, Arc<DynamicImage>>,
    lru: VecDeque<usize>,
}

/// Bounded, per-import cache for high-resolution pages used by physical
/// diagram crops. The internal mutex lets the existing concurrent extraction
/// futures share one cache without changing their scheduling model.
pub struct PageRenderCache {
    capacity: usize,
    state: Mutex<PageRenderCacheState>,
    /// Lazily-loaded PDF document shared across every page render in this
    /// cache. Loading once instead of once-per-page removes the repeated
    /// full-PDF parses that dominated figure-crop wall-time.
    document: Mutex<Option<(PathBuf, Arc<PdfDocument<'static>>)>>,
    /// Serializes pdfium renders. Pdfium itself is NOT thread-safe (its
    /// `thread_safe` feature only adds Send+Sync bounds — it does not lock),
    /// so concurrent figure-attach jobs must not render simultaneously even
    /// though the document is shared. Holds only during the actual render, so
    /// cache-hit lookups never block on an in-flight render.
    render: Mutex<()>,
    /// Number of times the PDF was loaded from disk (test instrumentation:
    /// a multi-page render sequence through one cache must see exactly 1).
    pub load_count: AtomicUsize,
}

impl PageRenderCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            state: Mutex::new(PageRenderCacheState {
                pages: HashMap::with_capacity(capacity.max(1)),
                lru: VecDeque::with_capacity(capacity.max(1)),
            }),
            document: Mutex::new(None),
            render: Mutex::new(()),
            load_count: AtomicUsize::new(0),
        }
    }

    /// Return a shared 300-DPI page image, rendering it exactly once while it
    /// remains resident in the bounded cache.
    pub fn get_or_render(
        &self,
        path: &Path,
        page_idx: usize,
    ) -> Result<Arc<DynamicImage>, String> {
        // Fast path: cache hit — touch LRU, return. Brief lock only.
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "300-DPI page cache lock poisoned".to_string())?;
            if let Some(image) = state.pages.get(&page_idx).cloned() {
                if let Some(position) = state.lru.iter().position(|cached| *cached == page_idx) {
                    state.lru.remove(position);
                }
                state.lru.push_back(page_idx);
                return Ok(image);
            }
        }

        // Slow path: render from the shared document. The document is loaded
        // once; the dedicated render mutex keeps the renders serialized
        // because pdfium is not thread-safe. The state lock is NOT held, so
        // concurrent cache-hit lookups never block on an in-flight render.
        let document = self.get_or_load_document(path)?;
        let _render_guard = self
            .render
            .lock()
            .map_err(|_| "300-DPI render lock poisoned".to_string())?;
        let image = Arc::new(render_page_from_document(
            document.as_ref(),
            page_idx,
            figure_render_dpi(),
        )?);
        drop(_render_guard);

        // Double-checked insert: a parallel job may have rendered this page
        // while we were rendering — prefer its entry so the LRU slot is not
        // wasted on a duplicate.
        let mut state = self
            .state
            .lock()
            .map_err(|_| "300-DPI page cache lock poisoned".to_string())?;
        if let Some(image) = state.pages.get(&page_idx).cloned() {
            return Ok(image);
        }
        if state.pages.len() >= self.capacity {
            if let Some(evicted) = state.lru.pop_front() {
                state.pages.remove(&evicted);
            }
        }
        state.pages.insert(page_idx, Arc::clone(&image));
        state.lru.push_back(page_idx);
        Ok(image)
    }

    /// Load the PDF document once per cache and hand back a shared handle,
    /// reloading only when a different path is requested (defensive: one
    /// import uses one cache, so this is at most one load per run).
    fn get_or_load_document(
        &self,
        path: &Path,
    ) -> Result<Arc<PdfDocument<'static>>, String> {
        let mut document = self
            .document
            .lock()
            .map_err(|_| "300-DPI document lock poisoned".to_string())?;
        if let Some((cached_path, doc)) = document.as_ref() {
            if cached_path.as_path() == path {
                return Ok(Arc::clone(doc));
            }
        }
        self.load_count.fetch_add(1, Ordering::Relaxed);
        let pdfium = get_pdfium()?;
        let doc = pdfium
            .load_pdf_from_file(path, None)
            .map_err(|e| format!("Failed to load PDF: {:?}", e))?;
        let shared = Arc::new(doc);
        *document = Some((path.to_path_buf(), Arc::clone(&shared)));
        Ok(shared)
    }
}

pub(crate) fn get_pdfium() -> Result<&'static Pdfium, String> {
    PDFIUM_INSTANCE.get_or_init(|| {
        let bindings = Pdfium::bind_to_system_library()
            .map_err(|e| format!("Failed to bind to pdfium: {:?}", e))?;
        Ok(Pdfium::new(bindings))
    }).as_ref().map_err(|e| e.clone())
}

pub const MAX_PAGES_PER_IMPORT: usize = 100;

#[allow(dead_code)]
pub fn render_pdf_pages(path: &Path) -> Result<Vec<PageInput>, String> {
    let pdfium = get_pdfium()?;

    let document = pdfium.load_pdf_from_file(path, None)
        .map_err(|e| format!("Failed to load PDF: {:?}", e))?;

    let page_count = document.pages().len() as usize;
    if page_count > MAX_PAGES_PER_IMPORT {
        return Err(format!(
            "Document contains {} pages, which exceeds the limit of {} pages per import. Please split the file into smaller sections.",
            page_count,
            MAX_PAGES_PER_IMPORT
        ));
    }

    let render_dpi = std::env::var("MERGEMARK_RENDER_DPI")
        .unwrap_or_else(|_| "140".to_string())
        .parse::<u32>()
        .unwrap_or(140);
    let target_width = (8.27 * render_dpi as f32).round() as i32;
    let render_config = PdfRenderConfig::new().set_target_width(target_width.try_into().unwrap());

    // Phase 1: Fast sequential pass to extract text, object types, and rasterize page bitmaps
    let mut raw_pages = Vec::with_capacity(document.pages().len() as usize);
    for (i, page) in document.pages().iter().enumerate() {
        let text = page.text().map_err(|e| e.to_string())?.all();
        
        let objects = page.objects();
        let has_images = objects.iter().any(|obj| matches!(obj.object_type(), PdfPageObjectType::Image));
        let has_vectors = objects.iter().any(|obj| matches!(obj.object_type(), PdfPageObjectType::Path));

        if text.trim().is_empty() && !has_images && !has_vectors {
            raw_pages.push((i, text, None));
            continue;
        }

        let bitmap = page.render_with_config(&render_config)
            .map_err(|e| format!("Failed to render page {}: {:?}", i, e))?;

        let img: DynamicImage = bitmap.as_image()
            .map_err(|e| format!("Failed to convert bitmap to image on page {}: {:?}", i, e))?;

        raw_pages.push((i, text, Some(img)));
    }

    // Phase 2: Parallel JPEG compression & Base64 encoding across all CPU cores with Rayon
    use rayon::prelude::*;
    let pages: Result<Vec<PageInput>, String> = raw_pages
        .into_par_iter()
        .map(|(i, text, img_opt)| {
            if let Some(img) = img_opt {
                let rgb_img = img.to_rgb8();
                let mut buf = Cursor::new(Vec::new());
                let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 90);
                encoder.encode_image(&rgb_img)
                    .map_err(|e| format!("Failed to encode jpeg on page {}: {:?}", i, e))?;
                
                let b64 = format!(
                    "data:image/jpeg;base64,{}", 
                    base64::engine::general_purpose::STANDARD.encode(buf.into_inner())
                );

                Ok(PageInput {
                    kind: PageInputKind::Image { b64 },
                    text,
                })
            } else {
                Ok(PageInput {
                    kind: PageInputKind::TextOnly,
                    text,
                })
            }
        })
        .collect();

    pages
}

/// One local text run with its rendered-space bounding box, normalized to
/// `[x, y, w, h]` in 0..1 with y from the top. Font size is not always exposed
/// by pdfium for a segment, so it is optional.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalTextRun {
    pub text: String,
    pub bbox: [f32; 4],
    pub font_size: Option<f32>,
}

/// One long horizontal rule (fraction bar, underline, table rule) in rendered
/// space, normalized like `LocalTextRun`.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalRule {
    pub bbox: [f32; 4],
}

/// One character with its source index on the page text page and its rendered
/// geometry. `index` is the pdfium char index, which is also the character
/// offset in `PageLayoutEvidence::text` (one char per index) — the anchor for
/// exact text replacement.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalChar {
    pub index: usize,
    pub ch: char,
    pub bbox: [f32; 4],
    pub font_size: f32,
}

/// One vector path segment, normalized to page space (y from the top).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    MoveTo,
    LineTo,
    BezierTo,
}

/// A local vector path with its segment kinds and normalized points, used for
/// symbols genuinely drawn as paths (e.g. a radical `√` with its overbar).
#[derive(Debug, Clone, PartialEq)]
pub struct PathEvidence {
    pub bbox: [f32; 4],
    pub segments: Vec<(PathKind, Vec<[f32; 2]>)>,
    pub closed: bool,
}

/// Per-page local layout evidence. Optional: text-only callers never build it,
/// and `pdf_layout_evidence` returns `Err` for an unparseable document so the
/// caller degrades to the plain-text path.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageLayoutEvidence {
    pub page: usize,
    /// Character-indexed reconstruction of the page text (one char per pdfium
    /// char index), so `chars[i].index == i`.
    pub text: String,
    pub chars: Vec<LocalChar>,
    pub runs: Vec<LocalTextRun>,
    pub rules: Vec<LocalRule>,
    pub paths: Vec<PathEvidence>,
    /// True when every pdfium char yielded a Unicode value AND valid bounds, so
    /// `chars[i].index == i` and `text` is the complete source character stream.
    /// When false the caller must fall back to the original page text: an
    /// unmapped char is never silently dropped or erased.
    pub fully_mapped: bool,
}

/// Immutable per-import evidence bundle. Built once for a document and carried
/// on the `PipelineConfig`, so no span reloads the PDF and there is no global
/// path-only cache that could serve stale evidence for a different import.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportEvidence {
    pub pages: Vec<PageLayoutEvidence>,
    /// Geometry-first page layouts (reading-order lines with reconstructed
    /// mathematics), one per page. Empty when the layout could not be built;
    /// the pipeline then keeps the text-layer path.
    pub layout: Vec<crate::layout::LayoutPage>,
}

/// One CONFIRMED right-margin allocation: the exact source char span of the
/// printed `(N)` token, the question it was associated with, and its value.
/// Only confirmed records may drive removal/relocation; every unassigned
/// candidate is preserved.
#[derive(Debug, Clone, PartialEq)]
pub struct MarginRecord {
    pub page: usize,
    pub start: usize,
    pub end: usize,
    pub question: u32,
    pub value: u32,
}

/// Per-import margin association result. Built once, after all question spans
/// are known.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MarginModel {
    pub per_question: std::collections::HashMap<u32, u32>,
    pub records: Vec<MarginRecord>,
}

impl MarginModel {
    /// Confirmed source ranges on one page.
    pub fn ranges_for_page(&self, page: usize) -> Vec<(usize, usize)> {
        self.records
            .iter()
            .filter(|r| r.page == page)
            .map(|r| (r.start, r.end))
            .collect()
    }
}

/// Extract local text-run + rule evidence for every page of `path`. Uses the
/// existing pdfium-render dependency and the same collectors `stroke_census`
/// already uses; the PDF is loaded once. Zero AI calls.
/// True when a long horizontal rule sits vertically BETWEEN `upper` and
/// `lower` (y from the top) and overlaps both horizontally — the geometric
/// signature of a stacked-fraction bar. Without this rule there is no evidence
/// of division, so callers must not fabricate `\frac`.
pub fn fraction_bar_between(
    upper: &LocalTextRun,
    lower: &LocalTextRun,
    rules: &[LocalRule],
) -> bool {
    let top = upper.bbox[1] + upper.bbox[3];
    let bottom = lower.bbox[1];
    if bottom <= top {
        return false;
    }
    let (x0, w0) = (upper.bbox[0], upper.bbox[2]);
    let (x1, w1) = (lower.bbox[0], lower.bbox[2]);
    let overlap_l = x0.max(x1);
    let overlap_r = (x0 + w0).min(x1 + w1);
    if overlap_r <= overlap_l {
        return false;
    }
    rules.iter().any(|r| {
        let ry = r.bbox[1];
        let rh = r.bbox[3];
        let between = ry >= top - 0.004 && ry + rh <= bottom + 0.004;
        let tall = rh <= 0.01;
        let rule_l = r.bbox[0];
        let rule_r = r.bbox[0] + r.bbox[2];
        between && tall && rule_l <= overlap_l + 0.02 && rule_r >= overlap_r - 0.02
    })
}

/// Map a raw PDF-space point into the rendered/normalized page space (page
/// rotation applied, then normalized with y from the top) exactly once.
fn map_point(
    px: f32,
    py: f32,
    raw_w: f32,
    raw_h: f32,
    rotation: &PdfPageRenderRotation,
    eff_w: f32,
    eff_h: f32,
) -> [f32; 2] {
    let raw = crate::geometry::rect_pt(px, px, py, py);
    let (m, _, _) = crate::stroke_census::rotate_rect_for_render(&raw, raw_w, raw_h, rotation);
    let nb = crate::geometry::normalize_pdf_box(m[0], m[1], m[2], m[3], eff_w, eff_h);
    [nb[0], nb[1]]
}

fn walk_path_objects(
    objects: &[PdfPageObject],
    parent: PdfMatrix,
    out: &mut Vec<PathEvidence>,
    raw_w: f32,
    raw_h: f32,
    rotation: &PdfPageRenderRotation,
    eff_w: f32,
    eff_h: f32,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    for obj in objects {
        match obj {
            PdfPageObject::Path(p) => {
                let Ok(pm) = p.matrix() else { continue };
                // `multiply(self, other)` applies self first, then other (proved
                // by pdf_matrix_multiply_applies_child_then_parent), so the path
                // matrix is applied before its container transform.
                let m = pm.multiply(parent);
                let segs = p.segments().transform(m);
                let mut segments: Vec<(PathKind, Vec<[f32; 2]>)> = Vec::new();
                let mut closed = false;
                let mut bad = false;
                for seg in segs.iter() {
                    let kind = match seg.segment_type() {
                        PdfPathSegmentType::MoveTo => PathKind::MoveTo,
                        PdfPathSegmentType::LineTo => PathKind::LineTo,
                        PdfPathSegmentType::BezierTo => PathKind::BezierTo,
                        _ => continue,
                    };
                    let (x, y) = seg.point();
                    if !x.value.is_finite() || !y.value.is_finite() {
                        bad = true;
                        break;
                    }
                    let np = map_point(x.value, y.value, raw_w, raw_h, rotation, eff_w, eff_h);
                    if !np[0].is_finite() || !np[1].is_finite() {
                        bad = true;
                        break;
                    }
                    if seg.is_close() {
                        closed = true;
                    }
                    segments.push((kind, vec![np]));
                }
                if bad || segments.is_empty() {
                    continue;
                }
                let mut bx0 = f32::INFINITY;
                let mut by0 = f32::INFINITY;
                let mut bx1 = 0.0f32;
                let mut by1 = 0.0f32;
                for (_, pts) in &segments {
                    for p in pts {
                        bx0 = bx0.min(p[0]);
                        by0 = by0.min(p[1]);
                        bx1 = bx1.max(p[0]);
                        by1 = by1.max(p[1]);
                    }
                }
                out.push(PathEvidence {
                    bbox: [bx0, by0, bx1 - bx0, by1 - by0],
                    segments,
                    closed,
                });
            }
            PdfPageObject::XObjectForm(f) => {
                let Ok(fm) = f.matrix() else { continue };
                let m = fm.multiply(parent);
                let children: Vec<PdfPageObject> = f.iter().collect();
                walk_path_objects(
                    &children, m, out, raw_w, raw_h, rotation, eff_w, eff_h, depth + 1,
                );
            }
            _ => {}
        }
    }
}

pub fn pdf_layout_evidence(path: &Path) -> Result<Vec<PageLayoutEvidence>, String> {
    let pdfium = get_pdfium()?;
    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| format!("Failed to load PDF: {:?}", e))?;
    let pages = document.pages();
    let mut out = Vec::with_capacity(pages.len() as usize);
    for (idx, page) in pages.iter().enumerate() {
        let raw_w = page.width().value;
        let raw_h = page.height().value;
        if raw_w <= 0.0 || raw_h <= 0.0 {
            out.push(PageLayoutEvidence { page: idx, ..Default::default() });
            continue;
        }
        let rotation = page.rotation().unwrap_or(PdfPageRenderRotation::None);
        let (_, eff_w, eff_h) =
            crate::stroke_census::rotate_rect_for_render(
                &crate::geometry::rect_pt(0.0, raw_w, raw_h, 0.0),
                raw_w,
                raw_h,
                &rotation,
            );
        let (runs, rules) =
            crate::stroke_census::collect_page_layout(&page, raw_w, raw_h, &rotation);
        let norm = |r: &crate::geometry::RectPt| {
            crate::geometry::normalize_pdf_box(r[0], r[1], r[2], r[3], eff_w, eff_h)
        };
        // Character-level evidence: pdfium gives one char per index, with its
        // own tight bounds and scaled font size. This is what distinguishes a
        // raised superscript/stacked digit from a baseline letter.
        let mut chars = Vec::new();
        let mut text = String::new();
        let mut fully_mapped = true;
        if let Ok(text_page) = page.text() {
            for c in text_page.chars().iter() {
                // Never drop a char: an unmapped char becomes a placeholder and
                // marks the page unresolved, so the caller falls back to the
                // original text rather than erasing unknown source.
                let ch = c.unicode_char().unwrap_or('\u{FFFD}');
                let bbox = match c.tight_bounds() {
                    Ok(b) => {
                        let raw = crate::geometry::rect_pt(
                            b.left().value,
                            b.right().value,
                            b.top().value,
                            b.bottom().value,
                        );
                        let (mapped, _, _) = crate::stroke_census::rotate_rect_for_render(
                            &raw, raw_w, raw_h, &rotation,
                        );
                        // Whitespace/newlines legitimately have a zero-size box;
                        // only an actual bounds failure marks the page unresolved.
                        norm(&mapped)
                    }
                    Err(_) => {
                        // A bounds error is a per-glyph geometry issue, not a
                        // Unicode/index identity failure: keep the page mapped
                        // (indices stay contiguous) and let per-edit checks
                        // reject any region that touches this glyph.
                        [0.0, 0.0, 0.0, 0.0]
                    }
                };
                if c.unicode_char().is_none() {
                    fully_mapped = false;
                }
                chars.push(LocalChar {
                    index: c.index() as usize,
                    ch,
                    bbox,
                    font_size: c.scaled_font_size().value,
                });
                text.push(ch);
            }
        }
        let mut paths = Vec::new();
        let root: Vec<PdfPageObject> = page.objects().iter().collect();
        walk_path_objects(
            &root,
            PdfMatrix::IDENTITY,
            &mut paths,
            raw_w,
            raw_h,
            &rotation,
            eff_w,
            eff_h,
            0,
        );
        out.push(PageLayoutEvidence {
            page: idx,
            text,
            chars,
            runs: runs
                .into_iter()
                .map(|(rect, text)| LocalTextRun { text, bbox: norm(&rect), font_size: None })
                .collect(),
            rules: rules
                .into_iter()
                .map(|rect| LocalRule { bbox: norm(&rect) })
                .collect(),
            paths,
            fully_mapped,
        });
    }
    Ok(out)
}

/// Build the immutable per-import evidence bundle once. Returns `None` when the
/// document cannot be parsed, so text-only/synthetic callers keep working.
pub fn load_import_evidence(path: &Path) -> Option<ImportEvidence> {
    let pages = pdf_layout_evidence(path).ok()?;
    let layout = load_layout_pages(path).unwrap_or_default();
    Some(ImportEvidence { pages, layout })
}

/// Build the geometry-first layout of every page (see `layout`). Uses the
/// shared pdfium binding and reads the document's font encodings once.
pub fn load_layout_pages(path: &Path) -> Option<Vec<crate::layout::LayoutPage>> {
    let pdfium = get_pdfium().ok()?;
    let document = pdfium.load_pdf_from_file(path, None).ok()?;
    let catalog = crate::layout::font_catalog(path);
    let glyphs = document.pages().iter().enumerate().map(|(i, page)| crate::layout::extract_page_with(&page, i, catalog.get(i))).collect();
    Some(crate::layout::layout_document(glyphs))
}

/// Repair a page's text using its character evidence. Currently:
///
/// * A spurious line break inserted between two glyphs that are actually on the
///   same visual line (a raised digit immediately followed by a baseline letter,
///   e.g. physics '21 Q14's `9` / `F`) is removed, restoring the printed
///   `9F`. Evidence only — no fraction or product is invented.
///
/// Returns the evidence-reconstructed text when it is available, falling back
/// to the caller's text otherwise so text-only callers are unaffected.
/// Detect a stacked fraction starting at char `i` in the page text:
///
/// ```text
///   9        <- numerator, raised
///   -        <- (implicit bar; the printed letter sits at this level)
///   2 F      <- denominator directly below, variable to the right
/// ```
///
/// Returns `(chars_consumed, "$\frac{9}{2}$F")` only when the geometry proves
/// the stack (same x, denominator below numerator, letter to the right at the
/// bar level). Otherwise `None`.
fn stacked_fraction_at(
    chars: &[char],
    page: &PageLayoutEvidence,
    i: usize,
) -> Option<(usize, String)> {
    let a = page.chars.get(i)?;
    if !a.ch.is_ascii_digit() || a.bbox[2] <= 0.0 || a.bbox[3] <= 0.0 {
        return None;
    }
    let skip_newlines = |mut j: usize| {
        while j < chars.len() && matches!(chars[j], '\r' | '\n') {
            j += 1;
        }
        j
    };
    let j = skip_newlines(i + 1);
    if j == i + 1 || j >= chars.len() || !chars[j].is_ascii_digit() {
        return None;
    }
    let b = page.chars.get(j)?;
    if b.bbox[2] <= 0.0 || b.bbox[3] <= 0.0 {
        return None;
    }
    // Denominator must sit directly below the numerator, within ~2.5 glyph
    // heights and horizontally centred on it (bounded proximity, not "anywhere
    // lower on the page").
    let dy = b.bbox[1] - a.bbox[1];
    let below = dy >= 0.3 * a.bbox[3] && dy <= 2.5 * a.bbox[3];
    let same_x = (b.bbox[0] - a.bbox[0]).abs() <= 0.5 * a.bbox[2];
    if !below || !same_x {
        return None;
    }
    let k = skip_newlines(j + 1);
    if k == j + 1 || k >= chars.len() || !chars[k].is_ascii_alphabetic() {
        return None;
    }
    let v = page.chars.get(k)?;
    if v.bbox[2] <= 0.0 || v.bbox[3] <= 0.0 {
        return None;
    }
    // The variable sits immediately to the right of the stack (within a few
    // glyph widths), at the fraction-bar level.
    let gap = v.bbox[0] - (a.bbox[0] + a.bbox[2]);
    let to_right = gap >= -0.005 && gap <= 3.0 * a.bbox[2];
    let mid_level =
        v.bbox[1] >= a.bbox[1] - 0.2 * a.bbox[3] && v.bbox[1] <= b.bbox[1] + b.bbox[3];
    if !to_right || !mid_level {
        return None;
    }
    Some((
        k - i + 1,
        format!("$\\frac{{{}}}{{{}}}${}", a.ch, b.ch, v.ch),
    ))
}

/// Printed right-margin allocations: `(5)`, `(6)`, … sitting at the far right
/// of their line (no other non-whitespace glyph further right). Returns
/// `(start_index, end_index_exclusive, value, y_center)`. Source-geometry only;
/// body text, table cells and equations are not matched.
pub fn margin_allocations(page: &PageLayoutEvidence) -> Vec<(usize, usize, u32, f32)> {
    let mut out = Vec::new();
    let chars = &page.chars;
    let mut i = 0usize;
    while i < chars.len() {
        let c = &chars[i];
        if c.ch == '(' && c.bbox[0] >= 0.80 && c.bbox[2] > 0.0 {
            let mut j = i + 1;
            let mut digits = String::new();
            while j < chars.len() && chars[j].ch.is_ascii_digit() {
                digits.push(chars[j].ch);
                j += 1;
            }
            if !digits.is_empty() && j < chars.len() && chars[j].ch == ')' {
                let y = c.bbox[1] + c.bbox[3] / 2.0;
                let token_right = chars[j].bbox[0] + chars[j].bbox[2];
                let token_left = c.bbox[0];
                // Isolation: nothing non-whitespace close to the left on the
                // same line (so `x=(5)`, a table cell, or an expression is not
                // mistaken for an isolated margin allocation), and no operator
                // immediately before it.
                let left_neighbour = chars
                    .iter()
                    .filter(|o| {
                        !o.ch.is_whitespace()
                            && (o.bbox[1] + o.bbox[3] / 2.0 - y).abs() <= 0.010
                            && o.bbox[0] + o.bbox[2] < token_left
                    })
                    .max_by(|a, b| {
                        (a.bbox[0] + a.bbox[2])
                            .partial_cmp(&(b.bbox[0] + b.bbox[2]))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                let isolated = left_neighbour
                    .map(|o| token_left - (o.bbox[0] + o.bbox[2]) >= 0.05)
                    .unwrap_or(true);
                let operator_before = left_neighbour
                    .map(|o| matches!(o.ch, '=' | '+' | '-' | '*' | '/' | '('))
                    .unwrap_or(false);
                let rightmost = !chars.iter().enumerate().any(|(k, o)| {
                    !(k >= i && k <= j)
                        && !o.ch.is_whitespace()
                        && (o.bbox[1] + o.bbox[3] / 2.0 - y).abs() <= 0.012
                        && o.bbox[0] + o.bbox[2] > token_right + 0.005
                });
                if rightmost && isolated && !operator_before {
                    out.push((c.index, chars[j].index + 1, digits.parse().unwrap_or(0), y));
                    i = j + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}


/// Printed answer-line labels (`cyclotron =`, `cost =`) whose y matches a long
/// answer rule to their right, plus the right-margin answer-box number that
/// shares that line. Only an alphabetic label ending in `=` qualifies, so a real
/// `cost = 10` (no answer rule) and an expression continued on the next line
/// are preserved. The replacement covers the exact source char span and is
/// rejected if any interposed non-whitespace glyph is not part of the label.
fn answer_prompt_edits(page: &PageLayoutEvidence) -> Vec<(usize, usize, String)> {
    let mut edits = Vec::new();
    for rule in &page.rules {
        let r = rule.bbox;
        if r[2] < 0.12 {
            continue;
        }
        let ry = r[1] + r[3] / 2.0;
        let mut line: Vec<&LocalChar> = page
            .chars
            .iter()
            .filter(|c| {
                !c.ch.is_whitespace()
                    && c.bbox[2] > 0.0
                    && (c.bbox[1] + c.bbox[3] / 2.0 - ry).abs() <= 0.008
                    && c.bbox[0] + c.bbox[2] <= r[0] + 0.005
            })
            .collect();
        line.sort_by(|a, b| a.bbox[0].partial_cmp(&b.bbox[0]).unwrap_or(std::cmp::Ordering::Equal));
        if line.last().map(|c| c.ch) != Some('=') {
            continue;
        }
        if !line.iter().all(|c| c.ch.is_ascii_alphabetic() || c.ch == '=') {
            continue;
        }
        let lo = line.first().unwrap().index;
        let hi = line.last().unwrap().index + 1;
        // Exact consumption: nothing non-whitespace may sit between the label
        // chars in the source span.
        let clean = page
            .chars
            .iter()
            .filter(|c| c.index >= lo && c.index < hi && !c.ch.is_whitespace())
            .all(|c| line.iter().any(|l| l.index == c.index));
        if !clean {
            continue;
        }
        edits.push((lo, hi, String::new()));
        // Also remove a right-margin answer-box number on the same line.
        let mut right_digits: Vec<&LocalChar> = page
            .chars
            .iter()
            .filter(|c| {
                c.ch.is_ascii_digit()
                    && c.bbox[0] >= 0.88
                    && (c.bbox[1] + c.bbox[3] / 2.0 - ry).abs() <= 0.015
            })
            .collect();
        if !right_digits.is_empty() && right_digits.len() <= 3 {
            right_digits.sort_by(|a, b| a.bbox[0].partial_cmp(&b.bbox[0]).unwrap_or(std::cmp::Ordering::Equal));
            let rlo = right_digits.first().unwrap().index;
            let rhi = right_digits.last().unwrap().index + 1;
            // Exact consumption: every non-whitespace glyph in the span must be
            // one of the box digits (no interposed value/operator).
            let clean = page
                .chars
                .iter()
                .filter(|c| c.index >= rlo && c.index < rhi && !c.ch.is_whitespace())
                .all(|c| right_digits.iter().any(|d| d.index == c.index));
            if clean {
                edits.push((rlo, rhi, String::new()));
            }
        }
    }
    edits
}

/// Bounded inline script attach: a body letter followed immediately (adjacent
/// glyphs) by a single alphabetic subscript and a short numeric superscript,
/// e.g. `E_k^{1.5}`. Requires adjacency, both scripts, and exact consumption,
/// which keeps ordinary variable/word pairs and number runs untouched.
fn inline_script_edits(page: &PageLayoutEvidence) -> Vec<(usize, usize, String)> {
    let mut edits = Vec::new();
    let chars = &page.chars;
    if chars.is_empty() {
        return edits;
    }
    let mut i = 0usize;
    while i < chars.len() {
        let base = &chars[i];
        // Local base size, not a whole-page maximum.
        if !base.ch.is_ascii_alphabetic()
            || base.font_size <= 0.0
            || base.bbox[2] <= 0.0
            || base.bbox[3] <= 0.0
        {
            i += 1;
            continue;
        }
        let base_size = base.font_size;
        let base_h = base.bbox[3];
        let small = base_size * 0.85;
        let mut last = base;
        let mut last_idx = i;
        let mut subs: Vec<&LocalChar> = Vec::new();
        let mut sups: Vec<&LocalChar> = Vec::new();
        let mut consumed: Vec<usize> = vec![base.index];
        let mut pos = i + 1;
        while pos < chars.len() {
            let c = &chars[pos];
            if matches!(c.ch, '\r' | '\n') {
                pos += 1;
                continue;
            }
            if c.font_size > small || c.bbox[2] <= 0.0 || c.bbox[3] <= 0.0 {
                break;
            }
            let gap = c.bbox[0] - (last.bbox[0] + last.bbox[2]);
            if !(-0.006..=0.012).contains(&gap) {
                break;
            }
            let base_bottom = base.bbox[1] + base.bbox[3];
            let c_bottom = c.bbox[1] + c.bbox[3];
            let lowered = c_bottom - base_bottom;
            let raised = base_bottom - c_bottom;
            // Bounded lowered / raised position relative to the local base,
            // measured by glyph bottom so a short period in a superscript run
            // is not mistaken for a subscript.
            if lowered > 0.1 * base_h && lowered <= 1.5 * base_h {
                subs.push(c);
            } else if raised > 0.1 * base_h && raised <= 1.5 * base_h {
                sups.push(c);
            } else {
                break; // baseline small number or too far → not a script
            }
            consumed.push(c.index);
            last = c;
            last_idx = pos;
            pos += 1;
        }
        if subs.len() == 1
            && subs[0].ch.is_ascii_alphabetic()
            && !sups.is_empty()
            && sups.len() <= 4
            && sups.iter().all(|c| c.ch.is_ascii_digit() || c.ch == '.')
        {
            let lo = base.index;
            let hi = chars[last_idx].index + 1;
            let clean = chars
                .iter()
                .filter(|c| c.index >= lo && c.index < hi && !c.ch.is_whitespace())
                .all(|c| consumed.contains(&c.index));
            if clean {
                let sup: String = sups.iter().map(|c| c.ch).collect();
                edits.push((
                    lo,
                    hi,
                    format!("${}_{{{}}}^{{{}}}$", base.ch, subs[0].ch, sup),
                ));
                i = last_idx + 1;
                continue;
            }
        }
        i += 1;
    }
    edits
}

/// Inline radical: a recognized radical path whose radicand glyphs sit under
/// its overbar. Replaces the radicand span with `\sqrt{...}`; the drawn radical
/// itself has no source char to consume.
fn inline_radical_edits(page: &PageLayoutEvidence) -> Vec<(usize, usize, String)> {
    let mut edits = Vec::new();
    for p in &page.paths {
        let Some(bar) = radical_overbar(p) else { continue };
        // Radicand = every non-whitespace glyph under the overbar AND within the
        // path's vertical band, captured in source order.
        let mut picked: Vec<&LocalChar> = page
            .chars
            .iter()
            .filter(|c| {
                !c.ch.is_whitespace()
                    && c.bbox[1] >= p.bbox[1] - 0.012
                    && c.bbox[1] <= p.bbox[1] + p.bbox[3] + 0.012
                    && c.bbox[0] >= bar[0] - 0.012
                    && c.bbox[0] + c.bbox[2] <= bar[1] + 0.012
            })
            .collect();
        if picked.is_empty() {
            continue;
        }
        picked.sort_by(|a, b| a.index.cmp(&b.index));
        let mut rad = String::new();
        for c in &picked {
            let ch = if c.ch == '\u{2212}' { '-' } else { c.ch };
            if ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '/' | '.' | '(' | ')') {
                rad.push(ch);
            } else {
                rad.clear();
                break;
            }
        }
        if rad.is_empty() {
            continue; // unsupported glyph inside the radicand → leave unchanged
        }
        let lo = picked.first().unwrap().index;
        let hi = picked.last().unwrap().index + 1;
        // Exact consumption: every non-whitespace source glyph in the span must
        // be one of the captured radicand glyphs.
        let clean = page
            .chars
            .iter()
            .filter(|c| c.index >= lo && c.index < hi && !c.ch.is_whitespace())
            .all(|c| picked.iter().any(|p| p.index == c.index));
        if clean {
            edits.push((lo, hi, format!("$\\sqrt{{{}}}$", rad)));
        }
    }
    edits
}

fn is_left_bracket(ch: char) -> bool {
    matches!(ch, '\u{f8eb}' | '\u{f8ec}' | '\u{f8ed}')
}
fn is_right_bracket(ch: char) -> bool {
    matches!(ch, '\u{f8f6}' | '\u{f8f7}' | '\u{f8f8}')
}

/// A radical path: a connected check-mark stroke with a near-horizontal overbar
/// to its right. Requires a single open sub-path of straight segments with a
/// dip below the bar and a long horizontal run; rejects closed polygons,
/// disconnected MoveTo points, Bezier strokes and small axis/graph shapes.
fn radical_overbar(path: &PathEvidence) -> Option<[f32; 2]> {
    if path.closed {
        return None;
    }
    let b = path.bbox;
    if b[2] <= 0.0 || b[3] <= 0.0 || b[2] > 0.06 || b[3] > 0.04 {
        return None;
    }
    let mut moves = 0usize;
    let mut curves = 0usize;
    let mut pts: Vec<[f32; 2]> = Vec::new();
    for (kind, ps) in &path.segments {
        match kind {
            PathKind::MoveTo => moves += 1,
            PathKind::BezierTo => curves += 1,
            PathKind::LineTo => {}
        }
        for p in ps {
            if !p[0].is_finite() || !p[1].is_finite() {
                return None;
            }
            pts.push(*p);
        }
    }
    if moves != 1 || curves > 0 || pts.len() < 4 {
        return None;
    }
    // A long near-horizontal run = the overbar.
    let mut bar: Option<[f32; 2]> = None;
    for w in pts.windows(2) {
        let (a, c) = (w[0], w[1]);
        if (a[1] - c[1]).abs() <= 0.15 * b[3] && (a[0] - c[0]).abs() >= 0.35 * b[2] {
            bar = Some([a[0].min(c[0]), a[0].max(c[0])]);
            break;
        }
    }
    let bar = bar?;
    // A dip below the bar level (the check-mark valley), left of the bar.
    let has_dip = pts
        .iter()
        .any(|p| p[1] > b[1] + 0.45 * b[3] && p[0] < bar[0] + 0.2 * b[2]);
    if !has_dip {
        return None;
    }
    Some(bar)
}

enum Item<'a> {
    Ch(&'a LocalChar),
    /// bar x-extent, vertical band (y0,y1), path identity.
    Rad([f32; 2], [f32; 2], usize),
}

fn render_matrix_cell(cell: &[&(f32, f32, Item)]) -> Option<(String, Vec<usize>, Vec<usize>)> {
    let mut s = String::new();
    let mut used: Vec<usize> = Vec::new();
    let mut used_paths: Vec<usize> = Vec::new();
    let mut i = 0usize;
    while i < cell.len() {
        match &cell[i].2 {
            Item::Ch(c) => {
                let ch = if c.ch == '\u{2212}' { '-' } else { c.ch };
                if ch.is_ascii_digit()
                    || ch == '-'
                    || ch == '+'
                    || ch == '/'
                    || ch.is_ascii_alphabetic()
                    || ch == '('
                    || ch == ')'
                {
                    s.push(ch);
                    used.push(c.index);
                } else {
                    return None;
                }
            }
            Item::Rad(bar, band, pid) => {
                // Radicand: following glyphs under the overbar AND inside the
                // path's vertical band.
                let mut rad = String::new();
                let mut j = i + 1;
                while j < cell.len() {
                    if let Item::Ch(c) = &cell[j].2 {
                        let cy = c.bbox[1] + c.bbox[3] / 2.0;
                        if (c.ch.is_ascii_alphanumeric() || matches!(c.ch, '+' | '-' | '/' | '.' | '(' | ')'))
                            && c.bbox[0] >= bar[0] - 0.012
                            && c.bbox[0] + c.bbox[2] <= bar[1] + 0.012
                            && cy >= band[0] - 0.012
                            && cy <= band[1] + 0.012
                        {
                            rad.push(c.ch);
                            used.push(c.index);
                            j += 1;
                            continue;
                        }
                    }
                    break;
                }
                if rad.is_empty() {
                    return None;
                }
                s.push_str(&format!("\\sqrt{{{}}}", rad));
                used_paths.push(*pid);
                i = j;
                continue;
            }
        }
        i += 1;
    }
    Some((s, used, used_paths))
}

fn render_matrix_rows(
    rows: &[Vec<(f32, f32, Item)>],
) -> Option<(String, Vec<usize>, Vec<usize>)> {
    // Cluster each row into cells by x gaps.
    let mut row_cells: Vec<Vec<Vec<&(f32, f32, Item)>>> = Vec::new();
    for row in rows {
        let mut cells: Vec<Vec<&(f32, f32, Item)>> = Vec::new();
        for it in row {
            if let Some(c) = cells.last_mut().filter(|c| it.0 - c.last().unwrap().0 < 0.025) {
                c.push(it);
            } else {
                cells.push(vec![it]);
            }
        }
        if cells.is_empty() {
            return None;
        }
        row_cells.push(cells);
    }
    // Global column grid from all cell centers.
    let mut centers: Vec<f32> = Vec::new();
    for cells in &row_cells {
        for c in cells {
            centers.push(c.iter().map(|it| it.0).sum::<f32>() / c.len() as f32);
        }
    }
    centers.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut col_centers: Vec<f32> = Vec::new();
    for cx in centers {
        if let Some(last) = col_centers.last_mut() {
            if cx - *last < 0.02 {
                *last = (*last + cx) / 2.0;
                continue;
            }
        }
        col_centers.push(cx);
    }
    let ncols = col_centers.len();
    if ncols == 0 {
        return None;
    }
    let mut body = String::new();
    let mut used: Vec<usize> = Vec::new();
    let mut used_paths: Vec<usize> = Vec::new();
    for (ri, cells) in row_cells.iter().enumerate() {
        // Consistent grid: every row must have exactly one cell per column.
        if cells.len() != ncols {
            return None;
        }
        let mut ordered: Vec<Option<&Vec<&(f32, f32, Item)>>> = vec![None; ncols];
        for c in cells {
            let cx: f32 = c.iter().map(|it| it.0).sum::<f32>() / c.len() as f32;
            let ci = col_centers
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| {
                    (cx - *a).abs().partial_cmp(&(cx - *b).abs()).unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(i, _)| i)?;
            if ordered[ci].is_some() {
                return None;
            }
            ordered[ci] = Some(c);
        }
        let mut cell_strs: Vec<String> = Vec::new();
        for slot in ordered {
            let cell = slot?;
            let (t, u, up) = render_matrix_cell(cell)?;
            cell_strs.push(t);
            used.extend(u);
            used_paths.extend(up);
        }
        if ri > 0 {
            body.push_str(" \\\\ ");
        }
        body.push_str(&cell_strs.join(" & "));
    }
    Some((
        format!("$$\\begin{{pmatrix}}{}\\end{{pmatrix}}$$", body),
        used,
        used_paths,
    ))
}

/// Reconstruct source matrices from bracket-piece geometry + entry glyphs and
/// radical paths. Rows/columns/entries come from captured source values; an
/// ambiguous group is skipped (left flagged) rather than guessed.
fn reconstruct_matrices(page: &PageLayoutEvidence) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let lefts: Vec<&LocalChar> = page.chars.iter().filter(|c| is_left_bracket(c.ch)).collect();
    let rights: Vec<&LocalChar> = page.chars.iter().filter(|c| is_right_bracket(c.ch)).collect();
    if lefts.is_empty() || rights.is_empty() {
        return out;
    }
    fn group_by_x<'a>(items: &[&'a LocalChar]) -> Vec<Vec<&'a LocalChar>> {
        // First cluster by x, then split each cluster by vertical gaps so two
        // vertically separate matrices sharing x do not merge.
        let mut xgroups: Vec<Vec<&'a LocalChar>> = Vec::new();
        for c in items {
            if let Some(g) = xgroups.iter_mut().find(|g| (g[0].bbox[0] - c.bbox[0]).abs() < 0.01) {
                g.push(c);
            } else {
                xgroups.push(vec![*c]);
            }
        }
        let mut groups: Vec<Vec<&'a LocalChar>> = Vec::new();
        for mut g in xgroups {
            g.sort_by(|a, b| a.bbox[1].partial_cmp(&b.bbox[1]).unwrap_or(std::cmp::Ordering::Equal));
            let mut cur: Vec<&'a LocalChar> = Vec::new();
            for c in g {
                if let Some(last) = cur.last() {
                    let gap = c.bbox[1] - (last.bbox[1] + last.bbox[3]);
                    if gap > 0.03 {
                        groups.push(std::mem::take(&mut cur));
                    }
                }
                cur.push(c);
            }
            if !cur.is_empty() {
                groups.push(cur);
            }
        }
        groups
    }
    for lg in group_by_x(&lefts) {
        let lx = lg.iter().map(|c| c.bbox[0]).fold(f32::INFINITY, f32::min);
        let ltop = lg.iter().map(|c| c.bbox[1]).fold(f32::INFINITY, f32::min);
        let lbot = lg.iter().map(|c| c.bbox[1] + c.bbox[3]).fold(0.0f32, f32::max);
        let rg = group_by_x(&rights)
            .into_iter()
            .filter(|rg| {
                let rx = rg.iter().map(|c| c.bbox[0]).fold(f32::INFINITY, f32::min);
                let rtop = rg.iter().map(|c| c.bbox[1]).fold(f32::INFINITY, f32::min);
                let rbot = rg.iter().map(|c| c.bbox[1] + c.bbox[3]).fold(0.0f32, f32::max);
                rx > lx
                    && (lbot.min(rbot) - ltop.max(rtop)) > -0.01
                    && (ltop - rtop).abs() <= 0.015
                    && (lbot - rbot).abs() <= 0.015
            })
            .min_by(|a, b| {
                let ax = a.iter().map(|c| c.bbox[0]).fold(f32::INFINITY, f32::min);
                let bx = b.iter().map(|c| c.bbox[0]).fold(f32::INFINITY, f32::min);
                ax.partial_cmp(&bx).unwrap_or(std::cmp::Ordering::Equal)
            });
        let Some(rg) = rg else { continue };
        let rx = rg.iter().map(|c| c.bbox[0] + c.bbox[2]).fold(0.0f32, f32::max);
        let rtop = rg.iter().map(|c| c.bbox[1]).fold(f32::INFINITY, f32::min);
        let rbot = rg.iter().map(|c| c.bbox[1] + c.bbox[3]).fold(0.0f32, f32::max);
        let (top, bottom) = (ltop.min(rtop), lbot.max(rbot));
        if rx <= lx || bottom <= top {
            continue;
        }
        let inside = |b: &[f32; 4]| {
            b[0] > lx && b[0] + b[2] < rx && b[1] >= top - 0.005 && b[1] + b[3] <= bottom + 0.005
        };
        let mut items: Vec<(f32, f32, Item)> = Vec::new();
        for c in &page.chars {
            if is_left_bracket(c.ch) || is_right_bracket(c.ch) || c.ch.is_whitespace() {
                continue;
            }
            if inside(&c.bbox) {
                items.push((c.bbox[0], c.bbox[1] + c.bbox[3] / 2.0, Item::Ch(c)));
            }
        }
        for (pid, p) in page.paths.iter().enumerate() {
            if inside(&p.bbox) {
                if let Some(bar) = radical_overbar(p) {
                    items.push((
                        p.bbox[0],
                        p.bbox[1] + p.bbox[3] / 2.0,
                        Item::Rad(bar, [p.bbox[1], p.bbox[1] + p.bbox[3]], pid),
                    ));
                }
            }
        }
        if items.is_empty() {
            continue;
        }
        items.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let mut rows: Vec<Vec<(f32, f32, Item)>> = Vec::new();
        for it in items {
            if let Some(r) = rows.last_mut().filter(|r| (it.1 - r[0].1).abs() < 0.012) {
                r.push(it);
            } else {
                rows.push(vec![it]);
            }
        }
        for r in rows.iter_mut() {
            r.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        }
        let Some((render, used, used_paths)) = render_matrix_rows(&rows) else { continue };
        let path_set: std::collections::BTreeSet<usize> = used_paths.iter().copied().collect();
        if path_set.len() != used_paths.len() {
            continue; // a radical path was reused
        }
        let mut all: Vec<usize> = used;
        all.extend(lg.iter().chain(rg.iter()).map(|c| c.index));
        let set: std::collections::BTreeSet<usize> = all.iter().copied().collect();
        if set.len() != all.len() {
            continue; // a source glyph was consumed twice
        }
        let lo = *set.iter().min().unwrap();
        let hi = *set.iter().max().unwrap() + 1;
        // Exact consumption: every non-whitespace source glyph in the interval
        // must be consumed exactly once (same invariant as equation repairs).
        let span: Vec<usize> = page
            .chars
            .iter()
            .filter(|c| c.index >= lo && c.index < hi && !c.ch.is_whitespace())
            .map(|c| c.index)
            .collect();
        if set.len() != span.len() || span.iter().any(|i| !set.contains(i)) {
            continue;
        }
        out.push((lo, hi, render));
    }
    out
}

pub fn repair_page_text_with_evidence(
    base: &str,
    page: &PageLayoutEvidence,
    remove_ranges: &[(usize, usize)],
) -> String {
    // Unresolved evidence (any unmapped char / bad bounds) must never erase the
    // original source: fall back to the caller's text unchanged.
    if page.text.is_empty() || !page.fully_mapped {
        return base.to_string();
    }
    // Explicit source-index → text-offset alignment check: every char index
    // must equal its offset and cover the whole string. Gapped/inconsistent
    // indices fall back to the original text rather than mis-splicing.
    let tlen = page.text.chars().count();
    if page.chars.len() != tlen || page.chars.iter().enumerate().any(|(i, c)| c.index != i) {
        return base.to_string();
    }
    // Compose ALL repairs (local equations + stacked fractions) against the
    // ORIGINAL mapped source, then reject overlapping replacement intervals
    // before applying them descending. Both repair kinds can coexist on a page.
    let mut edits = reconstruct_equations(page);
    edits.extend(stacked_fraction_edits(page));
    edits.extend(answer_prompt_edits(page));
    edits.extend(reconstruct_matrices(page));
    edits.extend(inline_script_edits(page));
    edits.extend(inline_radical_edits(page));
    // Confirmed margin-allocation records: exact source char spans only.
    for &(lo, hi) in remove_ranges {
        edits.push((lo, hi, String::new()));
    }
    edits.sort_by_key(|(lo, _, _)| *lo);
    let mut accepted: Vec<(usize, usize, String)> = Vec::new();
    let mut last_hi = 0usize;
    for (lo, hi, repl) in edits {
        if lo >= hi || hi > tlen || lo < last_hi {
            continue; // malformed or overlapping interval → drop
        }
        // Per-glyph geometry: reject an edit whose source span touches a glyph
        // with missing/non-finite/zero geometry (whitespace may lack geometry).
        let geometry_ok = page
            .chars
            .iter()
            .filter(|c| c.index >= lo && c.index < hi && !c.ch.is_whitespace())
            .all(|c| {
                c.bbox.iter().all(|v| v.is_finite())
                    && c.bbox[2] > 0.0
                    && c.bbox[3] > 0.0
            });
        if !geometry_ok {
            continue;
        }
        last_hi = hi;
        accepted.push((lo, hi, repl));
    }
    if accepted.is_empty() {
        return base.to_string();
    }
    let mut chars: Vec<char> = page.text.chars().collect();
    for (lo, hi, repl) in accepted.into_iter().rev() {
        chars.splice(lo..hi, repl.chars());
    }
    chars.into_iter().collect()
}

/// Render one horizontal group of glyphs as a LaTeX sub-expression, deriving
/// every symbol/exponent/subscript from the actual glyphs.
///
/// Returns `None` unless EVERY non-whitespace glyph in `group` is rendered
/// exactly once: an unassigned small glyph, or a small glyph that could attach
/// to two adjacent bases, is ambiguous and rejected rather than silently
/// dropped or duplicated.
fn render_group(group: &[&LocalChar]) -> Option<(String, Vec<usize>)> {
    if group.is_empty() {
        return None;
    }
    let body_max = group.iter().map(|c| c.font_size).fold(0.0, f32::max);
    let thresh = (body_max * 0.8).max(9.0);
    let mut bodies: Vec<&LocalChar> = group
        .iter()
        .copied()
        .filter(|c| c.font_size >= thresh && c.bbox[2] > 0.0)
        .collect();
    bodies.sort_by(|a, b| a.bbox[0].partial_cmp(&b.bbox[0]).unwrap_or(std::cmp::Ordering::Equal));
    let smalls: Vec<&LocalChar> = group
        .iter()
        .copied()
        .filter(|c| c.font_size < thresh && c.bbox[2] > 0.0)
        .collect();
    if bodies.is_empty() {
        return None;
    }
    // Assign each small glyph to exactly one body; ambiguous/unassigned rejects.
    let mut assigned: Vec<Vec<&LocalChar>> = vec![Vec::new(); bodies.len()];
    for s in &smalls {
        if s.bbox[3] <= 0.0 {
            return None;
        }
        let mut hit: Option<usize> = None;
        for (bi, b) in bodies.iter().enumerate() {
            let lo = b.bbox[0] - 0.4 * b.bbox[2];
            let hi = b.bbox[0] + b.bbox[2] + 0.4 * b.bbox[2];
            if s.bbox[0] >= lo && s.bbox[0] <= hi {
                if hit.is_some() {
                    return None; // attaches to two bases → ambiguous
                }
                hit = Some(bi);
            }
        }
        match hit {
            Some(bi) => assigned[bi].push(*s),
            None => return None, // unassigned glyph would be dropped
        }
    }
    let mut out = String::new();
    let mut consumed: Vec<usize> = Vec::new();
    for (bi, b) in bodies.iter().enumerate() {
        out.push(b.ch);
        consumed.push(b.index);
        for above in [true, false] {
            let mut picked: Vec<&LocalChar> = assigned[bi]
                .iter()
                .copied()
                .filter(|s| (s.bbox[1] < b.bbox[1]) == above)
                .collect();
            picked.sort_by(|a, c| a.bbox[0].partial_cmp(&c.bbox[0]).unwrap_or(std::cmp::Ordering::Equal));
            if picked.is_empty() {
                continue;
            }
            let body: String = picked.iter().map(|s| s.ch).collect();
            if above {
                out.push_str(&format!("^{{{}}}", body));
            } else {
                out.push_str(&format!("_{{{}}}", body));
            }
            consumed.extend(picked.iter().map(|s| s.index));
        }
    }
    Some((out, consumed))
}

/// Reconstruct local display equations from glyph clusters + thin fraction-bar
/// rules. Returns `(char_start, char_end_exclusive, latex)` spans. Every symbol,
/// exponent, subscript and operator comes from the glyphs; the replacement span
/// is the exact source char range of the cluster, and the cluster is rejected
/// if any interposed char lies outside it (so prose is never overwritten).
fn reconstruct_equations(page: &PageLayoutEvidence) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    for rule in &page.rules {
        let bar = rule.bbox;
        if bar[2] < 0.03 || bar[2] > 0.35 || bar[3] > 0.01 {
            continue;
        }
        let in_x = |c: &LocalChar| {
            let cx = c.bbox[0] + c.bbox[2] / 2.0;
            cx >= bar[0] - 0.002 && cx <= bar[0] + bar[2] + 0.002
        };
        let ok = |c: &LocalChar| c.bbox[2] > 0.0 && c.bbox[3] > 0.0 && !c.ch.is_whitespace();
        let num: Vec<&LocalChar> = page
            .chars
            .iter()
            .filter(|c| {
                ok(c) && in_x(c)
                    && c.bbox[1] + c.bbox[3] <= bar[1] + 0.004
                    && c.bbox[1] >= bar[1] - 0.04
            })
            .collect();
        let den: Vec<&LocalChar> = page
            .chars
            .iter()
            .filter(|c| {
                ok(c) && in_x(c)
                    && c.bbox[1] >= bar[1] + bar[3] - 0.004
                    && c.bbox[1] <= bar[1] + bar[3] + 0.045
            })
            .collect();
        if num.is_empty() || den.is_empty() {
            continue;
        }
        let lhs: Vec<&LocalChar> = page
            .chars
            .iter()
            .filter(|c| {
                ok(c) && c.bbox[0] + c.bbox[2] <= bar[0] + 0.002
                    && c.bbox[0] >= bar[0] - 0.07
                    && (c.bbox[1] - bar[1]).abs() <= 0.02
            })
            .collect();
        // Every group must render cleanly (no dropped/duplicated glyph).
        let Some((num_s, mut consumed)) = render_group(&num) else {
            continue;
        };
        let Some((den_s, den_used)) = render_group(&den) else {
            continue;
        };
        let (lhs_s, lhs_used) = if lhs.is_empty() {
            (String::new(), Vec::new())
        } else {
            match render_group(&lhs) {
                Some(v) => v,
                None => continue,
            }
        };
        consumed.extend(den_used);
        consumed.extend(lhs_used);
        let used: std::collections::BTreeSet<usize> = consumed.iter().copied().collect();
        if used.len() != consumed.len() {
            continue; // a glyph was consumed twice
        }
        let lo = *used.iter().min().unwrap();
        let hi = *used.iter().max().unwrap() + 1;
        // Every NON-WHITESPACE source glyph in the replacement span must be
        // consumed exactly once. No bbox-only exception: an unconsumed glyph is
        // a source-loss risk and rejects the rewrite.
        let in_span: Vec<&LocalChar> = page
            .chars
            .iter()
            .filter(|c| c.index >= lo && c.index < hi && !c.ch.is_whitespace())
            .collect();
        if in_span.iter().any(|c| !used.contains(&c.index)) || used.len() != in_span.len() {
            continue;
        }
        let latex = format!("${lhs_s}\\frac{{{num_s}}}{{{den_s}}}$");
        out.push((lo, hi, latex));
    }
    out
}

/// Per-character stacked-fraction / intra-line-join edits, as exact source
/// character spans. These compose with the equation edits in the caller.
fn stacked_fraction_edits(page: &PageLayoutEvidence) -> Vec<(usize, usize, String)> {
    let chars: Vec<char> = page.text.chars().collect();
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        // (1) Stacked fraction: numerator digit, then a digit directly BELOW it
        // at the same x, then a letter to the right at the fraction-bar level.
        // This is physics '21 Q14's printed `9/2 F` etc. Rebuild
        // `$\frac{9}{2}$F` from the real glyph geometry — the denominator is
        // never dropped.
        if c.is_ascii_digit() {
            if let Some((consumed, replacement)) = stacked_fraction_at(&chars, page, i) {
                edits.push((i, i + consumed, replacement));
                i += consumed;
                continue;
            }
        }
        // Try to join digit [newline] letter when the two glyphs share a line.
        if c.is_ascii_digit() && i + 2 < chars.len() && matches!(chars[i + 1], '\r' | '\n') {
            let mut j = i + 1;
            while j < chars.len() && matches!(chars[j], '\r' | '\n') {
                j += 1;
            }
            if j < chars.len() && chars[j].is_ascii_alphabetic() {
                if let (Some(a), Some(b)) = (page.chars.get(i), page.chars.get(j)) {
                    let a_bottom = a.bbox[1] + a.bbox[3];
                    let b_top = b.bbox[1];
                    let b_bottom = b.bbox[1] + b.bbox[3];
                    let same_line = a_bottom >= b_top - 0.2 * b.bbox[3] && a_bottom <= b_bottom;
                    let a_right = a.bbox[0] + a.bbox[2];
                    let adjacent = (a_right - b.bbox[0]).abs() <= 0.03;
                    if same_line && adjacent {
                        edits.push((i, j + 1, format!("{}{}", c, chars[j])));
                        i = j + 1; // consumed the digit, newlines and the letter
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    edits
}

/// A figure region detected deterministically from the PDF content stream.
#[derive(Debug, Clone, PartialEq)]
pub struct DetectedFigure {
    /// Normalized [x, y, w, h] in 0..1, y from the top — matches the vision
    /// schema used for model-proposed diagram boxes.
    pub bbox: [f32; 4],
    /// "Figure 1" caption text if a matching text segment was found.
    pub caption: Option<String>,
    /// Semantic kind inferred from the caption ("graph", "circuit", …).
    pub kind: Option<String>,
    /// Smart Scissors deterministic confidence (0..1): seed strength, grid
    /// presence, label richness, caption presence, ink normality. Recorded
    /// for Phase-2 consumers (pack quality metadata, Tier-0 gating); no
    /// pipeline behaviour depends on it yet.
    pub seg_confidence: f32,
    /// Printed MCQ option letter ("A".."E") whose text block sits inside or
    /// immediately above this region — the positional evidence used to bind
    /// per-option diagrams to their letter. `None` when no letter was captured,
    /// in which case the caller must NOT guess an association.
    pub option_label: Option<String>,
}

/// Detect figures on every page of a PDF from its vector content stream —
/// zero AI calls. Returns one `Vec<DetectedFigure>` per page, 0-indexed and
/// aligned with `render_pdf_pages`. A page with nothing figure-like yields an
/// empty `Vec`; an unparseable document yields an `Err` (callers degrade to
/// the vision path, which keeps the import working for scanned PDFs).
pub fn detect_pdf_figures(path: &Path) -> Result<Vec<Vec<DetectedFigure>>, String> {
    let pdfium = get_pdfium()?;
    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| format!("Failed to load PDF: {:?}", e))?;
    let pages = document.pages();
    let mut result = Vec::with_capacity(pages.len() as usize);
    for page in pages.iter() {
        result.push(detect_page_figures_inner(&page));
    }

    // Golden-fixture authoring aid: MERGEMARK_FIGURE_DEBUG_JSON=<path>
    // writes every detected box so a human can review/correct the output
    // into committed goldens. Never set in production.
    if let Ok(dump_path) = std::env::var("MERGEMARK_FIGURE_DEBUG_JSON") {
        dump_debug_json(&dump_path, path, &result);
    }
    Ok(result)
}

/// Write the golden-curation debug dump (see `detect_pdf_figures`).
fn dump_debug_json(out_path: &str, source: &Path, per_page: &[Vec<DetectedFigure>]) {
    use serde_json::json;
    let pages: Vec<_> = per_page
        .iter()
        .enumerate()
        .filter(|(_, figs)| !figs.is_empty())
        .map(|(idx, figs)| {
            json!({
                "index": idx,
                "figures": figs.iter().map(|f| json!({
                    "bbox": f.bbox,
                    "caption": f.caption,
                    "kind": f.kind,
                    "seg_confidence": (f.seg_confidence * 100.0).round() / 100.0,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let doc = json!({
        "source_pdf": source.file_name().and_then(|s| s.to_str()).unwrap_or("unknown"),
        "generator": concat!("mergemark/", env!("CARGO_PKG_VERSION"), " stroke_census"),
        "note": "UNCURATED candidate boxes — human review required before committing as golden",
        "pages": pages,
    });
    match serde_json::to_string_pretty(&doc) {
        Ok(body) => {
            if let Err(e) = std::fs::write(out_path, body) {
                eprintln!("[FIGURE_DEBUG] could not write {}: {}", out_path, e);
            } else {
                eprintln!("[FIGURE_DEBUG] candidate boxes written to {}", out_path);
            }
        }
        Err(e) => eprintln!("[FIGURE_DEBUG] serialization failed: {}", e),
    }
}

/// Per-page detection: delegated entirely to Smart Scissors
/// (`stroke_census::detect_page_figures`), which types every content-stream
/// primitive, builds text barriers, grows regions seed-first, and captures
/// internal labels — all deterministic, zero AI calls. A page with nothing
/// figure-like yields an empty `Vec`; an unparseable document yields an
/// `Err` (callers degrade to the vision path, which keeps the import working
/// for scanned PDFs).
fn detect_page_figures_inner(page: &PdfPage) -> Vec<DetectedFigure> {
    crate::stroke_census::detect_page_figures(page)
}

/// Serialize every pdfium touch made by tests. Pdfium's C API is not
/// thread-safe (see `PageRenderCache::render`) and production is naturally
/// serialized by `AppState::extraction_in_progress`, but the test suite runs
/// several pdfium-heavy fixtures concurrently — under load this manifested
/// as sporadic `FormatError`s and even STATUS_HEAP_CORRUPTION. Tests take
/// this lock for the duration of their pdfium work.
#[cfg(test)]
pub(crate) fn pdfium_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Extract each page's text layer via pdfium (cheap, no rendering). Used by
/// the deterministic figure detector's verification and diagnostics.
#[allow(dead_code)]
pub fn pdf_page_texts(path: &Path) -> Result<Vec<String>, String> {
    let pdfium = get_pdfium()?;
    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| format!("Failed to load PDF: {:?}", e))?;
    let mut texts = Vec::with_capacity(document.pages().len() as usize);
    for page in document.pages().iter() {
        texts.push(page.text().map(|t| t.all()).unwrap_or_default());
    }
    Ok(texts)
}

pub fn load_and_optimize_image_file(path: &Path) -> Result<PageInput, String> {
    let img = image::open(path).map_err(|e| format!("Failed to open image: {}", e))?;
    let (w, h) = (img.width(), img.height());
    let max_dim: u32 = 2048;
    let final_img = if w > max_dim || h > max_dim {
        let scale = max_dim as f32 / (w.max(h) as f32);
        let new_w = (w as f32 * scale).round().max(1.0) as u32;
        let new_h = (h as f32 * scale).round().max(1.0) as u32;
        image::DynamicImage::ImageRgba8(image::imageops::resize(
            &img,
            new_w,
            new_h,
            image::imageops::FilterType::Triangle,
        ))
    } else {
        img
    };

    let rgb_img = final_img.to_rgb8();
    let mut buf = Cursor::new(Vec::new());
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 90);
    encoder.encode_image(&rgb_img)
        .map_err(|e| format!("Failed to encode jpeg: {}", e))?;

    let b64 = format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(buf.into_inner())
    );

    Ok(PageInput {
        kind: PageInputKind::Image { b64 },
        text: String::new(),
    })
}

/// Default render DPI for figure-crop pages. Matches the pre-knob hardcoded
/// 300 DPI (A4 width 2480px) so crops stay pixel-identical unless a user opts
/// into a lower resolution.
const DEFAULT_FIGURE_RENDER_DPI: u32 = 300;

/// Render resolution for figure-crop pages. Env override
/// `MERGEMARK_FIGURE_RENDER_DPI` ∈ [96, 300] trades crop fidelity for ~4×
/// faster renders (e.g. 150 DPI) without a code change; the default keeps
/// today's 300-DPI output byte-for-byte identical.
fn figure_render_dpi() -> u32 {
    std::env::var("MERGEMARK_FIGURE_RENDER_DPI")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .map(|dpi| dpi.clamp(96, DEFAULT_FIGURE_RENDER_DPI))
        .unwrap_or(DEFAULT_FIGURE_RENDER_DPI)
}

/// Target render width for a given DPI. Scaled from the 300-DPI baseline
/// (2480px = 8.27in × 300), so the default reproduces today's pixels exactly
/// while lower DPIs shrink the render proportionally.
fn figure_target_width(dpi: u32) -> i32 {
    let width =
        ((2480u32 as f32 * dpi as f32 / DEFAULT_FIGURE_RENDER_DPI as f32).round() as u32).max(1);
    width.min(i32::MAX as u32) as i32
}

/// Render one page of an already-loaded document at the given DPI — the body
/// of the old `render_pdf_page_at_300dpi` minus the per-call document parse.
pub fn render_page_from_document(
    doc: &PdfDocument,
    page_idx: usize,
    dpi: u32,
) -> Result<image::DynamicImage, String> {
    let pages = doc.pages();
    if page_idx >= pages.len() as usize {
        return Err(format!("Page index {} out of bounds", page_idx));
    }

    let page = pages
        .get((page_idx as u16).into())
        .map_err(|e| format!("Failed to get page: {:?}", e))?;

    let render_config = PdfRenderConfig::new().set_target_width(figure_target_width(dpi));
    let bitmap = page
        .render_with_config(&render_config)
        .map_err(|e| format!("Failed to render page: {:?}", e))?;

    bitmap
        .as_image()
        .map_err(|e| format!("Failed to convert bitmap to image: {:?}", e))
}

#[allow(dead_code)]
pub fn render_pdf_page_at_300dpi(path: &Path, page_idx: usize) -> Result<image::DynamicImage, String> {
    let pdfium = get_pdfium()?;

    let document = pdfium.load_pdf_from_file(path, None)
        .map_err(|e| format!("Failed to load PDF: {:?}", e))?;

    render_page_from_document(&document, page_idx, DEFAULT_FIGURE_RENDER_DPI)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(y: f32, h: f32, x: f32, w: f32, text: &str) -> LocalTextRun {
        LocalTextRun { text: text.into(), bbox: [x, y, w, h], font_size: None }
    }
    fn rule(y: f32, x: f32, w: f32) -> LocalRule {
        LocalRule { bbox: [x, y, w, 0.002] }
    }

    #[test]
    fn character_evidence_reconstructs_stacked_fraction_and_rejects_non_stack() {
        let mk = |index: usize, ch: char, bbox: [f32; 4], size: f32| LocalChar {
            index,
            ch,
            bbox,
            font_size: size,
        };
        // "9" raised, "2" directly below at the same x, "F" to the right at
        // the bar level -> a stacked fraction, exactly physics '21 Q14.
        let text = "of 9\r\n2\r\nF\r\n".to_string();
        let mut chars = Vec::new();
        for (i, c) in text.chars().enumerate() {
            let bbox = match c {
                '9' => [0.35, 0.360, 0.010, 0.010],
                '2' => [0.35, 0.380, 0.010, 0.010],
                'F' => [0.37, 0.370, 0.014, 0.010],
                _ => [0.0, 0.0, 0.0, 0.0],
            };
            chars.push(mk(i, c, bbox, 12.0));
        }
        let page = PageLayoutEvidence { page: 0, text: text.clone(), chars, runs: vec![], rules: vec![], paths: vec![], fully_mapped: true };
        let repaired = repair_page_text_with_evidence("ignored", &page, &[]);
        assert!(
            repaired.contains("$\\frac{9}{2}$F"),
            "stacked fraction not reconstructed: {repaired:?}"
        );

        // Same text but the denominator is far to the side: NOT a stack, so the
        // source must be left unchanged (no guessed fraction).
        let mut chars2 = page.chars.clone();
        chars2[6].bbox = [0.60, 0.380, 0.010, 0.010];
        let page2 = PageLayoutEvidence { page: 0, text: text.clone(), chars: chars2, runs: vec![], rules: vec![], paths: vec![], fully_mapped: true };
        let repaired2 = repair_page_text_with_evidence("ignored", &page2, &[]);
        assert!(
            !repaired2.contains("\\frac"),
            "a non-stacked layout must not become a fraction: {repaired2:?}"
        );

        // Denominator far BELOW the numerator (more than ~2.5 glyph heights)
        // must not be treated as a stack.
        let mut chars3 = page.chars.clone();
        chars3[6].bbox = [0.35, 0.360 + 0.06, 0.010, 0.010];
        let page3 = PageLayoutEvidence { page: 0, text, chars: chars3, runs: vec![], rules: vec![], paths: vec![], fully_mapped: true };
        let repaired3 = repair_page_text_with_evidence("ignored", &page3, &[]);
        assert!(!repaired3.contains("\\frac"), "far-below denominator: {repaired3:?}");

        // Unresolved bounds (zero bbox) must not produce a fraction.
        let mut chars4 = page.chars.clone();
        chars4[6].bbox = [0.0, 0.0, 0.0, 0.0];
        let page4 = PageLayoutEvidence { page: 0, text: page.text.clone(), chars: chars4, runs: vec![], rules: vec![], paths: vec![], fully_mapped: true };
        assert!(!repair_page_text_with_evidence("ignored", &page4, &[]).contains("\\frac"));
    }

    #[test]
    fn char_evidence_reconstructs_local_equation_cluster_and_rejects_scatter() {
        // Real physics '21 Q5c glyph geometry (from pdf_layout_evidence): the
        // `2` superscripts sit above e/B/R, the denominator `2 m_p` below the
        // bar, and `E_k =` to the left.
        let mk = |index: usize, ch: char, bbox: [f32; 4], size: f32| LocalChar {
            index,
            ch,
            bbox,
            font_size: size,
        };
        let spec: [(char, [f32; 4], f32); 12] = [
            ('2', [0.4852, 0.4972, 0.0051, 0.0056], 6.97),
            ('2', [0.5060, 0.4972, 0.0051, 0.0056], 6.97),
            ('2', [0.5269, 0.4972, 0.0051, 0.0056], 6.97),
            ('B', [0.4920, 0.4997, 0.0123, 0.0095], 12.06),
            ('R', [0.5128, 0.4997, 0.0121, 0.0095], 12.06),
            ('e', [0.4760, 0.5028, 0.0080, 0.0065], 12.06),
            ('E', [0.4326, 0.5087, 0.0132, 0.0095], 12.06),
            ('=', [0.4587, 0.5128, 0.0103, 0.0034], 12.06),
            ('k', [0.4454, 0.5162, 0.0058, 0.0057], 6.97),
            ('2', [0.4892, 0.5197, 0.0088, 0.0097], 12.06),
            ('m', [0.4995, 0.5231, 0.0133, 0.0065], 12.06),
            ('p', [0.5133, 0.5293, 0.0055, 0.0056], 6.97),
        ];
        let text: String = spec.iter().map(|(c, _, _)| *c).collect();
        let chars: Vec<LocalChar> = spec
            .iter()
            .enumerate()
            .map(|(i, (c, b, s))| mk(i, *c, *b, *s))
            .collect();
        let bar = LocalRule { bbox: [0.4734, 0.5139, 0.0629, 0.0014] };
        let page = PageLayoutEvidence {
            page: 0,
            text: text.clone(),
            chars: chars.clone(),
            runs: vec![],
            rules: vec![bar.clone()],
            paths: vec![],
            fully_mapped: true,
        };
        let repaired = repair_page_text_with_evidence("ignored", &page, &[]);
        assert!(
            repaired.contains("\\frac{e^{2}B^{2}R^{2}}{2m_{p}}"),
            "local equation not reconstructed: {repaired:?}"
        );
        assert!(repaired.contains("E_{k}="), "LHS from glyphs: {repaired:?}");

        // Same glyphs but NO bar rule: no equation may be invented.
        let page_nobar = PageLayoutEvidence { rules: vec![], ..page.clone() };
        assert!(!repair_page_text_with_evidence("ignored", &page_nobar, &[]).contains("\\frac"));

        // Altered exponent glyphs must show up verbatim (e^{3}B^{2}R^{2}).
        let mut chars_alt = chars.clone();
        chars_alt[0].ch = '3';
        let page_alt = PageLayoutEvidence { chars: chars_alt, ..page.clone() };
        let repaired_alt = repair_page_text_with_evidence("ignored", &page_alt, &[]);
        assert!(repaired_alt.contains("e^{3}"), "altered exponent: {repaired_alt:?}");

        // Interposed prose INSIDE the source char span, far outside the cluster
        // bbox, must reject the rewrite entirely (no prose overwrite).
        let mut spec_prose: Vec<(char, [f32; 4], f32)> = spec.to_vec();
        spec_prose.insert(6, ('x', [0.10, 0.10, 0.01, 0.01], 12.06));
        let chars_prose: Vec<LocalChar> = spec_prose
            .iter()
            .enumerate()
            .map(|(i, (c, b, s))| mk(i, *c, *b, *s))
            .collect();
        let text_prose: String = spec_prose.iter().map(|(c, _, _)| *c).collect();
        let page_prose = PageLayoutEvidence {
            text: text_prose,
            chars: chars_prose,
            ..page.clone()
        };
        assert!(
            !repair_page_text_with_evidence("ignored", &page_prose, &[]).contains("\\frac"),
            "interposed prose must not be overwritten"
        );

        // An unresolved page (unmapped char / bad bounds) falls back unchanged.
        let page_unresolved = PageLayoutEvidence { fully_mapped: false, ..page.clone() };
        assert_eq!(repair_page_text_with_evidence("ORIGINAL", &page_unresolved, &[]), "ORIGINAL");
    }

    #[test]
    fn replacement_engine_rejects_ambiguity_overlap_and_index_gaps() {
        let mk = |index: usize, ch: char, bbox: [f32; 4], size: f32| LocalChar {
            index,
            ch,
            bbox,
            font_size: size,
        };
        let spec: [(char, [f32; 4], f32); 12] = [
            ('2', [0.4852, 0.4972, 0.0051, 0.0056], 6.97),
            ('2', [0.5060, 0.4972, 0.0051, 0.0056], 6.97),
            ('2', [0.5269, 0.4972, 0.0051, 0.0056], 6.97),
            ('B', [0.4920, 0.4997, 0.0123, 0.0095], 12.06),
            ('R', [0.5128, 0.4997, 0.0121, 0.0095], 12.06),
            ('e', [0.4760, 0.5028, 0.0080, 0.0065], 12.06),
            ('E', [0.4326, 0.5087, 0.0132, 0.0095], 12.06),
            ('=', [0.4587, 0.5128, 0.0103, 0.0034], 12.06),
            ('k', [0.4454, 0.5162, 0.0058, 0.0057], 6.97),
            ('2', [0.4892, 0.5197, 0.0088, 0.0097], 12.06),
            ('m', [0.4995, 0.5231, 0.0133, 0.0065], 12.06),
            ('p', [0.5133, 0.5293, 0.0055, 0.0056], 6.97),
        ];
        let build = |spec: &[(char, [f32; 4], f32)], rules: Vec<LocalRule>| PageLayoutEvidence {
            page: 0,
            text: spec.iter().map(|(c, _, _)| *c).collect(),
            chars: spec.iter().enumerate().map(|(i, (c, b, s))| mk(i, *c, *b, *s)).collect(),
            runs: vec![],
            rules,
            paths: vec![],
            fully_mapped: true,
        };
        let bar = LocalRule { bbox: [0.4734, 0.5139, 0.0629, 0.0014] };

        // (a) Ambiguous superscript: a small glyph overlapping two body spans
        // must reject the whole equation rather than duplicate it.
        let mut spec_amb = spec.to_vec();
        spec_amb[0].1 = [0.48715, 0.4972, 0.0051, 0.0056];
        let page_amb = build(&spec_amb, vec![bar.clone()]);
        assert!(
            !repair_page_text_with_evidence("ignored", &page_amb, &[]).contains("\\frac"),
            "ambiguous superscripts must reject"
        );

        // (b) An extra non-whitespace operator inside the replacement span but
        // outside every group must reject (it would otherwise be overwritten).
        let mut spec_extra: Vec<(char, [f32; 4], f32)> = spec.to_vec();
        spec_extra.insert(6, ('-', [0.500, 0.470, 0.005, 0.005], 12.06));
        let page_extra = build(&spec_extra, vec![bar.clone()]);
        assert!(
            !repair_page_text_with_evidence("ignored", &page_extra, &[]).contains("\\frac"),
            "unconsumed operator must reject"
        );

        // (c) Two overlapping fraction bars: dedup to a single edit.
        let page_overlap = build(&spec, vec![bar.clone(), bar.clone()]);
        let repaired_overlap = repair_page_text_with_evidence("ignored", &page_overlap, &[]);
        assert_eq!(
            repaired_overlap.matches("\\frac").count(),
            1,
            "overlapping bars must apply once: {repaired_overlap:?}"
        );

        // (d) Equation AND a separate stacked fraction on one page: both apply.
        let mut spec_mixed = spec.to_vec();
        for (c, b, s) in [
            ('9', [0.35, 0.360, 0.010, 0.010], 12.0f32),
            ('\r', [0.0, 0.0, 0.0, 0.0], 1.0),
            ('\n', [0.0, 0.0, 0.0, 0.0], 1.0),
            ('2', [0.35, 0.380, 0.010, 0.010], 12.0),
            ('\r', [0.0, 0.0, 0.0, 0.0], 1.0),
            ('\n', [0.0, 0.0, 0.0, 0.0], 1.0),
            ('F', [0.37, 0.370, 0.014, 0.010], 12.0),
        ] {
            spec_mixed.push((c, b, s));
        }
        let page_mixed = build(&spec_mixed, vec![bar.clone()]);
        let repaired_mixed = repair_page_text_with_evidence("ignored", &page_mixed, &[]);
        assert!(
            repaired_mixed.contains("\\frac{e^{2}B^{2}R^{2}}{2m_{p}}")
                && repaired_mixed.contains("$\\frac{9}{2}$F"),
            "equation + stacked fraction must both apply: {repaired_mixed:?}"
        );

        // (e) Gapped indices must fall back to the original text.
        let mut page_gap = build(&spec, vec![bar.clone()]);
        page_gap.chars[5].index = 99;
        assert_eq!(repair_page_text_with_evidence("ORIGINAL", &page_gap, &[]), "ORIGINAL");
    }

    #[test]
    fn margin_allocations_require_isolation() {
        let mk = |index: usize, ch: char, bbox: [f32; 4]| LocalChar {
            index,
            ch,
            bbox,
            font_size: 12.0,
        };
        let page = |chars: Vec<LocalChar>| {
            let text: String = chars.iter().map(|c| c.ch).collect();
            PageLayoutEvidence { page: 0, text, chars, runs: vec![], rules: vec![], paths: vec![], fully_mapped: true }
        };
        // Isolated (5) at the right margin with a big left gap → allocation.
        let isolated = page(vec![
            mk(0, '(', [0.84, 0.50, 0.006, 0.010]),
            mk(1, '5', [0.848, 0.50, 0.006, 0.010]),
            mk(2, ')', [0.856, 0.50, 0.006, 0.010]),
        ]);
        assert_eq!(margin_allocations(&isolated).len(), 1);

        // `x=(5)`: an operator immediately before → not an allocation.
        let expr = page(vec![
            mk(0, 'x', [0.60, 0.50, 0.010, 0.010]),
            mk(1, '=', [0.615, 0.50, 0.010, 0.010]),
            mk(2, '(', [0.84, 0.50, 0.006, 0.010]),
            mk(3, '5', [0.848, 0.50, 0.006, 0.010]),
            mk(4, ')', [0.856, 0.50, 0.006, 0.010]),
        ]);
        assert!(margin_allocations(&expr).is_empty());

        // Rightmost table cell `(5)` with a value close to its left → not an
        // allocation.
        let cell = page(vec![
            mk(0, '9', [0.80, 0.50, 0.010, 0.010]),
            mk(1, '(', [0.84, 0.50, 0.006, 0.010]),
            mk(2, '5', [0.848, 0.50, 0.006, 0.010]),
            mk(3, ')', [0.856, 0.50, 0.006, 0.010]),
        ]);
        assert!(margin_allocations(&cell).is_empty());
    }

    #[test]
    fn inline_radical_requires_glyphs_under_the_overbar() {
        let lc = |index: usize, ch: char, x: f32, y: f32| LocalChar {
            index,
            ch,
            bbox: [x, y, 0.006, 0.010],
            font_size: 12.0,
        };
        let rad = PathEvidence {
            bbox: [0.52, 0.50, 0.05, 0.02],
            segments: vec![
                (PathKind::MoveTo, vec![[0.512, 0.512]]),
                (PathKind::LineTo, vec![[0.516, 0.510]]),
                (PathKind::LineTo, vec![[0.520, 0.518]]),
                (PathKind::LineTo, vec![[0.524, 0.500]]),
                (PathKind::LineTo, vec![[0.560, 0.500]]),
            ],
            closed: false,
        };
        let page = |chars: Vec<LocalChar>| PageLayoutEvidence {
            page: 0,
            text: chars.iter().map(|c| c.ch).collect(),
            chars,
            runs: vec![],
            rules: vec![],
            paths: vec![rad.clone()],
            fully_mapped: true,
        };
        // Glyph under the overbar and in-band → radicand.
        let under = page(vec![lc(0, '3', 0.535, 0.502)]);
        assert_eq!(inline_radical_edits(&under).len(), 1);
        // Glyph beyond the overbar extent → no radicand, unchanged.
        let outside = page(vec![lc(0, '3', 0.60, 0.502)]);
        assert!(inline_radical_edits(&outside).is_empty());
        // Glyph under the bar but far below the path band → unchanged.
        let far = page(vec![lc(0, '3', 0.535, 0.60)]);
        assert!(inline_radical_edits(&far).is_empty());
    }

    #[test]
    fn inline_radical_keeps_operators_and_rejects_unsupported() {
        let lc = |index: usize, ch: char, x: f32, y: f32| LocalChar {
            index,
            ch,
            bbox: [x, y, 0.006, 0.010],
            font_size: 12.0,
        };
        let rad = |bar0: f32, bar1: f32| PathEvidence {
            bbox: [bar0 - 0.01, 0.50, (bar1 - bar0) + 0.02, 0.02],
            segments: vec![
                (PathKind::MoveTo, vec![[bar0 - 0.008, 0.512]]),
                (PathKind::LineTo, vec![[bar0 - 0.004, 0.510]]),
                (PathKind::LineTo, vec![[bar0, 0.518]]),
                (PathKind::LineTo, vec![[bar0 + 0.004, 0.500]]),
                (PathKind::LineTo, vec![[bar1, 0.500]]),
            ],
            closed: false,
        };
        let page = |chars: Vec<LocalChar>| PageLayoutEvidence {
            page: 0,
            text: chars.iter().map(|c| c.ch).collect(),
            chars,
            runs: vec![],
            rules: vec![],
            paths: vec![rad(0.50, 0.53)],
            fully_mapped: true,
        };
        // 2+3 under the bar → \sqrt{2+3}, surrounding text preserved.
        let p1 = page(vec![
            lc(0, 'A', 0.40, 0.50),
            lc(1, '2', 0.505, 0.502),
            lc(2, '+', 0.512, 0.502),
            lc(3, '3', 0.519, 0.502),
            lc(4, 'B', 0.56, 0.50),
        ]);
        let edits = inline_radical_edits(&p1);
        assert_eq!(edits.len(), 1, "{edits:?}");
        assert_eq!(edits[0].2, "$\\sqrt{2+3}$");
        // Unsupported glyph inside the radicand → unchanged.
        let p2 = page(vec![
            lc(0, '2', 0.505, 0.502),
            lc(1, '\u{2605}', 0.512, 0.502),
            lc(2, '3', 0.519, 0.502),
        ]);
        assert!(inline_radical_edits(&p2).is_empty());
    }

    #[test]
    fn matrix_grid_supports_rectangular_and_column() {
        let lc = |index: usize, ch: char, x: f32, y: f32| LocalChar {
            index,
            ch,
            bbox: [x, y, 0.006, 0.010],
            font_size: 12.0,
        };
        let br = |index: usize, ch: char, x: f32, y: f32| LocalChar {
            index,
            ch,
            bbox: [x, y, 0.006, 0.03],
            font_size: 12.0,
        };
        let brackets = |rx0: f32| {
            vec![
                br(0, '\u{f8eb}', 0.50, 0.40),
                br(1, '\u{f8ec}', 0.50, 0.41),
                br(2, '\u{f8ed}', 0.50, 0.42),
                br(3, '\u{f8f6}', rx0, 0.40),
                br(4, '\u{f8f7}', rx0, 0.41),
                br(5, '\u{f8f8}', rx0, 0.42),
            ]
        };
        let build = |mut chars: Vec<LocalChar>| {
            for (i, c) in chars.iter_mut().enumerate() {
                c.index = i;
            }
            PageLayoutEvidence {
                page: 0,
                text: chars.iter().map(|c| c.ch).collect(),
                chars,
                runs: vec![],
                rules: vec![],
                paths: vec![],
                fully_mapped: true,
            }
        };
        // 2x3 signed matrix.
        let mut chars = brackets(0.62);
        for (ch, x, y) in [
            ('-', 0.52, 0.405),
            ('1', 0.53, 0.405),
            ('2', 0.56, 0.405),
            ('-', 0.59, 0.405),
            ('3', 0.60, 0.405),
            ('4', 0.52, 0.435),
            ('-', 0.55, 0.435),
            ('5', 0.56, 0.435),
            ('6', 0.60, 0.435),
        ] {
            chars.push(lc(0, ch, x, y));
        }
        let edits = reconstruct_matrices(&build(chars));
        assert!(
            edits
                .iter()
                .any(|(_, _, s)| s.contains("-1 & 2 & -3") && s.contains("4 & -5 & 6")),
            "2x3 not reconstructed: {edits:?}"
        );
        // 3x1 column vector.
        let mut chars = brackets(0.56);
        for (ch, y) in [('-', 0.405), ('2', 0.425), ('3', 0.440)] {
            chars.push(lc(0, ch, 0.52, y));
        }
        let edits = reconstruct_matrices(&build(chars));
        assert!(
            edits.iter().any(|(_, _, s)| s.contains("\\begin{pmatrix}- \\\\ 2 \\\\ 3\\end{pmatrix}")),
            "3x1 not reconstructed: {edits:?}"
        );
    }

    #[test]
    fn inline_script_requires_local_vertical_bounds() {
        let lc = |index: usize, ch: char, x: f32, y: f32, size: f32| LocalChar {
            index,
            ch,
            bbox: [x, y, 0.006, 0.010],
            font_size: size,
        };
        let page = |chars: Vec<LocalChar>| PageLayoutEvidence {
            page: 0,
            text: chars.iter().map(|c| c.ch).collect(),
            chars,
            runs: vec![],
            rules: vec![],
            paths: vec![],
            fully_mapped: true,
        };
        let q5 = page(vec![
            lc(0, 'E', 0.6456, 0.1312, 12.0),
            lc(1, 'k', 0.6584, 0.1347, 7.98),
            lc(2, '1', 0.6667, 0.1289, 7.98),
            LocalChar { index: 3, ch: '.', bbox: [0.6724, 0.1345, 0.0022, 0.002], font_size: 7.98 },
            lc(4, '5', 0.6758, 0.1291, 7.98),
        ]);
        let edits = inline_script_edits(&q5);
        assert_eq!(edits.len(), 1, "{edits:?}");
        assert_eq!(edits[0].2, "$E_{k}^{1.5}$");
        // Later-line same-x small text (far below) must not attach.
        let later = page(vec![
            lc(0, 'E', 0.6456, 0.1312, 12.0),
            lc(1, 'k', 0.6584, 0.1347, 7.98),
            lc(2, '9', 0.6667, 0.300, 7.98),
        ]);
        assert!(inline_script_edits(&later).is_empty());
        // Baseline small number (same line) must not attach.
        let baseline = page(vec![
            lc(0, 'E', 0.6456, 0.1312, 12.0),
            lc(1, '2', 0.6667, 0.1312, 7.98),
        ]);
        assert!(inline_script_edits(&baseline).is_empty());
    }

    #[test]
    fn repair_eligibility_separates_geometry_from_mapping() {
        let lc = |index: usize, ch: char, x: f32, y: f32| LocalChar {
            index,
            ch,
            bbox: [x, y, 0.006, 0.010],
            font_size: 12.0,
        };
        let bad = |index: usize| LocalChar { index, ch: 'Z', bbox: [0.0, 0.0, 0.0, 0.0], font_size: 12.0 };
        let mk_page = |extra: Option<LocalChar>, fully_mapped: bool| {
            let mut chars = vec![
                lc(0, 'o', 0.34, 0.36),
                lc(1, '9', 0.35, 0.360),
                lc(2, '\r', 0.0, 0.0),
                lc(3, '\n', 0.0, 0.0),
                lc(4, '2', 0.35, 0.380),
                lc(5, '\r', 0.0, 0.0),
                lc(6, '\n', 0.0, 0.0),
                lc(7, 'F', 0.37, 0.370),
            ];
            if let Some(b) = extra {
                chars.push(b);
            }
            PageLayoutEvidence {
                page: 0,
                text: chars.iter().map(|c| c.ch).collect(),
                chars,
                runs: vec![],
                rules: vec![],
                paths: vec![],
                fully_mapped,
            }
        };
        // Unrelated bad bounds → the local fraction repair still applies.
        let p = mk_page(Some(bad(8)), true);
        let out = repair_page_text_with_evidence("orig", &p, &[]);
        assert!(out.contains("$\\frac{9}{2}$F"), "unrelated bad bounds blocked repair: {out:?}");
        // Bad bounds INSIDE the candidate → unchanged.
        let mut p2 = mk_page(None, true);
        p2.chars[4].bbox = [0.0, 0.0, 0.0, 0.0];
        assert_eq!(repair_page_text_with_evidence("ORIG", &p2, &[]), "ORIG");
        // Unknown Unicode (fully_mapped=false) → whole-page fallback.
        assert_eq!(repair_page_text_with_evidence("ORIG", &mk_page(None, false), &[]), "ORIG");
        // Index gap → whole-page fallback.
        let mut p3 = mk_page(None, true);
        p3.chars[4].index = 99;
        assert_eq!(repair_page_text_with_evidence("ORIG", &p3, &[]), "ORIG");
    }

    #[test]
    fn map_point_applies_page_rotation() {
        // Unrotated: the PDF bottom-left corner maps to the normalized bottom.
        let origin = map_point(0.0, 0.0, 100.0, 200.0, &PdfPageRenderRotation::None, 100.0, 200.0);
        assert!(origin[0].abs() < 1e-4 && (origin[1] - 1.0).abs() < 1e-4, "{origin:?}");
        // 180° rotation moves the same point across both axes.
        let flipped = map_point(0.0, 0.0, 100.0, 200.0, &PdfPageRenderRotation::Degrees180, 100.0, 200.0);
        assert!((flipped[0] - origin[0]).abs() > 0.5 && (flipped[1] - origin[1]).abs() > 0.5, "{origin:?} {flipped:?}");
    }

    #[test]
    fn pdf_matrix_multiply_applies_child_then_parent() {
        // Prove the SDK composition order so the walker applies the path matrix
        // before its container transform (scale and translate do not commute).
        let parent = PdfMatrix::new(2.0, 0.0, 0.0, 2.0, 0.0, 0.0);
        let child = PdfMatrix::new(1.0, 0.0, 0.0, 1.0, 1.0, 0.0);
        let self_then_other = parent.multiply(child);
        let (x1, _) = self_then_other.apply_to_points(PdfPoints::new(1.0), PdfPoints::new(0.0));
        assert!((x1.value - 3.0).abs() < 1e-4, "multiply(self,other) applies self first: {}", x1.value);
        let child_then_parent = child.multiply(parent);
        let (x2, _) = child_then_parent.apply_to_points(PdfPoints::new(1.0), PdfPoints::new(0.0));
        assert!((x2.value - 4.0).abs() < 1e-4, "child then parent: {}", x2.value);
    }

    #[test]
    fn two_vertically_separate_matrices_do_not_merge() {
        let mut chars: Vec<LocalChar> = Vec::new();
        let mut push = |ch: char, x: f32, y: f32, h: f32| {
            let index = chars.len();
            chars.push(LocalChar { index, ch, bbox: [x, y, 0.008, h], font_size: 12.0 });
        };
        // Each matrix's brackets and entry are contiguous in source order.
        for (dy, digit, sx, sy) in [(0.10f32, '1', 0.52f32, 0.112f32), (0.30, '2', 0.52, 0.312)] {
            for (ch, y) in [('\u{f8eb}', dy), ('\u{f8ec}', dy + 0.008), ('\u{f8ed}', dy + 0.016)] {
                push(ch, 0.50, y, 0.03);
            }
            push(digit, sx, sy, 0.010);
            for (ch, y) in [('\u{f8f6}', dy), ('\u{f8f7}', dy + 0.008), ('\u{f8f8}', dy + 0.016)] {
                push(ch, 0.55, y, 0.03);
            }
            if dy < 0.2 {
                push('|', 0.70, 0.22, 0.010);
            }
        }
        drop(push);
        let text: String = chars.iter().map(|c| c.ch).collect();
        let page = PageLayoutEvidence { page: 0, text, chars, runs: vec![], rules: vec![], paths: vec![], fully_mapped: true };
        let edits = reconstruct_matrices(&page);
        assert_eq!(edits.len(), 2, "expected two independent matrices: {edits:?}");
        let latex: Vec<&str> = edits.iter().map(|(_, _, s)| s.as_str()).collect();
        assert!(latex.iter().any(|s| s.contains("\\begin{pmatrix}1\\end{pmatrix}")), "{latex:?}");
        assert!(latex.iter().any(|s| s.contains("\\begin{pmatrix}2\\end{pmatrix}")), "{latex:?}");
        // The separator glyph is outside both bracket intervals and must remain.
        let repaired = repair_page_text_with_evidence(&page.text, &page, &[]);
        assert!(repaired.contains('|'), "separator lost: {repaired:?}");
    }

    #[test]
    fn radical_requires_check_shape_and_overbar() {
        let mk = |segs: Vec<(PathKind, Vec<[f32; 2]>)>| PathEvidence {
            bbox: [0.5, 0.5, 0.02, 0.015],
            segments: segs,
            closed: false,
        };
        // A check-mark with a top overbar → radical.
        let radical = mk(vec![
            (PathKind::MoveTo, vec![[0.500, 0.510]]),
            (PathKind::LineTo, vec![[0.504, 0.505]]),
            (PathKind::LineTo, vec![[0.508, 0.515]]),
            (PathKind::LineTo, vec![[0.512, 0.500]]),
            (PathKind::LineTo, vec![[0.520, 0.500]]),
        ]);
        assert!(radical_overbar(&radical).is_some());
        // A plain graph line (no top bar) → not a radical.
        let line = mk(vec![
            (PathKind::MoveTo, vec![[0.500, 0.515]]),
            (PathKind::LineTo, vec![[0.520, 0.500]]),
        ]);
        assert!(radical_overbar(&line).is_none());
        // A closed figure (rectangle) → not a radical.
        let rect = PathEvidence {
            bbox: [0.5, 0.5, 0.02, 0.015],
            closed: true,
            segments: vec![(PathKind::MoveTo, vec![[0.500, 0.500]])],
        };
        assert!(radical_overbar(&rect).is_none());
    }

    #[test]
    fn core_pure_2021_matrix_is_reconstructed_from_source_paths() {
        let _guard = pdfium_test_lock();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../past papers for mergemark/core pure 1 '21.pdf");
        if !path.exists() {
            eprintln!("[MX] fixture missing");
            return;
        }
        let ev = match pdf_layout_evidence(&path) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[MX] pdfium unavailable: {e}");
                return;
            }
        };
        let edits = reconstruct_matrices(&ev[1]);
        assert!(
            edits.iter().any(|(_, _, s)| s.contains(
                "\\begin{pmatrix}-4 & -4\\sqrt{3} \\\\ 4\\sqrt{3} & -4\\end{pmatrix}"
            )),
            "matrix not reconstructed: {edits:?}"
        );
    }

    #[test]
    fn fraction_bar_detection_requires_a_bar_between_the_runs() {
        // "3" above "R" with a bar between them is a stacked fraction.
        let upper = run(0.10, 0.03, 0.40, 0.05, "3");
        let lower = run(0.15, 0.03, 0.40, 0.05, "R");
        assert!(fraction_bar_between(&upper, &lower, &[rule(0.135, 0.38, 0.10)]));
        // No bar: the two lines may be a product / value+unit — not evidence.
        assert!(!fraction_bar_between(&upper, &lower, &[]));
        // A bar that spans the mid-point but is far to the side is unrelated.
        assert!(!fraction_bar_between(&upper, &lower, &[rule(0.135, 0.05, 0.10)]));
        // A thick "bar" (a filled box) is not a fraction rule.
        let thick = LocalRule { bbox: [0.38, 0.135, 0.10, 0.05] };
        assert!(!fraction_bar_between(&upper, &lower, &[thick]));
        // Overlapping / reversed vertical order is not a fraction.
        assert!(!fraction_bar_between(&lower, &upper, &[rule(0.135, 0.38, 0.10)]));
    }

    /// Real-fixture smoke test for deterministic detection. Environment-
    /// dependent: needs the repo's physics fixtures and a system pdfium
    /// binding. Skips (with a note) when either is absent so the suite stays
    /// green on machines without the DLL or fixtures, while still asserting
    /// real behaviour on dev machines.
    #[test]
    fn detect_figures_on_real_fixture() {
        let _guard = pdfium_test_lock();
        let manifest = env!("CARGO_MANIFEST_DIR");
        for name in ["../physics '24.pdf", "../physics '21.pdf"] {
            let path = std::path::Path::new(manifest).join(name);
            if !path.exists() {
                eprintln!("[DETECT_TEST] fixture missing: {}", path.display());
                continue;
            }
            let per_page = match detect_pdf_figures(&path) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[DETECT_TEST] pdfium unavailable, skipping: {}", e);
                    return;
                }
            };
            let total: usize = per_page.iter().map(Vec::len).sum();
            eprintln!(
                "[DETECT_TEST] {}: {} figures across {} pages",
                name,
                total,
                per_page.len()
            );
            let mut shown = 0;
            for (page_idx, figs) in per_page.iter().enumerate() {
                if figs.is_empty() {
                    continue;
                }
                for f in figs {
                    eprintln!(
                        "[DETECT_TEST]   page {}: bbox {:?}, caption {:?}, kind {:?}",
                        page_idx, f.bbox, f.caption, f.kind
                    );
                }
                shown += 1;
                if shown >= 12 {
                    break;
                }
            }
            assert!(
                total > 0,
                "{} must contain at least one detected figure region",
                name
            );
            for (page_idx, figs) in per_page.iter().enumerate() {
                for f in figs {
                    let b = f.bbox;
                    assert!(
                        b[0] >= 0.0 && b[0] <= 1.0,
                        "{} page {}: x within page",
                        name,
                        page_idx
                    );
                    assert!(
                        b[1] >= 0.0 && b[1] <= 1.0,
                        "{} page {}: y within page",
                        name,
                        page_idx
                    );
                    assert!(
                        b[2] > 0.0 && b[2] <= 1.0,
                        "{} page {}: positive width",
                        name,
                        page_idx
                    );
                    assert!(
                        b[3] > 0.0 && b[3] <= 1.0,
                        "{} page {}: positive height",
                        name,
                        page_idx
                    );
                }
            }
        }
    }

    /// Fixture-gated regression for the load-once document cache: one cache
    /// must parse the PDF exactly once and serve every page render from that
    /// single document. Skips cleanly when pdfium/fixture are absent.
    #[test]
    fn cache_reuses_loaded_document() {
        // Pdfium is not thread-safe: a render beside another test's pdfium
        // calls corrupts the library's state for every test after it.
        let _guard = pdfium_test_lock();
        let manifest = env!("CARGO_MANIFEST_DIR");
        let path = std::path::Path::new(manifest).join("../physics '24.pdf");
        if !path.exists() {
            eprintln!("[RENDER_TEST] fixture missing: {}", path.display());
            return;
        }
        let cache = PageRenderCache::new(2);
        let page0 = match cache.get_or_render(&path, 0) {
            Ok(img) => img,
            Err(e) => {
                eprintln!("[RENDER_TEST] pdfium unavailable, skipping: {}", e);
                return;
            }
        };
        assert!(page0.width() > 0 && page0.height() > 0);
        let page1 = cache
            .get_or_render(&path, 1)
            .expect("second page renders from the shared document");
        assert!(page1.width() > 0 && page1.height() > 0);
        // Two distinct pages rendered from a single document parse.
        assert_eq!(
            cache.load_count.load(Ordering::Relaxed),
            1,
            "document must be loaded exactly once for a multi-page render sequence"
        );
        // A cache hit neither re-renders nor re-loads the document.
        let hit = cache.get_or_render(&path, 0).expect("cached hit");
        assert_eq!(hit.width(), page0.width());
        assert_eq!(cache.load_count.load(Ordering::Relaxed), 1);
    }

    /// Fixture-gated smoke: render_page_from_document returns a non-empty
    /// image at the default DPI.
    #[test]
    fn render_page_from_document_smoke() {
        let _guard = pdfium_test_lock();
        let manifest = env!("CARGO_MANIFEST_DIR");
        let path = std::path::Path::new(manifest).join("../physics '24.pdf");
        if !path.exists() {
            eprintln!("[RENDER_TEST] fixture missing: {}", path.display());
            return;
        }
        let pdfium = match get_pdfium() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[RENDER_TEST] pdfium unavailable, skipping: {}", e);
                return;
            }
        };
        let document = match pdfium.load_pdf_from_file(&path, None) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[RENDER_TEST] pdfium load failed, skipping: {}", e);
                return;
            }
        };
        let img = render_page_from_document(&document, 0, DEFAULT_FIGURE_RENDER_DPI)
            .expect("render page 0 at default DPI");
        assert!(img.width() > 0 && img.height() > 0);
    }

    /// Fixture-gated: the DPI knob must shrink the render relative to the
    /// default 300 DPI.
    #[test]
    fn dpi_knob_shrinks_renders() {
        let _guard = pdfium_test_lock();
        let manifest = env!("CARGO_MANIFEST_DIR");
        let path = std::path::Path::new(manifest).join("../physics '24.pdf");
        if !path.exists() {
            eprintln!("[RENDER_TEST] fixture missing: {}", path.display());
            return;
        }
        let pdfium = match get_pdfium() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[RENDER_TEST] pdfium unavailable, skipping: {}", e);
                return;
            }
        };
        let document = match pdfium.load_pdf_from_file(&path, None) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[RENDER_TEST] pdfium load failed, skipping: {}", e);
                return;
            }
        };
        let high = render_page_from_document(&document, 0, DEFAULT_FIGURE_RENDER_DPI)
            .expect("300-DPI render");
        let low = render_page_from_document(&document, 0, 150).expect("150-DPI render");
        assert!(
            high.width() > low.width(),
            "300-DPI render width {} must exceed 150-DPI width {}",
            high.width(),
            low.width()
        );
    }
}
