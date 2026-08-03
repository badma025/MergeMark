// ── The PVRV pipeline: Propose → Validate → Repair → Verify ────────────────
//
// Orchestrates ingestion with the AI treated as an untrusted proposer:
//
//   1. STRUCTURE: a cheap per-page structure pass (tiny schema) + the
//      text-layer footer scan build a DocumentMap — the skeleton, derived
//      from ground truth, never from transcription output.
//   2. EXTRACT: the AI transcribes one question span at a time, against the
//      map. It never invents question numbers, merging, or continuations.
//   3. VALIDATE: every response goes through deterministic validators
//      (JSON discipline, question-number conformance, terminal-ending,
//      marks checksum vs the printed footer).
//   4. REPAIR: failures are round-tripped to the model with the exact
//      validator errors quoted. Bounded attempts (config.max_repairs).
//   5. VERIFY/REPORT: every acceptance, salvage, repair, rejection, and
//      quarantine lands in an ImportReport surfaced to the UI.
//
// Nothing silently `continue`s. Quarantine is a first-class, visible
// outcome — never a swallowed page.

use crate::doc_map::{self, PageStructureProposal, QuestionSpan, ValidatedPageStructure};
use crate::geometry;
use crate::json_salvage::{parse_llm_json, ParseOutcome};
use crate::llm::{self, LlmClient};
use crate::validate;
use std::path::PathBuf;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::sync::Semaphore;
use futures_util::{FutureExt, StreamExt};
use image::GenericImageView;

// ══════════════════════════════════════════════════════════════════════════
// Public types
// ══════════════════════════════════════════════════════════════════════════

#[derive(Clone)]
pub enum PageInputKind {
    Image { b64: String },
    TextOnly,
}

#[derive(Clone)]
pub struct PageInput {
    pub kind: PageInputKind,
    pub text: String,
}

impl PageInput {
    pub fn get_b64(&self) -> Option<&String> {
        match &self.kind {
            PageInputKind::Image { b64, .. } => Some(b64),
            _ => None,
        }
    }
}

pub trait Progress: Send + Sync {
    fn stage(&self, message: &str);
}

#[allow(dead_code)]
pub struct NullProgress;
impl Progress for NullProgress {
    fn stage(&self, _message: &str) {}
}

#[derive(Clone)]
pub struct PipelineConfig {
    pub model: String,
    pub paper_name: String,
    pub subject: String,
    pub module_name: String,
    pub allowed_topics: Vec<String>,
    /// Where cropped diagrams are written; `None` skips image persistence
    /// (used in tests).
    pub diagrams_dir: Option<PathBuf>,
    pub pdf_path: Option<PathBuf>,
    /// Repair attempts after the first request per unit of work.
    pub max_repairs: u32,
    pub max_output_tokens: u32,
    /// Maximum concurrent API requests.
    pub parallelism: usize,
}

impl PipelineConfig {
    pub fn new(model: String, paper_name: String, subject: String, module_name: String, pdf_path: Option<PathBuf>) -> Self {
        Self {
            model,
            paper_name,
            subject,
            module_name,
            allowed_topics: Vec::new(),
            diagrams_dir: None,
            pdf_path,
            max_repairs: 2,
            max_output_tokens: 32768,
            parallelism: DEFAULT_PARALLEL,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkCheck {
    pub question_number: u32,
    pub expected: Option<u32>,
    pub actual: u32,
    pub ok: bool,
    pub needs_review: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuarantineEvent {
    pub scope: String,
    pub page: Option<usize>,
    pub question_number: Option<u32>,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedPage {
    pub page: usize,
    pub role: String,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]

pub struct TimingEntry {
    pub stage: String,
    pub operation: String,
    pub page: Option<usize>,
    pub question_number: Option<u32>,
    pub milliseconds: u64,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub paper_name: String,
    pub kind: String,
    pub pages_total: usize,
    pub pages_processed: usize,
    pub questions_expected: usize,
    pub questions_extracted: usize,
    pub paper_total_marks: Option<u32>,
    pub extracted_total_marks: u32,
    pub marks_checksum_ok: Option<bool>,
    pub mark_checks: Vec<MarkCheck>,
    pub quarantined: Vec<QuarantineEvent>,
    pub skipped_pages: Vec<SkippedPage>,
    pub repairs: usize,
    pub salvage_events: usize,
    pub crop_rejections: usize,
    pub diagrams_saved: usize,
    pub diagrams_deduped: usize,
    pub anomalies: Vec<String>,
    pub timings: Vec<TimingEntry>,
}

/// Concurrent vision calls in flight at once. Validation is per unit of work
/// (page / span / window), so running units in parallel changes NOTHING about
/// correctness — every response still passes the same Rust gates. It only
/// stops us paying API latency serially. 429 backpressure is per-call
/// (llm.rs), so bursts self-limit.
const DEFAULT_PARALLEL: usize = 4;
const PAGE_RENDER_CACHE_CAPACITY: usize = 4;

async fn chat_with_permit<C: LlmClient>(
    client: &C,
    body: &serde_json::Value,
    semaphore: &Arc<Semaphore>,
) -> Result<serde_json::Value, crate::llm::LlmError> {
    let _permit = semaphore
        .acquire()
        .await
        .map_err(|_| crate::llm::LlmError::Network("request semaphore closed".to_string()))?;
    client.chat(body).await
}

struct ChunkImageInput {
    chunk_idx: usize,
    b64: String,
    start_y: Option<f32>,
    end_y: Option<f32>,
}

struct PreparedChunk {
    images: Vec<String>,
    local_to_chunk: Vec<usize>,
    page_bands: Vec<Option<(f32, f32)>>,
    page_crop_offsets: Vec<(f32, f32)>,
    decoded_pages: Vec<Option<Arc<image::DynamicImage>>>,
}

struct DiagramSaveRequest {
    global_page_idx: usize,
    bbox: Vec<f32>,
    ignore_grid: bool,
    graph_like: bool,
}

struct DiagramPersistence {
    links: Vec<Option<String>>,
    saved: Vec<([u8; 64], String)>,
    report: ImportReport,
}

async fn persist_diagrams(
    requests: Vec<DiagramSaveRequest>,
    page_b64: std::collections::HashMap<usize, String>,
    config: PipelineConfig,
    page_render_cache: Arc<crate::pdf_render::PageRenderCache>,
    saved: Vec<([u8; 64], String)>,
) -> Result<DiagramPersistence, tokio::task::JoinError> {
    tokio::task::spawn_blocking(move || {
        let mut saved = saved;
        let mut report = ImportReport::default();
        let mut links = Vec::with_capacity(requests.len());
        for request in requests {
            links.push(save_diagram(
                request.global_page_idx,
                page_b64.get(&request.global_page_idx).map(String::as_str),
                &request.bbox,
                &config,
                page_render_cache.as_ref(),
                &mut saved,
                &mut report,
                request.ignore_grid,
                request.graph_like,
            ));
        }
        DiagramPersistence {
            links,
            saved,
            report,
        }
    })
    .await
}

async fn prepare_chunk_images(
    chunk_len: usize,
    inputs: Vec<ChunkImageInput>,
) -> Result<PreparedChunk, tokio::task::JoinError> {
    tokio::task::spawn_blocking(move || {
        let mut images = Vec::with_capacity(inputs.len());
        let mut local_to_chunk = Vec::with_capacity(inputs.len());
        let mut page_bands = vec![None; chunk_len];
        let mut page_crop_offsets = Vec::with_capacity(inputs.len());
        let mut decoded_pages = vec![None; chunk_len];

        for input in inputs {
            let decoded = geometry::decode_page_image(&input.b64).map(Arc::new);
            let mut final_b64 = input.b64;
            let mut crop_offset = (0.0_f32, 1.0_f32);

            if input.start_y.is_some() || input.end_y.is_some() {
                let start = (input.start_y.unwrap_or(0.0) - 0.03).max(0.0);
                let end = (input.end_y.unwrap_or(1.0) + 0.03).min(1.0);
                if let Some(cropped) = decoded
                    .as_deref()
                    .and_then(|image| geometry::crop_page_vertical_from_image(image, start, end))
                {
                    final_b64 = cropped.b64;
                    crop_offset = (cropped.y_offset_frac, cropped.height_frac);
                }
                page_bands[input.chunk_idx] = Some((
                    input.start_y.unwrap_or(0.0),
                    input.end_y.unwrap_or(1.0),
                ));
            }

            // Phase 4: downsample the API image so the longest edge does not
            // exceed 1024 px. The retained decoded page and the 300-DPI cache
            // used by persist_diagrams remain at full resolution so physical
            // crops preserve original precision. Bounding boxes returned by
            // the vision model are expressed as fractions of this downsized
            // image, but the pipeline converts them back to absolute pixels
            // against the cached high-res page during cropping, so no
            // coordinate remapping is needed here.
            if let Some(img) = &decoded {
                let (w, h) = img.dimensions();
                let max_dim: u32 = 1024;
                if w > max_dim || h > max_dim {
                    let scale = max_dim as f32 / (w.max(h) as f32);
                    let new_w = (w as f32 * scale).round().max(1.0) as u32;
                    let new_h = (h as f32 * scale).round().max(1.0) as u32;
                    let resized = image::imageops::resize(
                        img.as_ref(),
                        new_w,
                        new_h,
                        image::imageops::FilterType::Triangle,
                    );
                    let mut buf = std::io::Cursor::new(Vec::with_capacity(
                        (new_w as usize * new_h as usize) / 8,
                    ));
                    use image::codecs::jpeg::JpegEncoder;
                    use image::ImageEncoder;
                    // Quality 80: visually identical for text/line-art OCR but
                    // ~35% smaller than 92. Reduces upload + provider processing
                    // time across all API calls. Diagram crops written to disk
                    // are unaffected (those go through save_diagram at full res).
                    let enc = JpegEncoder::new_with_quality(&mut buf, 80);
                    if enc
                        .write_image(
                            &resized,
                            new_w,
                            new_h,
                            image::ExtendedColorType::Rgba8,
                        )
                        .is_ok()
                    {
                        use base64::Engine;
                        final_b64 = base64::engine::general_purpose::STANDARD.encode(buf.into_inner());
                    }
                }
            }

            decoded_pages[input.chunk_idx] = decoded;
            images.push(final_b64);
            local_to_chunk.push(input.chunk_idx);
            page_crop_offsets.push(crop_offset);
        }

        PreparedChunk {
            images,
            local_to_chunk,
            page_bands,
            page_crop_offsets,
            decoded_pages,
        }
    })
    .await
}

impl ImportReport {
    /// Fold a per-unit report (one span / page / window processed inside a
    /// parallel batch) back into the master report.
    pub fn absorb(&mut self, o: ImportReport) {
        self.pages_processed += o.pages_processed;
        self.repairs += o.repairs;
        self.salvage_events += o.salvage_events;
        self.crop_rejections += o.crop_rejections;
        self.diagrams_saved += o.diagrams_saved;
        self.diagrams_deduped += o.diagrams_deduped;
        self.mark_checks.extend(o.mark_checks);
        self.quarantined.extend(o.quarantined);
        self.skipped_pages.extend(o.skipped_pages);
        self.anomalies.extend(o.anomalies);
        self.timings.extend(o.timings);
    }

    /// Record a timing entry.
    pub fn record_timing(
        &mut self,
        stage: &str,
        operation: &str,
        page: Option<usize>,
        question_number: Option<u32>,
        milliseconds: u64,
    ) {
        self.timings.push(TimingEntry {
            stage: stage.to_string(),
            operation: operation.to_string(),
            page,
            question_number,
            milliseconds,
        });
    }
}

#[derive(Debug, Clone)]
pub struct BuiltQuestion {
    pub question_number: u32,
    pub content: String,
    pub marks: i32,
    pub topics: Vec<String>,
    pub module: String,
    pub is_code: bool,
    pub needs_review: bool,
    #[allow(dead_code)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AnswerDraft {
    pub question_number: u32,
    pub markdown: String,
}

// ══════════════════════════════════════════════════════════════════════════
// AI response schemas (tolerant: numbers/marks/topics arrive as Value and
// are normalized deterministically — a type slip can't kill an extraction)
// ══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Default, serde::Deserialize, Clone)]
#[serde(default)]
struct AiQuestion {
    question_number: Option<serde_json::Value>,
    content: Option<String>,
    marks: Option<serde_json::Value>,
    topics: Option<serde_json::Value>,
    module: Option<String>,
    is_code: Option<bool>,
    diagram_bboxes: Option<Vec<Vec<f32>>>,
    /// Semantic figure metadata is separate from crop geometry.
    diagram_captions: Option<Vec<String>>,
    diagram_kinds: Option<Vec<String>>,
    bbox_page_indexes: Option<Vec<serde_json::Value>>,
    math_snippet: Option<String>,
    #[serde(alias = "choice_layout", alias = "option_layout", alias = "visual_option_type")]
    visual_options: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct AiQuestionPage {
    items: Vec<AiQuestion>,
}

fn merge_split_questions(items: Vec<AiQuestion>, question_number: u32) -> AiQuestion {
    let mut merged = AiQuestion {
        question_number: Some(serde_json::json!(question_number)),
        ..Default::default()
    };
    let mut content = Vec::new();
    let mut bboxes = Vec::new();
    let mut captions = Vec::new();
    let mut kinds = Vec::new();
    let mut indexes = Vec::new();
    let mut topics = Vec::new();
    let mut marks = 0i32;
    let mut has_marks = false;

    for item in items {
        if let Some(value) = item.content.filter(|value| !value.trim().is_empty()) {
            content.push(value);
        }
        if let Some(value) = item.diagram_bboxes {
            bboxes.extend(value);
        }
        if let Some(value) = item.diagram_captions {
            captions.extend(value);
        }
        if let Some(value) = item.diagram_kinds {
            kinds.extend(value);
        }
        if let Some(value) = item.bbox_page_indexes {
            indexes.extend(value);
        }
        if let Some(value) = item.topics {
            topics.extend(value_to_topics(&value));
        }
        if let Some(value) = item.marks.as_ref().and_then(validate::value_to_marks) {
            marks += value;
            has_marks = true;
        }
        merged.module = merged.module.or(item.module);
        merged.is_code = merged.is_code.or(item.is_code);
        merged.math_snippet = merged.math_snippet.or(item.math_snippet);
        if merged.visual_options.is_none()
            && item.visual_options.as_deref() == Some("composite_visual_options")
        {
            merged.visual_options = item.visual_options;
        }
    }

    merged.content = Some(content.join("\n\n"));
    if has_marks {
        merged.marks = Some(serde_json::json!(marks));
    }
    if !topics.is_empty() {
        merged.topics = Some(serde_json::json!(topics));
    }
    if !bboxes.is_empty() {
        merged.diagram_bboxes = Some(bboxes);
    }
    if !captions.is_empty() {
        merged.diagram_captions = Some(captions);
    }
    if !kinds.is_empty() {
        merged.diagram_kinds = Some(kinds);
    }
    if !indexes.is_empty() {
        merged.bbox_page_indexes = Some(indexes);
    }
    merged
}

fn is_composite_visual_options(item: &AiQuestion) -> bool {
    item.visual_options.as_deref() == Some("composite_visual_options")
        || item
            .diagram_kinds
            .as_ref()
            .map(|kinds| {
                kinds
                    .iter()
                    .any(|kind| kind == "composite_visual_options")
            })
            .unwrap_or(false)
}

fn is_visual_option_marker(line: &str) -> bool {
    let trimmed = line.trim();
    let Some(first) = trimmed.chars().next() else {
        return false;
    };
    if !matches!(first, 'A' | 'B' | 'C' | 'D') {
        return false;
    }
    let rest = trimmed[first.len_utf8()..].trim_start();
    rest.is_empty()
        || rest.starts_with("[DIAGRAM_PLACEHOLDER]")
        || matches!(rest.chars().next(), Some(')' | '.' | ':'))
}

/// Convert visual A-D choices into one placeholder-backed composite image.
/// Text-only MCQs do not enter this path.
fn normalize_composite_visual_options(item: &mut AiQuestion) {
    if !is_composite_visual_options(item) {
        return;
    }

    let Some(bboxes) = item.diagram_bboxes.clone() else {
        return;
    };
    let indexes = item.bbox_page_indexes.clone().unwrap_or_default();
    if bboxes.len() > 1 {
        let Some(first_page) = indexes.first().and_then(value_to_usize) else {
            return;
        };
        if indexes.len() != bboxes.len()
            || indexes
                .iter()
                .filter_map(value_to_usize)
                .any(|page| page != first_page)
        {
            // A single bitmap cannot span multiple page images. Preserve the
            // original per-page proposals rather than creating an invalid
            // cross-page crop.
            return;
        }
        let Some(union) = geometry::union_relative_bboxes(&bboxes) else {
            return;
        };
        item.diagram_bboxes = Some(vec![union]);
        item.bbox_page_indexes = Some(vec![serde_json::json!(first_page)]);
    }

    item.diagram_captions = Some(vec!["Composite visual options".to_string()]);
    item.diagram_kinds = Some(vec!["composite_visual_options".to_string()]);

    let Some(content) = item.content.as_deref() else {
        return;
    };
    let lines: Vec<&str> = content.lines().collect();
    let marker_positions: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| is_visual_option_marker(line).then_some(index))
        .collect();
    let placeholder_count = content.matches("[DIAGRAM_PLACEHOLDER]").count();
    if marker_positions.len() >= 2 && placeholder_count > 0 {
        let prefix = lines[..marker_positions[0]].join("\n").trim_end().to_string();
        item.content = Some(if prefix.is_empty() {
            "[DIAGRAM_PLACEHOLDER]".to_string()
        } else {
            format!("{}\n[DIAGRAM_PLACEHOLDER]", prefix)
        });
    } else if placeholder_count > 1 {
        let mut collapsed = String::new();
        for (index, part) in content.split("[DIAGRAM_PLACEHOLDER]").enumerate() {
            if index > 0 {
                if index == 1 {
                    collapsed.push_str("[DIAGRAM_PLACEHOLDER]");
                }
            }
            collapsed.push_str(part);
        }
        item.content = Some(collapsed.trim().to_string());
    }
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct AiAnswer {
    question_number: Option<serde_json::Value>,
    answer_markdown: Option<String>,
    diagram_bboxes: Option<Vec<Vec<f32>>>,
    diagram_page_indexes: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum AiAnswerEnvelope {
    Wrapped {
        #[serde(default)]
        answers: Vec<AiAnswer>,
    },
    Bare(Vec<AiAnswer>),
}

fn value_to_usize(v: &serde_json::Value) -> Option<usize> {
    match v {
        serde_json::Value::Number(n) => n
            .as_u64()
            .or_else(|| {
                n.as_f64().and_then(|f| {
                    if f.fract() == 0.0 {
                        Some(f as u64)
                    } else {
                        None
                    }
                })
            })
            .map(|x| x as usize),
        serde_json::Value::String(s) => s.trim().parse::<usize>().ok(),
        _ => None,
    }
}

fn value_to_topics(v: &serde_json::Value) -> Vec<String> {
    match v {
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|t| t.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect(),
        serde_json::Value::String(s) if !s.trim().is_empty() => vec![s.trim().to_string()],
        _ => Vec::new(),
    }
}

fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Import cancelled by user".to_string())
    } else {
        Ok(())
    }
}

/// Phase 1: heuristic used in fallback mode to decide whether a chunk
/// that arrived with the same question_number as the last built card is
/// truly a continuation, or a new short-answer/MCQ question that the
/// model mislabeled. Returns true when the new content strongly "looks
/// like" the start of a different question.
fn looks_like_new_question(prev_content: &str, new_content: &str) -> bool {
    let new = new_content.trim_start();
    let prev = prev_content;

    // Need a regex cache for these checks (one-time cost).
    use std::sync::OnceLock;
    static RE_SECTION_A: OnceLock<regex::Regex> = OnceLock::new();
    static RE_STARTS_W_ONE_MARK: OnceLock<regex::Regex> = OnceLock::new();
    static RE_HAS_BOLD_HEADER: OnceLock<regex::Regex> = OnceLock::new();
    static RE_QUESTION_NUM_START: OnceLock<regex::Regex> = OnceLock::new();

    // (a) resets part labels: new content begins with (a) / **(a)** and the
    //     previous content already advanced to (b) or later.
    let re_section_a = RE_SECTION_A.get_or_init(|| {
        regex::Regex::new(r"(?i)^\s*\*?\*?\s*[\(\[]\s*a\s*[\)\]]").unwrap()
    });
    if re_section_a.is_match(new) {
        // Did previous content ever get to (b), (c), ..., (i), (ii)?
        for lbl in ["b", "c", "d", "e", "f", "g", "h", "i", "j"] {
            let pat = format!(r"(?i)\({}\)", lbl);
            if let Ok(re) = regex::Regex::new(&pat) {
                if re.is_match(prev) {
                    return true;
                }
            }
        }
    }

    // (b) new content has a marks tag within the first 80 chars AND a
    //     period/question structure, suggesting a 1-mark question.
    let first_eighty: String = new.chars().take(80).collect();
    let re_marks = RE_STARTS_W_ONE_MARK.get_or_init(|| {
        regex::Regex::new(r"(?i)\[\s*\d{1,2}\s*marks?\s*\]").unwrap()
    });
    if re_marks.is_match(&first_eighty) && first_eighty.chars().filter(|c| c.is_alphabetic()).count() > 20 {
        return true;
    }

    // (c) new content begins with a bold heading ("**5.**" / "**Question 5**" / "**5**").
    let re_bold = RE_HAS_BOLD_HEADER.get_or_init(|| {
        regex::Regex::new(r"^\s*\*\*\s*(?:question\s*)?\d{1,2}\s*[\.\)\]]?\s*\*\*").unwrap()
    });
    if re_bold.is_match(new) {
        return true;
    }

    // (d) new content starts with a clear question number: "5.", "5)", "Q5".
    let re_num = RE_QUESTION_NUM_START.get_or_init(|| {
        regex::Regex::new(r"(?m)^\s*(?:Q(?:uestion)?\.?\s*)?0*[1-9]\d{0,2}\s*[\.\)\]]").unwrap()
    });
    if re_num.is_match(new) {
        return true;
    }

    false
}



// ══════════════════════════════════════════════════════════════════════════
// Prompts
// ══════════════════════════════════════════════════════════════════════════

fn structure_system_prompt() -> String {
    r#"You are an exam-document layout analyzer. Your single most important job is to draw the EXACT boundary between one top-level question and the next, so that no sub-part is ever assigned to the wrong parent question. Look at ONE page and report ONLY structural facts as a JSON object:

{
  "question_numbers_visible": [ints],
  "question_y_fracs": [[y_start, y_end], ...],
  "total_marks_footer": [question_number, marks] or null,
  "total_marks_footer_y": number or null,
  "page_role": "QUESTION" | "COVER" | "INSTRUCTIONS" | "BLANK" | "ANSWER_BOOKLET" | "REFERENCE"
}

All y values are fractions of page HEIGHT (0.0 = very top of printable area, 1.0 = very bottom). Measure by looking at where the question's TEXT starts and ends on the page.

═══ PRIME DIRECTIVE: ONE BAND = ONE MAIN QUESTION ═══
Each [y_start, y_end] band is used to CROP the page for a transcriber that is told "everything in this band is Question N". So a band MUST contain ALL sub-parts of its own main question and ZERO content from any other main question. Bands on one page MUST NOT OVERLAP and MUST be in strictly increasing y order (band[i] y_end <= band[i+1] y_start). A band that swallows even one line of the next question welds two questions together — the worst failure this system can produce.

═══ SCANNING PROCEDURE (follow in order) ═══
1. Sweep the page top-to-bottom and note the y of EVERY printed question label: whole numbers (1, 2, 3 / "0 1" / "17") AND sub-part labels ((a), (b), (i), 3.2, "01 5", "2 (b) (ii)").
2. For each label decide its PARENT main number. A sub-part label inherits the main number of the nearest whole-number heading ABOVE it, UNLESS the label itself prints a main number ("03.2" -> parent 3, "2(b)" -> parent 2). A printed main number always beats the "nearest heading above" heuristic.
3. Group labels by parent main number.
4. For each group: y_start = top of its FIRST element on this page, y_end = bottom of its LAST element on this page.
5. Verify: groups do not overlap and ascend in y. If two groups overlap you mis-assigned a sub-part in step 2 — redo step 2.

═══ WHERE EXACTLY DOES A QUESTION END? ═══
Walk down from the question's start until you hit the FIRST of these terminators and put y_end just ABOVE it:
  * the heading of the next main question (its number, or its first sub-part label such as "0 4 . 1" / "4 (a)");
  * the "(Total for Question N is M marks)" / "[Total: 8]" footer (that footer belongs to the footer fields, not to the band);
  * a horizontal rule or shaded separator bar the paper uses between questions;
  * the bottom of the printable area (question continues on the next page).
Answer lines, dotted rules and blank working space BEFORE the terminator still belong to question N — include them. Never let y_end reach or pass the next question's heading.

═══ MOST COMMON ERRORS TO AVOID ═══
- Treating a sub-part label like "(c)" or "03.4" as the start of a NEW main question. It is not; it extends the CURRENT question's band.
- Cutting question N at its last visible text while the next heading sits just below it, leaving that heading inside N's band. Cut ABOVE the next heading, always.
- Merging two adjacent main questions into one band because they look visually continuous. Two printed whole numbers = two entries, always.
- Assuming a page holds only one question. Re-scan the bottom third: short questions often start there.

RULES:
- "question_numbers_visible": WHOLE question numbers only, each listed AT MOST ONCE, in TOP-TO-BOTTOM order. IGNORE sub-questions (e.g. 1.1, 1a), page numbers and mark allocations (e.g. [2 marks]) as separate entries — they only widen their PARENT's band. AQA prints "0 1" for Q1, "0 2" for Q2 — those are question numbers 1, 2. "03.1" means sub-part 1 of Q3 so the visible whole number is 3. AQA also prints SPACED sub-parts: "01 5" means Question 1, sub-part 5 — the whole number is 1 (NOT 1.5, NOT 15). NEVER return decimals or concatenate spaced digits. Sub-part letters (a)(b)(c) and decimal labels alone are NOT whole question numbers.
- A question continuing from the previous page whose sub-parts carry on here is still ONE entry: its whole number with y_start = 0.0.
- MULTIPLE CHOICE / SHORT-ANSWER PAGES: when several independent questions share ONE page (MCQs, 1- or 2-mark questions), list EVERY question number that appears — e.g. [1,2,3,4,5] for 5 MCQs — each with its own tight, non-overlapping band. Do NOT bundle them. This is the most important rule on dense pages.
- "question_y_fracs": array of the SAME LENGTH as question_numbers_visible. Each entry is [y_start, y_end] for that question's vertical extent on THIS page:
    * y_start: fraction where the question (including its number/bold heading) begins, e.g. 0.05 for a question at the top.
    * y_end: fraction where the question ends — including ALL of its OWN sub-parts and working space, but NOT the "Total for Question N is M marks" line and NOT one pixel of the next question.
    * For a question that runs off the BOTTOM of the page (continues next page), set y_end to ~0.98.
    * For a question that starts ABOVE this page (continues from a previous page), set y_start to 0.0.
    * Be precise — within 0.02. Too tight truncates a question; too loose welds it to its neighbour. Leave ~0.01 padding, and when the gap between question N's last line and question N+1's heading is under 0.02, put the boundary at the MIDPOINT of that gap and give BOTH bands that same value (touching, never overlapping).
- "total_marks_footer": only if a line like "(Total for Question 5 is 8 marks)" or "[Total: 8]" is printed on this page. Format: [5, 8]. Otherwise null. This footer also CONFIRMS a boundary: everything below it belongs to the next question.
- "total_marks_footer_y": if you returned a total_marks_footer, the y-fraction of that footer line on the page.
- page_role: COVER (front cover / candidate details), INSTRUCTIONS (rubric, formula sheet), BLANK (empty or "BLANK PAGE"), ANSWER_BOOKLET (empty lined/dotted student writing space), REFERENCE (formula / data sheet), otherwise QUESTION.
- Output ONLY the JSON object. No commentary. No markdown."#
        .to_string()
}

/// Phase 1: describe the vertical clip for each page (if any) in the
/// user-facing transcription prompt. This is the CHEAP alternative to
/// physically cropping the page image: we tell the model exactly which
/// portion of each page contains Question N so it ignores neighbouring
/// questions. Combined with the hard `question_number` validator (which
/// rejects content addressed to another question number) and the
/// diagram-bbox y-range check, this produces the same correctness
/// guarantees as pixel-cropping without the complexity of shifting
/// coordinates through audit/save/dedupe.
#[allow(dead_code)]
fn page_band_note(span: &QuestionSpan, page_index_in_span: usize, total_pages_in_span: usize) -> Option<String> {
    let is_first = page_index_in_span == 0;
    let is_last = page_index_in_span + 1 == total_pages_in_span;
    // Only emit a note when there's a clip on this page (otherwise the
    // page is full-width / full-height and the existing rules apply).
    let start_clip = if is_first { span.start_y_frac } else { None };
    let end_clip = if is_last { span.end_y_frac } else { None };
    if start_clip.is_none() && end_clip.is_none() {
        return None;
    }
    let mut note = String::from(
        "IMPORTANT: this page image contains parts of MULTIPLE questions. Transcribe ONLY the content belonging to Question N that sits between these vertical positions (fraction of page height from the top, 0.0=top, 1.0=bottom):\n",
    );
    if let Some(s) = start_clip {
        note.push_str(&format!("- Start reading {:.0}% of the way DOWN from the top of the page.\n", s * 100.0));
    } else {
        note.push_str("- Start reading from the very top of the page.\n");
    }
    if let Some(e) = end_clip {
        note.push_str(&format!("- STOP at about {:.0}% of the way down the page. Ignore everything below that line — it belongs to the next question.\n", e * 100.0));
    } else {
        note.push_str("- Continue reading to the bottom of the page (the question continues onto the next page).\n");
    }
    note.push_str(
        "If the heading of a different main question (or a sub-part label printed with a different main number) appears inside this band, STOP transcribing at that heading — those sub-parts belong to another question and must never be merged into this one.\n",
    );
    Some(note)
}

fn extraction_system_prompt(config: &PipelineConfig, span: &QuestionSpan) -> String {
    let topics_instruction = if config.allowed_topics.is_empty() {
        "- \"topics\": array. MUST be empty []. Do NOT invent topics.".to_string()
    } else {
        format!(
            "- \"topics\": array. At least one. Select ONLY from this exact list: {:?}. Never invent topics.",
            config.allowed_topics
        )
    };

    format!(
        r#"You are a precise mathematical OCR engine transcribing exactly ONE exam question. Output ONLY a valid JSON object of the form {{"items": [ ... ]}}.

CONTEXT: The page image(s) show Question {number} of the paper '{paper}'. They may ALSO show the tail of the previous question or the head of the next one. Transcribe ONLY content that belongs to Question {number}. If nothing on these pages belongs to Question {number}, return {{"items": []}}.

═══ ABSOLUTE RULE — QUESTION ISOLATION (highest priority, overrides everything below) ═══
You are transcribing Question {number} and NOTHING ELSE. Sub-parts belonging to a different main question must NEVER appear in your output, not even one line of them.
1. OWNERSHIP TEST — before transcribing any line, ask: "which main question owns this line?" A sub-part label ((a), (b), (i), 4.2, "0 4 . 3", "5 (b) (ii)") belongs to the main question printed in the label itself, or, when the label prints no main number, to the nearest whole-number heading ABOVE it. If the owner is not {number}, DROP the line. When you cannot prove a line belongs to {number}, DROP it.
2. HARD STOP — while reading downward, the moment you meet ANY of these, stop transcribing immediately and never resume on that page:
   * a printed whole question number other than {number} (e.g. "{number} " followed later by the next integer, "0 5", "17");
   * a sub-part label whose printed main number is not {number} (e.g. "04.1" when you are extracting Question 3);
   * a "(Total for Question {number} is M marks)" / "[Total: M]" footer for Question {number} — that footer marks the END of your question;
   * a separator rule/shaded bar introducing a new question.
3. HARD START — skip everything above the start of Question {number}. Trailing sub-parts, answer lines or figures of the PREVIOUS question that appear at the top of the page are NOT yours, even if they flow visually into your question.
4. SUB-PART CONTINUITY — Question {number} may span several pages; its sub-parts continue in order ((a), (b), (c) ... never resetting). If a page begins with "(d)" and Question {number}'s previous page ended at "(c)", that "(d)" is yours. But if lettering RESTARTS at (a) after a totals footer, that (a) belongs to the NEXT question — drop it.
5. NEVER renumber, merge or "helpfully" include a neighbouring question's parts to make the item look complete. A short, correctly-isolated item is always better than a merged one. Merged questions are rejected outright.
6. If a y-band hint ("transcribe only between X% and Y% down the page") is given in the user message, treat it as authoritative for WHAT to transcribe, in addition to rules 1-4.

SELF-CHECK BEFORE OUTPUT: re-read your "content". Every sub-part label in it must belong to Question {number}. Any label that came after a totals footer, or that printed a different main number, must be deleted. Confirm your content contains no second main-question heading.

Normally there is exactly ONE item. Return more than one ONLY if Question {number} visibly consists of independent numbered tasks on these pages. NEVER return an item for a question number other than {number}.

EVERY item MUST have:
- "question_number": {number} (integer, exactly).
- "content": FULL transcription of Question {number} only (never a summary). Preserve all punctuation. Separate sub-parts (a), (b), (c) with double newlines, keeping them in printed order. Append the mark tag `**[X marks]**` to every sub-part that shows a mark allocation. Transcribe every sentence, including instructions to the candidate that belong to this question. Do NOT include: any text belonging to another question, page headers/footers ("Question X continued", "Turn over"), the "(Total for Question X is Y marks)" footer, plain ruled answer lines, or "BLANK PAGE".
- STRUCTURED TABLES WITH HEADERS — trace tables, function tables, working grids — ARE question content even when the body cells are EMPTY. If the text says Complete the trace table, Complete the table, or show the results of executing, NEVER return a diagram box for that grid, even when the question mentions another Figure; transcribe every row and pre-filled cell as Markdown. Transcribe them as Markdown tables in "content" (keeping every header and any pre-filled cells), NEVER as diagram boxes.
- "marks": integer total for this question's visible part, or null if unknown.
{topics_instruction}
- "module": string — output EXACTLY '{module}'.
- "is_code": boolean (true only for code/pseudocode questions).
- "diagram_captions": array of captions, one per figure box, or empty string; "diagram_kinds": array of semantic kinds such as graph, schema, flowchart, circuit, or multi-panel, one per box. Decide whether each exhibit is a figure before proposing geometry.
- "visual_options": null for ordinary questions and text-only multiple-choice options. Set exactly to "composite_visual_options" when the answer choices are primarily visual assets (graphs, diagrams, plots, circuits, or illustrated answer choices). For that case, return ONE diagram placeholder and ONE bounding box spanning the complete choices block from the start of option A through the bottom of option D, including the A/B/C/D labels, all graphs or diagrams, tick boxes, axes, captions, and surrounding whitespace. Never emit one crop per visual option.
- "diagram_bboxes": array of [x, y, w, h] boxes with RELATIVE 0.0-1.0 coordinates, one per visual exhibit. IMPORTANT: coordinates are ALWAYS relative to the FULL page image (0,0 at the top-left corner of the page, 1,1 at the bottom-right), EVEN when the prompt tells you to only transcribe between certain y-percentages (multi-question pages). The y-band is only a hint for what to TRANSCRIBE — never shift or rescale bbox coordinates to match the band. Box EVERY figure the paper draws — graphs, networks, trees, circuits — INCLUDING anything the paper labels as a Figure (e.g. "Figure 6"): printed relation/database schemas, algorithm screens, and grids that are part of the question exhibit are figures, return them as boxes, not as text. One box per WHOLE figure including its labels/caption, never two boxes on one figure. Do NOT box plain question text, tables you transcribed as Markdown (STRUCTURED TABLES rule above), or EMPTY student answer grids. The parser crop-checks every box (and rejects boxes whose center falls outside the question's band on multi-question pages). Include the complete semantic figure extent, including captions and disconnected components, rather than tight-boxing one shape.
GRAPH/CANVAS EXTENT: for every graph or chart, the box MUST include the complete visual canvas, not merely the plotted grid. Include the far-left y-axis title, variables, units, numeric tick labels and axis line; the bottom x-axis title, variables, units and tick labels; the top/right border or grid edge; and the printed Figure heading/caption. Leave visible whitespace around these elements. A graph crop that starts at the y-axis or ends at the plot border is incomplete.
The parser crop-checks every box: blank boxes, empty ruled grids, and duplicate boxes are rejected and cost you a repair round.
- "bbox_page_indexes": array with the SAME LENGTH as diagram_bboxes — the 0-based index of the page image each box refers to.
- Insert the exact token [DIAGRAM_PLACEHOLDER] in "content" where each diagram belongs chronologically.

FORMATTING RULES:
- OMIT the leading question number at the very start of the question text (e.g. if the text reads "17 Here is triangle ABC.", you MUST output "Here is triangle ABC." without the "17").
- OMIT trailing answer line units, symbols, and answer templates at the very end of the question (e.g. "..................... %", "£ .....................", "..................... cm", or "............ $\\le t <$ ............"). Do NOT transcribe the answer blanks or the mathematical operators embedded within them.
- Wrap inline math in single $...$. Use $$...$$ ONLY for display equations on their own line.
- Tables of text/data: standard Markdown tables. Pure mathematical matrices or Simplex tableaus: LaTeX \begin{{array}} inside $$...$$. Never put $ inside array environments.
- Multiple-choice options: keep their original capital letter labels (e.g. `A ...`, `B ...`) separated by newlines. Do NOT format them as lowercase sub-parts like `(a)`.
- Code/pseudocode/SQL/identifiers: Markdown backticks, NEVER LaTeX math mode.
- AQA decimal sub-parts: render '02.1'-style parts as (a), (b), (c) — positionally: .1 -> a, .2 -> b — and update inline cross-references accordingly. AQA also uses SPACED sub-parts: "01 5" means Question 1, sub-part 5 — render as (e). The whole question number is ALWAYS the integer before the space/dot. NEVER return decimals like 1.5 for spaced sub-parts. Whole-numbered MCQs are independent questions, never decimals.
- JSON ESCAPING: backslashes in LaTeX MUST be escaped (\\frac, \\theta). Unescaped backslashes break the parser and your work is discarded.
- The content MUST end with terminal punctuation or a mark tag. Never stop mid-sentence."#,
        number = span.number,
        paper = config.paper_name,
        module = config.module_name,
        topics_instruction = topics_instruction,
    )
}

fn markscheme_system_prompt() -> String {
    r#"You are an expert examiner transcribing a mark scheme into Markdown. Return ONLY a valid JSON object: {"answers": [...]} (or an empty array [] / {"answers": []} when the pages contain no real answers).

ESCAPE HATCH: If the images show only front covers, general marking guidance, abbreviation lists, or formula booklets, return an empty array. NEVER invent questions to fill the output.
EXTRACTION GUARDRAIL: Only extract entries with explicit mark-scheme structure: a question-number column header (e.g. 1(a), 2(b)(i)) AND mark labels (M1, A1, B1, dM1, ft). Numbered lists in guidance pages are NOT mark schemes.

Each array item: { "question_number": int (WHOLE question only; AQA 03.1 → 3), "answer_markdown": string, "diagram_bboxes": [[x,y,w,h]...] relative 0.0-1.0, "diagram_page_indexes": [ints, same length as bboxes, 0-based image index] }.

RULES:
- Group every part of one question (main + ONE alternative method max) into a SINGLE item for that question_number. Further alternatives: discard. Alternative appended after a Markdown divider `---` and a bold "**ALTERNATIVE METHOD**" header.
- Part labels bolded on their own line: **(a)**. Every distinct marking step separated by a double newline. Inline math with single $...$; display equations with $$...$$ on their own line. NEVER use code fences.
- Sub-part letters must continue across pages: do not reset (g) back to (a).
- Exclude: examiner notes about mark codes, page headers/footers, AQA margin numbers, blank answer-line numbers, and reprinted question text (the REPRINT BAN).
- Data/trace tables: Markdown tables. True matrices/Simplex tableaus: \begin{array} in $$...$$.
- Diagrams (activity networks, Gantt charts, trees, graphs): capture via diagram_bboxes + diagram_page_indexes and insert [DIAGRAM_PLACEHOLDER] where the diagram belongs. NEVER box text, math working, examiner notes, or empty grids (the CRITICAL DIAGRAM BAN).
- JSON ESCAPING: escape LaTeX backslashes (\\frac not \frac). Invalid JSON is rejected outright and your work is lost.
- You are a transcriber, not a solver. If there is no question-number column with mark labels on these pages, return an empty array."#
        .to_string()
}

// ══════════════════════════════════════════════════════════════════════════
// Question pipeline
// ══════════════════════════════════════════════════════════════════════════

pub async fn run_question_pipeline<C: LlmClient, P: Progress>(
    client: &C,
    pages: &[PageInput],
    config: &PipelineConfig,
    progress: &P,
    cancel: &AtomicBool,
) -> Result<(Vec<BuiltQuestion>, ImportReport), String> {
    let mut report = ImportReport {
        paper_name: config.paper_name.clone(),
        kind: "questions".to_string(),
        pages_total: pages.len(),
        ..Default::default()
    };
    let page_render_cache = Arc::new(crate::pdf_render::PageRenderCache::new(
        PAGE_RENDER_CACHE_CAPACITY,
    ));
    let request_semaphore = Arc::new(Semaphore::new(config.parallelism.max(1)));

    // Prefer the free PDF text layer: it avoids one vision request per page.
    let page_texts: Vec<String> = pages.iter().map(|p| p.text.clone()).collect();

    // Time the text-layer document map building
    let text_map_start = Instant::now();
    let scan = doc_map::scan_text_layer(&page_texts);
    // The vision structure pass (one AI call per page) is skippable when the
    // text layer alone can build the map: either via reliable footers
    // (Edexcel-style) or via a sufficiently dense heading sequence (AQA-style,
    // verified across all '17–'24 physics papers). Scanned/garbled PDFs fail
    // the check and keep the vision structure pass as before.
    let text_map_available = (!scan.footers.is_empty()
        && scan
            .page_reliability
            .iter()
            .all(|r| *r != doc_map::PageReliability::Ambiguous))
        || doc_map::text_layer_map_sufficient(&scan, pages.len());
    report.record_timing(
        "document_map",
        "text_layer_scan",
        None,
        None,
        text_map_start.elapsed().as_millis() as u64,
    );

    // ── 1. Structure pass ───────────────────────────────────────────────────
    // For papers where the text layer is NOT sufficient, we need the vision
    // structure pass (one AI call per page). Rather than running it
    // sequentially before map building, we overlap the structure pass with
    // the initial map-building setup via tokio::join!. Both are read-only
    // on the shared data and the semaphore naturally distributes permits
    // between structure-pass API calls and extraction API calls.
    let mut structures: Vec<ValidatedPageStructure> = Vec::with_capacity(pages.len());
    let mut structure_timing_ms: u64 = 0;
    if !text_map_available {
        progress.stage("Scanning document structure…");
        let structure_start = Instant::now();
        let system_structure = structure_system_prompt();
        let unknown_role = |i: usize| ValidatedPageStructure {
            page: i,
            questions: Vec::new(),
            question_y: Vec::new(),
            footer: None,
            footer_y: None,
            role: doc_map::PageRole::Unknown,
        };

        // Fire the structure pass concurrently with map-building setup.
        // Both read from the same shared data; the request_semaphore
        // naturally distributes permits between structure and extraction
        // API calls.
        let structure_future = async {
            let mut structure_results = futures_util::stream::iter(0..pages.len()).map(|page_index| {
                let page = &pages[page_index];
                let is_non_question_by_text = page_index < scan.page_reliability.len()
                    && scan.page_reliability[page_index]
                        == doc_map::PageReliability::NonQuestion;
                let is_text_only = matches!(page.kind, PageInputKind::TextOnly);
                let semaphore = Arc::clone(&request_semaphore);
                let system_structure = system_structure.clone();
                async move {
                    if is_non_question_by_text {
                        return (
                            page_index,
                            Ok(r#"{"question_numbers_visible":[],"page_role":"BLANK"}"#.to_string()),
                        );
                    }
                    let mut images = Vec::new();
                    if let PageInputKind::Image { b64, .. } = &page.kind {
                        images.push(b64.clone());
                    }
                    let (img_slice, text_opt): (&[String], Option<&str>) = if is_text_only {
                        (&[], Some(page.text.as_str()))
                    } else {
                        (&images, None)
                    };
                    let body = llm::chat_body(
                        &config.model,
                        &system_structure,
                        img_slice,
                        text_opt,
                        750,
                    );
                    let result = match chat_with_permit(client, &body, &semaphore).await {
                        Ok(resp) => llm::message_content(&resp)
                            .map_err(|e| format!("bad response shape ({})", e)),
                        Err(e) => Err(format!("API failure ({})", e)),
                    };
                    (page_index, result)
                }
            })
            .buffer_unordered(config.parallelism.max(1));
            let mut ordered = Vec::with_capacity(pages.len());
            while let Some(result) = structure_results.next().await {
                ordered.push(result);
            }
            ordered.sort_by_key(|(index, _)| *index);
            ordered
        };

        // Build the text-layer-only map concurrently. This is fast (ms)
        // but starts the setup work while API calls are in flight.
        let map_setup_future = async {
            let page_texts_setup: Vec<String> = pages.iter().map(|p| p.text.clone()).collect();
            doc_map::build_hybrid_map(&page_texts_setup, &[], pages.len())
        };

        // Both futures run concurrently on the same task. The structure
        // pass uses semaphore permits for API calls; map_setup uses no
        // permits. When the text layer is sufficient, the structure pass
        // still runs but its results are simply unused — no correctness
        // impact, and the parallel work is "free" since permits were idle.
        let (ordered, _) = tokio::join!(structure_future, map_setup_future);

        structure_timing_ms = structure_start.elapsed().as_millis() as u64;
        for (i, res) in ordered {
            match res {
                    Ok(content) => match parse_llm_json::<PageStructureProposal>(&content) {
                        ParseOutcome::Clean(p) | ParseOutcome::Salvaged { value: p, .. } => {
                            let (v, violations) =
                                doc_map::validate_structure_proposal(i, p, pages.len());
                            report.anomalies.extend(violations);
                            structures.push(v);
                        }
                        ParseOutcome::Malformed { error } => {
                            report.anomalies.push(format!(
                            "structure pass page {}: invalid JSON ({}), page treated as unknown role",
                            i + 1,
                            error
                        ));
                            structures.push(unknown_role(i));
                        }
                    },
                    Err(e) => {
                        report.anomalies.push(format!(
                            "structure pass page {}: {}, page treated as unknown role",
                            i + 1,
                            e
                        ));
                        structures.push(unknown_role(i));
                    }
            }
            }

        // Page-role bookkeeping (records every skip — nothing disappears quietly).
        for s in &structures {
            if !s.role.is_question_content() {
                report.skipped_pages.push(SkippedPage {
                    page: s.page + 1,
                    role: format!("{:?}", s.role),
                });
            }
        }
    } else {
        progress.stage("Text layer map is complete — skipping vision structure scan.");
    }
    report.record_timing("structure", "api_call_stream", None, None, structure_timing_ms);

    // Ensure structures contains an entry for every page even if vision structure pass was skipped
    if structures.len() < pages.len() {
        for i in structures.len()..pages.len() {
            let role = match scan.page_reliability.get(i) {
                Some(doc_map::PageReliability::NonQuestion) => doc_map::PageRole::Blank,
                _ => doc_map::PageRole::Question,
            };
            structures.push(ValidatedPageStructure {
                page: i,
                questions: Vec::new(),
                question_y: Vec::new(),
                footer: None,
                footer_y: None,
                role,
            });
        }
    }

    // ── 2. Document map ─────────────────────────────────────────────────────
    let page_texts: Vec<String> = pages.iter().map(|p| p.text.clone()).collect();
    let doc_map_start = Instant::now();

    // Use hybrid map building: reliable text pages + vision for ambiguous pages
    let mut map = doc_map::build_hybrid_map(&page_texts, &structures, pages.len());

    // Record which pages used vision fallback
    report.timings.push(TimingEntry {
        stage: "document_map".to_string(),
        operation: "build_hybrid_map".to_string(),
        page: None,
        question_number: None,
        milliseconds: doc_map_start.elapsed().as_millis() as u64,
    });

    // Report vision fallback pages
    if !map.vision_fallback_pages.is_empty() {
        report.anomalies.push(format!(
            "vision structure fallback used for {} pages: {:?}",
            map.vision_fallback_pages.len(),
            map.vision_fallback_pages
                .iter()
                .map(|p| p + 1)
                .collect::<Vec<_>>()
        ));
    }

    // Backfill footers from structure pass
    if !map.spans.is_empty() {
        for s in &structures {
            if let Some((q, m)) = s.footer {
                if let Some(span) = map.spans.iter_mut().find(|sp| sp.number == q) {
                    if span.expected_marks.is_none() {
                        span.expected_marks = Some(m);
                    }
                }
            }
        }
    }

    report.paper_total_marks = map.paper_total_marks;
    report.anomalies.extend(map.anomalies.clone());

    // ── 3. Span extraction ──────────────────────────────────────────────────
    let mut built: Vec<BuiltQuestion> = Vec::new();

    // Phase 1b: if the hybrid map came back empty but the structure pass
    // returned enough structure to build a pure-vision map, do that BEFORE
    // dropping into per-page fallback. build_map_from_structure enforces
    // monotonicity and uses the VisionBounds / y-clip info we already paid
    // for, so extraction runs with Phase 1's y-band safety net instead of
    // blindly welding pages. Only when the structure pass also failed do
    // we fall back to per-page extraction.
    if map.spans.is_empty() {
        let structure_qs: usize = structures.iter().map(|s| s.questions.len()).sum();
        if structure_qs >= 2 {
            if let Some(structure_map) =
                doc_map::build_map_from_structure(&structures, pages.len())
            {
                report.anomalies.push(
                    "text-layer map empty; built map from vision structure pass instead of per-page fallback".to_string(),
                );
                map = structure_map;
            }
        }
    }

    if map.spans.is_empty() {
        // No reliable map → per-page legacy mode with all validators still
        // on (numbers proposed by AI, but forced plausible + monotonic).
        // Pages run in PARALLEL batches; the question-order invariant is
        // re-checked sequentially during assembly, and any out-of-order
        // proposal is re-extracted alone with the true bound.
        let q_pages: Vec<usize> = (0..pages.len())
            .filter(|&i| {
                structures
                    .get(i)
                    .map(|s| s.role.is_question_content())
                    .unwrap_or(true)
            })
            .collect();
        let mut next_allowed: u32 = 1;
        cancelled(cancel)?;
        progress.stage(&format!("Extracting {} pages…", q_pages.len()));
        let extract_start = Instant::now();
        let batch_next_allowed = next_allowed;
        let mut results = futures_util::stream::iter(q_pages.iter().copied().enumerate().map(|(position, i)| {
            extract_fallback_page(
                client,
                config,
                &pages[i],
                i,
                batch_next_allowed,
                &page_render_cache,
                &request_semaphore,
            ).map(move |result| (position, result))
        }))
        .buffer_unordered(config.parallelism.max(1));
        let mut ordered_results = Vec::with_capacity(q_pages.len());
        while let Some(result) = results.next().await {
            ordered_results.push(result);
        }
        drop(results);
        ordered_results.sort_by_key(|(position, _)| *position);
        report.record_timing(
            "extraction",
            "fallback_stream",
            None,
            None,
            extract_start.elapsed().as_millis() as u64,
        );
        for (i, (_, (mut outcome, local))) in q_pages.iter().copied().zip(ordered_results) {
                report.absorb(local);
                report.pages_processed += 1;
                // Sequential assembly enforces monotonic numbering: a page
                // that came back backwards under the shared batch bound is
                // re-asked alone with the true bound.
                if let Some(questions) = &outcome {
                    if let Some(first_q) = questions.first() {
                        if first_q.question_number + 1 < next_allowed {
                                let (redo, redo_local) =
                                    extract_fallback_page(
                                    client,
                                    config,
                                    &pages[i],
                                    i,
                                    next_allowed,
                                        &page_render_cache,
                                        &request_semaphore,
                                    )
                                .await;
                            report.absorb(redo_local);
                            outcome = redo;
                        }
                    }
                }
                match outcome {
                    Some(questions) => {
                        // Phase 2: process EVERY question extracted from this page.
                        // Dense MCQ pages can return 4+ questions — each gets
                        // stitched or pushed independently.
                        for q in questions {
                            let (qnum, _should_stitch, q_for_push) =
                                if let Some(prev) = built.last_mut() {
                                    if prev.question_number == q.question_number {
                                        if looks_like_new_question(&prev.content, &q.content) {
                                            let mut new_q = q.clone();
                                            new_q.question_number = prev.question_number + 1;
                                            new_q.needs_review = true;
                                            new_q.notes.push(
                                                "fallback: same-number page looked like a new question — number bumped; verify".to_string()
                                            );
                                            (new_q.question_number, false, new_q)
                                        } else {
                                            // Genuine continuation — weld content.
                                            prev.content = format!("{}\n\n{}", prev.content, q.content);
                                            prev.marks = validate::sum_inline_marks(&prev.content)
                                                .max(prev.marks.max(0) as u32)
                                                as i32;
                                            continue;
                                        }
                                    } else {
                                        (q.question_number, false, q)
                                    }
                                } else {
                                    (q.question_number, false, q)
                                };
                            next_allowed = qnum + 1;
                            built.push(q_for_push);
                        }
                    }
                    None => {
                        report.quarantined.push(QuarantineEvent {
                            scope: "question-page".to_string(),
                            page: Some(i + 1),
                            question_number: None,
                            reason: "page failed validation and repair attempts".to_string(),
                        });
                    }
                }
        }
    } else {
        report.questions_expected = map.spans.len();
        let total = map.spans.len();
        // Pre-resolve span pages; spans with nothing extractable quarantine
        // without ever reaching the model.
        let mut jobs: Vec<(usize, &QuestionSpan, Vec<(usize, &PageInput)>)> = Vec::new();
        for (span_idx, span) in map.spans.iter().enumerate() {
            let span_pages: Vec<(usize, &PageInput)> = (span.start_page..=span.end_page)
                .filter(|&pi| pi < pages.len())
                .filter(|&pi| {
                    map.non_question_pages.is_empty()
                        || !map.non_question_pages.contains(&pi)
                        || structures
                            .get(pi)
                            .map(|s| s.role == doc_map::PageRole::Blank)
                            .unwrap_or(false)
                })
                .map(|pi| (pi, &pages[pi]))
                .collect();
            if span_pages.is_empty() {
                report.quarantined.push(QuarantineEvent {
                    scope: "question".to_string(),
                    page: None,
                    question_number: Some(span.number),
                    reason: "span contained no extractable pages".to_string(),
                });
                continue;
            }
            jobs.push((span_idx, span, span_pages));
        }

        cancelled(cancel)?;
        progress.stage(&format!("Extracting {} questions…", total));
        let extract_start = Instant::now();
        let job_count = jobs.len();
        let mut results = futures_util::stream::iter(0..job_count).map(|position| {
            let job = &jobs[position];
            extract_span(
                client,
                config,
                job.1,
                &job.2,
                &page_render_cache,
                &request_semaphore,
            )
            .map(move |result| (position, result))
        })
        .buffer_unordered(config.parallelism.max(1));
        let mut ordered_results = Vec::with_capacity(jobs.len());
        while let Some(result) = results.next().await {
            ordered_results.push(result);
        }
        ordered_results.sort_by_key(|(position, _)| *position);
        report.record_timing(
            "extraction",
            "span_stream",
            None,
            None,
            extract_start.elapsed().as_millis() as u64,
        );
        for (job, (_, (opt, local))) in jobs.iter().zip(ordered_results) {
            let span: &QuestionSpan = job.1;
            let sp = &job.2;
            report.absorb(local);
            match opt {
                Some(q) => {
                    report.pages_processed += sp.len();
                    push_mark_check(span, &q, &mut report);
                    built.push(q);
                }
                None => {
                    let mut reason = "failed validation and all repair attempts".to_string();
                    if let Some(err) = report.anomalies.last() {
                        if err.starts_with("quarantined: ") {
                            reason = format!(
                                "failed validation and all repair attempts (last error: {})",
                                err.trim_start_matches("quarantined: ")
                            );
                        }
                    }
                    report.quarantined.push(QuarantineEvent {
                        scope: "question".to_string(),
                        page: Some(span.start_page + 1),
                        question_number: Some(span.number),
                        reason,
                    });
                }
            }
        }
    }

    report.questions_extracted = built.len();
    report.extracted_total_marks = built.iter().map(|q| q.marks.max(0) as u32).sum();
    report.marks_checksum_ok = match (report.paper_total_marks, map.spans.is_empty()) {
        (Some(total), false) => Some(report.extracted_total_marks == total),
        _ => None,
    };

    Ok((built, report))
}

/// Marks checksum for one span → report.
fn push_mark_check(span: &QuestionSpan, q: &BuiltQuestion, report: &mut ImportReport) {
    if let Some(expected) = span.expected_marks {
        report.mark_checks.push(MarkCheck {
            question_number: span.number,
            expected: Some(expected),
            actual: q.marks.max(0) as u32,
            ok: q.marks.max(0) as u32 == expected,
            needs_review: q.needs_review,
        });
    }
}

/// Repair-loop core: repeatedly ask → parse → validate; quote failures back.
/// Returns (Some(question), report) on acceptance (possibly flagged),
/// (None, report) on quarantine — the LOCAL report is absorbed by the caller
/// (this runs inside a parallel batch).
async fn extract_span<C: LlmClient>(
    client: &C,
    config: &PipelineConfig,
    span: &QuestionSpan,
    span_pages: &[(usize, &PageInput)],
    page_render_cache: &Arc<crate::pdf_render::PageRenderCache>,
    request_semaphore: &Arc<Semaphore>,
) -> (Option<BuiltQuestion>, ImportReport) {
    // Own, local report: spans now run in parallel batches, so each unit
    // accumulates its own bookkeeping and the caller absorbs it in order.
    let mut report = ImportReport::default();
    let max_attempts = 1 + config.max_repairs;

    // Chunk long spans: at most 4 page images per call (your no-batching
    // constraint honored as per-chunk calls, Rust concatenates).
    const MAX_IMAGES: usize = 4;
    let mut chunks: VecDeque<Vec<(usize, &PageInput)>> = span_pages
        .chunks(MAX_IMAGES)
        .map(|chunk| chunk.to_vec())
        .collect();
    let mut split_mode = span_pages.len() > MAX_IMAGES;
    let mut split_raw_items: Vec<AiQuestion> = Vec::new();
    let mut split_decoded_pages: Vec<Option<Arc<image::DynamicImage>>> = vec![None; span_pages.len()];
    let mut split_local_to_chunk: Vec<usize> = Vec::new();
    let mut split_crop_offsets: Vec<(f32, f32)> = vec![(0.0, 1.0); span_pages.len()];
    let mut split_page_bands: Vec<Option<(f32, f32)>> = vec![None; span_pages.len()];
    let mut split_context = String::new();
    let mut split_image_count = 0usize;
    let mut unified_split = false;

    let mut contents: Vec<String> = Vec::new();
    let mut topics_acc: Vec<String> = Vec::new();
    let mut is_code_acc = false;
    let mut needs_review = false;
    let mut notes: Vec<String> = Vec::new();
    let mut ai_marks: Option<i32> = None;
    // Diagrams already persisted for this question: (signature, link) pairs
    // for near-duplicate reuse across chunk boundaries.
    let mut saved_diagrams: Vec<([u8; 64], String)> = Vec::new();

    'chunks: while let Some(mut chunk) = chunks.pop_front() {
        // Phase 0: filter out sentinel b64 values before they reach the
        // model. We build THREE parallel structures here:
        //   * `images`  — Vec<String> sent to the API (no sentinels)
        //   * `local_to_chunk` — maps image-index-as-seen-by-model → index
        //     into `chunk` (so bbox_page_indexes returned by the model can
        //     be resolved back to the correct PageInput for audit/save).
        //   * `page_bands` — parallel to `chunk`: Option<(low_y, high_y)>
        //     giving the vertical band of THIS span on each chunk page
        //     (None = full page). Used by audit_diagram_boxes to reject
        //     bboxes whose center-y falls outside the question's band —
        //     the deterministic safety net for the prompt-level band hints.
        let mut preparation_inputs = Vec::with_capacity(chunk.len());
        for (local_idx, (global_pi, _p)) in chunk.iter().enumerate() {
            let is_first_page_of_span = *global_pi == span.start_page;
            let is_last_page_of_span = *global_pi == span.end_page;
            let (s, e) = if is_first_page_of_span && is_last_page_of_span {
                (span.start_y_frac, span.end_y_frac)
            } else if is_first_page_of_span {
                (span.start_y_frac, None)
            } else if is_last_page_of_span {
                (None, span.end_y_frac)
            } else {
                (None, None)
            };
            if let Some(b64) = _p.get_b64() {
                preparation_inputs.push(ChunkImageInput {
                    chunk_idx: local_idx,
                    b64: b64.clone(),
                    start_y: s,
                    end_y: e,
                });
            }
        }
        let prepared = match prepare_chunk_images(chunk.len(), preparation_inputs).await {
            Ok(prepared) => prepared,
            Err(error) => {
                report.anomalies.push(format!(
                    "Question {} image preparation task failed: {}",
                    span.number, error
                ));
                return (None, report);
            }
        };
        let mut images = prepared.images;
        let mut local_to_chunk = prepared.local_to_chunk;
        let mut page_bands = prepared.page_bands;
        let mut page_crop_offsets = prepared.page_crop_offsets;
        let mut decoded_pages = prepared.decoded_pages;
        let mut raw_text: String = chunk
            .iter()
            .map(|(pi, p)| {
                if p.text.trim().is_empty() {
                    String::new()
                } else {
                    format!("RAW TEXT PAGE {}:\n{}\n\n", pi + 1, p.text)
                }
            })
            .collect();

        // Phase 1: vertical-band notes for multi-question pages. For each
        // page in this chunk, if the span's y clips apply on that page
        // (first page of the span gets start_y_frac; last page of the span
        // gets end_y_frac) emit a concrete "read between X% and Y%" hint.
        // Pages fully interior to the span get no hint (full page).
        let mut band_notes = String::new();
        for (model_idx, &chunk_idx) in local_to_chunk.iter().enumerate() {
            let (global_pi, _p) = chunk[chunk_idx];
            let is_first_page_of_span = global_pi == span.start_page;
            let is_last_page_of_span = global_pi == span.end_page;
            let (s, e) = if is_first_page_of_span && is_last_page_of_span {
                (span.start_y_frac, span.end_y_frac)
            } else if is_first_page_of_span {
                (span.start_y_frac, None)
            } else if is_last_page_of_span {
                (None, span.end_y_frac)
            } else {
                (None, None)
            };
            if s.is_some() || e.is_some() {
                use std::fmt::Write;
                let _ = write!(
                    &mut band_notes,
                    "\n\nPage {} of the attached images (original page {}): ",
                    model_idx + 1,
                    global_pi + 1
                );
                match (s, e) {
                    (Some(a), Some(b)) => {
                        let _ = write!(
                            &mut band_notes,
                            "Question {} begins at about {:.0}% down and ends at about {:.0}% down. Transcribe ONLY between those lines — content above or below belongs to a DIFFERENT main question and must not appear in your output.",
                            span.number, a * 100.0, b * 100.0,
                        );
                    }
                    (Some(a), None) => {
                        let _ = write!(
                            &mut band_notes,
                            "Question {} begins at about {:.0}% down the page. Transcribe from there to the bottom (it continues onto the next page).",
                            span.number, a * 100.0,
                        );
                    }
                    (None, Some(b)) => {
                        let _ = write!(
                            &mut band_notes,
                            "Question {} continues from the previous page and ends at about {:.0}% down this page. Do NOT transcribe anything below that line (it is the next question).",
                            span.number, b * 100.0,
                        );
                    }
                    (None, None) => {}
                }
            }
        }

        let system = extraction_system_prompt(config, span);
        let mut last_error = String::new();
        let mut accepted: Option<(Vec<AiQuestion>, bool)> = None; // (items, salvaged_truncated)

        for attempt in 1..=max_attempts {
            let repair_note = if attempt == 1 {
                String::new()
            } else {
                format!(
                    "\n\nPREVIOUS ATTEMPT FAILED VALIDATION: {}. Regenerate the COMPLETE corrected JSON for Question {}.",
                    last_error, span.number
                )
            };
            let user_text = format!(
                "Transcribe Question {} from the attached page image(s).{}{}{}{}",
                span.number,
                band_notes,
                if raw_text.is_empty() {
                    String::new()
                } else {
                    format!(
                        "\n\nReference OCR text (may be corrupt — images are authoritative):\n{}",
                        &raw_text
                    )
                },
                if split_context.is_empty() {
                    String::new()
                } else {
                    format!(
                        "\n\nThis is a continuation call. The preceding chunk already yielded the following beginning of Question {}. Continue it from the newly attached pages; do not repeat this text and do not return an empty items array merely because the page begins mid-question:\n{}",
                        span.number, split_context
                    )
                },
                repair_note
            );
            let body = llm::chat_body(
                &config.model,
                &system,
                &images,
                Some(&user_text),
                config.max_output_tokens,
            );

            let api_start = Instant::now();
            let resp = match chat_with_permit(client, &body, request_semaphore).await {
                Ok(r) => r,
                Err(e) => {
                    last_error = e.to_string();
                    if attempt == max_attempts {
                        break;
                    }
                    continue;
                }
            };
            report.record_timing(
                "extraction",
                "api_call",
                Some(span_pages[0].0 + 1),
                Some(span.number),
                api_start.elapsed().as_millis() as u64,
            );
            let mut content = match llm::message_content(&resp) {
                Ok(c) => c,
                Err(e) => {
                    last_error = e.to_string();
                    continue;
                }
            };

            let mut parsed = parse_llm_json::<AiQuestionPage>(&content);

            if let ParseOutcome::Malformed { ref error } = parsed {
                eprintln!(
                    "[DIAGNOSTIC][RAW_JSON_ERROR] question={} attempt={} split_mode={} pages={}..{} error={} raw_response:\n{}",
                    span.number,
                    attempt,
                    split_mode,
                    chunk.first().map(|(page, _)| page + 1).unwrap_or(0),
                    chunk.last().map(|(page, _)| page + 1).unwrap_or(0),
                    error,
                    content
                );
            }

            // Phase 4 fix: if we get an EOF error on a large span (3+ pages),
            // the provider might be struggling with the payload size.
            // Retry with fewer images to reduce load.
            if let ParseOutcome::Malformed { ref error } = parsed {
                if error.contains("EOF") && images.len() >= 3 && attempt == 1 {
                    eprintln!(
                        "WARNING: Question {} got EOF error with {} pages, retrying with first 2 pages only",
                        span.number, images.len()
                    );
                    split_mode = true;
                    // Keep the first two pages as this chunk and enqueue the
                    // remainder. The reduced response must not silently
                    // discard the pages that caused the original payload to
                    // overflow.
                    let remainder = chunk.split_off(2);
                    chunks.push_front(remainder);
                    let reduced_image_count = local_to_chunk
                        .iter()
                        .take_while(|&&chunk_idx| chunk_idx < 2)
                        .count();
                    let reduced_images = images[..reduced_image_count].to_vec();
                    let reduced_body = llm::chat_body(
                        &config.model,
                        &system,
                        &reduced_images,
                        Some(&format!(
                            "{}\n\nNOTE: This is a retry with fewer pages due to payload size issues. Transcribe Question {} from these pages only.",
                            user_text, span.number
                        )),
                        config.max_output_tokens,
                    );

                    let api_start = Instant::now();
                    let reduced_resp = match chat_with_permit(client, &reduced_body, request_semaphore).await {
                        Ok(r) => r,
                        Err(e) => {
                            last_error = e.to_string();
                            report.repairs += 1;
                            continue;
                        }
                    };
                    report.record_timing(
                        "extraction",
                        "api_call_reduced",
                        Some(span_pages[0].0 + 1),
                        Some(span.number),
                        api_start.elapsed().as_millis() as u64,
                    );
                    content = match llm::message_content(&reduced_resp) {
                        Ok(c) => c,
                        Err(e) => {
                            last_error = e.to_string();
                            report.repairs += 1;
                            continue;
                        }
                    };
                    parsed = parse_llm_json::<AiQuestionPage>(&content);

                    // The response and all page-index maps now describe only
                    // the first split chunk. The queued remainder gets a
                    // fresh preparation/audit context on the next iteration.
                    images.truncate(reduced_image_count);
                    local_to_chunk.truncate(reduced_image_count);
                    page_crop_offsets.truncate(reduced_image_count);
                    page_bands.truncate(2);
                    decoded_pages.truncate(2);
                    raw_text = chunk
                        .iter()
                        .map(|(pi, p)| {
                            if p.text.trim().is_empty() {
                                String::new()
                            } else {
                                format!("RAW TEXT PAGE {}:\n{}\n\n", pi + 1, p.text)
                            }
                        })
                        .collect();
                    band_notes.truncate(0);
                    for (model_idx, &chunk_idx) in local_to_chunk.iter().enumerate() {
                        let (global_pi, _p) = chunk[chunk_idx];
                        let is_first_page_of_span = global_pi == span.start_page;
                        let is_last_page_of_span = global_pi == span.end_page;
                        let (s, e) = if is_first_page_of_span && is_last_page_of_span {
                            (span.start_y_frac, span.end_y_frac)
                        } else if is_first_page_of_span {
                            (span.start_y_frac, None)
                        } else if is_last_page_of_span {
                            (None, span.end_y_frac)
                        } else {
                            (None, None)
                        };
                        if let (Some(a), Some(b)) = (s, e) {
                            use std::fmt::Write;
                            let _ = write!(
                                &mut band_notes,
                                "\n\nPage {} of the attached images (original page {}): Question {} begins at about {:.0}% down and ends at about {:.0}% down. Transcribe ONLY between those lines.",
                                model_idx + 1,
                                global_pi + 1,
                                span.number,
                                a * 100.0,
                                b * 100.0,
                            );
                        } else if let Some(a) = s {
                            use std::fmt::Write;
                            let _ = write!(
                                &mut band_notes,
                                "\n\nPage {} of the attached images (original page {}): Question {} begins at about {:.0}% down the page. Transcribe from there to the bottom.",
                                model_idx + 1,
                                global_pi + 1,
                                span.number,
                                a * 100.0,
                            );
                        } else if let Some(b) = e {
                            use std::fmt::Write;
                            let _ = write!(
                                &mut band_notes,
                                "\n\nPage {} of the attached images (original page {}): Question {} ends at about {:.0}% down this page. Do NOT transcribe anything below that line.",
                                model_idx + 1,
                                global_pi + 1,
                                span.number,
                                b * 100.0,
                            );
                        }
                    }
                }
            }

            let (mut page_items, salvaged) = match parsed {
                ParseOutcome::Clean(v) => (v, false),
                ParseOutcome::Salvaged {
                    value,
                    dropped_tail,
                } => {
                    eprintln!(
                        "[DIAGNOSTIC][JSON_SALVAGED] question={} attempt={} split_mode={} dropped_tail={} pages={}..{} raw_response:\n{}",
                        span.number,
                        attempt,
                        split_mode,
                        dropped_tail,
                        chunk.first().map(|(page, _)| page + 1).unwrap_or(0),
                        chunk.last().map(|(page, _)| page + 1).unwrap_or(0),
                        content
                    );
                    report.salvage_events += 1;
                    if dropped_tail {
                        last_error = "response was truncated; items may be missing".to_string();
                        if attempt < max_attempts {
                            continue; // ask for the full answer again
                        }
                    }
                    (value, dropped_tail)
                }
                ParseOutcome::Malformed { error } => {
                    last_error = format!("invalid JSON: {}", error);
                    report.repairs += 1;
                    continue;
                }
            };

            // Split calls are intentionally raw collection passes. A page
            // fragment cannot satisfy whole-span validation on its own, and
            // diagram indices are only meaningful after all chunks are
            // merged. Remap each model-local image index into one unified
            // image-index space, then defer every strict gate below.
            if split_mode {
                let raw_text_len: usize = page_items
                    .items
                    .iter()
                    .filter_map(|item| item.content.as_deref())
                    .map(str::chars)
                    .map(Iterator::count)
                    .sum();
                let raw_latex_len: usize = page_items
                    .items
                    .iter()
                    .filter_map(|item| item.math_snippet.as_deref())
                    .map(str::chars)
                    .map(Iterator::count)
                    .sum();
                let raw_bbox_len: usize = page_items
                    .items
                    .iter()
                    .filter_map(|item| item.diagram_bboxes.as_ref())
                    .map(Vec::len)
                    .sum();
                eprintln!(
                    "[DIAGNOSTIC][RAW_CHUNK] question={} pages={}..{} items={} text_chars={} latex_chars={} bbox_count={} salvaged={}",
                    span.number,
                    chunk.first().map(|(page, _)| page + 1).unwrap_or(0),
                    chunk.last().map(|(page, _)| page + 1).unwrap_or(0),
                    page_items.items.len(),
                    raw_text_len,
                    raw_latex_len,
                    raw_bbox_len,
                    salvaged
                );
                let image_offset = split_image_count;
                for (model_idx, &chunk_idx) in local_to_chunk.iter().enumerate() {
                    if let Some((global_page, _)) = chunk.get(chunk_idx) {
                        if let Some(span_idx) = span_pages
                            .iter()
                            .position(|(page, _)| page == global_page)
                        {
                            split_local_to_chunk.push(span_idx);
                            if let Some(decoded) = decoded_pages.get(chunk_idx).cloned().flatten() {
                                split_decoded_pages[span_idx] = Some(decoded);
                            }
                            if let Some(offset) = page_crop_offsets.get(chunk_idx) {
                                split_crop_offsets[span_idx] = *offset;
                            }
                            if let Some(band) = page_bands.get(chunk_idx) {
                                split_page_bands[span_idx] = *band;
                            }
                        }
                    }
                    let _ = model_idx;
                }
                split_image_count += local_to_chunk.len();
                for item in &mut page_items.items {
                    if let Some(indexes) = &mut item.bbox_page_indexes {
                        for index in indexes {
                            if let Some(local) = value_to_usize(index) {
                                *index = serde_json::json!(image_offset + local);
                            }
                        }
                    }
                }
                split_raw_items.extend(page_items.items);
                split_context = split_raw_items
                    .iter()
                    .filter_map(|item| item.content.as_deref())
                    .filter(|content| !content.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                if salvaged {
                    needs_review = true;
                    notes.push(
                        "response truncated; content recovered up to the last complete item"
                            .to_string(),
                    );
                }
                if !chunks.is_empty() {
                    continue 'chunks;
                }
                // The final split response is now available. Replace the
                // fragment context with span-global mappings and let the
                // existing strict validation/audit path run exactly once.
                split_mode = false;
                unified_split = true;
                chunk = span_pages.to_vec();
                local_to_chunk = split_local_to_chunk.clone();
                page_bands = split_page_bands.clone();
                page_crop_offsets = split_crop_offsets.clone();
                decoded_pages = split_decoded_pages.clone();
                page_items.items = vec![merge_split_questions(
                    std::mem::take(&mut split_raw_items),
                    span.number,
                )];
                let unified = &page_items.items[0];
                eprintln!(
                    "[DIAGNOSTIC][UNIFIED_OBJECT] question={} structure={:#?}",
                    span.number, unified
                );
            }

            for item in &mut page_items.items {
                normalize_composite_visual_options(item);
            }

            if page_items.items.is_empty() && contents.is_empty() {
                eprintln!(
                    "[DIAGNOSTIC][VALIDATION_ERROR] question={} rule=non_empty_items items=0 prior_contents={}",
                    span.number,
                    contents.len()
                );
                eprintln!("WARNING: Question {} extraction returned an empty items array.", span.number);
                report.quarantined.push(QuarantineEvent {
                    scope: "question".to_string(),
                    page: Some(span.start_page + 1),
                    question_number: Some(span.number),
                    reason: "No content extracted for this span".to_string(),
                });
                return (None, report);
            }

            // AUDITABLE RETENTION: collect collateral numbers, quote them in repair,
            // and enforce exactly ONE item per span. Multi-item responses are a
            // repair trigger, not a silent drop.
            let mut raw_numbers: Vec<u32> = Vec::new();
            for item in &page_items.items {
                if let Some(v) = &item.question_number {
                    if let Some(n) = crate::validate::value_to_question_number(v) {
                        raw_numbers.push(n);
                    }
                }
            }

            // Aggregate violation: more than one item = repair trigger.
            if page_items.items.len() > 1 {
                eprintln!(
                    "[DIAGNOSTIC][VALIDATION_ERROR] question={} rule=single_item actual_items={}",
                    span.number,
                    page_items.items.len()
                );
                let _collateral_numbers: Vec<String> = raw_numbers
                    .iter()
                    .filter(|&&n| n != span.number)
                    .map(|n| n.to_string())
                    .collect();
                if !page_items.items.iter().any(|i| {
                    i.question_number.as_ref()
                        .and_then(crate::validate::value_to_question_number) == Some(span.number)
                }) {
                    let extracted: Vec<u32> = page_items.items.iter()
                        .filter_map(|i| i.question_number.as_ref().and_then(crate::validate::value_to_question_number))
                        .collect();
                    last_error = format!(
                        "You extracted data for questions {:?}, but NONE of it was Question {}. Please extract ONLY Question {}.",
                        extracted, span.number, span.number
                    );
                    report.repairs += 1;
                }
            }

            // Filter to target question only, but quote what was dropped.
            let dropped_numbers: Vec<String> = raw_numbers
                .iter()
                .filter(|&&n| n != span.number)
                .map(|n| n.to_string())
                .collect();
            let original_items = page_items.items.clone();
            let initial_len = page_items.items.len();
            page_items.items.retain(|item| {
                item.question_number.as_ref()
                    .and_then(crate::validate::value_to_question_number) == Some(span.number)
            });

            if page_items.items.is_empty() {
                eprintln!(
                    "[DIAGNOSTIC][VALIDATION_ERROR] question={} rule=target_question_present dropped_numbers={:?}",
                    span.number,
                    dropped_numbers
                );
                // LLM hallucinated entirely wrong question number.
                // Instead of continuing and triggering a repair (which it
                // usually ignores and just hallucinates again), we try to
                // salvage it by ASSUMING it's the right question if it's
                // the only thing on the page.
                if initial_len == 1 && attempt == 0 {
                    report.salvage_events += 1;
                    page_items.items = original_items; // restore
                    page_items.items[0].question_number = Some(serde_json::json!(span.number)); // force it
                } else {
                    continue;
                }
            }

            // If collateral was found and dropped, include the dropped numbers
            // in the repair note so the model learns the boundary error.
            if !dropped_numbers.is_empty() {
                last_error = format!(
                    "{} (dropped collateral questions: [{}])",
                    last_error,
                    dropped_numbers.join(", ")
                );
            }

            // After filtering, enforce single-item output. If multiple items
            // matched the target (e.g., LLM split Q8 into sub-parts), we must
            // tell it to combine them into ONE single item.
            if page_items.items.len() > 1 {
                // More than one item with the target number: check if second item
                // looks like a genuine continuation (same number, advancing sub-parts)
                // or a split/collateral error.
                if page_items.items.len() == 2 {
                    let first_content = page_items.items[0].content.as_deref().unwrap_or("");
                    let second_content = page_items.items[1].content.as_deref().unwrap_or("");
                    if looks_like_new_question(first_content, second_content) {
                        // Second item is a new question misnumbered as the target.
                        last_error = format!(
                            "You returned 2 items for Question {}. The second item (starting with \"{}\") looks like a DIFFERENT question — delete it.",
                            span.number,
                            second_content.chars().take(40).collect::<String>()
                        );
                        report.repairs += 1;
                        continue;
                    } else {
                        // Genuine continuation — keep only the first item; discard
                        // the redundant second item (continuation should extend span,
                        // not split the item array).
                        page_items.items.truncate(1);
                    }
                } else {
                    last_error = format!(
                        "You returned {} items for Question {}. You MUST combine all sub-parts into a SINGLE item's `content` string, separated by double newlines.",
                        page_items.items.len(), span.number
                    );
                    report.repairs += 1;
                    continue;
                }
            }

            // If retention emptied the array, repair with enhanced message.
            if page_items.items.is_empty() {
                last_error = if dropped_numbers.is_empty() {
                    format!("No content matched Question {}. Please extract ONLY Question {}.", span.number, span.number)
                } else {
                    format!("You extracted data for questions [{}], but NONE of it was Question {}. Please extract ONLY Question {}.",
                        dropped_numbers.join(", "), span.number, span.number)
                };
                report.repairs += 1;
                continue;
            }

            // Un-shift diagram bounding boxes back to full-page coordinates
            for item in page_items.items.iter_mut() {
                if let (Some(bboxes), Some(indexes)) = (&mut item.diagram_bboxes, &item.bbox_page_indexes) {
                    for (i, bbox) in bboxes.iter_mut().enumerate() {
                        if bbox.len() != 4 {
                            continue;
                        }
                        if let Some(page_idx_val) = indexes.get(i) {
                            if let Some(page_idx) = page_idx_val.as_u64() {
                                if let Some(&(start_y, height)) = page_crop_offsets.get(page_idx as usize) {
                                    if height < 1.0 {
                                        bbox[1] = start_y + (bbox[1] * height);
                                        bbox[3] = start_y + (bbox[3] * height);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // ── Deterministic validation of the page items ────────────────
            let validation_errors = validate_span_items(&page_items, span);
            if !validation_errors.is_empty() {
                for error in &validation_errors {
                    eprintln!(
                        "[DIAGNOSTIC][SCHEMA_VALIDATION_ERROR] question={} rule={}",
                        span.number, error
                    );
                }
                last_error = validation_errors.join("; ");
                report.repairs += 1;
                continue;
            }

            // ── Figure-reference consistency: a referenced Figure must be
            // boxed (Figure 6 mashing into text was the regression) ────────
            let mut cons_errors: Vec<String> = Vec::new();
            for (ii, item) in page_items.items.iter().enumerate() {
                for e in validate::diagram_consistency_errors(
                    item.content.as_deref().unwrap_or(""),
                    item.diagram_bboxes.as_ref().map(|b| b.len()).unwrap_or(0),
                ) {
                    cons_errors.push(format!("item {}: {}", ii + 1, e));
                }
            }
            // A trace/answer-grid instruction overrides a Figure reference:
            // the referenced figure may be elsewhere, while this page's grid
            // must remain Markdown and must never trigger figure repairs.
            if cons_errors.len() > 0
                && page_items.items.iter().all(|item| {
                    validate::is_answer_grid_request(item.content.as_deref().unwrap_or(""))
                })
            {
                cons_errors.clear();
            }
            if !cons_errors.is_empty() {
                for error in &cons_errors {
                    eprintln!(
                        "[DIAGNOSTIC][FIGURE_CONSISTENCY_ERROR] question={} rule={}",
                        span.number, error
                    );
                }
                report.repairs += 1;
                if attempt < max_attempts {
                    last_error = cons_errors.join("; ");
                    continue;
                }
                report.anomalies.push(format!(
                    "Question {}: figure/diagram inconsistency kept after repair budget — {}",
                    span.number,
                    cons_errors.join("; ")
                ));
            }

            // ── Diagram boxes: Rust audits every crop the AI proposed ─────
            let audit_start = Instant::now();
            let audit_items = page_items.items;
            let audit_local_to_chunk = local_to_chunk.clone();
            let audit_page_bands = page_bands.clone();
            let audit_decoded_pages = decoded_pages.clone();
            let (audited_items, bad, box_issues) = match tokio::task::spawn_blocking(move || {
                let mut items = audit_items;
                let (bad, issues) = audit_diagram_boxes(
                    &audit_decoded_pages,
                    &mut items,
                    &audit_local_to_chunk,
                    &audit_page_bands,
                );
                (items, bad, issues)
            })
            .await
            {
                Ok(result) => result,
                Err(error) => {
                    last_error = format!("diagram audit task failed: {}", error);
                    report.repairs += 1;
                    continue;
                }
            };
            if !box_issues.is_empty() {
                for error in &box_issues {
                    eprintln!(
                        "[DIAGNOSTIC][DIAGRAM_AUDIT_ERROR] question={} rule={} bad_box_indices={:?}",
                        span.number, error, bad
                    );
                }
            }
            page_items.items = audited_items;
            report.record_timing(
                "diagram_processing",
                "crop_audit",
                Some(span_pages[0].0 + 1),
                Some(span.number),
                audit_start.elapsed().as_millis() as u64,
            );
            if !box_issues.is_empty() {
                let answer_grid_only = page_items.items.iter().all(|item| {
                    validate::is_answer_grid_request(item.content.as_deref().unwrap_or(""))
                }) && box_issues
                    .iter()
                    .all(|e| e.contains("EMPTY RULED ANSWER GRID"));
                if answer_grid_only {
                    let mut items = page_items.items;
                    prune_bad_diagram_boxes(&mut items, &bad, &mut report);
                    accepted = Some((items, salvaged));
                    break;
                }
                if unified_split {
                    eprintln!(
                        "[DIAGNOSTIC][DIAGRAM_AUDIT_TERMINAL] question={} unified split retained; pruning invalid boxes without re-requesting the stitched span",
                        span.number
                    );
                    let mut items = page_items.items;
                    report.anomalies.push(format!(
                        "Question {}: dropped {} invalid diagram box(es) from unified split after one final audit",
                        span.number,
                        bad.len()
                    ));
                    prune_bad_diagram_boxes(&mut items, &bad, &mut report);
                    accepted = Some((items, salvaged));
                    break;
                }
                last_error = box_issues.join("; ");
                report.repairs += 1;
                if attempt < max_attempts {
                    continue;
                }
                // Repair budget spent: keep the transcription, drop the bad
                // boxes — deterministically, and on the record.
                report.anomalies.push(format!(
                    "Question {}: dropped {} invalid diagram box(es) after repair budget spent — {}",
                    span.number,
                    bad.len(),
                    box_issues.join("; ")
                ));
                let mut items = page_items.items;
                prune_bad_diagram_boxes(&mut items, &bad, &mut report);
                accepted = Some((items, salvaged));
                break;
            }

            accepted = Some((page_items.items, salvaged));
            break;
        }

        let (items, salvaged) = match accepted {
            Some(v) => v,
            None => {
                eprintln!("WARNING: Question {} extraction failed: {}", span.number, last_error);
                report
                    .anomalies
                    .push(format!("quarantined: {}", last_error));
                return (None, report);
            }
        };

        for item in items {
            let mut item_content = item.content.unwrap_or_default();

            // Cropping: sanitizer + blank guard, fully deterministic.
            // IMPORTANT: bbox_page_indexes returned by the model refer to
            // the `images` vector we sent (sentinels filtered out). We
            // must translate through local_to_chunk to find the correct
            // PageInput inside `chunk` (which may contain sentinel pages
            // the model never saw).
            if let Some(bboxes) = &item.diagram_bboxes {
                let indexes = item.bbox_page_indexes.clone().unwrap_or_default();
                let diagram_save_start = Instant::now();
                let mut requests = Vec::with_capacity(bboxes.len());
                let mut page_b64 = std::collections::HashMap::new();
                for (bi, bbox) in bboxes.iter().enumerate() {
                    let model_idx = indexes
                        .get(bi)
                        .and_then(value_to_usize)
                        .filter(|&k| k < local_to_chunk.len())
                        .unwrap_or(0);
                    let chunk_idx = local_to_chunk[model_idx];
                    if chunk_idx >= chunk.len() {
                        report.crop_rejections += 1;
                        continue;
                    }
                    let global_page_idx = chunk[chunk_idx].0;
                    let page = chunk[chunk_idx].1;
                    let ignore_grid = validate::figure_references(&item_content) > 0 && !validate::is_answer_grid_request(&item_content);
                    if config.pdf_path.is_none() {
                        if let Some(b64) = page.get_b64() {
                            page_b64.entry(global_page_idx).or_insert_with(|| b64.clone());
                        }
                    }
                    requests.push(DiagramSaveRequest {
                        global_page_idx,
                        bbox: bbox.clone(),
                        ignore_grid,
                        graph_like: item
                            .diagram_kinds
                            .as_ref()
                            .and_then(|kinds| kinds.get(bi))
                            .map(|kind| {
                                let kind = kind.to_ascii_lowercase();
                                kind.contains("graph")
                                    || kind.contains("chart")
                                    || kind.contains("plot")
                                    || kind.contains("composite_visual_options")
                            })
                            .unwrap_or(false),
                    });
                }
                let saved_before = saved_diagrams.clone();
                match persist_diagrams(
                    requests,
                    page_b64,
                    config.clone(),
                    Arc::clone(page_render_cache),
                    std::mem::take(&mut saved_diagrams),
                )
                .await
                {
                    Ok(persisted) => {
                        saved_diagrams = persisted.saved;
                        report.absorb(persisted.report);
                        for link in persisted.links.into_iter().flatten() {
                        if item_content.contains("[DIAGRAM_PLACEHOLDER]") {
                            item_content = item_content.replacen("[DIAGRAM_PLACEHOLDER]", &link, 1);
                        } else {
                            item_content.push_str(&link);
                        }
                    }
                    }
                    Err(error) => {
                        saved_diagrams = saved_before;
                        report.anomalies.push(format!(
                            "Question {} diagram persistence task failed: {}",
                            span.number, error
                        ));
                    }
                }
                report.record_timing(
                    "diagram_processing",
                    "save_diagrams",
                    Some(span_pages[0].0 + 1),
                    Some(span.number),
                    diagram_save_start.elapsed().as_millis() as u64,
                );
            }
            item_content = item_content.replace("[DIAGRAM_PLACEHOLDER]", "");

            if let Some(t) = item.topics {
                for topic in value_to_topics(&t) {
                    if config.allowed_topics.is_empty() || config.allowed_topics.contains(&topic) {
                        topics_acc.push(topic);
                    }
                }
            }
            if item.is_code == Some(true) {
                is_code_acc = true;
            }
            if let Some(m) = item.marks.as_ref().and_then(validate::value_to_marks) {
                ai_marks = Some(ai_marks.map_or(m, |existing: i32| existing + m));
            }
            contents.push(item_content);
        }

        if salvaged {
            needs_review = true;
            notes.push(
                "response truncated; content recovered up to the last complete item".to_string(),
            );
        }
    }

    // ── Assemble + content-level validation ─────────────────────────────────
    // Each chunk is validated as exactly one target item before it reaches
    // this point. Join the validated continuations in page order; never keep
    // only the first chunk of a split span.
    let mut content = contents.join("\n\n");
    content = validate::clean_question_content(&content);
    // One labelling scheme forever: AQA '3 . 1'-style decimals → (a), (b), (c).
    content = validate::normalize_decimal_parts(&content, span.number);

    if content.trim().is_empty() && span.expected_marks.unwrap_or(0) > 0 {
        // A marked question with no content is a hard failure.
        return (None, report);
    }
    if content.trim().is_empty() {
        needs_review = true;
        notes.push("no content extracted for this span".to_string());
        content = String::new();
    }

    if !validate::has_terminal_ending(&content) {
        needs_review = true;
        notes.push("content lacks terminal punctuation (possible truncation)".to_string());
    }

    // Marks: printed footer is authoritative; inline tags next; AI estimate last.
    let inline = validate::sum_inline_marks(&content);
    let (marks, mark_note) = match (span.expected_marks, inline) {
        (Some(e), 0) => (e as i32, None),
        (Some(e), n) if n == e => (e as i32, None),
        (Some(e), n) => (
            e as i32,
            Some(format!(
                "inline marks sum ({}) differs from printed footer ({}) — trusting footer",
                n, e
            )),
        ),
        (None, n) if n > 0 => (n as i32, None),
        (None, _) => (
            ai_marks.unwrap_or(1).max(1),
            Some("marks estimated by AI (no footer/tags)".to_string()),
        ),
    };
    if let Some(n) = mark_note.clone() {
        if n.starts_with("inline marks sum") {
            needs_review = true;
        }
        notes.push(n);
    }

    // Topic containment: exact-match against the allow-list (deterministic).
    topics_acc.sort();
    topics_acc.dedup();

    (
        Some(BuiltQuestion {
            question_number: span.number,
            content,
            marks,
            topics: topics_acc,
            module: config.module_name.clone(),
            is_code: config.subject == "Computer Science" && is_code_acc,
            needs_review,
            notes,
        }),
        report,
    )
}

/// Deterministic per-item validation for a span. Returns human-readable
/// violations (quoted verbatim back to the model in the repair prompt).
fn validate_span_items(page: &AiQuestionPage, span: &QuestionSpan) -> Vec<String> {
    let mut errors = Vec::new();
    for (idx, item) in page.items.iter().enumerate() {
        if let Some(v) = &item.question_number {
            match validate::value_to_question_number(v) {
                Some(n) if n == span.number => {}
                Some(n) => errors.push(format!(
                    "item {} has question_number {} but this call is for Question {}",
                    idx + 1,
                    n,
                    span.number
                )),
                None => errors.push(format!(
                    "item {} has an implausible question_number ({}); expected exactly {}",
                    idx + 1,
                    v,
                    span.number
                )),
            }
        }
        let content = item.content.as_deref().unwrap_or("");
        if content.trim().len() < 5 && span.expected_marks.unwrap_or(0) > 1 {
            errors.push(format!(
                "item {} content is nearly empty but Question {} carries {} marks — transcribe the full text",
                idx + 1,
                span.number,
                span.expected_marks.unwrap_or(0)
            ));
        }
        if let Some(bboxes) = &item.diagram_bboxes {
            if let Some(indexes) = &item.bbox_page_indexes {
                if indexes.len() != bboxes.len() {
                    errors.push(
                        "bbox_page_indexes length must equal diagram_bboxes length".to_string(),
                    );
                }
            }
            for bbox in bboxes {
                if bbox.len() != 4 {
                    errors.push("every diagram bbox must have exactly 4 numbers".to_string());
                    break;
                }
            }
        }
    }
    errors
}

/// PVRV "Validate" for diagram proposals: every box the AI drew is pushed
/// through the Rust guard chain BEFORE the response is accepted.
///
/// Guard chain:
///   1. well-formed 4-number bbox
///   2. page index in range (using `local_to_chunk` to map model-visible
///      image indices back to `chunk` indices, since sentinel pages are
///      filtered before sending)
///   3. y-band check: the box's CENTER y must lie within this question's
///      vertical band on that page (±3% slack), otherwise the AI is boxing
///      a figure that belongs to a neighboring question
///   4. crop sanity (degenerate / blank / answer-grid)
///   5. near-duplicate signature check
///
/// `page_bands` is parallel to `chunk`; entries are `Some((low, high))`
/// when the page was given a vertical band hint. A None entry means the
/// whole page belongs to this span (no y-band restriction).
///
/// Returns the indices of offending boxes `(item_idx, bbox_idx)` plus a
/// quoted feedback message per violation for the repair loop. The AI draws
/// boxes; Rust decides which ones may ever become files.
fn audit_diagram_boxes(
    decoded_pages: &[Option<Arc<image::DynamicImage>>],
    items: &mut [AiQuestion],
    local_to_chunk: &[usize],
    _page_bands: &[Option<(f32, f32)>],
) -> (Vec<(usize, usize)>, Vec<String>) {
    let mut bad: Vec<(usize, usize)> = Vec::new();
    let mut issues: Vec<String> = Vec::new();
    let mut accepted_sigs: Vec<[u8; 64]> = Vec::new();

    for (ii, item) in items.iter_mut().enumerate() {
        let indexes = item.bbox_page_indexes.clone().unwrap_or_default();
        let Some(bboxes) = &mut item.diagram_bboxes else {
            continue;
        };
        for (bi, bbox) in bboxes.iter_mut().enumerate() {
            let label = format!("item {} diagram {}", ii + 1, bi + 1);
            if bbox.len() != 4 || bbox.iter().any(|v| !v.is_finite() || *v < 0.0) {
                bad.push((ii, bi));
                issues.push(format!(
                    "{label}: bbox must be exactly [x, y, w, h] (4 finite non-negative numbers)"
                ));
                continue;
            }

            let model_idx = indexes.get(bi).and_then(|v| value_to_usize(v)).unwrap_or(0);
            if model_idx >= local_to_chunk.len() {
                bad.push((ii, bi));
                issues.push(format!(
                    "{label}: bbox_page_indexes entry {} is out of range ({} page image(s) were sent) — renumber or drop this box",
                    model_idx,
                    local_to_chunk.len()
                ));
                continue;
            }
            let chunk_idx = local_to_chunk[model_idx];
            if chunk_idx >= decoded_pages.len() {
                bad.push((ii, bi));
                issues.push(format!(
                    "{label}: internal page-index translation failed for page {} — drop this box",
                    model_idx
                ));
                continue;
            }

            let img = match &decoded_pages[chunk_idx] {
                Some(image) => image.as_ref(),
                // Cannot judge an undecodable page here; the save-time guard
                // still applies, so nothing bad can reach disk.
                _ => continue,
            };
            let content = item.content.as_deref().unwrap_or("");
            let ignore_grid = validate::figure_references(content) > 0 && !validate::is_answer_grid_request(content);

            let graph_like = item
                .diagram_kinds
                .as_ref()
                .and_then(|kinds| kinds.get(bi))
                .map(|kind| {
                    let kind = kind.to_ascii_lowercase();
                    kind.contains("graph")
                        || kind.contains("chart")
                        || kind.contains("plot")
                        || kind.contains("composite_visual_options")
                })
                .unwrap_or(false);
            let cropped = match geometry::crop_diagram_with_options(
                img,
                bbox,
                40,
                ignore_grid,
                graph_like,
            ) {
                Ok(c) => c,
                Err(geometry::CropReject::BadBox) => {
                    bad.push((ii, bi));
                    issues.push(format!(
                        "{label}: the box is unusable (degenerate or outside the page) — redraw it tightly around the figure, or delete the box AND its [DIAGRAM_PLACEHOLDER]"
                    ));
                    continue;
                }

                Err(geometry::CropReject::AnswerGrid) => {
                    bad.push((ii, bi));
                    issues.push(format!(
                        "{label}: the box covers an EMPTY RULED ANSWER GRID (trace table / working grid). Never box these — transcribe the grid as a Markdown table inside \"content\" (keeping any pre-filled cells) and delete the box AND its [DIAGRAM_PLACEHOLDER]"
                    ));
                    continue;
                }
            };
            let sig = geometry::tile_signature(&cropped);
            if let Some(dup) = accepted_sigs
                .iter()
                .position(|s| geometry::signature_distance(s, &sig) < 4)
            {
                bad.push((ii, bi));
                issues.push(format!(
                    "{label}: identical image to box #{} — keep only ONE box and ONE placeholder per figure",
                    dup + 1
                ));
                continue;
            }
            accepted_sigs.push(sig);
        }
    }
    (bad, issues)
}

/// Terminal deterministic repair: after the repair budget is spent, drop the
/// offending boxes (and their page-index entries) so they can never reach
/// disk. The placeholders they leave behind are stripped by the caller's
/// trailing replace — nothing dangles, and every drop lands in the report.
fn prune_bad_diagram_boxes(
    items: &mut [AiQuestion],
    bad: &[(usize, usize)],
    report: &mut ImportReport,
) {
    for (ii, item) in items.iter_mut().enumerate() {
        let drop: Vec<usize> = bad
            .iter()
            .filter(|(i, _)| *i == ii)
            .map(|(_, b)| *b)
            .collect();
        if drop.is_empty() {
            continue;
        }
        let old_boxes = item.diagram_bboxes.take().unwrap_or_default();
        let old_indexes = item.bbox_page_indexes.take();
        let mut kept_boxes = Vec::new();
        let mut kept_indexes = Vec::new();
        for (bi, b) in old_boxes.into_iter().enumerate() {
            if drop.contains(&bi) {
                report.crop_rejections += 1;
                continue;
            }
            kept_boxes.push(b);
            if let Some(ix) = &old_indexes {
                if let Some(v) = ix.get(bi) {
                    kept_indexes.push(v.clone());
                }
            }
        }
        if !kept_boxes.is_empty() {
            item.diagram_bboxes = Some(kept_boxes);
            if old_indexes.is_some() {
                item.bbox_page_indexes = Some(kept_indexes);
            }
        }
    }
}

/// Crop + persist one diagram; returns the markdown link on success.
/// `saved` carries the (signature, link) pairs already persisted for this
/// unit of work — a near-identical crop reuses the stored file instead of
/// writing yet another PNG of the same figure.
fn save_diagram(
    global_page_idx: usize,
    page_b64: Option<&str>,
    bbox: &[f32],
    config: &PipelineConfig,
    page_render_cache: &crate::pdf_render::PageRenderCache,
    saved: &mut Vec<([u8; 64], String)>,
    report: &mut ImportReport,
    ignore_grid: bool,
    graph_like: bool,
) -> Option<String> {
    if bbox.len() != 4 {
        report.crop_rejections += 1;
        return None;
    }
    let img = if let Some(pdf_path) = &config.pdf_path {
        page_render_cache
            .get_or_render(pdf_path, global_page_idx)
            .ok()
    } else { None };

    let img = match img {
        Some(i) => i,
        None => {
            let b64 = page_b64?;
            std::sync::Arc::new(geometry::decode_page_image(b64)?)
        }
    };
    let cropped = match geometry::crop_diagram_with_options(
        img.as_ref(),
        bbox,
        40,
        ignore_grid,
        graph_like,
    ) {
        Ok(c) => c,
        Err(reason) => {
            report.crop_rejections += 1;
            report.anomalies.push(format!(
                "diagram box [{:.3}, {:.3}, {:.3}, {:.3}] rejected at save ({:?})",
                bbox[0], bbox[1], bbox[2], bbox[3], reason
            ));
            return None;
        }
    };
    let sig = geometry::tile_signature(&cropped);
    if let Some((_, link)) = saved
        .iter()
        .find(|(s, _)| geometry::signature_distance(s, &sig) < 4)
    {
        report.diagrams_deduped += 1;
        return Some(link.clone());
    }
    let dir = config.diagrams_dir.as_ref()?;
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join(format!("{}.png", uuid::Uuid::new_v4()));

    let (cw, ch) = (cropped.width(), cropped.height());
    let max_crop_dim: u32 = 1600;
    let final_crop = if cw > max_crop_dim || ch > max_crop_dim {
        let scale = max_crop_dim as f32 / (cw.max(ch) as f32);
        let new_w = (cw as f32 * scale).round().max(1.0) as u32;
        let new_h = (ch as f32 * scale).round().max(1.0) as u32;
        image::imageops::resize(
            &cropped,
            new_w,
            new_h,
            image::imageops::FilterType::Triangle,
        )
    } else {
        cropped
    };

    if final_crop.save(&path).is_err() {
        report.crop_rejections += 1;
        return None;
    }
    report.diagrams_saved += 1;
    let link = format!(
        "\n\n![Diagram]({})\n\n",
        path.to_string_lossy().replace('\\', "/")
    );
    saved.push((sig, link.clone()));
    Some(link)
}

/// Fallback: no map — per-page extraction, AI proposes the number but it
/// must be plausible and non-decreasing (monotonicity enforced).
///
/// Phase 2: returns ALL items extracted from the page, not just the first.
/// Dense MCQ / short-answer pages (AQA Section B) can have 4+ questions per
/// page — the previous `.next().unwrap()` silently discarded all but the first.
/// Returns:
///   - `Some(vec![])` — skip page (continuation / blank / no new questions)
///   - `Some(vec![q1, q2, ...])` — one or more questions extracted
///   - `None` — quarantine (all repair attempts exhausted)
async fn extract_fallback_page<C: LlmClient>(
    client: &C,
    config: &PipelineConfig,
    page: &PageInput,
    page_idx: usize,
    next_allowed: u32,
    page_render_cache: &Arc<crate::pdf_render::PageRenderCache>,
    request_semaphore: &Arc<Semaphore>,
) -> (Option<Vec<BuiltQuestion>>, ImportReport) {
    // Own, local report: pages now run in parallel batches.
    let mut report = ImportReport::default();
    let max_attempts = 1 + config.max_repairs;
    let system = format!(
        r#"You are a precise mathematical OCR engine. Output ONLY a valid JSON object {{"items": [ ... ]}}.

RULES:
- If this page contains NEW question(s) (each with its own printed whole-question number), return ONE item per question:
  {{ "question_number": <whole number printed>, "content": "<full transcription>", "marks": int|null,
     "topics": array, "module": "{module}", "is_code": bool,
     "diagram_bboxes": [[x,y,w,h]...] relative 0.0-1.0, "bbox_page_indexes": [0,...] }}
- MULTIPLE QUESTIONS ON ONE PAGE: when a page has several independent short-answer or multiple-choice questions (e.g. AQA Section B with 4 MCQs), return an item for EACH question. Do NOT bundle them into one item.
- QUESTION ISOLATION (highest priority): never place sub-parts of two different main questions in one item. A sub-part label ((a), (b), (i), "04.2") belongs to the main number printed in the label, or else to the nearest whole-number heading ABOVE it. A "(Total for Question N is M marks)" footer, or a new whole question number, ENDS that question — everything after it starts a new item. If sub-part lettering restarts at (a), a new main question has begun. When unsure which question owns a line, start a new item rather than merging.
- If this page is a CONTINUATION of the previous question, is blank, or contains no new question, return {{"items": []}}.
- Transcribe fully (never summarize). Preserve punctuation. `**[X marks]**` after each marked sub-part. Math in $...$/$$...$$. Markdown tables for text tables; \begin{{array}} only for matrices. Code in backticks, never math mode. Escape LaTeX backslashes (\\frac).
- AQA decimal sub-parts: render '03.1'-style part numbers as (a), (b), (c) — positional: .1 -> a, .2 -> b — and update inline cross-references. AQA also uses SPACED sub-parts: \"01 5\" means Question 1, sub-part 5 — render as (e). The whole question number is ALWAYS the integer (never a decimal like 1.5). The whole decimal run on this page is ONE item with its integer question number.
- Anything the paper labels as a Figure ("Figure 6" — printed schemas, algorithm screens, grids that are part of the question exhibit) MUST be returned as a diagram box, never as transcribed text.
- STRUCTURED TABLES WITH HEADERS (trace tables, function tables, working grids) are question content even when EMPTY — transcribe them as Markdown tables, NEVER as diagram boxes. Diagram boxes are ONLY for figures that cannot be typed (graphs, circuits, line drawings), one box per figure; blank, empty-grid, and duplicate boxes are rejected by the parser and cost a repair round.
- Exclude headers/footers ("Question X continued", "Turn over", totals footers), plain ruled answer lines, answer line templates with operators (e.g. "............ $\\le t <$ ............"), "BLANK PAGE".
- Content must end with terminal punctuation or a mark tag."#,
        module = config.module_name,
    );

    let preparation_inputs = match &page.kind {
        PageInputKind::Image { b64, .. } => vec![ChunkImageInput {
            chunk_idx: 0,
            b64: b64.clone(),
            start_y: None,
            end_y: None,
        }],
        PageInputKind::TextOnly => Vec::new(),
    };
    let prepared = match prepare_chunk_images(1, preparation_inputs).await {
        Ok(prepared) => prepared,
        Err(error) => {
            report.anomalies.push(format!(
                "page {} image preparation task failed: {}",
                page_idx + 1,
                error
            ));
            return (None, report);
        }
    };
    let page_images = prepared.images;
    let local_to_chunk = prepared.local_to_chunk;
    let page_bands = prepared.page_bands;
    let decoded_pages = prepared.decoded_pages;

    let mut last_error = String::new();
    for attempt in 1..=max_attempts {
        let user_text = format!(
            "Extract ALL NEW questions on this page (page {}), returning one item per question. Return an empty items array if the page is a continuation or blank.{}",
            page_idx + 1,
            if attempt == 1 {
                String::new()
            } else {
                format!(
                    "\n\nPREVIOUS ATTEMPT FAILED VALIDATION: {}. Regenerate corrected JSON.",
                    last_error
                )
            }
        );
        // Phase 0: never pass sentinel b64 values as images. Build a
        // (possibly-empty) image slice from the page; `chat_body` will
        // produce a text-only body when no images are supplied. Mirror
        // the mapped path's local_to_chunk so audit/save can resolve
        // bbox_page_indexes correctly even when sentinels are filtered.
        let body = llm::chat_body(
            &config.model,
            &system,
            &page_images,
            Some(&user_text),
            config.max_output_tokens,
        );
        let resp = match chat_with_permit(client, &body, request_semaphore).await {
            Ok(r) => r,
            Err(e) => {
                last_error = e.to_string();
                continue;
            }
        };
        let content = match llm::message_content(&resp) {
            Ok(c) => c,
            Err(e) => {
                last_error = e.to_string();
                continue;
            }
        };
        let page_out = match parse_llm_json::<AiQuestionPage>(&content) {
            ParseOutcome::Clean(v) => v,
            ParseOutcome::Salvaged { value, dropped_tail } => {
                report.salvage_events += 1;
                // Phase 2: truncation check. If the page response was cut off,
                // we must retry to avoid dropping questions.
                if dropped_tail {
                    last_error = "response was truncated; items may be missing".to_string();
                    if attempt < max_attempts {
                        continue;
                    }
                }
                value
            }
            ParseOutcome::Malformed { error } => {
                last_error = format!("invalid JSON: {}", error);
                report.repairs += 1;
                continue;
            }
        };
        if page_out.items.is_empty() {
            return (Some(vec![]), report);
        }

        // Phase 2: validate ALL items' question numbers. Each must be plausible
        // (≥ next_allowed - 1) and the sequence must be non-decreasing within
        // the page. Collect validated numbers parallel to items.
        let mut item_numbers: Vec<u32> = Vec::with_capacity(page_out.items.len());
        let mut number_valid = true;
        for (idx, item) in page_out.items.iter().enumerate() {
            let number = item
                .question_number
                .as_ref()
                .and_then(validate::value_to_question_number);
            match number {
                Some(n) if n >= next_allowed.saturating_sub(1) => {
                    // Check non-decreasing within page
                    if let Some(&prev) = item_numbers.last() {
                        if n < prev {
                            last_error = format!(
                                "item {} has question_number {} which is less than item {}'s {} — question numbers must be non-decreasing within a page",
                                idx + 1, n, idx, prev
                            );
                            number_valid = false;
                            break;
                        }
                    }
                    item_numbers.push(n);
                }
                Some(n) => {
                    last_error = format!(
                        "item {} has backwards question number {} (expected ≥ {})",
                        idx + 1, n, next_allowed
                    );
                    number_valid = false;
                    break;
                }
                None => {
                    last_error = format!(
                        "item {} has an implausible question_number ({}); expected a whole number ≥ {}",
                        idx + 1,
                        item.question_number.as_ref().map(|v| v.to_string()).unwrap_or_default(),
                        next_allowed
                    );
                    number_valid = false;
                    break;
                }
            }
        }
        if !number_valid {
            report.repairs += 1;
            continue;
        }

        // Phase 2: figure-reference consistency check on each item
        let mut all_fig_errors: Vec<String> = Vec::new();
        for (idx, item) in page_out.items.iter().enumerate() {
            let fig_errors = validate::diagram_consistency_errors(
                item.content.as_deref().unwrap_or(""),
                item.diagram_bboxes.as_ref().map(|b| b.len()).unwrap_or(0),
            );
            for e in fig_errors {
                all_fig_errors.push(format!("item {}: {}", idx + 1, e));
            }
        }
        if !all_fig_errors.is_empty() {
            report.repairs += 1;
            if attempt < max_attempts {
                last_error = all_fig_errors.join("; ");
                continue;
            }
            report.anomalies.push(format!(
                "page {}: figure/diagram inconsistency kept after repair budget — {}",
                page_idx + 1,
                all_fig_errors.join("; ")
            ));
        }

        // Phase 2: diagram audit on ALL items at once (not just the first)
        let audit_items = page_out.items;
        let audit_local_to_chunk = local_to_chunk.clone();
        let audit_page_bands = page_bands.clone();
        let audit_decoded_pages = decoded_pages.clone();
        let (mut items, bad, box_issues) = match tokio::task::spawn_blocking(move || {
            let mut items = audit_items;
            let (bad, issues) = audit_diagram_boxes(
                &audit_decoded_pages,
                &mut items,
                &audit_local_to_chunk,
                &audit_page_bands,
            );
            (items, bad, issues)
        })
        .await
        {
            Ok(result) => result,
            Err(error) => {
                last_error = format!("diagram audit task failed: {}", error);
                report.repairs += 1;
                continue;
            }
        };
        if !box_issues.is_empty() {
            report.repairs += 1;
            if attempt < max_attempts {
                last_error = box_issues.join("; ");
                continue;
            }
            report.anomalies.push(format!(
                "page {}: dropped {} invalid diagram box(es) after repair budget spent — {}",
                page_idx + 1,
                bad.len(),
                box_issues.join("; ")
            ));
            prune_bad_diagram_boxes(&mut items, &bad, &mut report);
        }

        // Phase 2: process EVERY item — build a BuiltQuestion for each
        let mut built_questions: Vec<BuiltQuestion> = Vec::with_capacity(items.len());
        let mut saved_diagrams: Vec<([u8; 64], String)> = Vec::new();

        for (idx, mut item) in items.into_iter().enumerate() {
            let number = item_numbers[idx];
            let mut item_content = item.content.take().unwrap_or_default();

            // Save diagrams for this item
            if let Some(bboxes) = &item.diagram_bboxes {
                let indexes = item.bbox_page_indexes.clone().unwrap_or_default();
                let mut requests = Vec::with_capacity(bboxes.len());
                for (bi, bbox) in bboxes.iter().enumerate() {
                    // Resolve the page index through local_to_chunk
                    let model_idx = indexes
                        .get(bi)
                        .and_then(value_to_usize)
                        .filter(|&k| k < local_to_chunk.len())
                        .unwrap_or(0);
                    let _chunk_idx = local_to_chunk[model_idx];
                    let ignore_grid = validate::figure_references(&item_content) > 0 && !validate::is_answer_grid_request(&item_content);
                    requests.push(DiagramSaveRequest {
                        global_page_idx: page_idx,
                        bbox: bbox.clone(),
                        ignore_grid,
                        graph_like: false,
                    });
                }
                let mut page_b64 = std::collections::HashMap::new();
                if config.pdf_path.is_none() {
                    if let Some(b64) = page.get_b64() {
                        page_b64.insert(page_idx, b64.clone());
                    }
                }
                let saved_before = saved_diagrams.clone();
                match persist_diagrams(
                    requests,
                    page_b64,
                    config.clone(),
                    Arc::clone(page_render_cache),
                    std::mem::take(&mut saved_diagrams),
                )
                .await
                {
                    Ok(persisted) => {
                        saved_diagrams = persisted.saved;
                        report.absorb(persisted.report);
                        for link in persisted.links.into_iter().flatten() {
                        if item_content.contains("[DIAGRAM_PLACEHOLDER]") {
                            item_content = item_content.replacen("[DIAGRAM_PLACEHOLDER]", &link, 1);
                        } else {
                            item_content.push_str(&link);
                        }
                    }
                    }
                    Err(error) => {
                        saved_diagrams = saved_before;
                        report.anomalies.push(format!(
                            "page {} diagram persistence task failed: {}",
                            page_idx + 1,
                            error
                        ));
                    }
                }
            }
            item_content = item_content.replace("[DIAGRAM_PLACEHOLDER]", "");

            let mut topics: Vec<String> = Vec::new();
            if let Some(t) = &item.topics {
                for topic in value_to_topics(t) {
                    if config.allowed_topics.is_empty() || config.allowed_topics.contains(&topic) {
                        topics.push(topic);
                    }
                }
            }
            topics.sort();
            topics.dedup();

            let built = BuiltQuestion {
                question_number: number,
                content: validate::normalize_decimal_parts(
                    &validate::clean_question_content(&item_content),
                    number,
                ),
                marks: item
                    .marks
                    .as_ref()
                    .and_then(validate::value_to_marks)
                    .unwrap_or(1)
                    .max(1),
                module: config.module_name.clone(),
                topics,
                is_code: config.subject == "Computer Science" && item.is_code == Some(true),
                needs_review: true,
                notes: vec!["extracted without document map (fallback mode)".to_string()],
            };
            built_questions.push(built);
        }

        return (Some(built_questions), report);
    }
    (None, report)
}

// ══════════════════════════════════════════════════════════════════════════
// Mark-scheme pipeline
// ══════════════════════════════════════════════════════════════════════════

/// One sliding mark-scheme window: images + raw text in, validated answers
/// out. Windows run in parallel batches, so each owns a local report;
/// errors come back as Err(last_error) for the caller's quarantine record.
async fn read_markscheme_window<C: LlmClient>(
    client: &C,
    config: &PipelineConfig,
    pages: &[PageInput],
    start: usize,
    end: usize,
    step: usize,
    system: &str,
    request_semaphore: &Arc<Semaphore>,
) -> (Result<Vec<AiAnswer>, String>, ImportReport) {
    let mut report = ImportReport::default();
    let images: Vec<String> = pages[start..end]
        .iter()
        .filter_map(|p| p.get_b64().cloned())
        .collect();
    let mut chunk_text = String::new();
    for i in start..end {
        if !pages[i].text.trim().is_empty() {
            chunk_text.push_str(&format!(
                "RAW TEXT PAGE {}:\n{}\n\n---\n\n",
                i + 1,
                pages[i].text
            ));
        }
    }
    let context_note = if start == 0 {
        format!("These are pages 1–{} of the mark scheme. Extract every answer anchored on any of these pages.", end)
    } else {
        let prim_end = (start + step).min(pages.len());
        format!(
            "Page {} is context (already processed). Extract ONLY answers anchored on page{s} {}.",
            start,
            if prim_end > start + 1 {
                format!("{}–{}", start + 1, prim_end)
            } else {
                format!("{}", start + 1)
            },
            s = if prim_end > start + 1 { "s" } else { "" }
        )
    };
    let user_text = format!(
        "{}\n\nRaw text is provided as a baseline (images are authoritative):\n{}",
        context_note, chunk_text
    );

    let mut last_error = String::new();
    let mut accepted: Option<Vec<AiAnswer>> = None;
    let max_attempts = 1 + config.max_repairs;

    for attempt in 1..=max_attempts {
        let text = if attempt == 1 {
            user_text.clone()
        } else {
            format!(
                "{}\n\nPREVIOUS ATTEMPT FAILED VALIDATION: {}. Regenerate the complete corrected JSON.",
                user_text, last_error
            )
        };
        let body = llm::chat_body(
            &config.model,
            system,
            &images,
            Some(&text),
            config.max_output_tokens,
        );
        let resp = match chat_with_permit(client, &body, request_semaphore).await {
            Ok(r) => r,
            Err(e) => {
                last_error = e.to_string();
                continue;
            }
        };
        let content = match llm::message_content(&resp) {
            Ok(c) => c,
            Err(e) => {
                last_error = e.to_string();
                continue;
            }
        };
        match parse_llm_json::<AiAnswerEnvelope>(&content) {
            ParseOutcome::Clean(AiAnswerEnvelope::Wrapped { answers })
            | ParseOutcome::Clean(AiAnswerEnvelope::Bare(answers))
            | ParseOutcome::Salvaged {
                value: AiAnswerEnvelope::Wrapped { answers },
                ..
            }
            | ParseOutcome::Salvaged {
                value: AiAnswerEnvelope::Bare(answers),
                ..
            } => {
                accepted = Some(answers);
                break;
            }
            ParseOutcome::Malformed { error } => {
                last_error = format!("invalid JSON: {}", error);
                report.repairs += 1;
            }
        }
    }

    match accepted {
        Some(a) => (Ok(a), report),
        None => (Err(last_error), report),
    }
}

pub async fn run_markscheme_pipeline<C: LlmClient, P: Progress>(
    client: &C,
    pages: &[PageInput],
    config: &PipelineConfig,
    progress: &P,
    cancel: &AtomicBool,
) -> Result<(Vec<AnswerDraft>, ImportReport), String> {
    let mut report = ImportReport {
        paper_name: config.paper_name.clone(),
        kind: "mark_scheme".to_string(),
        pages_total: pages.len(),
        ..Default::default()
    };
    let page_render_cache = Arc::new(crate::pdf_render::PageRenderCache::new(
        PAGE_RENDER_CACHE_CAPACITY,
    ));
    let request_semaphore = Arc::new(Semaphore::new(config.parallelism.max(1)));
    let mut drafts: Vec<AnswerDraft> = Vec::new();
    let mut alt_count: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    // Paper-global diagram dedupe: windows overlap, so the same worked
    // table/figure is naturally re-boxed — reuse the file, don't resave it.
    let mut saved_diagrams: Vec<([u8; 64], String)> = Vec::new();

    let system = markscheme_system_prompt();

    // Sliding windows of 3, step 2 (context for answers spanning pages),
    // read in PARALLEL bounded batches. Stitch/dedupe stays sequential and
    // ordered, so the merge result is identical to the serial version.
    let window: usize = 3;
    let step: usize = 2;
    let mut windows: Vec<(usize, usize)> = Vec::new();
    {
        let mut start = 0usize;
        while start < pages.len() {
            let end = (start + window).min(pages.len());
            windows.push((start, end));
            if end >= pages.len() {
                break;
            }
            start += step;
        }
    }

    cancelled(cancel)?;
    progress.stage(&format!("Reading {} mark-scheme windows…", windows.len()));
    let mut results = futures_util::stream::iter(windows.iter().copied().enumerate().map(|(position, (start, end))| {
        read_markscheme_window(
            client,
            config,
            pages,
            start,
            end,
            step,
            &system,
            &request_semaphore,
        )
        .map(move |result| (position, result))
    }))
    .buffer_unordered(config.parallelism.max(1));
    let mut ordered_results = Vec::with_capacity(windows.len());
    while let Some(result) = results.next().await {
        ordered_results.push(result);
    }
    ordered_results.sort_by_key(|(position, _)| *position);
    for ((start, end), (_, (res, local))) in windows.iter().copied().zip(ordered_results) {
            report.absorb(local);
            let img_count = pages[start..end]
                .iter()
                .map(|_| 1)
                .count();
            let answers = match res {
                Ok(a) => {
                    report.pages_processed += end - start;
                    a
                }
                Err(last_error) => {
                    report.quarantined.push(QuarantineEvent {
                        scope: "mark-scheme-window".to_string(),
                        page: Some(start + 1),
                        question_number: None,
                        reason: format!(
                            "window pages {}–{} failed validation: {}",
                            start + 1,
                            end,
                            last_error
                        ),
                    });
                    continue;
                }
            };

            for ans in answers {
                let q_num = match ans
                    .question_number
                    .as_ref()
                    .and_then(validate::value_to_question_number)
                {
                    Some(n) => n,
                    None => {
                        report.anomalies.push(format!(
                            "window {}–{}: answer without a valid question number skipped",
                            start + 1,
                            end
                        ));
                        continue;
                    }
                };
                let mut md = match ans.answer_markdown {
                    Some(m) if !m.trim().is_empty() => m,
                    _ => continue,
                };

                // Diagrams (sanitized crops; page index validated).
                if let Some(bboxes) = &ans.diagram_bboxes {
                    let indexes = ans.diagram_page_indexes.clone().unwrap_or_default();
                    let mut requests = Vec::with_capacity(bboxes.len());
                    let mut page_b64 = std::collections::HashMap::new();
                    for (bi, bbox) in bboxes.iter().enumerate() {
                        let local = indexes
                            .get(bi)
                            .and_then(value_to_usize)
                            .filter(|&k| k < img_count);
                        let local = match local {
                            Some(k) => k,
                            None => {
                                report.anomalies.push(format!(
                                "answer {}: diagram {} has out-of-range page index — using first page",
                                q_num, bi + 1
                            ));
                                0
                            }
                        };
                        let ignore_grid = validate::figure_references(&md) > 0;
                        let global_page_idx = start + local;
                        if config.pdf_path.is_none() {
                            if let Some(b64) = pages[global_page_idx].get_b64() {
                                page_b64
                                    .entry(global_page_idx)
                                    .or_insert_with(|| b64.clone());
                            }
                        }
                    requests.push(DiagramSaveRequest {
                        global_page_idx,
                        bbox: bbox.clone(),
                        ignore_grid,
                        graph_like: false,
                    });
                    }
                    let saved_before = saved_diagrams.clone();
                    match persist_diagrams(
                        requests,
                        page_b64,
                        config.clone(),
                        Arc::clone(&page_render_cache),
                        std::mem::take(&mut saved_diagrams),
                    )
                    .await
                    {
                        Ok(persisted) => {
                            saved_diagrams = persisted.saved;
                            report.absorb(persisted.report);
                            for link in persisted.links.into_iter().flatten() {
                                if md.contains("[DIAGRAM_PLACEHOLDER]") {
                                    md = md.replacen("[DIAGRAM_PLACEHOLDER]", &link, 1);
                                } else {
                                    md.push_str(&link);
                                }
                            }
                        }
                        Err(error) => {
                            saved_diagrams = saved_before;
                            report.anomalies.push(format!(
                                "answer {} diagram persistence task failed: {}",
                                q_num, error
                            ));
                        }
                    }
                }
                md = md.replace("[DIAGRAM_PLACEHOLDER]", "");
                md = validate::normalize_decimal_parts(&md, q_num);
                md = validate::harden_line_breaks(&md);
                md = validate::sanitize_markdown_math(&md);
                md = validate::normalize_mark_scheme_chunk(&md);

                // Dedupe/stitch: containment-based, not a brittle prefix fingerprint.
                if let Some(existing) = drafts.iter_mut().find(|d| d.question_number == q_num) {
                    if validate::is_duplicate_answer(&existing.markdown, &md) {
                        continue;
                    }
                    let alts = alt_count.entry(q_num).or_insert(0);
                    if *alts == 0 {
                        *alts += 1;
                        existing.markdown.push_str("\n\n---\n\n");
                        existing.markdown.push_str(&md);
                    } else {
                        continue;
                    }
                } else {
                    drafts.push(AnswerDraft {
                        question_number: q_num,
                        markdown: md,
                    });
                }
            }
        }

    Ok((drafts, report))
} // Tests — the golden suite. Deterministic: MockLlm replays scripted model
  // behaviour (valid, hallucinating, truncating, junk) so every failure class
  // stays dead forever.
  // ══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{ok_chat, LlmError, MockLlm};

    fn pages(n: usize) -> Vec<PageInput> {
        (0..n)
            .map(|_| PageInput {
                kind: PageInputKind::TextOnly,
                text: String::new(),
            })
            .collect()
    }

    fn config() -> PipelineConfig {
        let mut c = PipelineConfig::new(
            "test-model".into(),
            "Unit".into(),
            "Mathematics".into(),
            "Algebra".into(),
            None,
        );
        c.allowed_topics = vec!["Proof".into(), "Integration".into()];
        c.max_repairs = 2;
        c
    }

    fn cancel_flag() -> AtomicBool {
        AtomicBool::new(false)
    }

    fn paper_pages() -> Vec<PageInput> {
        vec![
            PageInput { kind: PageInputKind::TextOnly, text: "Instructions\nAnswer ALL questions".into() },
            PageInput { kind: PageInputKind::TextOnly, text: "1. Prove the thing. - This page needs to be longer than 100 characters so it is considered ambiguous, and we remove the footer so it's not considered reliable. Let's pad it out with some more text to be absolutely sure.".into() },
            PageInput { kind: PageInputKind::TextOnly, text: "2. Integrate this. (Total for Question 2 is 4 marks)\nTOTAL FOR PAPER IS 7 MARKS".into() },
        ]
    }

    fn structure_reply(
        role: &str,
        nums: &str,
        footer: &str,
    ) -> Result<serde_json::Value, LlmError> {
        ok_chat(&format!(
            r#"{{"question_numbers_visible": {}, "total_marks_footer": {}, "page_role": "{}"}}"#,
            nums, footer, role
        ))
    }

    #[tokio::test]
    async fn happy_path_full_checksum() {
        let mock = MockLlm::new(vec![
            // structure pass × 2 (page 0 is skipped because it's NonQuestion)
            structure_reply("QUESTION", "[1]", "[1, 3]"),
            structure_reply("QUESTION", "[2]", "[2, 4]"),
            // extraction span 1
            ok_chat(
                r#"{"items":[{"question_number":1,"content":"Prove that the thing holds. **[3 marks]**","marks":3,"topics":["Proof"],"module":"Pure"}]}"#,
            ),
            // extraction span 2
            ok_chat(
                r#"{"items":[{"question_number":2,"content":"Integrate $x^2$ from 0 to 2. **[4 marks]**","marks":4,"topics":["Integration"],"module":"Pure"}]}"#,
            ),
        ]);
        let pgs = paper_pages();
        let (built, report) =
            run_question_pipeline(&mock, &pgs, &config(), &NullProgress, &cancel_flag())
                .await
                .unwrap();
        println!("BUILT: {:#?}", built);
        println!("REPORT: {:#?}", report);

        assert_eq!(built.len(), 2);
        assert_eq!(built[0].question_number, 1);
        assert_eq!(built[0].marks, 3);
        assert_eq!(built[1].marks, 4);
        assert_eq!(report.questions_expected, 2);
        assert_eq!(report.questions_extracted, 2);
        assert_eq!(report.marks_checksum_ok, Some(true));
        assert!(report.quarantined.is_empty());
        assert_eq!(mock.remaining(), 0);
    }

    #[tokio::test]
    async fn invalid_json_is_repaired_not_corrupted() {
        let mock = MockLlm::new(vec![
            // structure pass
            structure_reply("QUESTION", "[1]", "[1, 3]"),
            structure_reply("QUESTION", "[2]", "[2, 4]"),
            // span 1: junk first, then the repair round-trip yields valid JSON
            ok_chat("sorry, I cannot help with that… not json"),
            ok_chat(
                r#"{"items":[{"question_number":1,"content":"Prove it fully here. **[3 marks]**","marks":3,"topics":["Proof"],"module":"Pure"}]}"#,
            ),
            // span 2 clean
            ok_chat(
                r#"{"items":[{"question_number":2,"content":"Integrate it. **[4 marks]**","marks":4,"topics":["Integration"],"module":"Pure"}]}"#,
            ),
        ]);
        let pgs = paper_pages();
        let (built, report) =
            run_question_pipeline(&mock, &pgs, &config(), &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert_eq!(built.len(), 2);
        assert!(report.repairs >= 1);
        assert!(report.quarantined.is_empty());
        // The repair response mentions the failure:
        let bodies = mock.bodies();
        let repair_body = &bodies[4];
        let sys = repair_body["messages"][0]["content"].as_str().unwrap();
        assert!(sys.contains("Question 1"));
    }

    #[tokio::test]
    async fn hallucinated_question_number_is_rejected() {
        let mock = MockLlm::new(vec![
            // structure pass
            structure_reply("QUESTION", "[1]", "[1, 3]"),
            structure_reply("QUESTION", "[2]", "[2, 4]"),
            // span 1: model insists on question 99 — every attempt rejected.
            ok_chat(r#"{"items":[{"question_number":99,"content":"wrong. **[3 marks]**"}]}"#),
            ok_chat(r#"{"items":[{"question_number":99,"content":"wrong. **[3 marks]**"}]}"#),
            ok_chat(r#"{"items":[{"question_number":99,"content":"wrong. **[3 marks]**"}]}"#),
            // span 2 fine
            ok_chat(
                r#"{"items":[{"question_number":2,"content":"Integrate it. **[4 marks]**","marks":4}]}"#,
            ),
        ]);
        let pgs = paper_pages();
        let (built, report) =
            run_question_pipeline(&mock, &pgs, &config(), &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert_eq!(built.len(), 1); // Only Q2 was built
        assert_eq!(report.quarantined.len(), 1); // Q1 was quarantined after 3 attempts
        assert_eq!(built[0].question_number, 2);
    }

    #[tokio::test]
    async fn truncated_mid_item_is_repaired() {
        let mock = MockLlm::new(vec![
            // structure pass
            structure_reply("QUESTION", "[1]", "[1, 3]"),
            structure_reply("QUESTION", "[2]", "[2, 4]"),
            // span 1: truncated mid-string (no complete item → repair), then valid
            ok_chat(
                r#"{"items":[{"question_number":1,"content":"Prove that the thing holds completely"#,
            ),
            ok_chat(
                r#"{"items":[{"question_number":1,"content":"Prove that the thing holds, with steps. **[3 marks]**","marks":3}]}"#,
            ),
            // span 2
            ok_chat(
                r#"{"items":[{"question_number":2,"content":"Integrate it. **[4 marks]**","marks":4}]}"#,
            ),
        ]);
        let pgs = paper_pages();
        let (built, report) =
            run_question_pipeline(&mock, &pgs, &config(), &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert_eq!(built.len(), 2);
        assert!(report.repairs >= 1);
    }

    #[tokio::test]
    async fn truncation_after_complete_item_uses_salvage_path() {
        let mock = MockLlm::new(vec![
            // structure pass
            structure_reply("QUESTION", "[1]", "[1, 3]"),
            structure_reply("QUESTION", "[2]", "[2, 4]"),
            // span 1: one full item then a truncated second item, then valid
            ok_chat(
                r#"{"items":[{"question_number":1,"content":"Prove the claim. **[3 marks]**"},{"question_number":1,"content":"cut off mid sen"#,
            ),
            ok_chat(
                r#"{"items":[{"question_number":1,"content":"Prove the claim. **[3 marks]**"}]}"#,
            ),
            // span 2
            ok_chat(
                r#"{"items":[{"question_number":2,"content":"Integrate it. **[4 marks]**","marks":4}]}"#,
            ),
        ]);
        let pgs = paper_pages();
        let (built, report) =
            run_question_pipeline(&mock, &pgs, &config(), &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert_eq!(built.len(), 2);
        assert!(report.salvage_events >= 1);
    }

    #[tokio::test]
    async fn mark_scheme_dedupes_overlapping_windows() {
        let pgs = pages(4); // window=3 step=2 → 2 overlapping calls
        let mock = MockLlm::new(vec![
            // window pages 1–3
            ok_chat(
                r#"{"answers":[{"question_number":1,"answer_markdown":"**(a)** Use integration to find the area of the region R = 12.5 units squared."},{"question_number":2,"answer_markdown":"Take logs of both sides then solve."}]}"#,
            ),
            // window pages 3–4 overlap: Q2 re-transcribed with noise → dup; Q3 new
            ok_chat(
                r#"{"answers":[{"question_number":2,"answer_markdown":"take logs of both sides and then solve."},{"question_number":3,"answer_markdown":"Differentiate implicitly to get the gradient."}]}"#,
            ),
        ]);
        let mut c = config();
        c.max_output_tokens = 4096;
        let (drafts, report) =
            run_markscheme_pipeline(&mock, &pgs, &c, &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert_eq!(drafts.len(), 3);
        assert!(report.quarantined.is_empty());
        let q2 = drafts.iter().find(|d| d.question_number == 2).unwrap();
        assert!(!q2.markdown.contains("---")); // not stitched twice
    }

    #[tokio::test]
    async fn mark_scheme_window_failure_is_quarantined() {
        let pgs = pages(4);
        let mock = MockLlm::new(vec![
            ok_chat("totally not json"),
            ok_chat("still not json"),
            ok_chat("nope"),
            // remaining windows fine
            ok_chat(r#"{"answers":[{"question_number":1,"answer_markdown":"Answer one."}]}"#),
            ok_chat(r#"{"answers":[{"question_number":2,"answer_markdown":"Answer two."}]}"#),
        ]);
        let c = config();
        let (_drafts, report) =
            run_markscheme_pipeline(&mock, &pgs, &c, &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert_eq!(report.quarantined.len(), 1);
        assert!(report.quarantined[0].scope.contains("mark-scheme"));
    }

    // ── Diagram audit: trace-table regression (AQA CS June 2024 Q30) ─────
    // Ten near-identical PNGs of an EMPTY student trace table were saved as
    // "diagrams" because the blank guard can't see ruled grids. These tests
    // pin the invariant: Rust audits every box, quotes violations back to
    // the model, prunes what never gets fixed, and dedupes what gets saved.

    fn gray_blank(w: u32, h: u32) -> image::GrayImage {
        image::GrayImage::from_pixel(w, h, image::Luma([255u8]))
    }
    fn g_hline(g: &mut image::GrayImage, y: u32) {
        for x in 0..g.width() {
            g.put_pixel(x, y, image::Luma([40u8]));
        }
    }
    fn g_vline(g: &mut image::GrayImage, x: u32, y0: u32, y1: u32) {
        for y in y0..y1 {
            g.put_pixel(x, y, image::Luma([40u8]));
        }
    }
    fn g_blob(g: &mut image::GrayImage, y: u32, x0: u32, w: u32) {
        for x in x0..(x0 + w).min(g.width()) {
            g.put_pixel(x, y, image::Luma([60u8]));
            g.put_pixel(x, y + 3, image::Luma([60u8]));
        }
    }

    /// The offending artifact: header blobs + 25 ruled rows + 6 column rules.
    fn trace_table_img() -> image::GrayImage {
        let mut g = gray_blank(600, 900);
        let rows: Vec<u32> = (0..25).map(|i| 20 + i * 34).collect();
        for &r in &rows {
            g_hline(&mut g, r);
        }
        for c in [20u32, 215, 420, 470, 520, 570] {
            g_vline(&mut g, c, 20, *rows.last().unwrap());
        }
        g_blob(&mut g, 40, 60, 220);
        g_blob(&mut g, 44, 260, 150);
        g
    }

    /// A legit figure: two axes and a plotted polyline, no ruled grid.
    fn chart_img() -> image::GrayImage {
        let mut g = gray_blank(600, 400);
        g_hline(&mut g, 370);
        g_vline(&mut g, 40, 0, 399);
        for x in 40..580u32 {
            let y = (200.0 - 120.0 * ((x as f64 - 40.0) / 90.0).sin()) as i64;
            if y >= 0 {
                g.put_pixel(x, y.min(399) as u32, image::Luma([30u8]));
            }
        }
        g
    }

    fn png_b64(gray: &image::GrayImage) -> String {
        use base64::Engine;
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(gray.clone())
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(buf.into_inner())
    }

    fn grid_page() -> PageInput {
        PageInput {
            kind: PageInputKind::Image { b64: png_b64(&trace_table_img()) },
            text: String::new(),
        }
    }
    fn chart_page() -> PageInput {
        PageInput {
            kind: PageInputKind::Image { b64: png_b64(&chart_img()) },
            text: String::new(),
        }
    }

    #[test]
    fn audit_rejects_grid_and_duplicate_keeps_chart() {
        let grid = grid_page();
        let chart = chart_page();
        let decoded_pages = vec![
            grid.get_b64()
                .and_then(|b64| geometry::decode_page_image(b64))
                .map(Arc::new),
            chart
                .get_b64()
                .and_then(|b64| geometry::decode_page_image(b64))
                .map(Arc::new),
        ];
        let item = AiQuestion {
            content: Some("Complete the table. [DIAGRAM_PLACEHOLDER]".into()),
            diagram_bboxes: Some(vec![
                vec![0.10, 0.10, 0.80, 0.80], // whole trace table → AnswerGrid
                vec![0.10, 0.15, 0.70, 0.70], // chart → keep
                vec![0.03, 0.06, 0.88, 0.80], // same chart → duplicate
            ]),
            bbox_page_indexes: Some(vec![
                serde_json::json!(0),
                serde_json::json!(1),
                serde_json::json!(1),
            ]),
            ..Default::default()
        };
        // Tests send no sentinel pages, so identity map + no bands.
        let l2c: Vec<usize> = (0..decoded_pages.len()).collect();
        let bands: Vec<Option<(f32, f32)>> = vec![None; decoded_pages.len()];
        let (bad, issues) =
            audit_diagram_boxes(&decoded_pages, &mut [item], &l2c, &bands);
        assert!(bad.contains(&(0, 0)), "trace-table box must be rejected");
        assert!(
            bad.contains(&(0, 2)),
            "duplicate chart box must be rejected"
        );
        assert!(!bad.contains(&(0, 1)), "the real chart must survive");
        let joined = issues.join("; ");
        assert!(
            joined.contains("EMPTY RULED ANSWER GRID"),
            "grid feedback: {joined}"
        );
        assert!(
            joined.contains("identical image"),
            "dedupe feedback: {joined}"
        );
    }

    #[tokio::test]
    async fn repair_loop_quotes_diagram_feedback_and_recovers() {
        let pgs = vec![grid_page()];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(6),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let bad_response = r#"{"items":[{"question_number":30,"content":"Complete the flow chart below. [DIAGRAM_PLACEHOLDER] **[6 marks]**","marks":6,"topics":["Proof"],"module":"A","diagram_bboxes":[[0.10,0.10,0.80,0.80]],"bbox_page_indexes":[0]}]}"#;
        let good_response = r#"{"items":[{"question_number":30,"content":"Complete the flow chart below.\n\n[flowchart descriptions]\n\nState the final value. **[6 marks]**","marks":6,"topics":["Proof"],"module":"A"}]}"#;
        let mock = MockLlm::new(vec![ok_chat(bad_response), ok_chat(good_response)]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let (built_opt, report) =
            extract_span(&mock, &config(), &span, &span_pages, &cache, &semaphore).await;
        let built = built_opt.expect("question must build after the repair round");

        assert_eq!(mock.remaining(), 0, "both attempts consumed");
        assert!(
            mock.bodies()[1]
                .to_string()
                .contains("EMPTY RULED ANSWER GRID"),
            "the audit feedback must be quoted back to the model"
        );
        assert!(
            built.content.contains("[flowchart descriptions]"),
            "recovered flowchart content"
        );
        assert!(!built.content.contains("[DIAGRAM_PLACEHOLDER]"));
        assert!(report.repairs >= 1);
    }

    #[tokio::test]
    async fn eof_split_extracts_and_concatenates_remaining_pages() {
        let source = grid_page();
        let pgs = vec![
            PageInput { kind: source.kind.clone(), text: "Question 8 starts here.".into() },
            PageInput { kind: source.kind.clone(), text: "Question 8 continues.".into() },
            PageInput { kind: source.kind, text: "Question 8 continues on the next page.".into() },
        ];
        let span_pages: Vec<(usize, &PageInput)> = pgs.iter().enumerate().collect();
        let span = doc_map::QuestionSpan {
            number: 8,
            start_page: 0,
            end_page: 2,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(6),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let mock = MockLlm::new(vec![
            ok_chat("{\"items\":["),
            ok_chat(r#"{"items":[{"question_number":8,"content":"First page content. **[2 marks]**","marks":2}]}"#),
            ok_chat(r#"{"items":[{"question_number":8,"content":"Remaining page content. **[4 marks]**","marks":4}]}"#),
        ]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let (built_opt, report) =
            extract_span(&mock, &config(), &span, &span_pages, &cache, &semaphore).await;
        let built = built_opt.expect("split span must build");

        assert!(built.content.contains("First page content."));
        assert!(built.content.contains("Remaining page content."));
        assert_eq!(mock.remaining(), 0);
        assert!(report.timings.iter().any(|t| t.operation == "api_call_reduced"));
        assert!(mock.bodies()[2].to_string().contains("continuation call"));
    }

    #[test]
    fn composite_visual_options_union_boxes_and_collapse_option_text() {
        let mut item = AiQuestion {
            content: Some(
                "Which graph is correct?\nA)\n[DIAGRAM_PLACEHOLDER]\nB)\n[DIAGRAM_PLACEHOLDER]\nC)\n[DIAGRAM_PLACEHOLDER]\nD)\n[DIAGRAM_PLACEHOLDER]"
                    .into(),
            ),
            visual_options: Some("composite_visual_options".into()),
            diagram_bboxes: Some(vec![
                vec![0.10, 0.20, 0.20, 0.15],
                vec![0.40, 0.20, 0.20, 0.15],
                vec![0.10, 0.55, 0.20, 0.15],
                vec![0.40, 0.55, 0.20, 0.15],
            ]),
            bbox_page_indexes: Some(vec![
                serde_json::json!(0),
                serde_json::json!(0),
                serde_json::json!(0),
                serde_json::json!(0),
            ]),
            ..Default::default()
        };

        normalize_composite_visual_options(&mut item);

        assert_eq!(item.diagram_bboxes.as_ref().unwrap().len(), 1);
        assert_eq!(item.bbox_page_indexes.as_ref().unwrap().len(), 1);
        assert_eq!(item.content.unwrap(), "Which graph is correct?\n[DIAGRAM_PLACEHOLDER]");
        assert_eq!(item.diagram_kinds.unwrap(), vec!["composite_visual_options"]);
    }

    #[tokio::test]
    async fn bad_boxes_pruned_deterministically_after_budget_spent() {
        let pgs = vec![grid_page()];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(6),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let heavy_boxing = r#"{"items":[{"question_number":30,"content":"Complete the flow chart below. [DIAGRAM_PLACEHOLDER] **[6 marks]**","marks":6,"topics":["Proof"],"module":"A","diagram_bboxes":[[0.02,0.02,0.93,0.93]],"bbox_page_indexes":[0]}]}"#;
        // Model never learns: every attempt comes back with the same bad box.
        let mock = MockLlm::new(vec![
            ok_chat(heavy_boxing),
            ok_chat(heavy_boxing),
            ok_chat(heavy_boxing),
        ]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let (built_opt, report) =
            extract_span(&mock, &config(), &span, &span_pages, &cache, &semaphore).await;
        let built = built_opt.expect("transcription must survive even when boxes never pass");

        assert!(
            !built.content.contains("[DIAGRAM_PLACEHOLDER]"),
            "no dangling tags"
        );
        assert!(built.content.contains("Complete the flow chart below."));
        assert!(
            report
                .anomalies
                .iter()
                .any(|a| a.contains("dropped 1 invalid diagram box")),
            "the drop must be on the record: {:?}",
            report.anomalies
        );
        assert!(report.crop_rejections >= 1, "every drop counted");
    }

    #[test]
    fn save_diagram_dedupes_identical_crops() {
        let chart = chart_page();
        let dir = std::env::temp_dir().join(format!("mm_dedupe_{}", uuid::Uuid::new_v4()));
        let mut cfg = config();
        cfg.diagrams_dir = Some(dir.clone());
        let mut report = ImportReport::default();
        let mut saved: Vec<([u8; 64], String)> = Vec::new();
        let cache = crate::pdf_render::PageRenderCache::new(PAGE_RENDER_CACHE_CAPACITY);

        let l1 = save_diagram(
            0,
            chart.get_b64().map(String::as_str),
            &[0.02, 0.05, 0.90, 0.82],
            &cfg,
            &cache,
            &mut saved,
            &mut report,
            false,
            false,
        )
        .expect("first crop saves");
        let l2 = save_diagram(
            0,
            chart.get_b64().map(String::as_str),
            &[0.03, 0.06, 0.88, 0.80],
            &cfg,
            &cache,
            &mut saved,
            &mut report,
            false,
            false,
        )
        .expect("duplicate crop resolves to the same link");

        assert_eq!(l1, l2, "same figure → same file");
        assert_eq!(report.diagrams_saved, 1, "exactly one PNG written");
        assert_eq!(report.diagrams_deduped, 1, "duplicate counted");

        // And an empty answer grid never reaches disk at all.
        let grid = grid_page();
        let g = save_diagram(
            0,
            grid.get_b64().map(String::as_str),
            &[0.02, 0.02, 0.93, 0.93],
            &cfg,
            &cache,
            &mut saved,
            &mut report,
            false,
            false,
        );
        assert!(g.is_none(), "answer grid rejected at save");
        assert!(report.crop_rejections >= 1);
        assert_eq!(report.diagrams_saved, 1, "still exactly one PNG written");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn looks_like_new_question_detects_part_reset_and_bold_headings() {
        // Continuation: still on part (b) advancing to (c) — NOT a new question.
        assert!(
            !looks_like_new_question(
                "(a) One thing. **[2 marks]**\n\n(b) Another thing. **[3 marks]**",
                "(c) Final part. **[2 marks]**",
            ),
            "advancing (a)→(b)→(c) is a continuation"
        );

        // Part reset: previous already reached (b), new starts (a) → new question.
        assert!(
            looks_like_new_question(
                "(a) First. **[1 mark]**\n\n(b) Second. **[1 mark]**",
                "(a) Reset to a. **[1 mark]**",
            ),
            "(a) after (b) must fire new-question heuristic"
        );

        // Bold heading at the start.
        assert!(
            looks_like_new_question(
                "previous content here **[3 marks]**",
                "**5.** Give two reasons. **[2 marks]**",
            ),
            "bold heading indicates a new question"
        );

        // Plain "5." at line start.
        assert!(
            looks_like_new_question(
                "previous content **[2 marks]**",
                "5. Start of question five. **[2 marks]**",
            ),
            "leading number+dash indicates a new question"
        );

        // New question starting with "Q3" prefix.
        assert!(
            looks_like_new_question(
                "previous content",
                "Q3) Transcribe this question. **[4 marks]**",
            ),
            "Q-prefix heading indicates a new question"
        );
    }
}
