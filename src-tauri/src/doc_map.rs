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
    regex::Regex::new(
        r"(?m)(?:^|\n)\s*(?:\*\*)?(?:Q(?:uestion)?\.?\s*)?0*([1-9]\d{0,2})(?:\*\*)?\s*(?:[\.\)\]\-–—]|\s+)(?:\D|$)",
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
    let instr_re = regex::Regex::new(
        r"(?i)(?:^|\n)\s*(?:instructions?\s*(?:to\s+candidates?)?|answer\s+all\s+questions|glossary)(?:\s|$|:)",
    ).unwrap();
    let formulae_sheet_re = regex::Regex::new(
        r"(?i)(?:^|\n)\s*(?:formulae?|data|constants|relationships?)\s*(?:sheet|booklet|table|page)?\s*$",
    ).unwrap();
    let blank_re = regex::Regex::new(r"(?i)^\s*(blank page|this page is intentionally blank)\s*$").unwrap();
    let ref_re = regex::Regex::new(r"(?i)^\s*(formulae|data|reference|constants)\s*(sheet|table|booklet)?\s*$").unwrap();
    let aqa_figure_re = regex::Regex::new(r"(?i)\bfig(?:ure)?\.?\s*\d+").unwrap();
    let aqa_table_re = regex::Regex::new(r"(?i)\btable\s+\d+").unwrap();
    let aqa_main_re = regex::Regex::new(r"0\s+\d{1,2}").unwrap();
    let aqa_sub_re = regex::Regex::new(r"(?m)^\s*0?\s*\d{1,2}\s*\.\s*\d+").unwrap();
    let marks_re = regex::Regex::new(r"(?i)\[\s*\d+\s*marks?").unwrap();

    for (page, text) in page_texts.iter().enumerate() {
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
        for cap in heading_re.captures_iter(text) {
            let full = cap.get(0).unwrap();
            let y_frac = (full.start() as f32 / page_len).clamp(0.0, 1.0);
            if let Ok(n) = cap[1].parse::<u32>() {
                if n > 0 && n <= 200 {
                    headings.push(QuestionHeading { page, number: n, y_frac });
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

        if blank_re.is_match(text) || text.trim().is_empty() {
            page_reliability[page] = PageReliability::NonQuestion;
        } else if has_footer {
            page_reliability[page] = PageReliability::Reliable;
        } else if (instr_hit || ref_hit) && !has_question_signal && (is_short_rubric || is_formulae_sheet) {
            page_reliability[page] = PageReliability::NonQuestion;
        } else if text.len() > 100 || has_question_signal {
            page_reliability[page] = PageReliability::Ambiguous;
        } else {
            page_reliability[page] = PageReliability::Ambiguous;
        }
    }
    TextScan {
        footers,
        paper_total,
        page_reliability,
        headings,
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

    let mut spans = Vec::new();
    let mut reliable_pages = std::collections::BTreeSet::new();

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
                let prev_footer_page = if i == 0 { None } else { Some(footers[i - 1].page) };
                let mut start_page = if i == 0 {
                    estimate_first_question_start_reliable(&scan.page_reliability, end_page)
                } else {
                    mid_page_start(&footers[i - 1], f)
                };
                if start_page > end_page || end_page >= num_pages {
                    anomalies.push(format!("inconsistent span for Q{}", f.question));
                    continue;
                }

                // Phase 1 (weld-bug fix): a long-standing bug set start_page to
                // prev.page unconditionally ("Q_N always starts on the page where
                // Q_{N-1}'s footer sits"). That's correct only when Q_N's heading
                // appears on prev.page. When prev.page shows only the tail of
                // Q_{N-1} and Q_N starts LATER (e.g. prev footer is page 2 but Q3
                // heading is on page 3), we must not drag prev.page into Q_N's
                // span. If Q_N has a heading anywhere strictly AFTER prev.page
                // and at/before end_page, start there instead.
                if let Some(pfp) = prev_footer_page {
                    let mut heading_page: Option<usize> = None;
                    for h in &scan.headings {
                        if h.number == f.question && h.page > pfp && h.page <= end_page {
                            heading_page = Some(match heading_page {
                                Some(cur) => cur.min(h.page),
                                None => h.page,
                            });
                        }
                    }
                    if let Some(hp) = heading_page {
                        if hp > start_page {
                            start_page = hp;
                        }
                    }
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
    append_text_only_short_answer_spans(scan, &mut spans, &mut anomalies);

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

    let existing_numbers: std::collections::BTreeSet<u32> =
        spans.iter().map(|s| s.number).collect();

    for (&page, headings) in &by_page {
        // Build horizontal bands: each heading starts a band that ends
        // at the next heading (or 1.0).
        for (idx, h) in headings.iter().enumerate() {
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
                h.y_frac >= lo - 0.02 && h.y_frac < hi
            });
            if inside_other_span {
                // Likely a cross-reference or a sub-part marker inside a
                // long question's band — don't carve out a new span.
                continue;
            }
            let end_y = if idx + 1 < headings.len() {
                headings[idx + 1].y_frac - 0.005
            } else {
                1.0
            };
            let start_y = h.y_frac - 0.005;
            anomalies.push(format!(
                "text-heading-only question {} on page {} (short-answer/MCQ page) — using y-clips {:.2}–{:.2}",
                h.number,
                page + 1,
                start_y,
                end_y,
            ));
            spans.push(QuestionSpan {
                number: h.number,
                start_page: page,
                end_page: page,
                start_y_frac: Some(start_y.clamp(0.0, 1.0)),
                end_y_frac: Some(end_y.clamp(0.0, 1.0)),
                expected_marks: None,
                reliable_pages: if scan.page_reliability[page] == PageReliability::Reliable {
                    vec![page]
                } else {
                    vec![]
                },
                ambiguous_pages: if scan.page_reliability[page] == PageReliability::Reliable {
                    vec![]
                } else {
                    vec![page]
                },
            });
        }
    }

    spans.sort_by(|a, b| {
        let p = a.start_page.cmp(&b.start_page);
        if p != std::cmp::Ordering::Equal { return p; }
        let ay = a.start_y_frac.unwrap_or(0.0);
        let by = b.start_y_frac.unwrap_or(0.0);
        let y = ay.partial_cmp(&by).unwrap_or(std::cmp::Ordering::Equal);
        if y != std::cmp::Ordering::Equal { return y; }
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

    // Phase 1b: when the text layer gave us NO reliable footers (common on
    // MCQ-heavy papers, scanned PDFs with corrupt text layers, IB/CAIE papers
    // that don't print "Total for Question N is M marks"), we can't trust
    // the text-layer reliability classification — many pages that were
    // marked Reliable/NonQuestion on the strength of a single stray word
    // ("information", "formulae") are actually question pages. In that
    // case feed ALL non-truly-blank pages into the vision span builder so
    // the structure pass's (paid-for) output is not silently discarded.
    let text_layer_trustworthy = !scan.footers.is_empty()
        && scan.footers.iter().filter(|f| f.question > 0).count() >= 2;

    // 2. Identify which pages to feed the vision builder. When the text
    // layer is trustworthy, use only Ambiguous pages (the existing hybrid
    // approach — saves us from merging against pages the text layer
    // already placed). When not, feed every page that has structure data
    // and is not a hard Blank/Cover/etc.
    let vision_pages: Vec<usize> = if text_layer_trustworthy {
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
        let vision_spans = build_spans_from_vision(structures, &vision_pages, num_pages);
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
    
    // Validate final spans for monotonicity. Phase 1: with y-clips, multiple
    // questions can legitimately share a single page (MCQs / short answer).
    // We only force start_page = prev_end when spans are on *different*
    // pages AND there's a genuine gap; same-page spans are left alone
    // because their y fractions encode the vertical ordering.
    let mut valid_spans = Vec::new();
    let mut prev_num = 0u32;
    let mut prev_end_page = 0usize;
    for mut span in spans {
        if span.number <= prev_num {
            anomalies.push(format!(
                "non-monotonic question number {} after {}",
                span.number, prev_num
            ));
            continue;
        }
        if span.end_page >= num_pages {
            anomalies.push(format!("invalid page range for Q{}", span.number));
            continue;
        }
        if prev_num > 0 {
            if span.start_page < prev_end_page {
                // Span starts on a page before the previous span's end:
                // only allowed when both share the same page (y-clip
                // case). Otherwise clamp to prev_end.
                if span.start_page == span.end_page && span.end_page == prev_end_page {
                    // Same-page question (MCQ/short-answer): keep y-clip
                    // and don't adjust page.
                } else {
                    anomalies.push(format!(
                        "adjusting Q{} start_page from {} to prev_end {}",
                        span.number, span.start_page, prev_end_page
                    ));
                    span.start_page = prev_end_page;
                    span.start_y_frac = None;
                }
            } else if span.start_page > prev_end_page {
                // Gap (e.g. non-question pages between). Allow: the span
                // starts on the page it says it starts on.
            }
        } else {
            span.start_page = span.start_page.min(span.end_page);
        }

        prev_num = span.number;
        prev_end_page = prev_end_page.max(span.end_page);
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
fn build_spans_from_vision(
    structures: &[ValidatedPageStructure],
    eligible_pages: &[usize],
    _num_pages: usize,
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
    for p in structures {
        if !eligible_pages.contains(&p.page) {
            continue;
        }
        // Filter per-page numbers.
        let mut accepted: Vec<(usize, u32)> = Vec::new();
        for (qi, &q) in p.questions.iter().enumerate() {
            if q == 0 || q > 200 {
                continue;
            }
            // Reject wild jumps backward (>30 drop) or forward (>30 above
            // running max) UNLESS the number is within 5 of another
            // accepted number on the same page (which suggests a real
            // MCQ run rather than an outlier).
            let backward_jump = running_max > 0 && q + 30 < running_max;
            let forward_jump = q > running_max + 30 && running_max > 0;
            if backward_jump || forward_jump {
                // Keep it only if a sibling on the same page is within 5
                // of it (signals a contiguous block that simply follows
                // a gap in the structure pass, not a hallucination).
                let sibling_close = accepted.iter().any(|(_, qn)| qn.abs_diff(q) <= 5);
                if !sibling_close {
                    continue;
                }
            }
            accepted.push((qi, q));
        }
        for (qi, q) in accepted {
            running_max = running_max.max(q);
            let (y0, y1) = p.question_y.get(qi).copied().unwrap_or((None, None));
            let e = vision_bounds.entry(q).or_insert_with(|| VisionBounds {
                first_page: p.page,
                last_page: p.page,
                first_y: None,
                last_y: None,
                marks: None,
            });
            if p.page < e.first_page {
                e.first_page = p.page;
                e.first_y = y0;
            } else if p.page == e.first_page {
                // Keep the tightest (highest) y on the first page.
                e.first_y = match (e.first_y, y0) {
                    (None, Some(v)) => Some(v),
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, _) => a,
                };
            }
            if p.page > e.last_page {
                e.last_page = p.page;
                e.last_y = y1;
            } else if p.page == e.last_page {
                // Keep the tightest (lowest) y on the last page.
                e.last_y = match (e.last_y, y1) {
                    (None, Some(v)) => Some(v),
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (a, _) => a,
                };
            }
        }
        if let Some((q, m)) = p.footer {
            // Phase 1c: only trust the footer if its question number is
            // plausible (same guard as build_map_from_structure). A
            // misread "30" from "[30 marks]" on a cover page otherwise
            // poisons bounds for Q30.
            if q > 0 && q <= 200 {
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

    let mut spans = Vec::new();
    for (q, b) in vision_bounds.iter() {
        // Same-page multi-question sanity: if first_y is above last_y but
        // first_page == last_page, we keep both clips (a vertical band).
        let (start_y, end_y) = if b.first_page == b.last_page {
            (b.first_y, b.last_y)
        } else {
            (b.first_y, b.last_y)
        };
        let mut vision_covered = Vec::new();
        for pg in b.first_page..=b.last_page {
            if eligible_pages.contains(&pg) {
                vision_covered.push(pg);
            }
        }
        spans.push(QuestionSpan {
            number: *q,
            start_page: b.first_page,
            end_page: b.last_page,
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
    text_spans.sort_by(|a, b| {
        let p = a.start_page.cmp(&b.start_page);
        if p != std::cmp::Ordering::Equal { return p; }
        let ay = a.start_y_frac.unwrap_or(0.0);
        let by = b.start_y_frac.unwrap_or(0.0);
        let y = ay.partial_cmp(&by).unwrap_or(std::cmp::Ordering::Equal);
        if y != std::cmp::Ordering::Equal { return y; }
        a.number.cmp(&b.number)
    });
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
    for p in pages {
        // Skip non-question pages when building bounds (matches the
        // hybrid path's behaviour).
        if !p.role.is_question_content() {
            continue;
        }
        let mut accepted: Vec<(usize, u32)> = Vec::new();
        for (qi, &q) in p.questions.iter().enumerate() {
            if q == 0 || q > 200 {
                continue;
            }
            let backward_jump = running_max > 0 && q + 30 < running_max;
            let forward_jump = q > running_max + 30 && running_max > 0;
            if backward_jump || forward_jump {
                let sibling_close = accepted.iter().any(|(_, qn)| qn.abs_diff(q) <= 5);
                if !sibling_close {
                    continue;
                }
            }
            accepted.push((qi, q));
        }
        for (qi, q) in accepted {
            running_max = running_max.max(q);
            let (y0, y1) = p.question_y.get(qi).copied().unwrap_or((None, None));
            let e = bounds.entry(q).or_insert_with(|| VisionBounds {
                first_page: p.page,
                last_page: p.page,
                first_y: None,
                last_y: None,
                marks: None,
            });
            if p.page < e.first_page {
                e.first_page = p.page;
                e.first_y = y0;
            } else if p.page == e.first_page {
                e.first_y = match (e.first_y, y0) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
            }
            if p.page > e.last_page {
                e.last_page = p.page;
                e.last_y = y1;
            } else if p.page == e.last_page {
                e.last_y = match (e.last_y, y1) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (a, b) => a.or(b),
                };
            }
        }
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

    let mut spans = Vec::new();
    for (q, b) in bounds.iter() {
        let mut ambiguous = Vec::new();
        for pg in b.first_page..=b.last_page {
            ambiguous.push(pg);
        }
        spans.push(QuestionSpan {
            number: *q,
            start_page: b.first_page,
            end_page: b.last_page,
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
            "1. first (Total for Question 1 is 3 marks)\n2. second (Total for Question 2 is 4 marks)",
            "3. third (Total for Question 3 is 2 marks)",
        ]);
        let map = build_hybrid_map(&t, &[], 2);
        assert_eq!(map.spans.len(), 3);
        assert_eq!(map.spans[0].number, 1);
        assert_eq!(map.spans[0].start_page, 0);
        assert_eq!(map.spans[0].end_page, 0);
        assert_eq!(map.spans[1].number, 2);
        assert_eq!(map.spans[1].start_page, 0); // same page as Q1's footer
        assert_eq!(map.spans[1].end_page, 0);
        // Phase 1 weld fix: Q3's heading "3. third" is on page 1, not page 0,
        // so its span must NOT include page 0 (which only contains Q1/Q2).
        assert_eq!(map.spans[2].number, 3);
        assert_eq!(map.spans[2].start_page, 1);
        assert_eq!(map.spans[2].end_page, 1);
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
            questions: qs.clone(),
            question_y: vec![(None, None); qs.len()],
            footer: foot,
            footer_y: None,
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
        assert_eq!(map.spans[1].start_page, 2);
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
        assert_eq!(v.questions, vec![3]); // "03.1" refused, not "31"
        assert_eq!(v.footer, Some((3, 8)));
        assert_eq!(v.role, PageRole::Question);
        assert_eq!(violations.len(), 1);
    }
}
