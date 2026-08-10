use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};
use pdfium_render::prelude::*;
use crate::pipeline::{PageInput, PageInputKind};
use base64::Engine;
use image::DynamicImage;
use std::io::Cursor;
use std::sync::OnceLock;

use crate::pdfium::init_pdfium;

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
    app_handle: Option<tauri::AppHandle>,
}

impl PageRenderCache {
    pub fn new(capacity: usize, app_handle: tauri::AppHandle) -> Self {
        Self {
            capacity: capacity.max(1),
            state: Mutex::new(PageRenderCacheState {
                pages: HashMap::with_capacity(capacity.max(1)),
                lru: VecDeque::with_capacity(capacity.max(1)),
            }),
            app_handle: Some(app_handle),
        }
    }

    /// Test-only constructor that uses system PDFium (no AppHandle required)
    #[cfg(test)]
    pub fn new_for_test(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            state: Mutex::new(PageRenderCacheState {
                pages: HashMap::with_capacity(capacity.max(1)),
                lru: VecDeque::with_capacity(capacity.max(1)),
            }),
            app_handle: None,
        }
    }

    /// Return a shared 300-DPI page image, rendering it exactly once while it
    /// remains resident in the bounded cache.
    pub fn get_or_render(
        &self,
        path: &Path,
        page_idx: usize,
    ) -> Result<Arc<DynamicImage>, String> {
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

        let app_handle = self.app_handle.as_ref().ok_or("No AppHandle available for production rendering")?;
        let image = Arc::new(render_pdf_page_at_300dpi(app_handle, path, page_idx)?);
        if state.pages.len() >= self.capacity {
            if let Some(evicted) = state.lru.pop_front() {
                state.pages.remove(&evicted);
            }
        }
        state.pages.insert(page_idx, Arc::clone(&image));
        state.lru.push_back(page_idx);
        Ok(image)
    }

    /// Test-only version that uses system PDFium
    #[cfg(test)]
    pub fn get_or_render_for_test(
        &self,
        path: &Path,
        page_idx: usize,
    ) -> Result<Arc<DynamicImage>, String> {
        use crate::pdfium::init_pdfium_for_test;
        let pdfium = init_pdfium_for_test()?;

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

        let document = pdfium.load_pdf_from_file(path, None)
            .map_err(|e| format!("Failed to load PDF: {:?}", e))?;

        let pages = document.pages();
        if page_idx >= pages.len() as usize {
            return Err(format!("Page index {} out of bounds", page_idx));
        }

        let page = pages.get((page_idx as u16).into())
            .map_err(|e| format!("Failed to get page: {:?}", e))?;

        let render_config = PdfRenderConfig::new().set_target_width(2480); // roughly 300 DPI for A4 width (8.27 * 300 = 2481)
        let bitmap = page.render_with_config(&render_config)
            .map_err(|e| format!("Failed to render page: {:?}", e))?;

        let image = Arc::new(bitmap.as_image()
            .map_err(|e| format!("Failed to convert bitmap to image: {:?}", e))?);

        if state.pages.len() >= self.capacity {
            if let Some(evicted) = state.lru.pop_front() {
                state.pages.remove(&evicted);
            }
        }
        state.pages.insert(page_idx, Arc::clone(&image));
        state.lru.push_back(page_idx);
        Ok(image)
    }
}

fn get_pdfium(app_handle: &tauri::AppHandle) -> Result<&'static Pdfium, String> {
    PDFIUM_INSTANCE.get_or_init(|| {
        init_pdfium(app_handle)
    }).as_ref().map_err(|e| e.clone())
}

#[allow(dead_code)]
pub fn render_pdf_pages(app_handle: &tauri::AppHandle, path: &Path) -> Result<Vec<PageInput>, String> {
    let pdfium = get_pdfium(app_handle)?;

    let document = pdfium.load_pdf_from_file(path, None)
        .map_err(|e| format!("Failed to load PDF: {:?}", e))?;

    let mut pages = Vec::new();
    let render_dpi = std::env::var("MERGEMARK_RENDER_DPI")
        .unwrap_or_else(|_| "200".to_string())
        .parse::<u32>()
        .unwrap_or(200);
    let target_width = (8.27 * render_dpi as f32).round() as i32;
    let render_config = PdfRenderConfig::new().set_target_width(target_width.try_into().unwrap());

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

        let bitmap = page.render_with_config(&render_config)
            .map_err(|e| format!("Failed to render page {}: {:?}", i, e))?;

        let img: DynamicImage = bitmap.as_image()
            .map_err(|e| format!("Failed to convert bitmap to image on page {}: {:?}", i, e))?;
        
        let mut buf = Cursor::new(Vec::new());
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 88);
        encoder.encode_image(&img)
            .map_err(|e| format!("Failed to encode jpeg on page {}: {:?}", i, e))?;
        
        let b64 = format!("data:image/jpeg;base64,{}", 
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

    let mut buf = Cursor::new(Vec::new());
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 88);
    encoder.encode_image(&final_img)
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

pub fn render_pdf_page_at_300dpi(app_handle: &tauri::AppHandle, path: &Path, page_idx: usize) -> Result<image::DynamicImage, String> {
    let pdfium = get_pdfium(app_handle)?;

    let document = pdfium.load_pdf_from_file(path, None)
        .map_err(|e| format!("Failed to load PDF: {:?}", e))?;

    let pages = document.pages();
    if page_idx >= pages.len() as usize {
        return Err(format!("Page index {} out of bounds", page_idx));
    }

    let page = pages.get((page_idx as u16).into())
        .map_err(|e| format!("Failed to get page: {:?}", e))?;

    let render_config = PdfRenderConfig::new().set_target_width(2480); // roughly 300 DPI for A4 width (8.27 * 300 = 2481)
    let bitmap = page.render_with_config(&render_config)
        .map_err(|e| format!("Failed to render page: {:?}", e))?;

    bitmap.as_image()
        .map_err(|e| format!("Failed to convert bitmap to image: {:?}", e))
}

/// Test-only version that uses system PDFium (no AppHandle required)
#[cfg(test)]
pub fn render_pdf_pages_for_test(path: &Path) -> Result<Vec<PageInput>, String> {
    use crate::pdfium::init_pdfium_for_test;
    let pdfium = init_pdfium_for_test()?;

    let document = pdfium.load_pdf_from_file(path, None)
        .map_err(|e| format!("Failed to load PDF: {:?}", e))?;

    let mut pages = Vec::new();
    let render_dpi = std::env::var("MERGEMARK_RENDER_DPI")
        .unwrap_or_else(|_| "200".to_string())
        .parse::<u32>()
        .unwrap_or(200);
    let target_width = (8.27 * render_dpi as f32).round() as i32;
    let render_config = PdfRenderConfig::new().set_target_width(target_width.try_into().unwrap());

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

        let bitmap = page.render_with_config(&render_config)
            .map_err(|e| format!("Failed to render page {}: {:?}", i, e))?;

        let img: DynamicImage = bitmap.as_image()
            .map_err(|e| format!("Failed to convert bitmap to image on page {}: {:?}", i, e))?;

        let mut buf = Cursor::new(Vec::new());
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 88);
        encoder.encode_image(&img)
            .map_err(|e| format!("Failed to encode jpeg on page {}: {:?}", i, e))?;

        let b64 = format!("data:image/jpeg;base64,{}",
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
