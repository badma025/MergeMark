use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use parking_lot::Mutex;
use pdfium_render::prelude::*;
use crate::pipeline::{PageInput, PageInputKind};
use base64::Engine;
use image::{DynamicImage, ImageFormat};
use std::io::Cursor;
use std::sync::OnceLock;
use tauri::{AppHandle, Manager};

static PDFIUM_INSTANCE: OnceLock<Result<Pdfium, String>> = OnceLock::new();
static PDFIUM_RESOURCE_DIR: OnceLock<std::path::PathBuf> = OnceLock::new();

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
}

impl PageRenderCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            state: Mutex::new(PageRenderCacheState {
                pages: HashMap::with_capacity(capacity.max(1)),
                lru: VecDeque::with_capacity(capacity.max(1)),
            }),
        }
    }

    /// Return a shared 300-DPI page image, rendering it exactly once while it
    /// remains resident in the bounded cache.
    ///
    /// The mutex is NOT held during the expensive pdfium render call. This
    /// prevents blocking-thread-pool starvation on macOS where multiple
    /// `spawn_blocking` tasks compete for this lock: if the lock were held
    /// during rendering, other blocking tasks (e.g. `prepare_chunk_images`)
    /// could never be scheduled, causing a deadlock.
    pub fn get_or_render(
        &self,
        path: &Path,
        page_idx: usize,
    ) -> Result<Arc<DynamicImage>, String> {
        // Fast path: check cache under lock, return immediately if hit.
        {
            let mut state = self.state.lock();

            if let Some(image) = state.pages.get(&page_idx).cloned() {
                if let Some(position) = state.lru.iter().position(|cached| *cached == page_idx) {
                    state.lru.remove(position);
                }
                state.lru.push_back(page_idx);
                return Ok(image);
            }
        } // Lock released before slow render.

        // Slow path: render WITHOUT holding the lock so other blocking
        // threads can access the cache concurrently.
        let image = Arc::new(render_pdf_page_at_300dpi(path, page_idx)?);

        // Re-acquire to insert into cache. Another thread may have rendered
        // the same page in the meantime — that's fine, we just overwrite.
        {
            let mut state = self.state.lock();

            if state.pages.len() >= self.capacity && !state.pages.contains_key(&page_idx) {
                if let Some(evicted) = state.lru.pop_front() {
                    state.pages.remove(&evicted);
                }
            }
            state.pages.insert(page_idx, Arc::clone(&image));
            state.lru.push_back(page_idx);
        }

        Ok(image)
    }
}

pub fn initialize_pdfium(app_handle: &AppHandle) -> Result<(), String> {
    let resource_dir = app_handle
        .path()
        .resource_dir()
        .map_err(|error| format!("failed to resolve Tauri resource directory: {error}"))?;
    let _ = PDFIUM_RESOURCE_DIR.set(resource_dir);
    get_pdfium().map(|_| ())
}

fn get_pdfium() -> Result<&'static Pdfium, String> {
    PDFIUM_INSTANCE.get_or_init(|| {
        let filename = if cfg!(target_os = "windows") {
            "pdfium.dll"
        } else if cfg!(target_os = "macos") {
            "libpdfium.dylib"
        } else {
            "libpdfium.so"
        };
        let mut candidates = Vec::new();
        if let Some(resource_dir) = PDFIUM_RESOURCE_DIR.get() {
            candidates.push(resource_dir.join("libs").join(filename));
            candidates.push(resource_dir.join(filename));
        }
        if let Ok(exe_dir) = std::env::current_exe()
            .and_then(|path| path.parent().map(|parent| parent.to_path_buf()).ok_or(std::io::Error::other("missing executable directory")))
        {
            candidates.push(exe_dir.join(filename));
            #[cfg(target_os = "macos")]
            candidates.push(exe_dir.join("../Frameworks").join(filename));
        }

        let mut last_error = None;
        for path in candidates.iter().filter(|path| path.is_file()) {
            match Pdfium::bind_to_library(path.to_string_lossy().as_ref()) {
                Ok(bindings) => return Ok(Pdfium::new(bindings)),
                Err(error) => last_error = Some(format!("{}: {:?}", path.display(), error)),
            }
        }

        let bindings = Pdfium::bind_to_system_library().map_err(|error| {
            format!(
                "failed to bind bundled {} or system PDFium: {:?} (last bundled error: {})",
                filename,
                error,
                last_error.unwrap_or_else(|| "no bundled candidate found".to_string())
            )
        })?;

        Ok(Pdfium::new(bindings))
    }).as_ref().map_err(|e| e.clone())
}

#[allow(dead_code)]
pub fn render_pdf_pages(path: &Path) -> Result<Vec<PageInput>, String> {
    let pdfium = get_pdfium()?;

    let document = pdfium.load_pdf_from_file(path, None)
        .map_err(|e| format!("Failed to load PDF: {:?}", e))?;

    let mut pages = Vec::new();
    let render_dpi = std::env::var("MERGEMARK_RENDER_DPI")
        .unwrap_or_else(|_| "200".to_string())
        .parse::<u32>()
        .unwrap_or(200);

    for (i, page) in document.pages().iter().enumerate() {
        let text = page.text().map_err(|e| e.to_string())?.all();
        
        let objects = page.objects();
        let has_images = objects.iter().any(|obj| matches!(obj.object_type(), PdfPageObjectType::Image));
        let has_vectors = objects.iter().any(|obj| matches!(obj.object_type(), PdfPageObjectType::Path));

        if text.trim().is_empty() && !has_images && !has_vectors {
            pages.push(PageInput {
                kind: PageInputKind::TextOnly,
                text,
            });
            continue;
        }

        // Use actual page width instead of hardcoded A4 (8.27"). This handles
        // US Letter (8.5"), A3, and other sizes correctly on all platforms.
        let page_width_inches = page.width().value / 72.0;
        let target_width = (page_width_inches * render_dpi as f32).round() as i32;
        let render_config = PdfRenderConfig::new().set_target_width(target_width.max(1).try_into().unwrap());

        let bitmap = page.render_with_config(&render_config)
            .map_err(|e| format!("Failed to render page {}: {:?}", i, e))?;

        let img: DynamicImage = bitmap.as_image()
            .map_err(|e| format!("Failed to convert bitmap to image on page {}: {:?}", i, e))?;
        
        let mut buf = Cursor::new(Vec::new());
        let format_str;
        if has_images || has_vectors {
            img.write_to(&mut buf, ImageFormat::Png)
                .map_err(|e| format!("Failed to encode image on page {}: {:?}", i, e))?;
            format_str = "png";
        } else {
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 92);
            encoder.encode_image(&img)
                .map_err(|e| format!("Failed to encode jpeg on page {}: {:?}", i, e))?;
            format_str = "jpeg";
        }
        
        let b64 = format!("data:image/{};base64,{}", 
            format_str,
            base64::engine::general_purpose::STANDARD.encode(buf.into_inner())
        );

        pages.push(PageInput {
            kind: PageInputKind::Image {
                b64,
            },
            text,
        });
    }

    Ok(pages)
}

pub fn render_pdf_page_at_300dpi(path: &Path, page_idx: usize) -> Result<image::DynamicImage, String> {
    let pdfium = get_pdfium()?;

    let document = pdfium.load_pdf_from_file(path, None)
        .map_err(|e| format!("Failed to load PDF: {:?}", e))?;

    let pages = document.pages();
    if page_idx >= pages.len() as usize {
        return Err(format!("Page index {} out of bounds", page_idx));
    }

    let page = pages.get((page_idx as u16).into())
        .map_err(|e| format!("Failed to get page: {:?}", e))?;

    // Use actual page width for correct DPI on all paper sizes (A4, Letter, etc.)
    let page_width_inches = page.width().value / 72.0;
    let target_width = (page_width_inches * 300.0).round() as i32;
    let render_config = PdfRenderConfig::new().set_target_width(target_width.max(1).try_into().unwrap());
    let bitmap = page.render_with_config(&render_config)
        .map_err(|e| format!("Failed to render page: {:?}", e))?;

    bitmap.as_image()
        .map_err(|e| format!("Failed to convert bitmap to image: {:?}", e))
}
