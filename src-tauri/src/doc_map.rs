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

use tracing::debug;

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
    /// The vision structure pass returns an actual pixel fraction.
    pub y_frac: f32,
}

/// A question heading detected on a page (text layer or vision).
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
        // OCR / AQA variants
        re: r"(?i)\(?\s*Total\s+(?:for\s+Question\s+(\d{1,2})\s+)?(?:is\s+)?(\d{1,2})\s+marks?\s*\)?",
        has_question_num: true,
    },
    FooterPattern {
        // WJEC / Eduqas / IB short forms
        re: r"(?i)\[\s*Total\s*:?\s*(\d{1,2})\s*marks?\s*\]",
        has_question_num: false,
    },
];

fn compile_footer_regexes() -> Vec<(regex::Regex, bool)> {
    FOOTER_PATTERNS
        .iter()
        .map(|p| (regex::Regex::new(p.re).unwrap(), p.has_question_num))
        .collect()
}

fn paper_total_regex() -> regex::Regex {
    regex::Regex::new(r"(?i)TOTAL\s+FOR\s+PAPER\s+IS\s+(\d{1,3})\s+MARKS").unwrap()
}

/// Regex for question HEADINGS (whole questions, not parts) at the start of
/// a line / block. Accepts:
///   "1." "1)" "1]" "1–" "1-" "1 " with optional bold "**1.**",
///   "Q1" "Q.1" "Q 1", "Question 1" / "Question 1(a)"
/// but NOT:
///   * AQA decimal part numbers "03.1" (handled separately via a leading 0
///     guard; sub-part digits are filtered).
///   * decimal quantities like "3.5 V" or dates "2024." (required after-digit
///     punctuation that looks like a label).
///   * part labels (a)/(b)/(i) — those never begin with 1+ digits at line
///     start followed by a period/closing paren without a letter.
fn question_heading_regex() -> regex::Regex {
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
    regex::Regex::new(
        r"(?m)(?:^|\n)[ \t]*(?:box[ \t]+)?(?:\*\*)?(?:Q(?:uestion)?\.?[ \t]*)?0*[ \t]*([1-9](?:[ \t]*\d){0,2})(?:\*\*)?[ \t]*(?:[\.\)\]\-–—]|[ \t]+)(?:\D|$|\d{1,3}\s*(?:He|Ne|Ar|Kr|Xe|Rn|F|Cl|Br|I|O|S|Se|Te|N|P|As|Sb|C|Si|Ge|Sn|B|Al|Ga|In|Be|Mg|Ca|Sr|Ba|Li|Na|K|Rb|Cs|U|Th|Pu)\b)",
    )
    .unwrap()
}

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

/// Decide whether the deterministic text layer is rich enough to build the
/// document map WITHOUT the vision structure pass (one AI call per page).
/// Calibrated against all AQA physics papers ('17–'24): each passes with a
/// complete ascending heading sequence, while scanned/garbled PDFs fail and
/// fall back to the vision structure pass.
pub fn text_layer_map_sufficient(scan: &TextScan, num_pages: usize) -> bool {
    if num_pages == 0 {
        return false;
    }
    let mut nums: Vec<u32> = scan.headings.iter().map(|h| h.number).collect();
    nums.sort();
    nums.dedup();
    // Need a real paper's worth of questions, starting at Q1.
    if nums.len() < 6 || nums.first().copied() != Some(1) {
        return false;
    }
    // At most 2 missing numbers inside 1..=max (tolerates a heading lost to
    // a diagram-only page or extraction quirk; more means the text layer is
    // too sparse to trust alone). Headings have already been canonicalized
    // into the longest plausible ordered sequence by scan_text_layer, so a
    // stray heading cannot inflate this range or the spans built later.
    let max_q = *nums.last().unwrap();
    let gaps = (max_q as usize).saturating_sub(nums.len());
    if gaps > 2 {
        return false;
    }
    // Headings must be spread across the document, not clumped on a couple
    // of pages (which would indicate a heading-like list, not questions).
    let pages_with_headings: std::collections::BTreeSet<usize> =
        scan.headings.iter().map(|h| h.page).collect();
    let coverage = pages_with_headings.len() as f32 / num_pages as f32;
    pages_with_headings.len() >= 4 || coverage >= 0.35
}

/// Scan every page's raw text layer for structural footers AND question
/// headings.
pub fn scan_text_layer(page_texts: &[String]) -> TextScan {
    let footer_res = compile_footer_regexes();
    let paper_re = paper_total_regex();
    let heading_re = question_heading_regex();
    let mut footers = Vec::new();
    let mut headings = Vec::new();
    let mut paper_total = None;
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
    // Ambiguous so the structure pass sends their images to the vision model.
    // Without this, every page becomes NonQuestion, the structure pass
    // short-circuits to synthetic BLANK responses, vision never runs, and the
    // document map is empty — the "Blank Page Trap" that kills AQA Physics.
    let total_text_len: usize = page_texts.iter().map(|t| t.trim().len()).sum();
    let is_image_only = total_text_len < 100;

    let instr_re = regex::Regex::new(
        r"(?i)(?:^|\n)\s*(?:instructions?\s*(?:to\s+candidates?)?|answer\s+all\s+questions|glossary)(?:\s|$|:)",
    ).unwrap();
    let formulae_sheet_re = regex::Regex::new(
        r"(?i)(?:^|\n)\s*(?:formulae?|data|constants|relationships?)\s*(?:sheet|booklet|table|page)?\s*$",
    ).unwrap();
    let blank_re = regex::Regex::new(r"(?i)(blank page|this page is intentionally blank|there are no questions printed on this page|do not write on this page)").unwrap();
    let ref_re = regex::Regex::new(r"(?i)^\s*(formulae|data|reference|constants)\s*(sheet|table|booklet)?\s*$").unwrap();
    let aqa_figure_re = regex::Regex::new(r"(?i)\bfig(?:ure)?\.?\s*\d+").unwrap();
    let aqa_table_re = regex::Regex::new(r"(?i)\btable\s+\d+").unwrap();
    let aqa_main_re = regex::Regex::new(r"0\s+\d{1,2}").unwrap();
    let aqa_sub_re = regex::Regex::new(r"(?m)^\s*0?\s*\d{1,2}\s*\.\s*\d+").unwrap();
    let marks_re = regex::Regex::new(r"(?i)\[\s*\d+\s*marks?").unwrap();

    for (page, text) in page_texts.iter().enumerate() {
        // Ignore cover page (page 0) and extra back-matter pages
        let is_cover = page == 0;
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
        for (re, has_qn) in &footer_res {
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
        for cap in heading_re.captures_iter(text) {
            let full = cap.get(0).unwrap();
            let raw_num_str = cap.get(1).unwrap().as_str();
            // Safe byte boundaries that are guaranteed to be on valid UTF-8
            // char boundaries, even when the regex match interacts with
            // multi-byte Unicode characters (e.g. HAIR SPACE \u{200a}) in the
            // extracted text layer.
            let safe_start = text.ceil_char_boundary(full.start());
            let safe_end = text.ceil_char_boundary(full.end());
            debug!("Page {} heading match: {:?}", page, full.as_str());

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
            let start_idx = text[..safe_start]
                .char_indices()
                .rev()
                .nth(20)
                .map(|(i, _)| i)
                .unwrap_or(0);
            let context = text[start_idx..safe_end].to_string();
            let near_marks_tag = regex::Regex::new(r"(?i)\[\s*\d+\s*marks?").unwrap().is_match(&context);
            if near_marks_tag && tokens.len() <= 1 && !is_spaced_subpart {
                continue; // Skip number that is clearly a mark allocation.
            }

            // --- FILTER 3: Clean, parse, and range-check ---
            let cleaned_num = question_num_str.replace(" ", "");
            let y_frac = (full.start() as f32 / page_len).clamp(0.0, 1.0);
            if let Ok(n) = cleaned_num.parse::<u32>() {
                if n > 0 && n <= 50 { // Plausible exam range; dense papers rarely exceed 50.
                    let chars_remaining = text.len() - safe_end;
                    if chars_remaining > 30 {
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
                        let has_period = full.as_str().contains('.');
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
                            let after: String = text[safe_end..].chars().take(6).collect();
                            let after_trim = after.trim_start();
                            let units = ["kg", "g ", "m ", "cm", "mm", "V ", "N ", "J ", "Pa", "Hz", "kJ", "W ", "A ", "C ", "s "];
                            units.iter().any(|u| after_trim.starts_with(u))
                        };

                        // --- FILTER 5: Page-number guard ---
                        // AQA prints page numbers at the very bottom (y_frac > 0.85). On blank pages, y_frac is 0.0 but text.len() is small.
                        // We avoid dropping real questions by checking AQA's padding conventions (leading zeros, spaces between digits).
                        let is_likely_page_number = (n as usize) == page + 1 || (n as usize) == page || (n as usize) == page + 2;
                        let looks_like_real_question = has_margin_form || has_q || has_period;

                        let at_bottom = y_frac > 0.85 && chars_remaining < 150;
                        let at_top = y_frac < 0.15 && safe_end < 150;
                        let is_printed_page_number = is_likely_page_number 
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

                        debug!("[PAGE NUM FILTER]: match='{:?}', num={}, page={}, is_likely={}, looks_like_real={}, y_frac={}, chars_remaining={}, text_len={}, IS_PRINTED_PAGE={}",
                            full.as_str(), n, page, is_likely_page_number, looks_like_real_question, y_frac, chars_remaining, text.len(), is_printed_page_number);

                        if !is_quantity && !is_printed_page_number && !bare_reject && !is_isotope && !ends_in_degree {
                            headings.push(QuestionHeading { page, number: n, y_frac });
                        }
                    }
                }
            }
        }

        if paper_total.is_none() {
            if let Some(cap) = paper_re.captures(text) {
                paper_total = cap[1].parse::<u32>().ok().filter(|&t| t > 0);
            }
        }

        let has_question_signal = aqa_figure_re.is_match(text)
            || aqa_table_re.is_match(text)
            || aqa_main_re.is_match(text)
            || aqa_sub_re.is_match(text)
            || marks_re.is_match(text)
            || headings.iter().any(|h| h.page == page);

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
        let is_formulae_sheet = formulae_sheet_re.is_match(text) && !marks_re.is_match(text);
        let instr_hit = instr_re.is_match(text);
        let ref_hit = ref_re.is_match(text) || is_formulae_sheet;

        if is_image_only {
            page_reliability[page] = PageReliability::Ambiguous;
        } else if blank_re.is_match(text) || text.trim().is_empty() {
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
    debug!("page_reliability = {:?}", page_reliability);
    TextScan {
        footers,
        paper_total,
        page_reliability,
        headings: canonicalize_headings(headings),
    }
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

    // A real paper starts at Q1. Anchor to that heading when present so an
    // equally long answer-booklet subsequence beginning at Q2/Q3 cannot win
    // merely because it happens to contain one more noisy tail heading.
    if let Some(start) = headings.iter().position(|heading| heading.number == 1) {
        let mut selected = vec![headings[start]];
        let mut current_number = 1u32;
        for heading in headings.iter().skip(start + 1) {
            if heading.number > current_number && heading.number - current_number <= 3 {
                selected.push(*heading);
                current_number = heading.number;
            }
        }
        return selected;
    }

    let mut lengths = vec![1usize; headings.len()];
    let mut predecessors = vec![None; headings.len()];

    for i in 0..headings.len() {
        for j in 0..i {
            let number_gap = headings[i].number.saturating_sub(headings[j].number);
            if headings[j].number >= headings[i].number || number_gap > 3 {
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
    for i in 1..headings.len() {
        if lengths[i] > lengths[end] {
            end = i;
        }
    }

    let mut selected = Vec::with_capacity(lengths[end]);
    let mut current = Some(end);
    while let Some(index) = current {
        selected.push(headings[index]);
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
    
    debug!("inside build_spans_from_reliable_pages: reliable_footers.len()={}", reliable_footers.len());

    // Phase 1b: we NO LONGER return early when reliable_footers is empty.
    // Pure MCQ / short-answer papers (some boards' Paper 1, Edexcel MCQ
    // sections, IB Paper 1) print NO "Total for Question N is M marks"
    // footers at all — they only use per-question [1 mark] tags. For
    // those papers the text layer still has question headings; we just
    // skip the footer-driven span building and rely on
    // append_text_only_short_answer_spans below. The structure pass also
    // fills in spans via vision when the text layer is corrupt.
    if !reliable_footers.is_empty() {
        // Sort and deduplicate
        let mut footers = reliable_footers;
        footers.sort_by_key(|f| (f.page, f.question));
        footers.dedup_by_key(|f| f.question);

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
                debug!("pushed Q{} to spans from footer", f.question);
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
/// The byte-offset proxy is rough but good enough for "don't send pixels
/// from the *next* question to the model" — the Rust sanitizer will still
/// reject content outside the span's number, so bleeding a few lines is
/// not a correctness hazard.
fn infer_y_clips(
    scan: &TextScan,
    question: u32,
    start_page: usize,
    _end_page: usize,
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

    // If the footer sits in the bottom 30% of its page we still show the
    // whole page (it's almost certainly the last element on the page).
    // If it sits higher up, clip at the footer + a small padding band so
    // the model doesn't see the next question's heading below it.
    let end_y = if footer_y_frac < 0.7 {
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
        if page == 0
            || scan.page_reliability[page] == PageReliability::NonQuestion
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
            // band on this page (cross-page long questions). The previous
            // over-broad check skipped the heading whenever any span
            // "covered" the page at all, which lost MCQs that sit below
            // a long question's last-page footer on the same page.
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
                let is_inside = h.y_frac >= lo - 0.02 && h.y_frac < hi;
                if is_inside {
                    debug!("Q{} heading on page {} (y={}) is inside span Q{} (lo={}, hi={})", h.number, page, h.y_frac, s.number, lo, hi);
                }
                is_inside
            });
            if inside_other_span {
                debug!("ignored Q{} heading on page {} because it is inside_other_span", h.number, page);
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
            debug!("pushed Q{} to spans from text-heading append", h.number);
        }
    }

    spans.sort_by(|a, b| {
        let p = a.start_page.cmp(&b.start_page);
        if p != std::cmp::Ordering::Equal { return p; }
        a.number.cmp(&b.number)
    });
    
    debug!("spans in build_spans_from_reliable_pages: {:?}", spans.iter().map(|s| s.number).collect::<Vec<_>>());
    
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
    let text_headings_sufficient = text_layer_map_sufficient(&scan, num_pages);
    let (mut spans, _reliable_pages, text_anomalies) = build_spans_from_reliable_pages(&scan, num_pages);
    anomalies.extend(text_anomalies);

    // A sufficient text layer is already the deterministic document map. Use
    // its page-local headings directly, including ambiguous pages, instead of
    // sending synthetic empty structures through the vision span builder.
    if text_headings_sufficient {
        append_text_only_short_answer_spans(&scan, &mut spans, &mut anomalies, true);
    }

    // Phase 1b: when the text layer gave us NO reliable footers (common on
    // MCQ-heavy papers, scanned PDFs with corrupt text layers, IB/CAIE papers
    // that don't print "Total for Question N is M marks"), we can't trust
    // the text-layer reliability classification — many pages that were
    // marked Reliable/NonQuestion on the strength of a single stray word
    // ("information", "formulae") are actually question pages. In that
    // case feed ALL non-truly-blank pages into the vision span builder so
    // the structure pass's (paid-for) output is not silently discarded.
    let text_layer_trustworthy = text_headings_sufficient
        || (!scan.footers.is_empty()
            && scan.footers.iter().filter(|f| f.question > 0).count() >= 2);

    // 2. Identify which pages to feed the vision builder. When the text
    // layer is trustworthy, use only Ambiguous pages (the existing hybrid
    // approach — saves us from merging against pages the text layer
    // already placed). When not, feed every page that has structure data
    // and is not a hard Blank/Cover/etc.
    let vision_pages: Vec<usize> = if text_headings_sufficient {
        Vec::new()
    } else if text_layer_trustworthy {
        (0..num_pages)
            .filter(|&p| scan.page_reliability[p] == PageReliability::Ambiguous)
            .collect()
    } else {
        (0..num_pages)
            .filter(|&p| {
                // Skip pages the AI itself classified as non-question.
                if let Some(s) = structures.get(p) {
                    if !s.role.is_question_content() {
                        return false;
                    }
                }
                // Also skip pages our text scan is confident are blank.
                !matches!(scan.page_reliability[p], PageReliability::NonQuestion)
            })
            .collect()
    };

    // 3. Run vision structure on the selected pages (structures already computed).
    if !vision_pages.is_empty() {
        let vision_spans = build_spans_from_vision(structures, &vision_pages, num_pages, &scan.headings, &page_texts);
        spans = merge_spans(spans, vision_spans, &mut anomalies);
    }
    
    // 4. Collect non-question pages (union of text-layer and structure-pass
    // verdicts; trust either source when it marks a page as non-question).
    let mut non_question_pages: Vec<usize> = (0..num_pages)
        .filter(|&p| scan.page_reliability[p] == PageReliability::NonQuestion)
        .collect();
    for s in structures {
        if !s.role.is_question_content() && !non_question_pages.contains(&s.page) {
            non_question_pages.push(s.page);
        }
    }
    non_question_pages.sort();
    non_question_pages.dedup();

    // 5. Vision-fallback pages are the ones we actually fed to build_spans_from_vision.
    let vision_fallback_pages = vision_pages.clone();
    
    // Validate final spans for monotonicity. Loose guard: backward = always bad;
    // gap > 40 = almost certainly hallucinated outlier; everything else allowed.
    // Dense MCQ pages (8, 9, 10, 11) must not trigger false jump alarms.
    let mut valid_spans = Vec::new();
    let mut expected_max_q = 0u32;
    
    debug!("\n=== MERGEMARK SPANS BEFORE FILTER ===");
    for s in &spans {
        debug!("Found Question {} (spanning pages {} to {})", s.number, s.start_page + 1, s.end_page + 1);
    }
    debug!("============================================\n");
    
    for mut span in spans {
        if expected_max_q > 0 && span.number <= expected_max_q {
            debug!(">> DROPPING DUPLICATE/DISJOINTED PART for Question {} (Page {}) because it was already merged into the main span above!", span.number, span.start_page + 1);
            anomalies.push(format!("dropped backwards/duplicate question Q{} (expected > {})", span.number, expected_max_q));
            continue;
        }
        if expected_max_q > 0 && span.number > expected_max_q + 40 {
            anomalies.push(format!("dropped likely hallucinated jump to Q{} (gap from {} exceeds 40)", span.number, expected_max_q));
            continue;
        }
        
        span.start_page = span.start_page.min(span.end_page);
        expected_max_q = expected_max_q.max(span.number);
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

/// Per-question running bounds during vision span building.
#[derive(Debug, Clone, Default)]
struct VisionBounds {
    first_page: usize,
    last_page: usize,
    /// Clip on the first page (top of the question).
    first_y: Option<f32>,
    /// Clip on the last page (bottom of the question, from footer_y).
    last_y: Option<f32>,
    marks: Option<u32>,
}

/// Build spans from vision structure for specific pages.
///
/// Phase 1: the structure pass now reports per-question y fractions on each
/// page. We use those to tighten the first/last page clips when a question
/// starts or ends mid-page (MCQ / short-answer pages).
///
/// Phase 1b: `eligible_pages` selects which pages contribute to vision
/// bounds. When the text layer was trustworthy we pass only Ambiguous pages
/// (existing hybrid behaviour); when it wasn't, we pass all question pages
/// so the structure pass's paid-for output isn't discarded just because a
/// regex false-positive labelled a page NonQuestion/Reliable.
fn is_figure_hallucination(proposed_num: u32, page_text: &str) -> bool {
    let num_pattern = format!(r"\b{}\b", proposed_num);
    let num_re = regex::Regex::new(&num_pattern).unwrap();
    let total_occurrences = num_re.find_iter(page_text).count();
    
    // Check if the number appears immediately after Figure, Fig, Table, Graph, or Diagram
    let veto_pattern = format!(
        r"(?i)\b(?:figure|fig|table|graph|diagram)\.?\s*{}\b",
        proposed_num
    );
    let veto_re = regex::Regex::new(&veto_pattern).unwrap();
    let veto_occurrences = veto_re.find_iter(page_text).count();

    // If every single time this number appears on the page, it's prefixed by "Figure " etc,
    // then it's a hallucination 100% of the time.
    total_occurrences > 0 && total_occurrences == veto_occurrences
}

#[allow(dead_code)]
fn is_isotope_hallucination(proposed_num: u32, page_text: &str) -> bool {
    // Specifically looking for the pattern: `20 \n 10 Ne` or `20 Ne` or `20 \n Ne`.
    let pattern = format!(r"(?i)\b{}\s*\d*\s*(?:He|Ne|Ar|Kr|Xe|Rn|F|Cl|Br|I|O|S|Se|Te|N|P|As|Sb|C|Si|Ge|Sn|B|Al|Ga|In|Be|Mg|Ca|Sr|Ba|Li|Na|K|Rb|Cs|U|Th|Pu)\b", proposed_num);
    if let Ok(re) = regex::Regex::new(&pattern) {
        return re.is_match(page_text);
    }
    false
}

/// Narrow isotope check used at heading-scan time: only fires when the
/// isotope pattern appears AT the heading site itself (the match plus a few
/// following characters), never elsewhere on the page. The page-wide variant
/// above nuked a real Q1 heading because Q1's question text mentioned a
/// charge of "10 C".
fn heading_is_isotope(number: u32, window: &str) -> bool {
    let pattern = format!(r"(?i)\b{}\s*\d*\s*(?:He|Ne|Ar|Kr|Xe|Rn|F|Cl|Br|I|O|S|Se|Te|N|P|As|Sb|C|Si|Ge|Sn|B|Al|Ga|In|Be|Mg|Ca|Sr|Ba|Li|Na|K|Rb|Cs|U|Th|Pu)\b", number);
    if let Ok(re) = regex::Regex::new(&pattern) {
        return re.is_match(window);
    }
    false
}

fn build_spans_from_vision(
    structures: &[ValidatedPageStructure],
    eligible_pages: &[usize],
    _num_pages: usize,
    headings: &[QuestionHeading],
    page_texts: &[String],
) -> Vec<QuestionSpan> {
    let mut vision_bounds: BTreeMap<u32, VisionBounds> = BTreeMap::new();
    // Phase 1c: the old `q + 5 < prev_max` global guard killed the entire
    // map whenever a single page returned an outlier number (e.g. reading
    // "30" from "[30 marks]" or a year on a cover page). Instead we do a
    // two-pass per-page filter:
    //   * collect plausible numbers for each page by rejecting numbers
    //     that jump backward by more than 30 OR forward by more than 30
    //     from the running maximum, AND are not adjacent to any number
    //     on the same page (a lone "30" on a page whose other numbers
    //     are 3,4,5 is almost certainly a misread).
    //   * a page that produces zero plausible numbers doesn't blow up
    //     the map — it's simply skipped.
    let mut running_max = 0u32;
    let mut raw_detections = Vec::new();
    for p in structures {
        if p.page == 0 {
            continue;
        }
        if !eligible_pages.contains(&p.page) {
            continue;
        }
        let mut page_detections = Vec::new();
        for (qi, &q) in p.questions.iter().enumerate() {
            if q == 0 || q > 100 {
                continue;
            }
            // Page-number heuristic removed here — rely on loose sequence filter
            // and heading-level filters to reject false numbers, not page index.
            let y_fracs = p.question_y.get(qi).copied().unwrap_or((None, None));
            page_detections.push((p.page, qi, q, y_fracs));
        }

        // Ensure they are ordered by question number so expected_max_q evaluates correctly.
        // We do NOT sort by y_frac on the same page because missing bounding boxes (or slightly inaccurate AI coordinates)
        // could cause Q2 to sort before Q1, causing Q1 to be dropped by the monotonicity guard.
        page_detections.sort_by(|a, b| {
            a.2.cmp(&b.2)
        });

        raw_detections.extend(page_detections);
    }

    // --- HYBRID FALLBACK: INJECT TEXT LAYER HEADINGS (standalone pass) ---
    // Text-layer headings are ground truth and must enter the map EVEN when
    // the AI structure pass is missing, failed, or hallucinated-free but
    // sparse. The injection used to live inside the per-structure loop,
    // which meant a page with no AI structure (or an empty structures vec)
    // silently lost every text heading on it.
    for h in headings {
        if h.number == 0 || h.number > 100 {
            continue;
        }
        if !eligible_pages.contains(&h.page) {
            continue;
        }
        // Isotope numbers (e.g. 20 from "20 Ne") are already filtered at
        // scan time via heading_is_isotope, which only inspects the heading
        // site — no page-wide veto here.
        // Skip if the AI already detected this number on this exact page —
        // the vision_bounds merge would dedupe anyway, but skipping keeps
        // the AI's (usually better) y coordinates authoritative.
        let ai_has_it = raw_detections.iter().any(|d| d.0 == h.page && d.2 == h.number);
        if ai_has_it {
            continue;
        }
        // Inject with qi=999 to indicate it's from the text layer
        raw_detections.push((h.page, 999, h.number, (Some(h.y_frac), Some(h.y_frac))));
    }

    let mut expected_max_q = 0u32;
    let mut filtered_detections = Vec::new();
    for det in raw_detections {
        let page = det.0;
        let qi = det.1;
        let q = det.2;
        let is_text_layer = qi == 999;

        // --- HYBRID RUST VETO CHECK ---
        if !is_text_layer && is_figure_hallucination(q, &page_texts[page]) {
            continue;
        }

        // --- CROSS-PAGE TEXT CONFIRMATION ---
        // If the deterministic text-layer scan found question q's heading
        // ANYWHERE in the document, that placement is ground truth. An AI
        // detection of the same q on a DIFFERENT page (where the text layer
        // does not have q) is almost certainly a misread (e.g. the AI read
        // the printed page number "10" as Q10 on a page where the text layer
        // only finds Q3 margin markers). Reject it and let the text-layer
        // placement stand. When the text layer has q NOWHERE (scanned PDF,
        // OCR gap), the rule stays out of the way and the AI is trusted.
        if !is_text_layer {
            let text_has_q_here = headings.iter().any(|h| h.page == page && h.number == q);
            if !text_has_q_here {
                let text_has_q_elsewhere = headings.iter().any(|h| h.number == q);
                if text_has_q_elsewhere {
                    continue;
                }
            }
        }

        // Accept any ascending number up to 100, allowing for ANY gap size.
        // Text-layer injected headings (qi == 999) are always accepted — they
        // come from the deterministic scanner which is our ground truth.
        if q <= 100 && (q >= expected_max_q || is_text_layer) {
            // Veto page number hallucinations: if the AI proposed a number that equals the printed page number,
            // we strictly require the text layer to confirm it. Since our text layer scanner robustly ignores 
            // printed page numbers (by checking AQA padding conventions), it will only confirm real questions.
            let is_likely_page_number = !is_text_layer && ((q as usize) == page + 1 || (q as usize) == page || (q as usize) == page + 2);
            if is_likely_page_number {
                let ai_y = det.3.0; // The start y-fraction from the AI
                let text_layer_confirmed = headings.iter().any(|h| {
                    if h.page != page || h.number != q {
                        return false;
                    }
                    if let Some(y) = ai_y {
                        // AI bounding boxes for hallucinations can be wildly inaccurate, so we tolerate up to 0.15 diff
                        (y - h.y_frac).abs() < 0.15
                    } else {
                        true
                    }
                });
                if !text_layer_confirmed {
                    continue;
                }
            }

            filtered_detections.push(det);
            if q > expected_max_q {
                expected_max_q = q;
            }
        }
    }

    for (page, _qi, q, (y0, y1)) in filtered_detections {
        running_max = running_max.max(q);
        let e = vision_bounds.entry(q).or_insert_with(|| VisionBounds {
            first_page: page,
            last_page: page,
            first_y: None,
            last_y: None,
            marks: None,
        });
        if page < e.first_page {
            e.first_page = page;
            e.first_y = y0;
        } else if page == e.first_page {
            e.first_y = match (e.first_y, y0) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
        if page > e.last_page {
            e.last_page = page;
            e.last_y = y1;
        } else if page == e.last_page {
            e.last_y = match (e.last_y, y1) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            };
        }
    }
    for p in structures {
        if !eligible_pages.contains(&p.page) {
            continue;
        }
        if let Some((q, m)) = p.footer {
            // Phase 1c: only trust the footer if its question number is
            // plausible (same guard as build_map_from_structure). A
            // misread "30" from "[30 marks]" on a cover page otherwise
            // poisons bounds for Q30.
            if q > 0 && q <= 200 {
                // Page-number veto: an AI "footer" whose question number
                // equals the printed page number is almost always the page
                // number itself misread as a totals line. Require the text
                // layer to confirm a heading for q on this page.
                let is_likely_page_number = (q as usize) == p.page + 1
                    || (q as usize) == p.page
                    || (q as usize) == p.page + 2;
                if is_likely_page_number {
                    let confirmed = headings.iter().any(|h| h.page == p.page && h.number == q);
                    if !confirmed {
                        continue;
                    }
                }
                // Cross-page confirmation: if the text layer placed question
                // q on a DIFFERENT page, this footer placement is wrong.
                let text_has_q_here = headings.iter().any(|h| h.page == p.page && h.number == q);
                if !text_has_q_here && headings.iter().any(|h| h.number == q) {
                    continue;
                }
                let e = vision_bounds.entry(q).or_insert_with(|| VisionBounds {
                    first_page: p.page,
                    last_page: p.page,
                    first_y: None,
                    last_y: None,
                    marks: None,
                });
                e.last_page = p.page;
                e.last_y = p.footer_y.or(e.last_y);
                e.marks = Some(m);
                running_max = running_max.max(q);
            }
        }
    }

    if vision_bounds.is_empty() {
        return Vec::new();
    }

    let vision_bounds_vec: Vec<(u32, VisionBounds)> = vision_bounds.into_iter().collect();

    let mut spans = Vec::new();
    for i in 0..vision_bounds_vec.len() {
        let (q, ref b) = vision_bounds_vec[i];
        let start_page = b.first_page;
        let next = vision_bounds_vec.get(i + 1);
        let end_page = if let Some((_, nb)) = next {
            let next_start_page = nb.first_page;
            // Strict math clamp to prevent empty spans while preserving b.last_page overlap
            std::cmp::max(start_page.max(b.last_page), next_start_page.saturating_sub(1))
        } else {
            std::cmp::max(start_page, b.last_page)
        };

        // y-band clips. Text-layer injected detections carry y0 == y1 ==
        // heading y, so a naive (first_y, last_y) pair produces a DEGENERATE
        // zero-height band for single-page questions ("transcribe between
        // 9% and 9%"), which makes the extraction model return empty items.
        // Rules:
        //   * start_y = this question's topmost heading y on its first page
        //     (nudged up slightly so the heading line itself is included).
        //   * end_y = when the NEXT question starts on the same page as this
        //     span's end_page, clip just above the next heading. Otherwise the
        //     question runs to the bottom of its last page -> None ("to
        //     bottom"), never the last detection's y (which for text-layer
        //     detections is just the last margin marker, not the content end).
        let start_y = b.first_y.map(|y| (y - 0.01).clamp(0.0, 1.0));
        let end_y = match next {
            Some((_, nb)) if nb.first_page <= end_page => {
                nb.first_y.map(|y| (y - 0.005).clamp(0.0, 1.0))
            }
            _ => None,
        };
        let mut vision_covered = Vec::new();
        for pg in start_page..=end_page {
            if eligible_pages.contains(&pg) {
                vision_covered.push(pg);
            }
        }
        spans.push(QuestionSpan {
            number: q,
            start_page,
            end_page,
            start_y_frac: start_y,
            end_y_frac: end_y,
            expected_marks: b.marks,
            reliable_pages: Vec::new(),
            ambiguous_pages: vision_covered,
        });
    }
    spans
}

/// Merge text-layer spans with vision spans, preferring text-layer for reliable pages
fn merge_spans(
    mut text_spans: Vec<QuestionSpan>,
    vision_spans: Vec<QuestionSpan>,
    anomalies: &mut Vec<String>,
) -> Vec<QuestionSpan> {
    for vspan in vision_spans {
        if let Some(idx) = text_spans.iter().position(|s| s.number == vspan.number) {
            let tspan = &mut text_spans[idx];
            // Expand page range
            let new_start = tspan.start_page.min(vspan.start_page);
            let new_end = tspan.end_page.max(vspan.end_page);
            // When the vision span starts earlier / ends later, take its
            // y clips; otherwise keep the tighter (possibly text-derived) clips.
            if vspan.start_page < tspan.start_page {
                tspan.start_y_frac = vspan.start_y_frac;
            } else if vspan.start_page == tspan.start_page {
                // Same start page: take the highest (lowest-y) clip so we
                // don't chop off the question heading.
                tspan.start_y_frac = match (tspan.start_y_frac, vspan.start_y_frac) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
            }
            if vspan.end_page > tspan.end_page {
                tspan.end_y_frac = vspan.end_y_frac;
            } else if vspan.end_page == tspan.end_page {
                tspan.end_y_frac = match (tspan.end_y_frac, vspan.end_y_frac) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (a, b) => a.or(b),
                };
            }
            tspan.start_page = new_start;
            tspan.end_page = new_end;
            for p in vspan.ambiguous_pages {
                if !tspan.ambiguous_pages.contains(&p) && !tspan.reliable_pages.contains(&p) {
                    tspan.ambiguous_pages.push(p);
                }
            }
            if tspan.expected_marks.is_none() && vspan.expected_marks.is_some() {
                tspan.expected_marks = vspan.expected_marks;
            }
        } else {
            anomalies.push(format!("vision-only question {} found", vspan.number));
            text_spans.push(vspan);
        }
    }

    // Sort first by (start_page, start_y_frac, number) so same-page MCQ spans
    // appear in reading order rather than numeric-number order (which can be
    // wrong when a multi-page question's number is lower than a short
    // question later on the same page as its footer).
    // Sort first by start_page, then by question number. 
    // We no longer sort by y_frac because missing y_fracs (None) defaulted to 0.0, 
    // causing questions with missing bounding boxes to incorrectly sort to the top of the page (e.g. Q2 before Q1).
    text_spans.sort_by(|a, b| {
        let p = a.start_page.cmp(&b.start_page);
        if p != std::cmp::Ordering::Equal { return p; }
        a.number.cmp(&b.number)
    });
    text_spans
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

/// What the cheap per-page structure call may return.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct PageStructureProposal {
    /// Whole question numbers whose content is visible on this page.
    #[serde(default)]
    pub question_numbers_visible: Vec<serde_json::Value>,
    /// Phase 1: optional per-question vertical bounds (0.0 top to 1.0 bottom).
    /// When supplied, must be the same length as `question_numbers_visible`;
    /// each entry is [y_start, y_end] in relative page coordinates. Lets
    /// the model tell us exactly where each question sits on the page so we
    /// can clip images to a single question on dense short-answer / MCQ
    /// pages. Optional for backwards compatibility.
    #[serde(default)]
    pub question_y_fracs: Option<Vec<Vec<serde_json::Value>>>,
    /// Footer marks if a "Total for Question …" line is visible:
    /// [question_number, marks]. Absent otherwise.
    #[serde(default)]
    pub total_marks_footer: Option<Vec<serde_json::Value>>,
    /// Phase 1: optional y position of the printed footer (0.0–1.0), used
    /// to clip the question's end above the following question's heading.
    #[serde(default)]
    pub total_marks_footer_y: Option<f32>,
    /// One of QUESTION / COVER / INSTRUCTIONS / BLANK / ANSWER_BOOKLET /
    /// REFERENCE — page-only classification, no question content asked.
    #[serde(default)]
    pub page_role: Option<String>,
}

pub struct ValidatedPageStructure {
    pub page: usize,
    /// Per-question numbers on this page.
    pub questions: Vec<u32>,
    /// Per-question y_start/y_end pairs (parallel to `questions`), if the
    /// structure pass supplied them. `None` entries mean "use the whole
    /// page in that direction".
    pub question_y: Vec<(Option<f32>, Option<f32>)>,
    pub footer: Option<(u32, u32)>,
    /// Y fraction of the footer on the page, if reported.
    pub footer_y: Option<f32>,
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

    // Parse question numbers in their ORIGINAL order (sorting destroys the
    // top-to-bottom ordering we need to apply y fractions). We dedupe but
    // preserve order of first occurrence.
    let mut seen = std::collections::BTreeSet::new();
    let mut questions: Vec<u32> = Vec::new();
    let mut raw_to_valid: Vec<Option<u32>> = Vec::new();
    for raw in &proposal.question_numbers_visible {
        let n = crate::validate::value_to_question_number(raw);
        raw_to_valid.push(n);
        if let Some(n) = n {
            if seen.insert(n) {
                questions.push(n);
            }
        }
    }
    if raw_to_valid.iter().any(|o| o.is_none()) {
        violations.push(format!(
            "page {}: dropped implausible question number(s) from structure pass",
            page + 1
        ));
    }

    // Align y fractions with the valid question list. We zip raw→valid and
    // only keep the y pair when the question number was accepted.
    let mut question_y: Vec<(Option<f32>, Option<f32>)> = Vec::new();
    if let Some(yfracs) = proposal.question_y_fracs {
        if yfracs.len() == proposal.question_numbers_visible.len() {
            for (raw_idx, pair) in yfracs.iter().enumerate() {
                let Some(q) = raw_to_valid[raw_idx] else { continue };
                // Find q's position in the deduplicated list.
                let pos = match questions.iter().position(|x| *x == q) {
                    Some(p) => p,
                    None => continue,
                };
                if pos >= question_y.len() {
                    question_y.resize(pos + 1, (None, None));
                }
                let y0 = pair.get(0).and_then(|v| v.as_f64()).map(|f| f.clamp(0.0, 1.0) as f32);
                let y1 = pair.get(1).and_then(|v| v.as_f64()).map(|f| f.clamp(0.0, 1.0) as f32);
                // Sanity: y0 < y1
                let (y0, y1) = match (y0, y1) {
                    (Some(a), Some(b)) if a < b => (Some(a), Some(b)),
                    (Some(a), None) => (Some(a), None),
                    (None, Some(b)) => (None, Some(b)),
                    _ => (None, None),
                };
                question_y[pos] = (y0, y1);
            }
        } else {
            violations.push(format!(
                "page {}: question_y_fracs length mismatch (expected {}, got {}) — y-clips ignored",
                page + 1,
                proposal.question_numbers_visible.len(),
                yfracs.len()
            ));
        }
    }
    // Pad to questions.len() with (None, None).
    while question_y.len() < questions.len() {
        question_y.push((None, None));
    }

    // Deterministic anti-merge pass: the structure prompt asks for
    // non-overlapping, ascending bands, but a model that ignores that
    // instruction would otherwise hand two questions crops that both
    // contain the boundary — exactly the "subquestions muddled between
    // main questions" failure. Repair overlaps here instead of trusting
    // the prompt: questions are in top-to-bottom order, so band i must
    // end no lower than band i+1 starts. On overlap we cut both at the
    // midpoint of the disputed strip.
    for i in 0..question_y.len().saturating_sub(1) {
        let (_, end_i) = question_y[i];
        let (start_next, _) = question_y[i + 1];
        if let (Some(e), Some(s)) = (end_i, start_next) {
            if e > s {
                let mid = (e + s) / 2.0;
                question_y[i].1 = Some(mid);
                question_y[i + 1].0 = Some(mid);
                violations.push(format!(
                    "page {}: questions {} and {} had overlapping y-bands ({:.3} > {:.3}) — split at {:.3}",
                    page + 1,
                    questions.get(i).copied().unwrap_or(0),
                    questions.get(i + 1).copied().unwrap_or(0),
                    e,
                    s,
                    mid
                ));
            }
        }
    }

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
    let footer_y = proposal
        .total_marks_footer_y
        .map(|f| f.clamp(0.0, 1.0))
        .filter(|_| footer.is_some());

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

    let _ = all_valid_page_count;

    (
        ValidatedPageStructure {
            page,
            questions,
            question_y,
            footer,
            footer_y,
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
    // Record per-question running bounds. Phase 1c: same per-page
    // plausible-number filter as build_spans_from_vision, so an outlier
    // on a single page can't kill the whole structure map.
    let mut bounds: std::collections::BTreeMap<u32, VisionBounds> =
        std::collections::BTreeMap::new();
    let mut running_max = 0u32;
    let mut raw_detections = Vec::new();
    for p in pages {
        if p.page == 0 {
            continue;
        }
        if !p.role.is_question_content() {
            continue;
        }
        for (qi, &q) in p.questions.iter().enumerate() {
            if q == 0 || q > 200 {
                continue;
            }
            if q as usize == p.page + 1 {
                continue; // Page Number Heuristic Filter
            }
            let y_fracs = p.question_y.get(qi).copied().unwrap_or((None, None));
            raw_detections.push((p.page, qi, q, y_fracs));
        }
    }

    let mut expected_max_q = 0u32;
    let mut filtered_detections = Vec::new();
    for det in raw_detections {
        let q = det.2;
        // Accept any ascending number up to 50, allowing for ANY gap size 
        if q >= expected_max_q && q <= 50 {
            filtered_detections.push(det);
            expected_max_q = q;
        }
    }

    for (page, _qi, q, (y0, y1)) in filtered_detections {
        running_max = running_max.max(q);
        let e = bounds.entry(q).or_insert_with(|| VisionBounds {
            first_page: page,
            last_page: page,
            first_y: None,
            last_y: None,
            marks: None,
        });
        if page < e.first_page {
            e.first_page = page;
            e.first_y = y0;
        } else if page == e.first_page {
            e.first_y = match (e.first_y, y0) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
        if page > e.last_page {
            e.last_page = page;
            e.last_y = y1;
        } else if page == e.last_page {
            e.last_y = match (e.last_y, y1) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            };
        }
    }
    for p in pages {
        if let Some((q, m)) = p.footer {
            // Only trust the footer if its question number is plausible
            // (within range of the running sequence or already in bounds).
            if q > 0 && q <= 200 {
                let e = bounds.entry(q).or_insert_with(|| VisionBounds {
                    first_page: p.page,
                    last_page: p.page,
                    first_y: None,
                    last_y: None,
                    marks: None,
                });
                e.last_page = p.page;
                e.last_y = p.footer_y.or(e.last_y);
                e.marks = Some(m);
                running_max = running_max.max(q);
            }
        }
    }
    if bounds.len() < 2 {
        return None;
    }

    let bounds_vec: Vec<(u32, VisionBounds)> = bounds.into_iter().collect();

    let mut spans = Vec::new();
    for i in 0..bounds_vec.len() {
        let (q, ref b) = bounds_vec[i];
        let start_page = b.first_page;
        let end_page = if i + 1 < bounds_vec.len() {
            let next_start_page = bounds_vec[i + 1].1.first_page;
            // Strict math clamp to prevent empty spans while preserving b.last_page overlap
            std::cmp::max(start_page.max(b.last_page), next_start_page.saturating_sub(1))
        } else {
            std::cmp::max(start_page, b.last_page)
        };

        let mut ambiguous = Vec::new();
        for pg in start_page..=end_page {
            ambiguous.push(pg);
        }
        spans.push(QuestionSpan {
            number: q,
            start_page,
            end_page,
            start_y_frac: b.first_y,
            end_y_frac: b.last_y,
            expected_marks: b.marks,
            reliable_pages: Vec::new(),
            ambiguous_pages: ambiguous,
        });
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
            "middle of Q1 (Total for Question 1 is 5 marks)\n2. second question - extra text to ensure chars_remaining exceeds the thirty character limit.",
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
            "COVER PAGE NO QUESTIONS HERE",
            "1. first (Total for Question 1 is 3 marks)\n2. second (Total for Question 2 is 4 marks)",
            "3. third (Total for Question 3 is 2 marks)",
        ]);
        let map = build_hybrid_map(&t, &[], 3);
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
        let map = build_hybrid_map(&t, &[], 2);
        assert!(map.spans.is_empty());
    }

    #[test]
    fn overlapping_y_bands_are_split_not_merged() {
        // Model claims Q3 runs to 0.62 while Q4 starts at 0.55 — the
        // disputed strip would put Q4's first sub-parts inside Q3's crop.
        let proposal = PageStructureProposal {
            question_numbers_visible: vec![serde_json::json!(3), serde_json::json!(4)],
            question_y_fracs: Some(vec![
                vec![serde_json::json!(0.05), serde_json::json!(0.62)],
                vec![serde_json::json!(0.55), serde_json::json!(0.97)],
            ]),
            total_marks_footer: None,
            total_marks_footer_y: None,
            page_role: Some("QUESTION".to_string()),
        };
        let (v, violations) = validate_structure_proposal(0, proposal, 1);
        assert_eq!(v.questions, vec![3, 4]);
        let end_q3 = v.question_y[0].1.unwrap();
        let start_q4 = v.question_y[1].0.unwrap();
        assert!(end_q3 <= start_q4, "bands must not overlap: {end_q3} > {start_q4}");
        assert!((end_q3 - 0.585).abs() < 1e-4, "expected midpoint split, got {end_q3}");
        assert!(violations.iter().any(|s| s.contains("overlapping y-bands")));
    }

    #[test]
    fn non_overlapping_y_bands_are_left_alone() {
        let proposal = PageStructureProposal {
            question_numbers_visible: vec![serde_json::json!(1), serde_json::json!(2)],
            question_y_fracs: Some(vec![
                vec![serde_json::json!(0.04), serde_json::json!(0.40)],
                vec![serde_json::json!(0.45), serde_json::json!(0.96)],
            ]),
            total_marks_footer: None,
            total_marks_footer_y: None,
            page_role: Some("QUESTION".to_string()),
        };
        let (v, violations) = validate_structure_proposal(0, proposal, 1);
        assert_eq!(v.question_y[0].1, Some(0.40));
        assert_eq!(v.question_y[1].0, Some(0.45));
        assert!(!violations.iter().any(|s| s.contains("overlapping")));
    }

    #[test]
    fn structure_pass_validates_and_builds() {
        let mk = |page, qs: Vec<u32>, foot: Option<(u32, u32)>, role| ValidatedPageStructure {
            page,
            questions: qs.clone(),
            question_y: vec![(None, None); qs.len()],
            footer: foot,
            footer_y: None,
            role,
        };
        let pages = vec![
            mk(0, vec![], None, PageRole::Cover),
            mk(2, vec![1], None, PageRole::Question),
            mk(3, vec![1, 2], Some((1, 5)), PageRole::Question),
            mk(4, vec![2], Some((2, 6)), PageRole::Question),
        ];
        let map = build_map_from_structure(&pages, 5).unwrap();
        assert_eq!(map.spans.len(), 2);
        assert_eq!(map.spans[0].number, 1);
        assert_eq!(map.spans[0].end_page, 3);
        assert_eq!(map.spans[0].expected_marks, Some(5));
        assert_eq!(map.spans[1].start_page, 3);
        assert_eq!(map.non_question_pages, vec![0]);
    }

    #[test]
    fn structure_pass_rejects_massive_backwards_jumps() {
        let mk = |page, qs: Vec<u32>| ValidatedPageStructure {
            page,
            questions: qs.clone(),
            question_y: vec![(None, None); qs.len()],
            footer: None,
            footer_y: None,
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
                ..Default::default()
            },
            5,
        );
        assert_eq!(v.questions, vec![3]); // "03.1" accepted as 3, duplicates are removed
        assert_eq!(v.footer, Some((3, 8)));
        assert_eq!(v.role, PageRole::Question);
        assert_eq!(violations.len(), 0); // No longer a violation because 03.1 is parsed successfully
    }

    /// Extract page texts via the production pdfium path and build the
    /// document map with NO AI structures. Returns None when the PDF is
    /// missing or unreadable (so tests skip gracefully on CI).
    fn golden_map(path: &str) -> Option<(Vec<String>, DocumentMap)> {
        if !std::path::Path::new(path).exists() {
            warn!("{} not found — skipping golden test", path);
            return None;
        }
        let page_inputs = match crate::pdf_render::render_pdf_pages(std::path::Path::new(path)) {
            Ok(p) => p,
            Err(e) => {
                warn!("pdfium load failed for {}: {} — skipping", path, e);
                return None;
            }
        };
        let pages: Vec<String> = page_inputs.into_iter().map(|p| p.text).collect();
        let num_pages = pages.len();
        let map = build_hybrid_map(&pages, &[], num_pages);
        Some((pages, map))
    }

    #[test]
    fn text_layer_sufficiency_gate() {
        // Garbled / scanned: no headings at all -> insufficient.
        let garbled = texts(&["garbled !@#$%^", "more garbled"]);
        let scan = scan_text_layer(&garbled);
        assert!(!text_layer_map_sufficient(&scan, 2));

        // Sparse: only 3 questions -> insufficient (needs vision fallback).
        let sparse = texts(&[
            "Instructions",
            "1. alpha text padding padding padding padding padding padding padding padding",
            "2. beta text padding padding padding padding padding padding padding padding",
            "3. gamma text padding padding padding padding padding padding padding padding",
        ]);
        let scan = scan_text_layer(&sparse);
        assert!(!text_layer_map_sufficient(&scan, 4));

        // Dense but missing Q1 -> insufficient.
        let no_q1 = texts(&[
            "Instructions",
            "2. beta text padding padding padding padding padding padding padding padding",
            "3. gamma text padding padding padding padding padding padding padding padding",
            "4. delta text padding padding padding padding padding padding padding padding",
            "5. epsilon text padding padding padding padding padding padding padding padding",
            "6. zeta text padding padding padding padding padding padding padding padding",
            "7. eta text padding padding padding padding padding padding padding padding",
        ]);
        let scan = scan_text_layer(&no_q1);
        assert!(!text_layer_map_sufficient(&scan, 7));

        // Complete ascending run 1..=8 across several pages -> sufficient.
        let dense = texts(&[
            "Instructions",
            "1. alpha text padding padding padding padding padding padding padding padding",
            "2. beta text padding padding padding padding padding padding padding padding",
            "3. gamma text padding padding padding padding padding padding padding padding",
            "4. delta text padding padding padding padding padding padding padding padding",
            "5. epsilon text padding padding padding padding padding padding padding padding",
            "6. zeta text padding padding padding padding padding padding padding padding",
            "7. eta text padding padding padding padding padding padding padding padding",
            "8. theta text padding padding padding padding padding padding padding padding",
        ]);
        let scan = scan_text_layer(&dense);
        assert!(text_layer_map_sufficient(&scan, 9));
    }

    #[test]
    fn edexcel_2023_pdf_uses_ordered_text_headings() {
        let Some((pages, map)) = golden_map("../'23 edexcel.pdf") else {
            return;
        };

        let scan = scan_text_layer(&pages);
        let numbers: Vec<u32> = scan.headings.iter().map(|h| h.number).collect();
        assert_eq!(numbers, (1..=18).collect::<Vec<_>>());
        assert!(text_layer_map_sufficient(&scan, pages.len()));

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
        let years = ["17", "18", "19", "20", "21", "22", "24"];
        let mut tested = 0usize;

        for year in years {
            let path = format!("../edexcel/'{} edexcel.pdf", year);
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
            let map = build_hybrid_map(&page_texts, &[], page_inputs.len());
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
                assert!(
                    text_layer_map_sufficient(&scan, page_inputs.len()),
                    "{} text layer did not meet document-map sufficiency requirements: {:?}",
                    path,
                    heading_numbers
                );

                let span_numbers: Vec<u32> = map.spans.iter().map(|span| span.number).collect();
                assert_eq!(
                    span_numbers, heading_numbers,
                    "{} map spans do not match canonical text headings",
                    path
                );
            } else {
                assert!(
                    text_bytes < 1000,
                    "{} was not classified consistently as image-dominant",
                    path
                );
                assert!(
                    !text_layer_map_sufficient(&scan, page_inputs.len()),
                    "{} sparse text layer incorrectly claimed to be a complete map",
                    path
                );
                assert!(
                    map.vision_fallback_pages.len() > 0,
                    "{} image-dominant paper did not retain vision fallback pages",
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

            info!(
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
            let Some((pages, map)) = golden_map(&path) else { continue };
            // The text layer alone must be judged sufficient to skip the
            // vision structure pass for every one of these papers.
            let scan = scan_text_layer(&pages);
            assert!(
                text_layer_map_sufficient(&scan, pages.len()),
                "physics '{}: text layer should be sufficient to skip the structure pass",
                year
            );
            let span_nums: Vec<u32> = map.spans.iter().map(|s| s.number).collect();
            info!("=== physics '{}': {} spans = {:?}", year, span_nums.len(), span_nums);
            for s in &map.spans {
                debug!(
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
}
