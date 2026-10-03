// ── Document Map ────────────────────────────────────────────────────────────
//
// The text-layer map, used when the geometric layout map (`layout.rs`) finds
// no questions. The document's skeleton is derived BEFORE any transcription,
// from what the paper prints:
//   * `(Total for Question N is M marks)` footers — printed once per question
//     (Edexcel/AQA); they give end-page + expected marks.
//   * `TOTAL FOR PAPER IS X MARKS` — the end-of-paper + paper checksum.
//   * question headings, for papers without footers.
// Each page's text-layer reliability is classified independently:
//   - Reliable: clear footer found, monotonic question numbers
//   - Ambiguous: some text but no clear footer, or conflicting signals
//   - Non-question: cover, instructions, blank, answer booklet, reference
// Spans are built from reliable and ambiguous pages and merged monotonically.

/// A regex-discovered "Total for Question …" footer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Footer {
    pub page: usize,
    pub question: u32,
    pub marks: u32,
    /// Approximate vertical position of the footer on the page as a
    /// fraction 0.0 (top) – 1.0 (bottom). Used to clip the previous
    /// question off before the footer on pages that also contain the
    /// next question's start (e.g. Q2 begins mid-page below Q1's footer).
    /// For the text-layer scan we use a byte-offset-within-page proxy
    /// (good enough to decide "near the bottom / middle / near the top").
    pub y_frac: f32,
}

/// A question heading detected on a page text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuestionHeading {
    pub page: usize,
    pub number: u32,
    /// Vertical position of the heading (top of the question), 0.0–1.0.
    pub y_frac: f32,
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

/// One contiguous question span, page-granular with optional sub-page
/// vertical bounds.
///
/// Phase 1: when a page contains multiple questions (end of Q_N, Q_{N+1},
/// start of Q_{N+2}) the structure pass records the vertical range each
/// question occupies on each shared page as a fraction of page height
/// (0.0 = top, 1.0 = bottom). The extractor then clips the page image
/// to that band before sending it to the model, eliminating cross-
/// question bleed without asking the AI to "just know" where questions
/// start and end. When y-fractions are `None` the span is treated as
/// covering the full page (the Phase 0 behaviour), which is always
/// safe.
#[derive(Debug, Clone, PartialEq)]
pub struct QuestionSpan {
    pub number: u32,
    pub start_page: usize,
    pub end_page: usize,
    /// Optional vertical clip on the first page of the span, 0.0–1.0.
    /// `Some(low)` means "the question starts `low` of the way down";
    /// `Some(1.0)` would mean no clip (use `None` for that).
    pub start_y_frac: Option<f32>,
    /// Optional vertical clip on the last page of the span, 0.0–1.0.
    /// `Some(high)` means "the question ends `high` of the way down";
    /// interior pages are shown in full.
    pub end_y_frac: Option<f32>,
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
    /// Pages the text scan determined to be non-question content
    /// (covers, instruction sheets, answer booklets, reference tables).
    pub non_question_pages: Vec<usize>,
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

// ── Text-layer scan ─────────────────────────────────────────────────────────
//
// Phase 1: support per-question-total footers across the major UK/international
// boards rather than just the Edexcel/AQA wording. Patterns covered:
//   * Edexcel/AQA:   "(Total for Question N is M marks)"
//   * OCR-A / OCR-B: "Total [for Question N] M marks" / "Total: M"
//   * CAIE/CIE 9702: "[Total: M]" / "(M marks)" alone (weak signal)
//   * WJEC/Eduqas:   "Total [M] marks"
//   * IB:            "[M marks]" placed at the end of a question (weak)
// We keep them ordered from strongest (explicit question numbering) to weakest
// so we always prefer an identified footer over an ambiguous one.
//
// We also scan for question-HEADINGS ("1.", "1)", "Q1", "Question 1") in the
// text layer. These give us the y-positions of question starts on pages that
// contain multiple questions (short-answer / MCQ pages), which is what lets
// the extractor clip the page image to just the question it's transcribing.
use std::collections::BTreeMap;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy)]
struct FooterPattern {
    re: &'static str,
    has_question_num: bool,
}
const FOOTER_PATTERNS: &[FooterPattern] = &[
    FooterPattern {
        // Edexcel / AQA classic
        re: r"(?i)\(?\s*Total\s+for\s+Question\s+(\d{1,2})\s+is\s+(\d{1,2})\s+marks?\s*\)?",
        has_question_num: true,
    },
    FooterPattern {
        // OCR / Edexcel equals sign variant: "(Total for Question 1 = 8 marks)"
        re: r"(?i)\(?\s*Total\s+for\s+Question\s+(\d{1,2})\s*(?:is\s*|=)\s*(\d{1,2})\s*marks?\s*\)?",
        has_question_num: true,
    },
    FooterPattern {
        // OCR / AQA variants
        re: r"(?i)\(?\s*Total\s+(?:for\s+Question\s+(\d{1,2})\s+)?(?:is\s+)?(\d{1,2})\s+marks?\s*\)?",
        has_question_num: true,
    },
    FooterPattern {
        // Cambridge CIE / WJEC / Eduqas / IB short forms: "[Total: 8]", "(Total 8 marks)", "[Total marks: 10]"
        re: r"(?i)[\[\(]\s*Total\s*(?:marks\s*)?:?\s*(\d{1,2})\s*(?:marks?)?\s*[\]\)]",
        has_question_num: false,
    },
];

static FOOTER_REGEXES: LazyLock<Vec<(regex::Regex, bool)>> = LazyLock::new(|| {
    FOOTER_PATTERNS
        .iter()
        .map(|p| (regex::Regex::new(p.re).unwrap(), p.has_question_num))
        .collect()
});

static PAPER_TOTAL_REGEX: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)(?:TOTAL\s+(?:FOR|MARKS\s+FOR)?\s+(?:THIS\s+)?PAPER\s*(?:IS|:|=)?\s*|MAXIMUM\s+MARK\s*:\s*)(\d{1,3})\s*(?:MARKS)?").unwrap()
});

/// Self-declared question count ("There are 12 questions in this question
/// paper"). Used only as an upper-bound cross-check on isolated bare-number
/// heading candidates, never to force or truncate a paper's questions.
static QUESTION_COUNT_REGEX: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(?i)there are\s+(\d{1,3})\s+questions").unwrap());

/// Regex for question HEADINGS (whole questions, not parts) at the start of
/// a line / block. Accepts:
///   "1." "1)" "1]" "1–" "1-" "1 " with optional bold "**1.**",
///   "Q1" "Q.1" "Q 1", "Question 1" / "Question 1(a)", "QUESTION 1"
/// but NOT:
///   * AQA decimal part numbers "03.1" (handled separately via a leading 0
///     guard; sub-part digits are filtered).
///   * decimal quantities like "3.5 V" or dates "2024." (required after-digit
///     punctuation that looks like a label).
///   * part labels (a)/(b)/(i) — those never begin with 1+ digits at line
///     start followed by a period/closing paren without a letter.
/// Element symbols accepted in glued nuclide notation ("27Mg", "3He").
///
/// Shared with the deterministic carver (`deterministic::glued_nuclide_prefix`)
/// so the document map and the span carver agree on when a heading may carry an
/// isotope straight after its number — the AQA Q31 layout
/// ("box 3 1 27Mg 12 can decay …").
pub(crate) const NUCLIDE_ELEMENTS: &str =
    "He|Ne|Ar|Kr|Xe|Rn|F|Cl|Br|I|O|S|Te|N|P|As|Sb|C|Si|Ge|Sn|B|Al|Ga|In|Be|Mg|Ca|Sr|Ba|Li|Na|K|Rb|Cs|U|Th|Pu";

pub(crate) static QUESTION_HEADING_REGEX: LazyLock<regex::Regex> = LazyLock::new(|| {
    // Tolerates AQA's spaced margin padding (e.g. "0 7" for question 7, "1 0" for 10).
    // The digits may be separated by spaces — but ONLY same-line whitespace is
    // allowed anywhere inside the match. Letting `\s` match \r\n used to glue
    // a lone number on one line to a single letter on the next ("\n2\r\nF" from
    // force labels "2F"/"4E" in physics diagrams), injecting phantom headings.
    //
    // Two AQA extraction quirks handled here:
    //   * The "Do not write outside the box" margin boilerplate sometimes
    //     extraction-glues onto the question line: "box 2 4 In a resistor…".
    //     An optional literal `box ` prefix recovers those headings.
    //   * Physics isotopes straight after the number ("3 1 27Mg 12 can decay…")
    //     start with a digit, so a plain `(?:\D|$)` tail rejects them. Allow a
    //     trailing isotope (mass number + element symbol) as well.
    regex::Regex::new(&format!(
        r"(?m)(?:^|\n)[ \t]*(?:\*+)?[ \t]*(?:(?:box|Section\s+[A-Z0-9]+)[ \t]+)?(?i:Q(?:uestion)?\.?[ \t]*)?(?:\*+)?[ \t]*0*[ \t]*([1-9](?:[ \t]*\d){{0,2}})(?:\*+)?[ \t]*(?:[\.\)\]\-–—:]|[ \t]+|$)(?:[^\d\r\n]|\r?\n|$|\d{{1,3}}\s*(?:{})\b)",
        NUCLIDE_ELEMENTS
    ))
    .unwrap()
});

static INSTR_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)(?:^|\n)\s*(?:instructions?\s*(?:to\s+candidates?)?|answer\s+all\s+questions|glossary)(?:\s|$|:)",
    ).unwrap()
});

static FORMULAE_SHEET_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)(?:^|\n)\s*(?:formulae?|data|constants|relationships?)\s*(?:sheet|booklet|table|page)?\s*$",
    ).unwrap()
});

static BLANK_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\b(blank page|this page is intentionally blank|there are no questions printed on this page|do not write on this page)\b").unwrap()
});

static REF_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)^\s*(formulae|data|reference|constants)\s*(sheet|table|booklet)?\s*$").unwrap()
});

static AQA_FIGURE_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\bfig(?:ure)?\.?\s*\d+").unwrap()
});

static AQA_TABLE_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\btable\s+\d+").unwrap()
});

static AQA_MAIN_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"0\s+\d{1,2}").unwrap()
});

static AQA_SUB_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?m)^\s*0?\s*\d{1,2}\s*\.\s*\d+").unwrap()
});

static MARKS_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\[\s*\d+\s*marks?").unwrap()
});

pub struct TextScan {
    pub footers: Vec<Footer>,
    pub paper_total: Option<u32>,
    /// Per-page reliability classification
    pub page_reliability: Vec<PageReliability>,
    /// Per-page question-heading hints (from regex on the text layer).
    /// Coarse y_frac uses byte offset within the extracted text as a
    /// proxy for vertical position — good enough to detect that a page
    /// contains more than one question, which is what drives the sub-page
    /// split when no reliable footers are present.
    pub headings: Vec<QuestionHeading>,
}

/// How much machine-readable text a document's PDF text layer carries, for
/// the import report: "digital" when ANY page carries extracted text (pages
/// without text are reported as unresolved), "scanned" when none does - a
/// genuine scan or image-only file. Every document is read on this machine
/// either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextLayerClass {
    /// Pages carrying extracted text.
    pub text_pages: usize,
    pub total_pages: usize,
    /// Pages with no extracted text, in page order.
    pub unresolved_pages: Vec<usize>,
}

impl TextLayerClass {
    /// True when the document carries extracted text on any page.
    pub fn is_digital(&self) -> bool {
        self.text_pages > 0
    }

    /// True for a genuine scan / image-only input: no extracted text anywhere.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_scanned_only(&self) -> bool {
        self.text_pages == 0
    }

    /// Pages with no extracted text layer.
    pub fn unresolved_pages(&self) -> &[usize] {
        &self.unresolved_pages
    }
}

/// A page carries extracted text when ANY non-whitespace character is present.
///
/// There is deliberately NO threshold: a complete one-mark question such as
/// "1. Solve x=2. [1]" - or a maths-only stem with no prose at all - is text.
/// Watermark, furniture and OCR-noise pages count as text too.
pub fn page_has_extracted_text(text: &str) -> bool {
    !text.trim().is_empty()
}

/// Classify the document's text layer from page text alone (for the report).
/// No question mapping, no PDF re-render, no threshold of any kind: one page
/// with extracted text makes the document "digital".
pub fn classify_text_layer(page_texts: &[String]) -> TextLayerClass {
    let mut text_pages = 0usize;
    let mut unresolved_pages = Vec::new();
    for (page, text) in page_texts.iter().enumerate() {
        if page_has_extracted_text(text) {
            text_pages += 1;
        } else {
            unresolved_pages.push(page);
        }
    }
    TextLayerClass {
        text_pages,
        total_pages: page_texts.len(),
        unresolved_pages,
    }
}

/// A heading that opens a REAL question: the heading is followed by at least 30
/// characters of content on the same line. Cover furniture such as
/// "1 hour 45 minutes 9PH0/02" matches the heading pattern but has no question
/// content after it.
fn has_strong_first_page_question(text: &str) -> bool {
    for cap in QUESTION_HEADING_REGEX.captures_iter(text) {
        let Some(full) = cap.get(0) else { continue };
        let end = full.end().min(text.len());
        let after = &text[end..];
        let line_rest = after.split('\n').next().unwrap_or("");
        if line_rest.trim().chars().count() >= 30 {
            return true;
        }
    }
    false
}

/// Front-matter evidence for page 0.
///
/// A real cover sheet is rubric-dominated ("Instructions", "Information",
/// "Advice", "Time:", ...). A first page that opens a REAL question (heading
/// plus substantial same-line content) is CONTENT: a one-page paper, or a paper
/// whose Q1 starts on page 1, must not be discarded. Only explicit front-matter
/// evidence skips the page.
fn is_front_matter_page(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    let rubric_markers = [
        "instructions",
        "answer all questions",
        "information",
        "advice",
        "time:",
        "examiner",
        "do not write",
    ]
    .iter()
    .filter(|marker| lower.contains(**marker))
    .count();
    let strong_question = has_strong_first_page_question(text);
    // A genuine question on the first page beats ordinary rubric wording.
    if strong_question && rubric_markers < 3 {
        return false;
    }
    // Rubric-dominated sheets (three or more cover markers) and pages with any
    // explicit rubric heading stay covers.
    if rubric_markers >= 1 {
        return true;
    }
    // Nothing question-like and nothing rubric-like: only very short pages are
    // treated as front matter.
    !strong_question && trimmed.len() < 300
}

/// Scan every page's raw text layer for structural footers AND question
/// headings.
pub fn scan_text_layer(page_texts: &[String]) -> TextScan {
    let mut footers = Vec::new();
    let mut headings = Vec::new();
    // Bare numbers that sit alone on their line are ambiguous: they may be a
    // printed page number, a table/figure value, or a genuine heading printed
    // above a diagram (CIE). They are resolved against the question sequence
    // after the whole document has been scanned.
    let mut lone_candidates = Vec::new();
    let mut paper_total = None;
    // Parse the self-declared question count over ALL pages (including the
    // cover, which is skipped by the classification loop below).
    let declared_question_count: Option<u32> = page_texts.iter().find_map(|t| {
        QUESTION_COUNT_REGEX
            .captures(t)
            .and_then(|cap| cap[1].parse::<u32>().ok())
            .filter(|&n| n > 0)
    });
    let mut page_reliability = vec![PageReliability::Ambiguous; page_texts.len()];

    // Phase 1b: instruction/cover detection used to match ANY page containing
    // the words "information" or "formulae", which fired constantly on
    // physics papers ("use the information in Figure 3…", "the following
    // formulae may be used…") and caused pages to be marked NonQuestion
    // even when they carried real question content. The cover/instruction
    // pages we actually want to skip are short, rubric-heavy sheets whose
    // content is DOMINATED by instruction wording AND have no question
    // signals. We detect them with:
    //   * a stricter regex that only matches the rubric phrases themselves
    //     ("answer all questions", "instructions to candidates", "information"
    //     only as a heading line, "formulae" only as a "formulae sheet"
    //     reference), AND
    //   * a page-length guard: real question pages almost always exceed 300
    //     characters in the text layer, while cover/instruction pages are
    //     short.
    // Phase 1c: "information" is removed from instr_re entirely. Physics
    // questions constantly wrap sentences so that "information" starts a
    // line ("Use the information\nin Figure 3…"). Require both "answer all
    // questions" / "instructions" / "glossary" AND a short page, and match
    // them only at line start. The line-end anchor (\s|$|:) keeps us from
    // matching partial words like "instructional".
    // Phase 2: image-only PDF detection. When the entire text layer across
    // all pages totals fewer than 100 characters, the PDF is almost certainly
    // a scanned/image-only document with no usable text layer. In that case,
    // pages with empty text must NOT be classified as NonQuestion — they are
    // unread scans, not blank pages, and are reported as Ambiguous.
    let total_text_len: usize = page_texts.iter().map(|t| t.trim().len()).sum();
    let is_image_only = total_text_len < 100;

    for (page, raw_text) in page_texts.iter().enumerate() {
        let text_normalized = raw_text.replace("\r\n", "\n").replace('\r', "");
        let text = &text_normalized;
        // Ignore cover page (page 0) and extra back-matter pages
        // Page 0 is USUALLY front matter, but that must be proven: a one-page
        // paper (or a paper whose question 1 starts on the first page) keeps
        // its question.
        let is_cover = page == 0 && is_front_matter_page(text);
        let is_extra_page = text.contains("Additional page") 
            || text.contains("There are no questions printed")
            || text.contains("Copyright information");

        if is_cover || is_extra_page {
            page_reliability[page] = PageReliability::NonQuestion;
            continue;
        }

        let page_len = text.len().max(1) as f32;
        let mut has_footer = false;

        // Run every footer pattern; strong (numbered) patterns win over
        // weak (numberless) ones.
        let mut best_footer: Option<Footer> = None;
        for (re, has_qn) in FOOTER_REGEXES.iter() {
            for cap in re.captures_iter(text) {
                let m = cap.get(0).unwrap();
                let y_frac = (m.start() as f32 / page_len).clamp(0.0, 1.0);
                let (q, mk) = if *has_qn {
                    let q = cap.get(1).and_then(|s| s.as_str().parse::<u32>().ok()).unwrap_or(0);
                    let mk = cap.get(2).and_then(|s| s.as_str().parse::<u32>().ok()).unwrap_or(0);
                    (q, mk)
                } else {
                    // Weak pattern — no question number, just "[Total: M]"
                    let mk = cap.get(1).and_then(|s| s.as_str().parse::<u32>().ok()).unwrap_or(0);
                    (0u32, mk)
                };
                if mk == 0 { continue; }
                let candidate = Footer { page, question: q, marks: mk, y_frac };
                match best_footer {
                    None => best_footer = Some(candidate),
                    Some(cur) => {
                        // Prefer numbered footer over numberless; on tie
                        // prefer the one lowest on the page.
                        let cur_strong = cur.question > 0;
                        let new_strong = q > 0;
                        if (new_strong && !cur_strong)
                            || (new_strong == cur_strong && candidate.y_frac > cur.y_frac) {
                            best_footer = Some(candidate);
                        }
                    }
                }
            }
        }
        if let Some(f) = best_footer {
            footers.push(f);
            has_footer = f.question > 0;
        }

        // Collect question headings, including MCQ / short-answer pages.
        // PHASE FIX: multi-layer filter prevents false positives from marks tags,
        // quantity strings, sub-part formats, and page numbers.
        for cap in QUESTION_HEADING_REGEX.captures_iter(text) {
            let full = cap.get(0).unwrap();
            let raw_num_str = cap.get(1).unwrap().as_str();
            // Safe byte boundaries that are guaranteed to be on valid UTF-8
            // char boundaries, even when the regex match interacts with
            // multi-byte Unicode characters (e.g. HAIR SPACE \u{200a}) in the
            // extracted text layer.
            let safe_start = text.ceil_char_boundary(full.start());
            let safe_end = text.ceil_char_boundary(full.end());

            // Skip continuation headers like "Question 1 continued"
            let slice_end = text.floor_char_boundary(safe_start.saturating_add(40).min(text.len()));
            let heading_slice = &text[safe_start..slice_end];
            if heading_slice.to_ascii_lowercase().contains("continued") {
                continue;
            }

            // --- FILTER 1: Spaced sub-part format ("01 5" -> Q1, never 15) ---
            // AQA prints sub-parts as "01 5" (zero-padded main number, space,
            // sub number). But AQA also prints TWO-DIGIT question numbers with
            // a space between the digits: "1 0" = Q10, "2 5" = Q25, "3 1" = Q31.
            // Disambiguate:
            //   * second token "0"  -> must be a two-digit question number
            //     (sub-part numbers are never 0): "1 0" = Q10.
            //   * a leading zero ATTACHED to the first token in the raw match
            //     (i.e. "01 5", zero directly before the digit with no space)
            //     -> genuine spaced sub-part: keep first token.
            //   * otherwise ("1 5") -> two-digit question number Q15.
            let trimmed_raw = raw_num_str.trim();
            let tokens: Vec<&str> = trimmed_raw.split_whitespace().collect();
            let (question_num_str, is_spaced_subpart) = if tokens.len() == 2
                && tokens[0].chars().all(|c| c.is_ascii_digit())
                && tokens[1].chars().all(|c| c.is_ascii_digit())
            {
                let second_is_zero = tokens[1] == "0";
                // Look at the characters immediately before the capture group
                // in the full match: "01 5" has a '0' directly attached to the
                // first digit (no whitespace between them).
                let group_start = full.as_str().find(raw_num_str).unwrap_or(0);
                let before = &full.as_str()[..group_start];
                let attached_leading_zero = before
                    .chars()
                    .last()
                    .map(|c| c == '0')
                    .unwrap_or(false);
                if second_is_zero || !attached_leading_zero {
                    // Two-digit question number ("1 0" -> 10, "1 5" -> 15).
                    (trimmed_raw, false)
                } else {
                    // AQA "01 5" = Q1 sub-part 5. Only the first token is the heading.
                    (tokens[0], true)
                }
            } else {
                (trimmed_raw, false)
            };

            // --- FILTER 2: Marks-tag proximity ("[30 marks]" must not become Q30) ---
            // Only the SAME LINE as the number can be its mark allocation. A
            // 20-char window that spills onto the previous line saw the prior
            // question's "[2 marks]" tag and skipped the next heading — fatal
            // for one-question-per-page layouts (AQA GCSE Further Maths).
            let line_start = text[..safe_start]
                .rfind('\n')
                .map(|i| i + 1)
                .unwrap_or(0);
            let context = &text[line_start..safe_end];
            let near_marks_tag = MARKS_RE.is_match(context);
            if near_marks_tag && tokens.len() <= 1 && !is_spaced_subpart {
                continue; // Skip number that is clearly a mark allocation.
            }

            // --- FILTER 3: Clean, parse, and range-check ---
            let cleaned_num = question_num_str.replace(" ", "");
            let y_frac = (full.start() as f32 / page_len).clamp(0.0, 1.0);
            if let Ok(n) = cleaned_num.parse::<u32>() {
                if n > 0 && n <= 1000 { // Plausible question number range; supports both single papers and large multi-question compilations.
                    let chars_remaining = text.len() - safe_end;
                    // A genuine heading can sit near the bottom of a short page
                    // (AQA GCSE Further Maths prints one question per page, so
                    // the "N …" heading is the last content line). Only require
                    // that *something* follows the number on the page.
                    if chars_remaining > 2 {
                        // Edexcel-style heading pattern: "1 A bicycle…"
                        // A number followed by an uppercase letter then a
                        // lowercase word is a strong real-question signal.
                        let matched = full.as_str();
                        let trailing_char = matched.trim_end().chars().last().unwrap_or(' ');
                        let next_is_word = text[safe_end..]
                            .chars()
                            .skip_while(|c| c.is_whitespace())
                            .next()
                            .map(|c| c.is_alphabetic())
                            .unwrap_or(false);
                        let edexcel_pattern = trailing_char.is_ascii_uppercase() && next_is_word;

                        // A bare number that occupies its whole line (nothing but
                        // whitespace after it before the newline) is a printed
                        // page number, a figure/table label, or other layout
                        // debris — not a question heading. Real headings always
                        // continue on the same line ("12 Charon…", "1 (a) …",
                        // "5. A manufacturer…"). Answer booklets restart their
                        // printed page numbers, so the `is_likely_page_number`
                        // guard cannot catch these; this line-locality check is
                        // board- and document-agnostic.
                        // Anchor on where the *number* ends, not the whole
                        // regex match: the trailing alternation consumes the
                        // newline, which made the page-number line look like it
                        // continued onto the next line.
                        let number_end = cap.get(1).unwrap().end();
                        let line_rest = {
                            let tail = &text[number_end..];
                            let end = tail
                                .find('\n')
                                .map(|i| number_end + i)
                                .unwrap_or(text.len());
                            text[number_end..end].trim()
                        };
                        let lone_on_line = line_rest.is_empty();

                        let raw_digits = cap.get(1).unwrap().as_str();
                        let has_space = raw_digits.contains(' ') || raw_digits.contains('\t');
                        // Leading zero from AQA's margin padding ("0 1", "01")
                        // — a zero that appears BEFORE the captured digits, not
                        // one inside the number itself. The old check
                        // `full.as_str().contains('0')` fired for "10"/"20"/"30",
                        // letting printed page numbers 10/20/30 through as
                        // phantom question headings.
                        let group_off = full.as_str().find(raw_digits).unwrap_or(0);
                        let has_zero = full.as_str()[..group_off].contains('0');
                        let has_q = full.as_str().to_lowercase().contains('q');
                        // A "Q5" token followed on the next line by a lowercase
                        // PROSE continuation ("Q5\nis a. …") is a margin
                        // cross-reference, not a question heading (CIE 2022 Q4
                        // was being displaced by such a token). Lowercase alone
                        // is NOT proof: a genuine formula heading ("Q1\nx = …")
                        // must survive, so require a known sentence connector.
                        let q_cross_reference = has_q && {
                            let mut next = "";
                            for line in text[number_end..].lines() {
                                let t = line.trim();
                                if !t.is_empty() {
                                    next = t;
                                    break;
                                }
                            }
                            let first_word: String = next
                                .chars()
                                .take_while(|c| c.is_ascii_alphabetic())
                                .collect::<String>()
                                .to_ascii_lowercase();
                            let starts_lower = next.chars().next().map(|c| c.is_ascii_lowercase()).unwrap_or(false);
                            starts_lower
                                && matches!(
                                    first_word.as_str(),
                                    "is" | "and" | "or" | "the" | "of" | "to" | "for" | "where"
                                        | "which" | "so" | "then" | "hence" | "when" | "with"
                                )
                        };
                        let has_period = full.as_str().contains('.');
                        let has_colon = full.as_str().contains(':');
                        let has_subpart = full.as_str().contains('(') || full.as_str().contains('[');
                        // AQA prints a page marker like "*02*" (and the run
                        // header "*JUN217408201*"). A bare starred digit run is
                        // a page/code marker, never a question heading.
                        let trimmed_match = full.as_str().trim();
                        let is_page_marker = trimmed_match.starts_with('*')
                            && trimmed_match.ends_with('*')
                            && trimmed_match
                                .trim_matches(['*', ' '])
                                .chars()
                                .all(|c| c.is_ascii_digit());
                        // Margin-marker form: spaced digits ("1 0") or leading
                        // zero ("0 1"). These are strong question signals that
                        // survive the quantity check below — physics MCQ pages
                        // are so dense with units that a 40-char unit scan
                        // nukes every real heading on the page.
                        let has_margin_form = has_space || has_zero;

                        // --- FILTER 4: Reject quantity/unit patterns ---
                        // Only fires for BARE numbers (no margin form): a unit
                        // must begin IMMEDIATELY after the match (within ~6
                        // chars). The old scan of 40 chars after the heading
                        // dropped real question headings on unit-dense pages.
                        let is_quantity = if has_margin_form || edexcel_pattern {
                            false
                        } else {
                            // Only inspect the SAME LINE as the number. Using
                            // `safe_end` here read the whole next line, so a
                            // heading like "5" followed by a diagram label "A"
                            // was misread as the quantity "5 A" (5 amperes) and
                            // dropped — losing the real CIE question.
                            let after_line = text[number_end..]
                                .split('\n')
                                .next()
                                .unwrap_or("");
                            let after: String = after_line.chars().take(6).collect();
                            let after_trim = after.trim_start();
                            let units = ["kg", "g ", "m ", "cm", "mm", "V ", "N ", "J ", "Pa", "Hz", "kJ", "W ", "A ", "C ", "s "];
                            units.iter().any(|u| after_trim.starts_with(u))
                        };

                        // --- FILTER 5: Page-number guard ---
                        // AQA prints page numbers at the very bottom (y_frac > 0.85). On blank pages, y_frac is 0.0 but text.len() is small.
                        // We avoid dropping real questions by checking AQA's padding conventions (leading zeros, spaces between digits).
                        let is_likely_page_number = (n as usize) == page + 1 || (n as usize) == page + 2;
                        let looks_like_real_question = has_margin_form || has_q || has_period || has_colon || has_subpart;

                        let at_bottom = y_frac > 0.85 && chars_remaining < 150;
                        let at_top = y_frac < 0.15 && safe_end < 150;
                        // A printed page number always sits ALONE on its line.
                        // Requiring line-locality stops the off-by-one
                        // `page + 1`/`page + 2` guard from rejecting a real
                        // heading whose text follows on the same line
                        // ("5 y = …" on the one-question-per-page AQA FM paper).
                        let is_printed_page_number = is_likely_page_number
                            && lone_on_line
                            && !looks_like_real_question
                            && !edexcel_pattern
                            && (at_bottom || at_top || text.len() < 300);

                        // --- FILTER 6: bare-number analysis ---
                        // Real AQA question numbers ALWAYS carry margin form:
                        // zero padding ("0 8"), spaced digits ("1 2"), a Q
                        // prefix, or a period ("12."). A BARE number at line
                        // start is a heading only when the text continues with
                        // a capitalised word ("12 Charon…"). Everything else
                        // is physics debris:
                        //   * "28 °" / "15 °C"   — angles / temperatures
                        //   * "2 r" / "3 x"      — formula fragments (2πr, ∛x)
                        //   * "7 ion" / "7 nucleus" — isotope tails (₇Li)
                        //   * "6 +" / "1 ="      — isotope / equation debris
                        //   * "20 N"             — quantities
                        let is_bare = !has_margin_form && !has_q && !has_period;
                        let mut bare_reject = false;
                        if is_bare {
                            if trailing_char.is_ascii_alphabetic() {
                                // Keep only "12 C" from "12 Charon…": an
                                // UPPERCASE letter whose word continues.
                                if !(trailing_char.is_ascii_uppercase() && next_is_word) {
                                    bare_reject = true;
                                }
                            } else if trailing_char == '(' && next_is_word {
                                // Edexcel can place the first sub-part marker
                                // immediately after the question number:
                                // "11 (a) ...". The opening parenthesis is
                                // part of the heading, not formula debris.
                                bare_reject = false;
                            } else if !trailing_char.is_ascii_digit() {
                                bare_reject = true; // "=", "+", "°", "(" …
                            }
                        }

                        // --- FILTER 7: isotope notation AT the heading site ---
                        // "20 Ne" / "27Al" at line start is a nuclide, not a
                        // question heading. Only the match + 3 following chars
                        // are inspected — a page-wide check used to kill real
                        // headings when the question TEXT mentioned e.g. "10 C".
                        // Only applies to bare numbers: margin-form headings
                        // ("0 8 In which…", "3 1 27Mg…") are real questions.
                        // Convert byte indices to char indices for safe slicing
                        let match_len_chars = full.as_str().chars().count();
                        let heading_window: String = text[safe_start..]
                            .chars()
                            .take(match_len_chars + 3) // +3 chars after match
                            .collect();
                        // A bare Edexcel heading such as "14 Information ..."
                        // can begin with the one-letter element symbol "I".
                        // The uppercase-letter + following lowercase-word
                        // pattern is stronger evidence of a heading than the
                        // ambiguous one-letter isotope interpretation.
                        let is_isotope =
                            is_bare && !edexcel_pattern && heading_is_isotope(n, &heading_window);

                        // Degree symbol: a number immediately followed by "°"
                        // is an angle/temperature, never a question heading.
                        let ends_in_degree = trailing_char == '°';

                        let hard_reject = is_quantity
                            || is_printed_page_number
                            || is_isotope
                            || ends_in_degree
                            || is_page_marker
                            || q_cross_reference;
                        if !hard_reject {
                            let heading = QuestionHeading { page, number: n, y_frac };
                            // Bare numbers that are alone on their line, or that
                            // failed the bare-number evidence test (e.g. a
                            // heading whose text starts with a lowercase formula
                            // token, "5 y = …"), are ambiguous: they are kept as
                            // gap-fill candidates and accepted only when they
                            // continue the strong heading set.
                            if (is_bare && lone_on_line) || bare_reject {
                                lone_candidates.push(heading);
                            } else {
                                headings.push(heading);
                            }
                        }
                    }
                }
            }
        }

        if paper_total.is_none() {
            if let Some(cap) = PAPER_TOTAL_REGEX.captures(text) {
                paper_total = cap[1].parse::<u32>().ok().filter(|&t| t > 0);
            }
        }

        let has_question_signal = AQA_FIGURE_RE.is_match(text)
            || AQA_TABLE_RE.is_match(text)
            || AQA_MAIN_RE.is_match(text)
            || AQA_SUB_RE.is_match(text)
            || MARKS_RE.is_match(text)
            || headings.iter().any(|h| h.page == page)
            || lone_candidates.iter().any(|h| h.page == page);

        // Phase 1b: tighten NonQuestion classification. A page is front
        // matter ONLY if (a) it's blank, OR (b) ALL of:
        //   * it matches an instruction/reference regex (rubric/formula sheet),
        //   * it has NO question signal (no headings, no marks, no figures,
        //     no AQA margin numbers, no sub-parts), AND
        //   * EITHER it is short (<300 chars of text, typical for a cover/
        //     instruction page) OR the formulae-sheet regex matches a line.
        // This prevents false positives on physics pages that say "use the
        // information in Figure 3…" or list "the following formulae" in a
        // real question.
        let is_short_rubric = text.trim().len() < 300;
        let is_formulae_sheet = FORMULAE_SHEET_RE.is_match(text) && !MARKS_RE.is_match(text);
        let instr_hit = INSTR_RE.is_match(text);
        let ref_hit = REF_RE.is_match(text) || is_formulae_sheet;

        if is_image_only {
            page_reliability[page] = PageReliability::Ambiguous;
        } else if (BLANK_RE.is_match(text) && !has_question_signal) || text.trim().is_empty() {
            page_reliability[page] = PageReliability::NonQuestion;
        } else if has_footer {
            page_reliability[page] = PageReliability::Reliable;
        } else if (instr_hit || ref_hit) && !has_question_signal && (is_short_rubric || is_formulae_sheet) {
            page_reliability[page] = PageReliability::NonQuestion;
        } else if text.len() > 100 || has_question_signal {
            page_reliability[page] = PageReliability::Ambiguous;
        } else {
            page_reliability[page] = PageReliability::NonQuestion;
        }
    }

    // Filter out heading candidates that appear on the same page as a reliable footer for question K,
    // but with question number > K and y_frac <= footer.y_frac.
    // A question K+1 cannot start before question K's footer on the same page.
    headings.retain(|h| {
        for f in &footers {
            if f.question > 0 && h.number > f.question && f.page == h.page && h.y_frac <= f.y_frac {
                return false;
            }
        }
        true
    });

    TextScan {
        footers,
        paper_total,
        page_reliability,
        headings: canonicalize_headings(resolve_lone_headings(
            headings,
            lone_candidates,
            declared_question_count,
        )),
    }
}

/// Decide which isolated bare-number candidates are genuine question headings.
///
/// A number alone on its line is ambiguous. Genuine examples (CIE prints the
/// question number above a diagram) CONTINUE the question sequence; printed
/// page numbers and stray table/figure values do not. We therefore walk all
/// headings in reading order and accept a lone candidate only when its number
/// is at (or within a tiny gap of) the next expected question. Strong headings
/// (with punctuation, margin padding, a `Q` prefix, or trailing text) are
/// always kept and advance the sequence.
fn resolve_lone_headings(
    mut strong: Vec<QuestionHeading>,
    lone: Vec<QuestionHeading>,
    declared_question_count: Option<u32>,
) -> Vec<QuestionHeading> {
    if lone.is_empty() {
        return strong;
    }
    let strong_numbers: std::collections::BTreeSet<u32> =
        strong.iter().map(|h| h.number).collect();
    let max_strong = strong_numbers.iter().next_back().copied().unwrap_or(0);
    fn reading_order(a: &QuestionHeading, b: &QuestionHeading) -> std::cmp::Ordering {
        a.page
            .cmp(&b.page)
            .then_with(|| a.y_frac.partial_cmp(&b.y_frac).unwrap_or(std::cmp::Ordering::Equal))
    }
    let last_strong = strong
        .iter()
        .max_by(|a, b| reading_order(a, b))
        .copied()
        .unwrap_or(QuestionHeading { page: 0, number: 0, y_frac: 0.0 });
    strong.sort_by(reading_order);
    let mut lone_sorted = lone;
    lone_sorted.sort_by(reading_order);

    let mut merged: Vec<(QuestionHeading, bool)> =
        Vec::with_capacity(strong.len() + lone_sorted.len());
    let mut si = 0usize;
    let mut li = 0usize;
    while si < strong.len() || li < lone_sorted.len() {
        let take_lone = match (strong.get(si), lone_sorted.get(li)) {
            (Some(s), Some(l)) => reading_order(l, s) == std::cmp::Ordering::Less,
            (None, Some(_)) => true,
            _ => false,
        };
        if take_lone {
            merged.push((lone_sorted[li], true));
            li += 1;
        } else {
            merged.push((strong[si], false));
            si += 1;
        }
    }

    let mut next_expected = 1u32;
    let mut out = Vec::with_capacity(merged.len());
    for (heading, is_lone) in merged {
        if is_lone {
            // A declared question count constrains only AMBIGUOUS bare-number
            // candidates — never a strong heading. A wrong/absent declaration
            // must not erase real questions (e.g. a nonconsecutive later
            // section numbered above the declaration).
            if let Some(declared) = declared_question_count {
                if heading.number > declared {
                    continue;
                }
            }
            if strong_numbers.is_empty() {
                // No strong headings to anchor on: keep only candidates that
                // continue the running question sequence.
                if heading.number < next_expected
                    || heading.number.saturating_sub(next_expected) > 2
                {
                    continue;
                }
            } else if strong_numbers.contains(&heading.number) {
                // Already a strong question (page number/duplicate): drop.
                continue;
            } else if heading.number > max_strong {
                // Above the strong range: only a genuine FINAL lone heading is
                // allowed — exactly max+1 AND positioned after the last strong
                // heading. Everything else (page numbers, table/figure values)
                // is rejected.
                let after_last_strong =
                    reading_order(&heading, &last_strong) == std::cmp::Ordering::Greater;
                if heading.number != max_strong + 1 || !after_last_strong {
                    continue;
                }
            }
        }
        if heading.number >= next_expected {
            next_expected = heading.number + 1;
        }
        out.push(heading);
    }
    out
}

/// Keep only the longest plausible question-heading sequence.
///
/// PDF text layers often contain question-like numbers from answer booklets,
/// diagrams, and mark annotations after the actual paper. A simple numeric
/// deduplication cannot distinguish those from real headings: for example,
/// Q24 between Q7 and Q8 poisons the document map, while repeated Q2/Q3 on
/// later answer pages expands their spans across the rest of the document.
///
/// The real question sequence is ordered in document position and normally
/// advances by one. A gap of up to three is allowed because the sufficiency
/// gate already tolerates two missing headings. The longest increasing
/// subsequence preserves legitimate papers with skipped or missed headings
/// without assuming a fixed number of questions or an exam board.
fn canonicalize_headings(mut headings: Vec<QuestionHeading>) -> Vec<QuestionHeading> {
    if headings.len() < 2 {
        return headings;
    }

    headings.sort_by(|a, b| {
        a.page
            .cmp(&b.page)
            .then_with(|| a.y_frac.partial_cmp(&b.y_frac).unwrap_or(std::cmp::Ordering::Equal))
    });

    // Anchor to Q1 when present so an equally long answer-booklet
    // subsequence beginning at Q2/Q3 cannot win.
    let start_idx = headings.iter().position(|heading| heading.number == 1).unwrap_or(0);
    let candidate_headings = &headings[start_idx..];
    if candidate_headings.len() < 2 {
        return candidate_headings.to_vec();
    }

    let n = candidate_headings.len();
    let mut lengths = vec![1usize; n];
    let mut predecessors = vec![None; n];

    for i in 0..n {
        for j in 0..i {
            let number_gap = candidate_headings[i].number.saturating_sub(candidate_headings[j].number);
            if candidate_headings[j].number >= candidate_headings[i].number || number_gap > 3 {
                continue;
            }

            let candidate_length = lengths[j] + 1;
            if candidate_length > lengths[i] {
                lengths[i] = candidate_length;
                predecessors[i] = Some(j);
            }
        }
    }

    let mut end = 0;
    for i in 1..n {
        if lengths[i] > lengths[end] {
            end = i;
        }
    }

    let mut selected = Vec::with_capacity(lengths[end]);
    let mut current = Some(end);
    while let Some(index) = current {
        selected.push(candidate_headings[index]);
        current = predecessors[index];
    }
    selected.reverse();
    selected
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

    let mut spans = Vec::new();
    let mut reliable_pages = std::collections::BTreeSet::new();

    // Phase 1b: we NO LONGER return early when reliable_footers is empty.
    // Pure MCQ / short-answer papers (some boards' Paper 1, Edexcel MCQ
    // sections, IB Paper 1) print NO "Total for Question N is M marks"
    // footers at all — they only use per-question [1 mark] tags. For
    // those papers the text layer still has question headings; we just
    // skip the footer-driven span building and rely on
    // append_text_only_short_answer_spans below.
    if !reliable_footers.is_empty() {
        // Sort into reading order, then keep only the FIRST footer for each
        // question number. Exam papers that ship a separate answer booklet
        // repeat every "Total for Question N is M marks" footer; if both copies
        // survive, the number sequence goes 1..N,1..N and fails the monotonic
        // check below, discarding every reliable footer and forcing (worse)
        // heading-only page-granular carving. The question paper's footer comes
        // first in reading order, so it wins.
        let mut footers = reliable_footers;
        footers.sort_by_key(|f| (f.page, f.question));
        let mut seen_questions = std::collections::BTreeSet::new();
        footers.retain(|f| seen_questions.insert(f.question));

        // Check monotonicity. When footers are non-monotonic we can't
        // trust them; skip footer spans and fall through to heading-
        // only carving instead of returning an empty Vec (which used to
        // push the pipeline into full per-page fallback).
        let monotone = footers.windows(2).all(|w| w[1].question > w[0].question);
        if !monotone {
            anomalies.push("reliable footers not monotonic — heading-only carving will be used".to_string());
        } else {
            for (i, f) in footers.iter().enumerate() {
                let end_page = f.page;
                let start_page = if i == 0 {
                    estimate_first_question_start_reliable(&scan.page_reliability, end_page)
                } else {
                    detect_page_split(&footers[i - 1], f, &scan.headings)
                };
                if start_page > end_page || end_page >= num_pages {
                    anomalies.push(format!("inconsistent span for Q{}", f.question));
                    continue;
                }

                // Phase 1: infer vertical clips from headings + footer position.
                let (start_y_frac, end_y_frac) =
                    infer_y_clips(scan, f.question, start_page, end_page, f.y_frac);

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

                for p in &span_reliable {
                    reliable_pages.insert(*p);
                }

                spans.push(QuestionSpan {
                    number: f.question,
                    start_page,
                    end_page,
                    start_y_frac,
                    end_y_frac,
                    expected_marks: Some(f.marks),
                    reliable_pages: span_reliable,
                    ambiguous_pages: span_ambiguous,
                });
            }
        }
    }

    // Detect questions that appear on pages WITHOUT a dedicated footer
    // (common on MCQ / short-answer pages where 3–6 questions share a
    // page and the board prints inline [1 mark] tags instead of "Total
    // for Question"). For any heading whose number isn't already covered
    // by a footer span, carve out a span from that heading's y to the
    // next heading's y on the same page (or end of page).
    append_text_only_short_answer_spans(scan, &mut spans, &mut anomalies, false);

    (spans, reliable_pages, anomalies)
}

/// Infer (start_y_frac, end_y_frac) for a text-layer-derived span.
///
/// * `start_y_frac`: set when the question's heading on its first page is
///   not at the top (i.e. another question occupies the top portion of
///   that page, as happens when Q1's footer and Q2's heading share a page).
/// * `end_y_frac`: set to the footer's y when the footer is above the
///   bottom of the page (leaving room for a following question).
/// The byte-offset proxy is rough but good enough for "don't take lines from
/// the *next* question" — the sanitizer still rejects content outside the
/// span's number, so bleeding a few lines is not a correctness hazard.
fn infer_y_clips(
    scan: &TextScan,
    question: u32,
    start_page: usize,
    end_page: usize,
    footer_y_frac: f32,
) -> (Option<f32>, Option<f32>) {
    let start_y = scan
        .headings
        .iter()
        .filter(|h| h.page == start_page && h.number == question)
        .map(|h| h.y_frac)
        .fold(None::<f32>, |acc, y| {
            // Pick the lowest plausible heading (earliest on page isn't
            // always right if the page starts mid-prev-question; we
            // actually want the first heading of THIS question. Without
            // ground truth we take the lowest y after a small top margin
            // to avoid picking up running headers).
            let y_clamped = y.clamp(0.02, 0.98);
            match acc {
                None => Some(y_clamped),
                Some(cur) if (y_clamped - cur).abs() < f32::EPSILON => Some(cur),
                // If multiple headings for the same question appear (page
                // wrap), prefer the one HIGHEST on the start page.
                Some(cur) => Some(cur.min(y_clamped)),
            }
        });

    // Check if there are following headings on the same end page
    let has_following_heading_on_end_page = scan
        .headings
        .iter()
        .any(|h| h.page == end_page && h.number > question && h.y_frac > footer_y_frac);

    // If the footer sits in the bottom 30% of its page and no following question starts there,
    // we show the whole page. If it sits higher up or has following headings, clip at footer.
    let end_y = if footer_y_frac < 0.7 || has_following_heading_on_end_page {
        Some((footer_y_frac + 0.04).clamp(0.0, 1.0))
    } else {
        None
    };

    // Don't bother with a start clip if the heading is at the top of the
    // page (margin of error).
    let start_y = start_y.filter(|y| *y > 0.05);

    (start_y, end_y)
}

/// Carve out spans for short-answer / MCQ questions detected via
/// question-number headings on the text layer but never picked up by a
/// "Total for Question …" footer. These spans are page-granular (they
/// can't cross page boundaries without footers to anchor them) and carry
/// tight y-clips so each short-question crop contains exactly one question.
fn append_text_only_short_answer_spans(
    scan: &TextScan,
    spans: &mut Vec<QuestionSpan>,
    anomalies: &mut Vec<String>,
    include_ambiguous: bool,
) {
    // Group headings by page.
    let mut by_page: BTreeMap<usize, Vec<QuestionHeading>> = BTreeMap::new();
    for h in &scan.headings {
        by_page.entry(h.page).or_default().push(*h);
    }
    for headings in by_page.values_mut() {
        headings.sort_by(|a, b| a.y_frac.partial_cmp(&b.y_frac).unwrap());
        headings.dedup_by(|a, b| a.number == b.number && (a.y_frac - b.y_frac).abs() < 0.05);
    }

    let mut ordered_headings = scan.headings.clone();
    ordered_headings.sort_by(|a, b| {
        a.page
            .cmp(&b.page)
            .then_with(|| a.y_frac.partial_cmp(&b.y_frac).unwrap_or(std::cmp::Ordering::Equal))
    });

    let existing_numbers: std::collections::BTreeSet<u32> =
        spans.iter().map(|s| s.number).collect();

    for (&page, headings) in &by_page {
        // No unconditional page-0 skip: a first-page question reaches here as
        // Ambiguous/Reliable (the scan only marks a genuine cover page as
        // NonQuestion), and the reliability guard below still keeps real covers
        // and blank pages out.
        if scan.page_reliability[page] == PageReliability::NonQuestion
            || (!include_ambiguous && scan.page_reliability[page] != PageReliability::Reliable)
        {
            continue;
        }
        // Build horizontal bands: each heading starts a band that ends
        // at the next heading (or 1.0).
        for h in headings.iter() {
            if existing_numbers.contains(&h.number) {
                continue;
            }
            // Skip if this heading falls INSIDE another span's vertical
            // band on this page (cross-page long questions).
            let inside_other_span = spans.iter().any(|s| {
                if s.number == h.number {
                    return false;
                }
                if page < s.start_page || page > s.end_page {
                    return false;
                }
                // Compute the band this span actually occupies on `page`.
                let lo = if page == s.start_page {
                    s.start_y_frac.unwrap_or(0.0)
                } else {
                    0.0
                };
                let hi = if page == s.end_page {
                    s.end_y_frac.unwrap_or(1.0)
                } else {
                    1.0
                };
                // Heading is inside the span if its y sits within [lo, hi).
                h.y_frac >= lo - 0.02 && h.y_frac < hi
            });
            if inside_other_span {
                // Likely a cross-reference or a sub-part marker inside a
                // long question's band — don't carve out a new span.
                continue;
            }

            let next_heading = ordered_headings
                .iter()
                .position(|candidate| {
                    candidate.page == h.page
                        && candidate.number == h.number
                        && (candidate.y_frac - h.y_frac).abs() < 0.0001
                })
                .and_then(|position| ordered_headings.get(position + 1));
            let end_page = match next_heading {
                Some(next) if next.page > page => next.page.saturating_sub(1),
                _ => page,
            };
            let end_y = match next_heading {
                Some(next) if next.page == page => Some(next.y_frac - 0.005),
                _ => None,
            };
            let start_y = h.y_frac - 0.005;
            anomalies.push(format!(
                "text-heading-only question {} on page {} (short-answer/MCQ page) — using y-clips {:.2}–{:.2}",
                h.number,
                page + 1,
                start_y,
                end_y.unwrap_or(1.0),
            ));
            spans.push(QuestionSpan {
                number: h.number,
                start_page: page,
                end_page,
                start_y_frac: Some(start_y.clamp(0.0, 1.0)),
                end_y_frac: end_y.map(|y| y.clamp(0.0, 1.0)),
                expected_marks: None,
                reliable_pages: (page..=end_page)
                    .filter(|p| scan.page_reliability[*p] == PageReliability::Reliable)
                    .collect(),
                ambiguous_pages: (page..=end_page)
                    .filter(|p| scan.page_reliability[*p] == PageReliability::Ambiguous)
                    .collect(),
            });
        }
    }

    spans.sort_by(|a, b| {
        let p = a.start_page.cmp(&b.start_page);
        if p != std::cmp::Ordering::Equal { return p; }
        a.number.cmp(&b.number)
    });
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

/// Validate final spans for monotonicity. Loose guard: backward = always bad;
/// gap > 500 = almost certainly hallucinated outlier; everything else allowed.
/// Dense MCQ pages (8, 9, 10, 11) must not trigger false jump alarms.
fn finalize_spans(mut spans: Vec<QuestionSpan>, anomalies: &mut Vec<String>) -> Vec<QuestionSpan> {
    let mut valid_spans = Vec::new();
    let mut expected_max_q = 0u32;

    for mut span in spans.drain(..) {
        if expected_max_q > 0 && span.number <= expected_max_q {
            anomalies.push(format!("dropped backwards/duplicate question Q{} (expected > {})", span.number, expected_max_q));
            continue;
        }
        if expected_max_q > 0 && span.number > expected_max_q + 500 {
            anomalies.push(format!("dropped likely hallucinated jump to Q{} (gap from {} exceeds 500)", span.number, expected_max_q));
            continue;
        }

        span.start_page = span.start_page.min(span.end_page);
        expected_max_q = expected_max_q.max(span.number);
        valid_spans.push(span);
    }
    valid_spans
}

/// Build the document map from the PDF text layer alone: the reliable-footer
/// and heading-only span builders (including ambiguous pages), then the
/// monotonicity validation. Used when the geometric layout map finds no
/// questions.
pub fn build_text_only_map(_page_texts: &[String], num_pages: usize, scan: &TextScan) -> DocumentMap {
    let mut anomalies = Vec::new();
    let (mut spans, _reliable_pages, text_anomalies) =
        build_spans_from_reliable_pages(scan, num_pages);
    anomalies.extend(text_anomalies);
    // Ambiguous pages are included deliberately: there is no second opinion
    // to defer to, and a page whose footer regex missed is still a page of
    // the paper.
    append_text_only_short_answer_spans(scan, &mut spans, &mut anomalies, true);

    let non_question_pages: Vec<usize> = (0..num_pages)
        .filter(|&p| scan.page_reliability[p] == PageReliability::NonQuestion)
        .collect();

    DocumentMap {
        spans: finalize_spans(spans, &mut anomalies),
        paper_total_marks: scan.paper_total,
        non_question_pages,
        anomalies,
    }
}

/// Narrow isotope check used at heading-scan time: only fires when the
/// isotope pattern appears AT the heading site itself (the match plus a few
/// following characters), never elsewhere on the page. The page-wide variant
/// above nuked a real Q1 heading because Q1's question text mentioned a
/// charge of "10 C".
fn heading_is_isotope(number: u32, window: &str) -> bool {
    // Case-sensitive: real nuclide notation uses an uppercase element symbol
    // ("20 Ne", "27 Al"). Case-insensitive matching made "22 f(x)" look like
    // fluorine isotope notation and dropped the real AQA FM Q22.
    let pattern = format!(r"\b{}\s*\d*\s*(?:He|Ne|Ar|Kr|Xe|Rn|F|Cl|Br|I|O|S|Se|Te|N|P|As|Sb|C|Si|Ge|Sn|B|Al|Ga|In|Be|Mg|Ca|Sr|Ba|Li|Na|K|Rb|Cs|U|Th|Pu)\b", number);
    if let Ok(re) = regex::Regex::new(&pattern) {
        return re.is_match(window);
    }
    false
}

/// Determine whether Question N starts on the same page where Question N-1 ended,
/// or on the next page. If we find a heading for Question N on N-1's footer page
/// that is *below* N-1's footer (y_frac > prev.y_frac), they share the page.
fn detect_page_split(prev: &Footer, cur: &Footer, headings: &[QuestionHeading]) -> usize {
    for h in headings {
        if h.number == cur.question && h.page == prev.page && h.y_frac > prev.y_frac {
            return prev.page;
        }
    }
    prev.page + 1
}

/// Find where question 1 plausibly starts: scan pages before its footer for
/// instruction/cover content; Q1 begins after the last such page.
#[allow(dead_code)]
fn estimate_first_question_start(page_texts: &[String], first_footer_page: usize) -> usize {
    let instr_re = regex::Regex::new(
        r"(?i)\binstructions\b|\binformation\b|answer all questions|formulae|\bglossary\b",
    )
    .unwrap();
    // Phase 2: tolerate AQA's spaced margin marker "0 1" in addition to the
    // compact "01" / bare "1" forms already accepted.
    let margin_re = regex::Regex::new(r"(?m)^\s*0?\s*1\s*$").unwrap();
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
            "middle of Q1 (Total for Question 1 is 5 marks)\n2. second question - extra text to ensure chars_remaining exceeds the thirty character limit.",
            "second continues (Total for Question 2 is 6 marks)",
            "TOTAL FOR PAPER IS 11 MARKS",
        ]);
        let map = build_text_only_map(&t, 5, &scan_text_layer(&t));
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
            "COVER PAGE NO QUESTIONS HERE",
            "1. first (Total for Question 1 is 3 marks)\n2. second (Total for Question 2 is 4 marks)",
            "3. third (Total for Question 3 is 2 marks)",
        ]);
        let map = build_text_only_map(&t, 3, &scan_text_layer(&t));
        assert_eq!(map.spans.len(), 3);
        assert_eq!(map.spans[0].number, 1);
        assert_eq!(map.spans[0].start_page, 1);
        assert_eq!(map.spans[0].end_page, 1);
        assert_eq!(map.spans[1].number, 2);
        assert_eq!(map.spans[1].start_page, 1); // same page as Q1's footer
        assert_eq!(map.spans[1].end_page, 1);
        assert_eq!(map.spans[2].number, 3);
        assert_eq!(map.spans[2].start_page, 2);
        assert_eq!(map.spans[2].end_page, 2);
    }

    #[test]
    fn corrupt_text_layer_falls_back() {
        let t = texts(&["garbled !@#$%^", "more garbled"]);
        let map = build_text_only_map(&t, 2, &scan_text_layer(&t));
        assert!(map.spans.is_empty());
    }

    /// Extract page texts via the production pdfium path and build the
    /// document map with NO AI structures. Returns None when the PDF is
    /// missing or unreadable (so tests skip gracefully on CI).
    fn golden_map(path: &str) -> Option<(Vec<String>, DocumentMap)> {
        if !std::path::Path::new(path).exists() {
            eprintln!("{} not found — skipping golden test", path);
            return None;
        }
        let page_inputs = match crate::pdf_render::render_pdf_pages(std::path::Path::new(path)) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("pdfium load failed for {}: {} — skipping", path, e);
                return None;
            }
        };
        let pages: Vec<String> = page_inputs.into_iter().map(|p| p.text).collect();
        let num_pages = pages.len();
        let map = build_text_only_map(&pages, num_pages, &scan_text_layer(&pages));
        Some((pages, map))
    }

    #[test]
    fn any_extracted_text_makes_the_whole_document_digital() {
        // Every page has text: digital, with nothing unresolved.
        let rich = "A page of real question text with plenty of words to transcribe. Calculate \
                    the resistance of the lamp when the current is 0.25 A and the potential \
                    difference across it is 3.0 V. Show all of your working clearly in the \
                    spaces provided on the answer lines below each part of the question. ";
        let digital = texts(&[rich, rich, rich]);
        let class = classify_text_layer(&digital);
        assert!(class.is_digital());
        assert!(!class.is_scanned_only());
        assert!(class.unresolved_pages().is_empty());
        assert_eq!(class.text_pages, 3);
        assert_eq!(class.total_pages, 3);

        // A SHORT paper is still digital: there is no document-size floor.
        let short = texts(&["1. Prove the thing. **[3 marks]**", "2. Integrate this."]);
        let class = classify_text_layer(&short);
        assert!(class.is_digital(), "short papers must stay local");
        assert_eq!(class.text_pages, 2);

        // A complete maths-only question with no prose clears the bar: no
        // threshold of any kind.
        let maths_only = texts(&["1. Solve x=2. [1]"]);
        let class = classify_text_layer(&maths_only);
        assert!(class.is_digital(), "maths-only content must stay local");
        assert_eq!(class.text_pages, 1);
        assert!(class.unresolved_pages().is_empty());

        // Any non-whitespace character counts, including furniture and noise.
        // Over-blocking keeps the import local; it never grants network access.
        let noisy = texts(&["12", "\u{200a}", "!@#$%^&*()"]);
        let class = classify_text_layer(&noisy);
        assert!(class.is_digital(), "noise must over-block, not pay");
        assert_eq!(class.text_pages, 2, "U+200A is whitespace-only");

        // Whitespace-only pages are unresolved, but one text page keeps the
        // whole import local and the gaps are reported.
        let gaps = texts(&[rich, "", "END OF QUESTIONS", "\u{200a}", "\t\n "]);
        let class = classify_text_layer(&gaps);
        assert!(class.is_digital(), "a blank page does not make a paper a scan");
        assert_eq!(class.text_pages, 2);
        assert_eq!(class.total_pages, 5);
        assert_eq!(class.unresolved_pages(), &[1, 3, 4]);

        // Genuinely scanned-only: no extracted text anywhere (whitespace only).
        let scanned = texts(&["", "   ", "\u{200a}", "\t\n "]);
        let class = classify_text_layer(&scanned);
        assert!(class.is_scanned_only(), "a real scan has no text anywhere");
        assert!(!class.is_digital());
        assert_eq!(class.text_pages, 0);
        assert_eq!(class.unresolved_pages(), &[0, 1, 2, 3]);
    }

    /// Map sufficiency and text-layer class answer different questions: a
    /// one-page paper keeps its first-page question; a real cover is skipped.
    #[test]
    fn one_page_paper_keeps_question_one_and_real_covers_are_still_skipped() {
        // A genuine one-page paper: Q1 sits on the first page.
        let one_page = texts(&[
            "1. The transformation P is an enlargement with scale factor k. Show that k = 3. [4 marks]\nEND OF QUESTIONS",
        ]);
        assert!(classify_text_layer(&one_page).is_digital());
        let scan = scan_text_layer(&one_page);
        assert!(
            scan.headings.iter().any(|h| h.number == 1 && h.page == 0),
            "first-page heading must be collected: {:?}",
            scan.headings
        );
        assert_ne!(
            scan.page_reliability[0],
            PageReliability::NonQuestion,
            "a page with a question heading is not front matter"
        );
        let map = build_text_only_map(&one_page, one_page.len(), &scan);
        assert_eq!(
            map.spans.iter().map(|s| s.number).collect::<Vec<_>>(),
            vec![1],
            "the first-page question must produce a span"
        );

        // A real cover keeps the old behaviour: rubric only, no question signal.
        let with_cover = texts(&[
            "Instructions to candidates\nAnswer ALL questions.\nTime: 1 hour 30 minutes",
            "1. Show that x = 2. [2 marks]\nEND OF QUESTIONS",
        ]);
        let scan = scan_text_layer(&with_cover);
        assert_eq!(scan.page_reliability[0], PageReliability::NonQuestion);
        assert!(scan.headings.iter().all(|h| h.page != 0));
        let map = build_text_only_map(&with_cover, with_cover.len(), &scan);
        assert_eq!(
            map.spans.iter().map(|s| s.number).collect::<Vec<_>>(),
            vec![1],
            "cover adds no spans"
        );
    }

    /// Map sufficiency and text-layer class answer different questions: a
    /// two-question digital paper fails the map gate but is still digital.
    #[test]
    fn weak_heading_map_is_still_digital() {
        let q1 = "1 A student measures the current in a filament lamp for a range of potential \
                  differences and records the results in a table before plotting a graph of \
                  current against potential difference for the lamp. **[3 marks]**";
        let q2 = "2 The student then explains why the resistance of the lamp increases as the \
                  temperature of the filament rises during the experiment. **[3 marks]**";
        let filler = "The apparatus is left to cool completely before the next reading is taken. \
                      The room temperature is recorded with a thermometer at the start and at the \
                      end of the experiment, and the values are compared with the table above. \
                      The student repeats each measurement three times so that a mean value can \
                      be calculated, and the results are recorded to an appropriate number of \
                      significant figures for the resolution of the measuring instruments used. ";
        let filler = filler.repeat(3);
        // Page 0 is always treated as cover/front matter, so the questions
        // start at index 1 exactly as they do in a real paper.
        let cover = "Answer ALL questions. Instructions to candidates: write your answers in the \
                     spaces provided and show all of your working clearly.";
        let pages = texts(&[cover, q1, q2, filler.as_str()]);
        let scan = scan_text_layer(&pages);
        assert!(classify_text_layer(&pages).is_digital());
        let map = build_text_only_map(&pages, pages.len(), &scan);
        assert_eq!(
            map.spans.iter().map(|s| s.number).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn text_scan_reads_cie_and_ocr_footers() {
        // Cambridge CIE style headings & footers
        let cie = texts(&[
            "Instructions",
            "1 (a) Find the derivative of f(x) padding padding padding padding padding padding [Total: 8]",
            "2 (a) Solve the differential equation padding padding padding padding padding [Total: 10]",
            "3 (a) Calculate the vector product padding padding padding padding padding [Total: 6]",
            "4 (a) Evaluate the definite integral padding padding padding padding padding [Total: 12]",
        ]);
        let scan_cie = scan_text_layer(&cie);
        assert_eq!(scan_cie.footers.len(), 4);

        // OCR style uppercase headings & footers
        let ocr = texts(&[
            "Instructions",
            "QUESTION 1\nCalculate the kinetic energy padding padding padding padding padding\n(Total for Question 1 = 6 marks)",
            "QUESTION 2\nDetermine the wavelength padding padding padding padding padding\n(Total for Question 2 = 8 marks)",
            "QUESTION 3\nFind the resistance padding padding padding padding padding\n(Total for Question 3 = 7 marks)",
            "QUESTION 4\nExplain the observed spectrum padding padding padding padding padding\n(Total for Question 4 = 9 marks)",
        ]);
        let scan_ocr = scan_text_layer(&ocr);
        assert_eq!(scan_ocr.footers.len(), 4);
    }

    #[test]
    fn edexcel_2023_pdf_uses_ordered_text_headings() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        let Some((pages, map)) = golden_map("../'23 edexcel.pdf") else {
            return;
        };

        let scan = scan_text_layer(&pages);
        let numbers: Vec<u32> = scan.headings.iter().map(|h| h.number).collect();
        assert_eq!(numbers, (1..=18).collect::<Vec<_>>());

        let span_numbers: Vec<u32> = map.spans.iter().map(|s| s.number).collect();
        assert_eq!(span_numbers, (1..=18).collect::<Vec<_>>());
        assert!(map
            .spans
            .iter()
            .all(|span| span.start_page <= span.end_page));
        assert!(!map.spans.iter().any(|span| span.number == 24));
        assert!(map
            .spans
            .iter()
            .all(|span| span.end_page.saturating_sub(span.start_page) <= 4));
    }

    #[test]
    fn edexcel_fixture_text_and_image_extraction_integrity() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        let years = ["17", "18", "19", "20", "21", "22", "24"];
        let mut tested = 0usize;

        for year in years {
            // (A paper fixed against moves from papers/unused to papers/used.)
            let path = ["../papers/unused", "../papers/used"]
                .iter()
                .map(|dir| format!("{dir}/edexcel '{year} edexcel.pdf"))
                .find(|p| std::path::Path::new(p).exists())
                .unwrap_or_else(|| format!("../papers/unused/edexcel '{year} edexcel.pdf"));
            assert!(
                std::path::Path::new(&path).exists(),
                "missing requested Edexcel fixture: {}",
                path
            );

            let page_inputs = crate::pdf_render::render_pdf_pages(std::path::Path::new(&path))
                .unwrap_or_else(|error| panic!("{} could not be rendered: {}", path, error));
            assert!(!page_inputs.is_empty(), "{} rendered zero pages", path);

            let text_bytes: usize = page_inputs
                .iter()
                .map(|page| page.text.trim().len())
                .sum();
            assert!(
                page_inputs
                    .iter()
                    .all(|page| page.text.len() <= 20_000_000),
                "{} contains an implausibly large page text payload",
                path,
            );

            let mut image_pages = 0usize;
            let mut text_only_pages = 0usize;
            for (page_index, page) in page_inputs.iter().enumerate() {
                match &page.kind {
                    crate::pipeline::PageInputKind::Image { b64 } => {
                        image_pages += 1;
                        assert!(
                            b64.starts_with("data:image/"),
                            "{} page {} image payload has an invalid data URL",
                            path,
                            page_index + 1
                        );
                        let decoded = crate::geometry::decode_page_image(b64).unwrap_or_else(|| {
                            panic!(
                                "{} page {} image payload could not be decoded",
                                path,
                                page_index + 1
                            )
                        });
                        assert!(
                            decoded.width() >= 100 && decoded.height() >= 100,
                            "{} page {} image is implausibly small: {}x{}",
                            path,
                            page_index + 1,
                            decoded.width(),
                            decoded.height()
                        );
                    }
                    crate::pipeline::PageInputKind::TextOnly => {
                        text_only_pages += 1;
                        assert!(
                            !page.text.trim().is_empty(),
                            "{} page {} is TextOnly but has no extracted text",
                            path,
                            page_index + 1
                        );
                    }
                }
            }
            assert!(
                image_pages > 0,
                "{} produced no page images for image extraction",
                path
            );

            // A scanned/image-dominant paper is valid even when its PDF text
            // layer is sparse. In that mode the text scan must explicitly
            // decline to build a map, while the image path remains complete.
            let text_rich = text_bytes >= 1000;

            let page_texts: Vec<String> = page_inputs.iter().map(|page| page.text.clone()).collect();
            let scan = scan_text_layer(&page_texts);
            let heading_numbers: Vec<u32> = scan.headings.iter().map(|heading| heading.number).collect();
            let map = build_text_only_map(&page_texts, page_inputs.len(), &scan);
            if text_rich {
                assert!(
                    heading_numbers.len() >= 6,
                    "{} produced too few canonical text headings: {:?}",
                    path,
                    heading_numbers
                );
                assert_eq!(
                    heading_numbers.first().copied(),
                    Some(1),
                    "{} text headings do not start at Q1: {:?}",
                    path,
                    heading_numbers
                );
                assert!(
                    heading_numbers.windows(2).all(|pair| pair[1] > pair[0]),
                    "{} text headings are not strictly ordered: {:?}",
                    path,
                    heading_numbers
                );

                let span_numbers: Vec<u32> = map.spans.iter().map(|span| span.number).collect();
                assert_eq!(
                    span_numbers, heading_numbers,
                    "{} map spans {:?} do not match canonical text headings {:?}",
                    path, span_numbers, heading_numbers
                );
            } else {
                assert!(
                    text_bytes < 1000,
                    "{} was not classified consistently as image-dominant",
                    path
                );
            }
            assert!(
                map.spans.iter().all(|span| {
                    span.start_page <= span.end_page
                        && span.end_page < page_inputs.len()
                        && span.start_y_frac.map(|y| (0.0..=1.0).contains(&y)).unwrap_or(true)
                        && span.end_y_frac.map(|y| (0.0..=1.0).contains(&y)).unwrap_or(true)
                }),
                "{} contains invalid question span bounds",
                path
            );

            eprintln!(
                "Edexcel {}: pages={}, text_bytes={}, images={}, text_only={}, headings={:?}",
                year,
                page_inputs.len(),
                text_bytes,
                image_pages,
                text_only_pages,
                heading_numbers
            );
            tested += 1;
        }

        assert_eq!(tested, years.len());
    }

    /// Golden test against every real AQA physics paper in the repo root.
    /// Exercises the text-layer heading scan + hybrid map with NO AI
    /// structures — the text layer alone must place every question.
    /// One single test over all papers: pdfium is not thread-safe, so the
    /// per-paper tests must not run in parallel. Skips gracefully when a
    /// PDF is not present (CI).
    #[test]
    fn aqa_physics_papers_place_all_questions() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        // (year, expected question count). '19 Section B is "Questions 07 to
        // 31" — 31 questions; '17/'18/'23/'24 have 32; '20/'21/'22 have 31.
        let cases: &[(&str, usize)] = &[
            ("17", 32),
            ("18", 32),
            ("19", 31),
            ("20", 31),
            ("21", 31),
            ("22", 31),
            ("23", 32),
            ("24", 32),
        ];
        for (year, expected) in cases {
            let path = format!("../physics '{}.pdf", year);
            let Some((_pages, map)) = golden_map(&path) else { continue };
            let span_nums: Vec<u32> = map.spans.iter().map(|s| s.number).collect();
            eprintln!("=== physics '{}: {} spans = {:?}", year, span_nums.len(), span_nums);
            for s in &map.spans {
                eprintln!(
                    "  Q{}: pages {}..={} y {:?}..{:?}",
                    s.number,
                    s.start_page + 1,
                    s.end_page + 1,
                    s.start_y_frac,
                    s.end_y_frac
                );
            }
            assert_eq!(
                span_nums.len(),
                *expected,
                "physics '{}: expected {} question spans, got {:?}",
                year,
                expected,
                span_nums
            );
            for (i, s) in map.spans.iter().enumerate() {
                assert_eq!(
                    s.number,
                    (i + 1) as u32,
                    "physics '{}: spans must be Q1..Q{} in order, got {:?}",
                    year,
                    expected,
                    span_nums
                );
                assert!(s.start_page <= s.end_page, "physics '{}: Q{} inverted pages", year, s.number);
                // No degenerate zero-height y-band on single-page questions.
                if s.start_page == s.end_page {
                    if let (Some(a), Some(b)) = (s.start_y_frac, s.end_y_frac) {
                        assert!(
                            b > a + 0.001,
                            "physics '{}: Q{} degenerate y-band {}..{}",
                            year,
                            s.number,
                            a,
                            b
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn large_worksheet_detects_100_plus_questions() {
        // Construct simulated 126-page worksheet where each page contains a question 1..=126
        let mut pages = vec!["Instructions\nAnswer all questions on this worksheet.".to_string()];
        for i in 1..=126 {
            pages.push(format!("{} (a) Solve this problem about topic {}. **[3 marks]**\n(Total for Question {} is 3 marks)", i, i, i));
        }

        let scan = scan_text_layer(&pages);
        let heading_numbers: Vec<u32> = scan.headings.iter().map(|h| h.number).collect();
        assert_eq!(heading_numbers.len(), 126, "All 126 question headings should be captured");
        assert_eq!(heading_numbers.first().copied(), Some(1));
        assert_eq!(heading_numbers.last().copied(), Some(126));

        let map = build_text_only_map(&pages, pages.len(), &scan_text_layer(&pages));
        let span_numbers: Vec<u32> = map.spans.iter().map(|s| s.number).collect();
        assert_eq!(span_numbers.len(), 126, "All 126 question spans should be built in the document map");
        assert_eq!(span_numbers.first().copied(), Some(1));
        assert_eq!(span_numbers.last().copied(), Some(126));
    }

    fn heading(page: usize, number: u32) -> QuestionHeading {
        QuestionHeading { page, number, y_frac: 0.1 }
    }

    /// A lone number is a heading only when it fills a gap in the strong
    /// heading set; page numbers (already present) and out-of-range values are
    /// rejected. Regression for Jan 2025 answer-book page numbers and the CIE
    /// "number printed above a diagram" headings.
    #[test]
    fn lone_headings_only_fill_strong_set_gaps() {
        let strong = vec![
            heading(0, 1),
            heading(1, 2),
            heading(2, 3),
            heading(3, 5),
        ];
        let lone = vec![
            heading(2, 3),  // duplicate of a strong heading (page number)
            heading(3, 4),  // genuine gap-filler
            heading(4, 99), // beyond the paper's range
        ];
        let out = resolve_lone_headings(strong, lone, None);
        let numbers: Vec<u32> = out.iter().map(|h| h.number).collect();
        assert_eq!(numbers, vec![1, 2, 3, 5, 4]);
        assert!(!numbers.contains(&99));
    }

    /// A declared question count constrains only ambiguous bare-number
    /// candidates. A strong heading above the declaration (a nonconsecutive
    /// later section, e.g. AQA "Section B" restarting at 7) must survive.
    #[test]
    fn declared_count_does_not_erase_strong_headings() {
        let strong = vec![
            heading(0, 1),
            heading(1, 2),
            heading(2, 7),
            heading(3, 8),
        ];
        let lone = vec![heading(4, 50)];
        let out = resolve_lone_headings(strong, lone, Some(3));
        let numbers: Vec<u32> = out.iter().map(|h| h.number).collect();
        assert_eq!(numbers, vec![1, 2, 7, 8]);
    }

    /// The last question may be printed as a bare number just below the last
    /// strong heading; retain it. A page/table value far beyond the range is
    /// still rejected.
    #[test]
    fn final_lone_heading_beyond_max_strong_is_retained() {
        let strong = vec![heading(0, 1), heading(1, 2), heading(2, 3)];
        let lone = vec![heading(3, 4), heading(4, 9)];
        let out = resolve_lone_headings(strong, lone, None);
        let numbers: Vec<u32> = out.iter().map(|h| h.number).collect();
        assert_eq!(numbers, vec![1, 2, 3, 4]);
    }

    /// Nuclide detection is case-sensitive: "27 Al" is an isotope, but
    /// "22 f(x)" is a function definition (AQA FM Q22), not fluorine.
    #[test]
    fn isotope_detection_requires_uppercase_symbol() {
        assert!(heading_is_isotope(27, "27 Al 12 can decay"));
        assert!(heading_is_isotope(20, "20 Ne 10"));
        assert!(!heading_is_isotope(22, "22 f(x) = +"));
        assert!(!heading_is_isotope(5, "5 y = xx + "));
    }

    /// "Q5" followed by a lowercase continuation is a margin cross-reference,
    /// not a heading (it was displacing CIE 2022 Q4). A real "Q2 Question …"
    /// heading is still retained.
    #[test]
    fn q_prefixed_margin_label_is_not_a_heading() {
        let pages = texts(&[
            "Instructions to candidates. There are 6 questions in this question paper.",
            "3 The coefficient of x\nQ5\nis a. The coefficient of x\ncontinues here.",
            "Q1\nx = 2y - 1\nand hence solve the simultaneous equations for y in terms of x and another variable so the page carries real content.",
            "Q2 Question two text begins here and continues for a while so the page is long enough to matter and carries real content for the parser to read.",
        ]);
        let scan = scan_text_layer(&pages);
        let numbers: Vec<u32> = scan.headings.iter().map(|h| h.number).collect();
        assert!(!numbers.contains(&5), "Q5 cross-reference became a heading: {numbers:?}");
        // Lowercase alone is not proof: a genuine formula heading "Q1" must
        // survive the narrowed rule.
        assert!(numbers.contains(&1), "genuine Q1 formula heading dropped: {numbers:?}");
    }

    /// AQA prints a "*02*" page marker; it must never become a question heading.
    #[test]
    fn aqa_page_marker_is_not_a_heading() {
        let pages = texts(&[
            "Answer ALL questions. Instructions to candidates.\nThere are 2 questions in this question paper.",
            "1 Work out the value of t where t = 2.42 \u{00d7} 103\n[2 marks]\nAnswer\nPMT\n2 Factorise x2 \u{2013} y2\n[1 mark]\nAnswer",
            "*02*\nDo not write outside the box\n1 Work out the value of t\n2 Factorise x2 \u{2013} y2",
        ]);
        let scan = scan_text_layer(&pages);
        let numbers: Vec<u32> = scan.headings.iter().map(|h| h.number).collect();
        assert!(numbers.contains(&1), "{numbers:?}");
        assert!(numbers.contains(&2), "{numbers:?}");
        // The asterisk page marker on page 3 must not re-introduce a heading 2
        // beyond the real ones (canonicalization keeps only the real sequence).
        let map = build_text_only_map(&pages, pages.len(), &scan);
        assert!(map.spans.iter().any(|s| s.number == 2));
    }

    /// The prior question's "[2 marks]" tag on a short one-question page must
    /// not suppress the next heading (AQA GCSE Further Maths layout).
    #[test]
    fn previous_line_marks_tag_does_not_suppress_next_heading() {
        let pages = texts(&[
            "Answer ALL questions. Instructions to candidates.\nThere are 3 questions in this question paper.",
            "1 Work out the value of t where t = 2.42 \u{00d7} 103\n[2 marks]\nAnswer",
            "2 Factorise x2 \u{2013} y2\n[1 mark]\nAnswer",
            "3 The nth term of a sequence is n + 34\n[1 mark]\nCircle the limiting value",
        ]);
        let scan = scan_text_layer(&pages);
        let numbers: Vec<u32> = scan.headings.iter().map(|h| h.number).collect();
        assert!(numbers.contains(&2), "Q2 suppressed by Q1 marks tag: {numbers:?}");
        assert!(numbers.contains(&3), "{numbers:?}");
    }

    /// A paper that ships a separate answer booklet repeats every
    /// "Total for Question N is M marks" footer. Keeping only the first footer
    /// per question must restore monotonicity instead of discarding all footers.
    #[test]
    fn duplicate_answer_book_footers_keep_monotone_sequence() {
        let qp1 = "1. first question text padded out to exceed one hundred characters so the page is treated as carrying question content rather than boilerplate. (Total for Question 1 is 3 marks)";
        let qp2 = "2. second question text padded out to exceed one hundred characters so the page is treated as carrying question content rather than boilerplate. (Total for Question 2 is 4 marks)";
        let ab1 = "1. answer space (Total for Question 1 is 3 marks)";
        let ab2 = "2. answer space (Total for Question 2 is 4 marks)";
        let pages = texts(&["Instructions to candidates.", qp1, qp2, ab1, ab2]);
        let scan = scan_text_layer(&pages);
        let map = build_text_only_map(&pages, pages.len(), &scan);
        let numbers: Vec<u32> = map.spans.iter().map(|s| s.number).collect();
        assert_eq!(numbers, vec![1, 2], "duplicate answer-book footers broke the span sequence");
    }
}
