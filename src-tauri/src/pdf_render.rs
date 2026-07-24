use std::path::Path;
use pdfium_render::prelude::*;
use crate::pipeline::{PageInput, PageInputKind};
use base64::Engine;
use image::{DynamicImage, ImageFormat};
use std::io::Cursor;

#[allow(dead_code)]
pub fn render_pdf_pages(path: &Path) -> Result<Vec<PageInput>, String> {
    let pdfium = Pdfium::new(
        Pdfium::bind_to_system_library()
            .map_err(|e| format!("Failed to bind to pdfium: {:?}", e))?
    );

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
    let pdfium = Pdfium::new(
        Pdfium::bind_to_system_library()
            .map_err(|e| format!("Failed to bind to pdfium: {:?}", e))?
    );

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
