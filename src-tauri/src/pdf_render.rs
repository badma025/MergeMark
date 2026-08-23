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

fn get_pdfium() -> Result<&'static Pdfium, String> {
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
    Ok(result)
}

/// Per-page detection: collect Image/Path object bounds, cluster strokes into
/// figure regions, filter specks/rule-lines/text-dense/header-footer regions,
/// then attach the nearest "Figure N" caption.
fn detect_page_figures_inner(page: &PdfPage) -> Vec<DetectedFigure> {
    let page_w = page.width().value;
    let page_h = page.height().value;
    if page_w <= 0.0 || page_h <= 0.0 {
        return Vec::new();
    }

    // 1. Raw object bounds (normalized 0..1). Images and paths are the
    //    drawing primitives a figure is made of. Per-object errors are
    //    skipped, never fatal.
    let objects = page.objects();
    let mut raw_boxes: Vec<[f32; 4]> = Vec::new();
    for obj in objects.iter() {
        let bounds = match &obj {
            PdfPageObject::Image(o) => o.bounds().ok(),
            PdfPageObject::Path(o) => o.bounds().ok(),
            _ => None,
        };
        let Some(b) = bounds else { continue };
        let b = crate::geometry::normalize_pdf_box(
            b.left().value,
            b.right().value,
            b.top().value,
            b.bottom().value,
            page_w,
            page_h,
        );
        if b[2] <= 0.0 || b[3] <= 0.0 {
            continue;
        }
        raw_boxes.push(b);
    }
    if raw_boxes.is_empty() {
        return Vec::new();
    }

    // 2. Text segments: used for text-density filtering and caption
    //    association ("Figure 1" is a text object in the content stream).
    let mut text_rects: Vec<[f32; 4]> = Vec::new();
    let mut caption_candidates: Vec<([f32; 2], String)> = Vec::new();
    if let Ok(text) = page.text() {
        for seg in text.segments().iter() {
            let rect = seg.bounds();
            let tr = [
                (rect.left().value / page_w).clamp(0.0, 1.0),
                (1.0 - rect.top().value / page_h).clamp(0.0, 1.0),
                (rect.width().value / page_w).clamp(0.0, 1.0),
                (rect.height().value / page_h).clamp(0.0, 1.0),
            ];
            if tr[2] <= 0.0 || tr[3] <= 0.0 {
                continue;
            }
            text_rects.push(tr);
            let seg_text = seg.text();
            if matches_caption(&seg_text) {
                caption_candidates.push((
                    [tr[0] + tr[2] / 2.0, tr[1] + tr[3] / 2.0],
                    seg_text,
                ));
            }
        }
    }

    // 3. Cluster strokes into figure regions, then filter the clusters.
    //
    // A tight gap is deliberate: exam pages are covered in ruled answer lines,
    // headers, and border rules made of hundreds of tiny path segments. A wide
    // merge tolerance chains them across the whole page into one near-full-page
    // box (which also steals the nearest caption). Figures are drawn as
    // connected strokes (axes cross curves, circuit wires touch), so a small
    // gap still merges them while keeping the page decoration separate. The
    // decoration clusters are then rejected below by area / thinness / density.
    let clusters = crate::geometry::cluster_boxes(raw_boxes, 0.006);
    let mut figures: Vec<DetectedFigure> = Vec::new();
    for b in clusters {
        if b[2] * b[3] > MAX_FIGURE_AREA_FRAC {
            continue;
        }
        if !crate::geometry::is_probable_figure_box(&b, &text_rects, 0.003, 0.015, 8.0, 0.4) {
            continue;
        }
        let caption = nearest_caption(b, &caption_candidates);
        let kind = caption
            .as_deref()
            .and_then(crate::geometry::caption_kind_from_text);
        figures.push(DetectedFigure {
            bbox: b,
            caption,
            kind,
        });
    }
    figures
}

/// No legitimate AQA figure fills more than half a page; the full-page border
/// rectangle (a chain of tiny path segments) is far larger and is decoration.
const MAX_FIGURE_AREA_FRAC: f32 = 0.5;

/// True when a text segment looks like a "Figure N" / "Fig. N" caption.
fn matches_caption(text: &str) -> bool {
    static RE_CAPTION: OnceLock<regex::Regex> = OnceLock::new();
    RE_CAPTION
        .get_or_init(|| regex::Regex::new(r"(?i)\bfig(?:ure)?\.?\s*\d+").unwrap())
        .is_match(text)
}

/// Pick the caption candidate whose center is nearest this figure's center.
fn nearest_caption(b: [f32; 4], candidates: &[([f32; 2], String)]) -> Option<String> {
    let cx = b[0] + b[2] / 2.0;
    let cy = b[1] + b[3] / 2.0;
    let mut best: Option<(f32, &str)> = None;
    for (center, text) in candidates {
        let dx = center[0] - cx;
        let dy = center[1] - cy;
        let dist = dx * dx + dy * dy;
        if best.map_or(true, |(bd, _)| dist < bd) {
            best = Some((dist, text));
        }
    }
    best.map(|(_, t)| t.to_string())
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

    /// Real-fixture smoke test for deterministic detection. Environment-
    /// dependent: needs the repo's physics fixtures and a system pdfium
    /// binding. Skips (with a note) when either is absent so the suite stays
    /// green on machines without the DLL or fixtures, while still asserting
    /// real behaviour on dev machines.
    #[test]
    fn detect_figures_on_real_fixture() {
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
