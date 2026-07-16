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

use crate::doc_map::{self, DocumentMap, PageStructureProposal, QuestionSpan, ValidatedPageStructure};
use crate::geometry;
use crate::json_salvage::{parse_llm_json, ParseOutcome};
use crate::llm::{self, LlmClient};
use crate::validate;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

// ══════════════════════════════════════════════════════════════════════════
// Public types
// ══════════════════════════════════════════════════════════════════════════

pub struct PageInput {
    /// base64 page render (with or without data-URL prefix)
    pub b64: String,
    /// raw text layer for the same page (may be empty/corrupt)
    pub text: String,
}

pub trait Progress: Send + Sync {
    fn stage(&self, message: &str);
}

pub struct NullProgress;
impl Progress for NullProgress {
    fn stage(&self, _message: &str) {}
}

pub struct PipelineConfig {
    pub model: String,
    pub paper_name: String,
    pub subject: String,
    pub allowed_topics: Vec<String>,
    /// Where cropped diagrams are written; `None` skips image persistence
    /// (used in tests).
    pub diagrams_dir: Option<PathBuf>,
    /// Repair attempts after the first request per unit of work.
    pub max_repairs: u32,
    pub max_output_tokens: u32,
}

impl PipelineConfig {
    pub fn new(model: String, paper_name: String, subject: String) -> Self {
        Self {
            model,
            paper_name,
            subject,
            allowed_topics: Vec::new(),
            diagrams_dir: None,
            max_repairs: 2,
            max_output_tokens: 32768,
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
    pub anomalies: Vec<String>,
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

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct AiQuestion {
    question_number: Option<serde_json::Value>,
    content: Option<String>,
    marks: Option<serde_json::Value>,
    topics: Option<serde_json::Value>,
    module: Option<String>,
    is_code: Option<bool>,
    diagram_bboxes: Option<Vec<Vec<f32>>>,
    bbox_page_indexes: Option<Vec<serde_json::Value>>,
    math_snippet: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct AiQuestionPage {
    items: Vec<AiQuestion>,
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
            .or_else(|| n.as_f64().and_then(|f| if f.fract() == 0.0 { Some(f as u64) } else { None }))
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

// ══════════════════════════════════════════════════════════════════════════
// Prompts
// ══════════════════════════════════════════════════════════════════════════

fn structure_system_prompt() -> String {
    r#"You are an exam-document layout analyzer. Look at ONE page and report ONLY structural facts as a JSON object:

{
  "question_numbers_visible": [ints],
  "total_marks_footer": [question_number, marks] or null,
  "page_role": "QUESTION" | "COVER" | "INSTRUCTIONS" | "BLANK" | "ANSWER_BOOKLET" | "REFERENCE"
}

RULES:
- "question_numbers_visible": WHOLE question numbers only (AQA "03.1" counts as 3). Sub-part letters and margin digits are NOT question numbers.
- "total_marks_footer": only if a line like "(Total for Question 5 is 8 marks)" is printed on this page. Format: [5, 8]. Otherwise null.
- page_role: COVER (front cover / candidate details), INSTRUCTIONS (rubric, formula sheet given to candidates), BLANK (empty or "BLANK PAGE"), ANSWER_BOOKLET (empty lined/dotted student writing space), REFERENCE (stand-alone reference/formula table), otherwise QUESTION.
- Output ONLY the JSON object. No commentary."#
        .to_string()
}

fn extraction_system_prompt(config: &PipelineConfig, span: &QuestionSpan, allowed_topics: &[String]) -> String {
    format!(r#"You are a precise mathematical OCR engine. Output ONLY a valid JSON object of the form {{"items": [ ... ]}}.

CONTEXT: The page image(s) belong to Question {number} of the paper '{paper}'. They may also show the tail of the previous question or the head of the next one. Transcribe ONLY content that belongs to Question {number}. If nothing on these pages belongs to Question {number}, return {{"items": []}}.

Normally there is exactly ONE item. Return more than one ONLY if Question {number} visibly consists of independent numbered tasks on these pages.

EVERY item MUST have:
- "question_number": {number} (integer, exactly).
- "content": FULL transcription (never a summary). Preserve all punctuation. Separate sub-parts (a), (b), (c) with double newlines. Append the mark tag `**[X marks]**` to every sub-part that shows a mark allocation. Transcribe every sentence, including instructions to the candidate that belong to the question. Do NOT include: page headers/footers ("Question X continued", "Turn over"), the "(Total for Question X is Y marks)" footer, blank answer lines/grids, or "BLANK PAGE".
- "marks": integer total for this question's visible part, or null if unknown.
- "topics": array chosen ONLY from this list: {topics:?}. At least one. Never invent topics.
- "module": string — infer from the paper context; use "Unknown" if unclear.
- "is_code": boolean (true only for code/pseudocode questions).
- "diagram_bboxes": array of [x, y, w, h] boxes with RELATIVE 0.0-1.0 coordinates, one per visual diagram (graphs, networks, Gantt charts, trees, force diagrams, line drawings). One box per WHOLE diagram including labels. Do NOT box text, tables of text, code, database schemas, or empty grids.
- "bbox_page_indexes": array with the SAME LENGTH as diagram_bboxes — the 0-based index of the page image each box refers to.
- Insert the exact token [DIAGRAM_PLACEHOLDER] in "content" where each diagram belongs chronologically.

FORMATTING RULES:
- Wrap inline math in single $...$. Use $$...$$ ONLY for display equations on their own line.
- Tables of text/data: standard Markdown tables. Pure mathematical matrices or Simplex tableaus: LaTeX \begin{{array}} inside $$...$$. Never put $ inside array environments.
- Multiple-choice options: `a) ...`, `b) ...` separated by double newlines, WITHOUT the question number prefix.
- Code/pseudocode/SQL/identifiers: Markdown backticks, NEVER LaTeX math mode.
- AQA decimal sub-parts: render '02.1'-style parts as (a), (b), (c) and update inline cross-references accordingly. Whole-numbered MCQs are independent questions, never decimals.
- JSON ESCAPING: backslashes in LaTeX MUST be escaped (\\frac, \\theta). Unescaped backslashes break the parser and your work is discarded.
- The content MUST end with terminal punctuation or a mark tag. Never stop mid-sentence."#,
        number = span.number,
        paper = config.paper_name,
        topics = allowed_topics,
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

    // ── 1. Structure pass ───────────────────────────────────────────────────
    progress.stage("Scanning document structure…");
    let mut structures: Vec<ValidatedPageStructure> = Vec::with_capacity(pages.len());
    for (i, page) in pages.iter().enumerate() {
        cancelled(cancel)?;
        let body = llm::chat_body(
            &config.model,
            &structure_system_prompt(),
            std::slice::from_ref(&page.b64),
            None,
            200,
        );
        match client.chat(&body).await {
            Ok(resp) => match llm::message_content(&resp) {
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
                        structures.push(ValidatedPageStructure {
                            page: i,
                            questions: Vec::new(),
                            footer: None,
                            role: doc_map::PageRole::Unknown,
                        });
                    }
                },
                Err(e) => {
                    report.anomalies.push(format!(
                        "structure pass page {}: bad response shape ({}), page treated as unknown role",
                        i + 1,
                        e
                    ));
                    structures.push(ValidatedPageStructure {
                        page: i,
                        questions: Vec::new(),
                        footer: None,
                        role: doc_map::PageRole::Unknown,
                    });
                }
            },
            Err(e) => {
                report.anomalies.push(format!(
                    "structure pass page {}: API failure ({}), page treated as unknown role",
                    i + 1,
                    e
                ));
                structures.push(ValidatedPageStructure {
                    page: i,
                    questions: Vec::new(),
                    footer: None,
                    role: doc_map::PageRole::Unknown,
                });
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

    // ── 2. Document map ─────────────────────────────────────────────────────
    let page_texts: Vec<String> = pages.iter().map(|p| p.text.clone()).collect();
    let text_map = doc_map::build_map_from_text(&page_texts, pages.len());
    let struct_map = doc_map::build_map_from_structure(&structures, pages.len());

    let mut map: DocumentMap = match (text_map, struct_map) {
        (Some(mut t), Some(s)) => {
            // Text-layer footers win; cross-check and note disagreements.
            let t_nums: Vec<u32> = t.spans.iter().map(|x| x.number).collect();
            let s_nums: Vec<u32> = s.spans.iter().map(|x| x.number).collect();
            if t_nums != s_nums {
                report.anomalies.push(format!(
                    "structure disagreement: text-layer questions {:?} vs vision {:?} — trusting text layer",
                    t_nums, s_nums
                ));
            }
            if t.paper_total_marks.is_none() {
                t.paper_total_marks = s.paper_total_marks;
            }
            t.non_question_pages = s.non_question_pages;
            t
        }
        (Some(t), None) => {
            let mut t = t;
            t.non_question_pages = structures
                .iter()
                .filter(|s| !s.role.is_question_content())
                .map(|s| s.page)
                .collect();
            t
        }
        (None, Some(s)) => {
            report
                .anomalies
                .push("text layer unusable; map built from vision structure pass".to_string());
            s
        }
        (None, None) => {
            report.anomalies.push(
                "no reliable document structure — falling back to per-page extraction".to_string(),
            );
            DocumentMap::default()
        }
    };

    // Mark footers seen in the structure pass backfill text-scan gaps.
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

    if map.spans.is_empty() {
        // No reliable map → per-page legacy mode with all validators still
        // on (numbers proposed by AI, but forced plausible + monotonic).
        let mut next_allowed: u32 = 1;
        for (i, page) in pages.iter().enumerate() {
            cancelled(cancel)?;
            let role = &structures[i];
            if !role.role.is_question_content() {
                continue;
            }
            report.pages_processed += 1;
            progress.stage(&format!("Extracting page {} of {}…", i + 1, pages.len()));
            match extract_fallback_page(client, config, page, i, next_allowed, &mut report).await {
                Some(ExtractedFallback::Question(q)) => {
                    next_allowed = q.question_number + 1;
                    // A repeated number on the next page = continuation in
                    // disguise: stitch, don't duplicate.
                    if let Some(prev) = built.last_mut() {
                        if prev.question_number == q.question_number {
                            prev.content = format!("{}\n\n{}", prev.content, q.content);
                            prev.marks = validate::sum_inline_marks(&prev.content).max(prev.marks.max(0) as u32) as i32;
                            continue;
                        }
                    }
                    built.push(q);
                }
                Some(ExtractedFallback::SkipPage) => {}
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
        for (span_idx, span) in map.spans.iter().enumerate() {
            cancelled(cancel)?;
            progress.stage(&format!(
                "Extracting question {} ({} of {})…",
                span.number,
                span_idx + 1,
                total
            ));
            // Pages for this span, excluding role-filtered ones.
            let span_pages: Vec<(usize, &PageInput)> = (span.start_page..=span.end_page)
                .filter(|&pi| pi < pages.len())
                .filter(|&pi| {
                    map.non_question_pages.is_empty()
                        || !map.non_question_pages.contains(&pi)
                        || structures[pi].role == doc_map::PageRole::Blank
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

            match extract_span(client, config, span, &span_pages, &mut report).await {
                Some(q) => {
                    report.pages_processed += span_pages.len();
                    push_mark_check(span, &q, &mut report);
                    built.push(q);
                }
                None => {
                    report.quarantined.push(QuarantineEvent {
                        scope: "question".to_string(),
                        page: Some(span.start_page + 1),
                        question_number: Some(span.number),
                        reason: "failed validation and all repair attempts".to_string(),
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
/// Returns Some(AiQuestionPage) on acceptance (possibly flagged), None on
/// quarantine (reason recorded in the report by the caller).
async fn extract_span<C: LlmClient>(
    client: &C,
    config: &PipelineConfig,
    span: &QuestionSpan,
    span_pages: &[(usize, &PageInput)],
    report: &mut ImportReport,
) -> Option<BuiltQuestion> {
    let max_attempts = 1 + config.max_repairs;

    // Chunk long spans: at most 4 page images per call (your no-batching
    // constraint honored as per-chunk calls, Rust concatenates).
    const MAX_IMAGES: usize = 4;
    let chunks: Vec<&[(usize, &PageInput)]> = span_pages.chunks(MAX_IMAGES).collect();

    let mut contents: Vec<String> = Vec::new();
    let mut topics_acc: Vec<String> = Vec::new();
    let mut module_acc: Option<String> = None;
    let mut is_code_acc = false;
    let mut needs_review = false;
    let mut notes: Vec<String> = Vec::new();
    let mut ai_marks: Option<i32> = None;

    for chunk in chunks {
        let images: Vec<String> = chunk
            .iter()
            .map(|(_, p)| p.b64.clone())
            .filter(|b| !b.trim().is_empty())
            .collect();
        let raw_text: String = chunk
            .iter()
            .enumerate()
            .map(|(_k, (pi, p))| {
                if p.text.trim().is_empty() {
                    String::new()
                } else {
                    format!("RAW TEXT PAGE {}:\n{}\n\n", pi + 1, p.text)
                }
            })
            .collect();

        let system = extraction_system_prompt(config, span, &config.allowed_topics);
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
                "Transcribe Question {} from the attached page image(s).{}{}",
                span.number,
                if raw_text.is_empty() {
                    String::new()
                } else {
                    format!("\n\nReference OCR text (may be corrupt — images are authoritative):\n{}", raw_text)
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

            let resp = match client.chat(&body).await {
                Ok(r) => r,
                Err(e) => {
                    last_error = e.to_string();
                    if attempt == max_attempts {
                        break;
                    }
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

            let parsed = parse_llm_json::<AiQuestionPage>(&content);
            let (page_items, salvaged) = match parsed {
                ParseOutcome::Clean(v) => (v, false),
                ParseOutcome::Salvaged { value, dropped_tail } => {
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

            // ── Deterministic validation of the page items ────────────────
            let validation_errors = validate_span_items(&page_items, span);
            if !validation_errors.is_empty() {
                last_error = validation_errors.join("; ");
                report.repairs += 1;
                continue;
            }
            accepted = Some((page_items.items, salvaged));
            break;
        }

        let (items, salvaged) = accepted?;

        for item in items {
            let mut item_content = item.content.unwrap_or_default();

            // Cropping: sanitizer + blank guard, fully deterministic.
            if let Some(bboxes) = &item.diagram_bboxes {
                let indexes = item.bbox_page_indexes.clone().unwrap_or_default();
                for (bi, bbox) in bboxes.iter().enumerate() {
                    let page_idx = indexes
                        .get(bi)
                        .and_then(value_to_usize)
                        .filter(|&k| k < chunk.len())
                        .unwrap_or(0);
                    let page = chunk[page_idx].1;
                    let link = save_diagram(page, bbox, config, report);
                    if let Some(link) = link {
                        if item_content.contains("[DIAGRAM_PLACEHOLDER]") {
                            item_content = item_content.replacen("[DIAGRAM_PLACEHOLDER]", &link, 1);
                        } else {
                            item_content.push_str(&link);
                        }
                    }
                }
            }
            item_content = item_content.replace("[DIAGRAM_PLACEHOLDER]", "");

            if let Some(t) = item.topics {
                topics_acc.extend(value_to_topics(&t));
            }
            if module_acc.is_none() {
                module_acc = item.module;
            }
            if item.is_code == Some(true) {
                is_code_acc = true;
            }
            if let Some(m) = item.marks.as_ref().and_then(validate::value_to_marks) {
                ai_marks = Some(ai_marks.map_or(m, |existing: i32| existing.max(m)));
            }
            contents.push(item_content);
        }

        if salvaged {
            needs_review = true;
            notes.push("response truncated; content recovered up to the last complete item".to_string());
        }
    }

    // ── Assemble + content-level validation ─────────────────────────────────
    let mut content = contents.join("\n\n");
    content = validate::clean_question_content(&content);

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
    let topics_valid: Vec<String> = topics_acc
        .into_iter()
        .filter(|t| config.allowed_topics.is_empty() || config.allowed_topics.contains(t))
        .collect();

    let module = if config.subject == "Physics" {
        "Physics".to_string()
    } else if config.subject == "Computer Science" {
        "Computer Science".to_string()
    } else {
        module_acc.unwrap_or_else(|| "Unknown".to_string())
    };

    Some(BuiltQuestion {
        question_number: span.number,
        content,
        marks,
        topics: topics_valid,
        module,
        is_code: config.subject == "Computer Science" && is_code_acc,
        needs_review,
        notes,
    })
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
                    idx + 1, n, span.number
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
        if !content.trim().is_empty() && !validate::has_terminal_ending(content) {
            errors.push(format!(
                "item {} content lacks terminal punctuation (truncation suspected)",
                idx + 1
            ));
        }
        if let Some(bboxes) = &item.diagram_bboxes {
            if let Some(indexes) = &item.bbox_page_indexes {
                if indexes.len() != bboxes.len() {
                    errors.push(
                        "bbox_page_indexes length must equal diagram_bboxes length".to_string()
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

/// Crop + persist one diagram; returns the markdown link on success.
fn save_diagram(
    page: &PageInput,
    bbox: &[f32],
    config: &PipelineConfig,
    report: &mut ImportReport,
) -> Option<String> {
    let img = geometry::decode_page_image(&page.b64)?;
    let cropped = geometry::crop_diagram(&img, bbox, 40).or_else(|| {
        report.crop_rejections += 1;
        None
    })?;
    let dir = config.diagrams_dir.as_ref()?;
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join(format!("{}.png", uuid::Uuid::new_v4()));
    if cropped.save(&path).is_err() {
        report.crop_rejections += 1;
        return None;
    }
    report.diagrams_saved += 1;
    Some(format!(
        "\n\n![Diagram]({})\n\n",
        path.to_string_lossy().replace('\\', "/")
    ))
}

enum ExtractedFallback {
    Question(BuiltQuestion),
    /// Page held no NEW question (continuation/blank) — not an error.
    SkipPage,
}

/// Fallback: no map — per-page extraction, AI proposes the number but it
/// must be plausible and non-decreasing (monotonicity enforced).
async fn extract_fallback_page<C: LlmClient>(
    client: &C,
    config: &PipelineConfig,
    page: &PageInput,
    page_idx: usize,
    next_allowed: u32,
    report: &mut ImportReport,
) -> Option<ExtractedFallback> {
    let max_attempts = 1 + config.max_repairs;
    let system = format!(r#"You are a precise mathematical OCR engine. Output ONLY a valid JSON object {{"items": [ ... ]}}.

RULES:
- If this page starts a NEW question (has its own printed whole-question number), return ONE item:
  {{ "question_number": <whole number printed>, "content": "<full transcription>", "marks": int|null,
     "topics": array from {topics:?} only, "module": string, "is_code": bool,
     "diagram_bboxes": [[x,y,w,h]...] relative 0.0-1.0, "bbox_page_indexes": [0,...] }}
- If this page is a CONTINUATION of the previous question, is blank, or contains no question, return {{"items": []}}.
- Transcribe fully (never summarize). Preserve punctuation. `**[X marks]**` after each marked sub-part. Math in $...$/$$...$$. Markdown tables for text tables; \begin{{array}} only for matrices. Code in backticks, never math mode. Escape LaTeX backslashes (\\frac).
- Exclude headers/footers ("Question X continued", "Turn over", totals footers), blank answer lines/grids, "BLANK PAGE".
- Content must end with terminal punctuation or a mark tag."#,
        topics = config.allowed_topics,
    );

    let mut last_error = String::new();
    for attempt in 1..=max_attempts {
        let user_text = format!(
            "Extract the NEW question on this page (page {}), or return an empty items array if it is a continuation.{}",
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
        let body = llm::chat_body(
            &config.model,
            &system,
            std::slice::from_ref(&page.b64),
            Some(&user_text),
            config.max_output_tokens,
        );
        let resp = match client.chat(&body).await {
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
            ParseOutcome::Salvaged { value, .. } => value,
            ParseOutcome::Malformed { error } => {
                last_error = format!("invalid JSON: {}", error);
                report.repairs += 1;
                continue;
            }
        };
        if page_out.items.is_empty() {
            return Some(ExtractedFallback::SkipPage);
        }
        let mut item = page_out.items.into_iter().next().unwrap();
        let number = item
            .question_number
            .as_ref()
            .and_then(validate::value_to_question_number);
        let number = match number {
            Some(n) if n >= next_allowed.saturating_sub(1) => n,
            _ => {
                last_error = format!(
                    "implausible or backwards question number; expected ≥ {}",
                    next_allowed
                );
                continue;
            }
        };

        let mut item_content = item.content.take().unwrap_or_default();
        if let Some(bboxes) = &item.diagram_bboxes {
            for bbox in bboxes {
                if let Some(link) = save_diagram(page, bbox, config, report) {
                    if item_content.contains("[DIAGRAM_PLACEHOLDER]") {
                        item_content = item_content.replacen("[DIAGRAM_PLACEHOLDER]", &link, 1);
                    } else {
                        item_content.push_str(&link);
                    }
                }
            }
        }
        item_content = item_content.replace("[DIAGRAM_PLACEHOLDER]", "");

        let topics: Vec<String> = item
            .topics
            .as_ref()
            .map(value_to_topics)
            .unwrap_or_default()
            .into_iter()
            .filter(|t| config.allowed_topics.is_empty() || config.allowed_topics.contains(t))
            .collect();

        return Some(ExtractedFallback::Question(BuiltQuestion {
            question_number: number,
            content: validate::clean_question_content(&item_content),
            marks: item
                .marks
                .as_ref()
                .and_then(validate::value_to_marks)
                .unwrap_or(1)
                .max(1),
            topics,
            module: if config.subject == "Physics" {
                "Physics".into()
            } else if config.subject == "Computer Science" {
                "Computer Science".into()
            } else {
                item.module.unwrap_or_else(|| "Unknown".into())
            },
            is_code: config.subject == "Computer Science" && item.is_code == Some(true),
            needs_review: true,
            notes: vec!["extracted without document map (fallback mode)".to_string()],
        }));
    }
    None
}

// ══════════════════════════════════════════════════════════════════════════
// Mark-scheme pipeline
// ══════════════════════════════════════════════════════════════════════════

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
    let mut drafts: Vec<AnswerDraft> = Vec::new();
    let mut alt_count: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();

    let system = markscheme_system_prompt();
    let max_attempts = 1 + config.max_repairs;

    // Sliding windows of 3, step 2 (context for answers spanning pages).
    let window: usize = 3;
    let step: usize = 2;
    let mut start = 0usize;
    while start < pages.len() {
        cancelled(cancel)?;
        let end = (start + window).min(pages.len());
        progress.stage(&format!("Reading mark scheme pages {}–{} of {}…", start + 1, end, pages.len()));

        let images: Vec<String> = pages[start..end]
            .iter()
            .map(|p| p.b64.clone())
            .filter(|b| !b.trim().is_empty())
            .collect();
        let mut chunk_text = String::new();
        for i in start..end {
            if !pages[i].text.trim().is_empty() {
                chunk_text.push_str(&format!("RAW TEXT PAGE {}:\n{}\n\n---\n\n", i + 1, pages[i].text));
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

        for attempt in 1..=max_attempts {
            let text = if attempt == 1 {
                user_text.clone()
            } else {
                format!(
                    "{}\n\nPREVIOUS ATTEMPT FAILED VALIDATION: {}. Regenerate the complete corrected JSON.",
                    user_text, last_error
                )
            };
            let body = llm::chat_body(&config.model, &system, &images, Some(&text), config.max_output_tokens);
            let resp = match client.chat(&body).await {
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

        let answers = match accepted {
            Some(a) => a,
            None => {
                report.quarantined.push(QuarantineEvent {
                    scope: "mark-scheme-window".to_string(),
                    page: Some(start + 1),
                    question_number: None,
                    reason: format!("window pages {}–{} failed validation: {}", start + 1, end, last_error),
                });
                if end >= pages.len() {
                    break;
                }
                start += step;
                continue;
            }
        };

        report.pages_processed += end - start;

        for ans in answers {
            let q_num = match ans.question_number.as_ref().and_then(validate::value_to_question_number) {
                Some(n) => n,
                None => {
                    report.anomalies.push(format!(
                        "window {}–{}: answer without a valid question number skipped",
                        start + 1, end
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
                for (bi, bbox) in bboxes.iter().enumerate() {
                    let local = indexes
                        .get(bi)
                        .and_then(value_to_usize)
                        .filter(|&k| k < images.len());
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
                    if let Some(link) = save_diagram(&pages[start + local], bbox, config, &mut report) {
                        if md.contains("[DIAGRAM_PLACEHOLDER]") {
                            md = md.replacen("[DIAGRAM_PLACEHOLDER]", &link, 1);
                        } else {
                            md.push_str(&link);
                        }
                    }
                }
            }
            md = md.replace("[DIAGRAM_PLACEHOLDER]", "");

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

        if end >= pages.len() {
            break;
        }
        start += step;
    }

    Ok((drafts, report))
}

// ══════════════════════════════════════════════════════════════════════════
// Tests — the golden suite. Deterministic: MockLlm replays scripted model
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
                b64: String::new(),
                text: String::new(),
            })
            .collect()
    }

    fn config() -> PipelineConfig {
        let mut c = PipelineConfig::new("test-model".into(), "Unit".into(), "Mathematics".into());
        c.allowed_topics = vec!["Proof".into(), "Integration".into()];
        c.max_repairs = 2;
        c
    }

    fn cancel_flag() -> AtomicBool {
        AtomicBool::new(false)
    }

    // Three-page paper: cover, Q1 (3 marks), Q2 (4 marks), total 7.
    fn paper_pages() -> Vec<PageInput> {
        vec![
            PageInput { b64: String::new(), text: "Instructions\nAnswer ALL questions".into() },
            PageInput { b64: String::new(), text: "1. Prove the thing. (Total for Question 1 is 3 marks)".into() },
            PageInput { b64: String::new(), text: "2. Integrate this. (Total for Question 2 is 4 marks)\nTOTAL FOR PAPER IS 7 MARKS".into() },
        ]
    }

    fn structure_reply(role: &str, nums: &str, footer: &str) -> Result<serde_json::Value, LlmError> {
        ok_chat(&format!(
            r#"{{"question_numbers_visible": {}, "total_marks_footer": {}, "page_role": "{}"}}"#,
            nums, footer, role
        ))
    }

    #[tokio::test]
    async fn happy_path_full_checksum() {
        let mock = MockLlm::new(vec![
            // structure pass × 3
            structure_reply("COVER", "[]", "null"),
            structure_reply("QUESTION", "[1]", "[1, 3]"),
            structure_reply("QUESTION", "[2]", "[2, 4]"),
            // extraction span 1
            ok_chat(r#"{"items":[{"question_number":1,"content":"Prove that the thing holds. **[3 marks]**","marks":3,"topics":["Proof"],"module":"Pure"}]}"#),
            // extraction span 2
            ok_chat(r#"{"items":[{"question_number":2,"content":"Integrate $x^2$ from 0 to 2. **[4 marks]**","marks":4,"topics":["Integration"],"module":"Pure"}]}"#),
        ]);
        let pgs = paper_pages();
        let (built, report) = run_question_pipeline(&mock, &pgs, &config(), &NullProgress, &cancel_flag())
            .await
            .unwrap();

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
            structure_reply("COVER", "[]", "null"),
            structure_reply("QUESTION", "[1]", "[1, 3]"),
            structure_reply("QUESTION", "[2]", "[2, 4]"),
            // span 1: junk first, then the repair round-trip yields valid JSON
            ok_chat("sorry, I cannot help with that… not json"),
            ok_chat(r#"{"items":[{"question_number":1,"content":"Prove it fully here. **[3 marks]**","marks":3,"topics":["Proof"],"module":"Pure"}]}"#),
            // span 2 clean
            ok_chat(r#"{"items":[{"question_number":2,"content":"Integrate it. **[4 marks]**","marks":4,"topics":["Integration"],"module":"Pure"}]}"#),
        ]);
        let pgs = paper_pages();
        let (built, report) = run_question_pipeline(&mock, &pgs, &config(), &NullProgress, &cancel_flag())
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
            structure_reply("COVER", "[]", "null"),
            structure_reply("QUESTION", "[1]", "[1, 3]"),
            structure_reply("QUESTION", "[2]", "[2, 4]"),
            // span 1: model insists on question 99 — every attempt rejected.
            ok_chat(r#"{"items":[{"question_number":99,"content":"wrong. **[3 marks]**"}]}"#),
            ok_chat(r#"{"items":[{"question_number":99,"content":"wrong. **[3 marks]**"}]}"#),
            ok_chat(r#"{"items":[{"question_number":99,"content":"wrong. **[3 marks]**"}]}"#),
            // span 2 fine
            ok_chat(r#"{"items":[{"question_number":2,"content":"Integrate it. **[4 marks]**","marks":4}]}"#),
        ]);
        let pgs = paper_pages();
        let (built, report) = run_question_pipeline(&mock, &pgs, &config(), &NullProgress, &cancel_flag())
            .await
            .unwrap();
        assert_eq!(built.len(), 1);
        assert_eq!(report.quarantined.len(), 1);
        assert_eq!(report.quarantined[0].question_number, Some(1));
    }

    #[tokio::test]
    async fn truncated_mid_item_is_repaired() {
        let mock = MockLlm::new(vec![
            structure_reply("COVER", "[]", "null"),
            structure_reply("QUESTION", "[1]", "[1, 3]"),
            structure_reply("QUESTION", "[2]", "[2, 4]"),
            // span 1: truncated mid-string (no complete item → repair), then valid
            ok_chat(r#"{"items":[{"question_number":1,"content":"Prove that the thing holds completely"#),
            ok_chat(r#"{"items":[{"question_number":1,"content":"Prove that the thing holds, with steps. **[3 marks]**","marks":3}]}"#),
            // span 2
            ok_chat(r#"{"items":[{"question_number":2,"content":"Integrate it. **[4 marks]**","marks":4}]}"#),
        ]);
        let pgs = paper_pages();
        let (built, report) = run_question_pipeline(&mock, &pgs, &config(), &NullProgress, &cancel_flag())
            .await
            .unwrap();
        assert_eq!(built.len(), 2);
        assert!(report.repairs >= 1);
    }

    #[tokio::test]
    async fn truncation_after_complete_item_uses_salvage_path() {
        let mock = MockLlm::new(vec![
            structure_reply("COVER", "[]", "null"),
            structure_reply("QUESTION", "[1]", "[1, 3]"),
            structure_reply("QUESTION", "[2]", "[2, 4]"),
            // span 1: one full item then a truncated second item, then valid
            ok_chat(r#"{"items":[{"question_number":1,"content":"Prove the claim. **[3 marks]**"},{"question_number":1,"content":"cut off mid sen"#),
            ok_chat(r#"{"items":[{"question_number":1,"content":"Prove the claim. **[3 marks]**"}]}"#),
            // span 2
            ok_chat(r#"{"items":[{"question_number":2,"content":"Integrate it. **[4 marks]**","marks":4}]}"#),
        ]);
        let pgs = paper_pages();
        let (built, report) = run_question_pipeline(&mock, &pgs, &config(), &NullProgress, &cancel_flag())
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
            ok_chat(r#"{"answers":[{"question_number":1,"answer_markdown":"**(a)** Use integration to find the area of the region R = 12.5 units squared."},{"question_number":2,"answer_markdown":"Take logs of both sides then solve."}]}"#),
            // window pages 3–4 overlap: Q2 re-transcribed with noise → dup; Q3 new
            ok_chat(r#"{"answers":[{"question_number":2,"answer_markdown":"take logs of both sides and then solve."},{"question_number":3,"answer_markdown":"Differentiate implicitly to get the gradient."}]}"#),
        ]);
        let mut c = config();
        c.max_output_tokens = 4096;
        let (drafts, report) = run_markscheme_pipeline(&mock, &pgs, &c, &NullProgress, &cancel_flag())
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
        let (_drafts, report) = run_markscheme_pipeline(&mock, &pgs, &c, &NullProgress, &cancel_flag())
            .await
            .unwrap();
        assert_eq!(report.quarantined.len(), 1);
        assert!(report.quarantined[0].scope.contains("mark-scheme"));
    }
}
