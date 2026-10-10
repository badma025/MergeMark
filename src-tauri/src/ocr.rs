//! Local OCR for scanned pages: pages printed as one picture with no text
//! layer, or only a watermark one. Recognition runs on this machine, with no
//! request and no cost:
//!
//! * Windows: the operating system's `Windows.Media.Ocr`.
//! * macOS: Apple's Vision framework (`VNRecognizeTextRequest`).
//! * Anywhere without one of those (Linux): Tesseract, when `tesseract` is
//!   on the PATH.
//! * Otherwise no engine: [`recognize`] returns `None` and a scanned page is
//!   reported as not transcribed, never guessed.
//!
//! OCR reads words, not mathematics: every card transcribed from it is kept
//! for review and never counts as a clean import.

/// One recognised word, its box in the image's pixels `[x0, y0, x1, y1]`.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrWord {
    pub text: String,
    pub bbox: [f32; 4],
}

/// One recognised line of words, in reading order.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrLine {
    pub words: Vec<OcrWord>,
}

/// Whether this machine has a local OCR engine.
pub fn available() -> bool {
    #[cfg(test)]
    if unavailable_for_test::active() {
        return false;
    }
    platform::available() || tesseract::available()
}

/// TEST ONLY: report the engine unavailable, as on a machine that has none.
/// Every OCR read goes through PDFium, so a test holds
/// `pdf_render::pdfium_test_lock()` while this is in force.
#[cfg(test)]
pub(crate) mod unavailable_for_test {
    use std::sync::atomic::{AtomicBool, Ordering};

    static ACTIVE: AtomicBool = AtomicBool::new(false);

    pub(crate) fn active() -> bool {
        ACTIVE.load(Ordering::SeqCst)
    }

    /// The engine is unavailable until the guard drops.
    pub(crate) fn guard() -> Guard {
        ACTIVE.store(true, Ordering::SeqCst);
        Guard
    }

    pub(crate) struct Guard;

    impl Drop for Guard {
        fn drop(&mut self) {
            ACTIVE.store(false, Ordering::SeqCst);
        }
    }
}

/// Recognise the text of a page image: the operating system's engine where
/// there is one, otherwise Tesseract. `None` when no engine could run.
pub fn recognize(img: &image::RgbaImage) -> Option<Vec<OcrLine>> {
    if platform::available() {
        platform::recognize(img)
    } else {
        tesseract::recognize(img)
    }
}

#[cfg(windows)]
mod platform {
    use super::{OcrLine, OcrWord};
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::DataWriter;

    pub fn available() -> bool {
        true
    }

    pub fn recognize(img: &image::RgbaImage) -> Option<Vec<OcrLine>> {
        // The windows crate keeps each agile activation factory it fetches
        // for the life of the process, so this keeps the multithreaded
        // apartment they come from as long. Joined only by the reading
        // threads, the apartment was left with none when they ended; about
        // 30 seconds later COM unloaded the engine's DLLs, and the next read
        // faulted in them. A thread that has joined no apartment reads in
        // this one.
        static MTA_KEPT: AtomicBool = AtomicBool::new(false);
        if !MTA_KEPT.load(Ordering::Acquire) {
            // SAFETY: no arguments; the usage it takes is never released.
            if unsafe { windows::Win32::System::Com::CoIncrementMTAUsage() }.is_err() {
                return None;
            }
            MTA_KEPT.store(true, Ordering::Release);
        }
        // Exam papers here are English; the profile's languages otherwise.
        let english = ["en-GB", "en-US"].iter().find_map(|tag| {
            let language = windows::Globalization::Language::CreateLanguage(&windows::core::HSTRING::from(*tag)).ok()?;
            OcrEngine::IsLanguageSupported(&language).ok()?.then(|| OcrEngine::TryCreateFromLanguage(&language).ok()).flatten()
        });
        let engine = english.or_else(|| OcrEngine::TryCreateFromUserProfileLanguages().ok())?;
        // The engine refuses images larger than its limit: scale to fit.
        let max = OcrEngine::MaxImageDimension().ok()?.max(1);
        let scale = (max as f32 / img.width().max(img.height()) as f32).min(1.0);
        let mut bgra = if scale < 1.0 {
            image::imageops::resize(img, (img.width() as f32 * scale) as u32, (img.height() as f32 * scale) as u32, image::imageops::FilterType::Triangle).into_raw()
        } else {
            img.as_raw().clone()
        };
        let (w, h) = ((img.width() as f32 * scale.min(1.0)) as u32, (img.height() as f32 * scale.min(1.0)) as u32);
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        let bitmap = SoftwareBitmap::CreateWithAlpha(BitmapPixelFormat::Bgra8, w as i32, h as i32, BitmapAlphaMode::Premultiplied).ok()?;
        let writer = DataWriter::new().ok()?;
        writer.WriteBytes(&bgra).ok()?;
        bitmap.CopyFromBuffer(&writer.DetachBuffer().ok()?).ok()?;
        let result = engine.RecognizeAsync(&bitmap).ok()?.get().ok()?;
        let mut lines = Vec::new();
        for line in result.Lines().ok()? {
            let mut words = Vec::new();
            for word in line.Words().ok()? {
                let r = word.BoundingRect().ok()?;
                words.push(OcrWord {
                    text: word.Text().ok()?.to_string_lossy(),
                    bbox: [r.X / scale, r.Y / scale, (r.X + r.Width) / scale, (r.Y + r.Height) / scale],
                });
            }
            if !words.is_empty() {
                lines.push(OcrLine { words });
            }
        }
        Some(lines)
    }
}

#[cfg(target_os = "macos")]
mod platform {
    //! Apple's Vision framework: `VNRecognizeTextRequest` at the accurate
    //! level, English (GB, US). Each recognised line is a Vision observation;
    //! each word's box is the one Vision gives for that word's range.
    //!
    //! SAFETY (every `msg_send!` below): the selectors and argument types are
    //! those of Apple's Vision API (VNImageRequestHandler,
    //! VNRecognizeTextRequest, VNRecognizedTextObservation,
    //! VNRecognizedText).
    use super::{OcrLine, OcrWord};
    use objc2::rc::{autoreleasepool, Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2::{msg_send, sel};
    use objc2_foundation::{NSArray, NSData, NSDictionary, NSObject, NSRange, NSRect, NSString};

    #[link(name = "Vision", kind = "framework")]
    unsafe extern "C" {}

    /// `VNRequestTextRecognitionLevelAccurate`.
    const ACCURATE: isize = 0;

    pub fn available() -> bool {
        AnyClass::get(c"VNRecognizeTextRequest").is_some()
    }

    pub fn recognize(img: &image::RgbaImage) -> Option<Vec<OcrLine>> {
        let mut lines = read(img)?;
        // Reading a whole page, Vision can drop a short label standing alone
        // in the margin: on GitHub's macOS runner, a virtual Mac without the
        // neural engine, the scanned M3 Jun 2004 paper lost "1." and "2.",
        // which a Mac reads. Read by itself, the margin left of the text
        // gives them back.
        if let Some(edge) = super::text_left_edge(&lines, img.width() as f32) {
            let margin = image::imageops::crop_imm(img, 0, 0, edge as u32, img.height()).to_image();
            if let Some(found) = read(&margin) {
                super::add_margin_labels(&mut lines, found, edge);
            }
        }
        Some(lines)
    }

    /// One reading of an image by Vision.
    fn read(img: &image::RgbaImage) -> Option<Vec<OcrLine>> {
        let (width, height) = (img.width() as f64, img.height() as f64);
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        let handler_class = AnyClass::get(c"VNImageRequestHandler")?;
        let request_class = AnyClass::get(c"VNRecognizeTextRequest")?;
        // (Vision's autoreleased objects are freed page by page.)
        autoreleasepool(|_| {
            let data = NSData::with_bytes(&png);
            let options = NSDictionary::<NSString, AnyObject>::new();
            let handler: Allocated<AnyObject> = unsafe { msg_send![handler_class, alloc] };
            let handler: Option<Retained<AnyObject>> = unsafe { msg_send![handler, initWithData: &*data, options: &*options] };
            let handler = handler?;
            let request: Option<Retained<AnyObject>> = unsafe { msg_send![request_class, new] };
            let request = request?;
            let _: () = unsafe { msg_send![&*request, setRecognitionLevel: ACCURATE] };
            // English, as far as this Vision recognises it (it lists "en-US";
            // an unsupported language would fail the request).
            let lists: Bool = unsafe { msg_send![&*request, respondsToSelector: sel!(supportedRecognitionLanguagesAndReturnError:)] };
            if lists.as_bool() {
                let mut error: *mut NSObject = std::ptr::null_mut();
                let supported: Option<Retained<NSArray<NSString>>> = unsafe { msg_send![&*request, supportedRecognitionLanguagesAndReturnError: &mut error] };
                let supported: Vec<String> = supported.map(|s| s.iter().map(|l| l.to_string()).collect()).unwrap_or_default();
                let english: Vec<Retained<NSString>> = ["en-GB", "en-US"].iter().filter(|tag| supported.iter().any(|l| l == *tag)).map(|tag| NSString::from_str(tag)).collect();
                if !english.is_empty() {
                    let languages = NSArray::from_retained_slice(&english);
                    let _: () = unsafe { msg_send![&*request, setRecognitionLanguages: &*languages] };
                }
            }
            let requests = NSArray::<AnyObject>::from_slice(&[&*request]);
            let mut error: *mut NSObject = std::ptr::null_mut();
            let performed: Bool = unsafe { msg_send![&*handler, performRequests: &*requests, error: &mut error] };
            if !performed.as_bool() {
                return None;
            }
            let observations: Option<Retained<NSArray<AnyObject>>> = unsafe { msg_send![&*request, results] };
            let mut lines = Vec::new();
            for observation in observations?.iter() {
                let candidates: Retained<NSArray<AnyObject>> = unsafe { msg_send![&*observation, topCandidates: 1usize] };
                let Some(text) = candidates.firstObject() else { continue };
                let string: Retained<NSString> = unsafe { msg_send![&*text, string] };
                let line = string.to_string();
                let mut words = Vec::new();
                // Word ranges in UTF-16 units, as NSString counts them.
                let mut at = 0usize;
                for (i, part) in line.split(' ').enumerate() {
                    if i > 0 {
                        at += 1;
                    }
                    let len = part.encode_utf16().count();
                    if !part.is_empty() {
                        let mut box_error: *mut NSObject = std::ptr::null_mut();
                        let found: Option<Retained<AnyObject>> = unsafe { msg_send![&*text, boundingBoxForRange: NSRange::new(at, len), error: &mut box_error] };
                        if let Some(found) = found {
                            // Normalised, origin at the bottom left.
                            let r: NSRect = unsafe { msg_send![&*found, boundingBox] };
                            let (x0, x1) = (r.origin.x * width, (r.origin.x + r.size.width) * width);
                            let (y0, y1) = ((1.0 - r.origin.y - r.size.height) * height, (1.0 - r.origin.y) * height);
                            words.push(OcrWord { text: part.to_string(), bbox: [x0 as f32, y0 as f32, x1 as f32, y1 as f32] });
                        }
                    }
                    at += len;
                }
                if !words.is_empty() {
                    lines.push(OcrLine { words });
                }
            }
            Some(lines)
        })
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use super::OcrLine;

    pub fn available() -> bool {
        false
    }

    pub fn recognize(_img: &image::RgbaImage) -> Option<Vec<OcrLine>> {
        None
    }
}

/// Where the text of a page read by OCR begins, in the image's pixels: a
/// little left of where its lines a third of the page wide or longer start
/// (their median, so a question number read into its line moves nothing).
/// `None` with no such line, or no margin to read.
#[cfg(any(target_os = "macos", test))]
fn text_left_edge(lines: &[OcrLine], width: f32) -> Option<f32> {
    let mut starts: Vec<f32> = lines
        .iter()
        .filter_map(|l| {
            let x0 = l.words.iter().map(|w| w.bbox[0]).fold(f32::MAX, f32::min);
            let x1 = l.words.iter().map(|w| w.bbox[2]).fold(f32::MIN, f32::max);
            (x1 - x0 >= width / 3.0).then_some(x0)
        })
        .collect();
    if starts.is_empty() {
        return None;
    }
    starts.sort_by(f32::total_cmp);
    let edge = starts[starts.len() / 2] - 0.005 * width;
    (edge >= 0.04 * width).then_some(edge)
}

/// Adds to a page's lines the question labels that a reading of its margin
/// alone (the image left of `edge`) found and the page's reading did not: a
/// number of one or two digits, with a full stop or brackets, level with a
/// line of the text, as a question's number is printed, and overlapping
/// nothing already read.
#[cfg(any(target_os = "macos", test))]
fn add_margin_labels(lines: &mut Vec<OcrLine>, margin: Vec<OcrLine>, edge: f32) {
    let label = |text: &str| {
        let t = text.strip_prefix('(').unwrap_or(text);
        let t = t.strip_suffix(['.', ')']).unwrap_or(t);
        (1..=2).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_digit())
    };
    let span = |l: &OcrLine, i: usize, start: f32, pick: fn(f32, f32) -> f32| l.words.iter().map(|w| w.bbox[i]).fold(start, pick);
    for word in margin.into_iter().flat_map(|l| l.words) {
        let b = word.bbox;
        let level = lines.iter().any(|l| {
            let (y0, y1) = (span(l, 1, f32::MAX, f32::min), span(l, 3, f32::MIN, f32::max));
            span(l, 0, f32::MAX, f32::min) >= edge && b[3].min(y1) - b[1].max(y0) >= 0.5 * (b[3] - b[1])
        });
        let read = lines.iter().flat_map(|l| &l.words).any(|w| w.bbox[0] < b[2] && b[0] < w.bbox[2] && w.bbox[1] < b[3] && b[1] < w.bbox[3]);
        if label(&word.text) && b[2] <= edge && level && !read {
            lines.push(OcrLine { words: vec![word] });
        }
    }
}

/// Tesseract, run as `tesseract <png> stdout --psm 1 tsv` on a temporary
/// copy of the page image: automatic page segmentation, one TSV row per
/// recognised element.
mod tesseract {
    use super::{OcrLine, OcrWord};
    use std::process::{Command, Stdio};

    /// `tesseract` is on the PATH and runs.
    pub fn available() -> bool {
        static FOUND: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
            Command::new("tesseract").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
        });
        *FOUND
    }

    pub fn recognize(img: &image::RgbaImage) -> Option<Vec<OcrLine>> {
        if !available() {
            return None;
        }
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("mergemark-ocr-{}-{n}.png", std::process::id()));
        // (A new file of our own, never one already there.)
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).ok()?;
        let written = std::io::Write::write_all(&mut file, &png);
        drop(file);
        let out = written.ok().and_then(|_| Command::new("tesseract").arg(&path).args(["stdout", "--psm", "1", "tsv"]).stderr(Stdio::null()).output().ok());
        let _ = std::fs::remove_file(&path);
        let out = out.filter(|o| o.status.success())?;
        Some(parse_tsv(&String::from_utf8_lossy(&out.stdout)))
    }

    /// Tesseract's TSV ("level page_num block_num par_num line_num word_num
    /// left top width height conf text"): its words (level 5), grouped by
    /// block, paragraph and line in the order printed. Rows without text are
    /// no words.
    pub(super) fn parse_tsv(tsv: &str) -> Vec<OcrLine> {
        let mut lines: Vec<([u32; 4], OcrLine)> = Vec::new();
        for row in tsv.lines().skip(1) {
            let f: Vec<&str> = row.split('\t').collect();
            let &[level, page, block, par, line, _word, left, top, width, height, _conf, text, ..] = f.as_slice() else { continue };
            let text = text.trim();
            if level != "5" || text.is_empty() {
                continue;
            }
            let int = |v: &str| v.trim().parse::<u32>().ok();
            let (Some(page), Some(block), Some(par), Some(line)) = (int(page), int(block), int(par), int(line)) else { continue };
            let (Some(left), Some(top), Some(width), Some(height)) = (int(left), int(top), int(width), int(height)) else { continue };
            let word = OcrWord { text: text.to_string(), bbox: [left as f32, top as f32, (left + width) as f32, (top + height) as f32] };
            let key = [page, block, par, line];
            match lines.last_mut() {
                Some((k, l)) if *k == key => l.words.push(word),
                _ => lines.push((key, OcrLine { words: vec![word] })),
            }
        }
        lines.into_iter().map(|(_, l)| l).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tesseract_tsv_words_group_into_their_lines() {
        // Tesseract's TSV columns: level, page, block, paragraph, line, word,
        // left, top, width, height, confidence, text. Levels 1-4 (page,
        // block, paragraph, line) carry no text; an empty word is none.
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                   1\t1\t0\t0\t0\t0\t0\t0\t2480\t3508\t-1\t\n\
                   2\t1\t1\t0\t0\t0\t236\t300\t1900\t120\t-1\t\n\
                   3\t1\t1\t1\t0\t0\t236\t300\t1900\t120\t-1\t\n\
                   4\t1\t1\t1\t1\t0\t236\t300\t1900\t50\t-1\t\n\
                   5\t1\t1\t1\t1\t1\t236\t300\t40\t50\t96.5\t1.\n\
                   5\t1\t1\t1\t1\t2\t300\t302\t60\t48\t95.1\tA\n\
                   5\t1\t1\t1\t1\t3\t380\t300\t300\t50\t93.0\tcircular\n\
                   4\t1\t1\t1\t2\t0\t236\t370\t900\t50\t-1\t\n\
                   5\t1\t1\t1\t2\t1\t236\t370\t200\t50\t91.2\tflywheel\n\
                   5\t1\t1\t1\t2\t2\t450\t370\t10\t50\t-1\t \n";
        let lines = tesseract::parse_tsv(tsv);
        let text: Vec<Vec<&str>> = lines.iter().map(|l| l.words.iter().map(|w| w.text.as_str()).collect()).collect();
        assert_eq!(text, vec![vec!["1.", "A", "circular"], vec!["flywheel"]]);
        assert_eq!(lines[0].words[2].bbox, [380.0, 300.0, 680.0, 350.0]);
    }

    fn ocr_line(words: &[(&str, [f32; 4])]) -> OcrLine {
        OcrLine { words: words.iter().map(|&(text, bbox)| OcrWord { text: text.to_string(), bbox }).collect() }
    }

    #[test]
    fn a_margin_read_alone_gives_back_the_question_labels_a_page_read_dropped() {
        // A page 2480 px wide (A4 at 300 dpi), its text from x = 320 and its
        // question numbers at 230, as on the scanned M3 Jun 2004 paper.
        let mut page = vec![
            ocr_line(&[("A", [320.0, 250.0, 340.0, 290.0]), ("circular", [350.0, 250.0, 2000.0, 290.0])]),
            ocr_line(&[("Two", [320.0, 1390.0, 380.0, 1430.0]), ("light", [390.0, 1390.0, 2100.0, 1430.0])]),
            ocr_line(&[("3.", [229.0, 1980.0, 258.0, 2020.0])]),
            ocr_line(&[("A", [320.0, 1980.0, 340.0, 2020.0]), ("particle", [350.0, 1980.0, 2050.0, 2020.0])]),
        ];
        let edge = text_left_edge(&page, 2480.0).expect("a margin");
        assert!(edge > 260.0 && edge < 320.0, "{edge}");
        let margin = vec![
            ocr_line(&[("1.", [236.0, 262.0, 262.0, 300.0])]),
            ocr_line(&[("2.", [233.0, 1400.0, 262.0, 1438.0])]),
            // Read by the page already.
            ocr_line(&[("3.", [229.0, 1985.0, 258.0, 2022.0])]),
            // Level with no line of the text.
            ocr_line(&[("7", [236.0, 3300.0, 262.0, 3340.0])]),
            // No question number.
            ocr_line(&[("N17132A", [212.0, 3417.0, 290.0, 3450.0])]),
        ];
        add_margin_labels(&mut page, margin, edge);
        let alone: Vec<&str> = page.iter().filter(|l| l.words.len() == 1).map(|l| l.words[0].text.as_str()).collect();
        assert_eq!(alone, ["3.", "1.", "2."]);
    }

    #[test]
    fn a_page_without_a_line_of_text_or_a_margin_has_no_margin_to_read() {
        let figure = vec![ocr_line(&[("Figure", [1100.0, 600.0, 1300.0, 640.0]), ("1", [1310.0, 600.0, 1340.0, 640.0])])];
        assert_eq!(text_left_edge(&figure, 2480.0), None);
        let edge_to_edge = vec![ocr_line(&[("Wide", [20.0, 100.0, 2400.0, 140.0])])];
        assert_eq!(text_left_edge(&edge_to_edge, 2480.0), None);
    }

    #[test]
    fn tesseract_reads_a_scanned_page_when_installed() {
        if !tesseract::available() {
            eprintln!("[OCR] tesseract is not on the PATH: live test skipped");
            return;
        }
        let _guard = crate::pdf_render::pdfium_test_lock();
        // The scanned fixture Edexcel M3 Jun 2004: Q1 is about a circular
        // flywheel.
        let Some(path) = crate::test_support::corpus_paper("papers/used/Maths_A-level_M3_Papers-Edexcel__June_2004_QP.pdf") else { return };
        let page = crate::pdf_render::render_pdf_page_at_300dpi(&path, 1).expect("the scanned page renders");
        let lines = tesseract::recognize(&page.to_rgba8()).expect("tesseract ran");
        let words: Vec<&str> = lines.iter().flat_map(|l| l.words.iter().map(|w| w.text.as_str())).collect();
        assert!(words.iter().any(|w| w.contains("flywheel")), "{words:?}");
    }

    /// The engine still reads after the threads that ran it have ended. The
    /// windows crate keeps each class's activation factory for the life of
    /// the process; when only the reading threads had joined the
    /// multithreaded apartment, COM unloaded the engine's DLLs about 30
    /// seconds after the last of them ended, and the next read faulted in
    /// the unloaded Windows.Globalization.dll (the crash in the verify
    /// chain's test run). Waits 45 seconds. (Alone it fails without the fix;
    /// in a parallel run other reads can keep the apartment for part of the
    /// wait.)
    #[cfg(windows)]
    #[test]
    fn the_engine_reads_again_after_its_threads_have_ended() {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetModuleHandleW(name: *const u16) -> *mut std::ffi::c_void;
        }
        let page = image::RgbaImage::from_pixel(200, 100, image::Rgba([255, 255, 255, 255]));
        let read = |img: image::RgbaImage| std::thread::spawn(move || recognize(&img).is_some()).join().unwrap();
        assert!(read(page.clone()));
        let name: Vec<u16> = "Windows.Globalization.dll\0".encode_utf16().collect();
        for _ in 0..90 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            // SAFETY: a NUL-terminated module name; the handle is only tested.
            assert!(!unsafe { GetModuleHandleW(name.as_ptr()) }.is_null(), "COM unloaded the engine's DLLs, whose factories the windows crate keeps");
        }
        assert!(read(page));
    }
}
