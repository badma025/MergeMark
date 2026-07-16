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

/// A regex-discovered "Total for Question …" footer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Footer {
    pub page: usize,
    pub question: u32,
    pub marks: u32,
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
}

#[derive(Debug, Clone, Default)]
pub struct DocumentMap {
    pub spans: Vec<QuestionSpan>,
    pub paper_total_marks: Option<u32>,
    /// Pages the structure pass determined to be non-question content
    /// (covers, instruction sheets, answer booklets, reference tables).
    pub non_question_pages: Vec<usize>,
    /// Anomalies found while building the map (for the import report).
    pub anomalies: Vec<String>,
}

impl DocumentMap {
    pub fn span_for_page(&self, page: usize) -> Option<&QuestionSpan> {
        self.spans
            .iter()
            .find(|s| s.start_page <= page && page <= s.end_page)
    }
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
}

/// Scan every page's raw text layer for structural footers.
pub fn scan_text_layer(page_texts: &[String]) -> TextScan {
    let foot_re = footer_regex();
    let paper_re = paper_total_regex();
    let mut footers = Vec::new();
    let mut paper_total = None;

    for (page, text) in page_texts.iter().enumerate() {
        for cap in foot_re.captures_iter(text) {
            let q = cap[1].parse::<u32>().unwrap_or(0);
            let m = cap[2].parse::<u32>().unwrap_or(0);
            if q > 0 && m > 0 {
                footers.push(Footer {
                    page,
                    question: q,
                    marks: m,
                });
            }
        }
        if paper_total.is_none() {
            if let Some(cap) = paper_re.captures(text) {
                paper_total = cap[1].parse::<u32>().ok().filter(|&t| t > 0);
            }
        }
    }
    TextScan {
        footers,
        paper_total,
    }
}

/// Build a map from regex footers. Requires ≥ 2 footers with strictly
/// increasing question numbers — anything less trustworthy returns `None`
/// and the caller falls back to the AI structure pass.
pub fn build_map_from_text(page_texts: &[String], num_pages: usize) -> Option<DocumentMap> {
    let scan = scan_text_layer(page_texts);
    if scan.footers.len() < 2 {
        return None;
    }

    // Deduplicate (page, question) pairs and enforce strict monotonicity.
    let mut footers = scan.footers.clone();
    footers.sort_by_key(|f| (f.page, f.question));
    footers.dedup_by_key(|f| f.question);
    let monotone = footers.windows(2).all(|w| w[1].question > w[0].question);
    if !monotone {
        return None;
    }

    let mut spans: Vec<QuestionSpan> = Vec::new();
    for (i, f) in footers.iter().enumerate() {
        let end_page = f.page;
        let start_page = if i == 0 {
            // Q1 may start several pages before its footer (covers,
            // instruction sheets sit in front). Heuristic: the last
            // instruction-looking page before the first footer + 1.
            estimate_first_question_start(page_texts, end_page)
        } else {
            mid_page_start(&footers[i - 1], f)
        };
        if start_page > end_page || end_page >= num_pages {
            return None; // inconsistent — let the structure pass arbitrate
        }
        spans.push(QuestionSpan {
            number: f.question,
            start_page,
            end_page,
            expected_marks: Some(f.marks),
        });
    }

    Some(DocumentMap {
        spans,
        paper_total_marks: scan.paper_total,
        non_question_pages: Vec::new(),
        anomalies: Vec::new(),
    })
}

/// When question N-1's footer and question N's footer are on the same page,
/// N both starts and ends there (one-page question). Otherwise N starts on
/// the page after N-1's footer page.
fn mid_page_start(prev: &Footer, cur: &Footer) -> usize {
    if prev.page == cur.page {
        cur.page
    } else {
        prev.page + 1
    }
}

/// Find where question 1 plausibly starts: scan pages before its footer for
/// instruction/cover content; Q1 begins after the last such page.
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
            "1. Question one text (a) part",
            "middle of Q1 (Total for Question 1 is 5 marks)\n3. second question",
            "second continues (Total for Question 2 is 6 marks)",
            "TOTAL FOR PAPER IS 11 MARKS",
        ]);
        let map = build_map_from_text(&t, 5).unwrap();
        assert_eq!(map.spans.len(), 2);
        assert_eq!(map.spans[0].number, 1);
        assert_eq!(map.spans[0].expected_marks, Some(5));
        assert_eq!(map.spans[0].end_page, 2);
        assert_eq!(map.spans[1].start_page, 3);
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
        let map = build_map_from_text(&t, 2).unwrap();
        assert_eq!(map.spans[1].number, 2);
        assert_eq!(map.spans[1].start_page, 0); // same page as Q1's footer
        assert_eq!(map.spans[2].start_page, 1);
    }

    #[test]
    fn corrupt_text_layer_falls_back() {
        let t = texts(&["garbled !@#$%^", "more garbled"]);
        assert!(build_map_from_text(&t, 2).is_none());
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
