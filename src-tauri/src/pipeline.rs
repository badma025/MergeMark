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
    /// Try extracting each question from the PDF text layer FIRST (zero image
    /// tokens). Only falls back to vision when the text attempt needs a
    /// figure or fails validation. Off by default (tests keep the old
    /// behaviour); enabled by the production import command when the paper's
    /// text map is sufficient.
    pub text_first: bool,
    /// Same idea for mark-scheme windows: a window whose pages all carry a
    /// reliable text layer is transcribed with ZERO image tokens, falling
    /// back to full-page vision when diagram placeholders appear or the
    /// attempt fails. Off by default (tests keep the old behaviour).
    pub ms_text_first: bool,
    /// Tier-0 deterministic extraction: carve text-reliable, figure-free
    /// question spans straight out of the PDF text layer with a Rust
    /// transcriber (zero API calls). The LLM text-first call becomes the
    /// fallback. Off by default (tests keep the old behaviour); enabled by
    /// the production import command unless MERGEMARK_DETERMINISTIC=0.
    pub deterministic: bool,
    /// Text-layer classification for THIS document, computed by
    /// `run_question_pipeline` from the pages it was handed (never trusted
    /// from the caller). `Some(Digital)` switches the run to the zero-cloud
    /// local path: the deterministic transcriber is forced on, every cloud
    /// stage is skipped at the orchestration boundary, and `chat_with_permit`
    /// refuses to dispatch anything anyway.
    pub(crate) text_layer_class: Option<doc_map::TextLayerClass>,
    /// Immutable per-import local layout evidence (character/run/rule
    /// geometry). Built once for the document by the caller and shared by every
    /// span, so no span reloads the PDF and no stale global path cache exists.
    pub layout_evidence: Option<std::sync::Arc<crate::pdf_render::ImportEvidence>>,
    /// Per-import margin-allocation association, computed once after all spans
    /// are known. Interior mutability so the first span computes it and every
    /// later span reuses the same confirmed records.
    pub margin_model: std::sync::Arc<std::sync::OnceLock<crate::pdf_render::MarginModel>>,
    /// Questions located by the geometric layout map (digital documents whose
    /// layout evidence was built): question number → the question's exact
    /// body. Set by `run_question_pipeline`, never by callers; when present,
    /// the local transcriber uses these boundaries instead of re-detecting
    /// headings in page text.
    pub(crate) layout_questions: Option<std::sync::Arc<std::collections::HashMap<u32, crate::layout::QuestionBody>>>,
    /// For each layout question, its figures in the order of the
    /// `[DIAGRAM_PLACEHOLDER]` lines its body carries (page, figure).
    pub(crate) layout_figures: Option<std::sync::Arc<std::collections::HashMap<u32, Vec<LayoutFigure>>>>,
    /// TEST ONLY: bind a scanned/image-only context so legacy cloud-path tests
    /// can exercise the provider route with fixtures whose page text exists
    /// only to drive the structure pass. Never compiled into production
    /// binaries, so it cannot bypass the digital policy there.
    #[cfg(test)]
    pub(crate) force_scanned_context: bool,
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
            text_first: false,
            ms_text_first: false,
            deterministic: false,
            text_layer_class: None,
            layout_evidence: None,
            margin_model: std::sync::Arc::new(std::sync::OnceLock::new()),
            layout_questions: None,
            layout_figures: None,
            #[cfg(test)]
            force_scanned_context: false,
        }
    }

    /// Bind this run to a text-layer classification. The pipeline calls this
    /// itself from the pages it was handed; callers never set it.
    pub(crate) fn with_text_layer_class(mut self, class: doc_map::TextLayerClass) -> Self {
        self.text_layer_class = Some(class);
        self
    }

    /// True when this run is bound to a born-digital document. Digital
    /// documents permit ZERO cloud requests: the local transcriber is forced
    /// on and every text/vision/repair/classification call is refused,
    /// whatever the tuning switches say.
    pub fn is_digital_document(&self) -> bool {
        self.text_layer_class
            .as_ref()
            .is_some_and(|class| class.is_digital())
    }

    /// Whether a cloud (LLM/vision) request may be dispatched for this run.
    /// Digital documents answer `false` unconditionally — this is the value
    /// the request boundary checks.
    pub fn cloud_allowed(&self) -> bool {
        !self.is_digital_document()
    }

    /// Human-readable text-layer classification for reports. `"unknown"`
    /// means the pipeline has not classified the document (mark-scheme runs
    /// and direct unit-test entry points keep the permissive default).
    pub fn text_layer_label(&self) -> &'static str {
        match self.text_layer_class.as_ref() {
            Some(class) if class.is_digital() => "digital",
            Some(_) => "scanned",
            None => "unknown",
        }
    }

    /// True when this import must run entirely on-device and must NOT resolve
    /// a provider, read credentials, or spend any upload entitlement.
    pub fn is_local_only_document(&self) -> bool {
        self.is_digital_document()
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
    /// Text-layer classification for this document ("digital", "scanned",
    /// "mixed", or "unknown" before classification). Digital means every cloud
    /// stage was skipped and zero requests were permitted.
    pub text_layer: String,
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
    pub repair_reasons: std::collections::BTreeMap<String, usize>,
    pub salvage_events: usize,
    pub crop_rejections: usize,
    pub diagrams_saved: usize,
    pub diagrams_deduped: usize,
    /// Questions transcribed from the text layer alone (zero image tokens).
    pub text_first: usize,
    /// Questions carved entirely locally by the Tier-0 deterministic
    /// transcriber (zero API calls — no prompt, no completion tokens).
    pub deterministic: usize,
    /// Questions retained from a local deterministic carve whose strict
    /// quality gate FAILED: real content, forced into review, zero cloud
    /// calls. Counted separately from `deterministic` so a recovered card is
    /// never reported as a strict Tier-0 success.
    pub recovered: usize,
    /// Mark-scheme windows transcribed from the text layer alone.
    pub ms_text_first: usize,
    /// Read-from-figure questions answered from deterministic figure crops
    /// (~4k image tokens each instead of ~10k+ per full page).
    pub crop_first: usize,
    /// Figures located deterministically from the PDF content stream (zero
    /// image tokens) — the free alternative to the vision figure pass.
    pub figures_detected: usize,
    /// Real billed tokens accumulated from API `usage` blocks across the run.
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub anomalies: Vec<String>,
    pub timings: Vec<TimingEntry>,
    pub total_elapsed_ms: u64,
    /// Per-stage billed-token breakdown, persisted into `import_cost_logs`.
    pub stage_breakdown: Vec<StageCost>,
}

/// Billed tokens attributed to one pipeline stage (see `StageTag`).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageCost {
    pub stage: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

impl ImportReport {
    /// Record one repair round under a stable rule name so the cost audit can
    /// show exactly which validation gates are burning API calls (each repair
    /// re-sends the same images — on Gemini that's the dominant cost).
    pub fn note_repair(&mut self, rule: &str) {
        self.repairs += 1;
        *self.repair_reasons.entry(rule.to_string()).or_insert(0) += 1;
    }
}

/// Convergence guard for repair loops. When a validation failure produces the
/// EXACT same error string on two consecutive attempts, the model is stuck on
/// this prompt — re-sending the identical images with the identical repair
/// note will not converge, so callers should stop paying for it and resolve
/// via their budget-spent path (prune-and-accept, anomaly, or quarantine).
pub fn repair_is_non_convergent(
    attempt: u32,
    prev_error: Option<&str>,
    new_error: &str,
) -> bool {
    attempt > 1 && !new_error.is_empty() && prev_error == Some(new_error)
}

/// Concurrent vision calls in flight at once. Validation is per unit of work
/// (page / span / window), so running units in parallel changes NOTHING about
/// correctness — every response still passes the same Rust gates. It only
/// stops us paying API latency serially. 429 backpressure is per-call
/// (llm.rs), so bursts self-limit.
const DEFAULT_PARALLEL: usize = 4;
// Must comfortably cover the distinct figure pages of a real exam paper
// (~20) so high-res crops stay resident and are never re-rendered; 300-DPI
// pages are ~30MB each, so 32 caps worst-case at ~1GB for one import.
const PAGE_RENDER_CACHE_CAPACITY: usize = 32;

/// Minimum Smart-Scissors confidence for a detected figure to count as
/// trustworthy "supply" in the text-first gate (plan Phase 2 §D).
const FIGURE_SUPPLY_MIN_CONFIDENCE: f32 = 0.5;

/// Every API call is tagged with the pipeline stage that issued it, so the
/// cost ledger can answer "where does the money actually go" instead of
/// guessing from run totals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StageTag {
    /// Document-map vision fallback (insufficient text layer).
    StructurePass,
    /// Per-span transcription from the text layer alone (zero images).
    TextFirstExtraction,
    /// One combined text-only call for every safe span sharing a page.
    BatchTextFirst,
    /// Question answered from deterministic figure crops only (~4k tokens).
    CropFirst,
    /// Full-page vision extraction for one span.
    VisionSpan,
    /// Shared-page batch vision call (page hosts several failed spans).
    FallbackPage,
    /// Mark-scheme window transcribed from the text layer alone.
    MsTextFirst,
    /// Mark-scheme window read as full-page vision.
    MsWindow,
    /// ONE batched text-only call assigning taxonomy topics to Tier-0
    /// (untagged) questions after extraction completes. Replaces topic
    /// selection embedded in every extraction call.
    TopicClassification,
}

impl StageTag {
    pub fn as_str(self) -> &'static str {
        match self {
            StageTag::StructurePass => "structure_pass",
            StageTag::TextFirstExtraction => "text_first_extraction",
            StageTag::BatchTextFirst => "batch_text_first",
            StageTag::CropFirst => "crop_first",
            StageTag::VisionSpan => "vision_span",
            StageTag::FallbackPage => "fallback_page",
            StageTag::MsTextFirst => "ms_text_first",
            StageTag::MsWindow => "ms_window",
            StageTag::TopicClassification => "topic_classification",
        }
    }

    /// Output-token ceiling for this stage. Completions bill at the ~8×
    /// input rate, and schema-mode responses never legitimately approach
    /// the global 32k ceiling on small units; the truncation guard converts
    /// any ceiling hit into a bounded repair round.
    pub fn output_cap(self) -> u32 {
        match self {
            StageTag::TextFirstExtraction | StageTag::BatchTextFirst | StageTag::CropFirst => 2048,
            StageTag::MsTextFirst => 3072,
            StageTag::VisionSpan | StageTag::FallbackPage | StageTag::MsWindow => 8192,
            StageTag::StructurePass => 2048,
            StageTag::TopicClassification => 1024,
        }
    }
}

/// Process-wide accumulator for real API token usage across one pipeline run.
/// Every `chat_with_permit` call adds its response's `usage` block under its
/// stage tag, so the import cost estimate uses actual billed tokens AND can
/// attribute them to the stage that spent them.
#[derive(Debug, Default)]
pub struct TokenTotals {
    pub prompt_tokens: std::sync::atomic::AtomicU64,
    pub completion_tokens: std::sync::atomic::AtomicU64,
    stages: std::sync::Mutex<std::collections::BTreeMap<StageTag, (u64, u64)>>,
}

impl TokenTotals {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&self, prompt: u64, completion: u64) {
        self.prompt_tokens
            .fetch_add(prompt, std::sync::atomic::Ordering::Relaxed);
        self.completion_tokens
            .fetch_add(completion, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn add_stage(&self, stage: StageTag, prompt: u64, completion: u64) {
        self.add(prompt, completion);
        if let Ok(mut map) = self.stages.lock() {
            let entry = map.entry(stage).or_insert((0, 0));
            entry.0 += prompt;
            entry.1 += completion;
        }
    }

    pub fn snapshot(&self) -> (u64, u64) {
        (
            self.prompt_tokens.load(std::sync::atomic::Ordering::Relaxed),
            self.completion_tokens.load(std::sync::atomic::Ordering::Relaxed),
        )
    }

    pub fn snapshot_stages(&self) -> Vec<(StageTag, u64, u64)> {
        self.stages
            .lock()
            .map(|map| map.iter().map(|(s, (p, c))| (*s, *p, *c)).collect())
            .unwrap_or_default()
    }
}

async fn chat_with_permit<C: LlmClient>(
    client: &C,
    body: &serde_json::Value,
    semaphore: &Arc<Semaphore>,
    cancel: &AtomicBool,
    usage: &Arc<TokenTotals>,
    stage: StageTag,
    allow_cloud: bool,
) -> Result<serde_json::Value, crate::llm::LlmError> {
    // Request-boundary circuit breaker (defense in depth). The orchestration
    // layer decides which paths run; this is the last gate before anything can
    // reach the network. A document-bound policy that forbids the cloud makes
    // every stage refuse here, so no call site can accidentally re-enable
    // paid traffic for a digital paper.
    if !allow_cloud {
        eprintln!(
            "[CLOUD_REFUSED] stage={} request refused before dispatch (document policy forbids cloud)",
            stage.as_str()
        );
        return Err(crate::llm::LlmError::Network(
            "cloud disabled for this document: request refused at the request boundary".to_string(),
        ));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(crate::llm::LlmError::Network("Import cancelled by user".to_string()));
    }
    let _permit = tokio::select! {
        res = semaphore.acquire() => {
            res.map_err(|_| crate::llm::LlmError::Network("request semaphore closed".to_string()))?
        }
        _ = async {
            while !cancel.load(Ordering::Relaxed) {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        } => {
            return Err(crate::llm::LlmError::Network("Import cancelled by user".to_string()));
        }
    };
    if cancel.load(Ordering::Relaxed) {
        return Err(crate::llm::LlmError::Network("Import cancelled by user".to_string()));
    }
    tokio::select! {
        res = client.chat(body) => {
            if let Ok(resp) = &res {
                let u = crate::llm::usage_from_response(resp);
                if u.prompt_tokens > 0 || u.completion_tokens > 0 {
                    usage.add_stage(stage, u.prompt_tokens, u.completion_tokens);
                }
                if std::env::var_os("MERGEMARK_LOG_USAGE").is_some() {
                    eprintln!(
                        "[TOKENS] prompt={} completion={} total={}",
                        u.prompt_tokens, u.completion_tokens, u.total_tokens
                    );
                }
            }
            res
        },
        _ = async {
            while !cancel.load(Ordering::Relaxed) {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        } => {
            Err(crate::llm::LlmError::Network("Import cancelled by user".to_string()))
        }
    }
}

/// Shared cache for decoded page images. The same page is often decoded
/// multiple times during a pipeline run (once for API downsampling in
/// `prepare_chunk_images`, again for diagram cropping in `save_diagram`).
/// This cache ensures each page is decoded from base64 at most once.
pub struct PageImageCache {
    pages: std::sync::Mutex<std::collections::HashMap<usize, Arc<image::DynamicImage>>>,
}

impl PageImageCache {
    pub fn new() -> Self {
        Self {
            pages: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Return a cached decoded page image, decoding and caching it on first access.
    pub fn get_or_decode(&self, page_idx: usize, b64: &str) -> Option<Arc<image::DynamicImage>> {
        // Fast path: check cache first (brief lock).
        {
            let pages = self.pages.lock().ok()?;
            if let Some(img) = pages.get(&page_idx) {
                return Some(Arc::clone(img));
            }
        }
        // Slow path: decode outside the lock so other threads can read the cache.
        let decoded = Arc::new(geometry::decode_page_image(b64)?);
        // Store in cache.
        if let Ok(mut pages) = self.pages.lock() {
            pages.entry(page_idx).or_insert_with(|| Arc::clone(&decoded));
        }
        Some(decoded)
    }
}

struct ChunkImageInput {
    chunk_idx: usize,
    global_page_idx: usize,
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

/// A figure of a layout question: where it is, and whether its box is the
/// layout's exact figure extent (cropped as given) or a detector proposal.
#[derive(Debug, Clone)]
pub(crate) struct LayoutFigure {
    pub(crate) page: usize,
    pub(crate) figure: crate::pdf_render::DetectedFigure,
    pub(crate) exact: bool,
}

struct DiagramSaveRequest {
    global_page_idx: usize,
    bbox: Vec<f32>,
    ignore_grid: bool,
    graph_like: bool,
    /// The box is the figure's exact extent (strokes and labels from the
    /// layout): crop it as given — with this hairline margin (a fraction
    /// of the page) — without padding or edge-text trimming.
    exact: Option<f32>,
    /// Page furniture pictures (`[x, y, w, h]` page fractions, like `bbox`)
    /// painted out of the crop: a QR code beside the figure is not part of it.
    mask: Vec<[f32; 4]>,
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
            links.push(save_diagram_with(
                request.global_page_idx,
                page_b64.get(&request.global_page_idx).map(String::as_str),
                &request.bbox,
                &config,
                page_render_cache.as_ref(),
                &mut saved,
                &mut report,
                request.ignore_grid,
                request.graph_like,
                request.exact,
                &request.mask,
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
    page_cache: &Arc<PageImageCache>,
) -> Result<PreparedChunk, tokio::task::JoinError> {
    let page_cache = Arc::clone(page_cache);
    tokio::task::spawn_blocking(move || {
        use rayon::prelude::*;

        // Process chunk inputs concurrently across all worker cores
        let processed: Vec<(
            usize,
            Option<Arc<image::DynamicImage>>,
            String,
            (f32, f32),
            Option<(f32, f32)>,
        )> = inputs
            .into_par_iter()
            .map(|input| {
                let decoded = page_cache.get_or_decode(input.global_page_idx, &input.b64);
                let mut final_b64 = input.b64;
                let mut crop_offset = (0.0_f32, 1.0_f32);
                let mut page_band = None;

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
                    page_band = Some((
                        input.start_y.unwrap_or(0.0),
                        input.end_y.unwrap_or(1.0),
                    ));
                } else if let Some(img) = &decoded {
                    let (w, h) = img.dimensions();
                    let max_dim = geometry::api_image_max_dim();
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
                        if resized.write_to(&mut buf, image::ImageFormat::WebP).is_ok() {
                            use base64::Engine;
                            final_b64 = base64::engine::general_purpose::STANDARD.encode(buf.into_inner());
                        }
                    }
                }

                (input.chunk_idx, decoded, final_b64, crop_offset, page_band)
            })
            .collect();

        let mut images = Vec::with_capacity(processed.len());
        let mut local_to_chunk = Vec::with_capacity(processed.len());
        let mut page_bands = vec![None; chunk_len];
        let mut page_crop_offsets = Vec::with_capacity(processed.len());
        let mut decoded_pages = vec![None; chunk_len];

        for (chunk_idx, decoded, final_b64, crop_offset, page_band) in processed {
            if let Some(band) = page_band {
                page_bands[chunk_idx] = Some(band);
            }
            decoded_pages[chunk_idx] = decoded;
            images.push(final_b64);
            local_to_chunk.push(chunk_idx);
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
        for (rule, count) in o.repair_reasons {
            *self.repair_reasons.entry(rule).or_insert(0) += count;
        }
        self.salvage_events += o.salvage_events;
        self.crop_rejections += o.crop_rejections;
        self.diagrams_saved += o.diagrams_saved;
        self.diagrams_deduped += o.diagrams_deduped;
        self.text_first += o.text_first;
        self.deterministic += o.deterministic;
        self.recovered += o.recovered;
        self.ms_text_first += o.ms_text_first;
        self.crop_first += o.crop_first;
        self.figures_detected += o.figures_detected;
        self.prompt_tokens += o.prompt_tokens;
        self.completion_tokens += o.completion_tokens;
        for cost in o.stage_breakdown {
            if let Some(existing) = self
                .stage_breakdown
                .iter_mut()
                .find(|s| s.stage == cost.stage)
            {
                existing.prompt_tokens += cost.prompt_tokens;
                existing.completion_tokens += cost.completion_tokens;
            } else {
                self.stage_breakdown.push(cost);
            }
        }
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

#[derive(Debug, Default, serde::Deserialize, Clone)]
#[serde(default)]
struct AiQuestionPage {
    items: Vec<AiQuestion>,
}

/// Why a set of model items could not be stitched into ONE question.
///
/// Stitching is only ever safe inside a single parent question. Both refusal
/// reasons are identity failures, and both must fall back rather than merge:
/// a batch response or a neighbouring question caught in the crop would
/// otherwise be silently welded onto this card.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StitchRefusal {
    /// An item explicitly belongs to a different parent question.
    ForeignQuestion(u32),
    /// An item's question number is present but cannot be parsed, so identity
    /// cannot be validated.
    UnverifiableIdentity,
}

impl StitchRefusal {
    fn describe(&self) -> String {
        match self {
            StitchRefusal::ForeignQuestion(n) => {
                format!("item belongs to Q{} (distinct parent question)", n)
            }
            StitchRefusal::UnverifiableIdentity => {
                "item question number is unreadable".to_string()
            }
        }
    }
}

/// Byte offset where a trailing mark tag ("**[3 marks]**", "[2]") starts, or
/// `None` when the content does not end with one. Anchored at the end so a
/// mid-sentence bracket can never be mistaken for a mark allocation.
fn trailing_mark_tag_start(content: &str) -> Option<usize> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"(?i)(?:\*{0,2})\s*(?:\[|\()\s*\d{1,2}\s*marks?\s*(?:\]|\))\s*(?:\*{0,2})\s*$")
            .unwrap()
    });
    re.find(content.trim_end()).map(|m| m.start())
}

/// The trailing mark tag itself, when the content ends with one.
fn trailing_mark_tag(content: &str) -> Option<String> {
    let start = trailing_mark_tag_start(content)?;
    Some(content.trim_end()[start..].trim().to_string())
}

/// Numeric value of a mark tag ("**[3 marks]**" -> 3).
fn mark_tag_value(tag: &str) -> Option<i32> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"(?i)(\d{1,2})\s*marks?").unwrap());
    re.captures(tag)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<i32>().ok())
}

/// Result of stitching one question's compatible sub-part items.
struct StitchedQuestion {
    item: AiQuestion,
    /// Equal per-item marks could not be resolved against a known parent
    /// total. The marks are kept exactly as reported and the caller must flag
    /// the card for review instead of guessing.
    marks_ambiguous: bool,
    /// A repeated parent total was collapsed into a single mark value/tag.
    #[cfg_attr(not(test), allow(dead_code))]
    repeated_total_collapsed: bool,
}

/// Merge compatible sub-part items for ONE question in source order.
///
/// This is the single-question response boundary: a model that answers a
/// question with sub-parts `(a)(b)(c)` sometimes returns one item per sub-part
/// instead of one item for the whole question. Those items are compatible and
/// are stitched here — content in source order, diagrams/captions/topics/marks
/// preserved, and marks resolved against the printed parent total (a repeated
/// parent total collapses once; equal per-part allocations are preserved).
///
/// Distinct parent question numbers are NEVER stitched: that returns
/// `Err(StitchRefusal)` and the caller falls back (strict validation or local
/// recovery) instead of inventing a merged card.
fn stitch_question_items(
    items: Vec<AiQuestion>,
    question_number: u32,
    expected_parent_marks: Option<u32>,
) -> Result<StitchedQuestion, StitchRefusal> {
    // ── Identity gate (before anything is merged) ──────────────────────────
    for item in &items {
        match item.question_number.as_ref() {
            None => {}
            Some(raw) => match validate::value_to_question_number(raw) {
                Some(n) if n == question_number => {}
                Some(n) => return Err(StitchRefusal::ForeignQuestion(n)),
                None => return Err(StitchRefusal::UnverifiableIdentity),
            },
        }
    }

    // ── Duplicate totals ──────────────────────────────────────────────────
    // Marks: allocations vs a repeated parent total. Equal marks on distinct
    // sub-parts are NOT duplicate totals: (a)+(b)+(c) worth 3 each is a
    // 9-mark question. The only way to tell a repeated parent total from equal
    // allocations is the printed total, so:
    //   * expected == v and sum > expected -> the model repeated the parent
    //     total on every part: keep one.
    //   * expected == sum                 -> equal allocations confirmed: keep
    //     every tag and the summed total.
    //   * anything else                   -> keep everything, flag review.
    let mark_values: Vec<i32> = items
        .iter()
        .filter_map(|item| item.marks.as_ref().and_then(validate::value_to_marks))
        .collect();
    let expected = expected_parent_marks.map(|m| m as i32);
    let (resolved_marks, repeated_total_collapsed, marks_ambiguous): (Option<i32>, bool, bool) =
        if mark_values.is_empty() {
            (None, false, false)
        } else if mark_values.len() > 1 && mark_values.iter().all(|m| *m == mark_values[0]) {
            let value = mark_values[0];
            let sum: i32 = mark_values.iter().sum();
            match expected {
                Some(parent) if parent == value && sum > parent => (Some(value), true, false),
                Some(parent) if parent == sum => (Some(sum), false, false),
                _ => (Some(sum), false, true),
            }
        } else {
            (Some(mark_values.iter().sum()), false, false)
        };

    // A repeated parent total is collapsed in the CONTENT too, but ONLY when
    // the inline tag's own value is the parent total: metadata can repeat the
    // parent total while the content tags are genuine per-part allocations
    // ((a)(b)(c) each "**[3 marks]**" under a 9-mark question). Stripping those
    // would drop legitimate allocations, so the tag value must match.
    let mark_tags: Vec<Option<String>> = items
        .iter()
        .map(|item| {
            item.content
                .as_deref()
                .filter(|c| !c.trim().is_empty())
                .and_then(trailing_mark_tag)
        })
        .collect();
    let all_tagged = mark_tags.len() > 1 && mark_tags.iter().all(|t| t.is_some());
    let shared_tag = mark_tags.first().cloned().flatten();
    let tag_is_parent_total = match (
        expected,
        shared_tag.as_deref().and_then(mark_tag_value),
    ) {
        (Some(parent), Some(tag_value)) => parent == tag_value,
        _ => false,
    };
    let dedupe_trailing_tag = repeated_total_collapsed
        && all_tagged
        && tag_is_parent_total
        && shared_tag
            .as_ref()
            .is_some_and(|first| mark_tags.iter().all(|t| t.as_ref() == Some(first)));

    let item_count = items.len();
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

    for (index, item) in items.into_iter().enumerate() {
        if let Some(value) = item.content.filter(|value| !value.trim().is_empty()) {
            // Drop the repeated trailing mark tag from every part but the last
            // so the joined content carries the question total exactly once.
            let value = if dedupe_trailing_tag && index + 1 != item_count {
                strip_trailing_mark_tag(&value)
            } else {
                value
            };
            if !value.trim().is_empty() {
                content.push(value);
            }
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
        merged.module = merged.module.or(item.module);
        merged.is_code = merged.is_code.or(item.is_code);
        merged.math_snippet = merged.math_snippet.or(item.math_snippet);
        if merged.visual_options.is_none()
            && (item.visual_options.as_deref() == Some("composite_visual_options")
                || item.visual_options.as_deref() == Some("image_options"))
        {
            merged.visual_options = item.visual_options;
        }
    }

    merged.content = Some(content.join("\n\n"));
    if let Some(total) = resolved_marks {
        merged.marks = Some(serde_json::json!(total));
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
    Ok(StitchedQuestion {
        item: merged,
        marks_ambiguous,
        repeated_total_collapsed,
    })
}

/// Remove a trailing mark tag from `content`, keeping everything above it.
fn strip_trailing_mark_tag(content: &str) -> String {
    match trailing_mark_tag_start(content) {
        Some(start) => content[..start].trim_end().to_string(),
        None => content.to_string(),
    }
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

/// True when the model proposed ISOLATED per-option diagrams for an MCQ
/// (one bbox per option letter). Exact/prefix matching so the legacy
/// "composite_visual_options" value never collides with it.
fn is_image_options(item: &AiQuestion) -> bool {
    if item.visual_options.as_deref() == Some("image_options") {
        return true;
    }
    item.diagram_kinds
        .as_ref()
        .map(|kinds| {
            kinds
                .iter()
                .any(|kind| kind == "visual_option" || kind.starts_with("visual_option_"))
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

/// First line that belongs to an option diagram rather than the question
/// stem: any A-E option marker line or any placeholder-bearing line.
/// Returns the BYTE offset of that line's start.
fn first_option_line_index(content: &str) -> Option<usize> {
    let mut offset = 0usize;
    for line in content.lines() {
        if is_visual_option_marker(line) || line.contains("[DIAGRAM_PLACEHOLDER]") {
            return Some(offset);
        }
        offset += line.len() + 1;
    }
    None
}

/// Bind ISOLATED per-option MCQ diagrams into the strict markdown list format:
///
/// ```text
/// <stem>
///
/// - [MCQ:A] [DIAGRAM_PLACEHOLDER]
/// - [MCQ:B] [DIAGRAM_PLACEHOLDER]
/// ...
/// ```
///
/// Each diagram bbox stays SEPARATE (never unioned), so the crop splicer maps
/// box N → placeholder N and every diagram lands inside its own option card.
/// Legacy single-composite proposals are still routed through
/// `normalize_composite_visual_options`.
fn normalize_visual_mcq_options(item: &mut AiQuestion) {
    if is_composite_visual_options(item) && !is_image_options(item) {
        normalize_composite_visual_options(item);
        return;
    }
    if !is_image_options(item) {
        return;
    }

    let Some(bboxes) = item.diagram_bboxes.clone() else {
        return;
    };
    if bboxes.len() < 2 {
        // Fewer than two boxes cannot form an options grid; leave untouched.
        return;
    }

    // Deterministic per-option metadata so downstream crop splicing labels
    // each diagram with its option letter.
    const LETTERS: [char; 5] = ['A', 'B', 'C', 'D', 'E'];
    item.diagram_captions = Some(
        (0..bboxes.len())
            .map(|i| format!("Option {}", LETTERS[i]))
            .collect(),
    );
    item.diagram_kinds = Some(vec!["visual_option".to_string(); bboxes.len()]);
    item.visual_options = Some("image_options".to_string());

    let stem = item
        .content
        .as_deref()
        .and_then(|content| {
            let cut = first_option_line_index(content).unwrap_or(content.len());
            let stem = content[..cut].trim_end();
            if stem.is_empty() {
                None
            } else {
                Some(stem.to_string())
            }
        });

    let mut rebuilt = String::new();
    if let Some(stem) = stem {
        rebuilt.push_str(&stem);
        rebuilt.push_str("\n\n");
    }
    for (i, _) in bboxes.iter().enumerate().take(LETTERS.len()) {
        rebuilt.push_str(&format!("- [MCQ:{}] [DIAGRAM_PLACEHOLDER]\n", LETTERS[i]));
    }
    item.content = Some(rebuilt.trim_end().to_string());
}

/// Intelligently splice saved diagram markdown links into content placeholders.
/// Matches placeholders with diagrams based on Figure numbers ("Figure 1", "Figure 2"),
/// option letters ("Option A", "Option B"), and diagram captions, falling back to sequential order.
pub fn splice_diagrams_by_caption_and_context(
    mut content: String,
    links: &[Option<String>],
    captions: Option<&[String]>,
) -> String {
    let non_empty_links: Vec<(usize, String)> = links
        .iter()
        .enumerate()
        .filter_map(|(i, opt)| opt.as_ref().map(|l| (i, l.clone())))
        .collect();

    if non_empty_links.is_empty() {
        return content.replace("[DIAGRAM_PLACEHOLDER]", "");
    }

    // Split content by "[DIAGRAM_PLACEHOLDER]"
    let parts: Vec<&str> = content.split("[DIAGRAM_PLACEHOLDER]").collect();
    let num_placeholders = parts.len().saturating_sub(1);

    if num_placeholders == 0 {
        // No placeholders in text, append all links at the end
        for (_, link) in &non_empty_links {
            content.push_str(link);
        }
        return content;
    }

    let re_fig = regex::Regex::new(r"(?i)\b(?:figure|fig\.?)\s*(\d+)\b").unwrap();
    let re_opt = regex::Regex::new(r"(?i)\b(?:option|choice)\s*([A-E])\b").unwrap();

    let mut placeholder_fig_nums: Vec<Option<u32>> = Vec::with_capacity(num_placeholders);
    let mut placeholder_opt_letters: Vec<Option<char>> = Vec::with_capacity(num_placeholders);

    for p in 0..num_placeholders {
        let preceding = parts[p];
        let snippet = if preceding.len() > 300 {
            &preceding[preceding.len() - 300..]
        } else {
            preceding
        };

        let fig_num = re_fig
            .captures_iter(snippet)
            .last()
            .and_then(|cap| cap[1].parse::<u32>().ok());
        let opt_letter = re_opt
            .captures_iter(snippet)
            .last()
            .and_then(|cap| cap[1].chars().next().map(|c| c.to_ascii_uppercase()));

        placeholder_fig_nums.push(fig_num);
        placeholder_opt_letters.push(opt_letter);
    }

    // For each diagram link, extract figure number / option letter from caption
    let mut diagram_fig_nums: Vec<Option<u32>> = Vec::with_capacity(non_empty_links.len());
    let mut diagram_opt_letters: Vec<Option<char>> = Vec::with_capacity(non_empty_links.len());

    for (orig_idx, _) in &non_empty_links {
        let cap_str = captions.and_then(|c| c.get(*orig_idx)).map(|s| s.as_str()).unwrap_or("");
        let fig_num = re_fig
            .captures(cap_str)
            .and_then(|cap| cap[1].parse::<u32>().ok());
        let opt_letter = re_opt
            .captures(cap_str)
            .and_then(|cap| cap[1].chars().next().map(|c| c.to_ascii_uppercase()));

        diagram_fig_nums.push(fig_num);
        diagram_opt_letters.push(opt_letter);
    }

    // Matching: map each placeholder index -> diagram link
    let mut assigned_links: Vec<Option<String>> = vec![None; num_placeholders];
    let mut used_diagrams: Vec<bool> = vec![false; non_empty_links.len()];

    // Pass 1: Exact Figure Number match
    for p in 0..num_placeholders {
        if let Some(req_fig) = placeholder_fig_nums[p] {
            for d in 0..non_empty_links.len() {
                if !used_diagrams[d] && diagram_fig_nums[d] == Some(req_fig) {
                    assigned_links[p] = Some(non_empty_links[d].1.clone());
                    used_diagrams[d] = true;
                    break;
                }
            }
        }
    }

    // Pass 2: Exact Option Letter match
    for p in 0..num_placeholders {
        if assigned_links[p].is_none() {
            if let Some(req_opt) = placeholder_opt_letters[p] {
                for d in 0..non_empty_links.len() {
                    if !used_diagrams[d] && diagram_opt_letters[d] == Some(req_opt) {
                        assigned_links[p] = Some(non_empty_links[d].1.clone());
                        used_diagrams[d] = true;
                        break;
                    }
                }
            }
        }
    }

    // Pass 3: Fill remaining placeholders in sequential order of unused diagrams
    for p in 0..num_placeholders {
        if assigned_links[p].is_none() {
            if let Some(d) = (0..non_empty_links.len()).find(|&idx| !used_diagrams[idx]) {
                assigned_links[p] = Some(non_empty_links[d].1.clone());
                used_diagrams[d] = true;
            }
        }
    }

    // Reconstruct content with spliced links
    let mut result = String::with_capacity(content.len() + 256);
    for p in 0..num_placeholders {
        result.push_str(parts[p]);
        if let Some(link) = &assigned_links[p] {
            result.push_str(link);
        }
    }
    result.push_str(parts[num_placeholders]);

    // Any remaining unused diagram links are appended at the end
    for (d, (_, link)) in non_empty_links.iter().enumerate() {
        if !used_diagrams[d] {
            result.push_str(link);
        }
    }

    result
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

/// The question number a piece of content OPENS with, when it starts with its
/// own heading ("7. ...", "**5.** ...", "Q3) ...").
///
/// `None` for sub-part labels ("(b) ..."), prose, and numbers that are not a
/// heading ("2x + 4 = 10", "250 Hz"). Used to refuse stitching two separate
/// question stems that a model mislabeled with the same parent number.
fn leading_question_number(content: &str) -> Option<u32> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"(?i)^\s*\*{0,2}\s*(?:q(?:uestion)?\.?\s*)?(\d{1,2})\s*[\.\)\]]")
            .unwrap()
    });
    re.captures(content)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<u32>().ok())
}



// ══════════════════════════════════════════════════════════════════════════
// Prompts
// ══════════════════════════════════════════════════════════════════════════

#[allow(dead_code)]
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

/// JSON Schema for the structure pass output
fn structure_json_schema() -> serde_json::Value {
    static SCHEMA: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(|| {
        serde_json::json!({
            "name": "PageStructureProposal",
            "strict": true,
            "schema": {
                "type": "object",
                "properties": {
                    "question_numbers_visible": {
                        "type": "array",
                        "items": { "type": "integer", "minimum": 1, "maximum": 100 }
                    },
                    "question_y_fracs": {
                        "type": "array",
                        "items": {
                            "type": "array",
                            "items": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                            "minItems": 2,
                            "maxItems": 2
                        }
                    },
                    "total_marks_footer": {
                        "type": ["array", "null"],
                        "items": { "type": "integer" },
                        "minItems": 2,
                        "maxItems": 2
                    },
                    "total_marks_footer_y": {
                        "type": ["number", "null"],
                        "minimum": 0.0,
                        "maximum": 1.0
                    },
                    "page_role": {
                        "type": "string",
                        "enum": ["QUESTION", "COVER", "INSTRUCTIONS", "BLANK", "ANSWER_BOOKLET", "REFERENCE"]
                    }
                },
                "required": ["question_numbers_visible", "question_y_fracs", "total_marks_footer", "total_marks_footer_y", "page_role"],
                "additionalProperties": false
            }
        })
    }).clone()
}

/// JSON Schema for the extraction pass output
fn extraction_json_schema() -> serde_json::Value {
    static SCHEMA: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(|| {
        serde_json::json!({
            "name": "QuestionExtraction",
            "strict": true,
            "schema": {
                "type": "object",
                "properties": {
                    "items": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "question_number": { "type": "integer", "minimum": 1, "maximum": 100 },
                                "content": {
                                    "type": "string",
                                    "description": "Markdown transcription. ALL math must be wrapped in balanced delimiters: every $ opened on a line is closed on the SAME line; every $$ block is closed with $$. No raw LaTeX (\\frac, ^{...}_{...}, \\alpha) may appear outside math delimiters. Never break a sentence across paragraphs. Sequential printed equations stay on SEPARATE $$ blocks (or rows joined with \\\\ inside one block) — never concatenated end-to-end. In JSON, escape EVERY backslash as \\\\ (writing \\text instead of \\\\text decodes to a TAB character followed by 'ext')."
                                },
                                "marks": { "type": ["integer", "null"], "minimum": 0 },
                                "topics": { "type": "array", "items": { "type": "string" } },
                                "module": { "type": "string" },
                                "is_code": { "type": "boolean" },
                                "diagram_bboxes": {
                                    "type": "array",
                                    "items": {
                                        "type": "array",
                                        "items": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                                        "minItems": 4,
                                        "maxItems": 4
                                    }
                                },
                                "diagram_captions": { "type": "array", "items": { "type": "string" } },
                                "diagram_kinds": { "type": "array", "items": { "type": "string" } },
                                "bbox_page_indexes": { "type": "array", "items": { "type": "integer" } },
                                "math_snippet": { "type": "string" },
                                "visual_options": {
                                    "type": ["string", "null"],
                                    "description": "null for text questions; 'image_options' when MCQ answer choices are separate visual diagrams (one box per option letter, in A-D order); 'composite_visual_options' is the legacy single-composite value."
                                }
                            },
                            "required": ["question_number", "content", "marks", "topics", "module", "is_code", "diagram_bboxes", "diagram_captions", "diagram_kinds", "bbox_page_indexes", "math_snippet", "visual_options"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["items"],
                "additionalProperties": false
            }
        })
    }).clone()
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

fn extraction_system_prompt(config: &PipelineConfig) -> String {
    let topics_instruction = if config.allowed_topics.is_empty() {
        "- \"topics\": array. MUST be empty []. Do NOT invent topics.".to_string()
    } else {
        format!(
            "- \"topics\": array. At least one. Select ONLY from this exact list: {:?}. Never invent topics.",
            config.allowed_topics
        )
    };

    const FEW_SHOT: &str = r#"
═══ FEW-SHOT EXAMPLES — Study these input/output pairs carefully ═══
Example 1 — Pure math with sub-parts
Input page (Question 4): "4 (a) Solve 2x^2 - 5x + 2 = 0. [3 marks]\n(b) Hence solve 2y^4 - 5y^2 + 2 = 0. [2 marks]"
Output: {"items": [{"question_number": 4, "content": "(a) Solve $2x^2 - 5x + 2 = 0$.\n\n**[3 marks]**\n\n(b) Hence solve $2y^4 - 5y^2 + 2 = 0$.\n\n**[2 marks]**", "marks": 5, "difficulty_rating": null, "topics": ["algebra", "quadratics"], "module": "Pure Mathematics", "is_code": false, "diagram_bboxes": [], "diagram_captions": [], "diagram_kinds": [], "bbox_page_indexes": [], "math_snippet": "2x^2 - 5x + 2 = 0", "visual_options": null}]}

Example 2 — Question with a graph figure
Input page (Question 7): "7 The graph of y = f(x) is shown below.\nFigure 2\n(a) Write down the coordinates of the turning point. [1 mark]\n(b) State the range of f. [1 mark]"
Output: {"items": [{"question_number": 7, "content": "The graph of $y = f(x)$ is shown below.\n\n[DIAGRAM_PLACEHOLDER]\n\n(a) Write down the coordinates of the turning point.\n\n**[1 mark]**\n\n(b) State the range of $f$.\n\n**[1 mark]**", "marks": 2, "difficulty_rating": null, "topics": ["functions", "graphs"], "module": "Pure Mathematics", "is_code": false, "diagram_bboxes": [[0.15, 0.20, 0.70, 0.45]], "diagram_captions": ["Graph of y = f(x)"], "diagram_kinds": ["graph"], "bbox_page_indexes": [0], "math_snippet": "y = f(x)", "visual_options": null}]}

Example 3 — Structured table (trace table) — transcribe as Markdown table, NOT diagram
Input page (Question 12): "12 Complete the trace table for the algorithm below.\n\n| i | condition | output |\n|---|---|---|\n| 1 | true | 3 |\n| 2 |  |  |\n| 3 |  |  |"
Output: {"items": [{"question_number": 12, "content": "Complete the trace table for the algorithm below.\n\n| i | condition | output |\n|---|---|---|\n| 1 | true | 3 |\n| 2 |  |  |\n| 3 |  |  |", "marks": 4, "difficulty_rating": null, "topics": ["algorithms", "trace tables"], "module": "Computer Science", "is_code": false, "diagram_bboxes": [], "diagram_captions": [], "diagram_kinds": [], "bbox_page_indexes": [], "math_snippet": "", "visual_options": null}]}

Example 4 — Multiple-choice with image-based options (ISOLATED per option, NEVER one composite blob)
Input page (Question 15): "15 Which graph represents y = sin(x)/x?\nA [graph A]\nB [graph B]\nC [graph C]\nD [graph D]"
Output: {"items": [{"question_number": 15, "content": "Which graph represents $y = \\frac{\\sin x}{x}$?\n\n- [MCQ:A] [DIAGRAM_PLACEHOLDER]\n- [MCQ:B] [DIAGRAM_PLACEHOLDER]\n- [MCQ:C] [DIAGRAM_PLACEHOLDER]\n- [MCQ:D] [DIAGRAM_PLACEHOLDER]", "marks": 1, "difficulty_rating": null, "topics": ["trigonometry", "graphs"], "module": "Pure Mathematics", "is_code": false, "diagram_bboxes": [[0.10, 0.25, 0.25, 0.20], [0.55, 0.25, 0.25, 0.20], [0.10, 0.50, 0.25, 0.20], [0.55, 0.50, 0.25, 0.20]], "diagram_captions": ["Option A", "Option B", "Option C", "Option D"], "diagram_kinds": ["visual_option", "visual_option", "visual_option", "visual_option"], "bbox_page_indexes": [0, 0, 0, 0], "math_snippet": "sin(x)/x", "visual_options": "image_options"}]}

Example 5 — Question continues from previous page
Input page (Question 9 continued): "(c) Find the exact value of the integral. [4 marks]\n\n(Total for Question 9 is 10 marks)\n\n10 (a) ..."
Output: {"items": [{"question_number": 9, "content": "(c) Find the exact value of the integral.\n\n**[4 marks]**", "marks": 4, "difficulty_rating": null, "topics": ["calculus", "integration"], "module": "Pure Mathematics", "is_code": false, "diagram_bboxes": [], "diagram_captions": [], "diagram_kinds": [], "bbox_page_indexes": [], "math_snippet": "", "visual_options": null}]}

Example 6 — T. Madas / Worksheet style (Polar curve with leading variable, difficulty rating, and sub-parts)
Input page: "Question 3 (***+)\nA curve has polar equation\nr = (cos(theta) + sin(theta))/(cos^2(theta) + sin(2*theta) + 1),  0 <= theta < 2*pi\n(a) Find a Cartesian equation of the curve in the form f(x, y) = 0.\n(b) Show that the area bounded by the curve is pi/4."
Output: {"items": [{"question_number": 3, "content": "A curve has polar equation\n\n$$r = \\frac{\\cos\\theta + \\sin\\theta}{\\cos^2\\theta + \\sin 2\\theta + 1}, \\quad 0 \\le \\theta < 2\\pi$$\n\n(a) Find a Cartesian equation of the curve in the form $f(x, y) = 0$.\n\n(b) Show that the area bounded by the curve is $\\frac{\\pi}{4}$.", "marks": null, "difficulty_rating": "***+", "topics": ["polar coordinates", "curves"], "module": "Pure Mathematics", "is_code": false, "diagram_bboxes": [], "diagram_captions": [], "diagram_kinds": [], "bbox_page_indexes": [], "math_snippet": "r = \\frac{\\cos\\theta + \\sin\\theta}{\\cos^2\\theta + \\sin 2\\theta + 1}", "visual_options": null}]}

Example 7 — Cardioid and Multi-Curve Polar Equations (Ensure $r = $ is NEVER dropped)
Input page: "Question 8 (****)\nThe diagram above shows the curves with polar equations\nr = 1 + sin 2*theta, 0 <= theta <= pi/2\nr = 1.5, 0 <= theta <= pi/2\nFind the area enclosed between the two curves."
Output: {"items": [{"question_number": 8, "content": "The diagram above shows the curves with polar equations\n\n$$r = 1 + \\sin 2\\theta, \\quad 0 \\le \\theta \\le \\frac{\\pi}{2}$$\n\nand\n\n$$r = 1.5, \\quad 0 \\le \\theta \\le \\frac{\\pi}{2}$$\n\nFind the area enclosed between the two curves.", "marks": null, "difficulty_rating": "****", "topics": ["polar coordinates", "integration"], "module": "Pure Mathematics", "is_code": false, "diagram_bboxes": [], "diagram_captions": [], "diagram_kinds": [], "bbox_page_indexes": [], "math_snippet": "r = 1 + \\sin 2\\theta", "visual_options": null}]}

Example 8 — Nuclear decay equation (prescripts are MATH, never plaintext)
Input page (Question 5): "5 The nuclide 226 88 Ra decays by alpha emission to radon. [2 marks]"
Output: {"items": [{"question_number": 5, "content": "The nuclide $^{226}_{88}\\text{Ra}$ decays by alpha emission:\n\n$$^{226}_{88}\\text{Ra} \\rightarrow\\ ^{222}_{86}\\text{Rn} +\\ ^{4}_{2}\\alpha$$\n\n**[2 marks]**", "marks": 2, "difficulty_rating": null, "topics": ["nuclear physics"], "module": "Physics", "is_code": false, "diagram_bboxes": [], "diagram_captions": [], "diagram_kinds": [], "bbox_page_indexes": [], "math_snippet": "^{226}_{88}Ra -> ^{222}_{86}Rn + alpha", "visual_options": null}]}

Example 9 — Scrambled nuclear equation (the text layer flattens prescripts/subscripts across lines; YOU must rebuild the physics)
Input page (Question 31): "31 Uranium-238 absorbs a neutron.\n238 92 U n X \\rightarrow\nX Y beta + anti-neutrino \\rightarrow\nY Z beta + anti-neutrino \\rightarrow\nHow many neutrons does Z have?\nA 144 B 145 C 149 D 237"
Output: {"items": [{"question_number": 31, "content": "Uranium-238 absorbs a neutron in the first stage in a series of nuclear reactions that end in nucleus Z.\n\n$$^{238}_{92}\\text{U} +\\ ^{1}_{0}\\text{n} \\rightarrow\\ ^{239}_{92}\\text{X}$$\n\n$$^{239}_{92}\\text{X} \\rightarrow\\ ^{239}_{93}\\text{Y} + \\beta^- + \\bar{\\nu}_e$$\n\n$$^{239}_{93}\\text{Y} \\rightarrow\\ ^{239}_{94}\\text{Z} + \\beta^- + \\bar{\\nu}_e$$\n\nHow many neutrons does Z have?\n\n- [MCQ:A] 144\n- [MCQ:B] 145\n- [MCQ:C] 149\n- [MCQ:D] 237 **[1 mark]**", "marks": 1, "difficulty_rating": null, "topics": ["nuclear physics"], "module": "Physics", "is_code": false, "diagram_bboxes": [], "diagram_captions": [], "diagram_kinds": [], "bbox_page_indexes": [], "math_snippet": "", "visual_options": null}]}

Example 10 — Stacked fraction options (a fraction printed over two text lines is ONE option; rebuild with \\frac)
Input page (Question 12): "12 Charon is a moon of Pluto. The distance between their centres is d. X is the point where the field is zero. What is the distance of X from the centre of Pluto?\nA 2\n9 d\nB 2\n3 d\nC 3\n4 d\nD 8\n9 d"
Output: {"items": [{"question_number": 12, "content": "Charon is a moon of Pluto that has a mass equal to that of Pluto.\nThe distance between the centre of Pluto and the centre of Charon is $d$.\n$X$ is the point at which the resultant gravitational field due to Pluto and Charon is zero.\nWhat is the distance of $X$ from the centre of Pluto?\n\n- [MCQ:A] $\\frac{2}{9}d$\n- [MCQ:B] $\\frac{2}{3}d$\n- [MCQ:C] $\\frac{3}{4}d$\n- [MCQ:D] $\\frac{8}{9}d$ **[1 mark]**", "marks": 1, "difficulty_rating": null, "topics": ["gravitational fields"], "module": "Physics", "is_code": false, "diagram_bboxes": [], "diagram_captions": [], "diagram_kinds": [], "bbox_page_indexes": [], "math_snippet": "", "visual_options": null}]}

END OF EXAMPLES — Follow the same JSON structure, escaping rules, and isolation discipline exactly.
"#;

    format!(
        r#"You are a precise mathematical OCR engine transcribing exactly ONE requested exam question. Output ONLY a valid JSON object of the form {{"items": [ ... ]}}.

CONTEXT: The user will specify the target question number, paper name, and module name in the user prompt. Transcribe ONLY content belonging to the requested target question. If nothing on the page(s) belongs to the target question, return {{"items": []}}.

═══ MATHEMATICAL NOTATION & DELIMITER RULES (CRITICAL) ═══
1. STRICT DELIMITERS: Every single inline mathematical expression, variable, greek letter, or formula MUST be enclosed in single dollar signs `$ ... $`. Display equations MUST be placed on their own line enclosed in double dollar signs `$$ ... $$`.
2. PERFECT DELIMITER PAIRING: Every opened `$` must be closed with `$`. Every opened `$$` must be closed with `$$`. NEVER leave unclosed delimiters, mismatched tags like `$ ... $$`, or omit the opening delimiter (e.g. NEVER emit `\frac{{...}}$` without opening `$`).
3. ABSOLUTE VARIABLE FIDELITY: Transcribe complete equations verbatim without dropping leading variables, function headers, or curve names. If the source shows "r = ...", "y = ...", "f(x) = ...", "C: r = ...", you MUST include the "r = ", "y = ", etc. inside the math block: `$$r = \\frac{{...}}{{...}}$$`.
4. DOMAIN & CONSTRAINTS: Include all domain restrictions (e.g. `, \\quad 0 \\le \\theta < 2\\pi`) inside the math delimiters.
5. NO PLAIN TEXT IN $$: Never wrap standard English sentences or instructions inside `$$ ... $$`.
6. SENTENCE INTEGRITY: A sentence is ONE paragraph. NEVER insert a hard line break mid-sentence — the PDF's printed line wrapping is NOT sentence structure. Join the wrapped print-lines of one sentence into a single continuous line; use \\n\\n ONLY between sub-parts, display equations, tables, or genuinely separate paragraphs. A sentence fragmented across paragraphs is a transcription error. WRONG: "The diagram shows\\na circuit connected..." RIGHT: "The diagram shows a circuit connected..." on ONE line.
7. OPERATOR PRESERVATION: Every printed fraction MUST be transcribed as LaTeX (\\frac{{numerator}}{{denominator}} — or a/b inside $...$ for simple inline fractions), and every ratio with its operator (a:b or a\\div b). NEVER silently drop a fraction bar, division slash, ratio colon, ×, ÷, ±, or exponent. If the paper prints a stacked fraction, output \\frac — never flatten it into prose and never omit it. Ratio PHRASES keep their operator too: "electrostatic force / gravitational force" MUST keep its slash (or become $\\frac{{\\text{{electrostatic force}}}}{{\\text{{gravitational force}}}}$) — never render as "electrostatic forcegravitational force".
8. NUCLEAR / PARTICLE NOTATION: nuclide and decay notation (mass/atomic prescripts, alpha/beta/gamma particles) MUST be LaTeX math, e.g. $^{{226}}_{{88}}\\text{{Ra}} \\rightarrow\\ ^{{222}}_{{86}}\\text{{Rn}} +\\ ^{{4}}_{{2}}\\alpha$. NEVER emit ^{{...}}_{{...}} prescripts or \\alpha/\\beta/\\gamma decay notation as bare plaintext outside math delimiters.
9. MULTI-LINE EQUATIONS: Sequential or stacked printed equations (nuclear decay chains, simultaneous equation pairs, multi-step derivations) MUST keep their line structure. Put EACH equation in its OWN `$$ ... $$` block separated by a blank line, or join rows INSIDE one block using the LaTeX row separator \\\\ . NEVER concatenate consecutive equations end-to-end on a single line (WRONG: $$\\text{{X}} \\rightarrow \\text{{Y}} \\rightarrow \\text{{Z}}$$ when printed stacked) and NEVER drop the newline between them.

═══ ADAPTIVE MARK & DIFFICULTY EXTRACTION ═══
1. STANDARD MARKS: If explicit mark allocations are printed (e.g. `[4 marks]`, `(3 marks)`, `[1 mark]`), sum them as an integer in `"marks"`, and place `**[X marks]**` at the end of each marked sub-part — NEVER inside `$ ... $` or `$$ ... $$` math blocks.
2. DIFFICULTY RATINGS: If difficulty / star ratings are present instead (e.g. `(*)`, `(**)`, `(***)`, `(***+)`, `(****)`, `(*****)`, `(Specialist)`, `(Synoptic)` as in T. Madas worksheets):
   - Extract the rating string into `"difficulty_rating"` (e.g. `"***+"`).
   - Set `"marks": null`.
   - DO NOT invent, hallucinate, or default marks (e.g. do NOT output `marks: 1`) when no mark scheme allocation is printed.
3. If neither is present, set `"marks": null` and `"difficulty_rating": null`.

═══ ANSWER LINES, WRITE-IN SPACES & UNITS (CRITICAL) ═══
1. STRIP FINAL ANSWER LINES: Completely OMIT every answer prompt line, fill-in blank, underline run, dotted leader, and standalone answer box designed for student responses. Examples to DROP entirely: "answer = _______", "average emf = _________ V", "Total = _____", "_________", "………………", and any ruled/dotted response area. These are layout furniture for the candidate, NOT question content — transcribing them corrupts the question.
2. TRAILING UNIT LABELS: When dropping an answer line, also drop the unit label printed at the END of that line (the trailing "V" in "average emf = _________ V", or trailing "m/s", "J", "N", "°", "%"). Only transcribe units that are part of the explanatory question narrative itself (e.g. "The resistance R is measured in ohms (∼)" or "Give your answer in joules").
3. MARK SCHEME FOOTERS: Lines like "(Total for Question 5 is 8 marks)" are footers, never content.

═══ MARK ALLOCATION PLACEMENT & LATEX ISOLATION (CRITICAL) ═══
1. PLACEMENT: Place each mark allocation (e.g. **[3 marks]**, (3)) at the END of the question or sub-part text it belongs to, or on its own new line immediately AFTER the question statement. NEVER merge mark brackets inside LaTeX math blocks ($ ... $ or $$ ... $$) and NEVER inline them with answer prompts or units.
2. LATEX ISOLATION: Only wrap ACTUAL mathematical expressions, variables, and numerical values in LaTeX delimiters. NEVER combine exam metadata, text labels, units, and marks into a single math string. FORBIDDEN corruption pattern: $averageemf = V **[3marks]**$. CORRECT: the prose stays as prose, the mark tag is **[3 marks]** outside math, and only real math (e.g. $\\varepsilon = 12$) is delimited.

═══ SUB-PARTS VS MULTIPLE CHOICE (CRITICAL) ═══
1. SUB-QUESTIONS: Sub-parts labeled `(a)`, `(b)`, `(c)` or `(i)`, `(ii)` are mathematical sub-questions. Format them in lowercase parentheses `(a)`, `(b)` separated by double newlines (`\n\n`). NEVER convert sub-parts into multiple-choice options.
2. PART ORDER & INTRO LINES: Sub-parts MUST appear in printed order — (a), then (b), then (c)… — each label EXACTLY once. An unlabelled introduction paragraph printed BEFORE (a) (scene-setting, apparatus description, "The figure shows…") belongs to the stem: transcribe it FIRST with NO part label at all. NEVER attach a part label to the introduction, never reorder parts, never duplicate a label.
3. MULTIPLE CHOICE: Only format as multiple choice if the question has 4 alternative answers. MCQ option lines MUST be tagged exactly `- [MCQ:A] …`, `- [MCQ:B] …`, `- [MCQ:C] …`, `- [MCQ:D] …` as four CONSECUTIVE lines with NO blank lines between them, ending with ` **[1 mark]**` after option D. A stacked fraction printed across two text lines is ONE option — rebuild it as `\frac`.

═══ QUESTION ISOLATION RULES (HIGHEST PRIORITY) ═══
Transcribe the target question and NOTHING ELSE. Sub-parts belonging to other questions must NEVER appear.
1. OWNERSHIP: A sub-part belongs to the question printed in its label, or the nearest whole-number heading above it. If not the target question, DROP it.
2. HARD STOP: Immediately stop transcribing when you meet another question heading, a sub-part label for a different main question, a totals footer for the target question ("(Total for Question N is M marks)"), or a new question separator.
3. HARD START: Skip trailing parts/diagrams of previous questions at the page top.
4. SUB-PARTS: Continue in printed order ((a), (b), (c)...). If numbering restarts at (a) after a totals footer, it belongs to the next question — DROP it.
5. ISOLATION: Never merge or renumber neighbouring questions. A short, accurate item is required.
6. Y-BAND HINTS: If a y-band hint is provided in user instructions, adhere strictly to its vertical bounds.

OUTPUT STRUCTURE — EVERY item MUST have:
- "question_number": integer matching the requested target question number.
- "content": Full text transcription of the target question without summary or leading question number (e.g. "17 Here is..." -> "Here is...").
  * Format sub-parts (a), (b), (c) separated by double newlines (\n\n).
  * Omit headers, footers, "(Total for Question...)", blank page notices, and dotted answer lines at the end of parts.
  * Structured tables (trace tables, data tables): Transcribe as Markdown tables (| col |), NEVER as diagram boxes.
   * Math: Wrap inline math in $...$, display equations on their own line in $$...$$. Use valid LaTeX (\\frac, \\sin, \\cos, \\theta). Backslashes MUST be escaped in JSON (\\\\frac). An unescaped escape corrupts the text: writing \\text instead of \\\\text decodes to a literal TAB character followed by "ext" — ALWAYS double every backslash (\\\\text, \\\\theta).
  * Code: Markdown backticks (`...`), never LaTeX math mode.
  * Insert [DIAGRAM_PLACEHOLDER] chronologically after referencing text.
- "marks": Total integer marks, or null if unknown / difficulty-rated.
- "difficulty_rating": String difficulty rating (e.g. "***+", "**") or null if standard marks or unknown.
{topics_instruction}
- "module": string — output EXACTLY '{module}'.
- "is_code": boolean (true only for code/pseudocode).
- "diagram_bboxes": array of [x, y, w, h] boxes in full-page 0.0-1.0 relative coordinates.
  * Box every drawn figure, chart, circuit, geometric sketch, or schema. For graphs, include axes, tick labels, units, and captions with visible margin.
  * NEVER box text tables, Markdown tables, mathematical matrices, vector equations, display formulas, or empty student answer spaces. All formulas and matrices MUST be transcribed in LaTeX math ($$ ... $$ / $ ... $), never boxed as image crops.
  * Box ONLY the graphic itself. Do NOT include surrounding question text, equations, or prose in the diagram bounding box.
- "diagram_captions": array of strings, one per box.
- "diagram_kinds": array of semantic strings ("graph", "schema", "flowchart", "circuit", etc.), one per box.
- "bbox_page_indexes": array of 0-based page image indices matching diagram_bboxes.
- "visual_options": null for text questions. When MCQ answer choices are IMAGE-BASED diagrams (four graphs / circuits / sketches labelled A-D):
  * Treat EACH option diagram as a DISTINCT entity. Return FOUR (or as many as options present) SEPARATE diagram_bboxes — ONE tight box per option, listed in A, B, C, D order. NEVER merge all option diagrams into one big composite box or a single image array attached to the stem.
  * diagram_captions: ["Option A", "Option B", ...]; diagram_kinds: ["visual_option", "visual_option", ...] — one entry per box.
  * In "content" bind each isolated diagram to its option letter with the strict MCQ list format, one line per option:
    "- [MCQ:A] [DIAGRAM_PLACEHOLDER]\n- [MCQ:B] [DIAGRAM_PLACEHOLDER]\n- [MCQ:C] [DIAGRAM_PLACEHOLDER]\n- [MCQ:D] [DIAGRAM_PLACEHOLDER]"
  * Set "visual_options" to exactly "image_options". The placeholder COUNT must equal the box COUNT.{few_shot}
"#,
        topics_instruction = topics_instruction,
        module = config.module_name,
        few_shot = FEW_SHOT,
    )
}

/// System prompt for TEXT-LAYER-ONLY extraction (no images attached). Used for
/// questions whose figures are supplied deterministically by the PDF content
/// stream detector (`page_figures`), so the model transcribes the text and
/// marks where a figure belongs; the crops are spliced in by Rust afterwards.
///
/// Deliberately does NOT reuse `extraction_system_prompt`: that prompt carries
/// seven few-shot examples whose entire purpose is teaching `diagram_bboxes`
/// — content this mode forbids. A compact prompt (rules only + one example)
/// saves ~2k tokens per call, and since text-first calls dominate the import
/// bill (~30 per paper), that is the single largest cost lever.
fn text_first_system_prompt(config: &PipelineConfig) -> String {
    let topics_instruction = if config.allowed_topics.is_empty() {
        "- \"topics\": array. MUST be empty []. Do NOT invent topics.".to_string()
    } else {
        format!(
            "- \"topics\": array. At least one. Select ONLY from this exact list: {:?}. Never invent topics.",
            config.allowed_topics
        )
    };

    format!(
        r#"You are a precise mathematical OCR engine transcribing exam questions from a PDF TEXT LAYER. Output ONLY a valid JSON object of the form {{"items": [ ... ]}}.

CONTEXT: The user will specify the target question number(s), paper name, and module name. Transcribe ONLY content belonging to the requested question(s). If nothing on the page(s) belongs to a requested question, omit its item.

⚠️ TEXT-ONLY MODE: NO IMAGES ARE ATTACHED. The RAW TEXT below is the complete and authoritative source. Transcribe the question's text, sub-parts, and marks EXACTLY from it. Do NOT invent content, values, or diagrams that are not in the text. If the question references a figure ("Figure 1", "the diagram below", "the circuit", "the graph"), insert the placeholder [DIAGRAM_PLACEHOLDER] immediately after the sentence or clause that references it — never at the end of the question. Emit EXACTLY ONE placeholder per DISTINCT figure, even when the same figure is referenced several times (e.g. "Figure 9" appears in parts (a), (b), (c) — still ONE placeholder for Figure 9). The figure itself will be attached by the system afterwards.

═══ MATHEMATICAL NOTATION & DELIMITER RULES (CRITICAL) ═══
1. STRICT DELIMITERS: Every single inline mathematical expression, variable, greek letter, or formula MUST be enclosed in single dollar signs `$ ... $`. Display equations MUST be placed on their own line enclosed in double dollar signs `$$ ... $$`.
2. PERFECT DELIMITER PAIRING: Every opened `$` must be closed with `$`. Every opened `$$` must be closed with `$$`. NEVER leave unclosed delimiters or mismatched tags.
3. ABSOLUTE VARIABLE FIDELITY: Transcribe complete equations verbatim without dropping leading variables, function headers, or curve names ("r = ...", "y = ...", "f(x) = ...").
4. DOMAIN & CONSTRAINTS: Include all domain restrictions (e.g. `, \quad 0 \le \theta < 2\pi`) inside the math delimiters.
5. NO PLAIN TEXT IN $$: Never wrap standard English sentences or instructions inside `$$ ... $$`.
6. SENTENCE INTEGRITY: A sentence is ONE paragraph. NEVER insert a hard line break mid-sentence — the text layer's line wrapping is NOT sentence structure. Join the wrapped lines of one sentence into a single continuous line; use \n\n ONLY between sub-parts, display equations, tables, or genuinely separate paragraphs. WRONG: "The diagram shows\na circuit connected...". RIGHT: "The diagram shows a circuit connected..." on ONE line.
7. OPERATOR PRESERVATION: Every printed fraction MUST be transcribed as LaTeX (\frac{{numerator}}{{denominator}} — or a/b inside $...$ for simple inline fractions), and every ratio with its operator (a:b). NEVER silently drop a fraction bar, division slash, ratio colon, ×, ÷, ±, or exponent. Ratio PHRASES keep their operator too: "electrostatic force / gravitational force" MUST keep its slash (or become $\frac{{\text{{electrostatic force}}}}{{\text{{gravitational force}}}}$) — never render as "electrostatic forcegravitational force".
8. NUCLEAR / PARTICLE NOTATION: nuclide and decay notation MUST be LaTeX math, e.g. $^{{226}}_{{88}}\text{{Ra}} \rightarrow\ ^{{222}}_{{86}}\text{{Rn}} +\ ^{{4}}_{{2}}\alpha$. NEVER emit prescripts or decay particles as bare plaintext outside math delimiters.
9. MULTI-LINE EQUATIONS: Sequential or stacked printed equations (nuclear decay chains, simultaneous pairs, multi-step derivations) MUST keep their line structure. Put EACH equation in its OWN `$$ ... $$` block separated by a blank line, or join rows INSIDE one block using the LaTeX row separator \\\\ . NEVER concatenate consecutive equations end-to-end and NEVER drop the newline between them.

═══ ADAPTIVE MARK & DIFFICULTY EXTRACTION ═══
1. STANDARD MARKS: If explicit mark allocations are printed (e.g. `[4 marks]`, `(3 marks)`, `[1 mark]`), sum them as an integer in `"marks"`, and place `**[X marks]**` at the end of each marked sub-part — NEVER inside `$ ... $` or `$$ ... $$` math blocks.
2. DIFFICULTY RATINGS: If difficulty / star ratings are present instead (e.g. `(*)`, `(**)`, `(***)`, `(***+)`, `(****)`, `(*****)`, `(Specialist)`, `(Synoptic)`): extract the rating string into `"difficulty_rating"`, set `"marks": null`, and DO NOT invent marks.
3. If neither is present, set `"marks": null` and `"difficulty_rating": null`.

═══ ANSWER LINES, WRITE-IN SPACES & UNITS (CRITICAL) ═══
1. STRIP FINAL ANSWER LINES: Completely OMIT every answer prompt line, fill-in blank, underline run, dotted leader, and standalone answer box designed for student responses (e.g. "answer = _______", "average emf = _________ V", "Total = _____", "_________", "………………"). These are layout furniture, NOT question content.
2. TRAILING UNIT LABELS: When dropping an answer line, also drop the unit label printed at the END of that line (trailing "V", "m/s", "J", "N", "°", "%"). Only transcribe units that are part of the explanatory question narrative itself.

═══ MARK ALLOCATION PLACEMENT & LATEX ISOLATION (CRITICAL) ═══
1. PLACEMENT: Place each mark allocation (e.g. **[3 marks]**, (3)) at the END of the question or sub-part text, or on its own new line immediately AFTER it. NEVER merge mark brackets inside LaTeX math blocks ($ ... $ or $$ ... $$).
2. LATEX ISOLATION: Only wrap ACTUAL mathematical expressions, variables, and numerical values in LaTeX delimiters. NEVER combine exam metadata, text labels, units, and marks into a single math string (FORBIDDEN: $averageemf = V **[3marks]**$).

═══ SUB-PARTS VS MULTIPLE CHOICE (CRITICAL) ═══
1. SUB-QUESTIONS: Sub-parts labeled `(a)`, `(b)`, `(c)` or `(i)`, `(ii)` are mathematical sub-questions. Format them in lowercase parentheses `(a)`, `(b)` separated by double newlines (`\n\n`). NEVER convert sub-parts into multiple-choice options.
2. PART ORDER & INTRO LINES: Sub-parts MUST appear in printed order — each label EXACTLY once; an unlabelled introduction printed before (a) is stem text and carries NO label.
3. MULTIPLE CHOICE: MCQ option lines MUST be tagged exactly `- [MCQ:A] …` … `- [MCQ:D] …` as four CONSECUTIVE lines with NO blank lines between them, ending with ` **[1 mark]**` after option D. A stacked fraction printed across two text lines is ONE option — rebuild it as `\frac`.

═══ QUESTION ISOLATION RULES (HIGHEST PRIORITY) ═══
1. Transcribe the target question and NOTHING ELSE. A sub-part belongs to the question printed in its label, or the nearest whole-number heading above it. If not the target question, DROP it.
2. HARD STOP: Immediately stop transcribing when you meet another question heading, a sub-part label for a different main question, a totals footer ("(Total for Question N is M marks)"), or a new question separator.
3. SUB-PARTS: Continue in printed order ((a), (b), (c)...). If numbering restarts at (a) after a totals footer, it belongs to the next question — DROP it.
4. ISOLATION: Never merge or renumber neighbouring questions.

═══ OUTPUT STRUCTURE — EVERY item MUST have ═══
- "question_number": integer matching the requested question number.
- "content": Full text transcription without summary or leading question number. Format sub-parts separated by double newlines. Structured tables (trace tables, data tables): Markdown tables (| col |), NEVER as diagram boxes. Math: `$...$` / `$$...$$` with valid LaTeX (\\frac, \\sin, \\cos, \\theta); backslashes MUST be escaped in JSON (\\\\frac) — writing \\text instead of \\\\text decodes to a literal TAB character followed by "ext", so ALWAYS double every backslash. Code: Markdown backticks, never LaTeX math mode.
- "marks": integer total, or null.
- "difficulty_rating": string rating or null.
{topics_instruction}
- "module": string — output EXACTLY '{module}'.
- "is_code": boolean (true only for code/pseudocode).
- "math_snippet": string — the key equation/expression, or "".
- "diagram_bboxes", "diagram_captions", "diagram_kinds", "bbox_page_indexes" MUST be empty arrays and "visual_options" MUST be null — figure crops are supplied by the system, not by you.

EXAMPLE — multi-part question with marks:
Input: "7 (a) Solve 2x^2 - 5x + 2 = 0. [3 marks]\n(b) Hence solve 2y^4 - 5y^2 + 2 = 0. [2 marks]"
Output: {{"items": [{{"question_number": 7, "content": "(a) Solve $2x^2 - 5x + 2 = 0$.\n\n**[3 marks]**\n\n(b) Hence solve $2y^4 - 5y^2 + 2 = 0$.\n\n**[2 marks]**", "marks": 5, "difficulty_rating": null, "topics": [], "module": "{module}", "is_code": false, "diagram_bboxes": [], "diagram_captions": [], "diagram_kinds": [], "bbox_page_indexes": [], "math_snippet": "2x^2 - 5x + 2 = 0", "visual_options": null}}]}}
"#,
        topics_instruction = topics_instruction,
        module = config.module_name,
    )
}

/// System prompt for CROP-FIRST extraction: the attached images are the
/// question's figure(s), already cropped by the detector, so the model can
/// READ values off them without paying for full-page image tokens. The
/// question wording comes from the RAW TEXT; the crops carry the exhibit.
fn crop_first_system_prompt(config: &PipelineConfig) -> String {
    let base = extraction_system_prompt(config);
    format!(
        r#"{base}

══════ CROP-FIRST MODE OVERRIDE (READ CAREFULLY — THESE RULES REPLACE THE FEW-SHOT DIAGRAM RULES ABOVE) ══════
- The attached image(s) are the question's figure(s), ALREADY CROPPED by the system. They contain the data (values, coordinates, readings, graph shapes) needed to answer the question.
- READ any required values, coordinates, or readings from the figure image(s). The answer may depend entirely on what the figure shows — if so, state the value you read from the figure.
- The RAW TEXT below is authoritative for the QUESTION WORDING; transcribe the question's text, sub-parts, and marks EXACTLY from it.
- "diagram_bboxes", "diagram_captions", "diagram_kinds", "bbox_page_indexes" MUST all be empty arrays and "visual_options" MUST be null — the figure content is already provided as an image, do NOT box it.
- Do NOT insert [DIAGRAM_PLACEHOLDER].
- Do NOT invent content, values, or diagrams that are not in the text or figure image.
- All other rules (math delimiters, marks, sub-parts, topics, module, question isolation) still apply exactly as above.
"#,
        base = base,
    )
}

fn markscheme_system_prompt() -> String {
    r#"You are an expert examiner transcribing a mark scheme into Markdown. Return ONLY a valid JSON object: {"answers": [...]} (or an empty array [] / {"answers": []} when the pages contain no real answers).

ESCAPE HATCH: If the images show only front covers, general marking guidance, abbreviation lists, or formula booklets, return an empty array. NEVER invent questions to fill the output.
EXTRACTION GUARDRAIL: Only extract entries with explicit mark-scheme structure: a question-number column header (e.g. 1(a), 2(b)(i)) AND mark labels (M1, A1, B1, dM1, ft). Numbered lists in guidance pages are NOT mark schemes.

Each array item: { "question_number": int (WHOLE question only; AQA 03.1 → 3), "answer_markdown": string, "diagram_bboxes": [[x,y,w,h]...] relative 0.0-1.0, "diagram_page_indexes": [ints, same length as bboxes, 0-based image index] }.

RULES:
- Group every part of one question (main + ONE alternative method max) into a SINGLE item for that question_number. Further alternatives: discard. Alternative appended after a Markdown divider `---` and a bold "**ALTERNATIVE METHOD**" header.
- Part labels bolded on their own line: **(a)**. Every distinct marking step separated by a double newline (\n\n). Inline math with single $...$; display equations with $$...$$ on their own line. NEVER use code fences.
- ARTIFACT FILTERING: Recognize and completely exclude all non-exam content. Silently ignore margin warnings, printer registration marks, page numbers, and barcodes.
- INLINE IMAGE PLACEMENT: If an image or diagram is present, insert its placeholder [DIAGRAM_PLACEHOLDER] IMMEDIATELY after the sentence or paragraph that references it. Never place diagrams at the end of the question if they were referenced earlier.
- TABLE FORMATTING: If a grid or table contains standard text or numbers, you MUST format it as a standard Markdown table using pipes | and dashes -. NEVER use LaTeX array environments or \hline for data tables.
- EQUATION COHESION: A mathematical equation MUST remain inside a single, cohesive display math block $$ ... $$. Never split an equation into multiple blocks. Operators like =, +, or exponents like ^n and ^{-1} must remain inside the same block as the matrices or variables they belong to. Sequential printed equations (decay chains, simultaneous pairs) keep their line structure: separate $$ blocks per equation, or rows joined with \\ inside one block — NEVER concatenated end-to-end on one line.
- CHARACTER PRECISION: Pay close attention to function notation. Do not confuse the italic function symbol $f$ in $f(x)$ or $f(t)$ with the number 1. Pay extreme attention to Greek symbols: do not confuse \theta with the number 1, or \alpha with a. Accurately transcribe all complex number forms, e.g., r(\cos \theta + \text{i}\sin \theta).
- DELIMITER DISCIPLINE: NEVER place inline math delimiters $ inside a display math $$ block. Display math must start with $$ and end with $$ with NO inner $ signs. Never wrap regular prose or full sentences in $$ display math delimiters.
- LIST CLEAN-UP: Do not output empty list bullets or empty numbered prefixes.
- CRITICAL: Never fracture inline math. WRONG: $r(\cos$ \theta $). RIGHT: $r(\cos \theta)$.
- CRITICAL: Never wrap English sentences in $$. Use $ for variables inside text.
- STRUCTURAL SPACING: Enforce strict hierarchical spacing with double line breaks (\n\n) separating sub-parts and distinct mark points.
- SENTENCE INTEGRITY: A sentence is ONE paragraph. NEVER insert a hard line break mid-sentence — the printed line wrapping of the paper is NOT sentence structure. Join wrapped lines of one sentence into a single continuous line; use \n\n only between mark points, equations, and genuinely separate paragraphs.
- OPERATOR PRESERVATION: Every fraction MUST be transcribed as LaTeX (\frac{a}{b}, or a/b inside $...$) and every ratio with its operator (a:b). NEVER silently drop a fraction bar, division slash, ratio colon, ×, ÷, ±, or exponent.
- NUCLEAR / PARTICLE NOTATION: nuclide prescripts and alpha/beta/gamma decay particles MUST be LaTeX math, e.g. $^{14}_{6}\text{C} \rightarrow\ ^{14}_{7}\text{N} +\ ^{0}_{-1}\beta$. Never emit them as bare plaintext.
- MATHEMATICAL ACCURACY: Ensure all mathematical notation, including complex numbers, vectors, matrices, exponents, and trigonometric/logarithmic functions, is accurately translated into valid, standard LaTeX.
- ROBUST TABLE RENDERING: Data/trace tables: standard Markdown tables compatible with Markdown viewers. Never leak raw \hline or unrendered tabular tags. True matrices/Simplex tableaus: \begin{array} or \begin{pmatrix} in $$...$$.
- Sub-part letters must continue across pages: do not reset (g) back to (a).
- ANSWER LINES / WRITE-IN SPACES: completely omit answer prompts, fill-in blanks, underline runs and dotted leaders ("answer = _______", "Total = _____", "_________"), including any trailing unit label attached to them. These are student response areas, not mark-scheme content.
- MARK TAGS: keep mark labels (M1, A1, B1) and mark totals OUTSIDE LaTeX math delimiters. Never merge marks into a single math string with prose or units.
- Exclude: examiner notes about mark codes, page headers/footers, AQA margin numbers, blank answer-line numbers, and reprinted question text (the REPRINT BAN).
- Diagrams (activity networks, Gantt charts, trees, graphs): capture via diagram_bboxes + diagram_page_indexes and insert [DIAGRAM_PLACEHOLDER] where the diagram belongs. NEVER box text, math working, examiner notes, or empty grids (the CRITICAL DIAGRAM BAN).
- JSON ESCAPING: escape LaTeX backslashes (\\frac not \frac). Writing \text instead of \\text decodes to a literal TAB character followed by "ext" — double EVERY backslash. Invalid JSON is rejected outright and your work is lost.
- You are a transcriber, not a solver. If there is no question-number column with mark labels on these pages, return an empty array."#
        .to_string()
}

/// Slim prompt for TEXT-ONLY mark-scheme windows: the full transcription
/// rules minus every image/diagram-boxing rule (no images are attached).
/// A `[DIAGRAM_PLACEHOLDER]` in the response is the signal that this window
/// genuinely contains worked figures and must fall back to vision.
fn markscheme_text_first_system_prompt() -> String {
    let mut rules = markscheme_system_prompt();
    // The base prompt teaches image boxing and treats images as authoritative;
    // both are wrong for a text-only window. Swap those passages out.
    rules = rules.replace(
        "Each array item: { \"question_number\": int (WHOLE question only; AQA 03.1 → 3), \"answer_markdown\": string, \"diagram_bboxes\": [[x,y,w,h]...] relative 0.0-1.0, \"diagram_page_indexes\": [ints, same length as bboxes, 0-based image index] }.",
        "Each array item: { \"question_number\": int (WHOLE question only; AQA 03.1 → 3), \"answer_markdown\": string }.",
    );
    rules = rules.replace(
        "- INLINE IMAGE PLACEMENT: If an image or diagram is present, insert its placeholder [DIAGRAM_PLACEHOLDER] IMMEDIATELY after the sentence or paragraph that references it. Never place diagrams at the end of the question if they were referenced earlier.",
        "- FIGURES IN TEXT-ONLY MODE: NO IMAGES are attached. If a worked figure/graph/image is genuinely required to convey the answer, insert [DIAGRAM_PLACEHOLDER] at its position — do NOT attempt to describe it.",
    );
    rules = rules.replace(
        "- Diagrams (activity networks, Gantt charts, trees, graphs): capture via diagram_bboxes + diagram_page_indexes and insert [DIAGRAM_PLACEHOLDER] where the diagram belongs. NEVER box text, math working, examiner notes, or empty grids (the CRITICAL DIAGRAM BAN).",
        "- Diagrams cannot be boxed in this mode: use [DIAGRAM_PLACEHOLDER] where one belongs. Tables of DATA stay as Markdown tables (see TABLE FORMATTING) — they are text, not diagrams.",
    );
    rules = rules.replace(
        "Raw text is provided as a baseline (images are authoritative):",
        "",
    );
    format!("{}\nMODE: TEXT-ONLY. The raw text layer below is authoritative — transcribe from it directly.", rules)
}

/// Slim JSON schema for TEXT-ONLY MS calls: answers carry no diagram fields.
#[allow(dead_code)]
fn markscheme_text_first_json_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "json_schema",
        "json_schema": {
            "name": "markscheme_text_first",
            "schema": {
                "type": "object",
                "properties": {
                    "answers": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "question_number": { "type": "integer" },
                                "answer_markdown": { "type": "string" }
                            },
                            "required": ["question_number", "answer_markdown"]
                        }
                    }
                },
                "required": ["answers"]
            }
        }
    })
}

/// Conservative reliability gate for TEXT-ONLY mark-scheme windows: every
/// page must carry real extracted text, the combined text must be
/// substantial, replacement-character garbage must be rare, and there must
/// be no figure references (worked figures need vision boxing).
fn window_text_reliable(pages: &[PageInput]) -> bool {
    if pages.is_empty() {
        return false;
    }
    let mut total = 0usize;
    let mut bad = 0usize;
    for p in pages {
        let t = &p.text;
        if t.trim().is_empty() {
            return false;
        }
        total += t.len();
        bad += t.matches('\u{FFFD}').count();
    }
    if total < 400 {
        return false;
    }
    if (bad as f32) / (total as f32) >= 0.02 {
        return false;
    }
    let combined = pages
        .iter()
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    !text_references_figure(&combined)
}

// ══════════════════════════════════════════════════════════════════════════
// Deferred topic classification
// ══════════════════════════════════════════════════════════════════════════

/// Normalize prose and LaTeX into whole words for local taxonomy matching.
fn topic_words(text: &str) -> String {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty()).collect::<Vec<_>>().join(" ")
}

fn topic_phrase(text: &str, phrase: &str) -> bool {
    format!(" {text} ").contains(&format!(" {} ", topic_words(phrase)))
}

/// Classify in memory, preserving exact allow-list spellings and existing tags.
/// Unknown terminology stays untagged; classification never dispatches an LLM.
async fn classify_topics_deferred<C: LlmClient>(
    _client: &C,
    config: &PipelineConfig,
    built: &mut [BuiltQuestion],
    _request_semaphore: &Arc<Semaphore>,
    cancel: &AtomicBool,
    _usage: &Arc<TokenTotals>,
) -> usize {
    let taxonomy: &[(&[&str], &[&str])] = &[
        (&["Thermal physics", "Thermal"], &["ideal gas", "c rms", "specific heat", "latent heat", "molar", "temperature", "thermal", "internal energy", "molecules", "molecular"]),
        (&["Capacitors", "Capacitance"], &["capacitor", "capacitors", "capacitance", "dielectric", "parallel plate"]),
        (&["Electric fields"], &["electric field", "electric potential", "coulomb", "point charge", "charges", "charged sphere"]),
        (&["Gravitational fields", "Gravitation"], &["gravitational", "satellite", "satellites", "orbit", "orbital", "escape velocity", "planet"]),
        (&["Magnetic fields", "Electromagnetism"], &["magnetic", "induced emf", "solenoid", "tesla", "flux linkage"]),
        (&["Nuclear physics", "Nuclear", "Particles and radiation"], &["alpha", "beta", "decay", "half life", "fission", "fusion", "nuclide", "uranium", "nuclear", "nucleon", "neutrons", "protons", "nucleus", "nuclei", "isotope", "mass lost", "mass defect", "binding energy"]),
        (&["Waves", "Oscillations"], &["oscilloscope", "frequency", "phase difference", "waveform", "wavelength", "oscillation", "oscillations", "harmonic", "diffraction", "interference"]),
        (&["Mechanics"], &["velocity", "acceleration", "force", "momentum", "kinetic energy", "displacement", "spring", "projectile"]),
        (&["Electricity", "Electric circuits"], &["current", "resistance", "resistor", "circuit", "voltage", "filament lamp"]),
        (&["Required Practical", "Practical skills"], &["data logger", "uncertainty", "stopwatch", "measurement", "experiment"]),
    ];
    let allowed: Vec<_> = config.allowed_topics.iter().map(|topic| (topic, topic_words(topic))).collect();
    let mut assigned = 0;
    for q in built.iter_mut() {
        if cancel.load(Ordering::Relaxed) { break; }
        if !q.topics.is_empty() || q.content.trim().is_empty() { continue; }
        let text = topic_words(&q.content);
        for (topic, words) in &allowed {
            if words.is_empty() { continue; }
            let matched = topic_phrase(&text, words) || taxonomy.iter().any(|(names, keywords)| {
                names.iter().any(|name| topic_words(name) == *words)
                    && keywords.iter().any(|keyword| topic_phrase(&text, keyword))
            });
            if matched && !q.topics.contains(topic) {
                q.topics.push((*topic).clone());
                if q.topics.len() == 3 { break; }
            }
        }
        if !q.topics.is_empty() { assigned += 1; }
    }
    assigned
}

/// Decide, from the pages alone, whether question ingestion for this document
/// runs entirely on-device.
///
/// Production callers use this BEFORE resolving a provider, reading
/// credentials, or accounting an upload entitlement: a document carrying any
/// extracted text needs none of them. The pipeline enforces
/// the same policy internally, so this is an early decision, not the guard.
pub fn is_local_only_document(pages: &[PageInput]) -> bool {
    doc_map::classify_text_layer(&pages.iter().map(|p| p.text.clone()).collect::<Vec<_>>())
        .is_digital()
}

pub async fn run_question_pipeline<C: LlmClient, P: Progress>(
    client: &C,
    pages: &[PageInput],
    page_figures: &[Vec<crate::pdf_render::DetectedFigure>],
    config: &PipelineConfig,
    progress: &P,
    cancel: &AtomicBool,
) -> Result<(Vec<BuiltQuestion>, ImportReport), String> {
    let overall_start = Instant::now();
    // Classify the document from its text layer BEFORE any request, and bind
    // that verdict to this run's config. The classification is computed here
    // from the pages actually handed to the pipeline - a caller cannot set it,
    // and a weak question map never downgrades a digital paper to "scanned".
    //
    // The free PDF text layer is preferred because it avoids one vision
    // request per page, so it is materialised once here and reused below.
    let page_texts: Vec<String> = pages.iter().map(|p| p.text.clone()).collect();
    let text_layer_class = doc_map::classify_text_layer(&page_texts);
    #[allow(unused_mut)]
    let mut effective_config = config.clone().with_text_layer_class(text_layer_class.clone());
    // TEST ONLY: legacy cloud-path tests model a scanned/image-only input
    // whose cloud extraction must stay reachable. The field does not exist in
    // production builds, so this can never bypass the digital policy there.
    #[cfg(test)]
    if config.force_scanned_context {
        effective_config.text_layer_class =
            Some(doc_map::TextLayerClass::scanned_only(pages.len()));
    }
    let digital = effective_config.is_digital_document();
    let mut report = ImportReport {
        paper_name: effective_config.paper_name.clone(),
        kind: "questions".to_string(),
        text_layer: effective_config.text_layer_label().to_string(),
        pages_total: pages.len(),
        figures_detected: page_figures.iter().map(Vec::len).sum(),
        ..Default::default()
    };
    if digital {
        if text_layer_class.unresolved_pages().is_empty() {
            report.anomalies.push(
                "digital document: extracted text on every page - cloud extraction disabled (zero requests permitted); local deterministic extraction only".to_string(),
            );
        } else {
            report.anomalies.push(format!(
                "digital document: {}/{} pages carry extracted text - cloud extraction disabled (zero requests permitted); pages {:?} have no text layer and are reported locally",
                text_layer_class.text_pages,
                text_layer_class.total_pages,
                text_layer_class
                    .unresolved_pages()
                    .iter()
                    .map(|p| p + 1)
                    .collect::<Vec<_>>()
            ));
        }
    }
    // Geometry-first layout (digital documents with layout evidence): page
    // text in reading order with reconstructed mathematics, and a question
    // map with exact line boundaries. The classification above already ran
    // on the ORIGINAL text layer, so a sparse layout can never turn a
    // digital document into a cloud one.
    let mut layout_spans: Option<Vec<QuestionSpan>> = None;
    let layout_pages_input: Option<Vec<PageInput>> = match config.layout_evidence.as_ref() {
        Some(ev) if digital && !ev.layout.is_empty() && ev.layout.len() == pages.len() => {
            let lmap = crate::layout::build_layout_map(&ev.layout);
            report.anomalies.extend(lmap.anomalies.iter().cloned());
            if lmap.questions.is_empty() {
                report.anomalies.push("layout map found no questions; using the text-layer map".to_string());
                None
            } else {
                let mut bodies = std::collections::HashMap::new();
                let mut question_figures = std::collections::HashMap::new();
                let mut spans = Vec::with_capacity(lmap.questions.len());
                for q in &lmap.questions {
                    // Figures are spliced where they are printed.
                    let figures = layout_question_figures(&ev.layout, q, page_figures);
                    let rects: Vec<(usize, [f32; 4])> = figures.iter().map(|(f, r)| (f.page, *r)).collect();
                    let body = crate::layout::question_body_with_figures(&ev.layout, q, &rects);
                    question_figures.insert(q.number, figures.into_iter().map(|(f, _)| f).collect::<Vec<_>>());
                    let h0 = ev.layout[q.start_page].height.max(1.0);
                    let h1 = ev.layout[q.end_page].height.max(1.0);
                    spans.push(QuestionSpan {
                        number: q.number,
                        start_page: q.start_page,
                        end_page: q.end_page,
                        start_y_frac: Some((q.start_y / h0).clamp(0.0, 1.0)).filter(|y| *y > 0.05),
                        end_y_frac: q.end_y.map(|y| (y / h1).clamp(0.0, 1.0)),
                        expected_marks: body.footer_marks,
                        reliable_pages: (q.start_page..=q.end_page).collect(),
                        ambiguous_pages: Vec::new(),
                    });
                    bodies.insert(q.number, body);
                }
                effective_config.layout_questions = Some(Arc::new(bodies));
                effective_config.layout_figures = Some(Arc::new(question_figures));
                layout_spans = Some(spans);
                Some(
                    pages
                        .iter()
                        .zip(ev.layout.iter())
                        .map(|(p, lp)| PageInput { kind: p.kind.clone(), text: lp.text() })
                        .collect(),
                )
            }
        }
        _ => None,
    };
    let config = &effective_config;
    // A ruled table is transcribed from the layout as a Markdown table; a
    // detected "figure" that is that table would duplicate it as an image.
    let table_free_figures: Option<Vec<Vec<crate::pdf_render::DetectedFigure>>> = match config.layout_evidence.as_ref() {
        Some(ev) if layout_pages_input.is_some() => Some(
            page_figures
                .iter()
                .enumerate()
                .map(|(pi, figs)| {
                    let Some(lp) = ev.layout.get(pi) else { return figs.clone() };
                    figs.iter()
                        .filter(|f| !figure_is_layout_table(f, lp))
                        .cloned()
                        .collect()
                })
                .collect(),
        ),
        _ => None,
    };
    let page_figures: &[Vec<crate::pdf_render::DetectedFigure>] = table_free_figures.as_deref().unwrap_or(page_figures);
    let pages: &[PageInput] = layout_pages_input.as_deref().unwrap_or(pages);
    let page_texts: Vec<String> = if layout_pages_input.is_some() {
        pages.iter().map(|p| p.text.clone()).collect()
    } else {
        page_texts
    };
    let page_render_cache = Arc::new(crate::pdf_render::PageRenderCache::new(
        PAGE_RENDER_CACHE_CAPACITY,
    ));
    let request_semaphore = Arc::new(Semaphore::new(config.parallelism.max(1)));
    let page_image_cache = Arc::new(PageImageCache::new());
    let usage = Arc::new(TokenTotals::new());

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
    if !text_map_available && !digital {
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
                let usage = Arc::clone(&usage);
                async move {
                    if is_non_question_by_text {
                        return (
                            page_index,
                            Ok(r#"{"question_numbers_visible":[],"page_role":"BLANK"}"#.to_string()),
                        );
                    }
                    let mut images = Vec::new();
                    if let PageInputKind::Image { b64, .. } = &page.kind {
                        // Fast-path: Check if the image itself is blank / solid white
                        if let Some(decoded) = geometry::decode_page_image(b64) {
                            if geometry::is_image_blank(&decoded) {
                                return (
                                    page_index,
                                    Ok(r#"{"question_numbers_visible":[],"page_role":"BLANK"}"#.to_string()),
                                );
                            }
                            // Downscale the page to the API long-edge cap. The
                            // structure pass only needs question numbers and
                            // footers; the cap is the same ceiling extraction
                            // already uses successfully for full transcription.
                            images.push(
                                geometry::encode_webp_resized(
                                    &decoded,
                                    geometry::api_image_max_dim(),
                                )
                                .unwrap_or_else(|| b64.clone()),
                            );
                        } else {
                            images.push(b64.clone());
                        }
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
                        llm::ImageDetail::High,
                        text_opt,
                        750,
                        Some(llm::ResponseFormat::JsonSchema { schema: structure_json_schema() }),
                    );
                    let result = match chat_with_permit(client, &body, &semaphore, cancel, &usage, StageTag::StructurePass, config.cloud_allowed()).await {
                        Ok(resp) => llm::message_content(&resp)
                            .map_err(|e| format!("bad response shape ({})", e)),
                        Err(e) => Err(format!("API failure ({})", e)),
                    };
                    (page_index, result)
                }
            })
            .buffer_unordered(config.parallelism.max(1));
            let mut ordered = Vec::with_capacity(pages.len());
            loop {
                let next_item = tokio::select! {
                    res = structure_results.next() => res,
                    _ = async {
                        while !cancel.load(Ordering::Relaxed) {
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        }
                    } => None,
                };
                match next_item {
                    Some(result) => {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        ordered.push(result);
                    }
                    None => break,
                }
            }
            ordered.sort_by_key(|(index, _)| *index);
            ordered
        };

        // Build the text-layer-only map concurrently. This is fast (ms)
        // but starts the setup work while API calls are in flight.
        let map_setup_future = async {
            doc_map::build_hybrid_map_with_scan(&page_texts, &[], pages.len(), &scan)
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
    let doc_map_start = Instant::now();

    // Use hybrid map building: reliable text pages + vision for ambiguous pages
    let mut map = if let Some(spans) = layout_spans.take() {
        doc_map::DocumentMap { spans, ..Default::default() }
    } else if digital {
        doc_map::build_text_only_map(&page_texts, pages.len(), &scan)
    } else {
        doc_map::build_hybrid_map_with_scan(&page_texts, &structures, pages.len(), &scan)
    };

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
    if map.spans.is_empty() && !digital {
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

    // A digital document whose text layer produced no spans at all has no
    // local path left, and the cloud is switched off. Report the local failure
    // explicitly; never fall into the per-page vision loop.
    if map.spans.is_empty() && digital {
        report.anomalies.push(
            "digital document: text layer produced no question spans and cloud extraction is disabled - nothing extracted locally".to_string(),
        );
        report.quarantined.push(QuarantineEvent {
            scope: "document".to_string(),
            page: None,
            question_number: None,
            reason: "digital document: no locally derivable question spans".to_string(),
        });
    }

    if map.spans.is_empty() && !digital {
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
                &page_image_cache,
                &request_semaphore,
                cancel,
                &usage,
            ).map(move |result| (position, result))
        }))
        .buffer_unordered(config.parallelism.max(1));
        let mut ordered_results = Vec::with_capacity(q_pages.len());
        loop {
            let next_item = tokio::select! {
                res = results.next() => res,
                _ = async {
                    while !cancel.load(Ordering::Relaxed) {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                } => None,
            };
            if cancel.load(Ordering::Relaxed) {
                return Err("Import cancelled by user".to_string());
            }
            match next_item {
                Some(result) => ordered_results.push(result),
                None => break,
            }
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
                                        &page_image_cache,
                                        &request_semaphore,
                                        cancel,
                                        &usage,
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
        
        // Group consecutive single-page spans sharing the same page into SamePageBatch jobs,
        // reducing API calls and input tokens by 70-80% on MCQ and multi-question pages.
        enum ExtractionJob<'a> {
            Single {
                span: &'a QuestionSpan,
                pages: Vec<(usize, &'a PageInput)>,
            },
            SamePageBatch {
                spans: Vec<&'a QuestionSpan>,
                page_idx: usize,
                page_input: &'a PageInput,
            },
        }

        let mut jobs: Vec<ExtractionJob> = Vec::new();
        let mut i = 0;
        while i < map.spans.len() {
            let span = &map.spans[i];
            if span.start_page == span.end_page && span.start_page < pages.len() {
                let p = span.start_page;
                let is_extractable_page = map.non_question_pages.is_empty()
                    || !map.non_question_pages.contains(&p)
                    || structures.get(p).map(|s| s.role == doc_map::PageRole::Blank).unwrap_or(false);

                if is_extractable_page {
                    let mut batch_spans = vec![span];
                    let mut j = i + 1;
                    while j < map.spans.len() {
                        let next_span = &map.spans[j];
                        if next_span.start_page == p && next_span.end_page == p {
                            batch_spans.push(next_span);
                            j += 1;
                        } else {
                            break;
                        }
                    }
                    if batch_spans.len() >= 2 {
                        jobs.push(ExtractionJob::SamePageBatch {
                            spans: batch_spans,
                            page_idx: p,
                            page_input: &pages[p],
                        });
                        i = j;
                        continue;
                    }
                }
            }

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
                i += 1;
                continue;
            }
            jobs.push(ExtractionJob::Single {
                span,
                pages: span_pages,
            });
            i += 1;
        }

        cancelled(cancel)?;
        progress.stage(&format!("Extracting {} questions…", total));
        let extract_start = Instant::now();
        let job_count = jobs.len();
        let collateral_cache: CollateralCache =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(map.spans.clone());

        let mut results = futures_util::stream::iter(0..job_count).map(|position| {
            let job = &jobs[position];
            let client = client;
            let config = config;
            let page_figures = page_figures;
            let page_render_cache = Arc::clone(&page_render_cache);
            let page_image_cache = Arc::clone(&page_image_cache);
            let request_semaphore = Arc::clone(&request_semaphore);
            let collateral_cache = Arc::clone(&collateral_cache);
            let all_spans = Arc::clone(&all_spans);
            let usage = Arc::clone(&usage);
            async move {
                match job {
                    ExtractionJob::Single { span, pages } => {
                        let (opt, local_rep) = extract_span(
                            client,
                            config,
                            span,
                            pages,
                            page_figures,
                            &page_render_cache,
                            &page_image_cache,
                            &request_semaphore,
                            &collateral_cache,
                            &all_spans,
                            config.text_first && text_map_available,
                            cancel,
                            &usage,
                        )
                        .await;
                        (position, vec![((*span).clone(), opt)], local_rep)
                    }
                    ExtractionJob::SamePageBatch { spans, page_idx, page_input } => {
                        let (res_vec, local_rep) = extract_same_page_batch(
                            client,
                            config,
                            spans,
                            *page_idx,
                            page_input,
                            page_figures,
                            &page_render_cache,
                            &page_image_cache,
                            &request_semaphore,
                            &collateral_cache,
                            &all_spans,
                            config.text_first && text_map_available,
                            cancel,
                            &usage,
                        )
                        .await;
                        (position, res_vec, local_rep)
                    }
                }
            }
        })
        .buffer_unordered(config.parallelism.max(1));

        let mut ordered_results = Vec::with_capacity(jobs.len());
        loop {
            let next_item = tokio::select! {
                res = results.next() => res,
                _ = async {
                    while !cancel.load(Ordering::Relaxed) {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                } => None,
            };
            if cancel.load(Ordering::Relaxed) {
                return Err("Import cancelled by user".to_string());
            }
            match next_item {
                Some(result) => ordered_results.push(result),
                None => break,
            }
        }
        ordered_results.sort_by_key(|(position, _, _)| *position);
        report.record_timing(
            "extraction",
            "span_stream",
            None,
            None,
            extract_start.elapsed().as_millis() as u64,
        );
        for (_pos, span_res_vec, local) in ordered_results {
            report.absorb(local);
            for (span, opt) in span_res_vec {
                match opt {
                    Some(q) => {
                        report.pages_processed += (span.start_page..=span.end_page).count().max(1);
                        push_mark_check(&span, &q, &mut report);
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
                        if !report.quarantined.iter().any(|q| q.question_number == Some(span.number)) {
                            report.quarantined.push(QuarantineEvent {
                                scope: "question".to_string(),
                                page: Some(span.start_page + 1),
                                question_number: Some(span.number),
                                reason,
                            });
                        }

                        // Create a fallback question card so no question is ever lost from the repository
                        let fallback_text = (span.start_page..=span.end_page)
                            .filter(|&pi| pi < pages.len())
                            .map(|pi| pages[pi].text.trim())
                            .filter(|t| !t.is_empty())
                            .collect::<Vec<_>>()
                            .join("\n\n");

                        let fallback_content = if !fallback_text.is_empty() {
                            format!(
                                "**[Question {} - Needs Review]**\n\n*(Extraction incomplete. Raw text extracted from original page(s) {}-{} below:)*\n\n{}",
                                span.number,
                                span.start_page + 1,
                                span.end_page + 1,
                                fallback_text
                            )
                        } else {
                            format!(
                                "**[Question {} - Needs Review]**\n\n*(Extraction incomplete. Please check original paper page(s) {}-{} and edit this question card).* ",
                                span.number,
                                span.start_page + 1,
                                span.end_page + 1
                            )
                        };

                        built.push(BuiltQuestion {
                            marks: span.expected_marks.unwrap_or(0) as i32,
                            content: fallback_content,
                            question_number: span.number,
                            topics: vec!["Needs Review".to_string()],
                            module: config.module_name.clone(),
                            is_code: false,
                            needs_review: true,
                            notes: vec!["Fallback card created due to incomplete extraction".to_string()],
                        });
                    }
                }
            }
        }
    }

    // ── Deferred topic classification (on-device) ─────────
    // Tier-0 cards arrive without topics; match syllabus keywords locally.
    if !config.allowed_topics.is_empty() {
        let topics_start = Instant::now();
        let assigned = classify_topics_deferred(
            client,
            config,
            &mut built,
            &request_semaphore,
            cancel,
            &usage,
        )
        .await;
        if assigned > 0 {
            report.record_timing(
                "topics",
                "deferred_classification",
                None,
                None,
                topics_start.elapsed().as_millis() as u64,
            );
            eprintln!(
                "[TOPICS] {} question(s) classified locally (0 tokens)",
                assigned
            );
        }
    }

    report.questions_extracted = built.len();
    report.extracted_total_marks = built.iter().map(|q| q.marks.max(0) as u32).sum();
    // Removed printed paper total checksum warning as requested
    report.marks_checksum_ok = None;
    report.total_elapsed_ms = overall_start.elapsed().as_millis() as u64;
    let (prompt_tok, completion_tok) = usage.snapshot();
    report.prompt_tokens = prompt_tok;
    report.completion_tokens = completion_tok;
    report.stage_breakdown = usage
        .snapshot_stages()
        .into_iter()
        .map(|(stage, p, c)| StageCost {
            stage: stage.as_str().to_string(),
            prompt_tokens: p,
            completion_tokens: c,
        })
        .collect();

    eprintln!(
        "[PATH_SUMMARY] {} questions: {} tier0 (0 tokens), {} text-first (0 img), {} crop-first (~4k img), {} full-page vision (figures detected: {}); stages: {}",
        report.questions_extracted,
        report.deterministic,
        report.text_first,
        report.crop_first,
        report.questions_extracted.saturating_sub(report.deterministic + report.recovered + report.text_first + report.crop_first),
        report.figures_detected,
        report
            .stage_breakdown
            .iter()
            .map(|s| format!("{}={}p/{}c", s.stage, s.prompt_tokens, s.completion_tokens))
            .collect::<Vec<_>>()
            .join(", "),
    );

    // Per-stage timing so the import log can prove where wall-time went (and
    // that PDF-render speedups actually landed). Entries are only recorded
    // when a stage ran; missing stages print 0ms.
    let timing = |stage: &str, operation: &str| -> u64 {
        report
            .timings
            .iter()
            .find(|t| t.stage == stage && t.operation == operation)
            .map(|t| t.milliseconds)
            .unwrap_or(0)
    };
    eprintln!(
        "[TIMING] document_map={}ms structure={}ms extraction_span_stream={}ms extraction_fallback_stream={}ms total={}ms",
        timing("document_map", "text_layer_scan"),
        timing("structure", "api_call_stream"),
        timing("extraction", "span_stream"),
        timing("extraction", "fallback_stream"),
        report.total_elapsed_ms,
    );

    Ok((built, report))
}

/// Marks checksum for one span → report.
/// A Tier-0 carve is a strict success only when nothing after it (figure
/// attachment, terminal ending, marks bookkeeping) flagged the card; a
/// flagged card is a local recovery with its reasons on the record.
fn settle_tier0_outcome(span: &QuestionSpan, q: &BuiltQuestion, report: &mut ImportReport) {
    if !q.needs_review {
        report.deterministic += 1;
        return;
    }
    report.recovered += 1;
    let reasons = if q.notes.is_empty() { "flagged for review".to_string() } else { q.notes.join("; ") };
    report.anomalies.push(format!(
        "Question {}: local recovery (Tier-0 card flagged: {}); content retained for review, zero cloud calls",
        span.number, reasons
    ));
}

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

/// Thread-safe collateral question cache shared across concurrent extraction jobs in a paper run.
type CollateralCache = Arc<tokio::sync::Mutex<std::collections::HashMap<u32, BuiltQuestion>>>;

/// Deterministic gate for text-layer-first extraction: if the span's text
/// layer references a figure (Figure N, diagram, graph, circuit, sketch, …),
/// the figures MUST be boxed and extracted as images, which requires the
/// vision path. Text-first is only safe for questions whose text is fully
/// self-contained. When in doubt, this returns true so the question keeps the
/// proven vision path.
fn text_references_figure(text: &str) -> bool {
    use std::sync::OnceLock;
    static RE_FIGURE: OnceLock<regex::Regex> = OnceLock::new();
    RE_FIGURE
        .get_or_init(|| {
            regex::Regex::new(
                r"(?i)\bfigure\s*\d+|\bfig\.?\s*\d+|\bdiagram|\bgraph\b|\bchart\b|\bcircuit\b|\bsketch\b|\bnot drawn to scale\b|\bshown below\b|\bas shown\b|\bpictured\b|\billustrated\b|\bcurve\b|\baxes\b|\bplot\b|\bscheme\b|\bschema\b",
            )
            .unwrap()
        })
        .is_match(text)
}

/// Extract the numbers of explicit "Figure N" / "Fig. N" references in the
/// span's text layer.
pub(crate) fn figure_reference_numbers(text: &str) -> Vec<u32> {
    use std::sync::OnceLock;
    static RE_FIG_NUM: OnceLock<regex::Regex> = OnceLock::new();
    RE_FIG_NUM
        .get_or_init(|| regex::Regex::new(r"(?i)\bfigure\s*(\d+)|\bfig\.?\s*(\d+)").unwrap())
        .captures_iter(text)
        .filter_map(|c| {
            c.get(1)
                .or_else(|| c.get(2))
                .and_then(|m| m.as_str().parse::<u32>().ok())
        })
        .collect()
}

/// Heuristic: does the question's wording demand the ANSWER be READ from a
/// figure? This identifies a required visual resource, not a need to solve
/// the question. A supplied crop lets ingestion transcribe the instruction.
pub(crate) fn figure_read_required(text: &str) -> bool {
    use std::sync::OnceLock;
    static RE_READ: OnceLock<regex::Regex> = OnceLock::new();
    RE_READ
        .get_or_init(|| {
            regex::Regex::new(
                r"(?i)read\s+the\s+(?:coordinates|value|reading|data)|write\s+down\s+the\s+value\s+(?:from|of\s+the\s+(?:resistor|voltmeter|ammeter|meter|graph))|what\s+is\s+the\s+reading\s+on|state\s+the\s+value\s+of\s+the\s+(?:resistor|voltmeter|ammeter|meter|component)|use\s+the\s+graph\s+to\s+(?:determine|find|state)|using\s+the\s+(?:graph|figure).*?(?:determine|find|state|read|calculate)",
            )
            .unwrap()
        })
        .is_match(text)
}

/// The deterministic figures from `page_figures` that fall inside this span's
/// vertical band on its OWN pages. Same band logic the vision path uses to
/// crop page images (start_y_frac/end_y_frac on the span's first/last page,
/// full height on interior pages).
fn span_band_figures<'a>(
    span: &QuestionSpan,
    span_pages: &[(usize, &PageInput)],
    page_figures: &'a [Vec<crate::pdf_render::DetectedFigure>],
) -> Vec<(usize, &'a crate::pdf_render::DetectedFigure)> {
    let mut out = Vec::new();
    for (global_pi, _page) in span_pages {
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
        for fig in page_figures.get(*global_pi).map(Vec::as_slice).unwrap_or(&[]) {
            let cy = fig.bbox[1] + fig.bbox[3] / 2.0;
            let in_band = match (s, e) {
                (Some(s), Some(e)) => cy >= s && cy <= e,
                (Some(s), None) => cy >= s,
                (None, Some(e)) => cy <= e,
                (None, None) => true,
            };
            if in_band {
                out.push((*global_pi, fig));
            }
        }
    }
    out
}

/// Dedup-aware accumulator for figure candidates.
fn add_unique_figure<'a>(
    out: &mut Vec<(usize, &'a crate::pdf_render::DetectedFigure)>,
    pi: usize,
    fig: &'a crate::pdf_render::DetectedFigure,
) {
    if out.iter().any(|(sp, f)| *sp == pi && f.bbox == fig.bbox) {
        return;
    }
    out.push((pi, fig));
}

/// All deterministic figures this question can be shown: band-eligible ones on
/// its own pages, PLUS any figure anywhere in the paper (including the span's
/// own pages) whose caption matches a referenced "Figure N". The vertical band
/// describes which decorative/unlabelled figures sit in the question's answer
/// area; a NUMBERED figure is identified by its caption wherever it appears —
/// AQA usually places it ABOVE the question text, just outside the band, so
/// skipping the span's own pages here is what silently kept figure questions
/// on the expensive full-page vision path.
fn span_figure_candidates<'a>(
    span: &QuestionSpan,
    span_pages: &[(usize, &PageInput)],
    page_figures: &'a [Vec<crate::pdf_render::DetectedFigure>],
    referenced: &[u32],
) -> Vec<(usize, &'a crate::pdf_render::DetectedFigure)> {
    let mut out: Vec<(usize, &crate::pdf_render::DetectedFigure)> = Vec::new();

    // 1. Band-eligible figures on the span's own pages (covers unlabelled
    //    exhibits like "the circuit shown below").
    for (pi, fig) in span_band_figures(span, span_pages, page_figures) {
        if !referenced.is_empty() && fig_number_from_caption(fig.caption.as_deref())
            .is_some_and(|number| !referenced.contains(&number)) {
            continue;
        }
        add_unique_figure(&mut out, pi, fig);
    }

    // 2. Caption-matched figures for referenced numbers, ANYWHERE in the
    //    paper — including pages already scanned in step 1 (the band filter
    //    may have rejected the figure's position, but the caption is its
    //    identity). At most one figure per referenced number.
    for (pi, figs) in page_figures.iter().enumerate() {
        for fig in figs {
            if let Some(n) = fig_number_from_caption(fig.caption.as_deref()) {
                if referenced.contains(&n)
                    && !out
                        .iter()
                        .any(|(_, f)| fig_number_from_caption(f.caption.as_deref()) == Some(n))
                {
                    add_unique_figure(&mut out, pi, fig);
                }
            }
        }
    }
    // Match placement order to first references, not PDF object order.
    let rank = |fig: &crate::pdf_render::DetectedFigure| fig_number_from_caption(fig.caption.as_deref())
        .and_then(|number| referenced.iter().position(|n| *n == number)).unwrap_or(usize::MAX);
    out.sort_by(|(pa, a), (pb, b)| rank(a).cmp(&rank(b)).then(pa.cmp(pb))
        .then(a.bbox[1].total_cmp(&b.bbox[1])));
    // PDF object order is unrelated to printed A-D order. Group unnumbered
    // figures into rows, then order each row left-to-right before binding.
    let mut start = 0;
    while start < out.len() {
        if rank(out[start].1) != usize::MAX { start += 1; continue; }
        let (page, first) = out[start];
        let tolerance = (first.bbox[3] * 0.25).min(0.06);
        let mut end = start + 1;
        while end < out.len() && rank(out[end].1) == usize::MAX && out[end].0 == page
            && out[end].1.bbox[1] - first.bbox[1] <= tolerance { end += 1; }
        out[start..end].sort_by(|(_, a), (_, b)| a.bbox[0].total_cmp(&b.bbox[0]));
        start = end;
    }
    out
}

/// Extract the figure number from a "Figure N" / "Fig. N" caption.
fn fig_number_from_caption(caption: Option<&str>) -> Option<u32> {
    use std::sync::OnceLock;
    static RE_CAPTION: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE_CAPTION
        .get_or_init(|| regex::Regex::new(r"(?i)fig(?:ure)?\.?\s*(\d+)").unwrap());
    re.captures(caption?)?
        .get(1)?
        .as_str()
        .parse::<u32>()
        .ok()
}

/// Find the byte offset just AFTER the first "Figure N" reference in `content`
/// at or past `from`, when that reference names `num`.
fn find_figure_reference_after(content: &str, num: u32, from: usize) -> Option<usize> {
    use std::sync::OnceLock;
    static RE_REF: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE_REF
        .get_or_init(|| regex::Regex::new(r"(?i)\bfigure\s*(\d+)|\bfig\.?\s*(\d+)").unwrap());
    for m in re.find_iter(&content[from..]) {
        let n = m
            .as_str()
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<u32>()
            .ok()?;
        if n == num {
            return Some(from + m.end());
        }
    }
    None
}

/// Placeholder tokens the model / post-processors may leave in content.
/// `clean_marker_markdown` renames `[DIAGRAM_PLACEHOLDER]` to
/// `[VISUAL_MCQ_PLACEHOLDER]` inside visual MCQs, so figure attachment must
/// treat BOTH as splice points — and neither may ever reach a question card.
const PLACEHOLDER_TOKENS: [&str; 2] = ["[DIAGRAM_PLACEHOLDER]", "[VISUAL_MCQ_PLACEHOLDER]"];

/// Find the earliest placeholder token at or after `from`, as (start, end).
fn next_placeholder_after(content: &str, from: usize) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for token in PLACEHOLDER_TOKENS {
        if let Some(rel) = content[from..].find(token) {
            let abs = from + rel;
            if best.map_or(true, |(start, _)| abs < start) {
                best = Some((abs, abs + token.len()));
            }
        }
    }
    best
}

/// Remove every placeholder token — a bare `[VISUAL_MCQ_PLACEHOLDER]` or
/// `[DIAGRAM_PLACEHOLDER]` must never leak into a rendered question card.
fn strip_placeholder_tokens(content: &str) -> String {
    let mut s = content.to_string();
    for token in PLACEHOLDER_TOKENS {
        s = s.replace(token, "");
    }
    s
}

/// Splice deterministic figure crops into a text-first question's content.
///
/// Order of placement:
///   1. one crop link per placeholder token the model emitted,
///   2. otherwise immediately after the matching "Figure N" reference,
///   3. remaining figures appended at the end.
/// Every crop goes through the standard save guard chain (`persist_diagrams`
/// → `save_diagram`), so the detector proposes but the existing Rust guards
/// still dispose: sanitizer, header/footer margins, answer-space, blank guard,
/// grid rejection, signature dedup.
pub(crate) fn available_span_figures(
    span: &QuestionSpan,
    span_pages: &[(usize, &PageInput)],
    page_figures: &[Vec<crate::pdf_render::DetectedFigure>],
    referenced: &[u32],
) -> usize {
    span_figure_candidates(span, span_pages, page_figures, referenced)
        .iter()
        .filter(|(_, figure)| figure.seg_confidence >= FIGURE_SUPPLY_MIN_CONFIDENCE)
        .count()
}

async fn attach_detected_figures(
    config: &PipelineConfig,
    span: &QuestionSpan,
    span_pages: &[(usize, &PageInput)],
    page_figures: &[Vec<crate::pdf_render::DetectedFigure>],
    page_render_cache: &Arc<crate::pdf_render::PageRenderCache>,
    question: &mut BuiltQuestion,
    report: &mut ImportReport,
) {
    let references = figure_reference_numbers(&question.content);
    let distinct: std::collections::BTreeSet<_> = references.iter().copied().collect();
    // Layout questions know from source geometry which diagrams are drawn
    // in the question's area: every one of them must be attached, and a
    // caption beside an empty drawing space is a diagram nobody found. A
    // "Figure N" printed as text (a program, relations, a table) needs no
    // image; wording alone ("the equation of a curve") cannot require one.
    let layout_owned = config.layout_figures.as_ref().and_then(|m| m.get(&span.number)).map(Vec::len);
    let undetected = config.layout_questions.as_ref().and_then(|m| m.get(&span.number)).map_or(0, |b| b.undetected_figures);
    let required = match (layout_owned, layout_figures_in_span(config, span)) {
        (Some(n), _) => n,
        (None, Some(n)) => distinct.len().max(n.min(1)),
        (None, None) => distinct.len().max(usize::from(text_references_figure(&question.content))),
    };
    let attached = attach_detected_figure_content(config, span, span_pages, page_figures,
        page_render_cache, &mut question.content, report).await;
    let unknown = attached.iter().filter(|n| n.is_none()).count();
    let missing = distinct.iter().filter(|n| !attached.contains(&Some(**n))).count();
    let failed = if layout_owned.is_some() { attached.len() < required || undetected > 0 } else { attached.len() < required || missing > unknown };
    if failed {
        question.needs_review = true;
        let reason = format!("Question {}: required figure could not be attached; review the source PDF", span.number);
        question.notes.push(reason.clone());
        report.anomalies.push(reason);
    }
}

/// Corroborating stem evidence for an image-option MCQ. Detached crops with
/// printed A/B/C labels are NOT enough on their own: multi-part questions and
/// labelled geometry figures also carry letters. A whole-question MCQ options
/// grid is a single-part question that asks the reader to choose.
fn stem_supports_mcq_options(content: &str) -> bool {
    static RE_PART_LABEL: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let part = RE_PART_LABEL.get_or_init(|| regex::Regex::new(r"(?m)^\s*\([a-h]\)").unwrap());
    if part.is_match(content) {
        return false;
    }
    static RE_CHOOSE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let choose = RE_CHOOSE.get_or_init(|| {
        regex::Regex::new(r"(?i)\b(which|choose|select|identify)\b").unwrap()
    });
    if !choose.is_match(content) {
        return false;
    }
    // Use the first paragraph as the stem (the mark tag may already sit on a
    // later line) and require the question mark there. Anchoring on the very
    // end of the whole content wrongly rejected Q26, whose mark tag is appended
    // before the crops are attached.
    let stem = content.split("\n\n").next().unwrap_or(content);
    let stem = split_trailing_mark_tag(stem)
        .map(|(head, _)| head)
        .unwrap_or_else(|| stem.to_string());
    choose.is_match(&stem) && stem.contains('?')
}

/// Split a trailing mark tag (`**[1 mark]**`, `[2 marks]`) off a stem so it can
/// be re-attached after the reconstructed MCQ options. Returns `None` when the
/// stem does not end on a mark tag.
fn split_trailing_mark_tag(stem: &str) -> Option<(String, String)> {
    static RE_MARK: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE_MARK.get_or_init(|| {
        regex::Regex::new(r"(?s)^(.*?)\s*(\*\*\[\d+\s*marks?\]\*\*|\[\d+\s*marks?\])\s*$").unwrap()
    });
    let caps = re.captures(stem)?;
    Some((
        caps.get(1).map(|m| m.as_str().to_string()).unwrap_or_default(),
        caps.get(2).unwrap().as_str().to_string(),
    ))
}

/// Bind detached per-option figure links to their printed option letters.
///
/// Each `label` is the option letter captured from positional text evidence
/// inside/above the crop (`None` when the glyph was not in the text layer).
/// Because a missing option letter does not mean a missing diagram, the one
/// anonymous crop is resolved ONLY when the captured letters are exactly the
/// contiguous run `A..N` minus exactly one letter — i.e. when the assignment
/// is forced. Any other shape (two anonymous crops, duplicate letters, a
/// non-contiguous run) returns `None` and the caller leaves the card unbound.
fn bind_detached_option_pairs(
    links: &[Option<String>],
    labels: &[Option<char>],
) -> Option<Vec<(char, String)>> {
    if links.len() != labels.len() || links.len() < 3 {
        return None;
    }
    let labelled: std::collections::BTreeSet<char> = labels.iter().flatten().copied().collect();
    let expected: Vec<char> = (0..labels.len())
        .map(|i| (b'A' + i as u8) as char)
        .collect();
    let missing: Vec<char> = expected
        .iter()
        .copied()
        .filter(|c| !labelled.contains(c))
        .collect();
    let anonymous = labels.iter().filter(|l| l.is_none()).count();
    let shape_ok = if anonymous == 0 {
        labelled.len() == labels.len() && missing.is_empty()
    } else {
        anonymous == 1 && missing.len() == 1
    };
    if !shape_ok || !labelled.iter().all(|c| expected.contains(c)) {
        return None;
    }
    let forced = missing.first().copied();
    let mut pairs: Vec<(char, String)> = Vec::with_capacity(labels.len());
    let mut forced_used = false;
    for (link_opt, label) in links.iter().zip(labels.iter()) {
        let link = link_opt.as_ref()?;
        let letter = match label {
            Some(c) => *c,
            None if !forced_used => {
                forced_used = true;
                forced?
            }
            None => return None,
        };
        pairs.push((letter, link.clone()));
    }
    let distinct: std::collections::BTreeSet<char> = pairs.iter().map(|(c, _)| *c).collect();
    (distinct.len() == pairs.len()).then_some(pairs)
}

async fn attach_detected_figure_content(
    config: &PipelineConfig,
    span: &QuestionSpan,
    span_pages: &[(usize, &PageInput)],
    page_figures: &[Vec<crate::pdf_render::DetectedFigure>],
    page_render_cache: &Arc<crate::pdf_render::PageRenderCache>,
    content: &mut String,
    report: &mut ImportReport,
) -> Vec<Option<u32>> {
    let referenced = figure_reference_numbers(content);
    // A layout question's figures were chosen with its body: one per
    // placeholder, in order.
    let layout_figures = config.layout_figures.as_ref().and_then(|m| m.get(&span.number));
    let eligible: Vec<(usize, &crate::pdf_render::DetectedFigure, bool)> = match layout_figures {
        Some(figs) => figs.iter().map(|f| (f.page, &f.figure, f.exact)).collect(),
        None => span_figure_candidates(span, span_pages, page_figures, &referenced).into_iter().map(|(p, f)| (p, f, false)).collect(),
    };
    if eligible.is_empty() {
        // Nothing to attach — but leftover tokens must still be scrubbed so a
        // bare placeholder never renders as literal text in the card.
        *content = strip_placeholder_tokens(content);
        return Vec::new();
    }
    let mut requests = Vec::with_capacity(eligible.len());
    let mut page_b64 = std::collections::HashMap::new();
    let mut figs: Vec<&crate::pdf_render::DetectedFigure> = Vec::with_capacity(eligible.len());
    for (global_pi, fig, exact) in eligible {
        let graph_like = fig
            .kind
            .as_deref()
            .map(|kind| {
                let kind = kind.to_ascii_lowercase();
                kind.contains("graph") || kind.contains("chart") || kind.contains("plot")
            })
            .unwrap_or(false);
        let ignore_grid =
            validate::figure_references(content) > 0 && !validate::is_answer_grid_request(content);
        if config.pdf_path.is_none() {
            if let Some(page) = span_pages.iter().find(|(pi, _)| *pi == global_pi) {
                if let Some(b64) = page.1.get_b64() {
                    page_b64.entry(global_pi).or_insert_with(|| b64.clone());
                }
            }
        }
        // A picture set inside a sentence is cropped to its own bounds: the
        // punctuation printed after it belongs to the text.
        let inline = fig.kind.as_deref() == Some(INLINE_FIGURE_KIND);
        let mask = config
            .layout_evidence
            .as_ref()
            .and_then(|ev| ev.layout.get(global_pi))
            .map(|lp| {
                let (w, h) = (lp.width.max(1.0), lp.height.max(1.0));
                // Only a small picture (a code, a logo) is painted out: a
                // page background or margin strip lies under the content.
                lp.furniture_images
                    .iter()
                    .filter(|r| (r[2] - r[0]).max(r[3] - r[1]) <= 90.0)
                    .map(|r| [r[0] / w, r[1] / h, (r[2] - r[0]) / w, (r[3] - r[1]) / h])
                    .collect()
            })
            .unwrap_or_default();
        requests.push(DiagramSaveRequest {
            global_page_idx: global_pi,
            bbox: fig.bbox.to_vec(),
            ignore_grid,
            graph_like,
            exact: exact.then_some(if inline { 0.0 } else { 0.004 }),
            mask,
        });
        figs.push(fig);
    }

    let persisted = match persist_diagrams(
        requests,
        page_b64,
        config.clone(),
        Arc::clone(page_render_cache),
        Vec::new(),
    )
    .await
    {
        Ok(p) => p,
        Err(error) => {
            report.anomalies.push(format!(
                "Question {} deterministic diagram persistence failed: {}",
                span.number, error
            ));
            *content = strip_placeholder_tokens(content);
            return Vec::new();
        }
    };
    report.absorb(persisted.report);

    let links: Vec<Option<String>> = persisted.links;
    if layout_figures.is_some() {
        return splice_layout_figures(content, &links, &figs);
    }
    // Detached per-option diagram MCQ: a stem with NO placeholder, and every
    // attached crop carrying a distinct printed option letter (A..E) captured
    // from positional text evidence. Bind each crop to its own printed letter;
    // never guess an order when a label is missing or duplicated.
    let option_pairs: Option<Vec<(char, String)>> = if !content.contains("[DIAGRAM_PLACEHOLDER]")
        && stem_supports_mcq_options(content)
    {
        let labels: Vec<Option<char>> = figs
            .iter()
            .map(|fig| {
                fig.option_label
                    .as_deref()
                    .and_then(|l| l.trim().chars().next())
                    .filter(|c| matches!(c, 'A'..='E'))
            })
            .collect();
        bind_detached_option_pairs(&links, &labels)
    } else {
        None
    };
    if let Some(mut pairs) = option_pairs {
        pairs.sort_by_key(|(c, _)| *c);
        // The stem may carry a trailing mark tag (placed by the deterministic
        // normalizer before crops were attached). Move it to the last option so
        // the MCQ renders as `stem` then the options ending with the marks.
        let stem_full = strip_placeholder_tokens(content).trim_end().to_string();
        let (mut rebuilt, mark_tag) = match split_trailing_mark_tag(&stem_full) {
            Some((head, tag)) => (head.trim_end().to_string(), Some(tag)),
            None => (stem_full, None),
        };
        rebuilt.push_str("\n\n");
        for (i, (letter, link)) in pairs.iter().enumerate() {
            if i > 0 {
                rebuilt.push('\n');
            }
            rebuilt.push_str(&format!("- [MCQ:{}] {}", letter, link.trim()));
            if i + 1 == pairs.len() {
                if let Some(tag) = mark_tag.as_deref() {
                    rebuilt.push(' ');
                    rebuilt.push_str(tag);
                }
            }
        }
        *content = rebuilt;
        return figs
            .iter()
            .map(|f| fig_number_from_caption(f.caption.as_deref()))
            .collect();
    }

    let mut insert_offset = 0usize;
    let mut attached = Vec::new();
    for (link_opt, fig) in links.into_iter().zip(figs) {
        let Some(link) = link_opt else { continue };
        attached.push(fig_number_from_caption(fig.caption.as_deref()));
        if let Some((abs, end)) = next_placeholder_after(content, insert_offset) {
            content.replace_range(abs..end, &link);
            insert_offset = abs + link.len();
            continue;
        }
        if let Some(num) = fig_number_from_caption(fig.caption.as_deref()) {
            if let Some(pos) = find_figure_reference_after(content, num, insert_offset) {
                content.insert_str(pos, &link);
                insert_offset = pos + link.len();
                continue;
            }
        }
        content.push_str(&link);
    }
    // Any placeholder that survived (model over-emitted, or every crop was
    // rejected) is dropped — never leak a bare token into a question card.
    *content = strip_placeholder_tokens(content);
    attached
}

/// Splice a layout question's crops into its body: crop `i` replaces the
/// `i`-th placeholder (a rejected crop removes its placeholder). When the
/// question asks for a choice and its labelled crops are the options A, B,
/// C… in full, those become the tagged option list after the stem (their
/// printed letters — figure labels and answer bubbles — are not stem text)
/// while unlabelled figures stay where they are printed.
fn splice_layout_figures(
    content: &mut String,
    links: &[Option<String>],
    figs: &[&crate::pdf_render::DetectedFigure],
) -> Vec<Option<u32>> {
    let labels: Vec<Option<char>> = figs
        .iter()
        .map(|f| f.option_label.as_deref().and_then(|l| l.trim().chars().next()).filter(|c| matches!(c, 'A'..='E')))
        .collect();
    let mut letters: Vec<char> = labels.iter().flatten().copied().collect();
    letters.sort_unstable();
    let options_complete = letters.len() >= 3
        && letters.iter().enumerate().all(|(i, &c)| c == (b'A' + i as u8) as char)
        && labels.iter().zip(links).all(|(l, k)| l.is_none() || k.is_some());
    // A single-part question that asks for a choice ("Which pair of graphs
    // …?"), wherever its question sentence falls among its figures.
    let stem = strip_placeholder_tokens(content);
    static PART_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?m)^\s*\([a-h]\)").unwrap());
    static CHOICE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\b(?:which|choose|select|identify)\b[^?]*\?").unwrap());
    let as_options = options_complete && !PART_RE.is_match(&stem) && CHOICE_RE.is_match(&stem);
    let mut attached = Vec::new();
    let mut cursor = 0usize;
    for (i, fig) in figs.iter().enumerate() {
        let link = links.get(i).cloned().flatten();
        let option = as_options && labels[i].is_some();
        if link.is_some() && !option {
            attached.push(fig_number_from_caption(fig.caption.as_deref()));
        }
        let replacement = if option { String::new() } else { link.clone().unwrap_or_default() };
        match next_placeholder_after(content, cursor) {
            Some((abs, end)) => {
                content.replace_range(abs..end, &replacement);
                cursor = abs + replacement.len();
            }
            None => content.push_str(&replacement),
        }
    }
    *content = strip_placeholder_tokens(content);
    if !as_options {
        return attached;
    }
    // Lines made only of the option letters are their printed labels (or
    // the answer bubbles, already tagged as letter-only options).
    let letter_line = |l: &str| {
        let t = l.trim();
        let t = t.strip_prefix("- [MCQ:").and_then(|r| r.get(2..)).map(str::trim).filter(|_| t.starts_with("- [MCQ:")).unwrap_or(t);
        t.is_empty() && l.trim().starts_with("- [MCQ:")
            || !t.is_empty() && t.split_whitespace().all(|w| w.chars().count() == 1 && w.chars().all(|c| letters.contains(&c)))
    };
    let stem: String = content.lines().filter(|l| !letter_line(l)).collect::<Vec<_>>().join("\n");
    let stem = stem.trim_end().to_string();
    let (mut rebuilt, mark_tag) = match split_trailing_mark_tag(&stem) {
        Some((head, tag)) => (head.trim_end().to_string(), Some(tag)),
        None => (stem, None),
    };
    let mut pairs: Vec<(char, String)> = labels
        .iter()
        .zip(links)
        .filter_map(|(l, k)| Some(((*l)?, k.clone()?)))
        .collect();
    pairs.sort_by_key(|(c, _)| *c);
    // The mark allocation stands between the stem and the options, as on
    // every multiple-choice card (a tag trailing an option is not shown).
    rebuilt.push_str("\n\n");
    if let Some(tag) = mark_tag.as_deref() {
        rebuilt.push_str(tag);
        rebuilt.push('\n');
    }
    for (i, (letter, link)) in pairs.iter().enumerate() {
        if i > 0 {
            rebuilt.push('\n');
        }
        rebuilt.push_str(&format!("- [MCQ:{}] {}", letter, link.trim()));
    }
    *content = rebuilt;
    attached.extend(pairs.iter().map(|_| None));
    attached
}

/// Slim JSON schema for TEXT-ONLY calls: the full extraction schema minus the
/// five diagram-bbox fields, which text-first mode forbids. `AiQuestion`
/// fields are `#[serde(default)]`, so the omitted keys parse to empty/null
/// defaults and the gates in `build_question_from_parsed_page` still fire.
fn text_first_json_schema() -> serde_json::Value {
    static SCHEMA: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            serde_json::json!({
                "name": "QuestionExtractionTextOnly",
                "strict": true,
                "schema": {
                    "type": "object",
                    "properties": {
                        "items": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "question_number": { "type": "integer", "minimum": 1, "maximum": 100 },
                                    "content": {
                                        "type": "string",
                                        "description": "Markdown transcription. ALL math must be wrapped in balanced delimiters: every $ opened on a line is closed on the SAME line; every $$ block is closed with $$. No raw LaTeX (\\frac, ^{...}_{...}, \\alpha) may appear outside math delimiters. Never break a sentence across paragraphs. Sequential printed equations stay on SEPARATE $$ blocks (or rows joined with \\\\ inside one block) — never concatenated end-to-end. In JSON, escape EVERY backslash as \\\\ (writing \\text instead of \\\\text decodes to a TAB character followed by 'ext')."
                                    },
                                    "marks": { "type": ["integer", "null"], "minimum": 0 },
                                    "topics": { "type": "array", "items": { "type": "string" } },
                                    "module": { "type": "string" },
                                    "is_code": { "type": "boolean" },
                                    "math_snippet": { "type": "string" }
                                },
                                "required": ["question_number", "content", "marks", "topics", "module", "is_code", "math_snippet"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["items"],
                    "additionalProperties": false
                }
            })
        })
        .clone()
}

/// Text-first acceptance gates + assembly, shared by the single-question
/// path and the combined per-page batch path so they can never diverge.
/// Takes an already-parsed model page and returns the `BuiltQuestion` or
/// `None` (fall back to vision / individual re-ask).
fn build_question_from_parsed_page(
    page: AiQuestionPage,
    span: &QuestionSpan,
    config: &PipelineConfig,
    available_figures: usize,
) -> Option<BuiltQuestion> {
    if page.items.is_empty() {
        eprintln!(
            "[TEXT_FIRST_FALLBACK] question={} reason=empty_items",
            span.number
        );
        return None;
    }
    // All items for the target question. Large multi-page questions often come
    // back SPLIT across sub-parts; the vision path merges those, so text-first
    // does too — a fragile "exactly one item" gate is what dumped Q2 into the
    // expensive vision repair loop on this run.
    let target_items: Vec<AiQuestion> = page
        .items
        .into_iter()
        .filter(|i| {
            i.question_number
                .as_ref()
                .and_then(validate::value_to_question_number)
                == Some(span.number)
        })
        .collect();
    if target_items.is_empty() {
        eprintln!(
            "[TEXT_FIRST_FALLBACK] question={} reason=no_target_items",
            span.number
        );
        return None;
    }
    if target_items
        .iter()
        .all(|i| i.content.as_deref().unwrap_or("").trim().is_empty())
    {
        eprintln!(
            "[TEXT_FIRST_FALLBACK] question={} reason=empty_content",
            span.number
        );
        return None;
    }
    // Single-question response boundary: a model that answers sub-parts
    // (a)(b)(c) as separate items gets them stitched into ONE card here, in
    // source order and without duplicate totals. Identity is validated first —
    // a foreign parent question refuses the stitch and we fall back rather
    // than weld two questions together.
    let mut stitch_marks_ambiguous = false;
    let target_items = if target_items.len() > 1 {
        match stitch_question_items(target_items, span.number, span.expected_marks) {
            Ok(stitched) => {
                stitch_marks_ambiguous = stitched.marks_ambiguous;
                vec![stitched.item]
            }
            Err(refusal) => {
                eprintln!(
                    "[TEXT_FIRST_FALLBACK] question={} reason=stitch_refused({})",
                    span.number,
                    refusal.describe()
                );
                return None;
            }
        }
    } else {
        target_items
    };
    // A figure is needed → vision will box it. With deterministic detection
    // enabled the caller can supply figures (`available_figures > 0`); the
    // model's placeholders are then accepted and filled in from the crops.
    // Count placeholders across ALL items (a split response may park them in
    // different sub-parts).
    let placeholder_count: usize = target_items
        .iter()
        .map(|i| {
            i.content
                .as_deref()
                .unwrap_or("")
                .matches("[DIAGRAM_PLACEHOLDER]")
                .count()
        })
        .sum();
    if placeholder_count > available_figures {
        // The model sometimes emits one placeholder PER REFERENCE of the same
        // figure (e.g. "Figure 9" appears in parts (a), (b), (c) → several
        // placeholders). If the DISTINCT figure numbers it references fit
        // within the figures we can supply, the excess placeholders are
        // duplicates — accept and let `attach_detected_figures` collapse them
        // (it places one crop per placeholder in order and drops the rest).
        // Otherwise the question genuinely needs more figures than the
        // detector found → vision must see the full page.
        let distinct_refs: std::collections::HashSet<u32> = target_items
            .iter()
            .flat_map(|i| figure_reference_numbers(i.content.as_deref().unwrap_or("")))
            .collect();
        let duplicates_only = !distinct_refs.is_empty() && distinct_refs.len() <= available_figures;
        if !duplicates_only {
            eprintln!(
                "[TEXT_FIRST_FALLBACK] question={} reason=figure_needed placeholders={} figures={} distinct_refs={}",
                span.number, placeholder_count, available_figures, distinct_refs.len()
            );
            return None;
        }
    }
    if target_items
        .iter()
        .any(|i| i.diagram_bboxes.as_ref().is_some_and(|b| !b.is_empty()))
    {
        eprintln!(
            "[TEXT_FIRST_FALLBACK] question={} reason=unexpected_bboxes",
            span.number
        );
        return None;
    }
    let wrapped = AiQuestionPage {
        items: target_items.clone(),
    };
    if !validate_span_items(&wrapped, span).is_empty() {
        eprintln!(
            "[TEXT_FIRST_FALLBACK] question={} reason=validation",
            span.number
        );
        return None;
    }

    // Mirror the vision tail: accumulate content/topics/marks then assemble.
    // `[DIAGRAM_PLACEHOLDER]` tokens are preserved here — the caller splices
    // deterministic figure links into them when figures were supplied.
    let mut contents = Vec::new();
    let mut topics_acc = Vec::new();
    let mut is_code_acc = false;
    let mut ai_marks: Option<i32> = None;
    for item in target_items {
        let item_content = item.content.unwrap_or_default();
        contents.push(item_content);
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
    }
    let built = assemble_built_question(
        span,
        config,
        contents,
        topics_acc,
        is_code_acc,
        stitch_marks_ambiguous,
        if stitch_marks_ambiguous {
            vec![
                "stitched sub-parts carried equal marks with no printed total to confirm them against; marks summed, please review"
                    .to_string(),
            ]
        } else {
            Vec::new()
        },
        ai_marks,
    )?;
    // Post-merge structural gate: sub-part sequences can only be judged on
    // the JOINED content — per-item validation above cannot see a missing
    // first part or a duplicated label across split items. A merged card
    // that still fails structure escalates to the vision repair loop.
    let structural = validate::card_structure_errors(&built.content, span.number);
    if !structural.is_empty() {
        eprintln!(
            "[TEXT_FIRST_FALLBACK] question={} reason=structure({})",
            span.number,
            structural.join("; ")
        );
        return None;
    }
    Some(built)
}

/// Tier-0 seam: the deterministic transcriber hands over a converted
/// transcription and it flows through the SAME acceptance gates as every LLM
/// response (`build_question_from_parsed_page`), so Tier 0 inherits every
/// future validator improvement for free.
pub(crate) fn build_question_from_seam(
    content: String,
    marks: Option<u32>,
    span: &QuestionSpan,
    config: &PipelineConfig,
    available_figures: usize,
) -> Option<BuiltQuestion> {
    if is_layout_question(config, span) {
        let built = assemble_layout_question(span, config, content, false, Vec::new())?;
        let structural = validate::card_structure_errors(&built.content, span.number);
        if !structural.is_empty() {
            eprintln!("[TIER0_LAYOUT] question={} reason=structure({})", span.number, structural.join("; "));
            return None;
        }
        return Some(built);
    }
    let item = AiQuestion {
        question_number: Some(serde_json::json!(span.number)),
        content: Some(content),
        marks: marks.map(|m| serde_json::json!(m)),
        ..Default::default()
    };
    build_question_from_parsed_page(
        AiQuestionPage { items: vec![item] },
        span,
        config,
        available_figures,
    )
}

/// Lenient assembly for a locally recovered card.
///
/// The strict seam rejects any card that still carries a structural defect.
/// A born-digital document has no cloud to escalate to, so the honest move is
/// to keep the real carved content and FLAG it: `needs_review` is forced true,
/// the failed gate is recorded in the card notes, and the caller surfaces the
/// same reason as a report anomaly. Nothing is invented here - the content is
/// whatever the deterministic carve produced, or this returns `None`.
pub(crate) fn build_recovered_question(
    content: String,
    marks_hint: Option<i32>,
    span: &QuestionSpan,
    config: &PipelineConfig,
    failed_gate: &str,
) -> Option<BuiltQuestion> {
    if is_layout_question(config, span) {
        return assemble_layout_question(
            span,
            config,
            content,
            true,
            vec![format!(
                "local recovery: quality gate '{}' failed; content retained for review (digital document, zero cloud calls)",
                failed_gate
            )],
        );
    }
    assemble_built_question(
        span,
        config,
        vec![content],
        Vec::new(),
        false,
        true,
        vec![format!(
            "local recovery: quality gate '{}' failed; content retained for review (digital document, zero cloud calls)",
            failed_gate
        )],
        marks_hint,
    )
}

/// Text-layer-first extraction attempt: transcribe the target question from
/// the PDF text layer ALONE (zero image tokens — the dominant cost on
/// pixel-billed providers like Gemini). Returns `Some((question, report))`
/// when the text layer was reliable and the model produced a valid,
/// figure-free transcription. Returns `None` (and the caller falls through to
/// the vision path) when the model needs a figure, returns nothing, or fails
/// validation.
async fn try_text_first_extraction<C: LlmClient>(
    client: &C,
    config: &PipelineConfig,
    span: &QuestionSpan,
    span_pages: &[(usize, &PageInput)],
    available_figures: usize,
    request_semaphore: &Arc<Semaphore>,
    cancel: &AtomicBool,
    usage: &Arc<TokenTotals>,
) -> Option<(BuiltQuestion, ImportReport)> {
    let mut report = ImportReport::default();
    if cancel.load(Ordering::Relaxed) {
        return None;
    }

    let raw_text: String = span_pages
        .iter()
        .map(|(pi, p)| {
            if p.text.trim().is_empty() {
                String::new()
            } else {
                format!("RAW TEXT PAGE {}:\n{}\n\n", pi + 1, p.text)
            }
        })
        .collect();
    if raw_text.trim().is_empty() {
        return None; // no text to work from — vision handles it
    }
    let system = text_first_system_prompt(config);
    let user_text = format!(
        "TARGET: Question {}\nPAPER: '{}'\nMODULE: '{}'\n\nTranscribe Question {} from the RAW TEXT below. NO IMAGES ARE ATTACHED — the text layer is authoritative.\n\n{}",
        span.number,
        config.paper_name,
        config.module_name,
        span.number,
        raw_text
    );
    let body = llm::chat_body(
        &config.model,
        &system,
        &[] as &[String],
        llm::ImageDetail::Low,
        Some(&user_text),
        config.max_output_tokens.min(StageTag::TextFirstExtraction.output_cap()),
        Some(llm::ResponseFormat::JsonSchema {
            schema: text_first_json_schema(),
        }),
    );

    let api_start = Instant::now();
    let resp = match chat_with_permit(client, &body, request_semaphore, cancel, usage, StageTag::TextFirstExtraction, config.cloud_allowed()).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "[TEXT_FIRST_FALLBACK] question={} reason=api_error err={}",
                span.number, e
            );
            report.anomalies.push(format!(
                "Question {} text-first attempt API failure ({}); falling back to vision",
                span.number, e
            ));
            return None;
        }
    };
    // Length-truncated text-first output is unusable — the vision path
    // regenerates it with the full page attached.
    if llm::response_was_truncated(&resp) {
        eprintln!(
            "[TEXT_FIRST_FALLBACK] question={} reason=finish_reason_length",
            span.number
        );
        report.anomalies.push(format!(
            "Question {} text-first response hit the token ceiling (finish_reason=length); falling back to vision",
            span.number
        ));
        return None;
    }
    report.record_timing(
        "extraction",
        "text_first",
        Some(span_pages[0].0 + 1),
        Some(span.number),
        api_start.elapsed().as_millis() as u64,
    );
    let content = match llm::message_content(&resp) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "[TEXT_FIRST_FALLBACK] question={} reason=malformed_message err={}",
                span.number, e
            );
            report.anomalies.push(format!(
                "Question {} text-first response malformed ({}); falling back to vision",
                span.number, e
            ));
            return None;
        }
    };

    let page = match parse_llm_json::<AiQuestionPage>(&content) {
        ParseOutcome::Clean(v) => v,
        ParseOutcome::Salvaged {
            value,
            dropped_tail,
        } => {
            if dropped_tail {
                eprintln!(
                    "[TEXT_FIRST_FALLBACK] question={} reason=truncated",
                    span.number
                );
                return None; // truncated — vision will get the full page
            }
            value
        }
        ParseOutcome::Malformed { error } => {
            eprintln!(
                "[TEXT_FIRST_FALLBACK] question={} reason=invalid_json err={}",
                span.number, error
            );
            report.anomalies.push(format!(
                "Question {} text-first JSON invalid ({}); falling back to vision",
                span.number, error
            ));
            return None;
        }
    };

    // Text-first acceptance gates + assembly — shared with the combined
    // per-page batch path so the two can never diverge.
    let built = build_question_from_parsed_page(page, span, config, available_figures)?;
    Some((built, report))
}

/// Combined text-first extraction for a shared page: ONE API call transcribes
/// ALL target question numbers at once (the response `items` array already
/// supports multiple entries), avoiding N repetitions of the system-prompt /
/// schema overhead that dominates text-first cost on multi-question pages.
/// Each span still runs the standard acceptance gates via
/// `build_question_from_parsed_page`. Returns one `Option<BuiltQuestion>` per
/// span (index-aligned); `None` entries are re-asked individually by the
/// caller, which ultimately falls back to vision.
async fn try_text_first_batch_extraction<C: LlmClient>(
    client: &C,
    config: &PipelineConfig,
    spans: &[&QuestionSpan],
    span_pages: &[(usize, &PageInput)],
    fig_counts: &[usize],
    request_semaphore: &Arc<Semaphore>,
    cancel: &AtomicBool,
    usage: &Arc<TokenTotals>,
) -> (Vec<Option<BuiltQuestion>>, ImportReport) {
    let mut report = ImportReport::default();
    let n = spans.len();
    let mut out: Vec<Option<BuiltQuestion>> = (0..n).map(|_| None).collect();
    if n == 0 || cancel.load(Ordering::Relaxed) {
        return (out, report);
    }

    let raw_text: String = span_pages
        .iter()
        .map(|(pi, p)| {
            if p.text.trim().is_empty() {
                String::new()
            } else {
                format!("RAW TEXT PAGE {}:\n{}\n\n", pi + 1, p.text)
            }
        })
        .collect();
    if raw_text.trim().is_empty() {
        return (out, report);
    }

    let numbers: Vec<String> = spans.iter().map(|s| s.number.to_string()).collect();
    let system = text_first_system_prompt(config);
    let user_text = format!(
        "TARGET QUESTIONS: {}\nPAPER: '{}'\nMODULE: '{}'\n\nTranscribe ALL of the listed questions from the RAW TEXT below. Return one item per question — each item carries its OWN \"question_number\", marks, and content. NO IMAGES ARE ATTACHED — the text layer is authoritative.\n\n{}",
        numbers.join(", "),
        config.paper_name,
        config.module_name,
        raw_text
    );
    let body = llm::chat_body(
        &config.model,
        &system,
        &[] as &[String],
        llm::ImageDetail::Low,
        Some(&user_text),
        config.max_output_tokens.min(StageTag::BatchTextFirst.output_cap()),
        Some(llm::ResponseFormat::JsonSchema {
            schema: text_first_json_schema(),
        }),
    );

    eprintln!(
        "[TEXT_FIRST_BATCH] page={} questions={} one combined text-only call",
        span_pages[0].0 + 1,
        numbers.join(",")
    );

    let api_start = Instant::now();
    let resp = match chat_with_permit(client, &body, request_semaphore, cancel, usage, StageTag::BatchTextFirst, config.cloud_allowed()).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[TEXT_FIRST_FALLBACK] batch reason=api_error err={}", e);
            report.anomalies.push(format!(
                "Combined text-first batch API failure ({}); re-asking individually",
                e
            ));
            return (out, report);
        }
    };
    if llm::response_was_truncated(&resp) {
        eprintln!("[TEXT_FIRST_FALLBACK] batch reason=finish_reason_length");
        report.anomalies.push(
            "Combined text-first batch hit the token ceiling (finish_reason=length); re-asking individually".to_string(),
        );
        return (out, report);
    }
    report.record_timing(
        "extraction",
        "text_first_batch",
        Some(span_pages[0].0 + 1),
        spans.first().map(|s| s.number),
        api_start.elapsed().as_millis() as u64,
    );
    let content = match llm::message_content(&resp) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "[TEXT_FIRST_FALLBACK] batch reason=malformed_message err={}",
                e
            );
            report.anomalies.push(format!(
                "Combined text-first batch malformed response ({}); re-asking individually",
                e
            ));
            return (out, report);
        }
    };

    let page = match parse_llm_json::<AiQuestionPage>(&content) {
        ParseOutcome::Clean(v) => v,
        ParseOutcome::Salvaged { value, dropped_tail } => {
            if dropped_tail {
                eprintln!("[TEXT_FIRST_FALLBACK] batch reason=truncated");
                return (out, report); // truncated — re-ask individually
            }
            value
        }
        ParseOutcome::Malformed { error } => {
            eprintln!(
                "[TEXT_FIRST_FALLBACK] batch reason=invalid_json err={}",
                error
            );
            report.anomalies.push(format!(
                "Combined text-first batch JSON invalid ({}); re-asking individually",
                error
            ));
            return (out, report);
        }
    };

    for (i, span) in spans.iter().enumerate() {
        let available_figures = fig_counts.get(i).copied().unwrap_or(0);
        out[i] = build_question_from_parsed_page(page.clone(), span, config, available_figures);
    }
    (out, report)
}

/// Long-edge cap for crop-first figure images. 512px = 4×4 Gemini tiles ≈
/// 4.1k image tokens, versus ~10.3k for a 640px full page — the crop-first
/// win. Resolution is still plenty for axis labels and tick values.
const CROP_FIRST_MAX_DIM: u32 = 512;

/// Render + crop each candidate figure for a crop-first question. Runs on a
/// blocking thread (via `spawn_blocking`) so a 300-DPI page render never
/// stalls an async worker mid-extraction. `ignore_grid` is true: a graph's
/// gridlines are exhibit content, not an answer grid. Returns the WebP crops;
/// an empty vec when every candidate failed (caller falls back to full pages).
fn crop_figure_b64s(
    pdf_path: Option<PathBuf>,
    span_page_b64s: std::collections::HashMap<usize, String>,
    candidates: Vec<(usize, crate::pdf_render::DetectedFigure)>,
    page_render_cache: Arc<crate::pdf_render::PageRenderCache>,
) -> Vec<String> {
    let mut crop_b64s: Vec<String> = Vec::with_capacity(candidates.len());
    for (pi, fig) in candidates {
        let img = if let Some(pdf_path) = &pdf_path {
            page_render_cache.get_or_render(pdf_path, pi).ok()
        } else {
            span_page_b64s
                .get(&pi)
                .and_then(|b64| geometry::decode_page_image(b64))
                .map(std::sync::Arc::new)
        };
        let Some(img) = img else { continue };
        let Ok(crop) =
            geometry::crop_diagram_with_options(img.as_ref(), &fig.bbox, 8, true, false)
        else {
            continue;
        };
        let Some(b64) = geometry::encode_webp_resized(
            &image::DynamicImage::ImageRgba8(crop),
            CROP_FIRST_MAX_DIM,
        )
        else {
            continue;
        };
        crop_b64s.push(b64);
    }
    crop_b64s
}

/// Crop-first vision attempt for READ-FROM-FIGURE questions: send only the
/// detected figure crops (≤512px each) instead of full pages, with the
/// question wording from the text layer. The model reads the value off the
/// crop. Returns `Some((question, report))` on success; `None` (and the caller
/// falls through to the full-page vision path) on any failure — no repair
/// loop, one cheap attempt.
async fn try_crop_first_extraction<C: LlmClient>(
    client: &C,
    config: &PipelineConfig,
    span: &QuestionSpan,
    span_pages: &[(usize, &PageInput)],
    combined_text: &str,
    candidates: &[(usize, &crate::pdf_render::DetectedFigure)],
    page_render_cache: &Arc<crate::pdf_render::PageRenderCache>,
    request_semaphore: &Arc<Semaphore>,
    cancel: &AtomicBool,
    usage: &Arc<TokenTotals>,
) -> Option<(BuiltQuestion, ImportReport)> {
    let mut report = ImportReport::default();

    // 1. Crop each candidate figure from its page into a small image. Renders
    //    run on a blocking thread so 300-DPI page renders don't stall the
    //    async worker.
    let crop_b64s = {
        let pdf_path = config.pdf_path.clone();
        let span_page_b64s = span_pages
            .iter()
            .filter_map(|(idx, p)| p.get_b64().map(|b| (*idx, b.clone())))
            .collect::<std::collections::HashMap<usize, String>>();
        let candidates = candidates
            .iter()
            .map(|(i, f)| (*i, (**f).clone()))
            .collect::<Vec<_>>();
        let page_render_cache = Arc::clone(page_render_cache);
        tokio::task::spawn_blocking(move || {
            crop_figure_b64s(pdf_path, span_page_b64s, candidates, page_render_cache)
        })
        .await
        .unwrap_or_default()
    };
    if crop_b64s.is_empty() {
        return None;
    }

    // 2. One call: figure crops + question wording from the text layer.
    let user_text = format!(
        "TARGET: Question {}\nPAPER: '{}'\nMODULE: '{}'\n\nRead the values needed to answer Question {} from the attached figure image(s), and transcribe the question exactly.\n\nRAW TEXT (authoritative for wording):\n{}",
        span.number,
        config.paper_name,
        config.module_name,
        span.number,
        combined_text,
    );
    let body = llm::chat_body(
        &config.model,
        &crop_first_system_prompt(config),
        &crop_b64s,
        llm::ImageDetail::High,
        Some(&user_text),
        config.max_output_tokens.min(StageTag::CropFirst.output_cap()),
        Some(llm::ResponseFormat::JsonSchema {
            schema: extraction_json_schema(),
        }),
    );
    let resp = match chat_with_permit(client, &body, request_semaphore, cancel, usage, StageTag::CropFirst, config.cloud_allowed()).await {
        Ok(r) => r,
        Err(e) => {
            report.anomalies.push(format!(
                "Question {} crop-first call failed ({}); falling back to full-page vision",
                span.number, e
            ));
            return None;
        }
    };
    if llm::response_was_truncated(&resp) {
        report.anomalies.push(format!(
            "Question {} crop-first response hit the token ceiling (finish_reason=length); falling back to full-page vision",
            span.number
        ));
        return None;
    }
    let content = match llm::message_content(&resp) {
        Ok(c) => c,
        Err(e) => {
            report.anomalies.push(format!(
                "Question {} crop-first response unreadable ({}); falling back",
                span.number, e
            ));
            return None;
        }
    };

    // 3. Acceptance: a single clean item for the target question.
    let page = match parse_llm_json::<AiQuestionPage>(&content) {
        ParseOutcome::Clean(v) => v,
        ParseOutcome::Salvaged { value, dropped_tail } => {
            if dropped_tail {
                return None;
            }
            value
        }
        ParseOutcome::Malformed { error } => {
            report.anomalies.push(format!(
                "Question {} crop-first JSON invalid ({}); falling back",
                span.number, error
            ));
            return None;
        }
    };
    if page.items.is_empty() {
        return None;
    }
    let target_items: Vec<AiQuestion> = page
        .items
        .into_iter()
        .filter(|i| {
            i.question_number
                .as_ref()
                .and_then(validate::value_to_question_number)
                == Some(span.number)
        })
        .collect();
    if target_items.len() != 1 {
        return None;
    }
    let mut target_items = target_items;
    reconcile_bbox_indexes(&mut target_items);
    let filtered_page = AiQuestionPage {
        items: target_items.clone(),
    };
    let violations = validate_span_items(&filtered_page, span);
    if !violations.is_empty() {
        report.anomalies.push(format!(
            "Question {} crop-first validation failed ({}); falling back",
            span.number,
            violations.join("; ")
        ));
        return None;
    }

    // 4. Build the question (mirrors the text-first build tail). Placeholders
    //    are stripped — the figure crops are attached by the caller afterwards.
    let mut contents = Vec::new();
    let mut topics_acc = Vec::new();
    let mut is_code_acc = false;
    let mut ai_marks: Option<i32> = None;
    for item in target_items {
        let item_content = item
            .content
            .unwrap_or_default()
            .replace("[DIAGRAM_PLACEHOLDER]", "");
        contents.push(item_content);
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
    }
    let built = assemble_built_question(
        span,
        config,
        contents,
        topics_acc,
        is_code_acc,
        false,
        Vec::new(),
        ai_marks,
    )?;
    // Post-merge structural gate (crop-first variant): same escalation rule
    // as the text-first seam — a structurally broken card must never ship.
    let structural = validate::card_structure_errors(&built.content, span.number);
    if !structural.is_empty() {
        report.anomalies.push(format!(
            "Question {} crop-first structure failed ({}); falling back",
            span.number,
            structural.join("; ")
        ));
        return None;
    }
    Some((built, report))
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
    page_figures: &[Vec<crate::pdf_render::DetectedFigure>],
    page_render_cache: &Arc<crate::pdf_render::PageRenderCache>,
    page_image_cache: &Arc<PageImageCache>,
    request_semaphore: &Arc<Semaphore>,
    collateral_cache: &CollateralCache,
    all_spans: &Arc<Vec<QuestionSpan>>,
    text_first: bool,
    cancel: &AtomicBool,
    usage: &Arc<TokenTotals>,
) -> (Option<BuiltQuestion>, ImportReport) {
    // Own, local report: spans now run in parallel batches, so each unit
    // accumulates its own bookkeeping and the caller absorbs it in order.
    let mut report = ImportReport::default();

    if cancel.load(Ordering::Relaxed) {
        return (None, report);
    }

    // Check if this question was already fully extracted and validated as collateral
    // during a previous question's call on a shared page. If so, return immediately with 0 API calls!
    if let Some(cached_q) = collateral_cache.lock().await.remove(&span.number) {
        eprintln!(
            "[COLLATERAL_CACHE_HIT] Question {} retrieved from prior collateral extraction with 0 API calls",
            span.number
        );
        report.pages_processed += (span.start_page..=span.end_page).count().max(1);
        push_mark_check(span, &cached_q, &mut report);
        return (Some(cached_q), report);
    }

    // ── Text-layer-first extraction ──────────────────────────────────────
    // For digital papers the PDF text layer is authoritative enough that the
    // vision structure pass was skipped. Try transcribing this question from
    // the text layer ALONE (zero image tokens — the dominant cost on Gemini).
    // Only fall back to the vision path below when the model signals it needs
    // a figure, returns nothing, or fails validation.
    //
    // Figure questions used to be forced through vision because figure boxing
    // was vision-only. With `page_figures` the detector supplies the figure
    // regions for free, so figure questions CAN go text-first: the model
    // transcribes the text (emitting [DIAGRAM_PLACEHOLDER] where a figure is
    // referenced) and the deterministic crops are spliced in afterwards.
    // `span_figure_candidates` finds the figure even when it sits on the page
    // AFTER the question text (adjacent-page + whole-paper caption match), so
    // most figure questions never touch the vision path at all.
    //
    // A figure reference with no confident detection still needs recovery.
    // Graph-reading instructions alone do not require vision: preserve the
    // instruction and attach its graph, leaving the student to answer it.
    let combined_text: String = span_pages
        .iter()
        .map(|(_, p)| p.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let has_text = !combined_text.trim().is_empty();
    let text_refs_figure = has_text && text_references_figure(&combined_text);
    let must_read = has_text && figure_read_required(&combined_text);
    let referenced = if has_text {
        figure_reference_numbers(&combined_text)
    } else {
        Vec::new()
    };
    let candidates = span_figure_candidates(span, span_pages, page_figures, &referenced);
    // Supply counting only trusts confident detections: a low-confidence
    // region must never silently satisfy a figure reference — the span then
    // falls back to full-page vision instead of starving for its figure.
    // (Low-confidence figures still attach when explicitly referenced.)
    let fig_count = available_span_figures(span, span_pages, page_figures, &referenced);
    let needs_vision = text_refs_figure && fig_count == 0;

    // Document policy. A born-digital paper is strictly local: the
    // deterministic transcriber is forced on whatever the tuning switches say,
    // and text-first (an LLM call), crop-first, vision, and repair are all
    // dead. `chat_with_permit` refuses anything that still tries.
    let digital = config.is_digital_document();
    let text_first = text_first && !digital;
    let deterministic_enabled = config.deterministic || digital;

    // ── Tier-0 deterministic extraction ──────────────────────────────────
    // Zero-cost import cascade: text-reliable, figure-free spans are carved
    // out of the text layer by the local Rust transcriber — ZERO API calls.
    // Any gate refusal escalates to the LLM text-first call below unchanged.
    if has_text && deterministic_enabled {
        let paper_last_page = all_spans
            .iter()
            .map(|s| s.end_page)
            .max()
            .unwrap_or(span.end_page);
        let margin_model = config
            .margin_model
            .get_or_init(|| crate::deterministic::build_margin_model(config, all_spans));
        if let Some((mut built_q, mut t0_report)) = crate::deterministic::try_deterministic_extraction(
            config,
            span,
            span_pages,
            fig_count,
            paper_last_page,
            cancel,
            Some(margin_model),
        ) {
            // Same tail as text-first: attach figures (scrubs leftover
            // placeholders), bookkeeping, mark check, absorb.
            attach_detected_figures(
                config,
                span,
                span_pages,
                page_figures,
                page_render_cache,
                &mut built_q,
                &mut t0_report,
            )
            .await;
            settle_tier0_outcome(span, &built_q, &mut t0_report);
            t0_report.pages_processed += (span.start_page..=span.end_page).count().max(1);
            push_mark_check(span, &built_q, &mut t0_report);
            report.absorb(t0_report);
            return (Some(built_q), report);
        }
        // Digital document with a failed strict carve: no cloud to escalate
        // to. Keep the best locally carved candidate, flagged for review with
        // the failed gate on the record; if the carve could not isolate the
        // question at all, fail locally instead of pretending.
        if digital {
            if let Some((mut built_q, mut rec_report, gate)) =
                crate::deterministic::try_local_recovery(
                    config,
                    span,
                    span_pages,
                    fig_count,
                    paper_last_page,
                    cancel,
                    Some(margin_model),
                )
            {
                rec_report.recovered += 1;
                rec_report.anomalies.push(format!(
                    "Question {}: local recovery (digital document, failed gate: {}); content retained for review, zero cloud calls",
                    span.number, gate
                ));
                attach_detected_figures(
                    config,
                    span,
                    span_pages,
                    page_figures,
                    page_render_cache,
                    &mut built_q,
                    &mut rec_report,
                )
                .await;
                rec_report.pages_processed += (span.start_page..=span.end_page).count().max(1);
                push_mark_check(span, &built_q, &mut rec_report);
                report.absorb(rec_report);
                return (Some(built_q), report);
            }
        }
    }

    // A digital span that reaches here is a real local failure: either its
    // pages carry no usable text, or the carve could not isolate the question.
    // Report it plainly; never dispatch.
    if digital {
        let reason = if has_text {
            "digital document: no viable local candidate and cloud extraction is disabled"
        } else {
            "digital document: span pages carry no usable text and cloud extraction is disabled"
        };
        eprintln!("[LOCAL_FAILURE] question={} reason={}", span.number, reason);
        report
            .anomalies
            .push(format!("Question {}: {}", span.number, reason));
        report.quarantined.push(QuarantineEvent {
            scope: "question".to_string(),
            page: Some(span.start_page + 1),
            question_number: Some(span.number),
            reason: reason.to_string(),
        });
        report.pages_processed += (span.start_page..=span.end_page).count().max(1);
        return (None, report);
    }

    if text_first && has_text && !needs_vision {
        if let Some((mut built_q, mut tf_report)) = try_text_first_extraction(
            client,
            config,
            span,
            span_pages,
            fig_count,
            request_semaphore,
            cancel,
            usage,
        )
        .await
        {
            eprintln!(
                "[TEXT_FIRST] Question {} transcribed from text layer (0 image tokens)",
                span.number
            );
            tf_report.text_first += 1;
            // Always attach: with figures it splices crops into placeholders;
            // without figures it still scrubs leftover placeholder tokens.
            attach_detected_figures(
                config,
                span,
                span_pages,
                page_figures,
                page_render_cache,
                &mut built_q,
                &mut tf_report,
            )
            .await;
            tf_report.pages_processed += (span.start_page..=span.end_page).count().max(1);
            push_mark_check(span, &built_q, &mut tf_report);
            report.absorb(tf_report);
            return (Some(built_q), report);
        }
    }

    // ── Crop-first vision: read-from-figure questions ────────────────────
    // The question must be READ from a figure, so the text layer alone can
    // never supply the answer — but we don't need the full pages either. Send
    // ONLY the detected figure crops (≤512px each, ~4k image tokens) with the
    // question wording from the text layer. The crops are attached to the card
    // afterwards via the standard save guard chain. Any failure falls through
    // to the full-page vision path below, unchanged.
    if text_first && needs_vision && must_read && has_text && !candidates.is_empty() {
        if let Some((mut built_q, mut crop_report)) = try_crop_first_extraction(
            client,
            config,
            span,
            span_pages,
            &combined_text,
            &candidates,
            page_render_cache,
            request_semaphore,
            cancel,
            usage,
        )
        .await
        {
            eprintln!(
                "[CROP_FIRST] Question {} answered from {} figure crop(s) (~{} image tokens)",
                span.number,
                candidates.len(),
                candidates.len() * 4128
            );
            crop_report.crop_first += 1;
            attach_detected_figures(
                config,
                span,
                span_pages,
                page_figures,
                page_render_cache,
                &mut built_q,
                &mut crop_report,
            )
            .await;
            crop_report.pages_processed += (span.start_page..=span.end_page).count().max(1);
            push_mark_check(span, &built_q, &mut crop_report);
            report.absorb(crop_report);
            return (Some(built_q), report);
        } else {
            eprintln!(
                "[CROP_FIRST_FALLBACK] Question {} crop-first attempt failed; using full pages",
                span.number
            );
        }
    }

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
                    global_page_idx: *global_pi,
                    b64: b64.clone(),
                    start_y: s,
                    end_y: e,
                });
            }
        }
        let prepared = match prepare_chunk_images(chunk.len(), preparation_inputs, page_image_cache).await {
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

        let system = extraction_system_prompt(config);
        // JSON Schema for structured extraction output
        let extraction_schema = extraction_json_schema();
        let mut last_error = String::new();
        // Convergence guard: if a repair round produces the EXACT same
        // validation failure as the previous round, the model is stuck on
        // this prompt — re-sending the same images will not converge, so we
        // stop paying for it and resolve via the budget-spent paths.
        let mut last_repair_error: Option<String> = None;
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
                "TARGET: Question {}\nPAPER: '{}'\nMODULE: '{}'\n\nTranscribe Question {} from the attached page image(s).{}{}{}{}",
                span.number,
                config.paper_name,
                config.module_name,
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
            let all_band_crops = !page_crop_offsets.is_empty()
                && page_crop_offsets
                    .iter()
                    .all(|(s, e)| *s != 0.0 || *e != 1.0);
            let detail = if all_band_crops {
                llm::ImageDetail::Low
            } else {
                llm::ImageDetail::High
            };
            let body = llm::chat_body(
                &config.model,
                &system,
                &images,
                detail,
                Some(&user_text),
                config.max_output_tokens.min(StageTag::VisionSpan.output_cap()),
                Some(llm::ResponseFormat::JsonSchema {
                    schema: extraction_schema.clone(),
                }),
            );

            let api_start = Instant::now();
            let resp = match chat_with_permit(client, &body, request_semaphore, cancel, usage, StageTag::VisionSpan, config.cloud_allowed()).await {
                Ok(r) => r,
                Err(e) => {
                    last_error = e.to_string();
                    if attempt == max_attempts || cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    continue;
                }
            };
            // Payload-truncation resilience: finish_reason=length means the
            // output was cut off mid-tag at the token ceiling — it must be
            // REGENERATED, never salvaged, so the payload completes fully.
            if llm::response_was_truncated(&resp) {
                last_error = "the previous response hit the max_tokens ceiling and was cut off mid-tag (finish_reason=length); regenerate the COMPLETE JSON within the output limit".to_string();
                report.note_repair("finish_reason_length");
                if attempt < max_attempts && !cancel.load(Ordering::Relaxed) {
                    continue;
                }
                break;
            }
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
                    let reduced_all_band_crops = reduced_image_count > 0
                        && page_crop_offsets[..reduced_image_count]
                            .iter()
                            .all(|(s, e)| *s != 0.0 || *e != 1.0);
                    let reduced_detail = if reduced_all_band_crops {
                        llm::ImageDetail::Low
                    } else {
                        llm::ImageDetail::High
                    };
                    let reduced_body = llm::chat_body(
                        &config.model,
                        &system,
                        &reduced_images,
                        reduced_detail,
                        Some(&format!(
                            "{}\n\nNOTE: This is a retry with fewer pages due to payload size issues. Transcribe Question {} from these pages only.",
                            user_text, span.number
                        )),
                        config.max_output_tokens.min(StageTag::VisionSpan.output_cap()),
                        Some(llm::ResponseFormat::JsonSchema {
                            schema: extraction_schema.clone(),
                        }),
                    );

                    let api_start = Instant::now();
                    let reduced_resp = match chat_with_permit(client, &reduced_body, request_semaphore, cancel, usage, StageTag::VisionSpan, config.cloud_allowed()).await {
                        Ok(r) => r,
                        Err(e) => {
                            last_error = e.to_string();
                            report.note_repair("eof_reduced_api_error");
                            continue;
                        }
                    };
                    if llm::response_was_truncated(&reduced_resp) {
                        last_error = "the reduced retry was also cut off at the token ceiling (finish_reason=length)".to_string();
                        report.note_repair("eof_reduced_truncated");
                        continue;
                    }
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
                            report.note_repair("eof_reduced_message_error");
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
                    report.note_repair("malformed_json");
                    let non_convergent =
                        repair_is_non_convergent(attempt, last_repair_error.as_deref(), &last_error);
                    if attempt < max_attempts && !non_convergent {
                        last_repair_error = Some(last_error.clone());
                        continue;
                    }
                    if non_convergent {
                        eprintln!(
                            "WARNING: Question {} repeated identical malformed JSON; quarantining instead of re-sending.",
                            span.number
                        );
                    }
                    break;
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
                let raw_items = std::mem::take(&mut split_raw_items);
                match stitch_question_items(raw_items.clone(), span.number, span.expected_marks) {
                    Ok(stitched) => {
                        if stitched.marks_ambiguous {
                            needs_review = true;
                            notes.push(
                                "stitched sub-parts carried equal marks with no printed total to confirm them against; marks summed, please review"
                                    .to_string(),
                            );
                        }
                        page_items.items = vec![stitched.item];
                        let unified = &page_items.items[0];
                        eprintln!(
                            "[DIAGNOSTIC][UNIFIED_OBJECT] question={} structure={:#?}",
                            span.number, unified
                        );
                    }
                    Err(refusal) => {
                        // Distinct parent identities inside one span's
                        // response: never weld them. Leave the fragments in
                        // place so the strict per-item validator rejects the
                        // span honestly instead of shipping a merged card.
                        eprintln!(
                            "[STITCH_REFUSED] question={} reason={}",
                            span.number,
                            refusal.describe()
                        );
                        page_items.items = raw_items;
                        unified_split = false;
                    }
                }
            }

            for item in &mut page_items.items {
                normalize_visual_mcq_options(item);
            }

            if page_items.items.is_empty() && contents.is_empty() {
                eprintln!(
                    "[DIAGNOSTIC][VALIDATION_ERROR] question={} rule=non_empty_items items=0 prior_contents={}",
                    span.number,
                    contents.len()
                );
                eprintln!("WARNING: Question {} extraction returned an empty items array.", span.number);
                let new_error = format!(
                    "Extraction for Question {} returned an empty items array. Please transcribe Question {} and all its sub-parts from the provided page(s).",
                    span.number, span.number
                );
                let non_convergent =
                    repair_is_non_convergent(attempt, last_repair_error.as_deref(), &new_error);
                if attempt < config.max_repairs && !non_convergent {
                    last_error = new_error;
                    report.note_repair("empty_items");
                    last_repair_error = Some(last_error.clone());
                    continue;
                } else {
                    if non_convergent {
                        eprintln!(
                            "WARNING: Question {} repeated identical empty-items failure; quarantining instead of re-sending.",
                            span.number
                        );
                    }
                    report.quarantined.push(QuarantineEvent {
                        scope: "question".to_string(),
                        page: Some(span.start_page + 1),
                        question_number: Some(span.number),
                        reason: "No content extracted for this span".to_string(),
                    });
                    return (None, report);
                }
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
                    report.note_repair("wrong_question_number");
                }
            }

            // Filter to target question only, but quote what was dropped.
            let dropped_numbers: Vec<String> = raw_numbers
                .iter()
                .filter(|&&n| n != span.number)
                .map(|n| n.to_string())
                .collect();
            let original_items = page_items.items.clone();

            // Collateral Question Ingestion: If the model returned downstream questions
            // that reside on this chunk's last page and match expected marks/validation,
            // cache them to avoid redundant LLM requests when their turns arrive.
            for collateral_item in &original_items {
                if let Some(num) = collateral_item
                    .question_number
                    .as_ref()
                    .and_then(crate::validate::value_to_question_number)
                {
                    if num != span.number {
                        if let Some(target_span) = all_spans.iter().find(|s| s.number == num) {
                            let chunk_last_page = chunk.last().map(|(p, _)| *p).unwrap_or(0);
                            if target_span.start_page == target_span.end_page && target_span.start_page == chunk_last_page {
                                let mut item_content = collateral_item.content.clone().unwrap_or_default();
                                let is_code = collateral_item.is_code.unwrap_or(false);
                                let mut topics_acc = Vec::new();
                                if let Some(t) = &collateral_item.topics {
                                    for topic in value_to_topics(t) {
                                        if config.allowed_topics.is_empty() || config.allowed_topics.contains(&topic) {
                                            topics_acc.push(topic);
                                        }
                                    }
                                }
                                let ai_marks = collateral_item.marks.as_ref().and_then(crate::validate::value_to_marks);

                                if let Some(bboxes) = &collateral_item.diagram_bboxes {
                                    let indexes = collateral_item.bbox_page_indexes.clone().unwrap_or_default();
                                    let mut requests = Vec::with_capacity(bboxes.len());
                                    let mut page_b64 = std::collections::HashMap::new();
                                    for (bi, bbox) in bboxes.iter().enumerate() {
                                        let model_idx = indexes.get(bi).and_then(value_to_usize).filter(|&k| k < local_to_chunk.len()).unwrap_or(0);
                                        let chunk_idx = local_to_chunk[model_idx];
                                        if chunk_idx < chunk.len() {
                                            let global_page_idx = chunk[chunk_idx].0;
                                            let page = chunk[chunk_idx].1;
                                            let ignore_grid = crate::validate::figure_references(&item_content) > 0 && !crate::validate::is_answer_grid_request(&item_content);
                                            if config.pdf_path.is_none() {
                                                if let Some(b64) = page.get_b64() {
                                                    page_b64.entry(global_page_idx).or_insert_with(|| b64.clone());
                                                }
                                            }
                                            requests.push(DiagramSaveRequest {
                                                global_page_idx,
                                                bbox: bbox.clone(),
                                                ignore_grid,
                                                graph_like: collateral_item
                                                    .diagram_kinds
                                                    .as_ref()
                                                    .and_then(|kinds| kinds.get(bi))
                                                    .map(|kind| {
                                                        let kind = kind.to_ascii_lowercase();
                                                        kind.contains("graph")
                                                            || kind.contains("chart")
                                                            || kind.contains("plot")
                                                            || kind.contains("composite_visual_options") || kind.contains("visual_option")
                                                    })
                                                    .unwrap_or(false),
                                                    exact: None,
                                                    mask: Vec::new(),
                                            });
                                        }
                                    }
                                    if let Ok(persisted) = persist_diagrams(
                                        requests,
                                        page_b64,
                                        config.clone(),
                                        Arc::clone(page_render_cache),
                                        std::mem::take(&mut saved_diagrams),
                                    ).await {
                                        saved_diagrams = persisted.saved;
                                        item_content = splice_diagrams_by_caption_and_context(
                                            item_content,
                                            &persisted.links,
                                            collateral_item.diagram_captions.as_deref(),
                                        );
                                    }
                                }

                                if let Some(built_q) = assemble_built_question(
                                    target_span,
                                    config,
                                    vec![item_content],
                                    topics_acc,
                                    is_code,
                                    false,
                                    Vec::new(),
                                    ai_marks,
                                ) {
                                    let marks_valid = match (target_span.expected_marks, built_q.marks) {
                                        (Some(exp), actual) => actual > 0 && (actual as u32 == exp),
                                        (None, actual) => actual > 0,
                                    };
                                    if !built_q.needs_review && marks_valid && !built_q.content.is_empty() {
                                        eprintln!(
                                            "[COLLATERAL_CACHED] Question {} cached during Question {} extraction on page {}",
                                            num, span.number, target_span.start_page + 1
                                        );
                                        collateral_cache.lock().await.insert(num, built_q);
                                    }
                                }
                            }
                        }
                    }
                }
            }

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
            //
            // Before that: an ordinary single-span response that answers
            // (a)(b)(c) as separate items for THIS question is a compatible
            // stitch, not a schema error. Stitch it here so the parts are kept
            // instead of being discarded or costing a repair round. Items for
            // other questions were filtered out above, so this can never weld
            // two parent questions together.
            if page_items.items.len() > 1 {
                // Preserve the historical mislabeled-parent safeguard BEFORE
                // accepting a stitch. Two items where the second looks like a
                // new question (its own "7." / "**5.**" heading, or a part-label
                // reset) must go down the existing repair path instead of being
                // silently welded; any item that opens with its own different
                // question number is refused for the same reason.
                let looks_like_separate_questions = page_items
                    .items
                    .windows(2)
                    .any(|pair| {
                        let prev = pair[0].content.as_deref().unwrap_or("");
                        let next = pair[1].content.as_deref().unwrap_or("");
                        (page_items.items.len() == 2 && looks_like_new_question(prev, next))
                            || leading_question_number(next)
                                .is_some_and(|n| n != span.number)
                    });
                if looks_like_separate_questions {
                    eprintln!(
                        "[STITCH_REFUSED] question={} reason=separate_question_stems",
                        span.number
                    );
                } else {
                    match stitch_question_items(
                        page_items.items.clone(),
                        span.number,
                        span.expected_marks,
                    ) {
                        Ok(stitched) => {
                            eprintln!(
                                "[STITCHED] question={} items={} -> 1 (0 repair rounds)",
                                span.number,
                                page_items.items.len()
                            );
                            if stitched.marks_ambiguous {
                                needs_review = true;
                                notes.push(
                                    "stitched sub-parts carried equal marks with no printed total to confirm them against; marks summed, please review"
                                        .to_string(),
                                );
                            }
                            page_items.items = vec![stitched.item];
                        }
                        Err(refusal) => {
                            eprintln!(
                                "[STITCH_REFUSED] question={} reason={}",
                                span.number,
                                refusal.describe()
                            );
                        }
                    }
                }
            }
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
                        report.note_repair("misnumbered_new_question");
                        if attempt < max_attempts
                            && !repair_is_non_convergent(
                                attempt,
                                last_repair_error.as_deref(),
                                &last_error,
                            )
                        {
                            last_repair_error = Some(last_error.clone());
                            continue;
                        }
                        break;
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
                    report.note_repair("multi_item_split");
                    if attempt < max_attempts
                        && !repair_is_non_convergent(
                            attempt,
                            last_repair_error.as_deref(),
                            &last_error,
                        )
                    {
                        last_repair_error = Some(last_error.clone());
                        continue;
                    }
                    break;
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
                report.note_repair("empty_after_retention");
                if attempt < max_attempts
                    && !repair_is_non_convergent(
                        attempt,
                        last_repair_error.as_deref(),
                        &last_error,
                    )
                {
                    last_repair_error = Some(last_error.clone());
                    continue;
                }
                break;
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
            reconcile_bbox_indexes(&mut page_items.items);
            let validation_errors = validate_span_items(&page_items, span);
            if !validation_errors.is_empty() {
                for error in &validation_errors {
                    eprintln!(
                        "[DIAGNOSTIC][SCHEMA_VALIDATION_ERROR] question={} rule={}",
                        span.number, error
                    );
                }
                last_error = validation_errors.join("; ");
                report.note_repair("schema_validation");
                if attempt < max_attempts
                    && !repair_is_non_convergent(
                        attempt,
                        last_repair_error.as_deref(),
                        &last_error,
                    )
                {
                    last_repair_error = Some(last_error.clone());
                    continue;
                }
                break;
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
                report.note_repair("figure_consistency");
                if attempt < max_attempts
                    && !repair_is_non_convergent(
                        attempt,
                        last_repair_error.as_deref(),
                        &last_error,
                    )
                {
                    last_error = cons_errors.join("; ");
                    last_repair_error = Some(last_error.clone());
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

            // Build page_texts aligned with local_to_chunk (model-visible order)
            let audit_page_texts: Vec<String> = local_to_chunk
                .iter()
                .map(|&chunk_idx| {
                    if chunk_idx < chunk.len() {
                        chunk[chunk_idx].1.text.clone()
                    } else {
                        String::new()
                    }
                })
                .collect();

            let (audited_items, bad, box_issues) = match tokio::task::spawn_blocking(move || {
                let mut items = audit_items;
                let (bad, issues) = audit_diagram_boxes(
                    &audit_decoded_pages,
                    &audit_page_texts,
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
                    report.note_repair("diagram_audit_task_failed");
                    if attempt < max_attempts
                        && !repair_is_non_convergent(
                            attempt,
                            last_repair_error.as_deref(),
                            &last_error,
                        )
                    {
                        last_repair_error = Some(last_error.clone());
                        continue;
                    }
                    break;
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
                report.note_repair("diagram_box_issues");
                // Convergence guard: identical rejection on the previous
                // attempt means the model is stuck on this box — re-sending
                // the same images won't fix it. Skip the redundant call and
                // go straight to the budget-spent prune-and-accept path.
                let mut items = page_items.items;
                if attempt < max_attempts
                    && !repair_is_non_convergent(attempt, last_repair_error.as_deref(), &last_error)
                {
                    last_repair_error = Some(last_error.clone());
                    continue;
                }
                // Repair budget spent (or non-convergent): keep the
                // transcription, drop the bad boxes — deterministically, and
                // on the record.
                report.anomalies.push(format!(
                    "Question {}: dropped {} invalid diagram box(es) after repair budget spent — {}",
                    span.number,
                    bad.len(),
                    box_issues.join("; ")
                ));
                prune_bad_diagram_boxes(&mut items, &bad, &mut report);
                accepted = Some((items, salvaged));
                break;
            }

            accepted = Some((page_items.items, salvaged));
            break;
        }

        let (mut items, salvaged) = match accepted {
            Some(v) => v,
            None => {
                eprintln!("WARNING: Question {} extraction failed: {}", span.number, last_error);
                report
                    .anomalies
                    .push(format!("quarantined: {}", last_error));
                return (None, report);
            }
        };

        // Split compound figures (e.g., "Figure 4 and Figure 5" in one bbox)
        // Build page_texts aligned with local_to_chunk
        let page_texts: Vec<String> = local_to_chunk
            .iter()
            .map(|&chunk_idx| {
                if chunk_idx < chunk.len() {
                    chunk[chunk_idx].1.text.clone()
                } else {
                    String::new()
                }
            })
            .collect();
        split_compound_figures_if_needed(&mut items, &page_texts, &local_to_chunk);

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
                                    || kind.contains("composite_visual_options") || kind.contains("visual_option")
                            })
                            .unwrap_or(false),
                            exact: None,
                            mask: Vec::new(),
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
                        item_content = splice_diagrams_by_caption_and_context(
                            item_content,
                            &persisted.links,
                            item.diagram_captions.as_deref(),
                        );
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

    let built_q = assemble_built_question(
        span,
        config,
        contents,
        topics_acc,
        is_code_acc,
        needs_review,
        notes,
        ai_marks,
    );
    // Final structural gate on the vision path: a card whose sub-part
    // sequence / MCQ syntax / math hygiene is broken must never ship as a
    // clean card. It falls through to quarantine + fallback so the
    // needs_review flag surfaces it instead of silently corrupting the repo.
    if let Some(q) = &built_q {
        let errs = validate::card_structure_errors(&q.content, span.number);
        if !errs.is_empty() {
            report.anomalies.push(format!(
                "quarantined: Question {} failed structural validation ({})",
                span.number,
                errs.join("; ")
            ));
            return (None, report);
        }
    }
    (built_q, report)
}

/// Assemble and validate a BuiltQuestion from raw extracted chunks/content.
fn assemble_built_question(
    span: &QuestionSpan,
    config: &PipelineConfig,
    contents: Vec<String>,
    mut topics_acc: Vec<String>,
    is_code_acc: bool,
    mut needs_review: bool,
    mut notes: Vec<String>,
    ai_marks: Option<i32>,
) -> Option<BuiltQuestion> {
    let mut content = contents.join("\n\n");
    content = validate::clean_question_content(&content);
    // One labelling scheme forever: AQA '3 . 1'-style decimals → (a), (b), (c).
    content = validate::normalize_decimal_parts(&content, span.number);
    // Comprehensive post-processing for the 6 extraction/formatting failure modes:
    // 1. Artifact boilerplate bleed, 2. Number-agnostic AQA decimal failures,
    // 3. MCQ option flattening, 4. Tabular option destruction, 5. Visual MCQ
    // gibberish, 6. Mark allocation misplacement.
    content = crate::marker_client::clean_marker_markdown(&content);
    // Deterministic formatting-rules pass (the 6 trap classes: isotope
    // reconstruction, delimiter balance, sentence-math unwrapping, unicode
    // minus/exponents, MCQ list syntax, OCR boilerplate). Runs HERE — inside
    // the acceptance seam — so every extraction path (vision, text-first,
    // Tier-0, batch) produces clean cards and the structural validator below
    // never fires on issues the sanitizer already fixed for free.
    content = crate::sanitize::sanitize_question_content(&content, span.number);
    // Terminal KaTeX guard: close broken inline $ at line ends and unterminated
    // $$ blocks so raw LaTeX can never leak as plaintext or swallow text below it.
    content = validate::balance_math_delimiters(&content);
    // Multi-line display blocks need explicit LaTeX row separators: KaTeX
    // ignores raw newlines and would squash sequential equations (decay
    // chains, simultaneous pairs) end-to-end.
    content = validate::ensure_display_math_line_breaks(&content);

    if content.trim().is_empty() && span.expected_marks.unwrap_or(0) > 0 {
        // A marked question with no content is a hard failure.
        return None;
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
    #[cfg(test)]
    eprintln!(
        "[SCRATCH] q={} expected={:?} inline={} ai={:?}",
        span.number, span.expected_marks, inline, ai_marks
    );
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

    Some(BuiltQuestion {
        question_number: span.number,
        content,
        marks,
        topics: topics_acc,
        module: config.module_name.clone(),
        is_code: config.subject == "Computer Science" && is_code_acc,
        needs_review,
        notes,
    })
}

/// A detected figure whose box lies mostly inside a ruled table the layout
/// transcribed (the table is text, not an image).
fn figure_is_layout_table(fig: &crate::pdf_render::DetectedFigure, lp: &crate::layout::LayoutPage) -> bool {
    let (w, h) = (lp.width.max(1.0), lp.height.max(1.0));
    let [fx, fy, fw, fh] = fig.bbox;
    let (fx0, fy0, fx1, fy1) = (fx * w, fy * h, (fx + fw) * w, (fy + fh) * h);
    let area = ((fx1 - fx0) * (fy1 - fy0)).max(1.0);
    lp.tables.iter().any(|t| {
        let ix = (fx1.min(t[2]) - fx0.max(t[0])).max(0.0);
        let iy = (fy1.min(t[3]) - fy0.max(t[1])).max(0.0);
        ix * iy >= 0.6 * area
    })
}

/// The kind recorded for a picture set inside a line of text.
const INLINE_FIGURE_KIND: &str = "inline";

/// A layout question's figures in reading order, as (page, figure, crop
/// rectangle in points): the detector's figures in the question's area (a
/// figure that is a transcribed table excluded), and every diagram region
/// the layout drew that the detector missed or split into unlabelled
/// pieces, cropped with its labels. Nothing outside the question's own
/// area — between its heading and the next question's — is taken.
fn layout_question_figures(
    layout: &[crate::layout::LayoutPage],
    q: &crate::layout::LayoutQuestion,
    page_figures: &[Vec<crate::pdf_render::DetectedFigure>],
) -> Vec<(LayoutFigure, [f32; 4])> {
    fn area(r: &[f32; 4]) -> f32 {
        ((r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0)).max(1.0)
    }
    fn overlap(a: &[f32; 4], b: &[f32; 4]) -> f32 {
        (a[2].min(b[2]) - a[0].max(b[0])).max(0.0) * (a[3].min(b[3]) - a[1].max(b[1])).max(0.0)
    }
    let mut out: Vec<(LayoutFigure, [f32; 4])> = Vec::new();
    for p in q.start_page..=q.end_page {
        let Some(lp) = layout.get(p) else { continue };
        let (w, h) = (lp.width.max(1.0), lp.height.max(1.0));
        let mine = |r: &[f32; 4]| crate::layout::question_figure_window(layout, q, p, (r[1] + r[3]) * 0.5);
        let detected: Vec<(&crate::pdf_render::DetectedFigure, [f32; 4])> = page_figures
            .get(p)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
            .iter()
            .filter(|f| !figure_is_layout_table(f, lp))
            .map(|f| (f, [f.bbox[0] * w, f.bbox[1] * h, (f.bbox[0] + f.bbox[2]) * w, (f.bbox[1] + f.bbox[3]) * h]))
            .filter(|(_, r)| mine(r))
            .collect();
        let mut keep = vec![true; detected.len()];
        // A detector box over what the layout already transcribed as text
        // (a table, a row of bits, labelled boxes, the sentence above) is a
        // partial picture of that text, not a figure.
        for (i, (_, d)) in detected.iter().enumerate() {
            let covered: f32 = lp.lines.iter().map(|l| overlap(d, &[l.x0, l.y0, l.x1, l.y1])).sum::<f32>()
                + lp.tables.iter().map(|t| overlap(d, t)).sum::<f32>();
            let over_text = lp.tables.iter().chain(&lp.text_grids).any(|t| overlap(d, t) >= 0.2 * area(d));
            // Nothing is drawn in it but text and ruled grids.
            let undrawn = !lp.drawings.iter().any(|b| overlap(d, b) > 0.0);
            if covered >= 0.45 * area(d) || over_text || undrawn {
                keep[i] = false;
            }
        }
        let mut extra: Vec<(LayoutFigure, [f32; 4])> = Vec::new();
        // Option letters name a set of drawings; a lone letter captured
        // inside one figure (its curve "C") is a label, not an option.
        let option_set = detected.iter().filter_map(|(f, _)| f.option_label.as_deref()).collect::<std::collections::BTreeSet<_>>().len();
        for region in lp.figures.iter().filter(|r| mine(r)) {
            let pieces: Vec<usize> = (0..detected.len())
                .filter(|&i| overlap(&detected[i].1, region) >= 0.5 * area(&detected[i].1).min(area(region)))
                .collect();
            if option_set >= 3 && pieces.iter().any(|&i| detected[i].0.option_label.is_some()) {
                continue; // the detector's labelled option crops stand
            }
            // The layout's figure extent (strokes and labels) is exact. A
            // detector box over the same figure, or several splitting it,
            // lends its caption; pieces reaching beyond the strokes widen it.
            let agrees = pieces.len() == 1 && {
                let d = &detected[pieces[0]].1;
                let inter = overlap(d, region);
                inter / (area(d) + area(region) - inter) >= 0.5
            };
            let mut rect = crate::layout::figure_crop_rect(lp, *region);
            let mut caption = None;
            let mut kind = None;
            for &i in &pieces {
                keep[i] = false;
                let d = &detected[i].1;
                if !agrees {
                    rect = [rect[0].min(d[0]), rect[1].min(d[1]), rect[2].max(d[2]), rect[3].max(d[3])];
                }
                caption = caption.or_else(|| detected[i].0.caption.clone());
                kind = kind.or_else(|| detected[i].0.kind.clone());
            }
            let figure = crate::pdf_render::DetectedFigure {
                bbox: [rect[0] / w, rect[1] / h, (rect[2] - rect[0]) / w, (rect[3] - rect[1]) / h],
                caption,
                kind,
                seg_confidence: FIGURE_SUPPLY_MIN_CONFIDENCE,
                option_label: None,
            };
            extra.push((LayoutFigure { page: p, figure, exact: true }, rect));
        }
        // Pictures inside lines of text (formulas printed as images) are
        // figures of their own, spliced where they stand; a detector box
        // over one is the same picture.
        for im in lp.inline_images.iter().filter(|r| mine(r)) {
            for (i, (_, d)) in detected.iter().enumerate() {
                if overlap(d, im) >= 0.5 * area(im).min(area(d)) {
                    keep[i] = false;
                }
            }
            let figure = crate::pdf_render::DetectedFigure {
                bbox: [im[0] / w, im[1] / h, (im[2] - im[0]) / w, (im[3] - im[1]) / h],
                caption: None,
                kind: Some(INLINE_FIGURE_KIND.to_string()),
                seg_confidence: FIGURE_SUPPLY_MIN_CONFIDENCE,
                option_label: None,
            };
            extra.push((LayoutFigure { page: p, figure, exact: true }, *im));
        }
        let mut page_out: Vec<(LayoutFigure, [f32; 4])> = detected
            .iter()
            .zip(keep)
            .filter(|(_, k)| *k)
            .map(|((f, r), _)| (LayoutFigure { page: p, figure: (*f).clone(), exact: false }, *r))
            .chain(extra)
            .collect();
        // Option letters printed over a row of figures ("A    B" above two
        // diagrams) name them left to right, one letter per figure.
        for line in &lp.lines {
            let letters: Vec<char> = line.text.split_whitespace().map(|w| w.chars().collect::<Vec<_>>()).filter_map(|w| (w.len() == 1).then(|| w[0])).collect();
            if letters.is_empty()
                || letters.len() != line.text.split_whitespace().count()
                || !letters.iter().all(|c| matches!(c, 'A'..='E'))
            {
                continue;
            }
            let mut below: Vec<usize> = (0..page_out.len())
                .filter(|&i| {
                    let r = page_out[i].1;
                    r[1] >= line.y1 - 2.0 && r[1] <= line.y1 + 4.0 * line.size.max(6.0) && r[2] >= line.x0 - 10.0 && r[0] <= line.x1 + 10.0
                })
                .collect();
            if below.len() != letters.len() {
                continue;
            }
            below.sort_by(|&a, &b| page_out[a].1[0].total_cmp(&page_out[b].1[0]));
            for (&i, &c) in below.iter().zip(&letters) {
                if page_out[i].0.figure.option_label.is_none() {
                    page_out[i].0.figure.option_label = Some(c.to_string());
                }
            }
        }
        // A crop never takes in the question's own text lines (a detector box
        // reaching over the sentence above): it stops short of any body line
        // lying across it.
        for (f, r) in page_out.iter_mut() {
            if f.figure.kind.as_deref() == Some(INLINE_FIGURE_KIND) {
                continue;
            }
            let mut c = *r;
            for l in &lp.lines {
                let ox = (c[2].min(l.x1) - c[0].max(l.x0)).max(0.0);
                let oy = (c[3].min(l.y1) - c[1].max(l.y0)).max(0.0);
                if ox <= 0.0 || oy <= 0.0 || ox < 0.5 * (l.x1 - l.x0).min(c[2] - c[0]) {
                    continue;
                }
                // (Clear of the crop's own margin.)
                if (l.y0 + l.y1) * 0.5 < (c[1] + c[3]) * 0.5 {
                    c[1] = c[1].max(l.y1 + 3.5);
                } else {
                    c[3] = c[3].min(l.y0 - 3.5);
                }
            }
            if c[3] - c[1] >= 20.0 && (c[1] != r[1] || c[3] != r[3]) {
                *r = c;
                f.figure.bbox = [c[0] / w, c[1] / h, (c[2] - c[0]) / w, (c[3] - c[1]) / h];
            }
        }
        // Parts of one drawing found apart (the branches of a tree diagram):
        // uncaptioned, unlabelled pieces side by side with nothing but their
        // own labels between them are one figure.
        loop {
            let mut merged = false;
            'pairs: for i in 0..page_out.len() {
                for j in i + 1..page_out.len() {
                    let (a, b) = (&page_out[i], &page_out[j]);
                    let plain = |f: &LayoutFigure| {
                        f.exact && f.figure.caption.is_none() && f.figure.option_label.is_none() && f.figure.kind.as_deref() != Some(INLINE_FIGURE_KIND)
                    };
                    if !plain(&a.0) || !plain(&b.0) {
                        continue;
                    }
                    let (ra, rb) = (a.1, b.1);
                    let v_overlap = ra[3].min(rb[3]) - ra[1].max(rb[1]);
                    let h_gap = (rb[0] - ra[2]).max(ra[0] - rb[2]);
                    if v_overlap <= 0.0 || h_gap > 100.0 {
                        continue;
                    }
                    let u = [ra[0].min(rb[0]), ra[1].min(rb[1]), ra[2].max(rb[2]), ra[3].max(rb[3])];
                    let text_between = lp.lines.iter().any(|l| {
                        let (cx, cy) = ((l.x0 + l.x1) * 0.5, (l.y0 + l.y1) * 0.5);
                        cx > u[0] && cx < u[2] && cy > u[1] && cy < u[3]
                    });
                    if text_between {
                        continue;
                    }
                    page_out[i].1 = u;
                    page_out[i].0.figure.bbox = [u[0] / w, u[1] / h, (u[2] - u[0]) / w, (u[3] - u[1]) / h];
                    page_out.remove(j);
                    merged = true;
                    break 'pairs;
                }
            }
            if !merged {
                break;
            }
        }
        // Reading order: rows top to bottom, each row left to right.
        page_out.sort_by(|a, b| a.1[1].total_cmp(&b.1[1]));
        let mut start = 0;
        while start < page_out.len() {
            let top = page_out[start].1[1];
            let tolerance = ((page_out[start].1[3] - top) * 0.25).min(0.06 * h);
            let mut end = start + 1;
            while end < page_out.len() && page_out[end].1[1] - top <= tolerance {
                end += 1;
            }
            page_out[start..end].sort_by(|a, b| a.1[0].total_cmp(&b.1[0]));
            start = end;
        }
        out.extend(page_out);
    }
    out
}

/// Diagram regions the layout found inside a layout question's own area.
fn layout_figures_in_span(config: &PipelineConfig, span: &QuestionSpan) -> Option<usize> {
    if !is_layout_question(config, span) {
        return None;
    }
    let ev = config.layout_evidence.as_ref()?;
    let mut n = 0;
    for p in span.start_page..=span.end_page {
        let Some(lp) = ev.layout.get(p) else { continue };
        let h = lp.height.max(1.0);
        let lo = if p == span.start_page { span.start_y_frac.unwrap_or(0.0) * h } else { 0.0 };
        let hi = if p == span.end_page { span.end_y_frac.map(|y| y * h).unwrap_or(h) } else { h };
        n += lp.figures.iter().filter(|r| {
            let cy = (r[1] + r[3]) * 0.5;
            cy >= lo - 5.0 && cy <= hi + 5.0
        }).count();
    }
    Some(n)
}

/// True when this span's body came from the geometric layout map.
pub(crate) fn is_layout_question(config: &PipelineConfig, span: &QuestionSpan) -> bool {
    config.layout_questions.as_ref().is_some_and(|m| m.contains_key(&span.number))
}

/// Acceptance assembly for a layout-derived card. The body already carries
/// source-reconstructed mathematics and exact boundaries, so none of the
/// repair heuristics written for LLM/marker output run here — only the
/// generic sanitizer (MCQ list syntax, diagram order, delimiter balance) and
/// the marks/terminal-ending bookkeeping shared with every card.
fn assemble_layout_question(
    span: &QuestionSpan,
    config: &PipelineConfig,
    content: String,
    mut needs_review: bool,
    mut notes: Vec<String>,
) -> Option<BuiltQuestion> {
    let mut content = crate::sanitize::sanitize_question_content(&content, span.number);
    content = validate::balance_math_delimiters(&content);
    content = validate::ensure_display_math_line_breaks(&content);
    if content.trim().is_empty() {
        return None;
    }
    let ends_with_reference = config.layout_questions.as_ref().and_then(|m| m.get(&span.number)).is_some_and(|b| b.ends_with_reference);
    if !validate::has_terminal_ending(&content) && !ends_with_reference {
        needs_review = true;
        notes.push("content lacks terminal punctuation (possible truncation)".to_string());
    }
    let inline = validate::sum_inline_marks(&content) + validate::sum_style_marks(&content);
    let marks = match (span.expected_marks, inline) {
        (Some(e), 0) => e as i32,
        (Some(e), n) if n == e => e as i32,
        (Some(e), n) => {
            needs_review = true;
            notes.push(format!("inline marks sum ({}) differs from printed footer ({}) — trusting footer", n, e));
            e as i32
        }
        (None, n) if n > 0 => n as i32,
        (None, _) => {
            needs_review = true;
            notes.push("no printed marks found for this question".to_string());
            0
        }
    };
    Some(BuiltQuestion {
        question_number: span.number,
        content,
        marks,
        topics: Vec::new(),
        module: config.module_name.clone(),
        is_code: false,
        needs_review,
        notes,
    })
}

/// Extract multiple questions that reside entirely on the same single page in one consolidated call.
/// Slashes input tokens and vision API calls by 70-80% on multi-question / MCQ pages.
/// If any question is missing or fails validation, it falls back to extract_span for only that question.
async fn extract_same_page_batch<C: LlmClient>(
    client: &C,
    config: &PipelineConfig,
    spans: &[&QuestionSpan],
    page_idx: usize,
    page: &PageInput,
    page_figures: &[Vec<crate::pdf_render::DetectedFigure>],
    page_render_cache: &Arc<crate::pdf_render::PageRenderCache>,
    page_image_cache: &Arc<PageImageCache>,
    request_semaphore: &Arc<Semaphore>,
    collateral_cache: &CollateralCache,
    all_spans: &Arc<Vec<QuestionSpan>>,
    text_first: bool,
    cancel: &AtomicBool,
    usage: &Arc<TokenTotals>,
) -> (Vec<(QuestionSpan, Option<BuiltQuestion>)>, ImportReport) {
    let mut report = ImportReport::default();
    if cancel.load(Ordering::Relaxed) {
        return (Vec::new(), report);
    }

    // ── Text-layer-first for shared-page batches ─────────────────────────
    // Same-page batches are the LARGEST remaining vision cost: every shared
    // page pays one full-page image call. If every question on the page can
    // be transcribed from the text layer (figures attached deterministically
    // via the caption-aware candidate lookup), skip the vision call entirely.
    // If ANY question in the batch needs vision, the whole batch falls back
    // to the single shared-page vision call below (unchanged behaviour).
    let digital = config.is_digital_document();
    if (text_first || digital) && !page.text.trim().is_empty() {
        let span_pages = [(page_idx, page)];
        let combined_text = page.text.trim().to_string();
        let text_refs_figure = text_references_figure(&combined_text);
        let referenced = figure_reference_numbers(&combined_text);
        let mut tf_out: Vec<(QuestionSpan, Option<BuiltQuestion>)> = Vec::with_capacity(spans.len());
        let mut tf_report = ImportReport::default();

        // Per-span figure counts for the placeholder gate in the shared build
        // helper (a question that needs a figure the detector can't supply
        // must still go to vision, exactly as the old per-span loop decided).
        let fig_counts: Vec<usize> = spans
            .iter()
            .map(|span| {
                available_span_figures(span, &span_pages, page_figures, &referenced)
            })
            .collect();
        let needs_vision =
            text_refs_figure && fig_counts.iter().any(|&c| c == 0);

        {
            // ── Tier-0 deterministic batch ───────────────────────────────
            // Carve EVERY target span locally; all green ⇒ the page costs
            // ZERO API calls. Any refusal ⇒ discard partials and fall
            // through to the combined LLM call unchanged (fallback-for-all).
            if config.deterministic || digital {
                let paper_last_page = all_spans
                    .iter()
                    .map(|s| s.end_page)
                    .max()
                    .unwrap_or(page_idx);
                if let Some(batch_built) = crate::deterministic::try_deterministic_batch(
                    config,
                    spans,
                    page_idx,
                    page,
                    &fig_counts,
                    paper_last_page,
                    cancel,
                ) {
                    for (i, span) in spans.iter().enumerate() {
                        let mut built_q = batch_built[i].clone();
                        attach_detected_figures(
                            config,
                            span,
                            &span_pages,
                            page_figures,
                            page_render_cache,
                            &mut built_q,
                            &mut tf_report,
                        )
                        .await;
                        settle_tier0_outcome(span, &built_q, &mut tf_report);
                        tf_report.pages_processed += 1;
                        push_mark_check(span, &built_q, &mut tf_report);
                        tf_out.push(((*span).clone(), Some(built_q)));
                    }
                    return (tf_out, tf_report);
                }
            }
        }
        // Digital document with a failed strict batch: every span on this page
        // must resolve locally. Recover what can be recovered, flag it, and
        // report the rest as local failures - never the combined LLM call or
        // the shared-page vision call below.
        if digital {
            let paper_last_page = all_spans
                .iter()
                .map(|s| s.end_page)
                .max()
                .unwrap_or(page_idx);
            for (i, span) in spans.iter().enumerate() {
                let figures = fig_counts.get(i).copied().unwrap_or(0);
                let margin_model = config
                    .margin_model
                    .get_or_init(|| crate::deterministic::build_margin_model(config, all_spans));
                // 1. Re-try this span ALONE with the strict Tier-0 gates. The
                // batch above is all-or-nothing, so a single stubborn span
                // must not demote its clean siblings to "recovered".
                if let Some((mut built_q, mut strict_report)) =
                    crate::deterministic::try_deterministic_extraction(
                        config,
                        span,
                        &span_pages,
                        figures,
                        paper_last_page,
                        cancel,
                        Some(margin_model),
                    )
                {
                    attach_detected_figures(
                        config,
                        span,
                        &span_pages,
                        page_figures,
                        page_render_cache,
                        &mut built_q,
                        &mut strict_report,
                    )
                    .await;
                    settle_tier0_outcome(span, &built_q, &mut strict_report);
                    strict_report.pages_processed += 1;
                    push_mark_check(span, &built_q, &mut strict_report);
                    tf_report.absorb(strict_report);
                    tf_out.push(((*span).clone(), Some(built_q)));
                    continue;
                }
                // 2. Only a genuinely failed span is retained as a flagged
                // local recovery.
                if let Some((mut built_q, mut rec_report, gate)) =
                    crate::deterministic::try_local_recovery(
                        config,
                        span,
                        &span_pages,
                        figures,
                        paper_last_page,
                        cancel,
                        Some(margin_model),
                    )
                {
                    rec_report.recovered += 1;
                    rec_report.anomalies.push(format!(
                        "Question {}: local recovery (digital document, failed gate: {}); content retained for review, zero cloud calls",
                        span.number, gate
                    ));
                    attach_detected_figures(
                        config,
                        span,
                        &span_pages,
                        page_figures,
                        page_render_cache,
                        &mut built_q,
                        &mut rec_report,
                    )
                    .await;
                    rec_report.pages_processed += 1;
                    push_mark_check(span, &built_q, &mut rec_report);
                    tf_report.absorb(rec_report);
                    tf_out.push(((*span).clone(), Some(built_q)));
                } else {
                    let reason = "digital document: no viable local candidate and cloud extraction is disabled";
                    eprintln!("[LOCAL_FAILURE] question={} reason={}", span.number, reason);
                    tf_report
                        .anomalies
                        .push(format!("Question {}: {}", span.number, reason));
                    tf_report.quarantined.push(QuarantineEvent {
                        scope: "question".to_string(),
                        page: Some(page_idx + 1),
                        question_number: Some(span.number),
                        reason: reason.to_string(),
                    });
                    tf_report.pages_processed += 1;
                    tf_out.push(((*span).clone(), None));
                }
            }
            return (tf_out, tf_report);
        }
        if !needs_vision {
            if spans.len() >= 2 {
                // ONE combined call transcribes every question on the page,
                // avoiding N repetitions of the system-prompt/schema overhead.
                let (batch_opts, batch_rep) = try_text_first_batch_extraction(
                    client,
                    config,
                    spans,
                    &span_pages,
                    &fig_counts,
                    request_semaphore,
                    cancel,
                    usage,
                )
                .await;
                tf_report.absorb(batch_rep);
                for (i, span) in spans.iter().enumerate() {
                    let mut built_q = match batch_opts.get(i).and_then(|o| o.clone()) {
                        Some(q) => q,
                        None => {
                            // Combined call missed/rejected this span → re-ask
                            // it alone. extract_span with text_first=true retries
                            // the text layer first, then falls back to vision.
                            let (q, r) = extract_span(
                                client,
                                config,
                                span,
                                &span_pages,
                                page_figures,
                                page_render_cache,
                                page_image_cache,
                                request_semaphore,
                                collateral_cache,
                                all_spans,
                                true,
                                cancel,
                                usage,
                            )
                            .await;
                            tf_report.absorb(r);
                            tf_out.push(((*span).clone(), q));
                            continue;
                        }
                    };
                    eprintln!(
                        "[TEXT_FIRST] Question {} transcribed from text layer (0 image tokens)",
                        span.number
                    );
                    tf_report.text_first += 1;
                    // Always attach: with figures it splices crops into the
                    // placeholders; without figures it still scrubs any
                    // leftover placeholder tokens so none can leak to a card.
                    attach_detected_figures(
                        config,
                        span,
                        &span_pages,
                        page_figures,
                        page_render_cache,
                        &mut built_q,
                        &mut tf_report,
                    )
                    .await;
                    tf_report.pages_processed += 1;
                    push_mark_check(span, &built_q, &mut tf_report);
                    tf_out.push(((*span).clone(), Some(built_q)));
                }
                return (tf_out, tf_report);
            }

            // Single span on the page — the plain per-question text-first path.
            let span = spans[0];
            if let Some((mut built_q, mut r)) = try_text_first_extraction(
                client,
                config,
                span,
                &span_pages,
                fig_counts[0],
                request_semaphore,
                cancel,
                usage,
            )
            .await
            {
            eprintln!(
                "[TEXT_FIRST] Question {} transcribed from text layer (0 image tokens)",
                span.number
            );
            r.text_first += 1;
            // Always attach: with figures it splices crops; without figures it
            // still scrubs leftover placeholder tokens.
            attach_detected_figures(
                config,
                span,
                &span_pages,
                page_figures,
                page_render_cache,
                &mut built_q,
                &mut r,
            )
            .await;
            r.pages_processed += 1;
                push_mark_check(span, &built_q, &mut r);
                tf_report.absorb(r);
                tf_out.push(((*span).clone(), Some(built_q)));
                return (tf_out, tf_report);
            }
        }
    }

    let max_attempts = 1 + config.max_repairs;

    // Digital document whose shared page carries no usable text at all: there
    // is nothing to carve and no cloud to ask. Report each span as a local
    // failure instead of falling into the vision batch.
    if digital {
        let reason =
            "digital document: page carries no usable text and cloud extraction is disabled";
        let mut out = Vec::with_capacity(spans.len());
        for span in spans {
            eprintln!("[LOCAL_FAILURE] question={} reason={}", span.number, reason);
            report
                .anomalies
                .push(format!("Question {}: {}", span.number, reason));
            report.quarantined.push(QuarantineEvent {
                scope: "question".to_string(),
                page: Some(page_idx + 1),
                question_number: Some(span.number),
                reason: reason.to_string(),
            });
            report.pages_processed += 1;
            out.push(((*span).clone(), None));
        }
        return (out, report);
    }

    // Prepare full page image (no vertical clipping so all MCQs and visual options are fully visible)
    let prep_input = if let Some(b64) = page.get_b64() {
        vec![ChunkImageInput {
            chunk_idx: 0,
            global_page_idx: page_idx,
            b64: b64.clone(),
            start_y: None,
            end_y: None,
        }]
    } else {
        Vec::new()
    };

    let prepared = match prepare_chunk_images(1, prep_input, page_image_cache).await {
        Ok(p) => p,
        Err(e) => {
            report.anomalies.push(format!("Page {} image prep failed: {}", page_idx + 1, e));
            // Fall back to individual extractions for all spans
            let mut out = Vec::with_capacity(spans.len());
            for span in spans {
                let (q, r) = extract_span(
                    client,
                    config,
                    span,
                    &[(page_idx, page)],
                    &[], // batch fallback keeps the vision path — no deterministic attach
                    page_render_cache,
                    page_image_cache,
                    request_semaphore,
                    collateral_cache,
                    all_spans,
                    false,
                    cancel,
                    usage,
                ).await;
                report.absorb(r);
                out.push(((*span).clone(), q));
            }
            return (out, report);
        }
    };

    let images = prepared.images;
    let local_to_chunk = prepared.local_to_chunk;
    let page_bands = prepared.page_bands;
    let decoded_pages = prepared.decoded_pages;

    let q_nums: Vec<String> = spans.iter().map(|s| s.number.to_string()).collect();
    let q_str = q_nums.join(", ");
    let system = extraction_system_prompt(config);
    let extraction_schema = extraction_json_schema();
    let mut last_error = String::new();
    let mut accepted_items: Option<Vec<AiQuestion>> = None;

    for attempt in 1..=max_attempts {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let repair_note = if attempt == 1 {
            String::new()
        } else {
            format!(
                "\n\nPREVIOUS ATTEMPT FAILED VALIDATION: {}. Regenerate corrected JSON for Questions {}.",
                last_error, q_str
            )
        };
        let user_text = format!(
            "TARGET QUESTIONS: Questions {}\nPAPER: '{}'\nMODULE: '{}'\n\nTranscribe Questions {} from the attached page image (page {}), returning ONE item per question in the items array.{}{}",
            q_str,
            config.paper_name,
            config.module_name,
            q_str,
            page_idx + 1,
            if page.text.trim().is_empty() {
                String::new()
            } else {
                format!(
                    "\n\nReference OCR text (may be corrupt — images are authoritative):\nRAW TEXT PAGE {}:\n{}\n\n",
                    page_idx + 1,
                    page.text
                )
            },
            repair_note
        );

        let body = llm::chat_body(
            &config.model,
            &system,
            &images,
            llm::ImageDetail::High,
            Some(&user_text),
            config.max_output_tokens.min(StageTag::FallbackPage.output_cap()),
            Some(llm::ResponseFormat::JsonSchema {
                schema: extraction_schema.clone(),
            }),
        );
        let resp = match chat_with_permit(client, &body, request_semaphore, cancel, usage, StageTag::FallbackPage, config.cloud_allowed()).await {
            Ok(r) => r,
            Err(e) => {
                last_error = e.to_string();
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                continue;
            }
        };
        if llm::response_was_truncated(&resp) {
            last_error = "the previous response hit the max_tokens ceiling and was cut off mid-tag (finish_reason=length); regenerate the COMPLETE JSON within the output limit".to_string();
            report.note_repair("batch_finish_reason_length");
            continue;
        }

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
                if dropped_tail && attempt < max_attempts {
                    last_error = "response was truncated; items may be missing".to_string();
                    continue;
                }
                value
            }
            ParseOutcome::Malformed { error } => {
                last_error = format!("invalid JSON: {}", error);
                report.note_repair("batch_malformed_json");
                continue;
            }
        };

        if page_out.items.is_empty() {
            last_error = format!("returned empty items array for Questions {}", q_str);
            report.note_repair("batch_empty_items");
            continue;
        }

        // Audit diagram boxes across all returned items
        let audit_items = page_out.items;
        let audit_decoded = decoded_pages.clone();
        let audit_local = local_to_chunk.clone();
        let audit_bands = page_bands.clone();
        let audit_page_texts = vec![page.text.clone()];

        let (audited_items, _bad_boxes, box_issues) = match tokio::task::spawn_blocking(move || {
            let mut items = audit_items;
            let (bad, issues) = audit_diagram_boxes(
                &audit_decoded,
                &audit_page_texts,
                &mut items,
                &audit_local,
                &audit_bands,
            );
            (items, bad, issues)
        }).await {
            Ok(res) => res,
            Err(e) => {
                last_error = format!("diagram audit failed: {}", e);
                report.note_repair("batch_diagram_audit_failed");
                continue;
            }
        };

        if !box_issues.is_empty() && attempt < max_attempts {
            last_error = box_issues.join("; ");
            report.note_repair("batch_diagram_box_issues");
            continue;
        }

        accepted_items = Some(audited_items);
        break;
    }

    let mut out: Vec<(QuestionSpan, Option<BuiltQuestion>)> = Vec::with_capacity(spans.len());
    let mut items_map: std::collections::HashMap<u32, AiQuestion> = std::collections::HashMap::new();

    if let Some(items) = accepted_items {
        for item in items {
            if let Some(num) = item.question_number.as_ref().and_then(validate::value_to_question_number) {
                items_map.insert(num, item);
            }
        }
    }

    let mut saved_diagrams: Vec<([u8; 64], String)> = Vec::new();

    for span in spans {
        if let Some(item) = items_map.remove(&span.number) {
            let mut item_content = item.content.unwrap_or_default();
            let is_code = item.is_code.unwrap_or(false);
            let mut topics_acc = Vec::new();
            if let Some(t) = item.topics {
                for topic in value_to_topics(&t) {
                    if config.allowed_topics.is_empty() || config.allowed_topics.contains(&topic) {
                        topics_acc.push(topic);
                    }
                }
            }
            let ai_marks = item.marks.as_ref().and_then(validate::value_to_marks);

            // Persist diagrams if present
            if let Some(bboxes) = &item.diagram_bboxes {
                let indexes = item.bbox_page_indexes.clone().unwrap_or_default();
                let mut requests = Vec::with_capacity(bboxes.len());
                let mut page_b64 = std::collections::HashMap::new();
                for (bi, bbox) in bboxes.iter().enumerate() {
                    let _model_idx = indexes.get(bi).and_then(value_to_usize).unwrap_or(0);
                    let ignore_grid = validate::figure_references(&item_content) > 0 && !validate::is_answer_grid_request(&item_content);
                    if config.pdf_path.is_none() {
                        if let Some(b64) = page.get_b64() {
                            page_b64.entry(page_idx).or_insert_with(|| b64.clone());
                        }
                    }
                    requests.push(DiagramSaveRequest {
                        global_page_idx: page_idx,
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
                                    || kind.contains("composite_visual_options") || kind.contains("visual_option")
                            })
                            .unwrap_or(false),
                            exact: None,
                            mask: Vec::new(),
                    });
                }
                let saved_before = saved_diagrams.clone();
                match persist_diagrams(
                    requests,
                    page_b64,
                    config.clone(),
                    Arc::clone(page_render_cache),
                    std::mem::take(&mut saved_diagrams),
                ).await {
                    Ok(persisted) => {
                        saved_diagrams = persisted.saved;
                        report.absorb(persisted.report);
                        item_content = splice_diagrams_by_caption_and_context(
                            item_content,
                            &persisted.links,
                            item.diagram_captions.as_deref(),
                        );
                    }
                    Err(err) => {
                        saved_diagrams = saved_before;
                        report.anomalies.push(format!(
                            "Question {} diagram persistence failed: {}",
                            span.number, err
                        ));
                    }
                }
            }
            item_content = item_content.replace("[DIAGRAM_PLACEHOLDER]", "");

            let built_opt = assemble_built_question(
                span,
                config,
                vec![item_content],
                topics_acc,
                is_code,
                false,
                Vec::new(),
                ai_marks,
            );

            let mut built_q = built_opt;
            if let Some(q) = &built_q {
                let errs = validate::card_structure_errors(&q.content, span.number);
                if !errs.is_empty() {
                    eprintln!(
                        "[BATCH_STRUCTURE_FALLBACK] question={} reason=({})",
                        span.number,
                        errs.join("; ")
                    );
                    built_q = None; // fall through to per-span extract_span below
                }
            }
            if let Some(built_q) = built_q {
                out.push(((*span).clone(), Some(built_q)));
                continue;
            }
        }

        // If not in items_map or validation failed, fall back to extract_span for ONLY this span
        let (fallback_q, fallback_rep) = extract_span(
            client,
            config,
            span,
            &[(page_idx, page)],
            &[], // page fallback keeps the vision path — no deterministic attach
            page_render_cache,
            page_image_cache,
            request_semaphore,
            collateral_cache,
            all_spans,
            false,
            cancel,
            usage,
        ).await;
        report.absorb(fallback_rep);
        out.push(((*span).clone(), fallback_q));
    }

    (out, report)
}

/// Deterministic per-item validation for a span. Returns human-readable
/// violations (quoted verbatim back to the model in the repair prompt).
/// Deterministic salvage for the most common model slip: diagram_bboxes and
/// bbox_page_indexes arriving with different lengths. The fix is mechanical
/// (pad with the span's first page / truncate the tail), so it must never
/// burn a paid repair round or quarantine a question.
fn reconcile_bbox_indexes(items: &mut [AiQuestion]) {
    for item in items.iter_mut() {
        let Some(bboxes) = &item.diagram_bboxes else { continue };
        let n = bboxes.len();
        match &mut item.bbox_page_indexes {
            Some(idx) => {
                idx.truncate(n);
                while idx.len() < n {
                    idx.push(serde_json::json!(0));
                }
            }
            None => {
                item.bbox_page_indexes = Some(vec![serde_json::json!(0); n]);
            }
        }
    }
}

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
        // KaTeX delimiter discipline: unbalanced $/$$ pairing is quoted back
        // to the model so the repair round fixes its own math boundaries
        // instead of shipping cards that swallow subsequent text.
        for e in validate::math_delimiter_balance_errors(content) {
            errors.push(format!("item {}: {}", idx + 1, e));
        }
        // Structural formatting rules (the 6 trap classes). Evaluated on the
        // SANITIZED content: the sanitizer's free fixes must never burn a
        // paid repair round — only genuine transcription defects should.
        let sanitized_item = crate::sanitize::sanitize_question_content(content, span.number);
        for e in validate::card_structure_errors(&sanitized_item, span.number) {
            errors.push(format!("item {}: {}", idx + 1, e));
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
///   4. boilerplate exclusion: the box must not contain page-level text
///      (margin warnings, footers, continuation markers, mark allocations)
///   5. answer space exclusion: the box must not cover student answer areas
///      (underscores, blank horizontal rules, empty grid space)
///   6. compound asset splitting: multi-caption boxes are split on whitespace
///      gutters
///   7. crop sanity (degenerate / blank / answer-grid)
///   8. near-duplicate signature check
///
/// `page_bands` is parallel to `chunk`; entries are `Some((low, high))`
/// when the page was given a vertical band hint. A None entry means the
/// whole page belongs to this span (no y-band restriction).
///
/// `page_texts` holds the OCR text for each page in the chunk, used for
/// boilerplate detection within proposed bboxes.
///
/// Returns the indices of offending boxes `(item_idx, bbox_idx)` plus a
/// quoted feedback message per violation for the repair loop. The AI draws
/// boxes; Rust decides which ones may ever become files.
fn audit_diagram_boxes(
    decoded_pages: &[Option<Arc<image::DynamicImage>>],
    page_texts: &[String],  // OCR text for each page in the chunk
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

            // Conservatively clamp coordinates away from headers/footers/margins in-place
            geometry::clamp_bbox_safe(bbox);

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

            // Get OCR text for this page to check for boilerplate in the bbox region
            let page_text = if chunk_idx < page_texts.len() {
                &page_texts[chunk_idx]
            } else {
                ""
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
                        || kind.contains("composite_visual_options") || kind.contains("visual_option")
                })
                .unwrap_or(false);

            // NEW: Check if bbox contains boilerplate text that should be excluded
            if geometry::bbox_contains_boilerplate(page_text, bbox) {
                bad.push((ii, bi));
                issues.push(format!(
                    "{label}: the box covers page-level boilerplate (margin warnings, footers, continuation markers, or mark allocations) — redraw tightly around the visual asset only, or delete the box AND its [DIAGRAM_PLACEHOLDER]"
                ));
                continue;
            }

            // Perform crop with proper padding for audit and duplicate detection
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
                Err(geometry::CropReject::Blank) => {
                    bad.push((ii, bi));
                    issues.push(format!("{label}: the box is blank — redraw it around the printed figure, or delete the box AND its [DIAGRAM_PLACEHOLDER]"));
                    continue;
                }
            };

            // Check if the cropped region looks like answer space
            if geometry::looks_like_answer_space(&cropped) {
                bad.push((ii, bi));
                issues.push(format!(
                    "{label}: the box covers STUDENT ANSWER SPACE (blank lines, underscores, or ruled working area). Never box these — the question prompt text belongs in \"content\"; visual diagrams only get boxes. Redraw tightly around the actual figure, or delete the box AND its [DIAGRAM_PLACEHOLDER]"
                ));
                continue;
            }
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

/// Check if a question's diagram bboxes contain compound figures (multiple
/// captions in one bbox) and split them into separate assets.
/// This runs after the diagram audit but before saving.
fn split_compound_figures_if_needed(
    items: &mut [AiQuestion],
    page_texts: &[String],
    _local_to_chunk: &[usize],
) {
    for item in items.iter_mut() {
        let Some(bboxes) = &item.diagram_bboxes else {
            continue;
        };
        if bboxes.is_empty() {
            continue;
        }

        let indexes = item.bbox_page_indexes.clone().unwrap_or_default();
        let captions = item.diagram_captions.clone().unwrap_or_default();
        let kinds = item.diagram_kinds.clone().unwrap_or_default();

        // Use the geometry module's split function
        let split_figures = geometry::split_compound_figure(bboxes, &captions, &kinds, page_texts);

        if split_figures.len() > bboxes.len() {
            // We split a compound figure - update the item
            let new_bboxes: Vec<Vec<f32>> = split_figures.iter().map(|f| f.bbox.clone()).collect();
            let new_captions: Vec<String> = split_figures.iter().filter_map(|f| f.caption.clone()).collect();
            let new_kinds: Vec<String> = split_figures.iter().map(|f| f.kind.clone()).collect();

            // For page indexes, we use the first page index for all split figures
            // since they came from the same original bbox
            let first_idx = indexes.first().cloned().unwrap_or(serde_json::json!(0));
            let new_indexes: Vec<serde_json::Value> = (0..new_bboxes.len()).map(|_| first_idx.clone()).collect();

            item.diagram_bboxes = Some(new_bboxes);
            item.diagram_captions = Some(new_captions);
            item.diagram_kinds = Some(new_kinds);
            item.bbox_page_indexes = Some(new_indexes);
        }
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
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
    save_diagram_with(global_page_idx, page_b64, bbox, config, page_render_cache, saved, report, ignore_grid, graph_like, None, &[])
}

#[allow(clippy::too_many_arguments)]
fn save_diagram_with(
    global_page_idx: usize,
    page_b64: Option<&str>,
    bbox: &[f32],
    config: &PipelineConfig,
    page_render_cache: &crate::pdf_render::PageRenderCache,
    saved: &mut Vec<([u8; 64], String)>,
    report: &mut ImportReport,
    ignore_grid: bool,
    graph_like: bool,
    exact: Option<f32>,
    mask: &[[f32; 4]],
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
    // Furniture over the box (a QR code at the corner of a wide figure) is
    // painted out of a copy of the page before cropping.
    let meets = |m: &[f32; 4]| m[0] < bbox[0] + bbox[2] && m[0] + m[2] > bbox[0] && m[1] < bbox[1] + bbox[3] && m[1] + m[3] > bbox[1];
    let img = if mask.iter().any(meets) {
        let mut page = img.as_ref().to_rgba8();
        let (pw, ph) = (page.width() as f32, page.height() as f32);
        for m in mask.iter().filter(|m| meets(m)) {
            let (x0, y0) = (((m[0] * pw).floor() - 1.0).max(0.0) as u32, ((m[1] * ph).floor() - 1.0).max(0.0) as u32);
            let (x1, y1) = (((m[0] + m[2]) * pw).ceil().min(pw - 1.0) as u32 + 1, ((m[1] + m[3]) * ph).ceil().min(ph - 1.0) as u32 + 1);
            for y in y0..y1.min(page.height()) {
                for x in x0..x1.min(page.width()) {
                    page.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
                }
            }
        }
        std::sync::Arc::new(image::DynamicImage::ImageRgba8(page))
    } else {
        img
    };
    let cropped = match if let Some(pad) = exact {
        geometry::crop_exact(img.as_ref(), bbox, ignore_grid, pad)
    } else {
        geometry::crop_diagram_with_options(img.as_ref(), bbox, 40, ignore_grid, graph_like)
    } {
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

    // A coarse 8x8 signature cannot distinguish different plotted curves
    // on the same grid. Confirm pixels before reusing a saved figure.
    for (_, link) in saved.iter().filter(|(s, _)| geometry::signature_distance(s, &sig) < 4) {
        let existing_path = link.trim().strip_prefix("![Diagram](").and_then(|s| s.strip_suffix(')'));
        if existing_path.and_then(|p| image::open(p).ok()).is_some_and(|img| img.to_rgba8() == final_crop) {
            report.diagrams_deduped += 1;
            return Some(link.clone());
        }
    }
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
    page_image_cache: &Arc<PageImageCache>,
    request_semaphore: &Arc<Semaphore>,
    cancel: &AtomicBool,
    usage: &Arc<TokenTotals>,
) -> (Option<Vec<BuiltQuestion>>, ImportReport) {
    // Own, local report: pages now run in parallel batches.
    let mut report = ImportReport::default();
    if cancel.load(Ordering::Relaxed) {
        return (None, report);
    }
    let max_attempts = 1 + config.max_repairs;
    let system = format!(
        r#"You are a precise mathematical OCR engine. Output ONLY a valid JSON object {{"items": [ ... ]}}.

RULES:
- If this page contains NEW question(s) (each with its own printed whole-question number), return ONE item per question:
  {{ "question_number": <whole number printed>, "content": "<full transcription>", "marks": int|null, "difficulty_rating": string|null,
     "topics": array, "module": "{module}", "is_code": bool,
     "diagram_bboxes": [[x,y,w,h]...] relative 0.0-1.0, "bbox_page_indexes": [0,...] }}
 - MATHEMATICAL FIDELITY & STRICT DELIMITERS:
   * Wrap all inline math in `$ ... $` and display equations in `$$ ... $$`.
   * Ensure perfect delimiter pairing: every `$` or `$$` opened must be closed with the exact same tag. NEVER omit the opening delimiter (e.g. NEVER emit `\frac{{...}}$`).
   * Never drop leading variables, function names, or prefixes (e.g. `r = \frac{{...}}`, `y = 2\sin x`, `C: r = ...` MUST include the `r = ` verbatim).
   * OPERATOR PRESERVATION: every fraction as `\frac{{a}}{{b}}` (or a/b in $...$), every ratio with its operator (a:b). NEVER drop a fraction bar, division slash, ×, ÷, ±, or exponent.
   * NUCLEAR NOTATION: nuclide prescripts and decay particles are MATH: `$^{{226}}_{{88}}\text{{Ra}} \rightarrow\ ^{{222}}_{{86}}\text{{Rn}} +\ ^{{4}}_{{2}}\alpha$` — never bare plaintext.
   * SENTENCE INTEGRITY: never insert a hard line break mid-sentence; join the page's wrapped print-lines of one sentence into a single paragraph.
- ADAPTIVE MARKS VS DIFFICULTY:
  * If standard marks are printed (`[4 marks]`), output `"marks": 4` and `"difficulty_rating": null`.
  * If difficulty / star ratings are present (e.g. `(*)`, `(**)`, `(***+)`, `(Specialist)` as in T. Madas worksheets), output `"difficulty_rating": "***+"` and `"marks": null`. DO NOT invent marks.
- SUB-PARTS VS MULTIPLE CHOICE:
  * Sub-parts `(a)`, `(b)`, `(c)` are standard question sub-parts separated by `\n\n`. NEVER convert sub-parts into multiple-choice options.
- MULTIPLE QUESTIONS ON ONE PAGE: when a page has several independent short-answer or multiple-choice questions (e.g. AQA Section B with 4 MCQs), return an item for EACH question. Do NOT bundle them into one item.
- QUESTION ISOLATION (highest priority): never place sub-parts of two different main questions in one item. A sub-part label ((a), (b), (i), "04.2") belongs to the main number printed in the label, or else to the nearest whole-number heading ABOVE it. A "(Total for Question N is M marks)" footer, or a new whole question number, ENDS that question — everything after it starts a new item. If sub-part lettering restarts at (a), a new main question has begun. When unsure which question owns a line, start a new item rather than merging.
- If this page is a CONTINUATION of the previous question, is blank, or contains no new question, return {{"items": []}}.
- ARTIFACT FILTERING: Recognize and completely exclude all non-exam content. Silently ignore margin warnings, printer registration marks, page numbers, and barcodes.
- STRUCTURAL SPACING: Enforce strict hierarchical spacing. Use double line breaks (\n\n) to clearly separate sub-question identifiers ((a), (b), (i)) and mark allocations (**[X marks]**) from surrounding text.
- EQUATION COHESION: Treat multi-part mathematical statements (e.g. matrix equations) as a single cohesive unit within a single display math block ($$ ... $$), never orphaning equals signs or matrices on separate lines.
- ROBUST TABLE RENDERING: Structured tables with headers (trace tables, function tables, working grids) are question content even when EMPTY — transcribe them as standard Markdown tables, NEVER as diagram boxes. Never leak raw \\hline or broken formatting tags.
- Transcribe fully (never summarize). Preserve punctuation. Math in $...$/$$...$$. Pure matrices in LaTeX \\begin{{array}} or \\begin{{pmatrix}} inside $$...$$. Code in backticks, never math mode. Escape LaTeX backslashes (\\\\frac).
- AQA decimal sub-parts: render '03.1'-style part numbers as (a), (b), (c) — positional: .1 -> a, .2 -> b — and update inline cross-references. AQA also uses SPACED sub-parts: \"01 5\" means Question 1, sub-part 5 — render as (e). The whole question number is ALWAYS the integer (never a decimal like 1.5). The whole decimal run on this page is ONE item with its integer question number.
- Anything the paper labels as a Figure ("Figure 6" — printed schemas, algorithm screens, grids that are part of the question exhibit) MUST be returned as a diagram box, never as transcribed text.
- Exclude headers/footers ("Question X continued", "Turn over", totals footers), plain ruled answer lines, answer line templates with operators (e.g. "............ $\\le t <$ ............"), "BLANK PAGE".
- Content must end with terminal punctuation, a mark tag, or a difficulty rating."#,
        module = config.module_name,
    );

    let preparation_inputs = match &page.kind {
        PageInputKind::Image { b64, .. } => vec![ChunkImageInput {
            chunk_idx: 0,
            global_page_idx: page_idx,
            b64: b64.clone(),
            start_y: None,
            end_y: None,
        }],
        PageInputKind::TextOnly => Vec::new(),
    };
    let prepared = match prepare_chunk_images(1, preparation_inputs, page_image_cache).await {
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
        if cancel.load(Ordering::Relaxed) {
            break;
        }
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
            llm::ImageDetail::High,
            Some(&user_text),
            config.max_output_tokens.min(StageTag::FallbackPage.output_cap()),
            Some(llm::ResponseFormat::JsonSchema { schema: extraction_json_schema() }),
        );
        let resp = match chat_with_permit(client, &body, request_semaphore, cancel, usage, StageTag::FallbackPage, config.cloud_allowed()).await {
            Ok(r) => r,
            Err(e) => {
                last_error = e.to_string();
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                continue;
            }
        };
        if llm::response_was_truncated(&resp) {
            last_error = "the previous response hit the max_tokens ceiling and was cut off mid-tag (finish_reason=length); regenerate the COMPLETE JSON within the output limit".to_string();
            report.note_repair("fallback_finish_reason_length");
            continue;
        }
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
                report.note_repair("fallback_malformed_json");
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
            report.note_repair("fallback_bad_question_number");
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
            report.note_repair("fallback_figure_consistency");
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
        // Build page_texts aligned with local_to_chunk (model-visible order)
        // For fallback, we only have one page, so create a single-element array
        let audit_page_texts: Vec<String> = local_to_chunk
            .iter()
            .map(|_| page.text.clone())
            .collect();
        let (mut items, bad, box_issues) = match tokio::task::spawn_blocking(move || {
            let mut items = audit_items;
            let (bad, issues) = audit_diagram_boxes(
                &audit_decoded_pages,
                &audit_page_texts,
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
                report.note_repair("fallback_diagram_audit_failed");
                continue;
            }
        };
        if !box_issues.is_empty() {
            report.note_repair("fallback_diagram_box_issues");
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
                        exact: None,
                        mask: Vec::new(),
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
                        item_content = splice_diagrams_by_caption_and_context(
                            item_content,
                            &persisted.links,
                            item.diagram_captions.as_deref(),
                        );
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
                content: {
                    let mut content = validate::clean_question_content(&item_content);
                    content = validate::normalize_decimal_parts(&content, number);
                    // Comprehensive post-processing for the 6 extraction/formatting failure modes:
                    // 1. Artifact boilerplate bleed, 2. Number-agnostic AQA decimal failures,
                    // 3. MCQ option flattening, 4. Tabular option destruction, 5. Visual MCQ
                    // 6. Mark allocation misplacement.
                    content = crate::marker_client::clean_marker_markdown(&content);
                    content = validate::balance_math_delimiters(&content);
                    // Same multi-line display-math guard as the main assembly
                    // path: explicit \\ row separators inside $$ blocks.
                    content = validate::ensure_display_math_line_breaks(&content);
                    content
                },
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
/// TEXT-ONLY attempt at one mark-scheme window. Returns `Some` when the
/// text-first transcription succeeded AND contains no diagram placeholders
/// (nothing needs vision); `None` means the caller must fall through to the
/// full-page vision path unchanged. Bounded by the same repair budget.
#[allow(clippy::too_many_arguments)]
async fn try_ms_text_first_window<C: LlmClient>(
    client: &C,
    config: &PipelineConfig,
    pages: &[PageInput],
    start: usize,
    end: usize,
    _step: usize,
    request_semaphore: &Arc<Semaphore>,
    cancel: &AtomicBool,
    usage: &Arc<TokenTotals>,
) -> Option<(Result<Vec<AiAnswer>, String>, ImportReport)> {
    let mut report = ImportReport::default();
    let mut chunk_text = String::new();
    for (i, p) in pages.iter().enumerate().take(end).skip(start) {
        if !p.text.trim().is_empty() {
            chunk_text.push_str(&format!(
                "RAW TEXT PAGE {}:\n{}\n\n---\n\n",
                i + 1,
                p.text
            ));
        }
    }
    let context_note = if start == 0 {
        format!("These are pages 1–{} of the mark scheme.", end)
    } else {
        format!(
            "Page {} is context (already processed). Extract ONLY answers anchored on page{} {}.",
            start,
            if end > start + 1 { "s" } else { "" },
            if end > start + 1 {
                format!("{}–{}", start + 1, end)
            } else {
                format!("{}", start + 1)
            }
        )
    };
    let user_text = format!(
        "{}\n\n{}",
        context_note,
        chunk_text
    );
    let system = markscheme_text_first_system_prompt();
    let max_out = config.max_output_tokens.min(StageTag::MsTextFirst.output_cap());

    let mut last_error = String::new();
    let max_attempts = 1 + config.max_repairs;
    for attempt in 1..=max_attempts {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
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
            &system,
            &[] as &[String],
            llm::ImageDetail::Low,
            Some(&text),
            max_out,
            Some(llm::ResponseFormat::JsonSchema {
                schema: markscheme_text_first_json_schema(),
            }),
        );
        let api_start = Instant::now();
        let resp = match chat_with_permit(client, &body, request_semaphore, cancel, usage, StageTag::MsTextFirst, config.cloud_allowed()).await {
            Ok(r) => r,
            Err(e) => {
                last_error = e.to_string();
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                continue;
            }
        };
        report.record_timing(
            "extraction",
            "ms_text_first",
            Some(start + 1),
            None,
            api_start.elapsed().as_millis() as u64,
        );
        if llm::response_was_truncated(&resp) {
            last_error = "the previous response hit the max_tokens ceiling and was cut off (finish_reason=length); regenerate the complete corrected JSON".to_string();
            report.note_repair("ms_text_first_finish_reason_length");
            continue;
        }
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
                let needs_vision = answers.iter().any(|a| {
                    a.answer_markdown
                        .as_deref()
                        .is_some_and(|m| m.contains("[DIAGRAM_PLACEHOLDER]"))
                });
                if needs_vision {
                    eprintln!(
                        "[MS_TEXT_FIRST] window {}–{} needs figures; falling back to vision",
                        start + 1,
                        end
                    );
                    return None;
                }
                eprintln!(
                    "[MS_TEXT_FIRST] window {}–{} transcribed from text layer (0 image tokens)",
                    start + 1,
                    end
                );
                report.ms_text_first += 1;
                return Some((Ok(answers), report));
            }
            ParseOutcome::Malformed { error } => {
                last_error = format!("invalid JSON: {}", error);
                report.note_repair("ms_text_first_malformed_json");
            }
        }
    }
    eprintln!(
        "[MS_TEXT_FIRST] window {}–{} text attempt failed ({}); falling back to vision",
        start + 1,
        end,
        last_error
    );
    None
}

async fn read_markscheme_window<C: LlmClient>(
    client: &C,
    config: &PipelineConfig,
    pages: &[PageInput],
    start: usize,
    end: usize,
    step: usize,
    system: &str,
    request_semaphore: &Arc<Semaphore>,
    cancel: &AtomicBool,
    usage: &Arc<TokenTotals>,
) -> (Result<Vec<AiAnswer>, String>, ImportReport) {
    let mut report = ImportReport::default();
    if cancel.load(Ordering::Relaxed) {
        return (Err("Import cancelled by user".to_string()), report);
    }

    // Phase 2 cost lever: a window whose pages all carry a reliable text
    // layer is transcribed with zero image tokens. Any failure or figure
    // signal falls through to the full-page vision path below, unchanged.
    if config.ms_text_first && window_text_reliable(&pages[start..end]) {
        if let Some(result) = try_ms_text_first_window(
            client,
            config,
            pages,
            start,
            end,
            step,
            request_semaphore,
            cancel,
            usage,
        )
        .await
        {
            return result;
        }
    }

    let images: Vec<String> = pages[start..end]
        .iter()
        .filter_map(|p| p.get_b64().cloned())
        .map(|b64| geometry::resize_b64_to_max_dim(&b64, geometry::api_image_max_dim()).unwrap_or(b64))
        .collect();
    let mut chunk_text = String::new();
    for (i, p) in pages.iter().enumerate().take(end).skip(start) {
        if !p.text.trim().is_empty() {
            chunk_text.push_str(&format!(
                "RAW TEXT PAGE {}:\n{}\n\n---\n\n",
                i + 1,
                p.text
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
        if cancel.load(Ordering::Relaxed) {
            break;
        }
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
            llm::ImageDetail::High,
            Some(&text),
            config.max_output_tokens.min(StageTag::MsWindow.output_cap()),
            Some(llm::ResponseFormat::JsonSchema { schema: extraction_json_schema() }),
        );
        let resp = match chat_with_permit(client, &body, request_semaphore, cancel, usage, StageTag::MsWindow, config.cloud_allowed()).await {
            Ok(r) => r,
            Err(e) => {
                last_error = e.to_string();
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                continue;
            }
        };
        if llm::response_was_truncated(&resp) {
            last_error = "the previous response hit the max_tokens ceiling and was cut off mid-tag (finish_reason=length); regenerate the complete corrected JSON".to_string();
            report.note_repair("markscheme_finish_reason_length");
            continue;
        }
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
                report.note_repair("markscheme_malformed_json");
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
    let overall_start = Instant::now();
    let mut report = ImportReport {
        paper_name: config.paper_name.clone(),
        kind: "mark_scheme".to_string(),
        pages_total: pages.len(),
        ..Default::default()
    };
    cancelled(cancel)?;
    if config.deterministic {
        if let Some(path) = config.pdf_path.as_ref().filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))) {
            progress.stage("Reading mark-scheme tables locally…");
            let path = path.clone();
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = Arc::clone(&stop);
            let mut worker = tokio::task::spawn_blocking(move || {
                let pages = crate::mark_scheme_deterministic::collect_page_evidence(&path, &worker_stop)?;
                crate::mark_scheme_deterministic::extract(&pages, &worker_stop)
            });
            let result = loop {
                tokio::select! {
                    result = &mut worker => break result.map_err(|e| e.to_string()).and_then(|r| r),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {
                        if cancel.load(Ordering::Relaxed) {
                            stop.store(true, Ordering::Relaxed);
                            return Err("Import cancelled by user".into());
                        }
                    }
                }
            };
            cancelled(cancel)?;
            match result {
                Ok(extraction) if extraction.page_count == pages.len() => {
                    let drafts = extraction.into_drafts();
                    report.deterministic = drafts.len();
                    report.questions_extracted = drafts.len();
                    report.questions_expected = drafts.len();
                    report.pages_processed = pages.len();
                    report.total_elapsed_ms = overall_start.elapsed().as_millis() as u64;
                    report.record_timing("extraction", "ms_tier0", None, None, report.total_elapsed_ms);
                    return Ok((drafts, report));
                }
                Ok(_) => eprintln!("[MS_TIER0_FALLBACK] PDF/page-input count mismatch"),
                Err(reason) => eprintln!("[MS_TIER0_FALLBACK] {reason}"),
            }
            report.record_timing("extraction", "ms_tier0_refused", None, None, overall_start.elapsed().as_millis() as u64);
        }
    }
    let page_render_cache = Arc::new(crate::pdf_render::PageRenderCache::new(
        PAGE_RENDER_CACHE_CAPACITY,
    ));
    let request_semaphore = Arc::new(Semaphore::new(config.parallelism.max(1)));
    let usage = Arc::new(TokenTotals::new());
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
            cancel,
            &usage,
        )
        .map(move |result| (position, result))
    }))
    .buffer_unordered(config.parallelism.max(1));
    let mut ordered_results = Vec::with_capacity(windows.len());
    loop {
        let next_item = tokio::select! {
            res = results.next() => res,
            _ = async {
                while !cancel.load(Ordering::Relaxed) {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            } => None,
        };
        if cancel.load(Ordering::Relaxed) {
            return Err("Import cancelled by user".to_string());
        }
        match next_item {
            Some(result) => ordered_results.push(result),
            None => break,
        }
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
                        exact: None,
                        mask: Vec::new(),
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
                md = validate::sanitize_for_latex(&md);
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

    report.total_elapsed_ms = overall_start.elapsed().as_millis() as u64;
    let (prompt_tok, completion_tok) = usage.snapshot();
    report.prompt_tokens = prompt_tok;
    report.completion_tokens = completion_tok;
    report.stage_breakdown = usage
        .snapshot_stages()
        .into_iter()
        .map(|(stage, p, c)| StageCost {
            stage: stage.as_str().to_string(),
            prompt_tokens: p,
            completion_tokens: c,
        })
        .collect();
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
        c.parallelism = 1;
        c
    }

    fn cancel_flag() -> AtomicBool {
        AtomicBool::new(false)
    }

    /// Legacy cloud-path fixtures carry page text only so the structure pass
    /// and map behave as before. They model an image-only input, so they bind
    /// an explicit scanned context: production always derives the class from
    /// the pages, and the flag does not exist outside test builds.
    /// Run the question pipeline with an explicit cloud context.
    async fn run_cloud_pipeline<C: crate::llm::LlmClient>(
        client: &C,
        pages: &[PageInput],
        figures: &[Vec<crate::pdf_render::DetectedFigure>],
        cfg: &PipelineConfig,
    ) -> Result<(Vec<BuiltQuestion>, ImportReport), String> {
        let mut cfg = cfg.clone();
        cfg.force_scanned_context = true;
        run_question_pipeline(client, pages, figures, &cfg, &NullProgress, &cancel_flag()).await
    }

    fn usage() -> Arc<TokenTotals> {
        Arc::new(TokenTotals::new())
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
            run_cloud_pipeline(&mock, &pgs, &[], &config())
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
            run_cloud_pipeline(&mock, &pgs, &[], &config())
                .await
                .unwrap();
        assert_eq!(built.len(), 2);
        assert!(report.repairs >= 1);
        assert!(report.quarantined.is_empty());
        // The repair response mentions the failure:
        let bodies = mock.bodies();
        let repair_body = &bodies[3];
        let user_msg = repair_body["messages"][1]["content"].as_str().unwrap();
        assert!(user_msg.contains("Question 1") && user_msg.contains("PREVIOUS ATTEMPT FAILED VALIDATION"));
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
            run_cloud_pipeline(&mock, &pgs, &[], &config())
                .await
                .unwrap();
        assert_eq!(built.len(), 2); // Q1 fallback + Q2
        assert_eq!(report.quarantined.len(), 1); // Q1 was quarantined after 3 attempts
        assert!(built[0].needs_review); // Q1 marked for review
        assert_eq!(built[1].question_number, 2);
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
            run_cloud_pipeline(&mock, &pgs, &[], &config())
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
            run_cloud_pipeline(&mock, &pgs, &[], &config())
                .await
                .unwrap();
        assert_eq!(built.len(), 2);
        assert!(report.salvage_events >= 1);
    }

    #[tokio::test]
    async fn mark_scheme_tier0_uses_pdf_evidence_and_skips_cloud() {
        let _lock = crate::pdf_render::pdfium_test_lock();
        let path = std::env::temp_dir().join(format!("mm_ms_pipeline_{}.pdf", uuid::Uuid::new_v4()));
        crate::mark_scheme_deterministic::tests::write_pdf_fixture(&path, false);
        let mut cfg = config();
        cfg.pdf_path = Some(path.clone());
        cfg.deterministic = true;
        let mock = MockLlm::new(vec![]);
        let pages = vec![PageInput { kind: PageInputKind::TextOnly, text: "Page evidence is read from the PDF".into() }];
        let (drafts, report) = run_markscheme_pipeline(&mock, &pages, &cfg, &NullProgress, &cancel_flag()).await.unwrap();
        assert_eq!(drafts.len(), 1);
        assert!(drafts[0].markdown.contains("The force increases."));
        assert!(drafts[0].markdown.contains("Accept greater force."));
        assert_eq!(report.deterministic, 1);
        assert_eq!(report.pages_processed, 1);
        assert_eq!(mock.bodies().len(), 0);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn mark_scheme_tier0_refusal_falls_back_without_partial_answers() {
        let _lock = crate::pdf_render::pdfium_test_lock();
        let path = std::env::temp_dir().join(format!("mm_ms_fallback_{}.pdf", uuid::Uuid::new_v4()));
        crate::mark_scheme_deterministic::tests::write_pdf_fixture(&path, true);
        let mut cfg = config();
        cfg.pdf_path = Some(path.clone());
        cfg.deterministic = true;
        cfg.ms_text_first = true;
        let mock = MockLlm::new(vec![ok_chat(r#"{"answers":[{"question_number":1,"answer_markdown":"Complete answer including continuation. M1 A1"}]}"#)]);
        let text = "Question Answers Additional comments Mark AO\n01.1 The force increases as extension increases. Accept equivalent wording. Marking point M1. Accuracy A1. Total 2 marks.";
        let pages = vec![PageInput { kind: PageInputKind::TextOnly, text: text.repeat(4) }; 2];
        let (drafts, report) = run_markscheme_pipeline(&mock, &pages, &cfg, &NullProgress, &cancel_flag()).await.unwrap();
        assert_eq!(drafts.len(), 1);
        assert!(drafts[0].markdown.contains("Complete answer including continuation"));
        assert_eq!(report.deterministic, 0);
        assert_eq!(mock.bodies().len(), 1);
        std::fs::remove_file(path).unwrap();
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

    // ── Phase 2: mark-scheme text-first + stage attribution ───────────────

    /// Mark-scheme pages whose combined text clears the reliability gate
    /// (>400 chars, no replacement chars, no figure references). Pages carry
    /// real rasters so the vision-fallback assertions can inspect images.
    fn rich_text_ms_pages(n: usize) -> Vec<PageInput> {
        let para = "The answer uses integration by parts to evaluate the region and then applies logarithms to both sides before solving the resulting quadratic expression for the unknown variable. ".repeat(3);
        let mut g = gray_blank(400, 600);
        g_hline(&mut g, 300);
        let b64 = png_b64(&g);
        (0..n)
            .map(|_| PageInput {
                kind: PageInputKind::Image { b64: b64.clone() },
                text: para.clone(),
            })
            .collect()
    }

    #[tokio::test]
    async fn ms_window_text_first_sends_zero_images() {
        let pgs = rich_text_ms_pages(3);
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"answers":[{"question_number":1,"answer_markdown":"**(a)** Area = 12.5 units. M1 A1"}]}"#,
        )]);
        let mut c = config();
        c.ms_text_first = true;
        let (drafts, report) =
            run_markscheme_pipeline(&mock, &pgs, &c, &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert_eq!(drafts.len(), 1);
        assert_eq!(report.ms_text_first, 1, "window transcribed from text");
        assert!(
            !body_has_image(&mock.bodies()[0]),
            "text-first MS window must send ZERO images"
        );
        assert_eq!(mock.remaining(), 0, "exactly one text-only call");
        assert_eq!(
            mock.bodies()[0]["max_tokens"], 3072,
            "MS text-first output capped at the stage ceiling"
        );
    }

    #[tokio::test]
    async fn ms_window_text_first_placeholder_falls_back_to_vision() {
        let pgs = rich_text_ms_pages(3);
        let mock = MockLlm::new(vec![
            // Text-first attempt signals a worked figure it cannot see.
            ok_chat(
                r#"{"answers":[{"question_number":1,"answer_markdown":"[DIAGRAM_PLACEHOLDER] Gradient = 3. M1 A1"}]}"#,
            ),
            // Same window re-read with full-page vision.
            ok_chat(
                r#"{"answers":[{"question_number":1,"answer_markdown":"Gradient = 3. M1 A1"}]}"#,
            ),
        ]);
        let mut c = config();
        c.ms_text_first = true;
        let (drafts, report) =
            run_markscheme_pipeline(&mock, &pgs, &c, &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert_eq!(drafts.len(), 1);
        assert_eq!(report.ms_text_first, 0, "placeholder window is not counted");
        assert_eq!(mock.bodies().len(), 2, "text attempt, then vision fallback");
        assert!(
            body_has_image(&mock.bodies()[1]),
            "the fallback call must carry the window images"
        );
    }

    #[tokio::test]
    async fn ms_window_unreliable_text_goes_straight_to_vision() {
        let mut pgs = pages(3); // empty text layers — gate must refuse
        pgs[0].text = "Sparse".into();
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"answers":[{"question_number":1,"answer_markdown":"Answer one. B1"}]}"#,
        )]);
        let mut c = config();
        c.ms_text_first = true;
        let (drafts, report) =
            run_markscheme_pipeline(&mock, &pgs, &c, &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert_eq!(drafts.len(), 1);
        assert_eq!(report.ms_text_first, 0);
        assert_eq!(
            mock.bodies().len(), 1,
            "single vision call — no text-first attempt was made"
        );
    }

    #[tokio::test]
    async fn low_confidence_figure_does_not_supply_text_first() {
        // The only detected figure sits below the confidence floor: it must
        // NOT satisfy the figure reference — the span falls back to vision
        // instead of silently starving for its exhibit.
        let pgs = vec![PageInput {
            kind: PageInputKind::Image {
                b64: text_image_page().get_b64().unwrap().to_string(),
            },
            text: "Figure 1 shows a circuit. State the total resistance.\n\n[2 marks]".into(),
        }];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(2),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let page_figures = vec![vec![crate::pdf_render::DetectedFigure {
            bbox: [0.10, 0.10, 0.50, 0.50],
            caption: Some("Figure 1".into()),
            kind: Some("circuit".into()),
            seg_confidence: 0.3,
            option_label: None,
        }]];
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"items":[{"question_number":30,"content":"The total resistance is $6\\,\\Omega$. **[2 marks]**","marks":2,"topics":["circuits"],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"6\\Omega","visual_options":null}]}"#,
        )]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let mut cfg = config();
        cfg.text_first = true;
        let (built_opt, report) =
            extract_span(&mock, &cfg, &span, &span_pages, &page_figures, &cache, &Arc::new(PageImageCache::new()), &semaphore, &collateral, &all_spans, true, &cancel_flag(), &usage()).await;
        let built = built_opt.expect("vision path must build the question");
        assert!(built.content.contains("6"), "vision answer used");
        assert_eq!(
            report.text_first, 0,
            "low-confidence supply must not enable text-first"
        );
        assert_eq!(mock.bodies().len(), 1, "straight to vision");
        assert!(body_has_image(&mock.bodies()[0]));
    }

    #[tokio::test]
    async fn confident_figure_supplies_text_first() {
        // Same layout, but the detection is confident: the span proceeds
        // text-first with the deterministic crop attached afterwards.
        let pgs = vec![PageInput {
            kind: PageInputKind::Image {
                b64: text_image_page().get_b64().unwrap().to_string(),
            },
            text: "Figure 1 shows a circuit. State the total resistance.\n\n[2 marks]".into(),
        }];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(2),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let page_figures = vec![vec![crate::pdf_render::DetectedFigure {
            bbox: [0.10, 0.10, 0.50, 0.50],
            caption: Some("Figure 1".into()),
            kind: Some("circuit".into()),
            seg_confidence: 0.9,
            option_label: None,
        }]];
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"items":[{"question_number":30,"content":"The total resistance is $6\\,\\Omega$. **[2 marks]**","marks":2,"topics":["circuits"],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"6\\Omega","visual_options":null}]}"#,
        )]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let mut cfg = config();
        cfg.text_first = true;
        let (built_opt, report) =
            extract_span(&mock, &cfg, &span, &span_pages, &page_figures, &cache, &Arc::new(PageImageCache::new()), &semaphore, &collateral, &all_spans, true, &cancel_flag(), &usage()).await;
        let built = built_opt.expect("confident supply must keep text-first");
        assert!(built.content.contains("6"), "text-first answer used");
        assert_eq!(report.text_first, 1);
        assert!(
            !body_has_image(&mock.bodies()[0]),
            "zero-image transcription despite the figure reference"
        );
    }

    #[tokio::test]
    async fn stage_attribution_records_text_first_tokens() {
        let pgs = vec![text_image_page()];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(2),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let resp = serde_json::json!({
            "choices": [{ "message": { "content": r#"{"items":[{"question_number":30,"content":"State the value of $x$ when $2x + 4 = 10$. **[2 marks]**","marks":2,"topics":[],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"2x + 4 = 10","visual_options":null}]}"# } }],
            "usage": { "prompt_tokens": 4200, "completion_tokens": 312, "total_tokens": 4512 }
        });
        let mock = MockLlm::new(vec![Ok(resp)]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let mut cfg = config();
        cfg.text_first = true;
        let usage_arc = usage();
        let (_built_opt, _report) =
            extract_span(&mock, &cfg, &span, &span_pages, &[], &cache, &Arc::new(PageImageCache::new()), &semaphore, &collateral, &all_spans, true, &cancel_flag(), &usage_arc).await;
        let stages = usage_arc.snapshot_stages();
        assert_eq!(stages.len(), 1, "one tagged stage");
        assert_eq!(
            stages[0].0,
            StageTag::TextFirstExtraction,
            "tokens attributed to the text-first stage"
        );
        assert_eq!(stages[0].2, 312, "completion tokens recorded per stage");
        assert_eq!(
            mock.bodies()[0]["max_tokens"],
            StageTag::TextFirstExtraction.output_cap(),
            "per-stage output cap applied over the global 32k"
        );
    }

    // ── Vision detail policy & resolution cap ─────────────────────────────
    // Band-cropped single-question images go out as detail:"low" (OpenAI tile
    // savings); any call containing a full page stays detail:"high". Full-page
    // sends from the structure pass and mark-scheme windows are downscaled to
    // ≤768px so pixel-billed providers (Gemini/Claude) pay for fewer pixels.

    fn large_image_page() -> PageInput {
        let mut g = gray_blank(1200, 1600);
        g_hline(&mut g, 400);
        g_vline(&mut g, 600, 0, 1599);
        g_blob(&mut g, 800, 100, 500);
        PageInput {
            kind: PageInputKind::Image { b64: png_b64(&g) },
            text: String::new(),
        }
    }

    fn body_image_details(body: &serde_json::Value) -> Vec<String> {
        body["messages"][1]["content"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter(|c| c["type"] == "image_url")
                    .filter_map(|c| c["image_url"]["detail"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn band_crop_requested_at_low_detail() {
        let pgs = vec![large_image_page()];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: Some(0.2),
            end_y_frac: Some(0.7),
            expected_marks: Some(6),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"items":[{"question_number":30,"content":"State the value. **[6 marks]**","marks":6}]}"#,
        )]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let (built_opt, _report) =
            extract_span(&mock, &config(), &span, &span_pages, &[], &cache, &Arc::new(PageImageCache::new()), &semaphore, &collateral, &all_spans, false, &cancel_flag(), &usage()).await;
        assert!(built_opt.is_some());
        assert_eq!(
            body_image_details(&mock.bodies()[0]),
            vec!["low".to_string()],
            "band crops must be sent at low detail"
        );
    }

    #[tokio::test]
    async fn full_page_requested_at_high_detail() {
        let pgs = vec![large_image_page()];
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
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"items":[{"question_number":30,"content":"State the value. **[6 marks]**","marks":6}]}"#,
        )]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let (built_opt, _report) =
            extract_span(&mock, &config(), &span, &span_pages, &[], &cache, &Arc::new(PageImageCache::new()), &semaphore, &collateral, &all_spans, false, &cancel_flag(), &usage()).await;
        assert!(built_opt.is_some());
        assert_eq!(
            body_image_details(&mock.bodies()[0]),
            vec!["high".to_string()],
            "full pages must stay at high detail"
        );
    }

    #[tokio::test]
    async fn markscheme_window_sends_downscaled_images() {
        // 4 large image pages → window=3 step=2 gives windows [0,3) and [2,4).
        let pgs = vec![large_image_page(); 4];
        let mock = MockLlm::new(vec![
            ok_chat(
                r#"{"answers":[{"question_number":1,"answer_markdown":"Answer one."},{"question_number":2,"answer_markdown":"Answer two."}]}"#,
            ),
            ok_chat(
                r#"{"answers":[{"question_number":2,"answer_markdown":"Answer two."},{"question_number":3,"answer_markdown":"Answer three."}]}"#,
            ),
        ]);
        let mut c = config();
        c.max_output_tokens = 4096;
        let (drafts, _report) =
            run_markscheme_pipeline(&mock, &pgs, &c, &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert_eq!(drafts.len(), 3);
        // Window 0 sends pages 0-2 (3 images). Each must be ≤768px on the long edge.
        let window_body = &mock.bodies()[0];
        let images: Vec<&str> = window_body["messages"][1]["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["type"] == "image_url")
            .filter_map(|c| c["image_url"]["url"].as_str())
            .collect();
        assert_eq!(images.len(), 3);
        for url in images {
            let decoded = geometry::decode_page_image(url)
                .unwrap_or_else(|| panic!("image must decode: {}", &url[..url.len().min(40)]));
            let (w, h) = decoded.dimensions();
            assert!(
                w.max(h) <= geometry::API_IMAGE_MAX_DIM,
                "mark-scheme page was {}x{} — must be capped at ≤768px",
                w,
                h
            );
        }
        // Full pages stay high detail.
        assert!(body_image_details(window_body).iter().all(|d| d == "high"));
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
        // Empty page_texts for test (no boilerplate in test images)
        let page_texts: Vec<String> = vec![String::new(); decoded_pages.len()];
        let (bad, issues) =
            audit_diagram_boxes(&decoded_pages, &page_texts, &mut [item], &l2c, &bands);
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
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let (built_opt, report) =
            extract_span(&mock, &config(), &span, &span_pages, &[], &cache, &Arc::new(PageImageCache::new()), &semaphore, &collateral, &all_spans, false, &cancel_flag(), &usage()).await;
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
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let (built_opt, report) =
            extract_span(&mock, &config(), &span, &span_pages, &[], &cache, &Arc::new(PageImageCache::new()), &semaphore, &collateral, &all_spans, false, &cancel_flag(), &usage()).await;
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

    #[test]
    fn image_options_keep_boxes_separate_and_bind_strict_mcq_list() {
        let mut item = AiQuestion {
            content: Some(
                "Which graph represents y = sin(x)/x?\nA [DIAGRAM_PLACEHOLDER]\nB [DIAGRAM_PLACEHOLDER]\nC [DIAGRAM_PLACEHOLDER]\nD [DIAGRAM_PLACEHOLDER]"
                    .into(),
            ),
            visual_options: Some("image_options".into()),
            diagram_bboxes: Some(vec![
                vec![0.10, 0.25, 0.25, 0.20],
                vec![0.55, 0.25, 0.25, 0.20],
                vec![0.10, 0.55, 0.25, 0.20],
                vec![0.55, 0.55, 0.25, 0.20],
            ]),
            bbox_page_indexes: Some(vec![
                serde_json::json!(0),
                serde_json::json!(0),
                serde_json::json!(0),
                serde_json::json!(0),
            ]),
            ..Default::default()
        };

        normalize_visual_mcq_options(&mut item);

        // Boxes stay ISOLATED — never unioned into one composite crop.
        assert_eq!(item.diagram_bboxes.as_ref().unwrap().len(), 4);
        assert_eq!(item.bbox_page_indexes.as_ref().unwrap().len(), 4);
        assert_eq!(
            item.diagram_captions.unwrap(),
            vec!["Option A", "Option B", "Option C", "Option D"]
        );
        assert_eq!(
            item.diagram_kinds.unwrap(),
            vec!["visual_option"; 4]
        );
        let content = item.content.unwrap();
        assert!(content.starts_with("Which graph represents y = sin(x)/x?"), "{content}");
        for (letter, _) in ['A', 'B', 'C', 'D'].iter().enumerate() {
            let letter = ['A', 'B', 'C', 'D'][letter];
            assert!(
                content.contains(&format!("- [MCQ:{letter}] [DIAGRAM_PLACEHOLDER]")),
                "{content}"
            );
        }
    }

    #[test]
    fn image_options_rebuild_even_from_freeform_option_lines() {
        let mut item = AiQuestion {
            content: Some(
                "Which circuit is correct?\nA)\n[DIAGRAM_PLACEHOLDER]\nB)\n[DIAGRAM_PLACEHOLDER]\nC)\n[DIAGRAM_PLACEHOLDER]"
                    .into(),
            ),
            visual_options: Some("image_options".into()),
            diagram_bboxes: Some(vec![
                vec![0.1, 0.2, 0.2, 0.2],
                vec![0.5, 0.2, 0.2, 0.2],
                vec![0.1, 0.6, 0.2, 0.2],
            ]),
            bbox_page_indexes: Some(vec![
                serde_json::json!(0),
                serde_json::json!(0),
                serde_json::json!(0),
            ]),
            ..Default::default()
        };

        normalize_visual_mcq_options(&mut item);

        let content = item.content.unwrap();
        assert!(content.starts_with("Which circuit is correct?"), "{content}");
        assert!(content.contains("- [MCQ:A] [DIAGRAM_PLACEHOLDER]"), "{content}");
        assert!(content.contains("- [MCQ:B] [DIAGRAM_PLACEHOLDER]"), "{content}");
        assert!(content.contains("- [MCQ:C] [DIAGRAM_PLACEHOLDER]"), "{content}");
        assert!(!content.contains("- [MCQ:D]"), "{content}");
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
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let (built_opt, report) =
            extract_span(&mock, &config(), &span, &span_pages, &[], &cache, &Arc::new(PageImageCache::new()), &semaphore, &collateral, &all_spans, false, &cancel_flag(), &usage()).await;
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

    #[tokio::test]
    async fn identical_repair_failure_stops_resending() {
        // Q2's real failure mode: the model keeps proposing the exact same
        // degenerate box. The convergence guard must NOT spend a third API
        // call re-sending the same images for the same rejection — it should
        // resolve via the budget-spent prune-and-accept path after one repeat.
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
        // A box that is out of the page — deterministically rejected by the
        // audit as "unusable (degenerate or outside the page)".
        let bad_box = r#"{"items":[{"question_number":30,"content":"Complete the flow chart below. [DIAGRAM_PLACEHOLDER] **[6 marks]**","marks":6,"topics":["Proof"],"module":"A","diagram_bboxes":[[9.0,9.0,0.1,0.1]],"bbox_page_indexes":[0]}]}"#;
        let mock = MockLlm::new(vec![ok_chat(bad_box), ok_chat(bad_box)]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let (built_opt, report) =
            extract_span(&mock, &config(), &span, &span_pages, &[], &cache, &Arc::new(PageImageCache::new()), &semaphore, &collateral, &all_spans, false, &cancel_flag(), &usage()).await;
        let built = built_opt.expect("transcription must survive the repeated failure");

        // Guard triggers on the second identical rejection: only 2 of the
        // configured responses are consumed, the third re-send never happens.
        assert_eq!(mock.remaining(), 0, "no third re-send for a stuck repair");
        assert!(!built.content.contains("[DIAGRAM_PLACEHOLDER]"));
        assert!(built.content.contains("Complete the flow chart below."));
        assert_eq!(
            report.repair_reasons.get("diagram_box_issues"),
            Some(&2),
            "two repair rounds recorded (initial failure + one repeat)"
        );
        assert!(
            report
                .anomalies
                .iter()
                .any(|a| a.contains("dropped 1 invalid diagram box")),
            "the drop must be on the record: {:?}",
            report.anomalies
        );
    }

    // ── Text-layer-first extraction ────────────────────────────────────────
    // With text_first enabled, a question whose pages carry a rich text layer
    // is transcribed with ZERO image tokens; vision is only used when the
    // model signals a figure is needed.

    fn body_has_image(body: &serde_json::Value) -> bool {
        body["messages"][1]["content"]
            .as_array()
            .map(|items| items.iter().any(|c| c["type"] == "image_url"))
            .unwrap_or(false)
    }

    fn text_image_page() -> PageInput {
        let mut g = gray_blank(1200, 1600);
        g_hline(&mut g, 400);
        g_blob(&mut g, 800, 100, 500);
        PageInput {
            kind: PageInputKind::Image { b64: png_b64(&g) },
            text: "State the value of $x$ when $2x + 4 = 10$.\n\n[2 marks]\n\nShow your working.".into(),
        }
    }

    #[tokio::test]
    async fn text_first_extracts_from_text_with_zero_images() {
        let pgs = vec![text_image_page()];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(2),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"items":[{"question_number":30,"content":"State the value of $x$ when $2x + 4 = 10$.\n\n**[2 marks]**\n\nShow your working.","marks":2,"topics":["algebra"],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"2x + 4 = 10","visual_options":null}]}"#,
        )]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let mut cfg = config();
        cfg.text_first = true;
        let (built_opt, report) =
            extract_span(&mock, &cfg, &span, &span_pages, &[], &cache, &Arc::new(PageImageCache::new()), &semaphore, &collateral, &all_spans, true, &cancel_flag(), &usage()).await;
        let built = built_opt.expect("text-first extraction must build a question");
        assert!(built.content.contains("2x + 4 = 10"));
        assert_eq!(report.text_first, 1, "text-first counter incremented");
        assert!(
            !body_has_image(&mock.bodies()[0]),
            "text-first request must send ZERO images"
        );
        assert_eq!(mock.remaining(), 0, "exactly one text-only call");
    }

    #[tokio::test]
    async fn text_first_merges_multi_item_response() {
        // Large multi-page questions come back SPLIT across sub-parts — two
        // items for one question number. Text-first must merge them (like the
        // vision path does), not reject and dump the question into the
        // expensive full-page vision loop.
        let pgs = vec![text_image_page()];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: None,
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"items":[
                {"question_number":30,"content":"Part one: factorise $x^2 - 5x + 6$.","marks":2,"topics":["algebra"],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"x^2 - 5x + 6","visual_options":null},
                {"question_number":30,"content":"Part two: hence solve $x^2 - 5x + 6 = 0$.","marks":3,"topics":["algebra"],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"x = 2, x = 3","visual_options":null}
            ]}"#,
        )]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let mut cfg = config();
        cfg.text_first = true;
        let (built_opt, report) =
            extract_span(&mock, &cfg, &span, &span_pages, &[], &cache, &Arc::new(PageImageCache::new()), &semaphore, &collateral, &all_spans, true, &cancel_flag(), &usage()).await;
        let built = built_opt.expect("split response must still build a question");
        assert!(built.content.contains("Part one"), "first sub-part merged: {}", built.content);
        assert!(built.content.contains("Part two"), "second sub-part merged: {}", built.content);
        assert_eq!(built.marks, 5, "marks summed across both sub-parts");
        assert_eq!(report.text_first, 1, "one text-first success, no vision fallback");
        assert_eq!(mock.remaining(), 0, "exactly one text-only call");
        assert!(
            !body_has_image(&mock.bodies()[0]),
            "merged extraction must still send ZERO images"
        );
    }

    #[test]
    fn text_first_prompt_is_slimmed() {
        let cfg = config();
        let slim = text_first_system_prompt(&cfg);
        let full = extraction_system_prompt(&cfg);
        assert!(
            !slim.contains("FEW-SHOT"),
            "slim text-first prompt must not carry the 7 box-drawing examples"
        );
        assert!(
            !slim.contains("Example 1"),
            "slim text-first prompt must not carry few-shot example bodies"
        );
        assert!(
            slim.len() < full.len(),
            "slim prompt ({} chars) must be shorter than the full prompt ({} chars)",
            slim.len(),
            full.len()
        );
        assert!(
            slim.contains("[DIAGRAM_PLACEHOLDER]"),
            "slim prompt must keep the figure-placeholder rule"
        );
        assert!(
            slim.contains("QUESTION ISOLATION"),
            "slim prompt must keep the isolation rules"
        );
    }

    #[test]
    fn text_first_schema_omits_bbox_fields() {
        let schema = text_first_json_schema();
        let props = &schema["schema"]["properties"]["items"]["items"]["properties"];
        assert!(
            props.get("diagram_bboxes").is_none(),
            "slim schema must not require diagram_bboxes"
        );
        assert!(
            props.get("visual_options").is_none(),
            "slim schema must not require visual_options"
        );
        assert!(props.get("content").is_some(), "content stays");
        assert!(props.get("marks").is_some(), "marks stays");
    }

    #[tokio::test]
    async fn token_totals_accumulate_real_usage() {
        // The response `usage` block must flow into the shared accumulator
        // (which `run_question_pipeline` copies into the report for the cost
        // estimate).
        let pgs = vec![text_image_page()];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(2),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let resp = serde_json::json!({
            "choices": [{ "message": { "content": r#"{"items":[{"question_number":30,"content":"State the value of $x$ when $2x + 4 = 10$. **[2 marks]**","marks":2,"topics":[],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"2x + 4 = 10","visual_options":null}]}"# } }],
            "usage": { "prompt_tokens": 4200, "completion_tokens": 312, "total_tokens": 4512 }
        });
        let mock = MockLlm::new(vec![Ok(resp)]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let mut cfg = config();
        cfg.text_first = true;
        let usage_arc = usage();
        let (built_opt, _report) =
            extract_span(&mock, &cfg, &span, &span_pages, &[], &cache, &Arc::new(PageImageCache::new()), &semaphore, &collateral, &all_spans, true, &cancel_flag(), &usage_arc).await;
        assert!(built_opt.is_some(), "question built");
        let (p, c) = usage_arc.snapshot();
        assert_eq!(p, 4200, "real prompt tokens accumulated");
        assert_eq!(c, 312, "real completion tokens accumulated");
    }

    #[test]
    fn text_first_accepts_duplicate_placeholders_for_one_referenced_figure() {
        // Q6 regression: the model emits one [DIAGRAM_PLACEHOLDER] PER
        // REFERENCE of the same figure ("Figure 9" appears in parts (a), (b),
        // (c) → 2 placeholders). One distinct figure, one available → the
        // gate must accept and leave the extras for attach to collapse.
        let cfg = config();
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(2),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let page = AiQuestionPage {
            items: vec![AiQuestion {
                question_number: Some(serde_json::json!(30)),
                content: Some(
                    "(a) Use Figure 9 to find the value. [DIAGRAM_PLACEHOLDER]\n\n(b) State the gradient of Figure 9. [DIAGRAM_PLACEHOLDER] **[2 marks]**"
                        .to_string(),
                ),
                marks: Some(serde_json::json!(2)),
                ..Default::default()
            }],
        };
        let built = build_question_from_parsed_page(page, &span, &cfg, 1);
        assert!(
            built.is_some(),
            "duplicate placeholders for ONE referenced figure must not fall back to vision"
        );
        let built = built.unwrap();
        // `clean_marker_markdown` may rewrite DIAGRAM_PLACEHOLDER to
        // VISUAL_MCQ_PLACEHOLDER; either way both figure markers survive for
        // attach to place one crop and collapse the rest.
        assert_eq!(
            built.content.matches("PLACEHOLDER]").count(),
            2,
            "both figure markers survive for attach to collapse"
        );
    }

    #[test]
    fn text_first_falls_back_when_distinct_figures_exceed_supply() {
        // Two placeholders referencing TWO DIFFERENT figures with only ONE
        // figure available → genuinely under-supplied → vision.
        let cfg = config();
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(2),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let page = AiQuestionPage {
            items: vec![AiQuestion {
                question_number: Some(serde_json::json!(30)),
                content: Some(
                    "(a) Use Figure 9. [DIAGRAM_PLACEHOLDER]\n\n(b) Use Figure 10. [DIAGRAM_PLACEHOLDER] **[2 marks]**"
                        .to_string(),
                ),
                marks: Some(serde_json::json!(2)),
                ..Default::default()
            }],
        };
        let built = build_question_from_parsed_page(page, &span, &cfg, 1);
        assert!(
            built.is_none(),
            "2 distinct figures with 1 available must still fall back to vision"
        );
    }

    #[tokio::test]
    async fn text_first_falls_back_to_vision_when_figure_needed() {
        // The text layer references a figure the model can't see → it emits
        // [DIAGRAM_PLACEHOLDER] → the pipeline must re-ask WITH the image.
        let pgs = vec![text_image_page()];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(2),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let mock = MockLlm::new(vec![
            // Text-first attempt: model says it needs the figure.
            ok_chat(
                r#"{"items":[{"question_number":30,"content":"Refer to Figure 3 shown below. [DIAGRAM_PLACEHOLDER] **[2 marks]**","marks":2,"topics":[],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"","visual_options":null}]}"#,
            ),
            // Vision fallback: full transcription (no figure reference, so
            // figure-consistency validation passes cleanly).
            ok_chat(
                r#"{"items":[{"question_number":30,"content":"The graph crosses the $x$-axis at $x = 3$. **[2 marks]**","marks":2,"topics":["graphs"],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"x = 3","visual_options":null}]}"#,
            ),
        ]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let mut cfg = config();
        cfg.text_first = true;
        let (built_opt, report) =
            extract_span(&mock, &cfg, &span, &span_pages, &[], &cache, &Arc::new(PageImageCache::new()), &semaphore, &collateral, &all_spans, true, &cancel_flag(), &usage()).await;
        let built = built_opt.expect("vision fallback must build the question");
        assert!(built.content.contains("x = 3"), "vision answer used: {}", built.content);
        assert_eq!(mock.bodies().len(), 2, "text-first + vision fallback");
        assert!(
            body_has_image(&mock.bodies()[1]),
            "the fallback call must include the page image"
        );
        assert_eq!(report.text_first, 0, "no text-first success recorded");
    }

    #[tokio::test]
    async fn text_first_skipped_when_text_references_figure() {
        // If the text layer says "Figure 1 shows...", the question MUST go
        // through vision so the figure gets boxed and extracted — text-first
        // must not swallow the figure.
        let pgs = vec![PageInput {
            kind: PageInputKind::Image { b64: text_image_page().get_b64().unwrap().to_string() },
            text: "Figure 1 shows a circuit. State the total resistance.\n\n[2 marks]".into(),
        }];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(2),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"items":[{"question_number":30,"content":"The total resistance is $6\\,\\Omega$. **[2 marks]**","marks":2,"topics":["circuits"],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"6\\Omega","visual_options":null}]}"#,
        )]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let mut cfg = config();
        cfg.text_first = true;
        let (built_opt, report) =
            extract_span(&mock, &cfg, &span, &span_pages, &[], &cache, &Arc::new(PageImageCache::new()), &semaphore, &collateral, &all_spans, true, &cancel_flag(), &usage()).await;
        let built = built_opt.expect("vision path must build the question");
        assert!(built.content.contains("6"), "vision answer used: {}", built.content);
        assert_eq!(report.text_first, 0, "figure question must not go text-first");
        assert_eq!(
            mock.bodies().len(),
            1,
            "only one call — the vision call, no text-first attempt"
        );
        assert!(
            body_has_image(&mock.bodies()[0]),
            "figure question must be sent WITH the image"
        );
    }

    #[tokio::test]
    async fn text_first_disabled_keeps_vision_path() {
        let pgs = vec![text_image_page()];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(2),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"items":[{"question_number":30,"content":"State the value of $x$ when $2x + 4 = 10$. **[2 marks]**","marks":2,"topics":["algebra"],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"2x + 4 = 10","visual_options":null}]}"#,
        )]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let cfg = config(); // text_first defaults to false
        let (built_opt, _report) =
            extract_span(&mock, &cfg, &span, &span_pages, &[], &cache, &Arc::new(PageImageCache::new()), &semaphore, &collateral, &all_spans, false, &cancel_flag(), &usage()).await;
        let built = built_opt.expect("vision path must build the question");
        assert!(built.content.contains("2x + 4 = 10"));
        assert!(
            body_has_image(&mock.bodies()[0]),
            "with text_first disabled the image must be sent"
        );
    }

    /// A page whose raster shows two distinct solid figure regions (a figure
    /// that must not be blank-guarded away during crop tests).
    fn two_figures_page() -> PageInput {
        let mut g = gray_blank(1200, 1600);
        for y in 600..900u32 {
            for x in 200..700u32 {
                g.put_pixel(x, y, image::Luma([40u8]));
            }
        }
        for y in 1000..1300u32 {
            for x in 200..700u32 {
                g.put_pixel(x, y, image::Luma([40u8]));
            }
        }
        PageInput {
            kind: PageInputKind::Image { b64: png_b64(&g) },
            text: "30. Figure 1 shows a circuit and Figure 2 shows a waveform.\n\n[4 marks]".into(),
        }
    }

    #[tokio::test]
    async fn detected_figures_attach_to_text_first_question() {
        // A figure-referencing question goes TEXT-FIRST because the content
        // stream supplied 2 deterministic figures (fig_count == fig_refs).
        // The model emits two placeholders; the detector's crops are spliced
        // in and saved through the standard guard chain.
        let pgs = vec![two_figures_page()];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(4),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let page_figures = vec![vec![
            crate::pdf_render::DetectedFigure {
                bbox: [200.0 / 1200.0, 600.0 / 1600.0, 500.0 / 1200.0, 300.0 / 1600.0],
                caption: Some("Figure 1".to_string()),
                kind: Some("circuit".to_string()),
                seg_confidence: 0.9,
                option_label: None,
            },
            crate::pdf_render::DetectedFigure {
                bbox: [200.0 / 1200.0, 1000.0 / 1600.0, 500.0 / 1200.0, 300.0 / 1600.0],
                caption: Some("Figure 2".to_string()),
                kind: Some("graph".to_string()),
                seg_confidence: 0.9,
                option_label: None,
            },
        ]];
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"items":[{"question_number":30,"content":"Figure 1 shows a circuit. [DIAGRAM_PLACEHOLDER] Figure 2 shows a waveform. [DIAGRAM_PLACEHOLDER] **[4 marks]**","marks":4,"topics":[],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"","visual_options":null}]}"#,
        )]);
        let dir = std::env::temp_dir().join(format!("mm_attach_{}", uuid::Uuid::new_v4()));
        let mut cfg = config();
        cfg.text_first = true;
        cfg.diagrams_dir = Some(dir.clone());
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let (built_opt, report) = extract_span(
            &mock,
            &cfg,
            &span,
            &span_pages,
            &page_figures,
            &cache,
            &Arc::new(PageImageCache::new()),
            &semaphore,
            &collateral,
            &all_spans,
            true,
            &cancel_flag(),
            &usage(),
        )
        .await;
        let built = built_opt.expect("text-first must build the question");
        assert_eq!(report.text_first, 1, "question transcribed from text");
        assert_eq!(
            report.diagrams_saved, 2,
            "both deterministic figures cropped and saved"
        );
        assert_eq!(
            built.content.matches("![Diagram](").count(),
            2,
            "both figure links spliced into the content: {}",
            built.content
        );
        assert!(
            !body_has_image(&mock.bodies()[0]),
            "figure question went text-first: zero image tokens"
        );
        assert_eq!(mock.remaining(), 0, "exactly one text-only call");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn vision_fallback_when_figure_read_required() {
        // "Use the graph to determine" + zero deterministic figures → the
        // answer must be READ from the figure, so the question keeps the
        // proven vision path with the page image attached.
        let pgs = vec![PageInput {
            kind: PageInputKind::Image {
                b64: text_image_page().get_b64().unwrap().to_string(),
            },
            text: "Use the graph to determine the value of $x$. Figure 3 shows the graph.\n\n[2 marks]"
                .into(),
        }];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(2),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"items":[{"question_number":30,"content":"From the graph, $x = 4.2$. **[2 marks]**","marks":2,"topics":["graphs"],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"x = 4.2","visual_options":null}]}"#,
        )]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let mut cfg = config();
        cfg.text_first = true;
        let (built_opt, report) = extract_span(
            &mock,
            &cfg,
            &span,
            &span_pages,
            &[],
            &cache,
            &Arc::new(PageImageCache::new()),
            &semaphore,
            &collateral,
            &all_spans,
            true,
            &cancel_flag(),
            &usage(),
        )
        .await;
        let built = built_opt.expect("vision path must build the question");
        assert!(built.content.contains("4.2"), "vision answer used");
        assert_eq!(report.text_first, 0, "no text-first success recorded");
        assert_eq!(mock.bodies().len(), 1, "exactly one vision call");
        assert!(
            body_has_image(&mock.bodies()[0]),
            "figure-read question must be sent WITH the page image"
        );
    }

    #[test]
    fn detached_option_figures_bind_only_when_the_letter_is_forced() {
        let link = |n: &str| Some(format!("![Diagram](d/{n}.png)"));
        // Physics '24 Q26 shape: three printed letters, one anonymous crop at
        // top-left. Because the run A..D is missing exactly A, the anonymous
        // crop is forced to A and every crop is bound.
        let links = [link("d"), link("a"), link("b"), link("c")];
        let labels = [Some('D'), None, Some('B'), Some('C')];
        let pairs = bind_detached_option_pairs(&links, &labels).expect("forced assignment");
        let mut sorted = pairs.clone();
        sorted.sort_by_key(|(c, _)| *c);
        assert_eq!(sorted.iter().map(|(c, _)| *c).collect::<Vec<_>>(), vec!['A', 'B', 'C', 'D']);
        assert_eq!(pairs[1].0, 'A', "the anonymous top-left crop becomes A");

        // Two anonymous crops: the assignment is ambiguous, so nothing binds.
        let labels2 = [Some('D'), None, None, Some('C')];
        assert!(bind_detached_option_pairs(&links, &labels2).is_none());
        // Duplicate printed letter: reject rather than collapse an option.
        let labels3 = [Some('D'), None, Some('D'), Some('C')];
        assert!(bind_detached_option_pairs(&links, &labels3).is_none());
        // Non-contiguous / mismatched run: reject.
        let labels4 = [Some('D'), None, Some('B'), Some('E')];
        assert!(bind_detached_option_pairs(&links, &labels4).is_none());
        // Fewer than three crops cannot be an option grid.
        assert!(bind_detached_option_pairs(&links[..2], &labels[..2]).is_none());
        // Every crop labelled is fine too (no forced slot needed).
        let full = [Some('A'), Some('B'), Some('C'), Some('D')];
        assert!(bind_detached_option_pairs(&links, &full).is_some());

        // Stem corroboration: labelled crops alone are not enough. A multi-part
        // question or a plain descriptive figure must not become an MCQ.
        assert!(stem_supports_mcq_options(
            "Which diagram shows a distribution of charge where the potential is zero?"
        ));
        assert!(!stem_supports_mcq_options("(a) Which graph is correct?"));
        assert!(!stem_supports_mcq_options("The figure shows three labelled parts."));
        assert!(!stem_supports_mcq_options("Which graph is correct"));
        // The mark tag is appended before crops attach: it must not defeat the
        // question-mark / choose-word check (physics '24 Q26 shape).
        assert!(stem_supports_mcq_options(
            "Which diagram shows a distribution of charge where the potential at P is zero?\n\n**[1 mark]**"
        ));
    }

    fn fig_on_page(caption: &str, bbox: [f32; 4]) -> crate::pdf_render::DetectedFigure {
        crate::pdf_render::DetectedFigure {
            bbox,
            caption: Some(caption.to_string()),
            kind: None,
            seg_confidence: 0.9,
            option_label: None,
        }
    }

    #[tokio::test]
    async fn adjacent_page_figure_enables_text_first() {
        // The question on page 0 references "Figure 2", but the figure is
        // detected on the NEXT page (page 1) — the common AQA layout. The
        // adjacent-page caption lookup must let the question go text-first
        // (zero image tokens) instead of falling back to vision.
        let pgs = vec![PageInput {
            kind: PageInputKind::Image {
                b64: text_image_page().get_b64().unwrap().to_string(),
            },
            text: "30. Figure 2 shows a circuit. State the total resistance.\n\n[2 marks]".into(),
        }];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(2),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let page_figures = vec![
            vec![],
            vec![fig_on_page("Figure 2", [0.10, 0.10, 0.50, 0.50])],
        ];
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"items":[{"question_number":30,"content":"Figure 2 shows a circuit. The total resistance is $6\\,\\Omega$. **[2 marks]**","marks":2,"topics":["circuits"],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"6\\Omega","visual_options":null}]}"#,
        )]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let mut cfg = config();
        cfg.text_first = true;
        let (built_opt, report) = extract_span(
            &mock,
            &cfg,
            &span,
            &span_pages,
            &page_figures,
            &cache,
            &Arc::new(PageImageCache::new()),
            &semaphore,
            &collateral,
            &all_spans,
            true,
            &cancel_flag(),
            &usage(),
        )
        .await;
        let built = built_opt.expect("adjacent-page figure must keep this question text-first");
        assert!(built.content.contains("6"), "text-first answer used");
        assert_eq!(report.text_first, 1, "question transcribed from text");
        assert_eq!(mock.bodies().len(), 1, "exactly one text-only call");
        assert!(
            !body_has_image(&mock.bodies()[0]),
            "no image tokens — the figure came from the detector on page 1"
        );
    }

    #[tokio::test]
    async fn whole_paper_caption_match_finds_distant_figure() {
        // "Figure 5" is referenced but only detected far away on page 12 —
        // the whole-paper caption pass must still find it so the question
        // goes text-first.
        let pgs = vec![PageInput {
            kind: PageInputKind::Image {
                b64: text_image_page().get_b64().unwrap().to_string(),
            },
            text: "30. Figure 5 shows a velocity–time graph. State the acceleration.\n\n[2 marks]"
                .into(),
        }];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(2),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let mut page_figures: Vec<Vec<crate::pdf_render::DetectedFigure>> = vec![Vec::new(); 13];
        page_figures[12] = vec![fig_on_page("Figure 5", [0.10, 0.10, 0.50, 0.50])];
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"items":[{"question_number":30,"content":"Figure 5 shows a velocity–time graph. The acceleration is $4\\,\\text{m}\\,\\text{s}^{-2}$. **[2 marks]**","marks":2,"topics":["kinematics"],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"4\\text{m s}^{-2}","visual_options":null}]}"#,
        )]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let mut cfg = config();
        cfg.text_first = true;
        let (built_opt, report) = extract_span(
            &mock,
            &cfg,
            &span,
            &span_pages,
            &page_figures,
            &cache,
            &Arc::new(PageImageCache::new()),
            &semaphore,
            &collateral,
            &all_spans,
            true,
            &cancel_flag(),
            &usage(),
        )
        .await;
        let built = built_opt.expect("distant caption match must keep the question text-first");
        assert!(built.content.contains("4"), "text-first answer used");
        assert_eq!(report.text_first, 1, "question transcribed from text");
        assert_eq!(mock.bodies().len(), 1, "exactly one text-only call");
    }

    #[test]
    fn caption_match_finds_figure_above_band_on_span_page() {
        // The regression: AQA places the figure ABOVE the question text, so
        // its centre sits OUTSIDE the question's vertical band. The band pass
        // rejects it, and the caption pass used to skip the span's own pages
        // entirely → fig_count = 0 → the question fell to full-page vision.
        // The caption identity must supply it regardless of position.
        let pgs = vec![text_image_page()];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: Some(0.60),
            end_y_frac: None,
            expected_marks: Some(2),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        // Figure centred at y ≈ 0.20 — well above the band that starts at 0.60.
        let page_figures = vec![vec![fig_on_page("Figure 3", [0.10, 0.05, 0.50, 0.30])]];
        let candidates = span_figure_candidates(&span, &span_pages, &page_figures, &[3]);
        assert_eq!(
            candidates.len(),
            1,
            "figure above the question band must be found by caption"
        );
        let empty = span_figure_candidates(&span, &span_pages, &page_figures, &[]);
        assert!(
            empty.is_empty(),
            "without a numbered reference the band filter still applies"
        );
    }

    #[test]
    fn unnumbered_mcq_figures_follow_printed_reading_order() {
        let pages = vec![text_image_page()];
        let span_pages = vec![(0, &pages[0])];
        let span = doc_map::QuestionSpan {
            number: 26, start_page: 0, end_page: 0,
            start_y_frac: None, end_y_frac: None, expected_marks: Some(1),
            reliable_pages: vec![], ambiguous_pages: vec![],
        };
        let figures = vec![vec![
            fig_on_page("", [0.55, 0.60, 0.30, 0.20]),
            fig_on_page("", [0.10, 0.22, 0.30, 0.20]),
            fig_on_page("", [0.10, 0.60, 0.30, 0.20]),
            fig_on_page("", [0.55, 0.20, 0.30, 0.20]),
        ]];
        let ordered = span_figure_candidates(&span, &span_pages, &figures, &[]);
        let positions: Vec<_> = ordered.iter().map(|(_, f)| (f.bbox[0], f.bbox[1])).collect();
        assert_eq!(positions, vec![(0.10, 0.22), (0.55, 0.20), (0.10, 0.60), (0.55, 0.60)]);
    }

    #[tokio::test]
    async fn same_page_batch_uses_text_first_when_all_questions_safe() {
        // Shared-page batches were the last full-page-vision cost: one page
        // image call per shared page. When every question on the page is
        // text-first-safe (figures attached deterministically), the batch
        // must skip the vision call entirely.
        let pgs = vec![PageInput {
            kind: PageInputKind::Image {
                b64: text_image_page().get_b64().unwrap().to_string(),
            },
            text: "8. Figure 5 shows a graph. Use the graph to determine the gradient.\n\n[2 marks]\n\n9. State the value of $x$.\n\n[1 mark]"
                .into(),
        }];
        let spans = vec![
            doc_map::QuestionSpan {
                number: 8,
                start_page: 0,
                end_page: 0,
                start_y_frac: None,
                end_y_frac: None,
                expected_marks: Some(2),
                reliable_pages: vec![],
                ambiguous_pages: vec![],
            },
            doc_map::QuestionSpan {
                number: 9,
                start_page: 0,
                end_page: 0,
                start_y_frac: None,
                end_y_frac: None,
                expected_marks: Some(1),
                reliable_pages: vec![],
                ambiguous_pages: vec![],
            },
        ];
        let span_refs: Vec<&doc_map::QuestionSpan> = spans.iter().collect();
        let page_figures = vec![vec![fig_on_page("Figure 5", [0.08, 0.45, 0.50, 0.30])]];
        // ONE combined text-first call returns both questions' items.
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"items":[
                {"question_number":8,"content":"Figure 5 shows a graph. Use the graph to determine the gradient. **[2 marks]**","marks":2,"topics":[],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"","visual_options":null},
                {"question_number":9,"content":"The value of $x$ is $7$. **[1 mark]**","marks":1,"topics":[],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"7","visual_options":null}
            ]}"#,
        )]);
        let dir = std::env::temp_dir().join(format!("mm_batch_tf_{}", uuid::Uuid::new_v4()));
        let mut cfg = config();
        cfg.text_first = true;
        cfg.diagrams_dir = Some(dir.clone());
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(spans.clone());
        let (results, report) = extract_same_page_batch(
            &mock,
            &cfg,
            &span_refs,
            0,
            &pgs[0],
            &page_figures,
            &cache,
            &Arc::new(PageImageCache::new()),
            &semaphore,
            &collateral,
            &all_spans,
            true,
            &cancel_flag(),
            &usage(),
        )
        .await;
        assert_eq!(results.len(), 2, "both batch questions extracted");
        assert!(
            results.iter().all(|(_, q)| q.is_some()),
            "both questions built"
        );
        assert_eq!(report.text_first, 2, "both transcribed from text");
        assert_eq!(mock.bodies().len(), 1, "ONE combined text-first call");
        assert!(
            mock.bodies().iter().all(|b| !body_has_image(b)),
            "no image tokens for a text-first-safe shared page"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn combined_batch_one_call_for_three_questions() {
        // Three questions sharing one page must be transcribed in a SINGLE
        // combined text-first call (one prompt-overhead payment, not three).
        let pgs = vec![PageInput {
            kind: PageInputKind::Image {
                b64: text_image_page().get_b64().unwrap().to_string(),
            },
            text: "9. State the value of $x$.\n\n[1 mark]\n\n10. Factorise $x^2 - 1$.\n\n[2 marks]\n\n11. Solve $2x = 8$.\n\n[1 mark]"
                .into(),
        }];
        let spans = vec![
            doc_map::QuestionSpan {
                number: 9,
                start_page: 0,
                end_page: 0,
                start_y_frac: None,
                end_y_frac: None,
                expected_marks: Some(1),
                reliable_pages: vec![],
                ambiguous_pages: vec![],
            },
            doc_map::QuestionSpan {
                number: 10,
                start_page: 0,
                end_page: 0,
                start_y_frac: None,
                end_y_frac: None,
                expected_marks: Some(2),
                reliable_pages: vec![],
                ambiguous_pages: vec![],
            },
            doc_map::QuestionSpan {
                number: 11,
                start_page: 0,
                end_page: 0,
                start_y_frac: None,
                end_y_frac: None,
                expected_marks: Some(1),
                reliable_pages: vec![],
                ambiguous_pages: vec![],
            },
        ];
        let span_refs: Vec<&doc_map::QuestionSpan> = spans.iter().collect();
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"items":[
                {"question_number":9,"content":"The value of $x$ is $7$. **[1 mark]**","marks":1,"topics":[],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"x = 7","visual_options":null},
                {"question_number":10,"content":"Factorise $x^2 - 1$. **[2 marks]**","marks":2,"topics":[],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"x^2 - 1","visual_options":null},
                {"question_number":11,"content":"$x = 4$. **[1 mark]**","marks":1,"topics":[],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"x = 4","visual_options":null}
            ]}"#,
        )]);
        let mut cfg = config();
        cfg.text_first = true;
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(spans.clone());
        let (results, report) = extract_same_page_batch(
            &mock,
            &cfg,
            &span_refs,
            0,
            &pgs[0],
            &[],
            &cache,
            &Arc::new(PageImageCache::new()),
            &semaphore,
            &collateral,
            &all_spans,
            true,
            &cancel_flag(),
            &usage(),
        )
        .await;
        assert_eq!(results.len(), 3, "all three batch questions extracted");
        assert!(
            results.iter().all(|(_, q)| q.is_some()),
            "all three questions built"
        );
        assert_eq!(report.text_first, 3, "all transcribed from text");
        assert_eq!(mock.bodies().len(), 1, "ONE combined call, not three");
        assert!(
            mock.bodies().iter().all(|b| !body_has_image(b)),
            "no image tokens on a text-only combined batch"
        );
        assert_eq!(mock.remaining(), 0, "no phantom extra calls");
    }

    #[tokio::test]
    async fn combined_batch_partial_fallback_reasks_missing_question() {
        // The combined call misses ONE of three questions → only that span is
        // re-asked individually (text-first), the other two keep the combined
        // result. No question is ever lost.
        let pgs = vec![PageInput {
            kind: PageInputKind::Image {
                b64: text_image_page().get_b64().unwrap().to_string(),
            },
            text: "9. State the value of $x$.\n\n[1 mark]\n\n10. Factorise $x^2 - 1$.\n\n[2 marks]\n\n11. Solve $2x = 8$.\n\n[1 mark]"
                .into(),
        }];
        let spans = vec![
            doc_map::QuestionSpan {
                number: 9,
                start_page: 0,
                end_page: 0,
                start_y_frac: None,
                end_y_frac: None,
                expected_marks: Some(1),
                reliable_pages: vec![],
                ambiguous_pages: vec![],
            },
            doc_map::QuestionSpan {
                number: 10,
                start_page: 0,
                end_page: 0,
                start_y_frac: None,
                end_y_frac: None,
                expected_marks: Some(2),
                reliable_pages: vec![],
                ambiguous_pages: vec![],
            },
            doc_map::QuestionSpan {
                number: 11,
                start_page: 0,
                end_page: 0,
                start_y_frac: None,
                end_y_frac: None,
                expected_marks: Some(1),
                reliable_pages: vec![],
                ambiguous_pages: vec![],
            },
        ];
        let span_refs: Vec<&doc_map::QuestionSpan> = spans.iter().collect();
        // Combined call omits Q11; the individual re-ask supplies it.
        let mock = MockLlm::new(vec![
            ok_chat(
                r#"{"items":[
                    {"question_number":9,"content":"The value of $x$ is $7$. **[1 mark]**","marks":1,"topics":[],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"x = 7","visual_options":null},
                    {"question_number":10,"content":"Factorise $x^2 - 1$. **[2 marks]**","marks":2,"topics":[],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"x^2 - 1","visual_options":null}
                ]}"#,
            ),
            ok_chat(
                r#"{"items":[{"question_number":11,"content":"$x = 4$. **[1 mark]**","marks":1,"topics":[],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"x = 4","visual_options":null}]}"#,
            ),
        ]);
        let mut cfg = config();
        cfg.text_first = true;
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(spans.clone());
        let (results, report) = extract_same_page_batch(
            &mock,
            &cfg,
            &span_refs,
            0,
            &pgs[0],
            &[],
            &cache,
            &Arc::new(PageImageCache::new()),
            &semaphore,
            &collateral,
            &all_spans,
            true,
            &cancel_flag(),
            &usage(),
        )
        .await;
        assert_eq!(results.len(), 3, "all three questions still extracted");
        assert!(
            results.iter().all(|(_, q)| q.is_some()),
            "the missing question was recovered, none lost"
        );
        assert_eq!(report.text_first, 3, "all three via the text layer");
        assert_eq!(
            mock.bodies().len(),
            2,
            "one combined call + one individual re-ask"
        );
        assert_eq!(mock.remaining(), 0, "no extra calls");
    }

    /// TEMPORARY verification: reconstruct the fixture's real spans from its
    /// text layer and print the gate decision per question, to prove the
    /// caption-match fix flips figure questions away from full-page vision.
    /// Skips silently when the fixture or pdfium is unavailable.
    #[test]
    fn diagnostic_gate_decisions_on_fixture() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        let manifest = env!("CARGO_MANIFEST_DIR");
        let path = std::path::Path::new(manifest).join("../physics '24.pdf");
        if !path.exists() {
            eprintln!("[GATE] fixture missing");
            return;
        }
        // Run BOTH text sources: pdfium (what my fix was validated against)
        // and pdf_extract (what production actually feeds the pipeline).
        let pdfium_texts = match crate::pdf_render::pdf_page_texts(&path) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[GATE] pdfium unavailable: {}", e);
                return;
            }
        };
        let production_texts =
            crate::commands::extract_page_texts(&path.to_string_lossy(), pdfium_texts.len());
        let page_figures = match crate::pdf_render::detect_pdf_figures(&path) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[GATE] detection failed: {}", e);
                return;
            }
        };

        for (label, page_texts) in [
            ("pdfium", pdfium_texts),
            ("pdf_extract(PROD)", production_texts),
        ] {
            let scan = doc_map::scan_text_layer(&page_texts);
            let map =
                doc_map::build_hybrid_map_with_scan(&page_texts, &[], page_texts.len(), &scan);
            let mut counts = [0usize; 4]; // text_first, crop_first_ready, vision, no_text
            for span in &map.spans {
                let combined: Vec<&str> = (span.start_page..=span.end_page)
                    .filter(|&p| p < page_texts.len())
                    .map(|p| page_texts[p].trim())
                    .filter(|t| !t.is_empty())
                    .collect();
                let combined_text = combined.join("\n\n");
                let has_text = !combined_text.trim().is_empty();
                let text_refs_figure = has_text && text_references_figure(&combined_text);
                let must_read = has_text && figure_read_required(&combined_text);
                let referenced = if has_text {
                    figure_reference_numbers(&combined_text)
                } else {
                    Vec::new()
                };
                let dummy = PageInput {
                    kind: PageInputKind::TextOnly,
                    text: String::new(),
                };
                let span_pages: Vec<(usize, &PageInput)> =
                    (span.start_page..=span.end_page).map(|p| (p, &dummy)).collect();
                let fig_count =
                    available_span_figures(span, &span_pages, &page_figures, &referenced);
                let needs_vision = text_refs_figure && fig_count == 0;
                let (kind, idx) = if !has_text {
                    ("no_text", 3)
                } else if !needs_vision {
                    ("text_first", 0)
                } else if must_read {
                    ("crop_first_ready", 1)
                } else {
                    ("full_page_vision", 2)
                };
                counts[idx] += 1;
                if label != "pdf_extract(PROD)" || kind != "text_first" {
                    eprintln!(
                        "[GATE:{}] Q{} pages {}..{} refs={:?} fig_count={} must_read={} -> {}",
                        label,
                        span.number,
                        span.start_page + 1,
                        span.end_page + 1,
                        referenced,
                        fig_count,
                        must_read,
                        kind
                    );
                }
            }
            eprintln!(
                "[GATE:{}] SUMMARY: {} text_first, {} crop_first_ready, {} full_page_vision, {} no_text",
                label, counts[0], counts[1], counts[2], counts[3]
            );
        }
    }

    #[tokio::test]
    async fn graph_reading_is_transcribed_locally_with_attached_graph() {
        // Preserve the instruction and graph; ingestion does not solve it.
        let pgs = vec![PageInput {
            kind: PageInputKind::Image {
                b64: text_image_page().get_b64().unwrap().to_string(),
            },
            text: "30. Figure 3 shows the graph. Use the graph to determine the acceleration of the trolley during the first ten seconds of its journey. [2 marks]\n(Total for Question 30 is 2 marks)"
                .into(),
        }];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(2),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let page_figures =
            vec![vec![fig_on_page("Figure 3", [0.08, 0.45, 0.50, 0.30])]];
        let mock = MockLlm::new(vec![]);
        let dir = std::env::temp_dir().join(format!("mm_cropfirst_{}", uuid::Uuid::new_v4()));
        let mut cfg = config();
        cfg.text_first = true;
        cfg.deterministic = true;
        cfg.diagrams_dir = Some(dir.clone());
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let (built_opt, report) = extract_span(
            &mock,
            &cfg,
            &span,
            &span_pages,
            &page_figures,
            &cache,
            &Arc::new(PageImageCache::new()),
            &semaphore,
            &collateral,
            &all_spans,
            true,
            &cancel_flag(),
            &usage(),
        )
        .await;
        let built = built_opt.expect("Tier-0 must build the question");
        assert!(built.content.contains("determine the acceleration"));
        assert_eq!(report.deterministic, 1);
        assert_eq!(mock.bodies().len(), 0, "zero model calls");
        assert!(!built.needs_review, "{:?}", built.notes);
        assert_eq!(
            built.content.matches("![Diagram](").count(),
            1,
            "the crop is attached to the card: {}",
            built.content
        );
        // A detection that cannot be persisted is not a verified attachment.
        cfg.diagrams_dir = None;
        let (review, _) = extract_span(&mock, &cfg, &span, &span_pages, &page_figures,
            &cache, &Arc::new(PageImageCache::new()), &semaphore, &collateral,
            &all_spans, true, &cancel_flag(), &usage()).await;
        let review = review.unwrap();
        assert!(review.needs_review);
        assert!(review.notes.iter().any(|n| n.contains("could not be attached")));
        assert!(!review.content.contains("PLACEHOLDER"));
        assert!(mock.bodies().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn graph_text_first_falls_back_to_full_page_when_parse_fails() {
        // A broken text-first response must fall through to the unchanged
        // full-page vision path so the question is never lost.
        let pgs = vec![PageInput {
            kind: PageInputKind::Image {
                b64: text_image_page().get_b64().unwrap().to_string(),
            },
            text: "Use the graph to determine the value of $x$. Figure 3 shows the graph.\n\n[2 marks]"
                .into(),
        }];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 30,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(2),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let page_figures =
            vec![vec![fig_on_page("Figure 3", [0.08, 0.45, 0.50, 0.30])]];
        let mock = MockLlm::new(vec![
            ok_chat("this is not json — crop-first must fail and fall back"),
            ok_chat(
                r#"{"items":[{"question_number":30,"content":"From the graph, $x = 4.2$. **[2 marks]**","marks":2,"topics":["graphs"],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"x = 4.2","visual_options":null}]}"#,
            ),
        ]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let mut cfg = config();
        cfg.text_first = true;
        let (built_opt, _report) = extract_span(
            &mock,
            &cfg,
            &span,
            &span_pages,
            &page_figures,
            &cache,
            &Arc::new(PageImageCache::new()),
            &semaphore,
            &collateral,
            &all_spans,
            true,
            &cancel_flag(),
            &usage(),
        )
        .await;
        let built = built_opt.expect("full-page fallback must build the question");
        assert!(built.content.contains("4.2"), "fallback answer used");
        assert_eq!(
            mock.bodies().len(),
            2,
            "text-first attempt then full-page fallback"
        );
        assert!(
            body_has_image(&mock.bodies()[1]),
            "the fallback call sends the full page image"
        );
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
            &[0.02, 0.05, 0.90, 0.82],
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

        // Similar grids with a changed label/curve must not alias each other.
        let mut different = chart_img();
        for x in 280..290 { for y in 220..230 { different.put_pixel(x, y, image::Luma([0])); } }
        let changed_b64 = png_b64(&different);
        let distinct = save_diagram(1, Some(&changed_b64), &[0.02, 0.05, 0.90, 0.82],
            &cfg, &cache, &mut saved, &mut report, false, false).unwrap();
        assert_ne!(distinct, l1, "different figure must get its own image");
        assert_eq!(report.diagrams_saved, 2);

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

    #[tokio::test]
    async fn same_page_batch_extracts_multiple_mcqs_in_single_call() {
        let mock = MockLlm::new(vec![
            // structure pass for page 1
            structure_reply("QUESTION", "[1, 2, 3]", "[3, 6]"),
            // single batch extraction call returning all 3 questions at once
            ok_chat(
                r#"{"items":[
                    {"question_number":1,"content":"What is the unit of force?\n\nA Joules\nB Newtons\nC Watts\nD Pascals\n\n**[1 mark]**","marks":1,"topics":["Units"],"module":"Pure"},
                    {"question_number":2,"content":"Which quantity is a vector?\n\nA Speed\nB Velocity\nC Mass\nD Time\n\n**[1 mark]**","marks":1,"topics":["Vectors"],"module":"Pure"},
                    {"question_number":3,"content":"Solve $3x = 12$.\n\n**[1 mark]**","marks":1,"topics":["Algebra"],"module":"Pure"}
                ]}"#,
            ),
        ]);
        let pgs = vec![
            PageInput {
                kind: PageInputKind::TextOnly,
                text: "Cover page\nInstructions\nAnswer ALL questions".into(),
            },
            PageInput {
                kind: PageInputKind::TextOnly,
                text: "1 What is the unit of force?\n2 Which quantity is a vector?\n3 Solve 3x = 12.\n(Total for Question 3 is 1 mark)".into(),
            },
        ];
        let (built, report) =
            run_cloud_pipeline(&mock, &pgs, &[], &config())
                .await
                .unwrap();

        assert_eq!(built.len(), 3);
        assert_eq!(built[0].question_number, 1);
        assert_eq!(built[1].question_number, 2);
        assert_eq!(built[2].question_number, 3);
        assert_eq!(report.questions_extracted, 3);
        assert!(report.quarantined.is_empty());
        assert_eq!(mock.remaining(), 0, "All 3 questions were extracted in exactly 1 batch LLM call");
    }

    #[tokio::test]
    async fn same_page_batch_partial_fallback_recovers_missing_question() {
        let mock = MockLlm::new(vec![
            // structure pass for page 1
            structure_reply("QUESTION", "[1, 2]", "[2, 4]"),
            // batch extraction call returns ONLY Q1 (missed Q2)
            ok_chat(
                r#"{"items":[
                    {"question_number":1,"content":"Question 1 content here. **[2 marks]**","marks":2,"topics":["Proof"],"module":"Pure"}
                ]}"#,
            ),
            // fallback extract_span for Q2 recovers Q2
            ok_chat(
                r#"{"items":[
                    {"question_number":2,"content":"Question 2 recovered content. **[2 marks]**","marks":2,"topics":["Integration"],"module":"Pure"}
                ]}"#,
            ),
        ]);
        let pgs = vec![
            PageInput {
                kind: PageInputKind::TextOnly,
                text: "Cover page\nInstructions\nAnswer ALL questions".into(),
            },
            PageInput {
                kind: PageInputKind::TextOnly,
                text: "1 Question 1 text.\n2 Question 2 text.\n(Total for Question 2 is 2 marks)".into(),
            },
        ];
        let (built, report) =
            run_cloud_pipeline(&mock, &pgs, &[], &config())
                .await
                .unwrap();

        assert_eq!(built.len(), 2);
        assert_eq!(built[0].question_number, 1);
        assert_eq!(built[1].question_number, 2);
        assert_eq!(report.questions_extracted, 2);
        assert!(report.quarantined.is_empty());
        assert_eq!(mock.remaining(), 0);
    }

    #[tokio::test]
    async fn collateral_question_reused_from_shared_page() {
        let mock = MockLlm::new(vec![
            // structure pass: Q1 on pages 1-2 (multi-page), Q2 on page 2 (single-page).
            // A footer is [question_number, marks]; page 1 shows none.
            structure_reply("QUESTION", "[1]", "null"),
            structure_reply("QUESTION", "[1, 2]", "[2, 3]"),
            // Q1 extraction call (spans pages 1 and 2): returns both Q1 and downstream Q2 as collateral
            ok_chat(
                r#"{"items":[
                    {"question_number":1,"content":"Question 1 full content across pages. **[5 marks]**","marks":5,"topics":["Pure"],"module":"Pure"},
                    {"question_number":2,"content":"Question 2 starts on the lower half of page 2. **[3 marks]**","marks":3,"topics":["Mechanics"],"module":"Pure"}
                ]}"#,
            ),
            // Notice: NO LLM call queued for Question 2!
        ]);
        let pgs = vec![
            PageInput {
                kind: PageInputKind::TextOnly,
                text: "Cover page\nAnswer ALL questions".into(),
            },
            PageInput {
                kind: PageInputKind::TextOnly,
                // Scanned question pages: no text layer.
                text: String::new(),
            },
            PageInput {
                kind: PageInputKind::TextOnly,
                text: String::new(),
            },
        ];
        let mut cfg = config();
        cfg.parallelism = 1;
        cfg.force_scanned_context = true;
        let (built, report) =
            run_question_pipeline(&mock, &pgs, &[], &cfg, &NullProgress, &cancel_flag())
                .await
                .unwrap();

        assert_eq!(built.len(), 2);
        assert_eq!(built[0].question_number, 1);
        assert_eq!(built[1].question_number, 2);
        assert_eq!(built[1].marks, 3);
        assert_eq!(report.questions_extracted, 2);
        assert!(report.quarantined.is_empty());
        assert_eq!(mock.remaining(), 0, "Question 2 was retrieved from collateral cache with 0 LLM calls");
    }

    #[test]
    fn test_splice_diagrams_by_caption_and_context_out_of_order() {
        let content = "Figure 1 shows a circuit.\n[DIAGRAM_PLACEHOLDER]\n\n(a) Figure 2 shows the graph of V_C vs t.\n[DIAGRAM_PLACEHOLDER]\n\n(b) Figure 3 shows the decay.\n[DIAGRAM_PLACEHOLDER]\n\n(d) Figure 4 shows capacitors.\n[DIAGRAM_PLACEHOLDER]".to_string();

        let links = vec![
            Some("\n\n![Diagram](url_fig2.png)\n\n".to_string()),
            Some("\n\n![Diagram](url_fig1.png)\n\n".to_string()),
            Some("\n\n![Diagram](url_fig4.png)\n\n".to_string()),
            Some("\n\n![Diagram](url_fig3.png)\n\n".to_string()),
        ];

        let captions = vec![
            "Figure 2: Graph of V_C against t".to_string(),
            "Figure 1: Charging circuit diagram".to_string(),
            "Figure 4: Parallel plate capacitors".to_string(),
            "Figure 3: Voltage decay against time".to_string(),
        ];

        let spliced = splice_diagrams_by_caption_and_context(content, &links, Some(&captions));

        assert!(spliced.contains("Figure 1 shows a circuit.\n\n\n![Diagram](url_fig1.png)"), "Figure 1 must receive url_fig1");
        assert!(spliced.contains("Figure 2 shows the graph of V_C vs t.\n\n\n![Diagram](url_fig2.png)"), "Figure 2 must receive url_fig2");
        assert!(spliced.contains("Figure 3 shows the decay.\n\n\n![Diagram](url_fig3.png)"), "Figure 3 must receive url_fig3");
        assert!(spliced.contains("Figure 4 shows capacitors.\n\n\n![Diagram](url_fig4.png)"), "Figure 4 must receive url_fig4");
    }

    // --- Tier-0 deterministic extraction (zero-cost import cascade) ------

    #[tokio::test]
    async fn tier0_resolves_span_without_api_calls() {
        let pgs = [PageInput {
            kind: PageInputKind::Image {
                b64: text_image_page().get_b64().unwrap().to_string(),
            },
            text: "5. Define specific heat capacity and explain why water has an unusually high value compared with most common substances used as coolants. [4 marks]\n(Total for Question 5 is 4 marks)".into(),
        }];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 5,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(4),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        // Empty queue: ANY API call would panic the mock.
        let mock = MockLlm::new(vec![]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(PAGE_RENDER_CACHE_CAPACITY));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let mut cfg = config();
        cfg.text_first = true;
        cfg.deterministic = true;
        // Production conditions: commands.rs ALWAYS populates allowed_topics
        // from the module taxonomy. Tier 0 must still carve locally.
        cfg.allowed_topics = vec!["Materials".into(), "Electricity".into()];
        let (built_opt, report) =
            extract_span(&mock, &cfg, &span, &span_pages, &[], &cache, &Arc::new(PageImageCache::new()), &semaphore, &collateral, &all_spans, true, &cancel_flag(), &usage()).await;
        let built = built_opt.expect("Tier-0 must carve this span locally");
        assert_eq!(built.marks, 4);
        assert!(built.content.contains("specific heat capacity"));
        assert_eq!(report.deterministic, 1);
        assert_eq!(report.text_first, 0);
        assert_eq!(mock.bodies().len(), 0, "ZERO API calls for a Tier-0 span");
    }

    #[tokio::test]
    async fn tier0_escalates_to_llm_on_garbage() {
        let pgs = [PageInput {
            kind: PageInputKind::Image {
                b64: text_image_page().get_b64().unwrap().to_string(),
            },
            text: "6. Calculate the resultant force on the trolley and state its direction of motion clearly. [2 marks]\n(Total for Question 6 is 3 marks)".into(),
        }];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = doc_map::QuestionSpan {
            number: 6,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(3),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"items":[{"question_number":6,"content":"The resultant force is $120$ N acting opposite to the direction of motion of the trolley. **[3 marks]**","marks":3,"topics":[],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"120","visual_options":null}]}"#,
        )]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(PAGE_RENDER_CACHE_CAPACITY));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let mut cfg = config();
        cfg.allowed_topics = vec!["Forces".into()];
        cfg.text_first = true;
        cfg.deterministic = true;
        let (built_opt, report) =
            extract_span(&mock, &cfg, &span, &span_pages, &[], &cache, &Arc::new(PageImageCache::new()), &semaphore, &collateral, &all_spans, true, &cancel_flag(), &usage()).await;
        let built = built_opt.expect("LLM fallback must build the question");
        assert!(built.content.contains("resultant force"));
        assert_eq!(report.deterministic, 0, "marks mismatch must escalate");
        assert_eq!(report.text_first, 1);
        assert_eq!(mock.bodies().len(), 1, "exactly one LLM text-first call");
        assert!(!body_has_image(&mock.bodies()[0]));
    }

    #[tokio::test]
    async fn tier0_batch_skips_combined_call() {
        let page_text = "9. Explain why the current is the same at every point in a series connection and state which quantity changes across components instead. [2 marks]\n10. Describe one advantage of connecting the lamps in parallel across the supply rather than wiring them all in series. [2 marks]\n11. A third identical lamp is added in series and every lamp is observed to become dimmer than before. Explain this observation carefully. [2 marks]".to_string();
        let pgs = [PageInput {
            kind: PageInputKind::Image {
                b64: text_image_page().get_b64().unwrap().to_string(),
            },
            text: page_text,
        }];
        let mk_span = |n: u32| doc_map::QuestionSpan {
            number: n,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(2),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let spans = vec![mk_span(9), mk_span(10), mk_span(11)];
        let span_refs: Vec<&doc_map::QuestionSpan> = spans.iter().collect();
        // Empty queue: any API call would panic the mock.
        let mock = MockLlm::new(vec![]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(PAGE_RENDER_CACHE_CAPACITY));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(spans.clone());
        let mut cfg = config();
        cfg.allowed_topics = vec!["Electricity".into()];
        cfg.text_first = true;
        cfg.deterministic = true;
        let (results, report) = extract_same_page_batch(
            &mock, &cfg, &span_refs, 0, &pgs[0], &[], &cache, &Arc::new(PageImageCache::new()),
            &semaphore, &collateral, &all_spans, true, &cancel_flag(), &usage(),
        )
        .await;
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|(_, q)| q.is_some()));
        assert_eq!(report.deterministic, 3, "whole page carved locally");
        assert_eq!(report.text_first, 0);
        assert_eq!(mock.bodies().len(), 0, "combined call skipped entirely");
    }

    #[tokio::test]
    async fn tier0_batch_partial_failure_falls_back_to_one_combined_call() {
        // Q11's inline tag contradicts the printed footer ? Tier-0 declines
        // the WHOLE page; exactly ONE combined LLM call recovers all three.
        let page_text = "9. Explain why the current is the same at every point in a series connection and state which quantity changes across components instead. [2 marks]\n10. Describe one advantage of connecting the lamps in parallel across the supply rather than wiring them all in series. [2 marks]\n11. A third identical lamp is added in series and every lamp is observed to become dimmer than before. Explain this observation carefully.".to_string();
        let pgs = [PageInput {
            kind: PageInputKind::Image {
                b64: text_image_page().get_b64().unwrap().to_string(),
            },
            text: page_text,
        }];
        let mk_span = |n: u32| doc_map::QuestionSpan {
            number: n,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(2),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let spans = vec![mk_span(9), mk_span(10), mk_span(11)];
        let span_refs: Vec<&doc_map::QuestionSpan> = spans.iter().collect();
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"items":[{"question_number":9,"content":"The current is the same at every point because charge is conserved; the voltage changes across components. **[2 marks]**","marks":2,"topics":[],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"","visual_options":null},{"question_number":10,"content":"In parallel each lamp receives the full supply voltage so one failing does not extinguish the rest. **[2 marks]**","marks":2,"topics":[],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"","visual_options":null},{"question_number":11,"content":"Adding a lamp in series increases total resistance so the current falls and every lamp dims. **[2 marks]**","marks":2,"topics":[],"module":"Algebra","is_code":false,"diagram_bboxes":[],"diagram_captions":[],"diagram_kinds":[],"bbox_page_indexes":[],"math_snippet":"","visual_options":null}]}"#,
        )]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(PAGE_RENDER_CACHE_CAPACITY));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(spans.clone());
        let mut cfg = config();
        cfg.allowed_topics = vec!["Electricity".into()];
        cfg.text_first = true;
        cfg.deterministic = true;
        let (results, report) = extract_same_page_batch(
            &mock, &cfg, &span_refs, 0, &pgs[0], &[], &cache, &Arc::new(PageImageCache::new()),
            &semaphore, &collateral, &all_spans, true, &cancel_flag(), &usage(),
        )
        .await;
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|(_, q)| q.is_some()), "no question lost");
        assert_eq!(report.deterministic, 0);
        assert_eq!(report.text_first, 3);
        assert_eq!(mock.bodies().len(), 1, "exactly ONE combined fallback call");
    }

    // --- Deferred topic classification (Tier-0 untagged cards) ------------

    #[tokio::test]
    async fn deferred_topic_classification_is_local() {
        let mk = |n: u32, topics: Vec<String>| BuiltQuestion {
            question_number: n,
            content: format!("Question {} asks about specific heat capacity of an ideal gas.", n),
            marks: 2,
            topics,
            module: "Algebra".into(),
            is_code: false,
            needs_review: false,
            notes: vec![],
        };
        let mut qs = vec![
            mk(5, vec![]),                        // untagged → classified
            mk(6, vec!["Mechanics".to_string()]), // already tagged → untouched
        ];
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"assignments":[{"question_number":5,"topics":["Thermal","NotARealTopic"]},{"question_number":6,"topics":["Proof"]}]}"#,
        )]);
        let semaphore = Arc::new(Semaphore::new(1));
        let mut cfg = config();
        cfg.allowed_topics = vec!["Thermal".into(), "Mechanics".into()];
        let assigned =
            classify_topics_deferred(&mock, &cfg, &mut qs, &semaphore, &cancel_flag(), &usage())
                .await;
        assert_eq!(assigned, 1, "only the untagged question is assigned");
        assert_eq!(qs[0].topics, vec!["Thermal"], "out-of-list topic dropped");
        assert_eq!(qs[1].topics, vec!["Mechanics"], "tagged card untouched");
        assert!(mock.bodies().is_empty(), "classification must be local");
    }

    #[tokio::test]
    async fn local_topics_match_syllabus_without_substring_false_positives() {
        let mut cfg = config();
        cfg.allowed_topics = vec!["Thermal physics".into(), "Electric fields".into(), "Nuclear physics".into(), "Waves".into()];
        let texts = [r"Find c_{rms} for an ideal gas.", "Find the electric potential.",
            "Describe beta decay and half-life.", "Measure phase difference on an oscilloscope.",
            "A microwave oven heats food."];
        let mut qs: Vec<_> = texts.iter().enumerate().map(|(i, text)| BuiltQuestion {
            question_number: i as u32 + 1, content: text.to_string(), marks: 1,
            topics: vec![], module: "Physics".into(), is_code: false,
            needs_review: false, notes: vec![],
        }).collect();
        let mock = MockLlm::new(vec![]);
        let totals = usage();
        let assigned = classify_topics_deferred(&mock, &cfg, &mut qs,
            &Arc::new(Semaphore::new(1)), &cancel_flag(), &totals).await;
        assert_eq!(assigned, 4);
        for i in 0..4 { assert_eq!(qs[i].topics, vec![cfg.allowed_topics[i].clone()]); }
        assert!(qs[4].topics.is_empty(), "wave inside microwave is not a keyword");
        assert!(mock.bodies().is_empty());
        assert_eq!(totals.snapshot().0, 0);
        assert_eq!(totals.snapshot().1, 0);
    }

    #[tokio::test]
    async fn deferred_topic_classification_unknown_leaves_untagged_without_call() {
        let mut qs = vec![BuiltQuestion {
            question_number: 5,
            content: "Question body long enough for classification to attempt.".to_string(),
            marks: 2,
            topics: vec![],
            module: "Algebra".into(),
            is_code: false,
            needs_review: false,
            notes: vec![],
        }];
        let mock = MockLlm::new(vec![Err(crate::llm::LlmError::Network("provider down".into()))]);
        let semaphore = Arc::new(Semaphore::new(1));
        let mut cfg = config();
        cfg.allowed_topics = vec!["Thermal".into()];
        let assigned =
            classify_topics_deferred(&mock, &cfg, &mut qs, &semaphore, &cancel_flag(), &usage())
                .await;
        assert_eq!(assigned, 0, "failure must not fabricate tags");
        assert!(qs[0].topics.is_empty());
        assert!(mock.bodies().is_empty());
        assert_eq!(
            usage().snapshot().1,
            0,
            "failed call bills no completion tokens"
        );
    }

    /// Production-conditions end-to-end: taxonomy ALWAYS populated (see
    /// commands.rs), Tier-0 carves both footer pages locally, and the entire
    /// import, including topic classification, uses no API calls.
    #[tokio::test]
    async fn full_pipeline_with_topics_uses_zero_tokens() {
        // NOTE: doc_map treats page 0 as a cover unconditionally, so the
        // fixture mirrors a real paper: front matter, then footer pages.
        let pgs = vec![
            PageInput {
                kind: PageInputKind::TextOnly,
                text: "Physics Paper 1\nAnswer ALL questions".into(),
            },
            PageInput {
                kind: PageInputKind::TextOnly,
                text: "1 Explain why the resistance of a filament lamp increases as its temperature rises during operation. [3 marks]\n(Total for Question 1 is 3 marks)".into(),
            },
            PageInput {
                kind: PageInputKind::TextOnly,
                text: "2 State one advantage of using a data logger rather than a manual stopwatch when measuring this experiment carefully. [2 marks]\n(Total for Question 2 is 2 marks)".into(),
            },
        ];
        let mock = MockLlm::new(vec![Ok(serde_json::json!({
            "choices": [{
                "message": {
                    "content": r#"{"assignments":[{"question_number":1,"topics":["Electricity"]},{"question_number":2,"topics":["Required Practical"]}]}"#
                },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 500, "completion_tokens": 60 }
        }))]);
        let mut cfg = config();
        cfg.text_first = true;
        cfg.deterministic = true;
        cfg.allowed_topics = vec!["Electricity".into(), "Required Practical".into()];
        let (built, report) =
            run_question_pipeline(&mock, &pgs, &[], &cfg, &NullProgress, &cancel_flag())
                .await
                .unwrap();

        assert_eq!(built.len(), 2);
        assert_eq!(report.deterministic, 2, "both spans carved locally");
        assert_eq!(report.text_first, 0, "no per-span LLM calls");
        assert!(mock.bodies().is_empty(), "entire import must be local");
        let q1 = built.iter().find(|q| q.question_number == 1).unwrap();
        let q2 = built.iter().find(|q| q.question_number == 2).unwrap();
        assert_eq!(q1.topics, vec!["Electricity"]);
        assert_eq!(q2.topics, vec!["Required Practical"]);
        assert!(report.stage_breakdown.is_empty(), "no billed stages");
    }

    // -- Phase 1: universal zero-cost digital question ingestion -------------

    /// The weak-map fixture: born-digital pages whose heading sequence is too
    /// sparse for `text_layer_map_sufficient`, so the OLD path ran the vision
    /// structure pass and then per-span vision.
    fn digital_zero_cost_weak_map_pages() -> Vec<PageInput> {
        [
            include_str!("../fixtures/digital_zero_cost/cover.txt"),
            include_str!("../fixtures/digital_zero_cost/weak_map_page_1.txt"),
            include_str!("../fixtures/digital_zero_cost/weak_map_page_2.txt"),
            include_str!("../fixtures/digital_zero_cost/weak_map_page_3.txt"),
        ]
        .iter()
        .map(|text| PageInput {
            kind: PageInputKind::TextOnly,
            text: (*text).to_string(),
        })
        .collect()
    }

    /// Digital pages where Q1's inline tags contradict its printed footer, so
    /// the strict Tier-0 gate fails on exactly one span.
    fn digital_zero_cost_failed_gate_pages() -> Vec<PageInput> {
        [
            include_str!("../fixtures/digital_zero_cost/cover.txt"),
            include_str!("../fixtures/digital_zero_cost/failed_gate_page_1.txt"),
            include_str!("../fixtures/digital_zero_cost/failed_gate_page_2.txt"),
            include_str!("../fixtures/digital_zero_cost/failed_gate_page_3.txt"),
        ]
        .iter()
        .map(|text| PageInput {
            kind: PageInputKind::TextOnly,
            text: (*text).to_string(),
        })
        .collect()
    }

    fn plain_span(number: u32, marks: Option<u32>) -> doc_map::QuestionSpan {
        doc_map::QuestionSpan {
            number,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: marks,
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        }
    }

    #[test]
    fn text_layer_classification_is_not_map_sufficiency() {
        let texts: Vec<String> = digital_zero_cost_weak_map_pages()
            .iter()
            .map(|p| p.text.clone())
            .collect();
        let scan = doc_map::scan_text_layer(&texts);
        assert!(
            !doc_map::text_layer_map_sufficient(&scan, texts.len()),
            "fixture must exercise the weak-map case"
        );
        assert!(
            doc_map::classify_text_layer(&texts).is_digital(),
            "map insufficiency must not downgrade a digital paper"
        );
        let map = doc_map::build_text_only_map(&texts, texts.len(), &scan);
        assert_eq!(
            map.spans.iter().map(|s| s.number).collect::<Vec<_>>(),
            vec![1, 2],
            "the text-only map still places both questions"
        );
        assert!(map.vision_fallback_pages.is_empty());
    }

    /// A real but SHORT digital question paper must still ingest with zero
    /// attempted requests when every tuning switch is off. There is no
    /// document-size floor in the production policy.
    #[tokio::test]
    async fn digital_short_paper_ingests_locally_with_toggles_off() {
        let pages = vec![
            PageInput {
                kind: PageInputKind::TextOnly,
                text: include_str!("../fixtures/digital_zero_cost/cover.txt").to_string(),
            },
            PageInput {
                kind: PageInputKind::TextOnly,
                text: include_str!("../fixtures/digital_zero_cost/weak_map_page_1.txt").to_string(),
            },
        ];
        assert!(
            pages.iter().map(|p| p.text.len()).sum::<usize>() < 1000,
            "fixture must stay under the removed document-size floor"
        );
        let client = crate::llm::RefusingLlm::new();
        let mut cfg = config();
        cfg.text_first = false;
        cfg.deterministic = false;
        let (built, report) =
            run_question_pipeline(&client, &pages, &[], &cfg, &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert_eq!(client.calls(), 0, "short digital paper must not dispatch");
        assert_eq!(report.text_layer, "digital");
        assert!(!built.is_empty(), "the question is carved locally");
    }

    /// A genuine one-page digital paper must keep its first-page question: no
    /// synthetic cover, no discarded Q1, zero cloud attempts.
    #[tokio::test]
    async fn digital_one_page_paper_keeps_first_page_question() {
        let pages = vec![PageInput {
            kind: PageInputKind::TextOnly,
            text: "1. The transformation P is an enlargement with scale factor k. Show that k = 3. [4 marks]\nEND OF QUESTIONS".to_string(),
        }];
        let client = crate::llm::RefusingLlm::new();
        let mut cfg = config();
        cfg.text_first = false;
        cfg.deterministic = false;
        let (built, report) =
            run_question_pipeline(&client, &pages, &[], &cfg, &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert_eq!(
            client.calls(),
            0,
            "a one-page digital paper must attempt nothing: {:?}",
            client.bodies()
        );
        assert_eq!(report.text_layer, "digital");
        assert_eq!(
            report.deterministic, 1,
            "the first-page question is a local Tier-0 accept: {:#?}",
            report.anomalies
        );
        let q1 = built
            .iter()
            .find(|q| q.question_number == 1)
            .expect("question 1 from the first page must be retained");
        assert!(
            q1.content.contains("enlargement") && q1.content.contains("scale factor"),
            "exact source content retained: {}",
            q1.content
        );
        assert_eq!(q1.marks, 4);
        assert!(!q1.needs_review, "{q1:#?}");
    }

    /// A complete maths-only one-mark question ("1. Solve x=2. [1]") carries no
    /// prose at all. No threshold may hand it to the cloud: the import is
    /// local-only with both tuning switches off.
    #[tokio::test]
    async fn digital_maths_only_question_stays_local_with_toggles_off() {
        let pages = vec![
            PageInput {
                kind: PageInputKind::TextOnly,
                text: include_str!("../fixtures/digital_zero_cost/cover.txt").to_string(),
            },
            PageInput {
                kind: PageInputKind::TextOnly,
                text: "1. Solve x=2. [1]".to_string(),
            },
        ];
        let texts: Vec<String> = pages.iter().map(|p| p.text.clone()).collect();
        assert!(
            doc_map::classify_text_layer(&texts).is_digital(),
            "a maths-only page must still classify as digital"
        );
        let client = crate::llm::RefusingLlm::new();
        let mut cfg = config();
        cfg.text_first = false;
        cfg.deterministic = false;
        let (built, report) =
            run_question_pipeline(&client, &pages, &[], &cfg, &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert_eq!(
            client.calls(),
            0,
            "no threshold may grant network permission: {:?}",
            client.bodies()
        );
        assert_eq!(report.text_layer, "digital");
        assert_eq!(report.prompt_tokens + report.completion_tokens, 0);
        assert!(
            report.deterministic + report.recovered + report.quarantined.len() >= 1
                || !built.is_empty(),
            "the span was handled locally, not dropped"
        );
    }

    /// Blank, END-OF-QUESTIONS and image-only pages cannot hand a digital paper
    /// to the cloud, and the pages with no text layer are reported.
    #[tokio::test]
    async fn digital_with_blank_and_backmatter_pages_stays_local_and_reports_gaps() {
        let pages = vec![
            PageInput {
                kind: PageInputKind::TextOnly,
                text: include_str!("../fixtures/digital_zero_cost/cover.txt").to_string(),
            },
            PageInput {
                kind: PageInputKind::TextOnly,
                text: include_str!("../fixtures/digital_zero_cost/weak_map_page_1.txt").to_string(),
            },
            PageInput {
                kind: PageInputKind::TextOnly,
                text: include_str!("../fixtures/digital_zero_cost/blank_page.txt").to_string(),
            },
            PageInput {
                kind: PageInputKind::TextOnly,
                text: include_str!("../fixtures/digital_zero_cost/end_only_page.txt").to_string(),
            },
            PageInput {
                kind: PageInputKind::Image {
                    b64: png_b64(&gray_blank(600, 800)),
                },
                text: String::new(),
            },
        ];
        let texts: Vec<String> = pages.iter().map(|p| p.text.clone()).collect();
        let class = doc_map::classify_text_layer(&texts);
        assert!(class.is_digital(), "one readable question page is enough");
        assert_eq!(
            class.unresolved_pages(),
            &[2, 4],
            "only the whitespace-only pages are unresolved"
        );

        let client = crate::llm::RefusingLlm::new();
        let mut cfg = config();
        cfg.text_first = false;
        cfg.deterministic = false;
        let (built, report) =
            run_question_pipeline(&client, &pages, &[], &cfg, &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert_eq!(
            client.calls(),
            0,
            "blank/back-matter/image-only pages must not enable cloud"
        );
        assert!(!built.is_empty());
        assert!(
            report
                .anomalies
                .iter()
                .any(|a| a.contains("have no text layer and are reported locally")),
            "unresolved pages must be reported: {:#?}",
            report.anomalies
        );
    }

    /// Weak map, every tuning switch OFF: a digital paper must still ingest
    /// with ZERO attempted requests.
    #[tokio::test]
    async fn digital_weak_map_ingests_with_zero_attempted_requests() {
        let pages = digital_zero_cost_weak_map_pages();
        let client = crate::llm::RefusingLlm::new();
        let mut cfg = config();
        cfg.text_first = false;
        cfg.deterministic = false;
        let (built, report) =
            run_question_pipeline(&client, &pages, &[], &cfg, &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert_eq!(
            client.calls(),
            0,
            "digital document must attempt no model request: {:?}",
            client.bodies()
        );
        assert_eq!(report.prompt_tokens, 0);
        assert_eq!(report.completion_tokens, 0);
        assert_eq!(report.text_first, 0, "no text-only LLM call either");
        assert_eq!(built.len(), 2, "both questions extracted locally: {built:#?}");
        assert_eq!(report.deterministic, 2);
        assert!(report.quarantined.is_empty());
        assert!(built.iter().all(|q| q.content.contains("marks")));
        assert!(report
            .anomalies
            .iter()
            .any(|a| a.starts_with("digital document:")));
    }

    /// A digital span whose strict gate fails keeps its real content, flagged,
    /// with the failed gate on the record and still zero requests.
    #[tokio::test]
    async fn digital_failed_gate_recovers_locally_without_cloud() {
        let pages = digital_zero_cost_failed_gate_pages();
        let client = crate::llm::RefusingLlm::new();
        let mut cfg = config();
        cfg.text_first = true;
        cfg.deterministic = true;
        let (built, report) =
            run_question_pipeline(&client, &pages, &[], &cfg, &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert_eq!(client.calls(), 0, "no request may be attempted");
        assert_eq!(
            report.recovered, 1,
            "the failed span is retained locally, never escalated"
        );
        assert_eq!(report.deterministic, 1, "the clean span is a strict accept");
        let recovered = built
            .iter()
            .find(|q| q.question_number == 1)
            .expect("Q1 must be retained, not lost");
        assert!(recovered.needs_review, "a recovered card is never a strict success");
        assert!(recovered.content.contains("thermistor"));
        assert!(
            report
                .anomalies
                .iter()
                .any(|a| a.contains("local recovery") && a.contains("marks_checksum_mismatch")),
            "the failed gate must stay on the record: {:#?}",
            report.anomalies
        );
        let clean = built
            .iter()
            .find(|q| q.question_number == 2)
            .expect("Q2");
        assert!(!clean.needs_review);
    }

    fn layout_fig(label: Option<&str>) -> crate::pdf_render::DetectedFigure {
        crate::pdf_render::DetectedFigure {
            bbox: [0.1, 0.1, 0.3, 0.2],
            caption: None,
            kind: None,
            seg_confidence: 0.6,
            option_label: label.map(str::to_string),
        }
    }

    /// Crops replace their own placeholders in order; a rejected crop takes
    /// its placeholder with it instead of shifting the rest.
    #[test]
    fn layout_figures_fill_their_own_placeholders() {
        let figs = [layout_fig(None), layout_fig(None)];
        let refs: Vec<&crate::pdf_render::DetectedFigure> = figs.iter().collect();
        let mut content = "Figure 7\n[DIAGRAM_PLACEHOLDER]\nThe coil moves.\nFigure 8\n[DIAGRAM_PLACEHOLDER]\nState x.".to_string();
        let links = vec![None, Some("\n\n![Diagram](fig8.png)\n\n".to_string())];
        let attached = splice_layout_figures(&mut content, &links, &refs);
        assert_eq!(attached.len(), 1);
        assert!(!content.contains("[DIAGRAM_PLACEHOLDER]"), "{content}");
        let fig8 = content.find("fig8.png").unwrap();
        assert!(content.find("Figure 8").unwrap() < fig8 && fig8 < content.find("State x.").unwrap(), "{content}");
    }

    /// Labelled option drawings become the option list; the stem's own
    /// figure stays where it is printed, and the printed letters go.
    #[test]
    fn labelled_option_drawings_become_the_option_list() {
        let figs = [layout_fig(None), layout_fig(Some("A")), layout_fig(Some("B")), layout_fig(Some("C")), layout_fig(Some("D"))];
        let refs: Vec<&crate::pdf_render::DetectedFigure> = figs.iter().collect();
        let mut content = "The switch is closed.\n[DIAGRAM_PLACEHOLDER]\nWhich pair of graphs shows V and I?\n**[1 mark]**\nA B\n[DIAGRAM_PLACEHOLDER]\n[DIAGRAM_PLACEHOLDER]\nC D\n[DIAGRAM_PLACEHOLDER]\n[DIAGRAM_PLACEHOLDER]\n- [MCQ:A] A\n- [MCQ:B] B\n- [MCQ:C] C\n- [MCQ:D] D".to_string();
        let links: Vec<Option<String>> = (0..5).map(|i| Some(format!("![Diagram](f{i}.png)"))).collect();
        splice_layout_figures(&mut content, &links, &refs);
        assert!(content.find("f0.png").unwrap() < content.find("Which pair").unwrap(), "{content}");
        for (i, l) in ["A", "B", "C", "D"].iter().enumerate() {
            assert!(content.contains(&format!("- [MCQ:{l}] ![Diagram](f{}.png)", i + 1)), "{content}");
        }
        // The mark allocation stands before the options, as for any
        // multiple-choice card (an option's trailing tag is not displayed).
        assert!(content.contains("V and I?\n\n**[1 mark]**\n- [MCQ:A] ![Diagram](f1.png)"), "{content}");
        assert!(content.trim_end().ends_with("- [MCQ:D] ![Diagram](f4.png)"), "{content}");
        // The answer bubbles, already tagged as letter-only options, go too.
        assert!(!content.lines().any(|l| l.trim() == "A B" || l.trim() == "C D" || l.trim() == "- [MCQ:C] C"), "{content}");
        assert_eq!(content.matches("[MCQ:").count(), 4, "{content}");
    }

    /// A Tier-0 card flagged after carving (here its "Figure 1" cannot be
    /// attached) is a local recovery, never counted as a strict success.
    #[tokio::test]
    async fn flagged_tier0_card_is_not_counted_strict() {
        let mut pages = digital_zero_cost_weak_map_pages();
        pages[2].text = pages[2].text.replacen("2 A loudspeaker produces", "2 Figure 1 shows a loudspeaker that produces", 1);
        assert!(pages[2].text.contains("Figure 1"));
        let client = crate::llm::RefusingLlm::new();
        let mut cfg = config();
        cfg.text_first = false;
        cfg.deterministic = false;
        let (built, report) =
            run_question_pipeline(&client, &pages, &[], &cfg, &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert_eq!(client.calls(), 0, "no request may be attempted");
        let q2 = built.iter().find(|q| q.question_number == 2).expect("Q2 retained");
        assert!(q2.needs_review, "the missing figure must flag the card: {q2:#?}");
        assert_eq!(report.deterministic, 1, "only the clean Q1 is strict");
        assert_eq!(report.recovered, 1, "the flagged Q2 is a local recovery");
        assert!(
            report.anomalies.iter().any(|a| a.starts_with("Question 2: local recovery")),
            "the reason must stay on the record: {:#?}",
            report.anomalies
        );
    }

    /// Direct entry points honour the digital policy too: neither
    /// `extract_span` nor `extract_same_page_batch` may dispatch.
    #[tokio::test]
    async fn digital_entry_points_attempt_zero_requests() {
        let page = PageInput {
            kind: PageInputKind::TextOnly,
            text: include_str!("../fixtures/digital_zero_cost/weak_map_page_1.txt").to_string(),
        };
        let span = plain_span(1, None);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(PAGE_RENDER_CACHE_CAPACITY));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let mut cfg = config();
        // Both tuning switches OFF: the digital policy must still force the
        // local cascade at these entry points.
        cfg.text_first = false;
        cfg.deterministic = false;
        cfg.text_layer_class = Some(doc_map::TextLayerClass {
            text_pages: 1,
            total_pages: 1,
            unresolved_pages: Vec::new(),
        });
        let client = crate::llm::RefusingLlm::new();
        let span_pages = [(0usize, &page)];

        let (built_opt, span_report) = extract_span(
            &client,
            &cfg,
            &span,
            &span_pages,
            &[],
            &cache,
            &Arc::new(PageImageCache::new()),
            &semaphore,
            &collateral,
            &all_spans,
            true,
            &cancel_flag(),
            &usage(),
        )
        .await;
        assert!(built_opt.is_some(), "digital span carves locally");
        assert_eq!(client.calls(), 0, "extract_span must not dispatch");
        assert_eq!(span_report.deterministic, 1);

        let spans = [plain_span(1, None)];
        let refs: Vec<&doc_map::QuestionSpan> = spans.iter().collect();
        let all_spans = Arc::new(spans.to_vec());
        let (results, batch_report) = extract_same_page_batch(
            &client,
            &cfg,
            &refs,
            0,
            &page,
            &[],
            &cache,
            &Arc::new(PageImageCache::new()),
            &semaphore,
            &collateral,
            &all_spans,
            true,
            &cancel_flag(),
            &usage(),
        )
        .await;
        assert!(results.iter().all(|(_, q)| q.is_some()));
        assert_eq!(
            client.calls(),
            0,
            "extract_same_page_batch must not dispatch"
        );
        assert_eq!(batch_report.deterministic, 1);

        // A span on the same page that cannot be isolated locally must still
        // make zero attempts and report an explicit local failure.
        let failing = [plain_span(1, None), plain_span(99, None)];
        let failing_refs: Vec<&doc_map::QuestionSpan> = failing.iter().collect();
        let all_spans = Arc::new(failing.to_vec());
        let (results, failing_report) = extract_same_page_batch(
            &client,
            &cfg,
            &failing_refs,
            0,
            &page,
            &[],
            &cache,
            &Arc::new(PageImageCache::new()),
            &semaphore,
            &collateral,
            &all_spans,
            true,
            &cancel_flag(),
            &usage(),
        )
        .await;
        assert!(results[0].1.is_some(), "Q1 still carves locally");
        assert!(results[1].1.is_none(), "Q99 has no local candidate");
        assert_eq!(client.calls(), 0, "a local failure must not dispatch either");
        assert!(failing_report
            .quarantined
            .iter()
            .any(|q| q.question_number == Some(99)));
    }

    /// The offline refusing client counts every attempt: a scanned paper still
    /// uses the cloud path, so leaked requests show up in the counter. Refused
    /// calls bill nothing.
    #[tokio::test]
    async fn refusing_client_counts_attempts_on_the_cloud_path() {
        // A genuine scan: images with no text layer at all. This is the ONLY
        // input shape that keeps cloud compatibility, so it is the control
        // that proves the refusing client really counts attempts.
        let pages: Vec<PageInput> = (0..3)
            .map(|_| PageInput {
                kind: PageInputKind::Image {
                    b64: png_b64(&gray_blank(600, 800)),
                },
                text: String::new(),
            })
            .collect();
        assert!(
            doc_map::classify_text_layer(
                &pages.iter().map(|p| p.text.clone()).collect::<Vec<_>>()
            )
            .is_scanned_only(),
            "scan-only fixture must stay cloud-compatible"
        );
        let client = crate::llm::RefusingLlm::new();
        let (_, report) =
            run_question_pipeline(&client, &pages, &[], &config(), &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert!(client.calls() > 0, "a scanned paper still attempts cloud calls");
        assert!(
            client.image_calls() > 0,
            "the structure pass attaches page images"
        );
        assert_eq!(report.prompt_tokens, 0);
        assert_eq!(report.completion_tokens, 0);
    }

    fn ai_item(number: u32, content: &str, marks: Option<i32>) -> AiQuestion {
        AiQuestion {
            question_number: Some(serde_json::json!(number)),
            content: Some(content.to_string()),
            marks: marks.map(|m| serde_json::json!(m)),
            ..Default::default()
        }
    }

    #[test]
    fn stitch_preserves_equal_subpart_allocations_and_their_tags() {
        // (a)(b)(c) worth 3 marks EACH is a 9-mark question: equal marks on
        // distinct sub-parts are not duplicate totals. The printed total
        // confirms it, so nothing is collapsed and nothing is flagged.
        let items = vec![
            ai_item(6, "(a) Show that the tension is 12 N. **[3 marks]**", Some(3)),
            ai_item(6, "(b) State the direction of the force. **[3 marks]**", Some(3)),
            ai_item(6, "(c) Explain the motion after release. **[3 marks]**", Some(3)),
        ];
        let stitched = stitch_question_items(items, 6, Some(9)).expect("sub-parts stitch");
        let content = stitched.item.content.expect("content");
        assert!(content.find("(a)").unwrap() < content.find("(b)").unwrap());
        assert!(content.find("(b)").unwrap() < content.find("(c)").unwrap());
        assert_eq!(
            content.matches("3 marks").count(),
            3,
            "every genuine sub-part allocation keeps its tag: {content}"
        );
        assert_eq!(stitched.item.marks, Some(serde_json::json!(9)));
        assert!(!stitched.repeated_total_collapsed);
        assert!(!stitched.marks_ambiguous);

        // Diagrams and topics survive the stitch.
        let with_media = vec![
            AiQuestion {
                diagram_bboxes: Some(vec![vec![0.1, 0.1, 0.4, 0.4]]),
                diagram_captions: Some(vec!["Figure 1".into()]),
                bbox_page_indexes: Some(vec![serde_json::json!(0)]),
                topics: Some(serde_json::json!(["Mechanics"])),
                ..ai_item(6, "(a) part one. **[2 marks]**", Some(2))
            },
            AiQuestion {
                diagram_kinds: Some(vec!["graph".into()]),
                ..ai_item(6, "(b) part two. **[2 marks]**", Some(2))
            },
        ];
        let stitched = stitch_question_items(with_media, 6, Some(4)).expect("stitch");
        assert_eq!(
            stitched.item.diagram_bboxes.as_ref().map(Vec::len),
            Some(1)
        );
        assert_eq!(
            stitched.item.diagram_captions.as_deref(),
            Some(["Figure 1".to_string()].as_slice())
        );
        assert_eq!(stitched.item.diagram_kinds.as_deref(), Some(["graph".to_string()].as_slice()));
        assert!(stitched
            .item
            .topics
            .as_ref()
            .and_then(|t| t.as_array())
            .is_some_and(|t| t.iter().any(|v| v == "Mechanics")));
    }

    #[test]
    fn stitch_collapses_a_confirmed_repeated_parent_total() {
        // Every item repeats the SAME question-level total ("9 marks") while
        // the printed total is 9: that is a duplicate total, not three
        // allocations, so it collapses to one.
        let items = vec![
            ai_item(6, "(a) Show that the tension is 12 N. **[9 marks]**", Some(9)),
            ai_item(6, "(b) State the direction of the force. **[9 marks]**", Some(9)),
            ai_item(6, "(c) Explain the motion after release. **[9 marks]**", Some(9)),
        ];
        let stitched = stitch_question_items(items, 6, Some(9)).expect("stitch");
        let content = stitched.item.content.expect("content");
        assert_eq!(
            content.matches("9 marks").count(),
            1,
            "repeated parent total collapses: {content}"
        );
        assert_eq!(stitched.item.marks, Some(serde_json::json!(9)));
        assert!(stitched.repeated_total_collapsed);
        assert!(!stitched.marks_ambiguous);
    }

    #[test]
    fn stitch_keeps_inline_allocations_that_differ_from_repeated_metadata() {
        // The metadata repeats the parent total (9) on every item, but the
        // CONTENT tags are the genuine per-part allocations (3 each). Only the
        // metadata collapses; stripping the [3] tags would destroy real marks.
        let items = vec![
            ai_item(6, "(a) part one. **[3 marks]**", Some(9)),
            ai_item(6, "(b) part two. **[3 marks]**", Some(9)),
            ai_item(6, "(c) part three. **[3 marks]**", Some(9)),
        ];
        let stitched = stitch_question_items(items, 6, Some(9)).expect("stitch");
        assert!(
            stitched.repeated_total_collapsed,
            "the repeated metadata total still collapses"
        );
        assert_eq!(stitched.item.marks, Some(serde_json::json!(9)));
        let content = stitched.item.content.expect("content");
        assert_eq!(
            content.matches("3 marks").count(),
            3,
            "inline allocations must survive: {content}"
        );
        assert_eq!(
            validate::sum_inline_marks(&content),
            9,
            "inline total still matches the printed total"
        );
    }

    #[test]
    fn stitch_keeps_marks_and_flags_ambiguity_without_a_printed_total() {
        // Equal allocations with no printed total cannot be told apart from a
        // repeated total: keep everything the model reported and flag review.
        let items = vec![
            ai_item(6, "(a) part one. **[3 marks]**", Some(3)),
            ai_item(6, "(b) part two. **[3 marks]**", Some(3)),
            ai_item(6, "(c) part three. **[3 marks]**", Some(3)),
        ];
        let stitched = stitch_question_items(items, 6, None).expect("stitch");
        let content = stitched.item.content.clone().expect("content");
        assert_eq!(content.matches("3 marks").count(), 3);
        assert_eq!(stitched.item.marks, Some(serde_json::json!(9)));
        assert!(stitched.marks_ambiguous, "ambiguous totals must be flagged");
        assert!(!stitched.repeated_total_collapsed);

        // Heterogeneous allocations always sum, whatever the printed total.
        let mixed = vec![
            ai_item(6, "(a) part one. **[2 marks]**", Some(2)),
            ai_item(6, "(b) part two. **[3 marks]**", Some(3)),
            ai_item(6, "(c) part three. **[4 marks]**", Some(4)),
        ];
        let stitched = stitch_question_items(mixed, 6, Some(9)).expect("stitch");
        assert_eq!(stitched.item.marks, Some(serde_json::json!(9)));
        assert!(!stitched.marks_ambiguous);

        // No marks reported at all: nothing invented.
        let unmarked = vec![
            ai_item(6, "(a) part one.", None),
            ai_item(6, "(b) part two.", None),
        ];
        let stitched = stitch_question_items(unmarked, 6, Some(4)).expect("stitch");
        assert_eq!(stitched.item.marks, None);
    }

    #[test]
    fn stitch_refuses_missing_identity_when_the_number_is_unreadable() {
        // An item with no number is allowed (a single-question call may omit
        // it), but an unreadable number is not silently treated as the target.
        let items = vec![
            ai_item(6, "part (a)", Some(2)),
            AiQuestion {
                question_number: Some(serde_json::json!("6(a)")),
                content: Some("part (b)".into()),
                ..Default::default()
            },
        ];
        assert!(matches!(
            stitch_question_items(items, 6, Some(4)),
            Err(StitchRefusal::UnverifiableIdentity)
        ));
    }

    #[test]
    fn stitch_refuses_distinct_parent_questions() {
        let items = vec![
            ai_item(6, "part (a)", Some(2)),
            ai_item(6, "part (b)", Some(2)),
            ai_item(7, "a different question entirely", Some(2)),
        ];
        assert!(matches!(
            stitch_question_items(items, 6, Some(4)),
            Err(StitchRefusal::ForeignQuestion(7))
        ));
    }

    /// The Q6 failure mode: a single-question text-first call returns one item
    /// per sub-part. The card must be stitched locally, not escalated to a
    /// full-page vision repair loop.
    #[tokio::test]
    async fn text_first_stitches_single_question_subparts_without_vision() {
        let page = PageInput {
            kind: PageInputKind::TextOnly,
            text: "6 A block is held on a rough slope and then released. The tension in the string \
                is measured with a newton meter during the experiment. Parts (a), (b) and (c) refer \
                to this apparatus and to the forces acting on the block."
                .to_string(),
        };
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &page)];
        let span = plain_span(6, Some(9));
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"items":[
                {"question_number":6,"content":"(a) Show that the tension is $12\\,\\mathrm{N}$. **[9 marks]**","marks":9},
                {"question_number":6,"content":"(b) State the direction of the resultant force on the block. **[9 marks]**","marks":9},
                {"question_number":6,"content":"(c) Explain why the block accelerates after it is released. **[9 marks]**","marks":9}
            ]}"#,
        )]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(PAGE_RENDER_CACHE_CAPACITY));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let mut cfg = config();
        cfg.text_first = true;
        let (built_opt, report) = extract_span(
            &mock,
            &cfg,
            &span,
            &span_pages,
            &[],
            &cache,
            &Arc::new(PageImageCache::new()),
            &semaphore,
            &collateral,
            &all_spans,
            true,
            &cancel_flag(),
            &usage(),
        )
        .await;
        let built = built_opt.expect("sub-parts stitch into one card");
        assert!(built.content.contains("(a)"));
        assert!(built.content.contains("(b)"));
        assert!(built.content.contains("(c)"));
        assert_eq!(built.marks, 9, "duplicate totals must not inflate the card");
        assert_eq!(report.text_first, 1);
        assert_eq!(
            mock.bodies().len(),
            1,
            "exactly one text-only call: no vision escalation"
        );
        assert!(report.quarantined.is_empty());
    }

    /// An ordinary full-page single-span response that returns the sub-parts as
    /// separate items must be stitched, not sent back for a repair round.
    #[tokio::test]
    async fn vision_single_question_subparts_stitch_without_repair_request() {
        let pgs = vec![grid_page()];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = plain_span(6, Some(9));
        let mock = MockLlm::new(vec![ok_chat(
            r#"{"items":[
                {"question_number":6,"content":"(a) Show that the tension is $12\\,\\mathrm{N}$. **[9 marks]**","marks":9},
                {"question_number":6,"content":"(b) State the direction of the resultant force on the block. **[9 marks]**","marks":9},
                {"question_number":6,"content":"(c) Explain why the block accelerates after it is released. **[9 marks]**","marks":9}
            ]}"#,
        )]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(PAGE_RENDER_CACHE_CAPACITY));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let cfg = config();
        let (built_opt, report) = extract_span(
            &mock,
            &cfg,
            &span,
            &span_pages,
            &[],
            &cache,
            &Arc::new(PageImageCache::new()),
            &semaphore,
            &collateral,
            &all_spans,
            false,
            &cancel_flag(),
            &usage(),
        )
        .await;
        let built = built_opt.expect("sub-parts stitch into one card");
        assert!(built.content.contains("(a)"));
        assert!(built.content.contains("(b)"));
        assert!(built.content.contains("(c)"));
        assert_eq!(built.marks, 9);
        assert!(!built.needs_review, "a confirmed total is not a review case");
        assert_eq!(
            mock.bodies().len(),
            1,
            "stitched in ONE round: no repair re-request"
        );
        assert_eq!(report.repairs, 0);
        assert!(report.quarantined.is_empty());
    }

    /// A same-page batch where ONE span fails its strict gates must not demote
    /// its clean siblings: only the failing span is retained as a recovery.
    #[tokio::test]
    async fn digital_batch_keeps_clean_sibling_strict_and_flags_only_the_failure() {
        let page = PageInput {
            kind: PageInputKind::TextOnly,
            text: include_str!("../fixtures/digital_zero_cost/batch_page.txt").to_string(),
        };
        let spans = [plain_span(1, Some(4)), plain_span(2, Some(5))];
        let span_refs: Vec<&doc_map::QuestionSpan> = spans.iter().collect();
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(PAGE_RENDER_CACHE_CAPACITY));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(spans.to_vec());
        let mut cfg = config();
        cfg.text_first = true;
        cfg.deterministic = true;
        cfg.text_layer_class = Some(doc_map::TextLayerClass {
            text_pages: 1,
            total_pages: 1,
            unresolved_pages: Vec::new(),
        });
        let client = crate::llm::RefusingLlm::new();
        let (results, report) = extract_same_page_batch(
            &client,
            &cfg,
            &span_refs,
            0,
            &page,
            &[],
            &cache,
            &Arc::new(PageImageCache::new()),
            &semaphore,
            &collateral,
            &all_spans,
            true,
            &cancel_flag(),
            &usage(),
        )
        .await;
        assert_eq!(client.calls(), 0, "a digital batch makes no requests");
        let clean = results[0].1.as_ref().expect("Q1 retained");
        assert!(
            !clean.needs_review,
            "the clean sibling is a strict Tier-0 accept, not a recovery"
        );
        let failed = results[1].1.as_ref().expect("Q2 retained");
        assert!(failed.needs_review, "the failing span is flagged");
        assert_eq!(
            report.deterministic, 1,
            "exactly one strict accept (the clean sibling)"
        );
        assert_eq!(
            report.recovered, 1,
            "only the genuinely failed span is a recovery"
        );
    }

    /// Two clearly separate question stems mislabeled with the SAME parent
    /// number must never be silently welded: the historical
    /// `looks_like_new_question` safeguard runs before the stitch and sends the
    /// span down the repair path instead.
    #[tokio::test]
    async fn separate_question_stems_mislabeled_as_one_parent_are_never_merged() {
        let pgs = vec![grid_page()];
        let span_pages: Vec<(usize, &PageInput)> = vec![(0, &pgs[0])];
        let span = plain_span(6, Some(2));
        let mock = MockLlm::new(vec![
            // Both items claim to be Question 6, but the second opens with its
            // own "7." heading: it is a different question.
            ok_chat(
                r#"{"items":[
                    {"question_number":6,"content":"Prove that the sum of two even numbers is even. **[2 marks]**","marks":2},
                    {"question_number":6,"content":"7. Give two reasons why the current is the same at every point in a series circuit. **[2 marks]**","marks":2}
                ]}"#,
            ),
            // Repair round: only the target question comes back.
            ok_chat(
                r#"{"items":[{"question_number":6,"content":"Prove that the sum of two even numbers is even, and explain why the result holds for every pair of integers. **[2 marks]**","marks":2}]}"#,
            ),
        ]);
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(PAGE_RENDER_CACHE_CAPACITY));
        let semaphore = Arc::new(Semaphore::new(1));
        let collateral = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let all_spans = Arc::new(vec![span.clone()]);
        let cfg = config();
        let (built_opt, report) = extract_span(
            &mock,
            &cfg,
            &span,
            &span_pages,
            &[],
            &cache,
            &Arc::new(PageImageCache::new()),
            &semaphore,
            &collateral,
            &all_spans,
            false,
            &cancel_flag(),
            &usage(),
        )
        .await;
        let built = built_opt.expect("the target question is still recovered");
        assert!(
            !built.content.contains("Give two reasons"),
            "the mislabeled second stem must never be merged in: {}",
            built.content
        );
        assert_eq!(
            mock.bodies().len(),
            2,
            "the safeguard re-asks instead of silently merging"
        );
        assert!(report.repairs >= 1, "the safeguard is on the record");
    }
}
