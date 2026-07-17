// ── Document Map ────────────────────────────────────────────────────────────
//
// The document's skeleton is derived BEFORE any transcription, from ground
// truth the AI doesn't control:
//   * `(Total for Question N is M marks)` footers — printed deterministically
//     once per question (Edexcel/AQA); they give end-page + expected marks.
//   * `TOTAL FOR PAPER IS X MARKS` — the end-of-paper + paper checksum.
// If the text layer is too corrupt (fewer than 2 usable footers), a cheap AI
// *structure pass* (tiny schema, validated for monotonicity) builds the map
// instead. The AI then transcribes against this map — it never invents
// question numbers, merging, or continuations.
//
// PAGE/SPAN-LEVEL FALLBACK (Change 4):
// Instead of document-wide fallback, we classify each page's text-layer
// reliability independently:
//   - Reliable: clear footer found, monotonic question numbers
//   - Ambiguous: some text but no clear footer, or conflicting signals
//   - Non-question: cover, instructions, blank, answer booklet, reference
// We build spans from reliable pages, run vision only on ambiguous pages,
// and merge monotonically.

/// A regex-discovered "Total for Question …" footer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Footer {
    pub page: usize,
    pub question: u32,
    pub marks: u32,
}

/// Page-level text-layer reliability classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageReliability {
    /// Page has a clear footer and fits monotonic sequence
    Reliable,
    /// Page has text but no clear footer, or conflicting signals
    Ambiguous,
    /// Page is explicitly non-question content
    NonQuestion,
}

/// One contiguous question span, page-granular.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionSpan {
    pub number: u32,
    pub start_page: usize,
    pub end_page: usize,
    /// Marks printed in the paper footer, when known — the per-question
    /// checksum the AI's transcription is validated against.
    pub expected_marks: Option<u32>,
    /// Which pages in this span are reliable vs ambiguous
    pub reliable_pages: Vec<usize>,
    pub ambiguous_pages: Vec<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct DocumentMap {
    pub spans: Vec<QuestionSpan>,
    pub paper_total_marks: Option<u32>,
    /// Pages the structure pass determined to be non-question content
    /// (covers, instruction sheets, answer booklets, reference tables).
    pub non_question_pages: Vec<usize>,
    /// Pages that required vision structure pass (for reporting)
    pub vision_fallback_pages: Vec<usize>,
    /// Anomalies found while building the map (for the import report).
    pub anomalies: Vec<String>,
}

impl DocumentMap {
    #[allow(dead_code)]
    pub fn span_for_page(&self, page: usize) -> Option<&QuestionSpan> {
        self.spans
            .iter()
            .find(|s| s.start_page <= page && page <= s.end_page)
    }
    #[allow(dead_code)]
    pub fn span_for_question(&self, number: u32) -> Option<&QuestionSpan> {
        self.spans.iter().find(|s| s.number == number)
    }
}

// ── Text-layer scan ─────────────────────────────────────────────────────────

fn footer_regex() -> regex::Regex {
    regex::Regex::new(
        r"(?i)\(?\s*Total\s+for\s+Question\s+(\d{1,2})\s+is\s+(\d{1,2})\s+marks?\s*\)?",
    )
    .unwrap()
}

fn paper_total_regex() -> regex::Regex {
    regex::Regex::new(r"(?i)TOTAL\s+FOR\s+PAPER\s+IS\s+(\d{1,3})\s+MARKS").unwrap()
}

pub struct TextScan {
    pub footers: Vec<Footer>,
    pub paper_total: Option<u32>,
    /// Per-page reliability classification
    pub page_reliability: Vec<PageReliability>,
}

/// Scan every page's raw text layer for structural footers.
pub fn scan_text_layer(page_texts: &[String]) -> TextScan {
    let foot_re = footer_regex();
    let paper_re = paper_total_regex();
    let mut footers = Vec::new();
    let mut paper_total = None;
    let mut page_reliability = vec![PageReliability::Ambiguous; page_texts.len()];

    let instr_re = regex::Regex::new(
        r"(?i)\binstructions\b|\binformation\b|answer all questions|formulae|\bglossary\b",
    ).unwrap();
    let blank_re = regex::Regex::new(r"(?i)^\s*(blank page|this page is intentionally blank)\s*$").unwrap();
    let ref_re = regex::Regex::new(r"(?i)^\s*(formulae|data|reference|constants)\s*(sheet|table|booklet)?\s*$").unwrap();
    // AQA CS signals that a page is question content even when it contains
    // instruction boilerplate (e.g. "Answer all questions" on same page as Q1).
    let aqa_figure_re = regex::Regex::new(r"(?i)\bfig(?:ure)?\.?\s*\d+").unwrap();
    let aqa_table_re = regex::Regex::new(r"(?i)\btable\s+\d+").unwrap();
    let aqa_main_re = regex::Regex::new(r"0\s+\d{1,2}").unwrap();
    let aqa_sub_re = regex::Regex::new(r"(?m)^\s*0?\s*\d{1,2}\s*\.\s*\d+").unwrap();
    let marks_re = regex::Regex::new(r"(?i)\[\s*\d+\s*marks?").unwrap();

    for (page, text) in page_texts.iter().enumerate() {
        let mut has_footer = false;
        for cap in foot_re.captures_iter(text) {
            let q = cap[1].parse::<u32>().unwrap_or(0);
            let m = cap[2].parse::<u32>().unwrap_or(0);
            if q > 0 && m > 0 {
                footers.push(Footer {
                    page,
                    question: q,
                    marks: m,
                });
                has_footer = true;
            }
        }
        if paper_total.is_none() {
            if let Some(cap) = paper_re.captures(text) {
                paper_total = cap[1].parse::<u32>().ok().filter(|&t| t > 0);
            }
        }

        // Does this page look like a real question despite containing generic
        // instruction keywords? (critical for AQA where "Answer all questions"
        // appears on Q1 page).
        let has_question_signal = aqa_figure_re.is_match(text)
            || aqa_table_re.is_match(text)
            || aqa_main_re.is_match(text)
            || aqa_sub_re.is_match(text)
            || marks_re.is_match(text);

        // Classify page reliability
        if blank_re.is_match(text) || text.trim().is_empty() {
            page_reliability[page] = PageReliability::NonQuestion;
        } else if has_footer {
            page_reliability[page] = PageReliability::Reliable;
        } else if (instr_re.is_match(text) || ref_re.is_match(text)) && !has_question_signal {
            // Instruction / reference pages that are NOT also question pages
            page_reliability[page] = PageReliability::NonQuestion;
        } else if text.len() > 100 || has_question_signal {
            // Has substantial text but no footer - ambiguous (needs vision)
            page_reliability[page] = PageReliability::Ambiguous;
        } else {
            page_reliability[page] = PageReliability::NonQuestion;
        }
    }
    TextScan {
        footers,
        paper_total,
        page_reliability,
    }
}

/// Build spans from reliable text-layer pages only.
/// Returns (spans, reliable_page_set, anomalies)
fn build_spans_from_reliable_pages(
    scan: &TextScan,
    num_pages: usize,
) -> (Vec<QuestionSpan>, std::collections::BTreeSet<usize>, Vec<String>) {
    let mut anomalies = Vec::new();
    
    // Filter to only reliable footers
    let reliable_footers: Vec<Footer> = scan.footers.iter()
        .filter(|f| scan.page_reliability[f.page] == PageReliability::Reliable)
        .copied()
        .collect();
    
    // We do NOT require reliable_footers.len() >= 2 here, because
    // this is a hybrid map and the rest might be ambiguous pages.
    if reliable_footers.is_empty() {
        return (Vec::new(), std::collections::BTreeSet::new(), anomalies);
    }
    
    // Sort and deduplicate
    let mut footers = reliable_footers;
    footers.sort_by_key(|f| (f.page, f.question));
    footers.dedup_by_key(|f| f.question);
    
    // Check monotonicity
    let monotone = footers.windows(2).all(|w| w[1].question > w[0].question);
    if !monotone {
        anomalies.push("reliable footers not monotonic".to_string());
        return (Vec::new(), std::collections::BTreeSet::new(), anomalies);
    }
    
    let mut spans = Vec::new();
    let mut reliable_pages = std::collections::BTreeSet::new();
    
    for (i, f) in footers.iter().enumerate() {
        let end_page = f.page;
        let start_page = if i == 0 {
            estimate_first_question_start_reliable(&scan.page_reliability, end_page)
        } else {
            mid_page_start(&footers[i - 1], f)
        };
        if start_page > end_page || end_page >= num_pages {
            anomalies.push(format!("inconsistent span for Q{}", f.question));
            continue;
        }
        
        // Collect reliable and ambiguous pages in this span
        let mut span_reliable = Vec::new();
        let mut span_ambiguous = Vec::new();
        for p in start_page..=end_page {
            match scan.page_reliability[p] {
                PageReliability::Reliable => span_reliable.push(p),
                PageReliability::Ambiguous => span_ambiguous.push(p),
                PageReliability::NonQuestion => {}
            }
        }
        
        // Mark pages as used
        for p in &span_reliable { reliable_pages.insert(*p); }
        
        spans.push(QuestionSpan {
            number: f.question,
            start_page,
            end_page,
            expected_marks: Some(f.marks),
            reliable_pages: span_reliable,
            ambiguous_pages: span_ambiguous,
        });
    }
    
    (spans, reliable_pages, anomalies)
}

/// Estimate Q1 start using only reliable pages
fn estimate_first_question_start_reliable(
    page_reliability: &[PageReliability],
    first_footer_page: usize,
) -> usize {
    let mut start = 0usize;
    for p in 0..first_footer_page {
        if page_reliability[p] == PageReliability::NonQuestion {
            start = p + 1;
        }
    }
    start.min(first_footer_page)
}

/// Hybrid map building: use reliable text pages, vision for ambiguous pages.
pub fn build_hybrid_map(
    page_texts: &[String],
    structures: &[ValidatedPageStructure],
    num_pages: usize,
) -> DocumentMap {
    let mut anomalies = Vec::new();
    let scan = scan_text_layer(page_texts);
    
    // 1. Build spans from reliable text-layer pages
    let (mut spans, _reliable_pages, text_anomalies) = build_spans_from_reliable_pages(&scan, num_pages);
    anomalies.extend(text_anomalies);
    
    // 2. Identify ambiguous pages that need vision
    let ambiguous_pages: Vec<usize> = (0..num_pages)
        .filter(|&p| scan.page_reliability[p] == PageReliability::Ambiguous)
        .collect();
    
    // 3. Run vision structure on ambiguous pages only (structures already computed)
    // Merge vision info for ambiguous pages into spans
    if !ambiguous_pages.is_empty() {
        let vision_spans = build_spans_from_vision(structures, &ambiguous_pages, num_pages);
        // Merge text and vision spans
        spans = merge_spans(spans, vision_spans, &mut anomalies);
    }
    
    // 4. Collect non-question pages
    let non_question_pages: Vec<usize> = (0..num_pages)
        .filter(|&p| scan.page_reliability[p] == PageReliability::NonQuestion)
        .collect();
    
    // 5. Vision fallback pages are the ambiguous ones we actually used vision for
    let vision_fallback_pages = ambiguous_pages.clone();
    
    // Validate final spans for monotonicity
    let mut valid_spans = Vec::new();
    let mut prev_num = 0u32;
    let mut prev_end = 0usize;
    for span in spans {
        if span.number <= prev_num {
            anomalies.push(format!("non-monotonic question number {} after {}", span.number, prev_num));
            continue;
        }
        if span.start_page > span.end_page || span.end_page >= num_pages {
            anomalies.push(format!("invalid page range for Q{}", span.number));
            continue;
        }
        // Check for backward jumps
        if span.start_page < prev_end && span.start_page + 1 < prev_end {
            anomalies.push(format!("backward jump in Q{} start_page {} < prev_end {}", span.number, span.start_page, prev_end));
        }
        prev_num = span.number;
        prev_end = span.end_page;
        valid_spans.push(span);
    }
    
    DocumentMap {
        spans: valid_spans,
        paper_total_marks: scan.paper_total,
        non_question_pages,
        vision_fallback_pages,
        anomalies,
    }
}

/// Build spans from vision structure for specific pages
fn build_spans_from_vision(
    structures: &[ValidatedPageStructure],
    ambiguous_pages: &[usize],
    _num_pages: usize,
) -> Vec<QuestionSpan> {
    let mut last_seen: std::collections::BTreeMap<u32, (usize, Option<u32>)> = 
        std::collections::BTreeMap::new();
    let mut prev_max = 0u32;
    
    for p in structures {
        if !ambiguous_pages.contains(&p.page) {
            continue; // Only process ambiguous pages
        }
        for &q in &p.questions {
            if q + 5 < prev_max {
                return Vec::new(); // Hallucination signal
            }
            prev_max = prev_max.max(q);
            let e = last_seen.entry(q).or_insert((p.page, None));
            e.0 = p.page;
        }
        if let Some((q, m)) = p.footer {
            let e = last_seen.entry(q).or_insert((p.page, None));
            e.0 = p.page;
            e.1 = Some(m);
        }
    }
    // We do NOT require last_seen.len() >= 2 here, because
    // this is a hybrid map and the rest might be reliable pages.
    if last_seen.is_empty() {
        return Vec::new();
    }
    
    let mut spans = Vec::new();
    let mut next_start = 0usize;
    for (q, (end, marks)) in last_seen.iter() {
        spans.push(QuestionSpan {
            number: *q,
            start_page: next_start.min(*end),
            end_page: *end,
            expected_marks: *marks,
            reliable_pages: Vec::new(),
            ambiguous_pages: vec![*end], // Vision pages are ambiguous by definition
        });
        next_start = end + 1;
    }
    spans
}

/// Merge text-layer spans with vision spans, preferring text-layer for reliable pages
fn merge_spans(
    mut text_spans: Vec<QuestionSpan>,
    vision_spans: Vec<QuestionSpan>,
    anomalies: &mut Vec<String>,
) -> Vec<QuestionSpan> {
    // For each vision span, try to find matching text span or insert
    for vspan in vision_spans {
        // Find text span with same question number
        if let Some(idx) = text_spans.iter().position(|s| s.number == vspan.number) {
            // Merge: extend page range if vision covers more
            let tspan = &mut text_spans[idx];
            tspan.start_page = tspan.start_page.min(vspan.start_page);
            tspan.end_page = tspan.end_page.max(vspan.end_page);
            // Add vision pages to ambiguous
            for p in vspan.ambiguous_pages {
                if !tspan.ambiguous_pages.contains(&p) && !tspan.reliable_pages.contains(&p) {
                    tspan.ambiguous_pages.push(p);
                }
            }
            // Use footer marks if text didn't have them
            if tspan.expected_marks.is_none() && vspan.expected_marks.is_some() {
                tspan.expected_marks = vspan.expected_marks;
            }
        } else {
            // New question from vision only
            anomalies.push(format!("vision-only question {} found", vspan.number));
            text_spans.push(vspan);
        }
    }
    
    // Sort by question number
    text_spans.sort_by_key(|s| s.number);
    text_spans
}

/// When question N-1's footer and question N's footer are on the same page,
/// N both starts and ends there (one-page question). Otherwise N starts on
/// the page where N-1's footer is, since questions often start on the same page
/// the previous question ended.
fn mid_page_start(prev: &Footer, _cur: &Footer) -> usize {
    prev.page
}

/// Find where question 1 plausibly starts: scan pages before its footer for
/// instruction/cover content; Q1 begins after the last such page.
#[allow(dead_code)]
fn estimate_first_question_start(page_texts: &[String], first_footer_page: usize) -> usize {
    let instr_re = regex::Regex::new(
        r"(?i)\binstructions\b|\binformation\b|answer all questions|formulae|\bglossary\b",
    )
    .unwrap();
    let margin_re = regex::Regex::new(r"(?m)^\s*0?1\s*$").unwrap();
    let mut start = 0usize;
    for p in 0..first_footer_page {
        let text = &page_texts[p];
        // A page that already shows a lone "1" margin marker looks like Q1.
        if margin_re.is_match(text) {
            return p;
        }
        if instr_re.is_match(text) {
            start = p + 1;
        }
    }
    start.min(first_footer_page)
}

// ── AI structure pass (validated) ──────────────────────────────────────────

/// What the cheap per-page structure call may return.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PageStructureProposal {
    /// Whole question numbers whose content is visible on this page.
    #[serde(default)]
    pub question_numbers_visible: Vec<serde_json::Value>,
    /// Footer marks if a "Total for Question …" line is visible:
    /// [question_number, marks]. Absent otherwise.
    #[serde(default)]
    pub total_marks_footer: Option<Vec<serde_json::Value>>,
    /// One of QUESTION / COVER / INSTRUCTIONS / BLANK / ANSWER_BOOKLET /
    /// REFERENCE — page-only classification, no question content asked.
    #[serde(default)]
    pub page_role: Option<String>,
}

pub struct ValidatedPageStructure {
    pub page: usize,
    pub questions: Vec<u32>,
    pub footer: Option<(u32, u32)>,
    pub role: PageRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageRole {
    Question,
    Cover,
    Instructions,
    Blank,
    AnswerBooklet,
    Reference,
    Unknown,
}

impl PageRole {
    pub fn is_question_content(self) -> bool {
        matches!(
            self,
            PageRole::Question | PageRole::Blank | PageRole::Unknown
        )
    }
}

/// Validate one structure proposal. Returns (normalized, list of violations).
/// Violations are for the report; nothing here silently trusts bad data.
pub fn validate_structure_proposal(
    page: usize,
    proposal: PageStructureProposal,
    all_valid_page_count: usize,
) -> (ValidatedPageStructure, Vec<String>) {
    let mut violations = Vec::new();
    let mut questions: Vec<u32> = proposal
        .question_numbers_visible
        .iter()
        .filter_map(crate::validate::value_to_question_number)
        .collect();
    if questions.len() != proposal.question_numbers_visible.len() {
        violations.push(format!(
            "page {}: dropped implausible question number(s) from structure pass",
            page + 1
        ));
    }
    questions.sort_unstable();
    questions.dedup();

    let footer = proposal.total_marks_footer.and_then(|pair| {
        if pair.len() == 2 {
            let q = crate::validate::value_to_question_number(&pair[0]);
            let m = crate::validate::value_to_marks(&pair[1]);
            if let (Some(q), Some(m)) = (q, m) {
                return Some((q, m.max(0) as u32));
            }
        }
        violations.push(format!(
            "page {}: malformed total_marks_footer ignored",
            page + 1
        ));
        None
    });

    let role = match proposal
        .page_role
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_uppercase()
        .as_str()
    {
        "QUESTION" | "QUESTION_PAPER" => PageRole::Question,
        "COVER" | "COVER_PAGE" => PageRole::Cover,
        "INSTRUCTIONS" | "INSTRUCTION" => PageRole::Instructions,
        "BLANK" => PageRole::Blank,
        "ANSWER_BOOKLET" => PageRole::AnswerBooklet,
        "REFERENCE" => PageRole::Reference,
        other => {
            if !other.is_empty() {
                violations.push(format!("page {}: unknown page_role '{}'", page + 1, other));
            }
            PageRole::Unknown
        }
    };

    let _ = all_valid_page_count; // reserved for future cross-page checks

    (
        ValidatedPageStructure {
            page,
            questions,
            footer,
            role,
        },
        violations,
    )
}

/// Fold validated per-page structure into a DocumentMap when the text-layer
/// scan failed (corrupt PDFs). Numbers must form a non-decreasing sequence
/// across pages — the single most effective anti-hallucination check there
/// is for page-by-page proposals.
#[allow(dead_code)]
pub fn build_map_from_structure(
    pages: &[ValidatedPageStructure],
    num_pages: usize,
) -> Option<DocumentMap> {
    // Record last page each question appears on with a footer.
    let mut last_seen: std::collections::BTreeMap<u32, (usize, Option<u32>)> =
        std::collections::BTreeMap::new();
    let mut prev_max = 0u32;
    for p in pages {
        for &q in &p.questions {
            if q + 5 < prev_max {
                // A backwards jump this big is a hallucination signal — if
                // everything downstream is unreliable, give up gracefully.
                return None;
            }
            prev_max = prev_max.max(q);
            let e = last_seen.entry(q).or_insert((p.page, None));
            e.0 = p.page;
        }
        if let Some((q, m)) = p.footer {
            let e = last_seen.entry(q).or_insert((p.page, None));
            e.0 = p.page;
            e.1 = Some(m);
        }
    }
    if last_seen.len() < 2 {
        return None;
    }

    let mut spans = Vec::new();
    let mut next_start = 0usize;
    for (q, (end, marks)) in last_seen.iter() {
        spans.push(QuestionSpan {
            number: *q,
            start_page: next_start.min(*end),
            end_page: *end,
            expected_marks: *marks,
            reliable_pages: Vec::new(),
            ambiguous_pages: vec![*end], // All structure-pass pages are ambiguous
        });
        next_start = end + 1;
    }
    let _ = num_pages;

    let non_question_pages = pages
        .iter()
        .filter(|p| !p.role.is_question_content())
        .map(|p| p.page)
        .collect();

    Some(DocumentMap {
        spans,
        paper_total_marks: None,
        non_question_pages,
        vision_fallback_pages: pages.iter().map(|p| p.page).collect(),
        anomalies: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(pages: &[&str]) -> Vec<String> {
        pages.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn edexcel_footers_build_spans() {
        let t = texts(&[
            "Centre Number\nInstructions\nAnswer ALL questions",
            "1. Question one text (a) part - this page contains enough text to be considered ambiguous instead of non-question. Let's pad it out with some more text to be absolutely sure it exceeds one hundred characters.",
            "middle of Q1 (Total for Question 1 is 5 marks)\n3. second question",
            "second continues (Total for Question 2 is 6 marks)",
            "TOTAL FOR PAPER IS 11 MARKS",
        ]);
        let map = build_hybrid_map(&t, &[], 5);
        assert_eq!(map.spans.len(), 2);
        assert_eq!(map.spans[0].number, 1);
        assert_eq!(map.spans[0].expected_marks, Some(5));
        assert_eq!(map.spans[0].end_page, 2);
        assert_eq!(map.spans[1].start_page, 2); // Now starts on the same page as Q1's footer
        assert_eq!(map.paper_total_marks, Some(11));
        // Cover page detected as front matter:
        assert_eq!(map.spans[0].start_page, 1);
    }

    #[test]
    fn one_page_questions_same_page() {
        let t = texts(&[
            "1. first (Total for Question 1 is 3 marks) 2. second (Total for Question 2 is 4 marks)",
            "3. third (Total for Question 3 is 2 marks)",
        ]);
        let map = build_hybrid_map(&t, &[], 2);
        assert_eq!(map.spans[1].number, 2);
        assert_eq!(map.spans[1].start_page, 0); // same page as Q1's footer
        assert_eq!(map.spans[2].start_page, 0); // Now starts on the same page as Q2's footer
    }

    #[test]
    fn corrupt_text_layer_falls_back() {
        let t = texts(&["garbled !@#$%^", "more garbled"]);
        let map = build_hybrid_map(&t, &[], 2);
        assert!(map.spans.is_empty());
    }

    #[test]
    fn structure_pass_validates_and_builds() {
        let mk = |page, qs: Vec<u32>, foot: Option<(u32, u32)>, role| ValidatedPageStructure {
            page,
            questions: qs,
            footer: foot,
            role,
        };
        let pages = vec![
            mk(0, vec![], None, PageRole::Cover),
            mk(1, vec![1], None, PageRole::Question),
            mk(2, vec![1, 2], Some((1, 5)), PageRole::Question),
            mk(3, vec![2], Some((2, 6)), PageRole::Question),
        ];
        let map = build_map_from_structure(&pages, 4).unwrap();
        assert_eq!(map.spans.len(), 2);
        assert_eq!(map.spans[0].number, 1);
        assert_eq!(map.spans[0].end_page, 2);
        assert_eq!(map.spans[0].expected_marks, Some(5));
        assert_eq!(map.spans[1].start_page, 3);
        assert_eq!(map.non_question_pages, vec![0]);
    }

    #[test]
    fn structure_pass_rejects_massive_backwards_jumps() {
        let mk = |page, qs: Vec<u32>| ValidatedPageStructure {
            page,
            questions: qs,
            footer: None,
            role: PageRole::Question,
        };
        let pages = vec![mk(0, vec![40]), mk(1, vec![1])]; // 40 → 1 hallucination
        assert!(build_map_from_structure(&pages, 2).is_none());
    }

    #[test]
    fn proposal_validation_normalizes() {
        let (v, violations) = validate_structure_proposal(
            0,
            PageStructureProposal {
                question_numbers_visible: vec![serde_json::json!(3), serde_json::json!("03.1")],
                total_marks_footer: Some(vec![serde_json::json!(3), serde_json::json!(8)]),
                page_role: Some("question".into()),
            },
            5,
        );
        assert_eq!(v.questions, vec![3]); // "03.1" refused, not "31"
        assert_eq!(v.footer, Some((3, 8)));
        assert_eq!(v.role, PageRole::Question);
        assert_eq!(violations.len(), 1);
    }
}
