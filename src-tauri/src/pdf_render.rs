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
