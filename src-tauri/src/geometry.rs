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

/// Minimum crop edge in pixels — anything smaller is a meaningless speck.
const MIN_EDGE_PX: u32 = 12;
/// Reject boxes covering more than this fraction of the page — that's a
/// whole-page misdetection, not a diagram.
const MAX_AREA_FRAC: f64 = 0.92;
/// Reject boxes smaller than this fraction of the page — OCR noise.
const MIN_AREA_FRAC: f64 = 0.0005;

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
    let mut readings: Vec<(f32, f32, f32, f32)> = Vec::with_capacity(2);
    if b[2] > 0.0 && b[3] > 0.0 {
        readings.push((fx, fy, fx + fv2, fy + fv3)); // (x, y, w, h)
    }
    if fv2 > fx && fv3 > fy {
        readings.push((fx, fy, fv2, fv3)); // (x1, y1, x2, y2)
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
        let far_x = (x1.min(1.0) * img_w as f32).round().max(0.0) as u32;
        let far_y = (y1.min(1.0) * img_h as f32).round().max(0.0) as u32;
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

/// Variance/luma check — true when a crop is blank or an empty student grid.
pub fn is_blank_or_grid(img: &image::DynamicImage) -> bool {
    use image::GenericImageView;
    let (width, height) = img.dimensions();
    if width == 0 || height == 0 {
        return true;
    }

    let mut sum = 0.0_f64;
    let mut sum_sq = 0.0_f64;
    let mut count = 0.0_f64;
    let mut non_white_count = 0.0_f64;

    for (_, _, pixel) in img.pixels() {
        let luma = pixel[0] as f64 * 0.299 + pixel[1] as f64 * 0.587 + pixel[2] as f64 * 0.114;
        sum += luma;
        sum_sq += luma * luma;
        count += 1.0;
        if luma < 180.0 {
            non_white_count += 1.0;
        }
    }

    if count == 0.0 {
        return true;
    }

    let mean = sum / count;
    let variance = (sum_sq / count) - (mean * mean);
    let non_white_ratio = non_white_count / count;

    variance < 5.0 || non_white_ratio < 0.0001
}

/// Crop a proposed diagram bbox out of a decoded page image.
/// Applies the sanitizer, padding, and the blank-grid guard.
/// Returns `None` (never panics) when the crop is unusable.
pub fn crop_diagram(
    img: &image::DynamicImage,
    bbox: &[f32],
    padding: u32,
) -> Option<image::RgbaImage> {
    use image::GenericImageView;
    let (img_w, img_h) = img.dimensions();
    let rect = sanitize_bbox(bbox, img_w, img_h)?;

    let safe_x = rect.x.saturating_sub(padding);
    let safe_y = rect.y.saturating_sub(padding);
    // img_w - safe_x / img_h - safe_y cannot underflow: the sanitizer
    // guarantees rect.x <= img_w - 1 and rect.y <= img_h - 1, and
    // saturating_sub only shrinks the origin.
    let x_pad_left = rect.x - safe_x;
    let y_pad_top = rect.y - safe_y;
    let safe_w = (rect.w + x_pad_left + padding).min(img_w - safe_x);
    let safe_h = (rect.h + y_pad_top + padding).min(img_h - safe_y);

    if safe_w < MIN_EDGE_PX || safe_h < MIN_EDGE_PX {
        return None;
    }

    let mut owned = img.clone();
    let cropped = image::imageops::crop(&mut owned, safe_x, safe_y, safe_w, safe_h).to_image();
    if is_blank_or_grid(&image::DynamicImage::ImageRgba8(cropped.clone())) {
        return None;
    }
    Some(cropped)
}

/// Decode a base64 page image (with or without a data-URL prefix).
pub fn decode_page_image(b64: &str) -> Option<image::DynamicImage> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(strip_data_url(b64))
        .ok()?;
    image::load_from_memory(&bytes).ok()
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

    const W: u32 = 1654;
    const H: u32 = 2339;

    fn assert_close(a: u32, b: u32, tol: u32) {
        let d = (a as i64 - b as i64).abs();
        assert!(d <= tol as i64, "{a} not within {tol} of {b}");
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
            let _ = crop_diagram(&img, b, 40); // must not panic
        }
    }
}
