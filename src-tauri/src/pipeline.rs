// ── The import pipeline: map → carve → verify, all on this machine ────────
//
//   1. MAP: the geometric layout map (or the text-layer scan) locates every
//      question span from the paper itself — numbers, footers, boundaries.
//   2. CARVE: the local transcriber (`deterministic.rs`) takes each span's
//      text, mathematics and figures from the PDF's own evidence.
//   3. VERIFY: every card goes through deterministic gates (question-number
//      conformance, terminal ending, marks checksum vs the printed footer).
//      A card that fails one is kept for review, never passed as clean.
//   4. REPORT: every acceptance, recovery, failure and page that could not be
//      read lands in an ImportReport surfaced to the UI.
//
// No request leaves the machine: there is no model client. Nothing silently
// `continue`s; quarantine is a first-class, visible outcome.

use crate::doc_map::{self, QuestionSpan};
use crate::geometry;
use crate::validate;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use futures_util::StreamExt;

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
    pub paper_name: String,
    pub subject: String,
    pub module_name: String,
    pub allowed_topics: Vec<String>,
    /// Where cropped diagrams are written; `None` skips image persistence
    /// (used in tests).
    pub diagrams_dir: Option<PathBuf>,
    pub pdf_path: Option<PathBuf>,
    /// Questions transcribed at once.
    pub parallelism: usize,
    /// Text-layer classification for THIS document, computed by
    /// `run_question_pipeline` from the pages it was handed (never trusted
    /// from the caller). Reported, never a route: every document is read on
    /// this machine.
    pub(crate) text_layer_class: Option<doc_map::TextLayerClass>,
    /// Immutable per-import local layout evidence (character/run/rule
    /// geometry). Built once for the document by the caller and shared by every
    /// span, so no span reloads the PDF and no stale global path cache exists.
    pub layout_evidence: Option<std::sync::Arc<crate::pdf_render::ImportEvidence>>,
    /// Per-import margin-allocation association, computed once after all spans
    /// are known. Interior mutability so the first span computes it and every
    /// later span reuses the same confirmed records.
    pub margin_model: std::sync::Arc<std::sync::OnceLock<crate::pdf_render::MarginModel>>,
    /// Questions located by the geometric layout map (documents whose
    /// layout evidence was built): question number → the question's exact
    /// body. Set by `run_question_pipeline`, never by callers; when present,
    /// the local transcriber uses these boundaries instead of re-detecting
    /// headings in page text.
    pub(crate) layout_questions: Option<std::sync::Arc<std::collections::HashMap<u32, crate::layout::QuestionBody>>>,
    /// For each layout question, its figures in the order of the
    /// `[DIAGRAM_PLACEHOLDER]` lines its body carries (page, figure).
    pub(crate) layout_figures: Option<std::sync::Arc<std::collections::HashMap<u32, Vec<LayoutFigure>>>>,
    /// Pages whose text local OCR read from a scan: no question on them is
    /// a clean import. Set by `run_question_pipeline`.
    pub(crate) ocr_pages: std::sync::Arc<std::collections::HashSet<usize>>,
}

impl PipelineConfig {
    pub fn new(paper_name: String, subject: String, module_name: String, pdf_path: Option<PathBuf>) -> Self {
        Self {
            paper_name,
            subject,
            module_name,
            allowed_topics: Vec::new(),
            diagrams_dir: None,
            pdf_path,
            parallelism: DEFAULT_PARALLEL,
            text_layer_class: None,
            layout_evidence: None,
            margin_model: std::sync::Arc::new(std::sync::OnceLock::new()),
            layout_questions: None,
            layout_figures: None,
            ocr_pages: Default::default(),
        }
    }

    /// Bind this run to a text-layer classification. The pipeline calls this
    /// itself from the pages it was handed; callers never set it.
    pub(crate) fn with_text_layer_class(mut self, class: doc_map::TextLayerClass) -> Self {
        self.text_layer_class = Some(class);
        self
    }

    /// Human-readable text-layer classification for reports. `"unknown"`
    /// means the pipeline has not classified the document (mark-scheme runs
    /// and direct unit-test entry points).
    pub fn text_layer_label(&self) -> &'static str {
        match self.text_layer_class.as_ref() {
            Some(class) if class.is_digital() => "digital",
            Some(_) => "scanned",
            None => "unknown",
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
    /// Text-layer classification for this document ("digital", "scanned",
    /// or "unknown" before classification).
    pub text_layer: String,
    pub pages_total: usize,
    /// Pages (1-based) that carry print but no text a local reader could
    /// read: a scan with no local OCR engine, or a picture. Nothing on them
    /// was transcribed.
    pub pages_not_transcribed: Vec<usize>,
    pub pages_processed: usize,
    pub questions_expected: usize,
    pub questions_extracted: usize,
    pub paper_total_marks: Option<u32>,
    pub extracted_total_marks: u32,
    pub marks_checksum_ok: Option<bool>,
    pub mark_checks: Vec<MarkCheck>,
    pub quarantined: Vec<QuarantineEvent>,
    pub skipped_pages: Vec<SkippedPage>,
    pub crop_rejections: usize,
    pub diagrams_saved: usize,
    pub diagrams_deduped: usize,
    /// Questions carved by the Tier-0 deterministic transcriber that passed
    /// every strict gate.
    pub deterministic: usize,
    /// Questions retained from a local deterministic carve whose strict
    /// quality gate FAILED: real content, forced into review. Counted
    /// separately from `deterministic` so a recovered card is never reported
    /// as a strict Tier-0 success.
    pub recovered: usize,
    /// Figures located deterministically from the PDF content stream.
    pub figures_detected: usize,
    pub anomalies: Vec<String>,
    /// Questions printed as alternatives, per choice (see
    /// `layout::LayoutMap::choice_groups`).
    pub choice_groups: Vec<crate::layout::ChoiceGroup>,
    pub timings: Vec<TimingEntry>,
    pub total_elapsed_ms: u64,
}

/// Questions transcribed at once. Each unit of work (a span or a shared page)
/// is carved and gated on its own, so running units in parallel changes
/// nothing about the result.
const DEFAULT_PARALLEL: usize = 4;
// Must comfortably cover the distinct figure pages of a real exam paper
// (~20) so high-res crops stay resident and are never re-rendered; 300-DPI
// pages are ~30MB each, so 32 caps worst-case at ~1GB for one import.
const PAGE_RENDER_CACHE_CAPACITY: usize = 32;

/// Minimum Smart-Scissors confidence for a detected figure to count as
/// supplying a figure the question refers to.
const FIGURE_SUPPLY_MIN_CONFIDENCE: f32 = 0.5;

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
    report: ImportReport,
}

async fn persist_diagrams(
    requests: Vec<DiagramSaveRequest>,
    page_b64: std::collections::HashMap<usize, String>,
    config: PipelineConfig,
    page_render_cache: Arc<crate::pdf_render::PageRenderCache>,
) -> Result<DiagramPersistence, tokio::task::JoinError> {
    tokio::task::spawn_blocking(move || {
        // Figures already saved for this question, so a repeated crop
        // reuses its file.
        let mut saved = Vec::new();
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
        DiagramPersistence { links, report }
    })
    .await
}

impl ImportReport {
    /// Fold a per-unit report (one span / page / window processed inside a
    /// parallel batch) back into the master report.
    pub fn absorb(&mut self, o: ImportReport) {
        self.pages_processed += o.pages_processed;
        self.crop_rejections += o.crop_rejections;
        self.diagrams_saved += o.diagrams_saved;
        self.diagrams_deduped += o.diagrams_deduped;
        self.deterministic += o.deterministic;
        self.recovered += o.recovered;
        self.figures_detected += o.figures_detected;
        self.pages_not_transcribed.extend(o.pages_not_transcribed);
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
    /// The pages the question was read from (0-based): its span in the
    /// layout map, kept so the card can be checked against the paper.
    pub pages: std::ops::RangeInclusive<usize>,
}

#[derive(Debug, Clone)]
pub struct AnswerDraft {
    pub question_number: u32,
    pub markdown: String,
}

fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Import cancelled by user".to_string())
    } else {
        Ok(())
    }
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
/// Unknown terminology stays untagged.
fn classify_topics_deferred(
    config: &PipelineConfig,
    built: &mut [BuiltQuestion],
    cancel: &AtomicBool,
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

pub async fn run_question_pipeline<P: Progress>(
    pages: &[PageInput],
    page_figures: &[Vec<crate::pdf_render::DetectedFigure>],
    config: &PipelineConfig,
    progress: &P,
    cancel: &AtomicBool,
) -> Result<(Vec<BuiltQuestion>, ImportReport), String> {
    let overall_start = Instant::now();
    // Classify the document from its text layer and bind that verdict to this
    // run's config (for the report). The classification is computed here from
    // the pages actually handed to the pipeline - a caller cannot set it.
    let page_texts: Vec<String> = pages.iter().map(|p| p.text.clone()).collect();
    let text_layer_class = doc_map::classify_text_layer(&page_texts);
    let mut effective_config = config.clone().with_text_layer_class(text_layer_class.clone());
    // Pages read by local OCR, by their text or their layout.
    effective_config.ocr_pages = std::sync::Arc::new(
        pages
            .iter()
            .enumerate()
            .filter(|(i, p)| crate::pdf_render::is_ocr_text(&p.text) || config.layout_evidence.as_ref().is_some_and(|ev| ev.layout.get(*i).is_some_and(|lp| lp.ocr)))
            .map(|(i, _)| i)
            .collect(),
    );
    let mut report = ImportReport {
        paper_name: effective_config.paper_name.clone(),
        kind: "questions".to_string(),
        text_layer: effective_config.text_layer_label().to_string(),
        pages_total: pages.len(),
        figures_detected: page_figures.iter().map(Vec::len).sum(),
        ..Default::default()
    };
    if !text_layer_class.unresolved_pages().is_empty() {
        report.anomalies.push(format!(
            "{}/{} pages carry extracted text; pages {:?} have no text layer",
            text_layer_class.text_pages,
            text_layer_class.total_pages,
            text_layer_class
                .unresolved_pages()
                .iter()
                .map(|p| p + 1)
                .collect::<Vec<_>>()
        ));
    }
    // Geometry-first layout (documents with layout evidence): page text in
    // reading order with reconstructed mathematics, and a question map with
    // exact line boundaries.
    let mut layout_spans: Option<Vec<QuestionSpan>> = None;
    let layout_pages_input: Option<Vec<PageInput>> = match config.layout_evidence.as_ref() {
        Some(ev) if !ev.layout.is_empty() && ev.layout.len() == pages.len() => {
            let lmap = crate::layout::build_layout_map(&ev.layout);
            report.anomalies.extend(lmap.anomalies.iter().cloned());
            report.choice_groups = lmap.choice_groups.clone();
            let scanned = |f: fn(&crate::layout::LayoutPage) -> bool| ev.layout.iter().filter(|lp| f(lp)).map(|lp| lp.page + 1).collect::<Vec<_>>();
            let (read, unread) = (scanned(|lp| lp.ocr), scanned(|lp| lp.scan_unread));
            if !read.is_empty() {
                report.anomalies.push(format!("scanned pages {read:?} read by local OCR: their questions are kept for review"));
            }
            if !unread.is_empty() {
                report.anomalies.push(format!("scanned pages {unread:?} have no usable text layer and no local OCR could read them: not transcribed"));
            }
            for &page in &unread {
                report.quarantined.push(QuarantineEvent {
                    scope: "page".to_string(),
                    page: Some(page),
                    question_number: None,
                    reason: "scanned page not transcribed".to_string(),
                });
            }
            report.pages_not_transcribed = unread;
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
        // No layout evidence (a picture uploaded on its own): its pages
        // without text are not transcribed.
        _ => {
            report.pages_not_transcribed = pages
                .iter()
                .enumerate()
                .filter(|(_, p)| matches!(p.kind, PageInputKind::Image { .. }) && p.text.trim().is_empty())
                .map(|(i, _)| i + 1)
                .collect();
            for &page in &report.pages_not_transcribed {
                report.quarantined.push(QuarantineEvent {
                    scope: "page".to_string(),
                    page: Some(page),
                    question_number: None,
                    reason: "picture not transcribed".to_string(),
                });
            }
            None
        }
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

    // Time the text-layer document map building
    let text_map_start = Instant::now();
    let scan = doc_map::scan_text_layer(&page_texts);
    report.record_timing(
        "document_map",
        "text_layer_scan",
        None,
        None,
        text_map_start.elapsed().as_millis() as u64,
    );

    // A page the text scan reads as non-question content (a blank page, an
    // answer booklet) still lends its text to a span the map runs across.
    let text_non_question = |p: usize| scan.page_reliability.get(p) == Some(&doc_map::PageReliability::NonQuestion);

    // ── 2. Document map ─────────────────────────────────────────────────────
    let doc_map_start = Instant::now();
    let map = if let Some(spans) = layout_spans.take() {
        doc_map::DocumentMap { spans, ..Default::default() }
    } else {
        doc_map::build_text_only_map(&page_texts, pages.len(), &scan)
    };
    report.timings.push(TimingEntry {
        stage: "document_map".to_string(),
        operation: "build_hybrid_map".to_string(),
        page: None,
        question_number: None,
        milliseconds: doc_map_start.elapsed().as_millis() as u64,
    });

    report.paper_total_marks = map.paper_total_marks;
    report.anomalies.extend(map.anomalies.clone());

    // ── 3. Span extraction ──────────────────────────────────────────────────
    let mut built: Vec<BuiltQuestion> = Vec::new();

    // A document whose text produced no spans at all has nothing left to
    // read. Report the local failure explicitly.
    if map.spans.is_empty() {
        report.anomalies.push(
            "the text layer produced no question spans - nothing extracted".to_string(),
        );
        report.quarantined.push(QuarantineEvent {
            scope: "document".to_string(),
            page: None,
            question_number: None,
            reason: "no locally derivable question spans".to_string(),
        });
    }

    report.questions_expected = map.spans.len();
    let total = map.spans.len();

    // Group consecutive single-page spans sharing the same page into
    // SamePageBatch jobs: they are carved together from that page.
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
                || text_non_question(p);

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
                    || text_non_question(pi)
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
    let all_spans = Arc::new(map.spans.clone());

    let mut results = futures_util::stream::iter(0..job_count).map(|position| {
        let job = &jobs[position];
        let config = config;
        let page_figures = page_figures;
        let page_render_cache = Arc::clone(&page_render_cache);
        let all_spans = Arc::clone(&all_spans);
        async move {
            match job {
                ExtractionJob::Single { span, pages } => {
                    let (opt, local_rep) = extract_span(
                        config,
                        span,
                        pages,
                        page_figures,
                        &page_render_cache,
                        &all_spans,
                        cancel,
                    )
                    .await;
                    (position, vec![((*span).clone(), opt)], local_rep)
                }
                ExtractionJob::SamePageBatch { spans, page_idx, page_input } => {
                    let (res_vec, local_rep) = extract_same_page_batch(
                        config,
                        spans,
                        *page_idx,
                        page_input,
                        page_figures,
                        &page_render_cache,
                        &all_spans,
                        cancel,
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
                    let mut reason = "failed local validation".to_string();
                    if let Some(err) = report.anomalies.last() {
                        if err.starts_with("quarantined: ") {
                            reason = format!(
                                "failed local validation (last error: {})",
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
                        pages: source_pages(config, &span),
                    });
                }
            }
        }
    }

    // ── Deferred topic classification (on-device) ─────────
    // Tier-0 cards arrive without topics; match syllabus keywords locally.
    if !config.allowed_topics.is_empty() {
        let topics_start = Instant::now();
        let assigned = classify_topics_deferred(config, &mut built, cancel);
        if assigned > 0 {
            report.record_timing(
                "topics",
                "deferred_classification",
                None,
                None,
                topics_start.elapsed().as_millis() as u64,
            );
            eprintln!(
                "[TOPICS] {} question(s) classified locally",
                assigned
            );
        }
    }

    report.questions_extracted = built.len();
    report.extracted_total_marks = built.iter().map(|q| q.marks.max(0) as u32).sum();
    // Removed printed paper total checksum warning as requested
    report.marks_checksum_ok = None;
    report.total_elapsed_ms = overall_start.elapsed().as_millis() as u64;

    eprintln!(
        "[PATH_SUMMARY] {} questions: {} strict local, {} local recoveries (figures detected: {})",
        report.questions_extracted,
        report.deterministic,
        report.recovered,
        report.figures_detected,
    );

    // Per-stage timing so the import log can prove where wall-time went.
    // Entries are only recorded when a stage ran; missing stages print 0ms.
    let timing = |stage: &str, operation: &str| -> u64 {
        report
            .timings
            .iter()
            .find(|t| t.stage == stage && t.operation == operation)
            .map(|t| t.milliseconds)
            .unwrap_or(0)
    };
    eprintln!(
        "[TIMING] document_map={}ms extraction_span_stream={}ms total={}ms",
        timing("document_map", "text_layer_scan"),
        timing("extraction", "span_stream"),
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
        "Question {}: local recovery (Tier-0 card flagged: {}); content retained for review",
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

/// Whether the text refers to a figure (Figure N, diagram, graph, circuit,
/// sketch, …): such a question needs its figure attached. When in doubt this
/// returns true.
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
/// vertical band on its OWN pages (start_y_frac/end_y_frac on the span's
/// first/last page, full height on interior pages).
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
/// skipping the span's own pages here would leave figure questions without
/// their figure.
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

/// Placeholder tokens a transcription may leave in content.
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

/// Splice deterministic figure crops into a question's content.
///
/// Order of placement:
///   1. one crop link per placeholder token,
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
    // Any placeholder that survived (more placeholders than crops, or every
    // crop was rejected) is dropped — never leak a bare token into a card.
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

/// Tier-0 seam: the local transcriber's text for one question becomes a
/// card here, or `None` when it fails a gate (the caller then keeps the
/// content as a flagged local recovery). A layout body is assembled as
/// given. A text-layer transcription must name a real question, have its
/// figures supplied, balance its maths delimiters, and pass the structural
/// checks both before and after assembly.
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
    let refuse = |reason: String| -> Option<BuiltQuestion> {
        eprintln!("[SEAM_REFUSED] question={} reason={}", span.number, reason);
        None
    };
    // A span numbered 0 comes from a footer that names no question, so the
    // card's question is not established.
    if !(1..=1000).contains(&span.number) {
        return refuse("question_number".to_string());
    }
    if content.trim().is_empty() {
        return refuse("empty_content".to_string());
    }
    // Every `[DIAGRAM_PLACEHOLDER]` needs a figure. More placeholders than
    // figures pass only when they repeat references to figures that can all
    // be supplied ("Figure 9" in parts (a) and (b)): figure attachment places
    // one crop and drops the repeats.
    let placeholders = content.matches("[DIAGRAM_PLACEHOLDER]").count();
    if placeholders > available_figures {
        let distinct: std::collections::HashSet<u32> = figure_reference_numbers(&content).into_iter().collect();
        if distinct.is_empty() || distinct.len() > available_figures {
            return refuse(format!(
                "figure_needed placeholders={} figures={} distinct_refs={}",
                placeholders, available_figures, distinct.len()
            ));
        }
    }
    if content.trim().len() < 5 && span.expected_marks.unwrap_or(0) > 1 {
        return refuse("nearly_empty".to_string());
    }
    let delimiters = validate::math_delimiter_balance_errors(&content);
    if !delimiters.is_empty() {
        return refuse(format!("delimiters({})", delimiters.join("; ")));
    }
    // Structure is judged on the sanitized content: a defect the sanitizer
    // repairs is not a defect of the transcription.
    let sanitized = crate::sanitize::sanitize_question_content(&content, span.number);
    let structural = validate::card_structure_errors(&sanitized, span.number);
    if !structural.is_empty() {
        return refuse(format!("structure({})", structural.join("; ")));
    }
    // The transcriber's marks total (at most 100) counts only when the card
    // has neither a printed footer nor mark tags.
    let built = assemble_built_question(
        span,
        config,
        vec![content],
        Vec::new(),
        false,
        false,
        Vec::new(),
        marks.map(|m| m.min(100) as i32),
    )?;
    // Assembly rewrites the content, so the card itself must pass too.
    let structural = validate::card_structure_errors(&built.content, span.number);
    if !structural.is_empty() {
        return refuse(format!("assembled_structure({})", structural.join("; ")));
    }
    Some(built)
}

/// Lenient assembly for a locally recovered card.
///
/// The strict seam rejects any card that still carries a structural defect.
/// There is nothing else to ask, so the honest move is to keep the real
/// carved content and FLAG it: `needs_review` is forced true,
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
                "local recovery: quality gate '{}' failed; content retained for review",
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
            "local recovery: quality gate '{}' failed; content retained for review",
            failed_gate
        )],
        marks_hint,
    )
}

/// One question span, carved locally.
/// Returns (Some(question), report) on acceptance (possibly flagged),
/// (None, report) on quarantine — the LOCAL report is absorbed by the caller
/// (this runs inside a parallel batch).
async fn extract_span(
    config: &PipelineConfig,
    span: &QuestionSpan,
    span_pages: &[(usize, &PageInput)],
    page_figures: &[Vec<crate::pdf_render::DetectedFigure>],
    page_render_cache: &Arc<crate::pdf_render::PageRenderCache>,
    all_spans: &Arc<Vec<QuestionSpan>>,
    cancel: &AtomicBool,
) -> (Option<BuiltQuestion>, ImportReport) {
    // Own, local report: spans now run in parallel batches, so each unit
    // accumulates its own bookkeeping and the caller absorbs it in order.
    let mut report = ImportReport::default();

    if cancel.load(Ordering::Relaxed) {
        return (None, report);
    }

    let combined_text: String = span_pages
        .iter()
        .map(|(_, p)| p.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let has_text = !combined_text.trim().is_empty();
    let referenced = if has_text {
        figure_reference_numbers(&combined_text)
    } else {
        Vec::new()
    };
    // Supply counting only trusts confident detections: a low-confidence
    // region must never silently satisfy a figure reference.
    // (Low-confidence figures still attach when explicitly referenced.)
    let fig_count = available_span_figures(span, span_pages, page_figures, &referenced);

    // ── Tier-0 deterministic extraction ──────────────────────────────────
    // Text-reliable spans are carved out of the text layer by the local Rust
    // transcriber. A span that fails a strict gate is kept as a flagged local
    // recovery; there is nothing else to ask.
    if has_text {
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
            // Attach figures (scrubs leftover placeholders), bookkeeping,
            // mark check, absorb.
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
        // A failed strict carve: keep the best locally carved candidate,
        // flagged for review with the failed gate on the record; if the carve
        // could not isolate the question at all, fail locally instead of
        // pretending.
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
                "Question {}: local recovery (failed gate: {}); content retained for review",
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

    // A span that reaches here is a real local failure: either its pages
    // carry no usable text, or the carve could not isolate the question.
    // Report it plainly.
    let reason = if has_text {
        "no viable local candidate"
    } else {
        "span pages carry no usable text"
    };
    local_failure(&mut report, span, span.start_page, reason);
    // A failed span still accounts for every page it covers.
    report.pages_processed += (span.start_page..=span.end_page).count().max(1) - 1;
    (None, report)
}

/// Record a question nothing could be carved for: logged, an anomaly, a
/// quarantine entry, and its page counted.
fn local_failure(report: &mut ImportReport, span: &QuestionSpan, page_idx: usize, reason: &str) {
    eprintln!("[LOCAL_FAILURE] question={} reason={}", span.number, reason);
    report.anomalies.push(format!("Question {}: {}", span.number, reason));
    report.quarantined.push(QuarantineEvent {
        scope: "question".to_string(),
        page: Some(page_idx + 1),
        question_number: Some(span.number),
        reason: reason.to_string(),
    });
    report.pages_processed += 1;
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
    item_marks: Option<i32>,
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
    // the acceptance seam — so every card is clean and the structural
    // validator below never fires on issues the sanitizer already fixed.
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
        span.number, span.expected_marks, inline, item_marks
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
            item_marks.unwrap_or(1).max(1),
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
        pages: source_pages(config, span),
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

/// The pages a card records as its source: the layout body's, which take in
/// a stem printed on an earlier page, or else the span's.
fn source_pages(config: &PipelineConfig, span: &QuestionSpan) -> std::ops::RangeInclusive<usize> {
    let first = config.layout_questions.as_ref().and_then(|m| m.get(&span.number)).map_or(span.start_page, |b| b.pages.0.min(span.start_page));
    first..=span.end_page
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
    // (From the page of material printed ahead of it for it.)
    for p in q.first_page()..=q.end_page {
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
/// text-carve repair heuristics run here — only the
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
    let end_is_printed = config.layout_questions.as_ref().and_then(|m| m.get(&span.number)).is_some_and(|b| b.end_is_printed());
    if !validate::has_terminal_ending(&content) && !end_is_printed {
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
        pages: source_pages(config, span),
    })
}

/// Extract the questions printed together on one page: carved as a batch, and
/// one by one when the batch is refused.
async fn extract_same_page_batch(
    config: &PipelineConfig,
    spans: &[&QuestionSpan],
    page_idx: usize,
    page: &PageInput,
    page_figures: &[Vec<crate::pdf_render::DetectedFigure>],
    page_render_cache: &Arc<crate::pdf_render::PageRenderCache>,
    all_spans: &Arc<Vec<QuestionSpan>>,
    cancel: &AtomicBool,
) -> (Vec<(QuestionSpan, Option<BuiltQuestion>)>, ImportReport) {
    let mut report = ImportReport::default();
    if cancel.load(Ordering::Relaxed) {
        return (Vec::new(), report);
    }

    // ── Shared-page batches ──────────────────────────────────────────────
    // Several questions printed on one page are carved together from its text
    // layer, figures attached deterministically via the caption-aware
    // candidate lookup.
    if !page.text.trim().is_empty() {
        let span_pages = [(page_idx, page)];
        let combined_text = page.text.trim().to_string();
        let referenced = figure_reference_numbers(&combined_text);
        let mut batch_out: Vec<(QuestionSpan, Option<BuiltQuestion>)> = Vec::with_capacity(spans.len());
        let mut batch_report = ImportReport::default();

        // Per-span figure counts for the placeholder gate in the shared build
        // helper.
        let fig_counts: Vec<usize> = spans
            .iter()
            .map(|span| {
                available_span_figures(span, &span_pages, page_figures, &referenced)
            })
            .collect();

        // ── Tier-0 deterministic batch ───────────────────────────────────
        // Carve EVERY target span locally; all green ⇒ done. Any refusal ⇒
        // discard partials and resolve each span on its own below.
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
                    &mut batch_report,
                )
                .await;
                settle_tier0_outcome(span, &built_q, &mut batch_report);
                batch_report.pages_processed += 1;
                push_mark_check(span, &built_q, &mut batch_report);
                batch_out.push(((*span).clone(), Some(built_q)));
            }
            return (batch_out, batch_report);
        }
        // A failed strict batch: every span on this page resolves locally.
        // Recover what can be recovered, flag it, and report the rest as
        // local failures.
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
                batch_report.absorb(strict_report);
                batch_out.push(((*span).clone(), Some(built_q)));
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
                    "Question {}: local recovery (failed gate: {}); content retained for review",
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
                batch_report.absorb(rec_report);
                batch_out.push(((*span).clone(), Some(built_q)));
            } else {
                local_failure(&mut batch_report, span, page_idx, "no viable local candidate");
                batch_out.push(((*span).clone(), None));
            }
        }
        return (batch_out, batch_report);
    }

    // A shared page that carries no usable text at all: there is nothing to
    // carve. Report each span as a local failure.
    let mut out = Vec::with_capacity(spans.len());
    for span in spans {
        local_failure(&mut report, span, page_idx, "page carries no usable text");
        out.push(((*span).clone(), None));
    }
    (out, report)
}

/// Crop + persist one diagram; returns the markdown link on success.
/// `saved` carries the (signature, link) pairs already persisted for this
/// unit of work — a near-identical crop reuses the stored file instead of
/// writing yet another PNG of the same figure.

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

// ══════════════════════════════════════════════════════════════════════════
// Mark-scheme pipeline
// ══════════════════════════════════════════════════════════════════════════

/// The quarantine scope of a mark scheme the local reader refused.
pub(crate) const MARK_SCHEME_REFUSAL: &str = "mark-scheme";

pub async fn run_markscheme_pipeline<P: Progress>(
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
    // The local reader takes a born-digital PDF scheme's tables cell by cell
    // and accepts the document only when every answer page is accounted for.
    // Whatever it refuses is reported, with its reason, as not transcribed.
    let refusal = match config.pdf_path.as_ref().filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))) {
        Some(path) => {
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
                Ok(_) => "the PDF and the pages read from it disagree on the page count".to_string(),
                Err(reason) => reason,
            }
        }
        None => "only a PDF mark scheme can be read locally".to_string(),
    };
    eprintln!("[MS_TIER0_REFUSED] {refusal}");
    report.anomalies.push(format!("mark scheme not transcribed: the local reader refused it ({refusal})"));
    report.quarantined.push(QuarantineEvent {
        scope: MARK_SCHEME_REFUSAL.to_string(),
        page: None,
        question_number: None,
        reason: refusal,
    });
    report.total_elapsed_ms = overall_start.elapsed().as_millis() as u64;
    report.record_timing("extraction", "ms_tier0_refused", None, None, report.total_elapsed_ms);
    Ok((Vec::new(), report))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> PipelineConfig {
        let mut c = PipelineConfig::new(
            "Unit".into(),
            "Mathematics".into(),
            "Algebra".into(),
            None,
        );
        c.allowed_topics = vec!["Proof".into(), "Integration".into()];
        c.parallelism = 1;
        c
    }

    fn cancel_flag() -> AtomicBool {
        AtomicBool::new(false)
    }

    /// A question's figures include those of a stem it shares with the
    /// questions before it, printed on an earlier page than its heading.
    #[test]
    fn a_question_takes_the_figure_of_its_stem_on_an_earlier_page() {
        let page = |figures: Vec<[f32; 4]>| crate::layout::LayoutPage { width: 595.0, height: 842.0, figures, ..Default::default() };
        let layout = vec![page(vec![[100.0, 440.0, 300.0, 480.0]]), page(vec![])];
        let stem = crate::layout::AheadWindow { start_page: 0, start_y: 400.0, end_page: 0, end_y: 500.0, appended: false };
        let q = crate::layout::LayoutQuestion { number: 3, lines: vec![], start_page: 1, end_page: 1, start_y: 100.0, end_y: None, reference_from: None, ahead: Some(stem) };
        let figures = layout_question_figures(&layout, &q, &[vec![], vec![]]);
        assert_eq!(figures.iter().map(|(f, _)| f.page).collect::<Vec<_>>(), vec![0]);
    }

    /// A card records the pages its layout body was read from, an earlier
    /// page of a stem included; without a layout body, its span's.
    #[test]
    fn a_card_records_the_pages_its_body_was_read_from() {
        let span = QuestionSpan { number: 3, start_page: 1, end_page: 2, start_y_frac: None, end_y_frac: None, expected_marks: None, reliable_pages: vec![], ambiguous_pages: vec![] };
        let mut c = config();
        assert_eq!(source_pages(&c, &span), 1..=2);
        let body = crate::layout::QuestionBody { pages: (0, 2), ..Default::default() };
        c.layout_questions = Some(Arc::new([(3, body)].into_iter().collect()));
        assert_eq!(source_pages(&c, &span), 0..=2);
    }

    #[tokio::test]
    async fn mark_scheme_tier0_uses_pdf_evidence() {
        let _lock = crate::pdf_render::pdfium_test_lock();
        let path = std::env::temp_dir().join(format!("mm_ms_pipeline_{}.pdf", uuid::Uuid::new_v4()));
        crate::mark_scheme_deterministic::tests::write_pdf_fixture(&path, false);
        let mut cfg = config();
        cfg.pdf_path = Some(path.clone());
        let pages = vec![PageInput { kind: PageInputKind::TextOnly, text: "Page evidence is read from the PDF".into() }];
        let (drafts, report) = run_markscheme_pipeline(&pages, &cfg, &NullProgress, &cancel_flag()).await.unwrap();
        assert_eq!(drafts.len(), 1);
        assert!(drafts[0].markdown.contains("The force increases."));
        assert!(drafts[0].markdown.contains("Accept greater force."));
        assert_eq!(report.deterministic, 1);
        assert_eq!(report.pages_processed, 1);
        std::fs::remove_file(path).unwrap();
    }

    /// A scheme the local reader refuses gives no answers at all, never a
    /// partial set, and its reason is on the record.
    #[tokio::test]
    async fn mark_scheme_tier0_refusal_is_reported_without_partial_answers() {
        let _lock = crate::pdf_render::pdfium_test_lock();
        let path = std::env::temp_dir().join(format!("mm_ms_fallback_{}.pdf", uuid::Uuid::new_v4()));
        crate::mark_scheme_deterministic::tests::write_pdf_fixture(&path, true);
        let mut cfg = config();
        cfg.pdf_path = Some(path.clone());
        let text = "Question Answers Additional comments Mark AO\n01.1 The force increases as extension increases. Accept equivalent wording. Marking point M1. Accuracy A1. Total 2 marks.";
        let pages = vec![PageInput { kind: PageInputKind::TextOnly, text: text.repeat(4) }; 2];
        let (drafts, report) = run_markscheme_pipeline(&pages, &cfg, &NullProgress, &cancel_flag()).await.unwrap();
        assert!(drafts.is_empty());
        assert_eq!(report.deterministic, 0);
        assert_eq!(report.quarantined.len(), 1);
        assert_eq!(report.quarantined[0].scope, "mark-scheme");
        assert!(report.anomalies.iter().any(|a| a.contains("not transcribed")), "{:?}", report.anomalies);
        std::fs::remove_file(path).unwrap();
    }

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

    // ── Figures ────────────────────────────────────────────────────────────

    fn text_image_page() -> PageInput {
        let mut g = gray_blank(1200, 1600);
        g_hline(&mut g, 400);
        g_blob(&mut g, 800, 100, 500);
        PageInput {
            kind: PageInputKind::Image { b64: png_b64(&g) },
            text: "State the value of $x$ when $2x + 4 = 10$.\n\n[2 marks]\n\nShow your working.".into(),
        }
    }

    fn span_with_marks(number: u32, marks: u32) -> doc_map::QuestionSpan {
        doc_map::QuestionSpan {
            number,
            start_page: 0,
            end_page: 0,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: Some(marks),
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        }
    }

    #[test]
    fn seam_accepts_duplicate_placeholders_for_one_referenced_figure() {
        // Q6 regression: one placeholder PER REFERENCE of the same figure
        // ("Figure 9" appears in parts (a) and (b) → 2 placeholders). One
        // distinct figure, one available → the gate accepts and leaves the
        // extras for attach to collapse.
        let content = "(a) Use Figure 9 to find the value. [DIAGRAM_PLACEHOLDER]\n\n(b) State the gradient of Figure 9. [DIAGRAM_PLACEHOLDER] **[2 marks]**";
        let built = build_question_from_seam(content.to_string(), Some(2), &span_with_marks(30, 2), &config(), 1)
            .expect("duplicate placeholders for ONE referenced figure are supplied");
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
    fn seam_refuses_when_distinct_figures_exceed_supply() {
        // Two placeholders referencing TWO DIFFERENT figures with only ONE
        // figure available → genuinely under-supplied.
        let content = "(a) Use Figure 9. [DIAGRAM_PLACEHOLDER]\n\n(b) Use Figure 10. [DIAGRAM_PLACEHOLDER] **[2 marks]**";
        assert!(
            build_question_from_seam(content.to_string(), Some(2), &span_with_marks(30, 2), &config(), 1).is_none(),
            "2 distinct figures with 1 available must be refused"
        );
    }

    #[test]
    fn seam_refuses_an_unestablished_question_and_unbalanced_maths() {
        let seam = |content: &str, span: &doc_map::QuestionSpan| {
            build_question_from_seam(content.to_string(), Some(2), span, &config(), 0)
        };
        let clean = "Find the value of $x$ when $2x + 4 = 10$. **[2 marks]**";
        let built = seam(clean, &span_with_marks(3, 2)).expect("a clean transcription is a card");
        assert_eq!((built.marks, built.needs_review), (2, false));
        // A span numbered 0 (a footer that names no question) is no question.
        assert!(seam(clean, &span_with_marks(0, 2)).is_none());
        assert!(seam("   ", &span_with_marks(3, 2)).is_none());
        assert!(seam("(a)", &span_with_marks(3, 2)).is_none(), "nearly empty for a 2-mark question");
        assert!(seam("Find the value of $x when $2x + 4 = 10$. **[2 marks]**", &span_with_marks(3, 2)).is_none());
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

    #[test]
    fn caption_match_finds_figure_above_band_on_span_page() {
        // The regression: AQA places the figure ABOVE the question text, so
        // its centre sits OUTSIDE the question's vertical band. The band pass
        // rejects it, and the caption pass used to skip the span's own pages
        // entirely → fig_count = 0 → the question lost its figure.
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
        let dir = std::env::temp_dir().join(format!("mm_cropfirst_{}", uuid::Uuid::new_v4()));
        let mut cfg = config();
        cfg.diagrams_dir = Some(dir.clone());
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(
            PAGE_RENDER_CACHE_CAPACITY,
        ));
        let all_spans = Arc::new(vec![span.clone()]);
        let (built_opt, report) = extract_span(
            &cfg,
            &span,
            &span_pages,
            &page_figures,
            &cache,
            &all_spans,
            &cancel_flag(),
        )
        .await;
        let built = built_opt.expect("Tier-0 must build the question");
        assert!(built.content.contains("determine the acceleration"));
        assert_eq!(report.deterministic, 1);
        assert!(!built.needs_review, "{:?}", built.notes);
        assert_eq!(
            built.content.matches("![Diagram](").count(),
            1,
            "the crop is attached to the card: {}",
            built.content
        );
        // A detection that cannot be persisted is not a verified attachment.
        cfg.diagrams_dir = None;
        let (review, _) = extract_span(&cfg, &span, &span_pages, &page_figures,
            &cache, &all_spans, &cancel_flag()).await;
        let review = review.unwrap();
        assert!(review.needs_review);
        assert!(review.notes.iter().any(|n| n.contains("could not be attached")));
        assert!(!review.content.contains("PLACEHOLDER"));
        let _ = std::fs::remove_dir_all(&dir);
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
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(PAGE_RENDER_CACHE_CAPACITY));
        let all_spans = Arc::new(vec![span.clone()]);
        let mut cfg = config();
        // Production conditions: commands.rs ALWAYS populates allowed_topics
        // from the module taxonomy. Tier 0 must still carve locally.
        cfg.allowed_topics = vec!["Materials".into(), "Electricity".into()];
        let (built_opt, report) =
            extract_span(&cfg, &span, &span_pages, &[], &cache, &all_spans, &cancel_flag()).await;
        let built = built_opt.expect("Tier-0 must carve this span locally");
        assert_eq!(built.marks, 4);
        assert!(built.content.contains("specific heat capacity"));
        assert_eq!(report.deterministic, 1);
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
        let cache = Arc::new(crate::pdf_render::PageRenderCache::new(PAGE_RENDER_CACHE_CAPACITY));
        let all_spans = Arc::new(spans.clone());
        let mut cfg = config();
        cfg.allowed_topics = vec!["Electricity".into()];
        let (results, report) = extract_same_page_batch(
            &cfg, &span_refs, 0, &pgs[0], &[], &cache, &all_spans, &cancel_flag(),
        )
        .await;
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|(_, q)| q.is_some()));
        assert_eq!(report.deterministic, 3, "whole page carved locally");
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
            pages: 0..=0,
        };
        let mut qs = vec![
            mk(5, vec![]),                        // untagged → classified
            mk(6, vec!["Mechanics".to_string()]), // already tagged → untouched
        ];
        let mut cfg = config();
        cfg.allowed_topics = vec!["Thermal".into(), "Mechanics".into()];
        let assigned = classify_topics_deferred(&cfg, &mut qs, &cancel_flag());
        assert_eq!(assigned, 1, "only the untagged question is assigned");
        assert_eq!(qs[0].topics, vec!["Thermal"], "out-of-list topic dropped");
        assert_eq!(qs[1].topics, vec!["Mechanics"], "tagged card untouched");
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
            needs_review: false, notes: vec![], pages: 0..=0,
        }).collect();
        let assigned = classify_topics_deferred(&cfg, &mut qs, &cancel_flag());
        assert_eq!(assigned, 4);
        for i in 0..4 { assert_eq!(qs[i].topics, vec![cfg.allowed_topics[i].clone()]); }
        assert!(qs[4].topics.is_empty(), "wave inside microwave is not a keyword");
    }

    #[tokio::test]
    async fn deferred_topic_classification_unknown_leaves_untagged() {
        let mut qs = vec![BuiltQuestion {
            question_number: 5,
            content: "Question body long enough for classification to attempt.".to_string(),
            marks: 2,
            topics: vec![],
            module: "Algebra".into(),
            is_code: false,
            needs_review: false,
            notes: vec![],
            pages: 0..=0,
        }];
        let mut cfg = config();
        cfg.allowed_topics = vec!["Thermal".into()];
        let assigned = classify_topics_deferred(&cfg, &mut qs, &cancel_flag());
        assert_eq!(assigned, 0, "unknown terminology must not fabricate tags");
        assert!(qs[0].topics.is_empty());
    }

    /// Production-conditions end-to-end: taxonomy ALWAYS populated (see
    /// commands.rs), Tier-0 carves both footer pages locally, and topic
    /// classification tags them on this machine.
    #[tokio::test]
    async fn full_pipeline_with_topics_is_local() {
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
        let mut cfg = config();
        cfg.allowed_topics = vec!["Electricity".into(), "Required Practical".into()];
        let (built, report) =
            run_question_pipeline(&pgs, &[], &cfg, &NullProgress, &cancel_flag())
                .await
                .unwrap();

        assert_eq!(built.len(), 2);
        assert_eq!(report.deterministic, 2, "both spans carved locally");
        let q1 = built.iter().find(|q| q.question_number == 1).unwrap();
        let q2 = built.iter().find(|q| q.question_number == 2).unwrap();
        assert_eq!(q1.topics, vec!["Electricity"]);
        assert_eq!(q2.topics, vec!["Required Practical"]);
    }

    // -- Digital question ingestion ------------------------------------------

    /// The weak-map fixture: born-digital pages whose heading sequence is
    /// sparse (two questions).
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
            doc_map::classify_text_layer(&texts).is_digital(),
            "map insufficiency must not downgrade a digital paper"
        );
        let map = doc_map::build_text_only_map(&texts, texts.len(), &scan);
        assert_eq!(
            map.spans.iter().map(|s| s.number).collect::<Vec<_>>(),
            vec![1, 2],
            "the text-only map still places both questions"
        );
    }

    /// A real but SHORT digital question paper still ingests: there is no
    /// document-size floor.
    #[tokio::test]
    async fn digital_short_paper_ingests_locally() {
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
        let cfg = config();
        let (built, report) =
            run_question_pipeline(&pages, &[], &cfg, &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert_eq!(report.text_layer, "digital");
        assert!(!built.is_empty(), "the question is carved locally");
    }

    /// A genuine one-page digital paper must keep its first-page question: no
    /// synthetic cover, no discarded Q1.
    #[tokio::test]
    async fn digital_one_page_paper_keeps_first_page_question() {
        let pages = vec![PageInput {
            kind: PageInputKind::TextOnly,
            text: "1. The transformation P is an enlargement with scale factor k. Show that k = 3. [4 marks]\nEND OF QUESTIONS".to_string(),
        }];
        let cfg = config();
        let (built, report) =
            run_question_pipeline(&pages, &[], &cfg, &NullProgress, &cancel_flag())
                .await
                .unwrap();
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
    /// prose at all, and is still handled, not dropped.
    #[tokio::test]
    async fn digital_maths_only_question_is_handled_locally() {
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
        let cfg = config();
        let (built, report) =
            run_question_pipeline(&pages, &[], &cfg, &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert_eq!(report.text_layer, "digital");
        assert!(
            report.deterministic + report.recovered + report.quarantined.len() >= 1
                || !built.is_empty(),
            "the span was handled locally, not dropped"
        );
    }

    /// Blank, END-OF-QUESTIONS and image-only pages in a digital paper: its
    /// questions still ingest, and the pages with no text layer are reported.
    #[tokio::test]
    async fn digital_with_blank_and_backmatter_pages_reports_gaps() {
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

        let cfg = config();
        let (built, report) =
            run_question_pipeline(&pages, &[], &cfg, &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert!(!built.is_empty());
        assert!(
            report
                .anomalies
                .iter()
                .any(|a| a.contains("pages [3, 5] have no text layer")),
            "unresolved pages must be reported: {:#?}",
            report.anomalies
        );
    }

    /// Weak map: a digital paper whose headings are too sparse for the
    /// text-layer sufficiency test still ingests.
    #[tokio::test]
    async fn digital_weak_map_ingests_locally() {
        let pages = digital_zero_cost_weak_map_pages();
        let cfg = config();
        let (built, report) =
            run_question_pipeline(&pages, &[], &cfg, &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert_eq!(built.len(), 2, "both questions extracted locally: {built:#?}");
        assert_eq!(report.deterministic, 2);
        assert!(report.quarantined.is_empty());
        assert!(built.iter().all(|q| q.content.contains("marks")));
    }

    /// A digital span whose strict gate fails keeps its real content, flagged,
    /// with the failed gate on the record.
    #[tokio::test]
    async fn digital_failed_gate_recovers_locally() {
        let pages = digital_zero_cost_failed_gate_pages();
        let cfg = config();
        let (built, report) =
            run_question_pipeline(&pages, &[], &cfg, &NullProgress, &cancel_flag())
                .await
                .unwrap();
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
        let cfg = config();
        let (built, report) =
            run_question_pipeline(&pages, &[], &cfg, &NullProgress, &cancel_flag())
                .await
                .unwrap();
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

    /// Pictures with no text layer and no layout to read (an uploaded scan
    /// image) are reported as not transcribed; nothing is invented for them.
    #[tokio::test]
    async fn pictures_with_no_text_are_reported_not_transcribed() {
        let pages: Vec<PageInput> = (0..3)
            .map(|_| PageInput {
                kind: PageInputKind::Image {
                    b64: png_b64(&gray_blank(600, 800)),
                },
                text: String::new(),
            })
            .collect();
        let (built, report) =
            run_question_pipeline(&pages, &[], &config(), &NullProgress, &cancel_flag())
                .await
                .unwrap();
        assert!(built.is_empty(), "{built:#?}");
        assert_eq!(report.text_layer, "scanned");
        assert_eq!(report.pages_not_transcribed, vec![1, 2, 3]);
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
        let all_spans = Arc::new(spans.to_vec());
        let mut cfg = config();
        cfg.text_layer_class = Some(doc_map::TextLayerClass {
            text_pages: 1,
            total_pages: 1,
            unresolved_pages: Vec::new(),
        });
        let (results, report) = extract_same_page_batch(
            &cfg,
            &span_refs,
            0,
            &page,
            &[],
            &cache,
            &all_spans,
            &cancel_flag(),
        )
        .await;
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

}
