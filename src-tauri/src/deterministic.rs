// ── Tier-0 deterministic extraction ────────────────────────────────────────
//
// Zero-cost import cascade, work item #1: replaces the LLM text-first call
// with a LOCAL Rust transcriber for text-reliable, figure-free question
// spans. The carved + converted transcription flows through the UNCHANGED
// acceptance seam (`pipeline::build_question_from_parsed_page`), so Tier 0
// inherits every future validator improvement for free. Any hard-gate
// refusal escalates invisibly to the LLM text-first path (today's status
// quo) — Tier 0 can only ever save calls, never produce a wrong card.

use crate::doc_map::QuestionSpan;
use crate::pipeline::{BuiltQuestion, ImportReport, PageInput, PipelineConfig};
use crate::validate;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;
use std::time::Instant;

// ── Boundary evidence (self-contained regexes; NOT coupled to doc_map) ──────

/// Numbered "(Total for Question N is M marks)" footer family (strongest
/// boards: Edexcel / AQA / OCR variants). Weak numberless footers are NOT
/// used here: a Tier-0 carve must know exactly WHICH question ended.
static NUMBERED_FOOTER_RES: LazyLock<Vec<regex::Regex>> = LazyLock::new(|| {
    [
        r"(?i)\(?\s*Total\s+for\s+Question\s+(\d{1,2})\s+is\s+(\d{1,2})\s+marks?\s*\)?",
        r"(?i)\(?\s*Total\s+for\s+Question\s+(\d{1,2})\s*(?:is\s*|=)\s*(\d{1,2})\s*marks?\s*\)?",
        r"(?i)\(?\s*Total\s+(?:for\s+Question\s+(\d{1,2})\s+)?(?:is\s+)?(\d{1,2})\s+marks?\s*\)?",
    ]
    .iter()
    .map(|re| regex::Regex::new(re).unwrap())
    .collect()
});

static PAPER_TOTAL_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)(?:TOTAL\s+(?:FOR|MARKS\s+FOR)?\s+(?:THIS\s+)?PAPER\s*(?:IS|:|=)?\s*|MAXIMUM\s+MARK\s*:\s*)(\d{1,3})\s*(?:MARKS)?",
    )
    .unwrap()
});

static END_OF_PAPER_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)^\s*\**\s*end\s+of\s+(?:questions?|examination|paper|section)\b").unwrap()
});

/// Line-anchored detector for ANY whole-number question heading; captures the
/// (possibly space-separated, AQA-style) number. Mirrors doc_map's guard
/// against AQA decimal sub-parts ("03.1"): a trailing digit right after the
/// separator rejects the match.
static GENERIC_HEADING_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"^[ \t]*(?:\*+)?[ \t]*(?:(?:box|Section\s+[A-Z0-9]+)[ \t]+)?(?:Q(?:uestion)?\.?[ \t]*)?(?:\*+)?[ \t]*0*[ \t]*([1-9](?:[ \t]*\d){0,2})(?:\*+)?[ \t]*(?:[\.\)\]\-–—:]|[ \t]+|$)(?:[^\d\r\n]|$)",
    )
    .unwrap()
});

/// Line-anchored heading regex for ONE specific number (digits optionally
/// space-separated, optional AQA leading-zero padding).
fn heading_line_re(n: u32) -> regex::Regex {
    let num = n
        .to_string()
        .chars()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join(r"[ \t]*");
    let pat = format!(
        r"^[ \t]*(?:\*+)?[ \t]*(?:(?:box|Section\s+[A-Z0-9]+)[ \t]+)?(?:Q(?:uestion)?\.?[ \t]*)?(?:\*+)?[ \t]*0*[ \t]*({})(?:[\.\)\]\-–—:]|[ \t]+|$)",
        num
    );
    regex::Regex::new(&pat).unwrap()
}

/// A real heading continues with substantial content starting with an
/// uppercase letter or "(" — "15 °C?", "9 d", "5 C of charge" are physics
/// debris, not questions. AQA's margin boilerplate sometimes glues AFTER
/// the number ("0 2 box The Global…") — an optional leading "box" is
/// skipped before the check.
fn heading_tail(line: &str, num_end: usize) -> Option<&str> {
    const SEPS: &[char] = &[' ', '\t', '.', ')', ']', ':', '-', '–', '—', '*', '>'];
    let mut s = line[num_end..].trim_start_matches(SEPS);
    if s.get(..3).is_some_and(|w| w.eq_ignore_ascii_case("box")) && matches!(s.as_bytes().get(3), Some(b' ' | b'\t'))
    {
        s = s[3..].trim_start();
    }
    let c = s.chars().next()?;
    static QUANTITY: LazyLock<regex::Regex> = LazyLock::new(||
        regex::Regex::new(r"^\d+(?:\.\d+)?[ \t]+(?:kW|W|kJ|J|kg|m|s|V|A|N|C|K|Hz)\b").unwrap());
    if c.is_uppercase() || c == '(' || QUANTITY.is_match(s) || glued_nuclide_prefix(s) {
        Some(s)
    } else {
        None
    }
}

/// A glued nuclide prefix ("27Mg", "3He", "238U") straight after the heading
/// number — AQA prints these ("box 3 1 27Mg 12 can decay …"). The element list
/// is shared with `doc_map` so both agree on what may follow a heading.
fn glued_nuclide_prefix(s: &str) -> bool {
    static RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(&format!(
            r"^\d{{1,3}}(?:{})\b",
            crate::doc_map::NUCLIDE_ELEMENTS
        ))
        .unwrap()
    });
    RE.is_match(s)
}

/// Heading-decorator parentheticals ("(*****)", "(***)") are difficulty
/// ratings, not question text: the stem follows on the next line, so they are
/// too short for the usual tail-length guard.
static HEADING_DECOR_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^\(\s*\*{1,8}\s*\)$").unwrap());

/// Tail evidence for a heading: substantial question text, or a rating
/// parenthetical that stands in for it.
fn tail_is_heading_evidence(tail: &str) -> bool {
    tail.chars().count() >= 12 || HEADING_DECOR_RE.is_match(tail.trim())
}

/// Nuclide DATA rows ("Ne 10 19.99244", "O 8 16.99913") open with a number and
/// an uppercase element symbol, which looks exactly like a heading — but the
/// element symbol is separated from its number and more numbers follow. They
/// are table data, never a question boundary. (Without this guard, the row
/// "20 Ne 10 19.99244" inside a table truncates the question that owns it.)
fn nuclide_data_row(tail: &str) -> bool {
    static RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(&format!(
            r"^(?:{})\b[ \t]+\d",
            crate::doc_map::NUCLIDE_ELEMENTS
        ))
        .unwrap()
    });
    RE.is_match(tail.trim_start())
}

fn heading_number_on_line(line: &str) -> Option<u32> {
    // Match the heading separately from a numeric stem; the generic digit
    // run would otherwise absorb the first digit of "1 9 4.8 kW h ...".
    static NUMERIC_STEM: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(
        r"^[ \t]*(?:box[ \t]+)?([1-9][ \t]?\d)[ \t]+(?:box[ \t]+)?(\d+(?:\.\d+)?[ \t]+(?:kW|W|kJ|J|kg|m|s|V|A|N|C|K|Hz)\b.*)"
    ).unwrap());
    if let Some(caps) = NUMERIC_STEM.captures(line) {
        if caps[2].chars().count() >= 12 {
            return caps[1].replace(' ', "").parse().ok();
        }
    }
    let caps = GENERIC_HEADING_RE.captures(line)?;
    let num = caps.get(1)?;
    let m: u32 = num.as_str().replace(' ', "").parse().ok()?;
    let trimmed = line.trim();
    if (trimmed.starts_with('*') && trimmed.ends_with('*')) || trimmed.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    const SEPS: &[char] = &[' ', '\t', '.', ')', ']', ':', '-', '–', '—', '>'];
    let after_num = line[num.end()..].trim_matches(SEPS);
    if after_num.is_empty() && (line.contains('.') || line.contains(')') || line.to_ascii_lowercase().contains("question") || line.contains('Q')) {
        return Some(m);
    }
    let tail = heading_tail(line, num.end())?;
    if nuclide_data_row(tail) {
        return None;
    }
    if !tail_is_heading_evidence(tail) {
        return None;
    }
    Some(m)
}

// ── Page-furniture stripping ────────────────────────────────────────────────

const HEADER_ZONE_LINES: usize = 5;
const FOOTER_ZONE_LINES: usize = 3;

static BOARD_HEADER_RES: LazyLock<Vec<regex::Regex>> = LazyLock::new(|| {
    [
        // Board name / branding lines ("Pearson Edexcel Level 3 GCE", "AQA", "PMT")
        r"(?i)^\s*(?:[©\xa9]\s*)?(?:pearson\s+)?(?:edexcel\b|aqa\b|ocr\b|wjec\b|eduqas\b|cambridge\b|ucles\b|pmt\b).{0,70}$",
        // Paper codes ("P67097A", "PHY 1234/01"). Two shapes only, because the
        // old single pattern also matched numeric MCQ options: the spaced form
        // REQUIRES the slash, and the glued form has no space at all. A bare
        // "A 500" / "B 250" option line must never be treated as a board code.
        r"(?i)^\s*[a-z]{1,4}\d{3,5}[a-z0-9]{0,3}\s*$",
        r"(?i)^\s*[a-z]{1,4}\s+\d{3,5}\s*/\s*[a-z0-9]{0,6}\s*$",
        // Subject name alone
        r"(?i)^\s*(?:physics|mathematics|maths|chemistry|biology|further\s+maths|pure\s+mathematics)\s*$",
    ]
    .iter()
    .map(|re| regex::Regex::new(re).unwrap())
    .collect()
});

static PAGE_NOISE_RES: LazyLock<Vec<regex::Regex>> = LazyLock::new(|| {
    [
        r"(?i)^\s*PMT\s*$", // branding stays furniture even before trailing blank lines
        r"(?i)^\s*(?:https?://)?(?:www\.)?[a-z0-9\-]+(?:\.[a-z0-9\-]+)*(?:\.com|\.org|\.education|\.co\.uk|\.net)\b\S*\s*$",
        r"^\s*[._\-\u2026]{1,}\s*$",
        r"(?i)^\s*[©\xa9]\s*ucles\b.{0,60}$",
        r"(?i)^\s*(?:DO NOT WRITE ON THIS PAGE|ANSWER IN THE SPACES PROVIDED)\s*$",
        r"^\s*\d{1,3}\s*$",                                   // bare page number
        r"^\s*\*{1,}\s*$",                                    // barcode star runs
        r"(?i)^\s*\**\s*do\s+not\s+write\s+outside\s+the\s+box\s*\**\s*$",
        // Glued/mid-column variants of AQA's margin warning leak through the
        // split-line forms below when pdf_extract interleaves columns.
        r"(?i)^\s*\**\s*do\s+not\s+write\b.{0,50}$",
        r"(?i)^\s*.{0,20}outside\s+the\s+box\b.{0,30}$",
        r"(?i)^\s*\**\s*do\s+not\s+write\s*,?\s*$",
        r"(?i)^\s*outside\s+the\s*$",
        r"(?i)^\s*box\s*$",
        r"(?i)^\s*turn\s+over\s*>?+\s*$",
        r"(?i)^\s*turn\s+over\s+for\s+the\s+next\s+question(?:\s+box)?\s*$",
        r"(?i)^\s*\**\s*question\s+\d+\s+continues?\s+on\s+the\s+next\s+page\s*\**\s*$",
        r"(?i)^\s*page\s+\d+\s+of\s+\d+\s*$",
        r"(?i)^\s*centre\s+(?:number\s+)?candidate.{0,40}$",
        r"(?i)^\s*candidate(?:\s+(?:surname|number|name)){0,3}\s*$",
        // Orphan margin sub-part indices: AQA prints "(i)", "(ii)", "(h)" in
        // the right margin and pdf_extract interleaves them as standalone
        // lines mid-question. A REAL sub-part label always carries its body
        // text; if we ever strip a genuine label the subpart-contiguity gate
        // refuses the carve, so this can never produce a wrong card.
        r"^\s*\*{0,2}\(\s*[a-h]\s*\)\*{0,2}\s*$",
        r"^\s*\*{0,2}\(\s*(?:i{1,3}|iv|v|vi{1,3}|ix|x)\s*\)\*{0,2}\s*$",
    ]
    .iter()
    .map(|re| regex::Regex::new(re).unwrap())
    .collect()
});

fn is_furniture(line: &str, zone: Option<(usize, usize, usize)>) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    if PAGE_NOISE_RES.iter().any(|re| re.is_match(t)) {
        return true;
    }
    if let Some((idx, total, zone_size)) = zone {
        let in_header = idx < zone_size;
        let in_footer = idx >= total.saturating_sub(zone_size);
        if (in_header || in_footer)
            && BOARD_HEADER_RES.iter().any(|re| re.is_match(t))
        {
            return true;
        }
    }
    false
}

fn strip_furniture(lines: &[&str]) -> Vec<String> {
    let total = lines.len();
    lines
        .iter()
        .enumerate()
        .filter(|(idx, l)| {
            let zone = Some((*idx, total, HEADER_ZONE_LINES.max(FOOTER_ZONE_LINES)));
            !is_furniture(l, zone)
        })
        .map(|(_, l)| l.to_string())
        .collect()
}

/// True when the whole line is margin/page furniture ("Do not write outside
/// the box", orphan "(ii)" indices, bare page numbers).
fn is_ghost_line(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty() && PAGE_NOISE_RES.iter().any(|re| re.is_match(t))
}

// ── Span carving ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CarvedSpan {
    pub text: String,
    pub found_start: bool,
    pub found_end: bool,
    pub footer_marks: Option<u32>,
    pub subpart_numbers: Vec<u32>,
}

static SUBPART_PAREN_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^\(([a-h])\)").unwrap());

static DECIMAL_LABEL_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^((?:\d[ \t]*){1,2})(?:box[ \t]*)?\.[ \t]*(\d{1,2})(?:\D|$)").unwrap());

/// Optional whole-number heading prefix glued onto a content line
/// ("7. (a) State …", "Q3 (b) …").
static LEADING_HEADING_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"^(?:\*+)?[ \t]*(?:Q(?:uestion)?\.?[ \t]*)?(?:\*+)?[ \t]*0*[ \t]*\d(?:[ \t]*\d){0,2}[ \t]*[\.\)\]]?[ \t]+",
    )
    .unwrap()
});

fn subpart_label_on_line(line: &str, span_number: u32) -> Option<u32> {
    let t = line.trim_start();
    if let Some(caps) = DECIMAL_LABEL_RE.captures(t) {
        let main: u32 = caps[1].split_whitespace().collect::<String>().parse().unwrap_or(0);
        let part: u32 = caps[2].parse().unwrap_or(0);
        if main == span_number && (1..=20).contains(&part) {
            return Some(part);
        }
    }
    let mut rest = t.trim_start_matches('*');
    if let Some(m) = LEADING_HEADING_RE.find(rest) {
        rest = &rest[m.end()..];
    }
    let rest = rest.trim_start_matches(['*', '.', ')', ']', ':', ' ', '\t']);
    // A roman part can open the line its first lettered sub-part is on
    // ("(ii) (a) Prove that …").
    static ROMAN_LEAD_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^\((?:i{1,3}|iv|vi{0,3}|ix|x)\)[ \t]+\(").unwrap());
    let rest = match ROMAN_LEAD_RE.find(rest) {
        Some(m) => &rest[m.end() - 1..],
        None => rest,
    };
    if let Some(caps) = SUBPART_PAREN_RE.captures(rest) {
        let letter = caps[1].as_bytes()[0];
        return Some((letter - b'a' + 1) as u32);
    }
    if let Some(caps) = DECIMAL_LABEL_RE.captures(rest) {
        let main: u32 = caps[1].split_whitespace().collect::<String>().parse().unwrap_or(0);
        let part: u32 = caps[2].parse().unwrap_or(0);
        if main == span_number && (1..=20).contains(&part) {
            return Some(part);
        }
    }
    None
}

fn heading_line_matches(line: &str, re: &regex::Regex) -> bool {
    let Some(caps) = re.captures(line) else {
        return false;
    };
    let Some(num) = caps.get(1) else {
        return false;
    };
    let trimmed = line.trim();
    if (trimmed.starts_with('*') && trimmed.ends_with('*')) || trimmed.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    const SEPS: &[char] = &[' ', '\t', '.', ')', ']', ':', '-', '–', '—', '>'];
    let after_num = line[num.end()..].trim_matches(SEPS);
    if after_num.is_empty() && (line.contains('.') || line.contains(')') || line.to_ascii_lowercase().contains("question") || line.contains('Q')) {
        return true;
    }
    match heading_tail(line, num.end()) {
        Some(tail) => !nuclide_data_row(tail) && tail_is_heading_evidence(tail),
        None => false,
    }
}

/// Decimal sub-part heading for THIS question ("0 5 . 1 Calculate …"):
/// the only start signal on pages where pdfium dropped the whole-number
/// heading. Requires a substantial uppercase tail like the whole-heading
/// guard.
fn decimal_heading_here(line: &str, span_number: u32) -> bool {
    let t = line.trim_start();
    let Some(caps) = DECIMAL_LABEL_RE.captures(t) else {
        return false;
    };
    let main: u32 = caps[1].split_whitespace().collect::<String>().parse().unwrap_or(0);
    if main != span_number {
        return false;
    }
    matches!(heading_tail(t, caps.get(2).unwrap().end()), Some(tail) if tail.chars().count() >= 12)
}

fn collect_subparts(text: &str, span_number: u32) -> Vec<u32> {
    let mut nums = Vec::new();
    for line in text.lines() {
        if let Some(p) = subpart_label_on_line(line, span_number) {
            nums.push(p);
        }
    }
    nums.sort_unstable();
    nums.dedup();
    nums
}

/// Line-based boundary parser over the span's page texts (`texts[i]` is the
/// text of page `start_page + i`).
///
/// No pre-slicing by y: doc_map's y_fracs are byte-ratio proxies while a
/// line slice would be line-count based — the two disagree enough to cut
/// real headings out. Instead the heading is located by exact-number match
/// (optionally disambiguated with the y hint, ±0.2 tolerance) and the end is
/// decided by STRONG terminators only: the printed footer for this question,
/// a paper-total line, an end-of-paper marker, or the next whole-number
/// heading. Footer regexes run over the WHOLE page (pdfium/pdf_extract often
/// split "(Total for Question 9 is 5 marks)" across lines; `\s+` bridges).
pub fn carve_span(
    span: &QuestionSpan,
    texts: &[&str],
    allow_run_to_document_end: bool,
) -> CarvedSpan {
    let no_start = CarvedSpan {
        text: String::new(),
        found_start: false,
        found_end: false,
        footer_marks: None,
        subpart_numbers: Vec::new(),
    };
    if texts.is_empty() {
        return no_start;
    }

    // Furniture-stripped lines per page + per-page base index in the global
    // line vector + each page re-joined for whole-page regex matching.
    let mut lines: Vec<String> = Vec::new();
    let mut page_bases: Vec<usize> = Vec::with_capacity(texts.len());
    let mut page_texts_joined: Vec<String> = Vec::with_capacity(texts.len());
    for raw in texts.iter() {
        let raw = crate::validate::clean_ligatures(raw).replace('\r', "");
        let refs: Vec<&str> = raw.lines().collect();
        // Stacked nuclides MUST be merged before furniture stripping: the
        // mass number ("238") sits alone on its line and the bare page-number
        // noise regex would delete it, scrambling $^{238}_{92}U$ into "92 U".
        let merged = merge_stacked_nuclides(&refs);
        let borrowed: Vec<&str> = merged.iter().map(String::as_str).collect();
        let stripped = strip_furniture(&borrowed);
        page_bases.push(lines.len());
        let joined = stripped.join("\n");
        page_texts_joined.push(joined.clone());
        lines.extend(stripped);
    }

    // ── Start: first exact-number heading line. ────────────────────────────
    let heading_re = heading_line_re(span.number);
    let heading_here = |l: &str| heading_line_matches(l, &heading_re);
    let find_whole_heading = || -> Option<usize> {
        if let Some(y) = span.start_y_frac {
            // Prefer a match on the FIRST page at/below the y hint (±0.2);
            // fall back to the first match anywhere.
            let total_first = (page_bases.get(1).copied().unwrap_or(lines.len()))
                .saturating_sub(page_bases[0])
                .max(1);
            let mut fallback = None;
            let first_end = page_bases.get(1).copied().unwrap_or(lines.len());
            for (off, l) in lines[page_bases[0]..first_end].iter().enumerate() {
                if !heading_here(l) {
                    continue;
                }
                let i = page_bases[0] + off;
                if fallback.is_none() {
                    fallback = Some(i);
                }
                let frac = off as f32 / total_first as f32;
                if frac + 0.2 >= y {
                    return Some(i);
                }
            }
            fallback.or_else(|| lines.iter().position(|l| heading_here(l)))
        } else {
            lines.iter().position(|l| heading_here(l))
        }
    };
    let start_idx = match find_whole_heading() {
        Some(i) => i,
        None => {
            // No whole-number heading in the text layer (pdfium dropped
            // it): a decimal sub-part label for THIS question ("0 5 . 1
            // Calculate …") is the only start signal left.
            match lines.iter().position(|l| decimal_heading_here(l, span.number)) {
                Some(i) => i,
                None => return no_start,
            }
        }
    };

    // ── End: earliest strong terminator after the start. ──────────────────
    let line_of_page_offset = |page: usize, offset: usize| -> Option<usize> {
        let base = *page_bases.get(page)?;
        let joined = page_texts_joined.get(page)?;
        if offset > joined.len() {
            return None;
        }
        Some(base + joined[..offset].matches('\n').count())
    };
    let mut candidates: Vec<(usize, Option<u32>)> = Vec::new();
    for (p, joined) in page_texts_joined.iter().enumerate() {
        for re in NUMBERED_FOOTER_RES.iter() {
            for caps in re.captures_iter(joined) {
                let q: u32 = caps.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
                let mk: u32 = caps.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
                if q == span.number && mk > 0 {
                    if let Some(line) = line_of_page_offset(p, caps.get(0).unwrap().start()) {
                        candidates.push((line, Some(mk)));
                    }
                }
            }
        }
        for m in PAPER_TOTAL_RE.find_iter(joined) {
            if let Some(line) = line_of_page_offset(p, m.start()) {
                candidates.push((line, None));
            }
        }
    }
    for (off, l) in lines[start_idx + 1..].iter().enumerate() {
        let i = start_idx + 1 + off;
        if END_OF_PAPER_RE.is_match(l) {
            candidates.push((i, None));
            break;
        }
        if let Some(m) = heading_number_on_line(l) {
            if m > span.number {
                candidates.push((i, None));
                break;
            }
        }
    }
    let terminator = candidates.into_iter().filter(|(l, _)| *l > start_idx).min_by_key(|(l, _)| *l);

    let (end_exclusive, found_end, footer_marks) = match terminator {
        Some((line, mk)) => (line, true, mk),
        None => match self_terminating_end(span, &lines, &page_bases, allow_run_to_document_end) {
            Some(line) => (line, true, None),
            None => (lines.len(), false, None),
        },
    };

    let text = lines[start_idx..end_exclusive].join("\n");
    let subpart_numbers = collect_subparts(&text, span.number);
    CarvedSpan {
        text,
        found_start: true,
        found_end,
        footer_marks,
        subpart_numbers,
    }
}

/// No strong terminator on the span's pages: doc_map's span contract says
/// the question owns everything up to its end_page UNLESS a next heading
/// shares that page — and in that case `end_y_frac` is `Some(y)` marking the
/// boundary. The y proxy is byte-based while lines are line-based, so the
/// cut is only trusted near an obvious boundary (blank line / option /
/// sub-part label / marks tag / terminal punctuation); otherwise decline.
fn self_terminating_end(
    span: &QuestionSpan,
    lines: &[String],
    page_bases: &[usize],
    allow_run_to_document_end: bool,
) -> Option<usize> {
    if allow_run_to_document_end {
        return Some(lines.len());
    }
    let last_page_idx = span.end_page.checked_sub(span.start_page)?;
    if last_page_idx >= page_bases.len() {
        return None;
    }
    let base = page_bases[last_page_idx];
    let last_page_lines = lines.len().saturating_sub(base);
    match span.end_y_frac {
        None => Some(lines.len()),
        Some(y) if y >= 0.85 => Some(lines.len()),
        Some(y) => {
            let cut = base
                + ((last_page_lines as f32 * (y + 0.05).min(1.0)).ceil() as usize)
                    .min(last_page_lines);
            let boundary_ish = |i: usize| -> bool {
                let Some(l) = lines.get(i) else { return false };
                let t = l.trim();
                t.is_empty()
                    || t.starts_with('(')
                    || DECIMAL_LABEL_RE.is_match(t.trim_start_matches('*'))
                    || MARK_TAG_RE.is_match(t)
                    || prev_ends_terminal(t)
            };
            for i in cut..=(cut + 4).min(lines.len().saturating_sub(1)) {
                if i >= base && boundary_ish(i) {
                    return Some(i);
                }
            }
            None
        }
    }
}

// ── Text normalization pipeline ─────────────────────────────────────────────

// ── Stacked nuclide recovery ────────────────────────────────────────────────
//
// The text layer prints $^{238}_{92}\text{U}$ as TWO lines:
//     238
//     92 U
// The mass number alone on a line would be deleted as page-number furniture,
// and the single-line TEXTUAL_NUCLIDE_RE could never see the pair. Merging
// happens BEFORE furniture stripping and reuses the existing "238/92 U"
// textual conversion downstream.

/// Common nuclide symbols, two-character entries FIRST so longest-match wins.
const ELEMENT_SYMBOLS: &[&str] = &[
    "He", "Li", "Be", "Ne", "Na", "Mg", "Al", "Si", "Cl", "Ar", "Ca", "Sc", "Ti", "Cr", "Mn",
    "Fe", "Ni", "Co", "Cu", "Zn", "Ga", "Ge", "As", "Se", "Br", "Kr", "Rb", "Sr", "Zr", "Nb",
    "Mo", "Tc", "Ru", "Rh", "Pd", "Ag", "Cd", "In", "Sn", "Sb", "Te", "Xe", "Cs", "Ba", "La",
    "Pb", "Bi", "Po", "At", "Rn", "Fr", "Ra", "Ac", "Th", "Pa", "Np", "Pu", "Am", "Cm", "Hf",
    "Ta", "Re", "Os", "Ir", "Pt", "Au", "Hg", "Tl", "Eu", "Gd", "Tb", "Dy", "Ho", "Er", "Tm",
    "Yb", "Lu", "Pr", "Nd", "Pm", "Sm", "Tb",
    "H", "B", "C", "N", "O", "F", "P", "S", "K", "V", "Y", "I", "U", "W",
];

/// `(atomic_number, symbol, remainder)` when a line begins with an atomic
/// number followed by an element symbol at a word boundary. The boundary
/// check ("92 U" yes, "5 Complete" no — `Co` is followed by more letters)
/// keeps prose from being read as nuclides.
fn atomic_element_on(line: &str) -> Option<(u32, &str, String)> {
    let t = line.trim_start();
    let digit_len = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digit_len == 0 || digit_len > 3 {
        return None;
    }
    let z: u32 = t[..digit_len].parse().ok()?;
    let after = t[digit_len..].trim_start();
    for sym in ELEMENT_SYMBOLS {
        if let Some(rest) = after.strip_prefix(sym) {
            let boundary = rest
                .chars()
                .next()
                .map(|c| !c.is_alphanumeric())
                .unwrap_or(true);
            if boundary {
                return Some((z, sym, rest.to_string()));
            }
        }
    }
    None
}

/// Merge "mass-number line + atomic-number/element line" pairs into the
/// textual "238/92 U" form that `convert_unicode_segment` already handles.
fn merge_stacked_nuclides(lines: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0usize;
    while i < lines.len() {
        let mass_t = lines[i].trim();
        let is_mass = !mass_t.is_empty()
            && mass_t.len() <= 3
            && mass_t.chars().all(|c| c.is_ascii_digit());
        if is_mass {
            if let Some(next) = lines.get(i + 1) {
                if let Some((z, sym, rest)) = atomic_element_on(next) {
                    if let Ok(m) = mass_t.parse::<u32>() {
                        if m > z && z >= 1 {
                            out.push(format!("{}/{} {}{}", m, z, sym, rest));
                            i += 2;
                            continue;
                        }
                    }
                }
            }
        }
        out.push(lines[i].to_string());
        i += 1;
    }
    out
}

// ── Vertical-fraction recovery ──────────────────────────────────────────────

fn is_fraction_bar(line: &str) -> bool {
    let t = line.trim();
    t.chars().count() >= 3
        && t.chars().any(|c| c != ' ')
        && t.chars()
            .all(|c| matches!(c, ' ' | '─' | '━' | '═' | '-' | '−' | '–' | '—' | '_'))
}

fn mathify_side(side: &str) -> String {
    let s = side.trim();
    // Prose ratios become \text{…}; pure math passes through with unicode
    // tokens already converted so `\frac{x + 1}{2}` renders directly.
    if has_prose_word(s) {
        format!("\\text{{{}}}", s)
    } else {
        convert_unicode_segment(s)
    }
}

/// Rebuild vertical fractions that the PDF text layer flattens into three
/// lines (numerator / bar / denominator). Without this the bar is stripped
/// as answer-line furniture and numerator+denominator weld into run-on prose
/// ("electrostatic force gravitational force").
///
/// A bar only becomes a fraction when BOTH neighbours look like expression
/// parts: the numerator must not close a sentence or an assignment (`=` /
/// terminal punctuation ⇒ it was a fill-in blank above an answer line), and
/// both sides must be short non-table lines. Everything else falls through to
/// the ordinary furniture stripping unchanged.
fn recover_vertical_fractions(lines: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0usize;
    while i < lines.len() {
        if !is_fraction_bar(&lines[i]) {
            out.push(lines[i].clone());
            i += 1;
            continue;
        }
        let numerator = out
            .last()
            .map(|l| l.trim().to_string())
            .filter(|l| {
                !l.is_empty()
                    && !l.ends_with('=')
                    && !l.ends_with(':')
                    && !prev_ends_terminal(l)
                    && l.split_whitespace().count() <= 12
                    && !line_is_tableish(l)
                    && !l.starts_with("$$")
                    && !l.contains("![")
                    && !l.contains("[DIAGRAM_PLACEHOLDER]")
            });
        let denom_idx = (i + 1..lines.len()).find(|&j| !lines[j].trim().is_empty());
        let denominator = denom_idx.and_then(|j| {
            let d = lines[j].trim();
            let usable = !d.is_empty()
                && d.split_whitespace().count() <= 12
                && !line_is_tableish(d)
                && !MARK_TAG_RE.is_match(d)
                && !starts_mcq_option(d)
                && !starts_subpart_label(d)
                && !d.contains("![");
            usable.then_some((j, d.to_string()))
        });
        match (numerator, denominator) {
            (Some(num), Some((j, den))) => {
                out.pop();
                out.push(format!("$$\\frac{{{}}}{{{}}}$$", mathify_side(&num), mathify_side(&den)));
                i = j + 1;
            }
            _ => {
                // Not a fraction — keep the bar; answer-line stripping
                // removes it as furniture.
                out.push(lines[i].clone());
                i += 1;
            }
        }
    }
    out
}

// ── MCQ block separation ────────────────────────────────────────────────────

/// Markdown renders consecutive lines as ONE paragraph, which fuses option
/// lists into a single blob ("A … B … C …"). Insert blank separator lines
/// between consecutive option lines so each option becomes its own block.
/// A plain A-D option run whose FIRST option is glued to a short header
/// fragment ("x y A pressure in Pa temperature in ºC", AQA's two-column MCQ
/// table) must be split so the run detector can see option A. The fragment
/// must be short (1..=4 tokens), free of digits/operators, and the following
/// non-empty lines must open with B, C, D in order - otherwise nothing changes.
fn split_glued_header_option_run(lines: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0usize;
    while i < lines.len() {
        let line = &lines[i];
        let mut split_at: Option<usize> = None;
        if !line.trim_start().starts_with('A') {
            if let Some(pos) = find_glued_option_a(line) {
                let mut expected = b'B';
                let mut j = i + 1;
                while j < lines.len() && expected <= b'D' {
                    if lines[j].trim().is_empty() {
                        j += 1;
                        continue;
                    }
                    if line_starts_with_option_letter(&lines[j], expected as char) {
                        expected += 1;
                        j += 1;
                        continue;
                    }
                    break;
                }
                if expected > b'D' {
                    split_at = Some(pos);
                }
            }
        }
        match split_at {
            Some(pos) => {
                out.push(line[..pos].trim_end().to_string());
                out.push(line[pos..].trim_start().to_string());
            }
            None => out.push(line.clone()),
        }
        i += 1;
    }
    out
}

/// Byte offset of a glued " A <content>" option start when the text before it
/// reads as a short header fragment.
fn find_glued_option_a(line: &str) -> Option<usize> {
    let mut search = 0usize;
    while let Some(rel) = line[search..].find(" A ") {
        let pos = search + rel;
        let prefix = line[..pos].trim();
        let rest = line[pos + 3..].trim();
        let prefix_tokens = prefix.split_whitespace().count();
        let header_like = !prefix.is_empty()
            && prefix_tokens <= 4
            && !prefix
                .chars()
                .any(|c| c.is_ascii_digit() || matches!(c, '$' | '=' | '<' | '>' | '^' | '\\'))
            && rest.split_whitespace().count() >= 2;
        if header_like {
            return Some(pos + 1);
        }
        search = pos + 2;
    }
    None
}

/// True when the trimmed line opens with `letter` plus a separator.
fn line_starts_with_option_letter(line: &str, letter: char) -> bool {
    let t = line.trim_start();
    let mut chars = t.chars();
    if chars.next() != Some(letter) {
        return false;
    }
    matches!(chars.next(), Some(' ') | Some(')') | Some('.') | Some('\t'))
}

/// A token that looks like a table VALUE (number, value+unit, or math span).
fn is_value_token(tok: &str) -> bool {
    let t = tok.trim();
    if t.is_empty() {
        return false;
    }
    let has_digit = t.chars().any(|c| c.is_ascii_digit());
    let value_like = t.chars().all(|c| {
        c.is_ascii_digit()
            || matches!(
                c,
                '.' | ',' | '-' | '+' | '/' | '%' | '^' | '×' | '−' | '\\' | '{' | '}' | '_' | '(' | ')' | '*' | '$'
            )
    });
    has_digit && value_like
}

/// Split a table header into `columns` cells using its "/ unit" separators:
/// each cell boundary sits one token before a slash token.
fn split_table_header(header: &str, columns: usize) -> Option<Vec<String>> {
    let tokens: Vec<&str> = header.split_whitespace().collect();
    let slashes: Vec<usize> = tokens
        .iter()
        .enumerate()
        .filter(|(_, tok)| **tok == "/" || tok.ends_with('/'))
        .map(|(idx, _)| idx)
        .collect();
    if slashes.len() != columns - 1 {
        return None;
    }
    let mut boundaries: Vec<usize> = slashes.iter().map(|idx| idx.saturating_sub(1)).collect();
    boundaries.sort_unstable();
    boundaries.dedup();
    if boundaries.len() != columns - 1 || boundaries[0] == 0 {
        return None;
    }
    let mut cells: Vec<String> = Vec::with_capacity(columns);
    let mut start = 0usize;
    for &b in &boundaries {
        if b <= start {
            return None;
        }
        cells.push(tokens[start..b].join(" "));
        start = b;
    }
    cells.push(tokens[start..].join(" "));
    if cells.len() != columns || cells.iter().any(|c| c.is_empty()) {
        return None;
    }
    Some(cells)
}

/// Rebuild `Table N` blocks from the flattened text layer into Markdown pipe
/// tables. The column count comes from the ROWS (their run of trailing value
/// tokens) and the header split comes from the header's own "/ unit"
/// separators. Nothing is invented: when either signal is missing the block is
/// left exactly as it was, so the span stays a flagged recovery.
fn recover_markdown_data_tables(lines: Vec<String>) -> Vec<String> {
    static CAPTION_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?i)^table\s+\d+$").unwrap());
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0usize;
    while i < lines.len() {
        let caption = lines[i].trim().to_string();
        if !CAPTION_RE.is_match(&caption) {
            out.push(lines[i].clone());
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < lines.len() && lines[j].trim().is_empty() {
            j += 1;
        }
        let Some(header) = lines.get(j).map(|l| l.trim().to_string()) else {
            out.push(lines[i].clone());
            i += 1;
            continue;
        };
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut k = j + 1;
        while k < lines.len() {
            let t = lines[k].trim();
            if t.is_empty() {
                k += 1;
                continue;
            }
            let tokens: Vec<String> = t.split_whitespace().map(str::to_string).collect();
            let trailing = tokens.iter().rev().take_while(|tok| is_value_token(tok)).count();
            if tokens.len() >= 2 && trailing >= 1 {
                rows.push(tokens);
                k += 1;
                continue;
            }
            break;
        }
        if rows.len() < 2 {
            out.push(lines[i].clone());
            i += 1;
            continue;
        }
        let columns = 1 + rows
            .iter()
            .map(|tokens| tokens.iter().rev().take_while(|tok| is_value_token(tok)).count())
            .max()
            .unwrap_or(0);
        let Some(header_cells) = split_table_header(&header, columns) else {
            out.push(lines[i].clone());
            i += 1;
            continue;
        };
        out.push(caption.clone());
        out.push(format!("| {} |", header_cells.join(" | ")));
        out.push(format!("|{}", " --- |".repeat(columns)));
        for tokens in &rows {
            let first_len = tokens.len().saturating_sub(columns - 1);
            let mut cells: Vec<String> = Vec::with_capacity(columns);
            cells.push(tokens[..first_len].join(" "));
            cells.extend(tokens[first_len..].iter().cloned());
            while cells.len() < columns {
                cells.push(String::new());
            }
            out.push(format!("| {} |", cells.join(" | ")));
        }
        i = k;
    }
    out
}

fn separate_mcq_options(blocks: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(blocks.len());
    let mut in_code = false;
    for block in blocks {
        for line in block.split('\n') {
            if line.trim_start().starts_with("```") {
                in_code = !in_code;
            }
            if !in_code
                && starts_mcq_option(line)
                && out.last().map(|l| starts_mcq_option(l)).unwrap_or(false)
            {
                out.push(String::new());
            }
            out.push(line.to_string());
        }
    }
    out
}

// ── Block-media isolation ───────────────────────────────────────────────────

static MEDIA_LINE_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(!\[[^\]]*\]\([^)]*\)|\[DIAGRAM_PLACEHOLDER\])").unwrap()
});

/// Diagram images are rendered as BLOCK-level containers by the frontend.
/// When one sits inline inside a paragraph the DOM parser closes the `<p>`
/// early and breaks the page layout, so force every image link onto its own
/// paragraph boundaries.
fn isolate_block_media(content: &str) -> String {
    let mut out = String::with_capacity(content.len() + 32);
    for line in content.split('\n') {
        if MEDIA_LINE_RE.is_match(line) {
            let mut last = 0usize;
            let mut wrote_media = false;
            for m in MEDIA_LINE_RE.find_iter(line) {
                let before = line[last..m.start()].trim_end();
                if !before.is_empty() {
                    if !out.is_empty() && !out.ends_with("\n\n") {
                        out.push_str("\n\n");
                    }
                    out.push_str(before);
                    out.push_str("\n\n");
                } else if !out.is_empty() && !out.ends_with("\n\n") {
                    out.push_str("\n\n");
                }
                out.push_str(m.as_str());
                out.push_str("\n\n");
                last = m.end();
                wrote_media = true;
            }
            if !wrote_media {
                out.push_str(line);
            } else {
                let tail = line[last..].trim();
                if !tail.is_empty() {
                    out.push_str(tail);
                    out.push_str("\n\n");
                }
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    regex::Regex::new(r"\n{3,}")
        .unwrap()
        .replace_all(&out, "\n\n")
        .into_owned()
}

#[derive(Debug, Clone)]
pub struct Normalized {
    pub content: String,
    pub marks_total: u32,
    /// Answer-line removals counted during stripping (gate evidence).
    #[allow(dead_code)]
    pub answer_lines_seen: usize,
}

static PURE_BLANK_LINE_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    // Includes fraction-bar glyphs (─ ━ ═ ·) so any bar NOT consumed by
    // recover_vertical_fractions is still stripped as answer-line furniture.
    regex::Regex::new(r"^\s*(?:[_.\u2026\uFF0E‒–—\-─━═·]\s*){3,}$").unwrap()
});

static ANSWER_EQ_LINE_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)^\s*[a-z0-9 ,''’()\[\]/°%\-\+]{0,52}[=:]\s*(?:[_.\u2026]\s*){2,}[a-zA-Z°%^0-9/]{0,10}\s*$",
    )
    .unwrap()
});


static LEFTOVER_BLANK_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(?:[_]{3,}|[.\u2026]{5,})").unwrap());

static MARK_TAG_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(?i)[\(\[]\s*(\d{1,2})\s*marks?\s*[\)\]]").unwrap());

/// Bare line-end allocations — Edexcel/CIE style "(3)", "[2]" at the end of
/// a line. The marks-checksum gate validates the sum against the printed
/// footer whenever one exists, so a stray bracket cannot slip through.
static BARE_MARK_TAG_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"[\(\[]\s*(\d{1,2})\s*[\)\]][.:]?[ \t]*$").unwrap());

static TABLE_GAP_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\S\s{2,}\S").unwrap());

static FIGURE_REF_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)\bfig(?:ure)?\.?\s*\d+|\bdiagrams?\b|\bgraphs?\b|\bcircuit\b|\bsketch(?:ed)?\b|\bshown below\b|\bnot drawn to scale\b",
    )
    .unwrap()
});

fn is_answer_line(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    if t.eq_ignore_ascii_case("left")
        || t.eq_ignore_ascii_case("right")
        || t.eq_ignore_ascii_case("left right")
    {
        return true;
    }
    if PURE_BLANK_LINE_RE.is_match(t) || ANSWER_EQ_LINE_RE.is_match(t) {
        return true;
    }
    if let Some(i) = t.find(['=', ':']) {
        let lhs = t[..i].trim();
        let rhs = t[i + 1..].trim();
        let rhs_val = rhs.trim_start();
        if !rhs_val.is_empty() && rhs_val.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return false;
        }
        let rhs_clean = rhs_val.trim_end_matches(|c: char| c.is_ascii_digit() || c.is_whitespace());
        let rhs_is_unit = !rhs_clean.is_empty()
            && rhs_clean.chars().all(|c| {
                c.is_alphabetic()
                    || matches!(c, ' ' | '/' | '^' | '-' | '−' | '–' | '°' | '%' | 'Ω' | 'μ' | '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8' | '9' | '0' | '(' | ')' | '[' | ']')
            });
        let has_descriptive_word = lhs
            .split_whitespace()
            .any(|w| w.chars().filter(|c| c.is_alphabetic()).count() >= 4);
        let is_var_lhs = matches!(
            lhs.to_lowercase().as_str(),
            "crms" | "λ" | "\\lambda" | "emf" | "v" | "i0" | "vr" | "vc" | "p" | "t" | "r"
        );
        if (has_descriptive_word || is_var_lhs) && (rhs_clean.is_empty() || rhs_is_unit) {
            return true;
        }
    }
    false
}

fn strip_answer_lines(lines: Vec<String>) -> (Vec<String>, usize) {
    let before = lines.len();
    let mut is_ans = vec![false; lines.len()];
    for (idx, line) in lines.iter().enumerate() {
        if is_answer_line(line) {
            is_ans[idx] = true;
            if idx > 0 {
                let prev = lines[idx - 1].trim();
                if prev.len() <= 4
                    && !prev.ends_with('.')
                    && !prev.ends_with('?')
                    && !prev.ends_with(':')
                    && !MARK_TAG_RE.is_match(prev)
                    && !BARE_MARK_TAG_RE.is_match(prev)
                {
                    is_ans[idx - 1] = true;
                }
            }
            let mut next_i = idx + 1;
            while next_i < lines.len() && lines[next_i].trim().is_empty() {
                next_i += 1;
            }
            if next_i < lines.len() {
                let next = lines[next_i].trim();
                if next.len() <= 4
                    && !next.ends_with('.')
                    && !next.ends_with('?')
                    && !next.ends_with(':')
                    && !starts_subpart_label(next)
                    && !MARK_TAG_RE.is_match(next)
                    && !BARE_MARK_TAG_RE.is_match(next)
                {
                    is_ans[next_i] = true;
                }
            }
        }
    }
    let kept: Vec<String> = lines
        .into_iter()
        .enumerate()
        .filter(|(idx, _)| !is_ans[*idx])
        .map(|(_, l)| l)
        .collect();
    let removed = before - kept.len();
    (kept, removed)
}

fn line_is_tableish(line: &str) -> bool {
    TABLE_GAP_RE.find_iter(line).count() >= 2
}

static CONTINUATION_WORDS: &[&str] = &[
    "of", "the", "and", "to", "in", "with", "or", "is", "are", "was", "were", "be", "been", "a",
    "an", "as", "at", "by", "for", "from", "on", "that", "this", "which", "using", "use", "where",
    "when", "while", "then", "than", "into", "onto", "also", "each", "its", "it",
    // Sub-clause continuators: a sentence split across a hard break usually
    // resumes with one of these, including after a closing parenthesis.
    "because", "since", "so", "but", "nor", "if", "unless", "until", "whereas", "whose", "both",
    "either", "neither", "yet", "hence", "thus",
];

fn prev_ends_terminal(prev: &str) -> bool {
    let t = prev.trim_end_matches(|c: char| c.is_whitespace() || c == '*');
    t.ends_with('.')
        || t.ends_with('?')
        || t.ends_with('!')
        || t.ends_with(':')
        || t.ends_with(';')
        || t.ends_with('|')
        || t.ends_with('$')
        || t.ends_with('`')
}

fn starts_subpart_label(line: &str) -> bool {
    let t = line.trim_start_matches('*').trim_start();
    t.starts_with('(') || DECIMAL_LABEL_RE.is_match(t)
}

fn starts_mcq_option(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with("- [MCQ:") { return true; }
    let b = t.as_bytes();
    if b.is_empty() || !(b'A'..=b'E').contains(&b[0]) {
        return false;
    }
    match b.get(1) {
        Some(b'.') | Some(b')') => true,
        // "A 780 W" / "A r < 10" style — but NOT the article "A similar
        // approach…". The discriminator is the SECOND token: option values
        // are a bare number, a single-letter variable, or start with an
        // operator — never a multi-letter word.
        Some(b' ') => match t[2..].split_whitespace().next() {
            None => true,
            Some(tok) => {
                let tb = tok.as_bytes();
                tok.len() == 1 && (tb[0].is_ascii_digit() || tb[0].is_ascii_alphabetic())
                    || tok.len() > 1
                        && (tok.chars().next().is_some_and(|c| c.is_ascii_digit() || c == '<' || c == '>' || c == '=' || c == '-' || c == '+' || c == '$')
                            || tok.starts_with("\\"))
            }
        },
        _ => false,
    }
}

fn is_display_math_candidate(line: &str) -> bool {
    let t = line.trim();
    if t.starts_with("$$") || t.starts_with('\\') {
        return true;
    }
    t.contains('=') && t.split_whitespace().count() <= 6 && !t.ends_with('.')
}

/// Conservative hard-wrap repair: merge a line into the previous one ONLY when
/// the previous lacks terminal punctuation AND the next line starts lowercase
/// or with a continuation word. Never joins across blank lines, sub-part
/// labels, MCQ options, display-math candidates, or table-ish lines.
fn conservative_rejoin(lines: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for cur in lines {
        let joined = match out.last() {
            Some(prev) if !prev.trim().is_empty() && !cur.trim().is_empty() => {
                let t = cur.trim_start();
                let first_word = t.split_whitespace().next().unwrap_or("");
                let cont = first_word.chars().next().map(|c| c.is_ascii_lowercase()).unwrap_or(false)
                    || CONTINUATION_WORDS.contains(&first_word.to_ascii_lowercase().as_str())
                    || first_word.starts_with('=');
                // Media links are block-level in the rendered card; welding
                // one into a prose line nests the diagram container inside
                // the paragraph element and breaks the DOM.
                !prev.contains("![")
                    && !prev.contains("[DIAGRAM_PLACEHOLDER]")
                    && !t.contains("![")
                    && !t.contains("[DIAGRAM_PLACEHOLDER]")
                    && !MARK_TAG_RE.is_match(prev)
                    && !BARE_MARK_TAG_RE.is_match(prev)
                    && !MARK_TAG_RE.is_match(t)
                    && !BARE_MARK_TAG_RE.is_match(t)
                    && !prev_ends_terminal(prev)
                    && cont
                    && !starts_subpart_label(t)
                    && !starts_mcq_option(t)
                    && !is_display_math_candidate(t)
                    && !line_is_tableish(t)
                    && !line_is_tableish(prev)
                    && !is_display_math_candidate(prev)
            }
            _ => false,
        };
        if joined {
            let prev = out.pop().unwrap();
            out.push(format!("{} {}", prev.trim_end(), cur.trim()));
        } else {
            out.push(cur.trim().to_string());
        }
    }
    out
}

fn ensure_subpart_paragraphs(lines: &[String]) -> Vec<String> {
    subpart_paragraphs(lines, true)
}

/// As [`ensure_subpart_paragraphs`]; `decimal_labels` = AQA "01.2" labels
/// may still be in the text (a layout body has converted them already, so
/// a line opening on a decimal, "0.71 billion years", is not a part).
fn subpart_paragraphs(lines: &[String], decimal_labels: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut in_code = false;
    for line in lines {
        let t = line.trim();
        // Fenced code is copied verbatim (indentation, blank lines).
        if t.starts_with("```") {
            in_code = !in_code;
        } else if in_code {
            out.push(line.trim_end().to_string());
            continue;
        }
        if t.is_empty() {
            if out.last().map(|l| !l.is_empty()).unwrap_or(false) {
                out.push(String::new());
            }
            continue;
        }
        let label = if decimal_labels { starts_subpart_label(t) } else { t.trim_start_matches('*').trim_start().starts_with('(') };
        let needs_break = label && out.last().map(|l| !l.is_empty()).unwrap_or(false);
        if needs_break {
            out.push(String::new());
        }
        out.push(t.to_string());
    }
    while out.last().map(|l| l.is_empty()).unwrap_or(false) {
        out.pop();
    }
    out
}

fn format_marks(blocks: &[String]) -> (Vec<String>, u32) {
    let mut out = Vec::with_capacity(blocks.len());
    let mut total = 0u32;
    for block in blocks {
        if !MARK_TAG_RE.is_match(block) && !block.lines().any(|l| BARE_MARK_TAG_RE.is_match(l.trim()))
        {
            out.push(block.clone());
            continue;
        }
        let mut block_total = 0u32;
        let cleaned_lines: Vec<String> = block
            .split('\n')
            .map(|line| {
                let tags: Vec<u32> = MARK_TAG_RE
                    .captures_iter(line)
                    .filter_map(|c| c[1].parse().ok())
                    .collect();
                if !tags.is_empty() {
                    block_total += tags.iter().sum::<u32>();
                    return MARK_TAG_RE.replace_all(line, "").into_owned();
                }
                if let Some(caps) = BARE_MARK_TAG_RE.captures(line.trim()) {
                    let trimmed = line.trim();
                    if caps.get(0).unwrap().end() == trimmed.len() {
                        if let Ok(v) = caps[1].parse::<u32>() {
                            block_total += v;
                            let indent = line.len() - line.trim_start().len();
                            let stripped = BARE_MARK_TAG_RE.replace(trimmed, "");
                            return format!("{}{}", &line[..indent], stripped.trim_end());
                        }
                    }
                }
                line.to_string()
            })
            .collect();
        total += block_total;
        if block_total > 0 {
            let unit = if block_total == 1 { "mark" } else { "marks" };
            let joined = cleaned_lines.join("\n");
            out.push(format!("{}\n\n**[{} {}]**", joined.trim_end(), block_total, unit));
        } else {
            out.push(block.clone());
        }
    }
    (out, total)
}

// ── Unicode → LaTeX (token level) ───────────────────────────────────────────

fn super_char(c: char) -> Option<char> {
    match c {
        '⁰' => Some('0'),
        '¹' => Some('1'),
        '²' => Some('2'),
        '³' => Some('3'),
        '⁴' => Some('4'),
        '⁵' => Some('5'),
        '⁶' => Some('6'),
        '⁷' => Some('7'),
        '⁸' => Some('8'),
        '⁹' => Some('9'),
        '⁺' => Some('+'),
        '⁻' => Some('-'),
        'ⁿ' => Some('n'),
        _ => None,
    }
}

fn sub_char(c: char) -> Option<char> {
    match c {
        '₀' => Some('0'),
        '₁' => Some('1'),
        '₂' => Some('2'),
        '₃' => Some('3'),
        '₄' => Some('4'),
        '₅' => Some('5'),
        '₆' => Some('6'),
        '₇' => Some('7'),
        '₈' => Some('8'),
        '₉' => Some('9'),
        '₊' => Some('+'),
        '₋' => Some('-'),
        'ₙ' => Some('n'),
        'ₓ' => Some('x'),
        _ => None,
    }
}

fn symbol_latex(c: char) -> Option<&'static str> {
    Some(match c {
        '×' => "\\times",
        '÷' => "\\div",
        '±' => "\\pm",
        '≤' => "\\le",
        '≥' => "\\ge",
        '≠' => "\\neq",
        '≈' => "\\approx",
        '≡' => "\\equiv",
        '≅' => "\\cong",
        '∝' => "\\propto",
        '√' => "\\sqrt",
        '∞' => "\\infty",
        '∴' => "\\therefore",
        '°' => "^{\\circ}",
        '·' => "\\cdot",
        '%' => "\\%",
        '→' => "\\rightarrow",
        '←' => "\\leftarrow",
        '⇒' => "\\Rightarrow",
        'Δ' => "\\Delta",
        'Λ' => "\\Lambda",
        'Σ' => "\\Sigma",
        'Φ' => "\\Phi",
        'Ψ' => "\\Psi",
        'Γ' => "\\Gamma",
        'Ω' => "\\Omega",
        'α' => "\\alpha",
        'β' => "\\beta",
        'γ' => "\\gamma",
        'δ' => "\\delta",
        'ε' => "\\epsilon",
        'η' => "\\eta",
        'θ' => "\\theta",
        'κ' => "\\kappa",
        'λ' => "\\lambda",
        'μ' => "\\mu",
        'π' => "\\pi",
        'ρ' => "\\rho",
        'σ' => "\\sigma",
        'τ' => "\\tau",
        'φ' => "\\phi",
        'χ' => "\\chi",
        'ψ' => "\\psi",
        'ω' => "\\omega",
        'ζ' => "\\zeta",
        'ξ' => "\\xi",
        'ℓ' => "\\ell",
        'Å' => "\\text{\\AA}",
        _ => return None,
    })
}

static TEXTUAL_NUCLIDE_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\b(\d{1,3})\s*/\s*(\d{1,3})\s+([A-Z][a-z]?)\b").unwrap()
});

static BRACED_NUCLIDE_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\^\{(-?\d{1,3})\}_\{(-?\d{1,3})\}\s*([A-Z][a-z]?)\b").unwrap()
});

static PRESCRIPT_ONLY_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\^\{(\d{1,3})\}\s*([A-Z][a-z]?)\b").unwrap());

/// The text layer flattens superscript exponents: "1.0 × 1022" and
/// "6.3 × 10-14" mean $1.0 \times 10^{22}$ / $6.3 \times 10^{-14}$. Recover
/// the exponent whenever a mantissa×10^digits shape survives extraction.
/// The mantissa×10 prefix is REQUIRED — a bare "10-14" may be a range
/// ("ages 10–14") and must stay untouched.
static SCIENTIFIC_FLATTENED_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(\d(?:\.\d+)?)\s*(?:\\times|×)\s*10\s*(-?)\s*(\d{1,2})").unwrap()
});

fn convert_unicode_segment(seg: &str) -> String {
    // Superscript / subscript runs → ^{…} / _{…}
    let mut with_runs = String::with_capacity(seg.len() + 16);
    let mut chars = seg.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(sc) = super_char(c) {
            let mut run = String::new();
            run.push(sc);
            while let Some(&nc) = chars.peek() {
                match super_char(nc) {
                    Some(sc) => {
                        run.push(sc);
                        chars.next();
                    }
                    None => break,
                }
            }
            with_runs.push_str(&format!("^{{{}}}", run));
            continue;
        }
        if let Some(sc) = sub_char(c) {
            let mut run = String::new();
            run.push(sc);
            while let Some(&nc) = chars.peek() {
                match sub_char(nc) {
                    Some(sc) => {
                        run.push(sc);
                        chars.next();
                    }
                    None => break,
                }
            }
            with_runs.push_str(&format!("_{{{}}}", run));
            continue;
        }
        match symbol_latex(c) {
            Some(lat) => with_runs.push_str(lat),
            None => with_runs.push(c),
        }
    }

    // Scientific notation with flattened exponents ("1.0 × 1022" → 10^22,
    // "6.3 × 10-14" → 10^{-14}). Guards: a digit immediately after the
    // match means we clipped a longer number ("1022"), and a following `=`
    // means it was arithmetic ("4 × 10 - 2 = 38"), not an exponent.
    let scientific = SCIENTIFIC_FLATTENED_RE
        .replace_all(&with_runs, |caps: &regex::Captures| {
            let after = caps.get(0).unwrap().end();
            let next_meaningful = with_runs[after..]
                .chars()
                .find(|c| !c.is_whitespace());
            if next_meaningful.map(|c| c.is_ascii_digit() || c == '=').unwrap_or(false) {
                return caps[0].to_string();
            }
            if caps[2].eq("-") {
                format!("{}\\times10^{{-{}}}", &caps[1], &caps[3])
            } else {
                format!("{}\\times10^{{{}}}", &caps[1], &caps[3])
            }
        })
        .into_owned();

    // Nuclear notation: braced prescript pairs, bare prescripts, textual m/z.
    let braced = BRACED_NUCLIDE_RE
        .replace_all(&scientific, "^{$1}_{$2}\\text{$3}")
        .into_owned();
    let textual = TEXTUAL_NUCLIDE_RE
        .replace_all(&braced, |caps: &regex::Captures| {
            let mass: u32 = caps[1].parse().unwrap_or(0);
            let atomic: u32 = caps[2].parse().unwrap_or(u32::MAX);
            if mass > atomic && atomic >= 1 {
                format!("^{{{}}}_{{{}}}\\text{{{}}}", &caps[1], &caps[2], &caps[3])
            } else {
                caps[0].to_string()
            }
        })
        .into_owned();
    PRESCRIPT_ONLY_RE
        .replace_all(&textual, "^{$1}\\text{$2}")
        .into_owned()
}

fn convert_unicode(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            line.split('`')
                .enumerate()
                .map(|(i, seg)| if i % 2 == 1 { seg.to_string() } else { convert_unicode_segment(seg) })
                .collect::<Vec<_>>()
                .join("`")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Math-cluster wrapping ───────────────────────────────────────────────────

static LATEX_COMMAND_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\\[A-Za-z]+").unwrap());

static TEXT_GROUP_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\\text\{[^}]*\}").unwrap());

static PROSE_WORD_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"[A-Za-z]{2,}").unwrap());

fn has_prose_word(line: &str) -> bool {
    let stripped = LATEX_COMMAND_RE.replace_all(line, "");
    let stripped = TEXT_GROUP_RE.replace_all(&stripped, "");
    PROSE_WORD_RE.is_match(&stripped)
}

/// Wrap contiguous math-token clusters (identifier + operators + digits +
/// converted symbols) in `$…$`. Never wraps prose, standalone units, marks
/// tags, existing `$…$`/`$$` regions, or backtick (code) spans. A cluster
/// must carry clear math evidence: an operator, a `^`/`_`, a LaTeX command,
/// or a digit adjacent to a single-letter variable.
struct ClusterState {
    out: String,
    cluster: String,
    has_op: bool,
    has_digit: bool,
    has_single_letter: bool,
    brace_depth: usize,
}

impl ClusterState {
    fn flush(&mut self) {
        if !self.cluster.is_empty() && (self.has_op || (self.has_digit && self.has_single_letter)) {
            self.out.push('$');
            self.out.push_str(self.cluster.trim());
            self.out.push('$');
        } else if !self.cluster.is_empty() {
            self.out.push_str(&self.cluster);
        }
        self.cluster.clear();
        self.has_op = false;
        self.has_digit = false;
        self.has_single_letter = false;
    }
}

/// SI unit tokens eligible for binding into the preceding math block.
/// Case-sensitive on purpose: "m" the metre is bound, "a"/"the" never match,
/// and unknown words fall through to prose handling unchanged.
const UNIT_TOKENS: &[&str] = &[
    "m", "g", "s", "A", "K", "N", "J", "W", "V", "F", "T", "H", "Hz", "Pa", "mol", "kg", "km",
    "cm", "dm", "mm", "nm", "pm", "ms", "ns", "kJ", "kW", "MJ", "kN", "mA", "eV", "MeV", "keV",
    "GeV", "h",
];

/// `(token, chars_consumed)` when an exact unit token starts at `idx` AND
/// ends at a boundary (whitespace, punctuation, end of line).
fn unit_token_at(chars: &[char], idx: usize) -> Option<(&str, usize)> {
    let mut len = 0usize;
    while idx + len < chars.len() && chars[idx + len].is_alphanumeric() {
        len += 1;
    }
    if len == 0 || len > 4 {
        return None;
    }
    let token: String = chars[idx..idx + len].iter().collect();
    let after = chars.get(idx + len);
    let boundary = after
        .map(|c| !c.is_alphanumeric())
        .unwrap_or(true);
    if !boundary {
        return None;
    }
    UNIT_TOKENS
        .iter()
        .find(|u| **u == token)
        .map(|u| (*u, len))
}

/// Detect flattened-math / nuclide debris left by column-flattened text
/// extraction. Returns a gate reason when a line is a formula/nuclide fragment
/// rather than a clean question body, so the card becomes an honest local
/// recovery instead of a strict-clean success.
fn flattened_math_debris(content: &str) -> Option<&'static str> {
    static RE_SPAN: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\$[^$\n]{1,12}\$").unwrap());
    let elements: Vec<&str> = crate::doc_map::NUCLIDE_ELEMENTS.split('|').collect();
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty()
            || t.starts_with('|')
            || t.starts_with("![")
            || t.starts_with("- [MCQ:")
            || t.starts_with("**")
        {
            continue;
        }
        // (a) Flattened nuclide/reaction row: several bare small integers plus
        // an element symbol, with no sentence punctuation.
        let tokens: Vec<&str> = t.split_whitespace().collect();
        if tokens.len() >= 5 {
            let numerics = tokens
                .iter()
                .filter(|tok| tok.len() <= 3 && tok.chars().all(|c| c.is_ascii_digit()))
                .count();
            let elems = tokens
                .iter()
                .filter(|tok| elements.contains(tok))
                .count();
            // A flattened row is mostly numbers and symbols; a prose sentence
            // that happens to quote "1 N", "1 m" and "1 C" is not.
            if numerics >= 3 && elems >= 1 && 2 * (numerics + elems) >= tokens.len() {
                return Some("flattened_nuclide_row");
            }
        }
        // NOTE: a "math span starting with an operator" rule was tried here but
        // is too broad — legitimate prose carries `$=$` / `$= 10$`. Only the
        // nuclide-row and symbol-debris signatures below are specific enough.
        // (c) A short variable span trailed by uppercase symbol debris
        // outside math (e.g. "$k p 2$ eBR E").
        // (c) A SHORT line that is a short variable span plus uppercase symbol
        // debris (e.g. "$k p 2$ eBR E"). Long prose lines are never flagged.
        if t.len() <= 25 {
            if let Some(cap) = RE_SPAN.captures(t) {
                let inner = cap[0].trim_matches('$').trim();
                let without = RE_SPAN.replace_all(t, " ");
                let has_word = without
                    .split_whitespace()
                    .any(|tok| tok.chars().filter(|c| c.is_ascii_lowercase()).count() >= 2);
                let has_symbol_debris = without.split_whitespace().any(|tok| {
                    let upper = tok.chars().filter(|c| c.is_ascii_uppercase()).count();
                    upper >= 2 && tok.len() <= 5 && !elements.contains(&tok)
                });
                if inner.len() <= 6 && !has_word && has_symbol_debris {
                    return Some("flattened_symbol_sequence");
                }
            }
        }
    }
    None
}

/// Wrap each cell of a Markdown pipe row independently, and only when the cell
/// carries explicit math evidence (`\`, `^`, `_`). The pipes always stay OUTSIDE
/// any `$...$` span, and a plain "Label / unit" cell is left as text.
fn wrap_table_math_cells(line: &str) -> String {
    if !line.contains('|') {
        return line.to_string();
    }
    line.split('|')
        .map(|cell| {
            let t = cell.trim();
            if t.is_empty() {
                String::new()
            } else if t.contains('\\') || t.contains('^') || t.contains('_') {
                wrap_math_line(t)
            } else {
                t.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn wrap_math_line(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut st = ClusterState {
        out: String::with_capacity(line.len() + 8),
        cluster: String::new(),
        has_op: false,
        has_digit: false,
        has_single_letter: false,
        brace_depth: 0,
    };
    let mut i = 0usize;

    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            st.flush();
            st.out.push(c);
            i += 1;
            while i < chars.len() && chars[i] != '`' {
                st.out.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                st.out.push('`');
                i += 1;
            }
            continue;
        }
        if c == '$' {
            st.flush();
            if i + 1 < chars.len() && chars[i + 1] == '$' {
                st.out.push_str("$$");
                i += 2;
            } else {
                st.out.push('$');
                i += 1;
                while i < chars.len() && chars[i] != '$' {
                    st.out.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    st.out.push('$');
                    i += 1;
                }
            }
            continue;
        }
        if c.is_whitespace() {
            if st.cluster.is_empty() {
                st.out.push(c);
                i += 1;
                continue;
            }
            let mut j = i;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            // Unit binding: a short unit token right after math evidence
            // joins the SAME $…$ block ("10^{-14} mol"), so the value and
            // its unit never split into separate KaTeX spans.
            if st.has_op || st.has_digit {
                if let Some((token, len)) = unit_token_at(&chars, j) {
                    st.cluster.push(' ');
                    st.cluster.push_str("\\text{");
                    st.cluster.push_str(token);
                    st.cluster.push('}');
                    i = j + len;
                    continue;
                }
            }
            let next_starts_prose = j < chars.len() && {
                let mut k = j;
                while k < chars.len() && chars[k].is_alphabetic() {
                    k += 1;
                }
                k - j >= 2
            };
            let next_sentence_end = j >= chars.len();
            if next_starts_prose || next_sentence_end {
                st.flush();
                st.out.push_str(&line.chars().skip(i).take(j - i).collect::<String>());
            } else {
                st.cluster.push_str(&line.chars().skip(i).take(j - i).collect::<String>());
            }
            i = j;
            continue;
        }
        if c.is_alphabetic() {
            let mut j = i;
            while j < chars.len() && chars[j].is_alphabetic() {
                j += 1;
            }
            if st.brace_depth > 0 {
                // Inside a LaTeX argument (`\text{Ra}`): never prose.
                st.cluster.extend(&chars[i..j]);
            } else if j - i >= 2 {
                st.flush();
                st.out.extend(&chars[i..j]);
            } else {
                st.cluster.push(chars[i]);
                st.has_single_letter = true;
            }
            i = j;
            continue;
        }
        if c.is_ascii_digit() {
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_digit() || (chars[j] == '.' && j + 1 < chars.len() && chars[j + 1].is_ascii_digit())) {
                j += 1;
            }
            st.cluster.extend(&chars[i..j]);
            st.has_digit = true;
            i = j;
            continue;
        }
        if c == '\\' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_alphabetic() {
                j += 1;
            }
            if j > i + 1 {
                st.cluster.extend(&chars[i..j]);
            } else {
                st.cluster.push(c);
            }
            st.has_op = true;
            i = j.max(i + 1);
            continue;
        }
        match c {
            '=' | '<' | '>' | '+' | '/' | '^' => {
                st.cluster.push(c);
                st.has_op = true;
                i += 1;
            }
            '-' => {
                let hyphenated_word = !st.has_op && st.cluster.trim().len() == 1
                    && st.cluster.trim().chars().all(|ch| ch.is_ascii_alphabetic())
                    && chars.get(i+1).is_some_and(|ch| ch.is_ascii_alphabetic())
                    && chars.get(i+2).is_some_and(|ch| ch.is_ascii_alphabetic());
                if hyphenated_word {
                    st.flush();
                    st.out.push('-');
                    i += 1;
                    continue;
                }
                let prev_is_letter = st.out.chars().last().is_some_and(|ch| ch.is_alphabetic());
                if st.cluster.is_empty() && prev_is_letter {
                    // Grammatical hyphen glued to a prose word — "half-life",
                    // "lead-207". A real minus never attaches directly to a
                    // multi-letter word with no space, and treating it as an
                    // operator fractures the word into stray KaTeX spans.
                    st.out.push(c);
                } else {
                    st.cluster.push(c);
                    st.has_op = true;
                }
                i += 1;
            }
            '_' => {
                let prev_is_alnum = st
                    .out
                    .chars()
                    .last()
                    .is_some_and(|ch| ch.is_alphanumeric());
                if st.cluster.is_empty() && prev_is_alnum {
                    // Underscore GLUE inside an identifier ("DIAGRAM_PLACEHOLDER",
                    // SNAKE_CASE): not a LaTeX subscript. A genuine plain-text
                    // subscript ("x_1") keeps its cluster alive through the
                    // underscore, so it still takes the operator path.
                    st.out.push(c);
                } else {
                    st.cluster.push(c);
                    st.has_op = true;
                }
                i += 1;
            }
            '(' | ')' | '[' | ']' | '{' | '}' | '|' | ':' => {
                st.cluster.push(c);
                if c == '{' {
                    st.brace_depth += 1;
                } else if c == '}' {
                    st.brace_depth = st.brace_depth.saturating_sub(1);
                }
                i += 1;
            }
            ',' | ';' | '.' => {
                let next = chars.get(i + 1);
                let boundary = next.is_none()
                    || next.is_some_and(|n| n.is_whitespace())
                    || next.is_some_and(|n| n.is_uppercase());
                if boundary {
                    st.flush();
                    st.out.push(c);
                } else {
                    st.cluster.push(c);
                }
                i += 1;
            }
            _ => {
                st.flush();
                st.out.push(c);
                i += 1;
            }
        }
    }
    st.flush();
    st.out
}

fn wrap_math(content: &str) -> String {
    content
        .split('\n')
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("$$") {
                return line.to_string();
            }
            // Markdown pipe rows/headers: math is confined to CELLS. Never run
            // the whole-line clusterer here, or a `$...$` span swallows the
            // pipes and corrupts the table into `$| a | b |$`.
            if trimmed.starts_with('|') {
                return wrap_table_math_cells(line);
            }
            if trimmed.starts_with('|') && trimmed.chars().all(|c| c == '-' || c == ':' || c == '|' || c.is_whitespace()) {
                return line.to_string();
            }
            if trimmed.starts_with("- [MCQ:") {
                if let Some(end) = trimmed.find("] ") {
                    return format!("{}{}", &trimmed[..end+2], wrap_math_line(&trimmed[end+2..]));
                }
            }
            if starts_mcq_option(trimmed) && trimmed.len() > 2 {
                return format!("{}{}", &trimmed[..2], wrap_math_line(&trimmed[2..]));
            }
            let indent_len = line.len() - trimmed.len();
            let (indent, rest) = line.split_at(indent_len);
            static PART: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^\([a-h]\)[ \t]+").unwrap());
            if let Some(label) = PART.find(rest) {
                return format!("{}{}{}", indent, label.as_str(), wrap_math_line(&rest[label.end()..]));
            }
            format!("{}{}", indent, wrap_math_line(rest))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Standalone equation lines (nuclide decay chains, derivations) become
/// `$$…$$` display blocks — never prose lines.
fn promote_display_equations(content: &str) -> String {
    content
        .split('\n')
        .map(|line| {
            let t = line.trim();
            if t.is_empty()
                || t.starts_with('$')
                || t.starts_with("**[")
                || starts_subpart_label(t)
                || starts_mcq_option(t)
                || has_prose_word(t)
                || t.starts_with("![")
                || t.contains("[DIAGRAM_PLACEHOLDER]")
            {
                return line.to_string();
            }
            if (t.contains('=') || t.contains("\\rightarrow")) && (t.contains('\\') || t.contains('^')) {
                format!("$${}$$", t)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize(raw: &str) -> Normalized {
    let raw = crate::sanitize::reconstruct_isotope_notation(&raw.replace('\r', ""));
    let ligature_fixed = validate::clean_ligatures(&raw).replace('\u{2212}', "-");
    let lines: Vec<String> = ligature_fixed.lines().map(str::to_string).collect();
    // Fractions BEFORE ghost filtering: a bare-number denominator ("2",
    // "10") is textually identical to a printed page number, so recovery
    // must claim it before the noise regex can delete it.
    let fractioned = recover_vertical_fractions(lines);
    let fractioned: Vec<String> = fractioned
        .into_iter()
        .filter(|l| !is_ghost_line(l))
        .collect();
    let (kept, answer_lines_seen) = strip_answer_lines(fractioned);
    let rejoined = conservative_rejoin(&kept);
    // Phase 3 local layout recovery: rebuild flattened multi-column tables and
    // split an option run whose first option is glued to a short header.
    let rejoined = recover_markdown_data_tables(rejoined);
    let rejoined = split_glued_header_option_run(rejoined);
    let blocks = ensure_subpart_paragraphs(&rejoined);
    let blocks = separate_mcq_options(blocks);
    let (marked, marks_total) = format_marks(&blocks);
    let marked_text = marked.join("\n");
    let converted = convert_unicode(&marked_text);
    let wrapped = wrap_math(&converted);
    let collapsed = regex::Regex::new(r"\n{3,}").unwrap().replace_all(&wrapped, "\n\n");
    let content = promote_display_equations(&collapsed);
    let content = isolate_block_media(&content);
    Normalized {
        content,
        marks_total,
        answer_lines_seen,
    }
}

// ── Confidence gate (hard gates, all-or-nothing) ────────────────────────────

fn check_gates(
    carved: &CarvedSpan,
    norm: &mut Normalized,
    span: &QuestionSpan,
    _config: &PipelineConfig,
    _available_figures: usize,
) -> Result<(), &'static str> {
    // NOTE: topics are NOT a gate. Production imports ALWAYS carry the
    // module taxonomy in `config.allowed_topics` (commands.rs loads it
    // unconditionally), so refusing here would disable Tier 0 everywhere.
    // Untagged cards are classified after extraction by the local matcher —
    // `pipeline::classify_topics_deferred`.
    if !carved.found_start || !carved.found_end {
        return Err("boundary_not_found");
    }
    let target_marks = span.expected_marks.or(carved.footer_marks);
    match target_marks {
        Some(expected) => {
            // A single-part question prints its allocation only in its own
            // footer: with no parts and no inline tags, the footer is the
            // whole allocation. Parts must still account for the footer.
            let footer_only = norm.marks_total == 0 && carved.subpart_numbers.is_empty() && carved.footer_marks == Some(expected);
            // Style-and-clarity allocations "(+S1)" belong to no single part.
            let style = validate::sum_style_marks(&norm.content);
            if norm.marks_total + style != expected && !footer_only {
                eprintln!("[MARKS_MISMATCH] Q{} norm={} expected={} (span_expected={:?} footer={:?})",
                    span.number, norm.marks_total, expected, span.expected_marks, carved.footer_marks);
                return Err("marks_checksum_mismatch");
            }
        }
        None => {
            if norm.marks_total == 0 {
                return Err("no_marks_signal");
            }
        }
    }
    let parts = &carved.subpart_numbers;
    // Flattened-math / nuclide debris: the text layer sometimes prints a
    // formula across columns so the extracted line is a run of bare numbers
    // and element symbols (or a short variable span trailed by symbol debris).
    // That is NOT a clean strict card. Local recovery keeps the content with a
    // review flag until glyph/layout evidence reconstructs it.
    if let Some(reason) = flattened_math_debris(&norm.content) {
        eprintln!("[FLATTENED_MATH] Q{}: {}", span.number, reason);
        return Err(reason);
    }
    // Detached margin allocations: exam papers print a question's allocation in
    // a margin/right column, so a footerless carve with MORE THAN ONE
    // allocation and NO sub-part labels is ambiguous - it may have picked up a
    // neighbouring question's allocation with no way to associate it. Keep the
    // content as a flagged recovery instead of reporting the sum as a
    // confident Tier-0 accept. Phase 3 reconstructs the layout and can remove
    // the uncertainty. (Source-layout evidence only: no paper or question is
    // special-cased.)
    // (A layout body spans exactly its own question's region: every
    // allocation in it is that question's.)
    if target_marks.is_none() && carved.subpart_numbers.is_empty() && !crate::pipeline::is_layout_question(_config, span) {
        let detached_allocations = MARK_TAG_RE.captures_iter(&norm.content).count();
        if detached_allocations > 1 {
            eprintln!(
                "[AMBIGUOUS_MARKS] Q{}: {} detached allocations with no sub-part labels",
                span.number, detached_allocations
            );
            return Err("ambiguous_mark_allocation");
        }
    }
    if !parts.is_empty() {
        let complete = parts
            .iter()
            .enumerate()
            .all(|(i, &p)| p == (i + 1) as u32);
        if !complete {
            return Err("subpart_gap");
        }
    }
    if LEFTOVER_BLANK_RE.is_match(&norm.content) || is_answer_line(norm.content.trim_end()) {
        return Err("leftover_answer_line");
    }
    // ── Figure-reference cost lever ──────────────────────────────────────
    // A figure reference used to refuse the carve outright, forcing every
    // figure question onto a paid LLM call. The deterministic detector
    // (`page_figures`) already supplies crop-able regions for free, and the
    // caller splices them into `[DIAGRAM_PLACEHOLDER]` tokens. So:
    // A graph-reading instruction is transcribed, not solved. Both reading
    // and illustrative references accept independently of crop detection;
    // the caller verifies persistence and flags missing attachments for review.
    // (A layout body already carries a placeholder where each figure is
    // printed.)
    if !crate::pipeline::is_layout_question(_config, span)
        && (FIGURE_REF_RE.is_match(&norm.content) || crate::pipeline::figure_read_required(&norm.content))
    {
        insert_figure_placeholders(&mut norm.content);
    }
    // MCQ questions legitimately end on their last option line; extraction
    // sometimes splits the value ("D 8" / "9 d"), so scan back a few lines.
    let non_empty: Vec<&str> = norm
        .content
        .lines()
        .rev()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .take(3)
        .collect();
        let ends_on_mcq_option = non_empty
            .iter()
            .any(|l| {
                if l.starts_with("- [MCQ:") { return true; }
                let b = l.as_bytes();
                !b.is_empty() && (b'A'..=b'E').contains(&b[0])
                    && (b.len() == 1 || matches!(b[1], b'.' | b')' | b' '))
            });
    let ends_on_choice_row = carved
        .text
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| {
            let cells: Vec<&str> = l.split('\t').map(str::trim).filter(|c| !c.is_empty()).collect();
            cells.len() >= 2 && cells.len() <= 8 && cells.iter().all(|c| c.chars().count() <= 24 && !c.contains('.'))
        })
        .unwrap_or(false);
    // (Reference material appended to a layout body ends where it is
    // printed, punctuated or not.)
    let ends_with_reference = _config.layout_questions.as_ref().and_then(|m| m.get(&span.number)).is_some_and(|b| b.ends_with_reference);
    if !validate::has_terminal_ending(&norm.content) && !ends_on_mcq_option && !ends_on_choice_row && !ends_with_reference {
        return Err("no_terminal_ending");
    }
    let len = norm.content.chars().count();
    if len < 30 {
        return Err("too_short");
    }
    // A text carve that lost content is short for its marks. A layout body
    // spans its whole region between headings, so only a near-empty body is
    // suspect there ("Prove that 0.4̇6̇2̇ = 229/495" is a complete 3-mark
    // question).
    let per_mark: u64 = if crate::pipeline::is_layout_question(_config, span) { 12 } else { 25 };
    if let Some(m) = target_marks {
        if m > 0 && (len as u64) < per_mark * m as u64 {
            return Err("too_short_for_marks");
        }
    }
    if !validate::math_delimiter_balance_errors(&norm.content).is_empty() {
        return Err("unbalanced_math");
    }
    // Flattened table rows (column gaps in plain text). Markdown table rows
    // are a recovered table, not a flattened one.
    let gap_flags: Vec<bool> = carved
        .text
        .lines()
        .map(|l| !l.trim_start().starts_with('|') && TABLE_GAP_RE.find_iter(l).count() >= 2)
        .collect();
    if gap_flags.windows(2).any(|w| w[0] && w[1]) {
        return Err("table_detected");
    }
    Ok(())
}

fn tier0_fallback(question: u32, reason: &str) {
    eprintln!("[TIER0_FALLBACK] question={} reason={}", question, reason);
}

/// Insert one `[DIAGRAM_PLACEHOLDER]` after the FIRST line mentioning each
/// distinct numbered figure. The caller (`attach_detected_figures`) splices
/// the deterministic crop into each placeholder and scrubs any leftovers.
fn insert_figure_placeholders(content: &mut String) {
    let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    let mut out = String::with_capacity(content.len() + 64);
    let mut seen: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    for line in &lines {
        out.push_str(line);
        out.push('\n');
        for num in crate::pipeline::figure_reference_numbers(line) {
            if seen.insert(num) {
                out.push_str("[DIAGRAM_PLACEHOLDER]\n");
                break;
            }
        }
    }
    // Unnumbered graph-reading instructions also need a crop attachment slot.
    if seen.is_empty() {
        out.push_str("[DIAGRAM_PLACEHOLDER]\n");
    }
    // Trailing newline bookkeeping: lines() lost the final terminator.
    while out.ends_with("\n\n") {
        out.pop();
    }
    *content = out;
}

// ── Layout path (geometric question map) ────────────────────────────────────

/// Normalise a question body produced by the geometry-first layout: the
/// mathematics is already reconstructed and delimited, reading order and
/// line boundaries are exact, and furniture/answer prompts were withheld from
/// source geometry. Only structural formatting remains: sub-part paragraphs,
/// MCQ option blocks, marks tags, and media isolation. Nothing is re-wrapped
/// or re-guessed.
/// "$y = 2x^{3} + 10$\t$x > 0$" (an equation and its domain printed apart)
/// becomes one formula with the gap kept: "$y = 2x^{3} + 10 \qquad x > 0$".
fn join_gapped_formulas(line: &str) -> String {
    let b = line.as_bytes();
    let mut out = String::with_capacity(line.len() + 8);
    let mut dollars = 0usize;
    let mut i = 0usize;
    while i < line.len() {
        let single = |k: usize| b[k] == b'$' && (k == 0 || b[k - 1] != b'$' && b[k - 1] != b'\\') && b.get(k + 1) != Some(&b'$');
        if b[i] == b'$' && dollars % 2 == 1 && single(i) && b.get(i + 1) == Some(&b'\t') && i + 2 < line.len() && single(i + 2) {
            out.push_str(" \\qquad ");
            dollars += 2;
            i += 3;
            continue;
        }
        if b[i] == b'$' && (i == 0 || b[i - 1] != b'\\') {
            dollars += 1;
        }
        let ch = line[i..].chars().next().unwrap_or('\u{fffd}');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn normalize_layout(raw: &str) -> Normalized {
    let mut in_code = false;
    let lines: Vec<String> = raw
        .lines()
        .map(|l| {
            let t = l.trim_end();
            // Code inside a fence is verbatim, indentation included.
            if t.trim_start().starts_with("```") {
                in_code = !in_code;
                return t.trim_start().to_string();
            }
            if in_code {
                return t.to_string();
            }
            // Column gaps are tab-separated; outside tables they read as a
            // space (a leading tab would render as a code block), except
            // between two formulas, where the gap stays wide.
            if t.trim_start().starts_with('|') {
                t.to_string()
            } else {
                join_gapped_formulas(t).replace('\t', " ").trim().to_string()
            }
        })
        .collect();
    let blocks = subpart_paragraphs(&lines, false);
    let blocks = separate_mcq_options(blocks);
    let (marked, marks_total) = format_marks(&blocks);
    let joined = marked.join("\n");
    let collapsed = regex::Regex::new(r"\n{3,}").unwrap().replace_all(&joined, "\n\n");
    let content = isolate_block_media(&collapsed);
    Normalized { content, marks_total, answer_lines_seen: 0 }
}

/// Difficulty rating decorators printed after a heading ("(*****)").
static RATING_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\A[ \t]*\(\s*\*{1,8}\s*\)[ \t]*").unwrap());

/// Carve + normalise a question from its mapped layout body. The boundaries
/// come from the geometric map (heading line to the next heading or an
/// end-of-paper marker), so start and end are always established.
fn layout_transcription(span: &QuestionSpan, body: &crate::layout::QuestionBody) -> (CarvedSpan, Normalized) {
    let text = crate::sanitize::strip_question_heading(&body.text, span.number);
    let text = RATING_RE.replace(&text, "").to_string();
    // Part headings that repeat the question number in the margin column
    // ("11 (b)", "0 3 (c)") keep only the part label.
    let digits = span.number.to_string().chars().map(|c| c.to_string()).collect::<Vec<_>>().join(r"[ \t]*");
    let part_prefix = regex::Regex::new(&format!(r"(?m)^[ \t]*0?[ \t]*{}[ \t]+(\((?:[a-h]|i{{1,3}}|iv|vi{{0,3}}|ix|x)\))", digits)).unwrap();
    let text = part_prefix.replace_all(&text, "$1").to_string();
    // AQA part numbers are printed in boxes: this question's own number, then
    // a spaced dot ("0 1 . 2", "1 3 . 1"). A decimal at a line start ("2.4%
    // for the first year") is never a part label.
    let aqa_part = regex::Regex::new(&format!(r"(?m)^[ \t]*(?:0[ \t]*)?{}[ \t]+\.[ \t]*([1-9])(?:[ \t]+|$)", digits)).unwrap();
    let text = aqa_part
        .replace_all(&text, |c: &regex::Captures| format!("({}) ", (b'a' + c[1].parse::<u8>().unwrap_or(1) - 1) as char))
        .into_owned();
    let text = recover_aqa_part_labels(&text, span.number);
    let text = bracket_bare_part_labels(&text);
    let carved = CarvedSpan {
        subpart_numbers: collect_subparts(&text, span.number),
        text: text.clone(),
        found_start: true,
        found_end: true,
        footer_marks: body.footer_marks,
    };
    let mut norm = normalize_layout(&text);
    norm.content = validate::balance_math_delimiters(&norm.content);
    (carved, norm)
}

/// Part labels printed without an opening bracket ("a) Determine …", Madas)
/// become "(a)" — only when they run a, b, c… in order at line starts, so a
/// stray "x) " in prose is never relabelled.
fn bracket_bare_part_labels(text: &str) -> String {
    static BARE_RE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?m)^([a-h])\)([ \t]+)").unwrap());
    let letters: Vec<u8> = BARE_RE.captures_iter(text).map(|c| c[1].as_bytes()[0]).collect();
    let in_order = letters.len() >= 2 && letters.iter().enumerate().all(|(i, &l)| l == b'a' + i as u8);
    if !in_order {
        return text.to_string();
    }
    BARE_RE.replace_all(text, "($1)$2").into_owned()
}

/// A glyph whose identity could not be established from the source is
/// carried as U+FFFD; such a card can never be a strict success.
fn unknown_glyph_gate(content: &str) -> Result<(), &'static str> {
    if content.contains(crate::layout::UNKNOWN_GLYPH) {
        Err("unidentified_source_glyph")
    } else {
        Ok(())
    }
}

// ── Entry points ────────────────────────────────────────────────────────────

/// Recover printed AQA zero-padded sub-part labels like `0 5 . 3` (and the
/// boxed `box 0 5 . 4` form) into the human `(c)`. Structural: the question
/// number comes from the span, and the part index maps a→1 … h→8. No paper or
/// question keyed literal.
fn recover_aqa_part_labels(content: &str, question_number: u32) -> String {
    let digits: Vec<String> = question_number
        .to_string()
        .chars()
        .map(|c| c.to_string())
        .collect();
    let qpat = digits.join(r"[ \t]*");
    // Require printed heading evidence: either a leading zero pad (`0 5 . 4`)
    // or a box token, and a space before the dot; and require the part digit to
    // be followed by whitespace/end so `5.25 kg` can never match.
    let pattern = format!(
        r"(?m)^[ \t]*(?:(?:box[ \t]+)(?:0[ \t]*)?|0[ \t]+){qpat}[ \t]+(?:box[ \t]+)*\.[ \t]*([1-9])([ \t]+|$)"
    );
    let Ok(re) = regex::Regex::new(&pattern) else {
        return content.to_string();
    };
    re.replace_all(content, |c: &regex::Captures| {
        let n: u32 = c[1].parse().unwrap_or(0);
        if (1..=8).contains(&n) {
            format!("({}){}", (b'a' + (n as u8 - 1)) as char, &c[2])
        } else {
            c[0].to_string()
        }
    })
    .to_string()
}

/// Repair the span's page texts with per-import local layout evidence. Text-only
/// callers (no evidence) get the original slices back unchanged.
fn repaired_span_texts(
    config: &PipelineConfig,
    span: &QuestionSpan,
    texts: &[&str],
    margin: Option<&crate::pdf_render::MarginModel>,
) -> Vec<String> {
    match config.layout_evidence.as_ref() {
        None => texts.iter().map(|s| (*s).to_string()).collect(),
        Some(ev) => texts
            .iter()
            .enumerate()
            .map(|(offset, base)| match ev.pages.get(span.start_page + offset) {
                Some(page) => {
                    let ranges = margin
                        .map(|m| m.ranges_for_page(span.start_page + offset))
                        .unwrap_or_default();
                    crate::pdf_render::repair_page_text_with_evidence(base, page, &ranges)
                }
                None => (*base).to_string(),
            })
            .collect(),
    }
}

/// Sum the printed right-margin allocations (`(5)`, `(6)`, …) whose vertical
/// position falls inside this span's y band. Madas prints per-question marks in
/// the right margin; associating them by y-region prevents a neighbouring
/// question's allocation (or a figure label) from being counted. Source
/// geometry only — no paper or question literal.
/// Associate every printed right-margin allocation with the question whose
/// heading is the nearest one above it (greatest heading y ≤ allocation y,
/// small tolerance for baseline rounding). Uses the MEASURED heading y from the
/// source LocalTextRun geometry (not the doc-map text-offset estimate), and the
/// span's end page as the upper region bound, so a neighbouring question or
/// backmatter allocation never lands on this card.
/// Detect a source inline superscript the transcription did not carry. Returns
/// the superscript text when a small-font numeric run (e.g. `1.5`) sits
/// immediately right of a body letter and the content does not contain it.
fn unrepresented_inline_superscript(
    config: &PipelineConfig,
    span: &QuestionSpan,
    content: &str,
) -> Option<String> {
    let ev = config.layout_evidence.as_ref()?;
    for pi in span.start_page..=span.end_page {
        let page = ev.pages.get(pi)?;
        let mut i = 0usize;
        while i < page.chars.len() {
            let c = &page.chars[i];
            if c.ch.is_ascii_digit() {
                let mut j = i;
                let mut run = String::new();
                while j < page.chars.len()
                    && (page.chars[j].ch.is_ascii_digit() || page.chars[j].ch == '.')
                {
                    run.push(page.chars[j].ch);
                    j += 1;
                }
                if run.len() >= 3 && run.contains('.') {
                    let run_max_font = page.chars[i..j]
                        .iter()
                        .map(|g| g.font_size)
                        .fold(0.0, f32::max);
                    // Nearest preceding letter on the same line: the LOCAL base.
                    // Walk backwards in SOURCE ORDER, skipping newlines and the
                    // smaller script glyphs, to the body letter the run is
                    // attached to (deterministic; no geometric guess).
                    let mut base: Option<&crate::pdf_render::LocalChar> = None;
                    let mut k = i;
                    while k > 0 {
                        k -= 1;
                        let o = &page.chars[k];
                        if o.ch.is_whitespace() {
                            continue;
                        }
                        if o.font_size <= run_max_font * 1.1 {
                            continue; // smaller script glyph → keep walking
                        }
                        if o.ch.is_ascii_alphabetic()
                            && o.bbox[2] > 0.0
                            && o.bbox[3] > 0.0
                            && (o.bbox[1] + o.bbox[3] / 2.0 - (c.bbox[1] + c.bbox[3] / 2.0)).abs()
                                < 0.02
                        {
                            base = Some(o);
                        }
                        break;
                    }
                    if let Some(base) = base {
                        let base_bottom = base.bbox[1] + base.bbox[3];
                        let base_h = base.bbox[3];
                        // TRUE raised geometry: every run glyph is small relative
                        // to the local base and sits above its baseline, close by.
                        let run_glyphs = &page.chars[i..j];
                        let raised = run_glyphs.iter().all(|g| {
                            g.font_size <= base.font_size * 0.85
                                && g.bbox[2] > 0.0
                                && g.bbox[3] > 0.0
                                && {
                                    let bottom = g.bbox[1] + g.bbox[3];
                                    let up = base_bottom - bottom;
                                    up > 0.1 * base_h && up <= 1.5 * base_h
                                }
                        });
                        if raised && !content.contains(&run) {
                            return Some(run);
                        }
                    }
                }
                i = j;
                continue;
            }
            i += 1;
        }
    }
    None
}

/// Build the per-import margin model ONCE (all_spans known). Returns confirmed
/// records only; ambiguous or missing-heading regions are rejected, so no
/// unassigned candidate is ever removed or counted.
pub fn build_margin_model(config: &PipelineConfig, spans: &[QuestionSpan]) -> crate::pdf_render::MarginModel {
    use std::collections::HashMap;
    let mut model = crate::pdf_render::MarginModel::default();
    let Some(ev) = config.layout_evidence.as_ref() else {
        return model;
    };
    // Known measured headings per page: (y, number).
    let mut known: HashMap<usize, Vec<(f32, u32)>> = HashMap::new();
    for s in spans {
        if let Some(page) = ev.pages.get(s.start_page) {
            if let Some(y) = measured_heading_y(page, s.number) {
                known.entry(s.start_page).or_default().push((y, s.number));
            }
        }
    }
    for v in known.values_mut() {
        v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    }
    for (pi, page) in ev.pages.iter().enumerate() {
        let Some(heads) = known.get(&pi) else {
            continue;
        };
        for (lo, hi, value, y) in crate::pdf_render::margin_allocations(page) {
            // Nearest measured heading at or above the token.
            let Some(idx) = heads
                .iter()
                .rposition(|(hy, _)| *hy <= y + 0.02)
            else {
                continue; // no heading above → unassigned, preserved
            };
            let (_, candidate) = heads[idx];
            let Some(span) = spans.iter().find(|s| s.number == candidate) else {
                continue;
            };
            if pi > span.end_page {
                continue; // token past this question's region
            }
            // If the next question is expected on this page, its measured
            // heading must exist and bound this region; otherwise reject as
            // ambiguous rather than assigning to the previous question.
            let next_expected_here = spans
                .iter()
                .any(|s| s.number == candidate + 1 && s.start_page == pi);
            if next_expected_here {
                let next_y = heads.iter().find(|(_, n)| *n == candidate + 1).map(|(hy, _)| *hy);
                match next_y {
                    Some(ny) if y < ny => {}
                    _ => continue,
                }
            }
            *model.per_question.entry(candidate).or_insert(0) += value;
            model.records.push(crate::pdf_render::MarginRecord {
                page: pi,
                start: lo,
                end: hi,
                question: candidate,
                value,
            });
        }
    }
    model
}

/// Measured y of a question's heading line, from the source LocalTextRun
/// geometry. Returns `None` when the heading run is missing or appears more
/// than once (ambiguous), so association is rejected rather than guessed.
fn measured_heading_y(page: &crate::pdf_render::PageLayoutEvidence, number: u32) -> Option<f32> {
    let digits: Vec<String> = number
        .to_string()
        .chars()
        .map(|c| c.to_string())
        .collect();
    let qpat = digits.join(r"\s*");
    // Prefer an explicit "Question N" / "Q N" run; only fall back to a bare
    // number when it sits in the left margin. Require a unique match.
    let named = regex::Regex::new(&format!(
        r"(?i)^\s*(?:question|q)\s*0*\s*{qpat}\s*[.:\)]?\s*$"
    ))
    .ok()?;
    let bare = regex::Regex::new(&format!(r"^\s*0*\s*{qpat}\s*[.:\)]?\s*$")).ok()?;
    let mut ys: Vec<f32> = page
        .runs
        .iter()
        .filter(|r| named.is_match(r.text.trim()))
        .map(|r| r.bbox[1])
        .collect();
    if ys.is_empty() {
        ys = page
            .runs
            .iter()
            .filter(|r| bare.is_match(r.text.trim()) && r.bbox[0] < 0.2)
            .map(|r| r.bbox[1])
            .collect();
    }
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    ys.dedup_by(|a, b| (*a - *b).abs() < 0.005);
    if ys.len() == 1 {
        Some(ys[0])
    } else {
        None
    }
}


fn transcribe_span(
    config: &PipelineConfig,
    span: &QuestionSpan,
    texts: &[&str],
    paper_last_page: usize,
    available_figures: usize,
) -> Result<String, &'static str> {
    transcribe_span_with_marks(config, span, texts, paper_last_page, available_figures, None)
}

fn transcribe_span_with_marks(
    config: &PipelineConfig,
    span: &QuestionSpan,
    texts: &[&str],
    paper_last_page: usize,
    available_figures: usize,
    margin: Option<&crate::pdf_render::MarginModel>,
) -> Result<String, &'static str> {
    let allow_run_to_end = span.end_page >= paper_last_page;
    // Repair the span's page text from real character geometry before carving
    // (text-only callers are unchanged).
    let repaired = repaired_span_texts(config, span, texts, margin);
    let repaired_refs: Vec<&str> = repaired.iter().map(String::as_str).collect();
    let texts: &[&str] = &repaired_refs;
    let carved = carve_span(span, texts, allow_run_to_end);
    // Symbol-font fragments have no recoverable semantics in the text layer
    // (for example vertically assembled parentheses around stacked powers).
    if carved.text.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c)) {
        return Err("unmapped_symbol_font_glyphs");
    }
    let body = crate::sanitize::strip_question_heading(&carved.text, span.number);
    let body = crate::marker_client::convert_aqa_decimal_parts(&body);
    let body = recover_aqa_part_labels(&body, span.number);
    let mut body = body;
    if let Some(n) = margin.and_then(|m| m.per_question.get(&span.number).copied()) {
        if !body.contains("marks]") && !body.contains("mark]") {
            body.push_str(&format!("\n\n[{} marks]", n));
        } else if validate::sum_inline_marks(&body) != n {
            return Err("mark_allocation_mismatch");
        }
    }
    let mut norm = normalize(&body);
    // Conservative pre-gate repair: close minor unescaped single-dollar
    // defects (and unterminated display blocks) BEFORE `unbalanced_math` is
    // evaluated, so a repairable card is not demoted to a recovery. The
    // assembler applies the same idempotent healer to every card, so the
    // content that reaches the gate is what the card will actually contain.
    norm.content = validate::balance_math_delimiters(&norm.content);
    // An inline source superscript that is not represented in the result must
    // not be reported as a clean strict card.
    // Check the SANITIZED content: a run present pre-sanitize but dropped by
    // the sanitizer must still fail the gate.
    let sanitized_for_gate =
        crate::sanitize::sanitize_question_content(&norm.content, span.number);
    if let Some(sup) = unrepresented_inline_superscript(config, span, &sanitized_for_gate) {
        eprintln!("[SUPERSCRIPT] Q{} unrecovered {:?}", span.number, sup);
        return Err("source_superscript_unrecovered");
    }
    check_gates(&carved, &mut norm, span, config, available_figures)?;
    Ok(norm.content)
}

/// Tier-0 deterministic extraction for ONE span. Returns
/// `Some((question, report))` when the span was carved locally with every
/// gate green; the caller attaches figures / mark-checks exactly as it does
/// for the LLM text-first path. `None` ⇒ caller falls back unchanged.
pub fn try_deterministic_extraction(
    config: &PipelineConfig,
    span: &QuestionSpan,
    span_pages: &[(usize, &PageInput)],
    available_figures: usize,
    paper_last_page: usize,
    cancel: &AtomicBool,
    margin: Option<&crate::pdf_render::MarginModel>,
) -> Option<(BuiltQuestion, ImportReport)> {
    let started = Instant::now();
    let mut report = ImportReport::default();
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let texts: Option<Vec<&str>> = (span.start_page..=span.end_page)
        .map(|pi| {
            span_pages
                .iter()
                .find(|(p, _)| *p == pi)
                .map(|(_, pg)| pg.text.as_str())
        })
        .collect();
    let texts = texts?;
    let layout_body = config.layout_questions.as_ref().and_then(|m| m.get(&span.number)).cloned();
    let transcribed = match layout_body {
        Some(body) => {
            let (carved, mut norm) = layout_transcription(span, &body);
            unknown_glyph_gate(&norm.content)
                .and_then(|_| check_gates(&carved, &mut norm, span, config, available_figures))
                .map(|_| norm.content)
        }
        None => transcribe_span_with_marks(config, span, &texts, paper_last_page, available_figures, margin),
    };
    let content = match transcribed
    {
        Ok(c) => c,
        Err(reason) => {
            tier0_fallback(span.number, reason);
            return None;
        }
    };
    // Structural formatting gate: the transcription must satisfy the same
    // formatting rules as every LLM card. Evaluated on SANITIZED content —
    // the sanitizer's free fixes (e.g. wrapping stray \text{kg} into math)
    // must not burn a Tier-0 accept and push a question onto the paid path.
    // A transcription that still cannot comply (e.g. scrambled fractions the
    // text layer mangled) falls through to the LLM text-first path.
    // Layout bodies are already source-faithful; the marker/LLM repair pass
    // is only for text-layer and model output.
    let cleaned = if crate::pipeline::is_layout_question(config, span) {
        content.clone()
    } else {
        crate::marker_client::clean_marker_markdown(&content)
    };
    let sanitized = crate::sanitize::sanitize_question_content(&cleaned, span.number);
    let structural_errors = crate::validate::card_structure_errors(&sanitized, span.number);
    if !structural_errors.is_empty() {
        tier0_fallback(
            span.number,
            structural_errors
                .first()
                .map(|e| e.as_str())
                .unwrap_or("structure"),
        );
        return None;
    }
    let content = sanitized;
    // Plain text does not retain column geometry. A named table without
    // Markdown cells must go through the layout-aware transcription fallback.
    if regex::Regex::new(r"(?i)\btable\s+\d+\b").unwrap().is_match(&content) && !content.contains('|') {
        tier0_fallback(span.number, "table_columns_not_recovered");
        return None;
    }
    let marks = validate::sum_inline_marks(&content);
    let Some(built) = crate::pipeline::build_question_from_seam(
        content.clone(),
        Some(marks),
        span,
        config,
        available_figures,
    ) else {
        tier0_fallback(span.number, "seam_validation");
        return None;
    };
    eprintln!(
        "[TIER0] Question {} carved locally (0 tokens)",
        span.number
    );
    report.record_timing(
        "extraction",
        "tier0",
        Some(span.start_page + 1),
        Some(span.number),
        started.elapsed().as_millis() as u64,
    );
    Some((built, report))
}

/// All-or-nothing Tier-0 attempt for a shared page: EVERY target span must
/// carve cleanly, otherwise `None` (caller falls back to the combined LLM
/// call unchanged — fallback-for-all semantics).
pub fn try_deterministic_batch(
    config: &PipelineConfig,
    spans: &[&QuestionSpan],
    page_idx: usize,
    page: &PageInput,
    fig_counts: &[usize],
    paper_last_page: usize,
    cancel: &AtomicBool,
) -> Option<Vec<BuiltQuestion>> {
    // Digital documents force the deterministic cascade regardless of the
    // tuning switch, exactly as the single-span path does.
    if cancel.load(Ordering::Relaxed) || !(config.deterministic || config.is_digital_document()) {
        return None;
    }
    let mut out = Vec::with_capacity(spans.len());
    for (i, span) in spans.iter().enumerate() {
        let available_figures = fig_counts.get(i).copied().unwrap_or(0);
        let (q, _) = try_deterministic_extraction(
            config,
            span,
            &[(page_idx, page)],
            available_figures,
            paper_last_page,
            cancel,
            None,
        )?;
        out.push(q);
    }
    Some(out)
}

/// Minimum characters of carved content before a failed-gate carve counts as
/// a viable candidate worth retaining. Below this the span is an explicit
/// local failure, not a recoverable card.
const MIN_RECOVERED_CHARS: usize = 20;

/// Zero-cloud local recovery for a span whose strict Tier-0 gates failed.
///
/// Born-digital documents may not make ANY model request, so a failed gate
/// must resolve locally: carve the span, keep the real content, and flag it
/// for review with the failed gate recorded. This never invents content,
/// marks, or boundaries - when the carve cannot isolate the question at all
/// (`found_start`/`found_end` absent) the function returns `None` and the
/// caller reports an explicit local failure instead.
///
/// Returns `(question, report, failed_gate_reason)`.
pub fn try_local_recovery(
    config: &PipelineConfig,
    span: &QuestionSpan,
    span_pages: &[(usize, &PageInput)],
    available_figures: usize,
    paper_last_page: usize,
    cancel: &AtomicBool,
    margin: Option<&crate::pdf_render::MarginModel>,
) -> Option<(BuiltQuestion, ImportReport, String)> {
    let started = Instant::now();
    let mut report = ImportReport::default();
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let texts: Option<Vec<&str>> = (span.start_page..=span.end_page)
        .map(|pi| {
            span_pages
                .iter()
                .find(|(p, _)| *p == pi)
                .map(|(_, pg)| pg.text.as_str())
        })
        .collect();
    let texts = texts?;
    let allow_run_to_end = span.end_page >= paper_last_page;
    let layout_body = config.layout_questions.as_ref().and_then(|m| m.get(&span.number)).cloned();
    if let Some(body) = layout_body {
        let (carved, mut norm) = layout_transcription(span, &body);
        let mut reason: Option<String> = unknown_glyph_gate(&norm.content).err().map(str::to_string);
        if reason.is_none() {
            reason = check_gates(&carved, &mut norm, span, config, available_figures).err().map(|r| r.to_string());
        }
        let sanitized = crate::sanitize::sanitize_question_content(&norm.content, span.number);
        if sanitized.trim().chars().count() < MIN_RECOVERED_CHARS {
            tier0_fallback(span.number, "recovery_too_little_content");
            return None;
        }
        let structural = crate::validate::card_structure_errors(&sanitized, span.number);
        if reason.is_none() && !structural.is_empty() {
            reason = Some(format!("structure: {}", structural.join("; ")));
        }
        let failed_gate = reason.unwrap_or_else(|| "strict_seam_validation".to_string());
        let inline = validate::sum_inline_marks(&sanitized);
        let marks_hint = if inline > 0 { Some(inline as i32) } else { span.expected_marks.map(|m| m as i32) };
        let built = crate::pipeline::build_recovered_question(sanitized, marks_hint, span, config, &failed_gate)?;
        eprintln!("[LOCAL_RECOVERY] question={} gate={} retained for review (0 tokens)", span.number, failed_gate);
        report.record_timing("extraction", "local_recovery", Some(span.start_page + 1), Some(span.number), started.elapsed().as_millis() as u64);
        return Some((built, report, failed_gate));
    }
    let repaired = repaired_span_texts(config, span, &texts, margin);
    let repaired_refs: Vec<&str> = repaired.iter().map(String::as_str).collect();
    let carved = carve_span(span, &repaired_refs, allow_run_to_end);
    // Boundaries are the one thing we will not invent. Without an isolated
    // start AND end there is no honest candidate to keep.
    if !carved.found_start || !carved.found_end {
        tier0_fallback(span.number, "recovery_boundary_not_found");
        return None;
    }
    if carved.text.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c)) {
        tier0_fallback(span.number, "recovery_unmapped_symbol_font_glyphs");
        return None;
    }

    let body = crate::sanitize::strip_question_heading(&carved.text, span.number);
    let body = crate::marker_client::convert_aqa_decimal_parts(&body);
    let body = recover_aqa_part_labels(&body, span.number);
    let mut body = body;
    let mut forced_mismatch = false;
    if let Some(n) = margin.and_then(|m| m.per_question.get(&span.number).copied()) {
        if !body.contains("marks]") && !body.contains("mark]") {
            body.push_str(&format!("\n\n[{} marks]", n));
        } else if validate::sum_inline_marks(&body) != n {
            forced_mismatch = true;
        }
    }
    let mut norm = normalize(&body);
    // Same conservative pre-gate repair as the strict path.
    norm.content = validate::balance_math_delimiters(&norm.content);

    let mut reason: Option<String> = check_gates(&carved, &mut norm, span, config, available_figures)
        .err()
        .map(|r| r.to_string());
    if forced_mismatch && reason.is_none() {
        reason = Some("mark_allocation_mismatch".to_string());
    }
    if reason.is_none() {
        let sanitized_for_gate =
            crate::sanitize::sanitize_question_content(&norm.content, span.number);
        if let Some(sup) = unrepresented_inline_superscript(config, span, &sanitized_for_gate) {
            eprintln!("[SUPERSCRIPT] Q{} unrecovered {:?}", span.number, sup);
            reason = Some("source_superscript_unrecovered".to_string());
        }
    }

    // Gates may have bailed before the figure block ran; give the crops their
    // attachment slots either way.
    if FIGURE_REF_RE.is_match(&norm.content)
        || crate::pipeline::figure_read_required(&norm.content)
    {
        insert_figure_placeholders(&mut norm.content);
    }

    let cleaned = crate::marker_client::clean_marker_markdown(&norm.content);
    let sanitized = crate::sanitize::sanitize_question_content(&cleaned, span.number);
    if sanitized.trim().chars().count() < MIN_RECOVERED_CHARS {
        tier0_fallback(span.number, "recovery_too_little_content");
        return None;
    }
    let structural = crate::validate::card_structure_errors(&sanitized, span.number);
    if reason.is_none() && !structural.is_empty() {
        reason = Some(format!("structure: {}", structural.join("; ")));
    }
    let failed_gate = reason.unwrap_or_else(|| "strict_seam_validation".to_string());

    let inline = validate::sum_inline_marks(&sanitized);
    let marks_hint = if inline > 0 {
        Some(inline as i32)
    } else {
        span.expected_marks.map(|m| m as i32)
    };
    let built = crate::pipeline::build_recovered_question(
        sanitized,
        marks_hint,
        span,
        config,
        &failed_gate,
    )?;
    eprintln!(
        "[LOCAL_RECOVERY] question={} gate={} retained for review (0 tokens)",
        span.number, failed_gate
    );
    report.record_timing(
        "extraction",
        "local_recovery",
        Some(span.start_page + 1),
        Some(span.number),
        started.elapsed().as_millis() as u64,
    );
    Some((built, report, failed_gate))
}

/// Diagnostics harness for fixture calibration (accept rate + refusal
/// reasons per paper). Not an assertion surface.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn tier0_diagnose_span(
    config: &PipelineConfig,
    span: &QuestionSpan,
    page_texts: &[String],
    paper_last_page: usize,
    available_figures: usize,
) -> Result<usize, &'static str> {
    let texts: Vec<&str> = (span.start_page..=span.end_page)
        .filter(|&p| p < page_texts.len())
        .map(|p| page_texts[p].as_str())
        .collect();
    match transcribe_span(config, span, &texts, paper_last_page, available_figures) {
        Ok(content) => {
            let content = crate::sanitize::sanitize_question_content(&content, span.number);
            if !validate::card_structure_errors(&content, span.number).is_empty() {
                return Err("structure");
            }
            let marks = validate::sum_inline_marks(&content);
            crate::pipeline::build_question_from_seam(content.clone(), Some(marks), span, config, available_figures)
                .ok_or("seam_validation")?;
            Ok(content.chars().count())
        }
        Err(reason) => Err(reason),
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Tests — synthetic page texts only; no pdfium required.
// ════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {

    #[test]
    #[ignore = "manual corpus diagnostic (presence-dependent PDFs); phase 4 runs the explicit corpus gate"]
    fn phase2_boundary_agreement_diagnostic() {
        // Diagnostics only (no assertions): prints, for every map span of a
        // paper, the carver's boundary result and the first blocking reason.
        // Used to separate boundary defects from glyph/gate defects.
        let _guard = crate::pdf_render::pdfium_test_lock();
        for path in [
            "../past papers for mergemark/madas_paper_2_t.pdf",
            "../past papers for mergemark/core pure 1 '21.pdf",
            "../physics '21.pdf",
        ] {
            if !std::path::Path::new(path).exists() {
                eprintln!("== {path}: MISSING");
                continue;
            }
            let pages = crate::pdf_render::render_pdf_pages(std::path::Path::new(path))
                .unwrap_or_else(|e| panic!("{path}: {e}"));
            let page_texts: Vec<String> = pages.iter().map(|p| p.text.clone()).collect();
            let scan = crate::doc_map::scan_text_layer(&page_texts);
            let map = crate::doc_map::build_text_only_map(&page_texts, pages.len(), &scan);
            let paper_last_page = map.spans.iter().map(|s| s.end_page).max().unwrap_or(0);
            let cfg = PipelineConfig::new(
                "diag".into(),
                "diag".into(),
                "Maths".into(),
                "Mod".into(),
                None,
            );
            eprintln!(
                "== {path}: {} pages, {} span(s), {} heading(s)",
                pages.len(),
                map.spans.len(),
                scan.headings.len()
            );
            for span in &map.spans {
                let texts: Vec<&str> = (span.start_page..=span.end_page)
                    .filter(|&p| p < page_texts.len())
                    .map(|p| page_texts[p].as_str())
                    .collect();
                let carved = carve_span(span, &texts, span.end_page >= paper_last_page);
                let glyphs = carved
                    .text
                    .chars()
                    .any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c));
                let reason = if !carved.found_start {
                    "start_missing".to_string()
                } else if glyphs {
                    "glyph_blocked".to_string()
                } else {
                    match transcribe_span(&cfg, span, &texts, paper_last_page, 0) {
                        Ok(_) => "strict_ok".to_string(),
                        Err(e) => e.to_string(),
                    }
                };
                eprintln!(
                    "  Q{} pages {}..={} found_start={} found_end={} glyphs={} reason={}",
                    span.number,
                    span.start_page + 1,
                    span.end_page + 1,
                    carved.found_start,
                    carved.found_end,
                    glyphs,
                    reason
                );
                // Detail dump for spans whose endings/delimiters are the
                // phase-2 target: what the normalizer produced and whether the
                // existing balancer would fix it.
                let detail = match path {
                    p if p.contains("physics") => matches!(span.number, 5 | 6 | 9 | 25 | 27 | 31),
                    p if p.contains("madas") => matches!(span.number, 1 | 2),
                    _ => false,
                };
                if detail && carved.found_start {
                    let body =
                        crate::sanitize::strip_question_heading(&carved.text, span.number);
                    let body = crate::marker_client::convert_aqa_decimal_parts(&body);
                    let norm = normalize(&body);
                    let errs = validate::math_delimiter_balance_errors(&norm.content);
                    let repaired = validate::balance_math_delimiters(&norm.content);
                    let after = validate::math_delimiter_balance_errors(&repaired);
                    eprintln!("  --- Q{} normalized ---", span.number);
                    eprintln!("{}", norm.content);
                    eprintln!(
                        "  --- Q{} balance_errors={:?} after_balance={:?} terminal={} ---",
                        span.number,
                        errs,
                        after,
                        validate::has_terminal_ending(&norm.content)
                    );
                }
            }
        }
    }

    #[test]
    fn flattened_formula_debris_is_gated_not_strict() {
        assert_eq!(
            flattened_math_debris("3 17 20 He $+ O$ Ne $2 8 10 \\rightarrow$"),
            Some("flattened_nuclide_row")
        );
        assert_eq!(
            flattened_math_debris("$k p 2$ eBR E"),
            Some("flattened_symbol_sequence")
        );
        // A relation span in prose (`cost $= 10$`) is NOT specific enough to
        // gate on its own; it is left for the equation reconstruction path.
        assert_eq!(flattened_math_debris("cost $= 10$"), None);
        // Ordinary prose, table rows and figures are not debris.
        assert_eq!(flattened_math_debris("The value of $x$ is stored in the DNA sample."), None);
        assert_eq!(flattened_math_debris("|Cyclotron|B / T|R / m|"), None);
        assert_eq!(flattened_math_debris("![Diagram](d.png)"), None);
        // A prose option that quotes "1 N", "1 m" and "1 C" is not a nuclide row.
        assert_eq!(
            flattened_math_debris("D the charge on a metal sphere which experiences a force of 1 N when its centre is placed 1 m from the centre of a metal sphere that carries 1 C of charge"),
            None
        );
    }

    #[test]
    fn margin_model_requires_provenance_and_rejects_ambiguity() {
        use crate::pdf_render::{ImportEvidence, LocalChar, LocalRule, LocalTextRun, PageLayoutEvidence};
        let run = |text: &str, y: f32| LocalTextRun {
            text: text.into(),
            bbox: [0.1, y, 0.2, 0.02],
            font_size: None,
        };
        let lc = |index: usize, ch: char, x: f32, y: f32| LocalChar {
            index,
            ch,
            bbox: [x, y, 0.006, 0.010],
            font_size: 12.0,
        };
        let page = |chars: Vec<LocalChar>, runs: Vec<LocalTextRun>| {
            let text: String = chars.iter().map(|c| c.ch).collect();
            PageLayoutEvidence {
                page: 0,
                text,
                chars,
                runs,
                rules: Vec::<LocalRule>::new(),
                paths: Vec::new(),
                fully_mapped: true,
            }
        };
        let token = |base_index: usize, x: f32, y: f32, digit: char| {
            vec![
                lc(base_index, '(', x, y),
                lc(base_index + 1, digit, x + 0.008, y),
                lc(base_index + 2, ')', x + 0.016, y),
            ]
        };
        let span = |number: u32, start_page: usize, end_page: usize| QuestionSpan {
            number,
            start_page,
            end_page,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: None,
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        };
        let cfg_with = |pages: Vec<PageLayoutEvidence>| {
            let mut cfg = PipelineConfig::new("m".into(), "p".into(), "Maths".into(), "Mod".into(), None);
            cfg.layout_evidence = Some(std::sync::Arc::new(ImportEvidence { pages, layout: Vec::new() }));
            cfg
        };

        // (a) Two headings, two isolated allocations → clear association.
        let mut chars: Vec<LocalChar> = Vec::new();
        chars.extend(token(0, 0.85, 0.30, '6'));
        chars.extend(token(3, 0.85, 0.70, '5'));
        let page_a = page(
            chars,
            vec![run("Question 1", 0.10), run("Question 2", 0.50)],
        );
        let cfg = cfg_with(vec![page_a]);
        let model = build_margin_model(&cfg, &[span(1, 0, 0), span(2, 0, 0)]);
        assert_eq!(model.per_question.get(&1), Some(&6), "{:?}", model.per_question);
        assert_eq!(model.per_question.get(&2), Some(&5), "{:?}", model.per_question);
        assert_eq!(model.records.len(), 2);

        // (b) No heading above the token → unassigned, preserved.
        let mut chars: Vec<LocalChar> = Vec::new();
        chars.extend(token(0, 0.85, 0.05, '5'));
        let cfg = cfg_with(vec![page(chars, vec![run("Question 1", 0.10)])]);
        let model = build_margin_model(&cfg, &[span(1, 0, 0)]);
        assert!(model.per_question.is_empty() && model.records.is_empty());

        // (c) Missing next heading on the same page → ambiguous region, reject.
        let mut chars: Vec<LocalChar> = Vec::new();
        chars.extend(token(0, 0.85, 0.30, '6'));
        let cfg = cfg_with(vec![page(chars, vec![run("Question 1", 0.10)])]);
        let model = build_margin_model(&cfg, &[span(1, 0, 0), span(2, 0, 0)]);
        assert!(model.per_question.is_empty() && model.records.is_empty());

        // (d) Token on a page with no measured heading (backmatter) → preserved.
        let mut chars: Vec<LocalChar> = Vec::new();
        chars.extend(token(0, 0.85, 0.30, '9'));
        let cfg = cfg_with(vec![page(chars, vec![])]);
        let model = build_margin_model(&cfg, &[span(1, 0, 0)]);
        assert!(model.per_question.is_empty() && model.records.is_empty());
    }

    #[test]
    fn aqa_padded_part_labels_become_human_labels() {
        let raw = "0 5 . 1 Explain the path.\n0 5 box . 2 The peak pd.\n0 5 . 3 Show that Ek is given by\nbox 0 5 . 4 A hospital decides.";
        let out = recover_aqa_part_labels(raw, 5);
        assert!(out.contains("(a) Explain the path."), "{out}");
        assert!(out.contains("(b) The peak pd."), "{out}");
        assert!(out.contains("(c) Show that Ek"), "{out}");
        assert!(out.contains("(d) A hospital decides."), "{out}");
        // A different question number's labels are not touched.
        assert_eq!(recover_aqa_part_labels("0 5 . 1 x", 6), "0 5 . 1 x");
        // Bare decimals / code must never be read as a padded part heading.
        assert_eq!(recover_aqa_part_labels("5.25 kg", 5), "5.25 kg");
        assert_eq!(
            recover_aqa_part_labels("const v = 5.25; // 0 5", 5),
            "const v = 5.25; // 0 5"
        );
    }

    #[test]
    fn phase3_table_recovery_and_glued_option_header() {
        // Table 2 shape: two columns, the header's "/ unit" separator marks the
        // cell boundary, and the rows' trailing value marks the column count.
        let table = vec![
            "Table 2".to_string(),
            "Nucleus Mass / u".to_string(),
            "$^{3}_{2}\\text{He}$ 3.01603".to_string(),
            "$^{17}_{8}\\text{O}$ 16.99913".to_string(),
            "Calculate, in J, the energy released when this reaction occurs.".to_string(),
        ];
        let out = recover_markdown_data_tables(table);
        assert!(
            out.iter().any(|l| l.contains("| Nucleus | Mass / u |")),
            "{out:?}"
        );
        assert!(
            out.iter()
                .any(|l| l.contains("| $^{3}_{2}\\text{He}$ | 3.01603 |")),
            "{out:?}"
        );
        assert!(out.iter().any(|l| l.starts_with("| --- ")), "{out:?}");

        // A pipe row must never be wrapped as one math span: the pipes stay
        // outside `$...$` and a plain "Label / unit" cell stays plain text.
        let row = "| Cyclotron | B / \\text{T} | R / \\text{m} |";
        let wrapped = wrap_table_math_cells(row);
        assert_eq!(wrapped, "|Cyclotron|$B / \\text{T}$|$R / \\text{m}$|", "{wrapped}");
        let plain = wrap_table_math_cells("| Nucleus | Mass / u |");
        assert_eq!(plain, "|Nucleus|Mass / u|", "{plain}");

        // Without a unit separator in the header the block is left untouched:
        // no invented column split.
        let no_units = vec![
            "Table 3".to_string(),
            "Alpha Beta".to_string(),
            "x 1".to_string(),
            "y 2".to_string(),
        ];
        assert_eq!(recover_markdown_data_tables(no_units.clone()), no_units);

        // Glued header + option A ("x y A ...") followed by a B/C/D run.
        let glued = vec![
            "x y A pressure in Pa temperature in ºC".to_string(),
            "B temperature in ºC pressure in Pa".to_string(),
            "C pressure in Pa temperature in K".to_string(),
            "D temperature in K pressure in Pa".to_string(),
        ];
        let out = split_glued_header_option_run(glued);
        assert_eq!(out[0], "x y");
        assert!(out[1].starts_with("A pressure"), "{out:?}");

        // Prose that merely begins with A is untouched.
        let prose = vec![
            "A room contains 200 items and a table.".to_string(),
            "B".to_string(),
            "Second line.".to_string(),
        ];
        assert_eq!(split_glued_header_option_run(prose.clone()), prose);
    }

    #[test]
    fn phase3_two_column_mcq_table_is_tagged() {
        let text = "0 1 A fixed mass of gas is heated at constant volume. The graph is drawn for this process.\n\
                    What do x and y represent?\n[1 mark]\n\
                    x y A pressure in Pa temperature in ºC\n\
                    B temperature in ºC pressure in Pa\n\
                    C pressure in Pa temperature in K\n\
                    D temperature in K pressure in Pa\n\
                    2 The next question starts here. [1 mark]\n";
        let cfg = PipelineConfig::new("m".into(), "p".into(), "Physics".into(), "Mod".into(), None);
        let content = transcribe_span(&cfg, &span(1, 0, 0, None), &[text], 0, 0)
            .expect("two-column MCQ must pass the local gates");
        let sanitized = crate::sanitize::sanitize_question_content(&content, 1);
        assert_eq!(
            sanitized.matches("- [MCQ:").count(),
            4,
            "all four options tagged: {sanitized}"
        );
        assert!(
            crate::validate::card_structure_errors(&sanitized, 1).is_empty(),
            "{sanitized}"
        );
    }

    #[test]
    fn phase3_numeric_mcq_options_survive_the_carver() {
        let page = "36\n*36*\nIB/M/Jun21/7408/2\nDo not write \noutside the \n2 6 box The diagram shows the path of a proton being deflected by the nucleus of an atom.\nPoint P is the position of the proton when it is closest to the nucleus.\nWhat is not true about the proton along its path at P?\n[1 mark]\nA Its rate of change of momentum is at a minimum.\nB Its kinetic energy is at a minimum.\nC Its potential energy is at a maximum.\nD Its acceleration is at a maximum.\n2 7 The diagram shows an area of 0.10 m2 normal to a line connecting it to a point source of \ngamma radiation. The source emits photons uniformly in all directions.\nThe area and the source are separated by a distance of 2.0 m.\nThe source emits 5000 gamma photons per second.\nHow many photons pass through the area every second?\n[1 mark]\nA 500\nB 250\nC 10\nD 2.5";
        // Regression source: a numeric MCQ run on the LAST lines of a page
        // ("A 500" / "B 250") that a zone-based board-code rule used to eat.
        let carved = carve_span(&span(27, 0, 0, None), &[page], false);
        assert!(carved.found_start && carved.found_end, "{carved:?}");
        for option in ["A 500", "B 250", "C 10", "D 2.5"] {
            assert!(
                carved.text.contains(option),
                "option {option:?} must survive the carver: {}",
                carved.text
            );
        }
        // A real board-code line is still furniture in the page zone.
        assert!(is_furniture("PHY 1234/01", Some((0, 26, 5))));
        assert!(is_furniture("P67097A", Some((25, 26, 5))));
        assert!(!is_furniture("A 500", Some((22, 26, 5))));
        assert!(!is_furniture("B 250", Some((23, 26, 5))));

        // The deterministic gate accepts the span, and the sanitizer tags the
        // four options without inventing an isotope.
        let cfg = PipelineConfig::new("m".into(), "p".into(), "Physics".into(), "Mod".into(), None);
        let content = transcribe_span(&cfg, &span(27, 0, 0, None), &[page], 0, 0)
            .expect("Q27-shaped numeric MCQ must pass the local gates");
        let sanitized = crate::sanitize::sanitize_question_content(&content, 27);
        assert_eq!(
            sanitized.matches("- [MCQ:").count(),
            4,
            "all four numeric options tagged: {sanitized}"
        );
        assert!(
            crate::validate::card_structure_errors(&sanitized, 27).is_empty(),
            "no isotope false positive: {:?}",
            crate::validate::card_structure_errors(&sanitized, 27)
        );
    }

    /// A config whose span bodies come from the geometric layout map.
    fn layout_config(bodies: Vec<(u32, &str, Option<u32>)>) -> PipelineConfig {
        let mut cfg = PipelineConfig::new("m".into(), "p".into(), "Mathematics".into(), "Mod".into(), None);
        let map: std::collections::HashMap<u32, crate::layout::QuestionBody> = bodies
            .into_iter()
            .map(|(n, text, footer_marks)| (n, crate::layout::QuestionBody { text: text.to_string(), footer_marks, pages: (0, 0), undetected_figures: 0, ends_with_reference: false }))
            .collect();
        cfg.layout_questions = Some(std::sync::Arc::new(map));
        cfg
    }

    fn layout_extract(cfg: &PipelineConfig, s: &QuestionSpan) -> Option<BuiltQuestion> {
        let page = PageInput { kind: crate::pipeline::PageInputKind::TextOnly, text: String::new() };
        try_deterministic_extraction(cfg, s, &[(0, &page)], 0, 0, &AtomicBool::new(false), None).map(|(q, _)| q)
    }

    #[test]
    fn single_part_question_with_footer_only_allocation_is_strict() {
        // Edexcel GCSE prints a single-part question's marks only in its
        // "(Total for Question 3 is 2 marks)" footer.
        let cfg = layout_config(vec![(
            3,
            "3 Use ruler and compasses to construct the bisector of angle ABC.\nYou must show your construction lines.",
            Some(2),
        )]);
        let q = layout_extract(&cfg, &span(3, 0, 0, Some(2))).expect("footer-only allocation is the whole allocation");
        assert_eq!(q.marks, 2);
        assert!(!q.needs_review, "{:?}", q.notes);
    }

    #[test]
    fn style_and_clarity_allocation_counts_towards_the_question_total() {
        // AEA: "(Total for Question 4 is 16 marks)" = (5) + (10) + (+S1).
        let cfg = layout_config(vec![(
            4,
            "4. (a) Use the substitution $x = \\sqrt{3} \\tan u$ to show that\n$\\int \\frac{1}{3 + x^{2}} \\mathrm{d}x = p\\arctan(px) + c$\nwhere $p$ is a real constant to be determined and $c$ is an arbitrary constant.\n(5)\n(b) Use the substitution $x = \\frac{3u + 3}{u - 3}$ to determine the exact value of $I$ where\n$I = \\int_{-3}^{1} \\frac{\\ln(3 - x)}{3 + x^{2}} \\mathrm{d}x$\ngiving your answer in simplest form.\n(10)\n(+S1)",
            Some(16),
        )]);
        let q = layout_extract(&cfg, &span(4, 0, 0, Some(16))).expect("parts plus the style mark account for the footer");
        assert_eq!(q.marks, 16);
        assert!(!q.needs_review, "{:?}", q.notes);
        assert!(q.content.contains("(+S1)"), "{}", q.content);
        // Without the style mark the parts no longer account for the footer.
        let cfg = layout_config(vec![(
            4,
            "4. (a) Use the substitution $x = \\sqrt{3} \\tan u$ to show that\n$\\int \\frac{1}{3 + x^{2}} \\mathrm{d}x = p\\arctan(px) + c$\nwhere $p$ is a real constant to be determined and $c$ is an arbitrary constant.\n(5)\n(b) Use the substitution $x = \\frac{3u + 3}{u - 3}$ to determine the exact value of $I$ where\n$I = \\int_{-3}^{1} \\frac{\\ln(3 - x)}{3 + x^{2}} \\mathrm{d}x$\ngiving your answer in simplest form.\n(10)",
            Some(16),
        )]);
        assert!(layout_extract(&cfg, &span(4, 0, 0, Some(16))).is_none());
    }

    #[test]
    fn a_wide_gap_between_two_formulas_stays_wide() {
        // "y = 2x³ + 10      x > 0": the domain is printed well apart from the
        // equation; one plain space would read "10 x > 0" as "10x > 0".
        let text = "11 A curve has equation\n$y = 2x^{3} + 10$\t$x > 0$\nFind the gradient.\n(2)";
        let cfg = layout_config(vec![(11, text, None)]);
        let q = layout_extract(&cfg, &span(11, 0, 0, None)).expect("strict layout card");
        assert!(q.content.contains("$y = 2x^{3} + 10 \\qquad x > 0$"), "{}", q.content);
        // A gap before prose (a mark, a word) is still a plain space.
        assert!(!q.content.contains('\t'), "{}", q.content);
    }

    #[test]
    fn layout_code_listing_keeps_indentation_and_blank_lines() {
        // A monospace listing is fenced verbatim by the layout engine; its
        // indentation and group separators are part of the program.
        let text = "0 9\tFigure 7 shows an assembly language program.\nFigure 7\n```\n  LDR R0, 120\nloop:\n  CMP R1, #0\n\nexit:\n  HALT\n```\n0 9 . 1 State the name of the addressing mode used.\n[1 mark]";
        let cfg = layout_config(vec![(9, text, None)]);
        let q = layout_extract(&cfg, &span(9, 0, 0, None)).expect("strict layout card");
        assert!(q.content.contains("```\n  LDR R0, 120\nloop:\n  CMP R1, #0\n\nexit:\n  HALT\n```"), "{}", q.content);
        let exported = crate::sanitize::sanitize_question_content(&q.content, 9);
        assert!(exported.contains("\n  LDR R0, 120\nloop:\n  CMP R1, #0\n\nexit:\n  HALT\n"), "{exported}");
    }

    #[test]
    fn unidentified_source_glyph_is_retained_but_never_strict() {
        // A glyph whose identity the source does not establish (no usable
        // unicode, no font table entry) is carried as U+FFFD.
        let text = "4 (a) Prove the identity $\\frac{\\sin^{3} \\theta}{\u{FFFD}} \\equiv 1$.\n[4]";
        let cfg = layout_config(vec![(4, text, Some(4))]);
        let s = span(4, 0, 0, Some(4));
        assert!(layout_extract(&cfg, &s).is_none(), "an unidentified glyph is not a strict success");
        let page = PageInput { kind: crate::pipeline::PageInputKind::TextOnly, text: String::new() };
        let (built, _, gate) = try_local_recovery(&cfg, &s, &[(0, &page)], 0, 0, &AtomicBool::new(false), None)
            .expect("the source-read content is retained for review");
        assert_eq!(gate, "unidentified_source_glyph");
        assert!(built.needs_review);
        assert!(built.content.contains("Prove the identity"), "{}", built.content);
    }

    #[test]
    fn multi_part_question_missing_part_allocations_is_not_strict() {
        // Part (b)'s "(1)" is missing: the parts no longer account for the footer.
        let cfg = layout_config(vec![(
            2,
            "2 (a) Write $3.402 \\times 10^{5}$ as an ordinary number.\n(1)\n(b) Write 0.8026 in standard form.",
            Some(2),
        )]);
        assert!(layout_extract(&cfg, &span(2, 0, 0, Some(2))).is_none());
        // No part allocations at all is not a footer-only question either.
        let cfg = layout_config(vec![(
            2,
            "2 (a) Write $3.402 \\times 10^{5}$ as an ordinary number.\n(b) Write 0.8026 in standard form.",
            Some(2),
        )]);
        assert!(layout_extract(&cfg, &span(2, 0, 0, Some(2))).is_none());
    }

    #[test]
    fn phase2_detached_margin_allocations_are_ambiguous_not_strict() {
        // Source layout: two questions' allocations are printed together in a
        // margin column at the end of the page text, and the carve for one
        // UNLABELLED question picks up both.
        let text = "Question 1 (*****)\n\
                    Show by a suitable algebraic method that the sum is 1620.\n\
                    ,\n(5)\n4 cm\nA B\nC D\nE\n(6)\n\
                    Question 2 (*****)\n\
                    Find the value of x.\n(7)\n";
        let cfg = PipelineConfig::new("m".into(), "p".into(), "Maths".into(), "Mod".into(), None);
        assert_eq!(
            transcribe_span(&cfg, &span(1, 0, 0, None), &[text], 0, 0),
            Err("ambiguous_mark_allocation"),
            "two detached allocations in one unlabelled question are not a strict accept"
        );

        // The recovery path still retains the real content, flagged.
        let page = PageInput {
            kind: crate::pipeline::PageInputKind::TextOnly,
            text: text.to_string(),
        };
        let (built, report, gate) =
            try_local_recovery(
                &cfg,
                &span(1, 0, 0, None),
                &[(0, &page)],
                0,
                0,
                &AtomicBool::new(false),
                None,
            )
                .expect("content must be retained as a flagged recovery");
        assert_eq!(gate, "ambiguous_mark_allocation");
        assert!(built.needs_review, "an ambiguous total is never a confident accept");
        assert!(built.content.contains("1620"), "{}", built.content);
        // The failed gate is on the card and stays visible to the caller.
        assert!(
            built
                .notes
                .iter()
                .any(|n| n.contains("ambiguous_mark_allocation")),
            "{:?}",
            built.notes
        );
        assert!(report.timings.iter().any(|t| t.operation == "local_recovery"));
    }

    #[test]
    fn phase2_labelled_and_single_allocations_stay_strict() {
        let cfg = PipelineConfig::new("m".into(), "p".into(), "Maths".into(), "Mod".into(), None);
        // Labelled sub-parts with equal allocations are legitimate.
        let labelled = "1 The student investigates the lamp.\n\
                        (a) State what happens to the resistance of the lamp. [2 marks]\n\
                        (b) Explain why the resistance changes. [3 marks]\n\
                        2 The next question starts here. [1 mark]\n";
        let content = transcribe_span(&cfg, &span(1, 0, 0, None), &[labelled], 0, 0)
            .expect("labelled sub-parts must stay strict");
        assert_eq!(crate::validate::sum_inline_marks(&content), 5, "{content}");

        // A single detached allocation is unambiguous.
        let single = "Question 1 (*****)\n\
                      Show that the area of the shaded region is 4 cm2.\n(5)\n\
                      Question 2 (*****)\n\
                      Find the value of x.\n(6)\n";
        let content = transcribe_span(&cfg, &span(1, 0, 0, None), &[single], 0, 0)
            .expect("one detached allocation stays strict");
        assert_eq!(crate::validate::sum_inline_marks(&content), 5, "{content}");
    }

    #[test]
    fn phase2_heading_evidence_matches_doc_map_layouts() {
        // Madas-style difficulty rating: "Question 1 (*****)" — too short for
        // the usual tail-length guard, but the stem follows on the next line.
        assert!(heading_line_matches("Question 1 (*****)", &heading_line_re(1)));
        assert_eq!(heading_number_on_line("Question 1 (*****)"), Some(1));
        assert!(heading_line_matches(
            "Question 12 (*****)",
            &heading_line_re(12)
        ));
        // AQA isotope heading: "box 3 1 27Mg 12 can decay …".
        assert!(heading_line_matches(
            "box 3 1 27Mg 12 can decay by beta minus emission to one of two possible states",
            &heading_line_re(31)
        ));
        // Nuclide DATA rows are table data, never headings (they used to
        // truncate the question that owned the table).
        assert_eq!(heading_number_on_line("20 Ne 10 19.99244"), None);
        assert_eq!(heading_number_on_line("17O 8 16.99913"), None);
        assert_eq!(heading_number_on_line("3He 2 3.01603"), None);
        // Ordinary headings still match.
        assert!(heading_line_matches(
            "1. The transformation P is an enlargement, centre the origin",
            &heading_line_re(1)
        ));
        assert_eq!(heading_number_on_line("7 The next question starts here."), Some(7));
    }

    #[test]
    fn phase2_carve_keeps_content_after_a_nuclide_data_row() {
        let text = "0 6 box . 1 Explain, in terms of binding energy, why energy can be released.\n[2 marks]\n\
                    Table 2 gives data for these nuclei.\nNucleus Mass / u\n\
                    3He 2 3.01603\n\
                    17O 8 16.99913\n\
                    20 Ne 10 19.99244\n\
                    Calculate, in J, the energy released when this reaction occurs.\n[2 marks]\n\
                    7 In a resistor of resistance R, a steady current I dissipates a power P.\n[1 mark]\n";
        let carved = carve_span(&span(6, 0, 0, Some(4)), &[text], false);
        assert!(carved.found_start && carved.found_end, "{carved:?}");
        assert!(
            carved.text.contains("Calculate, in J"),
            "content after the nuclide table must be kept: {}",
            carved.text
        );
        assert!(
            !carved.text.contains("In a resistor"),
            "the next real question still ends the carve: {}",
            carved.text
        );
    }

    #[test]
    fn phase2_madas_shaped_question_carves_and_passes_gates() {
        let text = "Question 1 (*****)\n\
                    The figure above is constructed as follows.\n\
                    A semicircle with diameter AB of 4 cm is first drawn.\n\
                    Show that the area of the shaded region is 4 cm2.\n\
                    (5)\n\n\
                    Question 2 (*****)\n\
                    Show by a suitable algebraic method that the sum is 1620.\n\
                    (6)\n";
        let cfg = PipelineConfig::new("m".into(), "p".into(), "Maths".into(), "Mod".into(), None);
        let content = transcribe_span(&cfg, &span(1, 0, 0, None), &[text], 0, 0)
            .expect("madas-shaped Q1 must carve and pass the gates");
        assert_eq!(validate::sum_inline_marks(&content), 5, "{content}");
        assert!(
            content.contains("area of the shaded region"),
            "{content}"
        );
    }

    #[test]
    fn physics24_structured_subparts_keep_marks() {
        let cfg = PipelineConfig::new("m".into(), "p".into(), "Physics".into(), "Mod".into(), None);
        for (number, texts, marks, figures) in [
            (1, vec![include_str!("../fixtures/physics24_text/page_2.txt"), include_str!("../fixtures/physics24_text/page_3.txt")], 9, 0),
            (7, vec![include_str!("../fixtures/physics24_text/page_19.txt"), include_str!("../fixtures/physics24_text/page_20.txt"), include_str!("../fixtures/physics24_text/page_21.txt")], 10, 1),
        ] {
            let content = transcribe_span(&cfg, &span(number, 0, texts.len()-1, Some(marks)), &texts, texts.len()-1, figures).unwrap();
            let content = crate::marker_client::clean_marker_markdown(&content);
            assert_eq!(validate::sum_inline_marks(&content), marks, "Q{number}: {content}");
            for letter in ['a','b','c','d'] { assert!(content.contains(&format!("({letter})")), "Q{number}: {content}"); }
        }
    }
    #[test]
    fn physics24_frequency_choices_keep_both_columns() {
        let cfg = PipelineConfig::new("m".into(), "p".into(), "Physics".into(), "Mod".into(), None);
        let content = transcribe_span(&cfg, &span(14, 0, 0, Some(1)), &[include_str!("../fixtures/physics24_text/page_25.txt")], 0, 3).unwrap();
        let content = crate::marker_client::clean_marker_markdown(&content);
        let content = crate::sanitize::sanitize_question_content(&content, 14);
        // Each option row keeps both printed columns: the frequency and the
        // phase difference (no invented labels).
        let options: Vec<&str> = content.lines().filter(|l| l.starts_with("- [MCQ:")).collect();
        assert_eq!(options.len(), 4, "{content}");
        for (opt, (freq, phase)) in options.iter().zip([("50", r"0.30\pi"), ("50", r"0.15\pi"), ("25", r"0.30\pi"), ("25", r"0.15\pi")]) {
            assert!(opt.contains(freq) && opt.contains(phase), "{opt}");
        }
    }
    #[test]
    fn physics24_section_b_cards_are_strict_with_source_maths() {
        // Production seam: the layout map's bodies through the deterministic
        // gates and the card assembler, on the real paper. The expressions
        // are the ones printed in the source (stacked fractions, radicals,
        // powers and nuclide prescripts rebuilt from glyph geometry).
        let _guard = crate::pdf_render::pdfium_test_lock();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../past papers for mergemark/physics '24.pdf");
        let Some(pages) = path.exists().then(|| crate::pdf_render::load_layout_pages(&path)).flatten() else {
            eprintln!("[P24] fixture or PDFium unavailable");
            return;
        };
        let map = crate::layout::build_layout_map(&pages);
        let mut cfg = PipelineConfig::new("m".into(), "p".into(), "Physics".into(), "Mod".into(), None);
        let bodies: std::collections::HashMap<u32, crate::layout::QuestionBody> =
            map.questions.iter().map(|q| (q.number, crate::layout::question_body(&pages, q))).collect();
        cfg.layout_questions = Some(std::sync::Arc::new(bodies.clone()));
        let inputs: Vec<PageInput> = pages.iter().map(|lp| PageInput { kind: crate::pipeline::PageInputKind::TextOnly, text: lp.text() }).collect();
        let expected: &[(u32, &[&str])] = &[
            (14, &[r"0.30\pi", r"0.15\pi"]),
            (16, &[r"\frac{2\pi m_e}{Be}", r"\frac{v}{\pi r}", r"\frac{Be}{2\pi m_e}"]),
            (18, &[r"{}^{235}_{92}\mathrm{U}", r"{}^{146}_{57}\text{La}"]),
            (21, &[r"\frac{R}{\sqrt[3]{2}}", r"\frac{R}{\sqrt[3]{16}}", r"\frac{R}{2}", r"\frac{\sqrt{2}R}{8}"]),
            (22, &[r"10^{3}", r"10^{4}", r"10^{5}", r"10^{6}"]),
            (31, &[r"^{238}_{92}"]),
        ];
        for &(number, exprs) in expected {
            let q = map.questions.iter().find(|q| q.number == number).expect("mapped");
            let s = QuestionSpan {
                number,
                start_page: q.start_page,
                end_page: q.end_page,
                start_y_frac: None,
                end_y_frac: None,
                expected_marks: bodies[&number].footer_marks,
                reliable_pages: (q.start_page..=q.end_page).collect(),
                ambiguous_pages: vec![],
            };
            let span_pages: Vec<(usize, &PageInput)> = (q.start_page..=q.end_page).map(|p| (p, &inputs[p])).collect();
            let (built, _) = try_deterministic_extraction(&cfg, &s, &span_pages, 0, pages.len() - 1, &AtomicBool::new(false), None)
                .unwrap_or_else(|| panic!("Q{number} must be a strict card: {}", bodies[&number].text));
            let card = crate::pipeline::build_question_from_seam(built.content.clone(), Some(1), &s, &cfg, 0)
                .unwrap_or_else(|| panic!("Q{number} rejected at the production seam: {}", built.content));
            assert_eq!(card.marks, 1);
            let letters: Vec<&str> = card.content.lines().filter_map(|l| l.strip_prefix("- [MCQ:")).map(|l| &l[..1]).collect();
            assert_eq!(letters, ["A", "B", "C", "D"], "Q{number}: {}", card.content);
            for e in exprs {
                assert!(card.content.contains(e), "Q{number} lacks {e}: {}", card.content);
            }
        }
    }
    #[test]
    fn physics24_remaining_tier0_spans() {
        let cfg = PipelineConfig::new("m".into(), "p".into(), "Physics".into(), "Mod".into(), None);
        for (number, pages, marks) in [
            (2, vec![include_str!("../fixtures/physics24_text/page_4.txt"), include_str!("../fixtures/physics24_text/page_5.txt"), include_str!("../fixtures/physics24_text/page_6.txt"), include_str!("../fixtures/physics24_text/page_7.txt")], 9),
            (4, vec![include_str!("../fixtures/physics24_text/page_10.txt"), include_str!("../fixtures/physics24_text/page_11.txt")], 5),
            (23, vec![include_str!("../fixtures/physics24_text/page_30.txt")], 1),
            (24, vec![include_str!("../fixtures/physics24_text/page_30.txt")], 1),
            (26, vec![include_str!("../fixtures/physics24_text/page_32.txt")], 1),
            (29, vec![include_str!("../fixtures/physics24_text/page_33.txt")], 1),
            (30, vec![include_str!("../fixtures/physics24_text/page_34.txt")], 1),
        ] {
            let span = span(number, 0, pages.len()-1, Some(marks));
            let content = transcribe_span(&cfg, &span, &pages, pages.len()-1, 0)
                .unwrap_or_else(|e| panic!("Q{number}: {e}\n{:?}", normalize(&carve_span(&span, &pages, true).text)));
            let content = crate::marker_client::clean_marker_markdown(&content);
            let content = crate::sanitize::sanitize_question_content(&content, number);
            assert!(validate::card_structure_errors(&content, number).is_empty(), "Q{number}: {content}");
            assert!(crate::pipeline::build_question_from_seam(content.clone(), Some(marks), &span, &cfg, 0).is_some(), "Q{number}: {content}");
        }
        // Q20's bracket pieces are private-use Symbol glyphs in the text
        // layer: without glyph geometry the card is refused, never guessed.
        assert_eq!(
            transcribe_span(&cfg, &span(20, 0, 0, Some(1)), &[include_str!("../fixtures/physics24_text/page_28.txt")], 0, 0),
            Err("unmapped_symbol_font_glyphs")
        );
    }

    #[test]
    fn physics24_numeric_heading_preserves_energy() {
        let raw = "1 9 4.8 kW h of heat energy is supplied to a heater. What is the power?\n[1 mark]\nA 1 kW\nB 2 kW\nC 3 kW\nD 4 kW\n2 0 The next question starts here.";
        let cfg = PipelineConfig::new("m".into(), "p".into(), "Physics".into(), "Mod".into(), None);
        let content = transcribe_span(&cfg, &span(19, 0, 0, Some(1)), &[raw], 0, 0).unwrap();
        let content = crate::marker_client::clean_marker_markdown(&content);
        assert!(content.contains("4.8"), "{content}");
        assert!(!content.contains("next question"), "{content}");
        assert!(!heading_line_matches("19.1 Calculate the energy transferred.", &heading_line_re(19)));
        assert_eq!(heading_number_on_line("1 9 4.8 kW h of heat energy is supplied."), Some(19));
    }

    #[test]
    fn physics24_shared_page_31_32() {
        let raw = "box 3 1 Uranium-238 absorbs a neutron in the first stage in a series of nuclear reactions that end\nin a nucleus Z.\n238Un X 92 + →\nXY v + + e → β−\nYZ v + + e → β−\nHow many neutrons does Z have?\n[1 mark]\nA 144\nB 145\nC 149\nD 237\n3 2 A rock sample is found to contain the stable isotope lead-207. When it was formed, the\nrock contained uranium-235 but did not contain any lead-207.\nHow long ago was the rock formed?\n[1 mark]\nA 0.23 billion years\nB 0.31 billion years\nC 1.4 billion years\nD 2.0 billion years\nEND OF QUESTIONS";
        let cfg = PipelineConfig::new("m".into(), "p".into(), "Physics".into(), "Mod".into(), None);
        for number in [31, 32] {
            let content = transcribe_span(&cfg, &span(number, 0, 0, Some(1)), &[raw], 0, 0).unwrap();
            let content = crate::sanitize::sanitize_question_content(&content, number);
            let errors = validate::card_structure_errors(&content, number);
            assert!(errors.is_empty(), "Q{number}: {errors:?}\n{content}");
            assert_eq!(content.contains("rock sample"), number == 32, "{content}");
            assert_eq!(content.contains("nucleus Z"), number == 31, "{content}");
        }
    }
    use super::*;

    fn span(number: u32, start: usize, end: usize, marks: Option<u32>) -> QuestionSpan {
        QuestionSpan {
            number,
            start_page: start,
            end_page: end,
            start_y_frac: None,
            end_y_frac: None,
            expected_marks: marks,
            reliable_pages: vec![],
            ambiguous_pages: vec![],
        }
    }

    const LONG_INSTRUCTION: &str =
        "Explain, with reference to the physical principles outlined in the data booklet above, why the measured value differs from the theoretical prediction";

    #[test]
    fn carve_mid_page_start_under_previous_footer() {
        let page = format!(
            "1 State the principle of moments. {}\n(Total for Question 1 is 4 marks)\n3. {}\n(Total for Question 3 is 3 marks)",
            "a".repeat(80),
            LONG_INSTRUCTION
        );
        let s = span(3, 0, 0, Some(3));
        let carved = carve_span(&s, &[&page], false);
        assert!(carved.found_start && carved.found_end);
        assert_eq!(carved.footer_marks, Some(3));
        assert!(carved.text.starts_with("3."));
        assert!(!carved.text.contains("moments"));
    }

    #[test]
    fn carve_shared_page_stops_at_next_heading() {
        let page = format!(
            "2. {}\n[2 marks]\n3. {}\n[3 marks]\n4. Next question body.",
            "b".repeat(90),
            LONG_INSTRUCTION
        );
        let s = span(3, 0, 0, None);
        let carved = carve_span(&s, &[&page], false);
        assert!(carved.found_start && carved.found_end);
        assert!(carved.text.contains(LONG_INSTRUCTION));
        assert!(!carved.text.contains("Next question"));
    }

    #[test]
    fn carve_multipage_span_takes_interior_whole() {
        let p1 = format!("2. Old question.\n(Total for Question 2 is 2 marks)\n3. {}", LONG_INSTRUCTION);
        let p2 = "c ".repeat(60);
        let p3 = format!("{}\n(Total for Question 3 is 5 marks)", "continuation of the derivation follows here with more words");
        let s = span(3, 0, 2, Some(5));
        let carved = carve_span(&s, &[&p1, &p2, &p3], false);
        assert!(carved.found_start && carved.found_end);
        assert_eq!(carved.footer_marks, Some(5));
        assert!(carved.text.contains(LONG_INSTRUCTION));
        assert!(carved.text.contains("continuation"));
    }

    #[test]
    fn carve_missing_footer_at_paper_end_is_legitimate() {
        let page = format!(
            "5. {}\n[4 marks]",
            "Data recorded during the investigation was checked carefully before any analysis. "
                .repeat(2)
        );
        let s = span(5, 0, 0, Some(4));
        let carved = carve_span(&s, &[&page], true);
        assert!(carved.found_start && carved.found_end);
        assert!(carved.footer_marks.is_none());

        // doc_map contract: end_y None ⇒ the question owns its end page.
        // A mid-page end_y without a nearby boundary must decline instead.
        let mut shared = s.clone();
        shared.end_y_frac = Some(0.3);
        let long_page = format!(
            "5. Each reading in the table below was recorded during the full investigation\n{}\n[4 marks]",
            std::iter::repeat_n(
                "each reading was then repeated twice more for overall reliability".to_string(),
                20
            )
            .collect::<Vec<_>>()
            .join("\n")
        );
        let carved_shared = carve_span(&shared, &[&long_page], false);
        assert!(carved_shared.found_start && !carved_shared.found_end);
    }

    #[test]
    fn carve_strips_running_headers_and_page_numbers() {
        let page = format!(
            "AQA\n7408/1\nDo not write outside the box\n4\n5. {}\n[3 marks]",
            LONG_INSTRUCTION
        );
        let s = span(5, 0, 0, Some(3));
        let carved = carve_span(&s, &[&page], true);
        assert!(!carved.text.contains("7408"));
        assert!(!carved.text.contains("outside"));
        assert!(carved.text.starts_with("5."));
    }

    #[test]
    fn normalizer_strips_answer_lines_with_units() {
        let raw = "The battery has emf 9.0 V.\naverage emf = _________ V\ndiscuss how the internal resistance affects the terminal potential difference under load conditions";
        let norm = normalize(raw);
        assert!(!norm.content.contains("___"));
        assert!(norm.content.contains("internal resistance"));
        assert_eq!(norm.answer_lines_seen, 1);
    }

    #[test]
    fn normalizer_joins_wrapped_sentences_but_not_subparts_or_equations() {
        let raw = format!(
            "{}\nthe circuit shown is incomplete\n(a) State the law.\n\n(b) v = u + at\nhence derive the expression fully with all working shown",
            "Complete the following analysis carefully and thoroughly"
        );
        let norm = normalize(&raw);
        assert!(norm.content.contains("thoroughly the circuit shown is incomplete"), "wrapped sentence joined: {:?}", norm.content);
        assert!(norm.content.contains("(a) State the law."), "subpart kept: {:?}", norm.content);
        assert!(!norm.content.contains("at hence"), "equation not glued to prose: {:?}", norm.content);
    }

    #[test]
    fn normalizer_formats_marks_tags() {
        let raw = "(a) Solve the equation completely showing every step of your working in detail. [3 marks]\n(b) Verify the result independently. [2 marks]";
        let norm = normalize(raw);
        assert_eq!(norm.marks_total, 5);
        assert!(norm.content.contains("**[3 marks]**"));
        assert!(norm.content.contains("**[2 marks]**"));
    }

    #[test]
    fn normalizer_unicode_scientific_and_nuclear() {
        let norm = normalize("The charge is 1.6×10⁻¹⁹ C and the nuclide ²²⁶Ra decays by alpha emission to radon.");
        assert!(norm.content.contains("\\times10^{-19}"), "{:?}", norm.content);
        assert!(norm.content.contains("$^{226}\\text{Ra}$"), "{}", norm.content);
    }

    #[test]
    fn normalizer_mcq_options_kept_verbatim_one_per_line() {
        let raw = "Which statement is correct about the momentum of the isolated system described?\nA Momentum increases.\nB Momentum decreases.\nC Momentum is conserved.\nD Momentum is zero.";
        let norm = normalize(raw);
        for opt in ["A Momentum", "B Momentum", "C Momentum", "D Momentum"] {
            assert!(norm.content.contains(opt), "{:?}", norm.content);
        }
        assert!(!norm.content.contains("A Momentum increases. B"));
    }

    // ── Hardening: fractions, nuclides, hyphens, MCQ, DOM ──────────────────

    #[test]
    fn fraction_bar_becomes_latex_frac_not_runon_prose() {
        let raw = "The ratio of the two forces is calculated as follows in the working.\nelectrostatic force\n─────────────────\ngravitational force\n[3 marks]";
        let norm = normalize(raw);
        assert!(
            norm.content.contains("\\frac{\\text{electrostatic force}}{\\text{gravitational force}}"),
            "prose fraction must become \\frac: {:?}", norm.content
        );
        assert!(
            !norm.content.contains("force gravitational"),
            "numerator and denominator must not weld: {:?}", norm.content
        );
    }

    #[test]
    fn numeric_fraction_bar_becomes_bare_frac() {
        let raw = "Evaluate the expression below showing every stage of the working clearly.\nx + 1\n───\n2\n[2 marks]";
        let norm = normalize(raw);
        assert!(norm.content.contains("\\frac{x + 1}{2}"), "{:?}", norm.content);
    }

    #[test]
    fn answer_blank_bar_is_still_stripped() {
        let raw = "Write the equation for the moment of the couple about the pivot point.\n_________________\n[2 marks]";
        let norm = normalize(raw);
        assert!(!norm.content.contains("\\frac"), "{:?}", norm.content);
        assert!(!norm.content.contains("___"), "{:?}", norm.content);
    }

    #[test]
    fn stacked_nuclide_survives_furniture_and_gets_prescripts() {
        let page = "9. The uranium nucleus decays by emitting an alpha particle as shown below.\n238\n92 U →\n234\n90 Th +\n4\n2 He\n(Total for Question 9 is 2 marks)";
        let s = span(9, 0, 0, Some(2));
        let carved = carve_span(&s, &[&page], false);
        assert!(carved.found_start && carved.found_end);
        let norm = normalize(&carved.text);
        assert!(
            norm.content.contains("^{238}_{92}\\text{U}"),
            "stacked mass/atomic pair must become prescripts: {:?}", norm.content
        );
        assert!(
            norm.content.contains("^{234}_{90}\\text{Th}"),
            "decay chain members preserved: {:?}", norm.content
        );
        assert!(
            norm.content.contains("^ {4}".replace(' ', "").as_str())
                && norm.content.contains("_ {2}".replace(' ', "").as_str()),
            "alpha particle preserved: {:?}",
            norm.content
        );
        assert!(validate::math_delimiter_balance_errors(&norm.content).is_empty());
    }

    #[test]
    fn hyphenated_words_keep_plain_hyphens() {
        let raw = "The lead-207 isotope has a very long half-life indeed compared with other materials.";
        let norm = normalize(raw);
        assert!(norm.content.contains("lead-207"), "{:?}", norm.content);
        assert!(norm.content.contains("half-life"), "{:?}", norm.content);
        assert!(
            !norm.content.contains("$-") && !norm.content.contains("-$"),
            "grammatical hyphen must not become a KaTeX minus: {:?}", norm.content
        );
    }

    #[test]
    fn math_minus_between_variables_still_wraps() {
        let norm = normalize("Simplify the expression given here so that only one term remains at the end. a-b");
        assert!(norm.content.contains("$a-b$"), "{:?}", norm.content);
    }

    #[test]
    fn flattened_negative_exponent_recovered_contiguously_with_unit() {
        let raw = "The activity of the sample is 6.3 × 10-14 mol per second after correction.";
        let norm = normalize(raw);
        assert!(
            !norm.content.contains("10-14"),
            "flattened exponent must be recovered: {:?}", norm.content
        );
        let block_start = norm.content.find("$").expect("math block present");
        let block_end = norm.content[block_start + 1..].find("$").unwrap() + block_start + 1;
        let block = &norm.content[block_start..=block_end];
        assert!(block.contains("10^{-14}"), "exponent inside block: {:?}", block);
        assert!(block.contains("\\text{mol}"), "unit bound into same block: {:?}", block);
    }

    #[test]
    fn mcq_option_labels_stay_outside_math_bodies() {
        let raw = "Which row gives the correct arrangement for the circuit described above?\n[1 mark]\nA r < 20\nB r = 20\nC r > 20\nD r = 40";
        let norm = normalize(raw);
        assert_eq!(norm.marks_total, 1, "[1 mark] assigned to metadata");
        assert!(norm.content.contains("**[1 mark]**"), "{:?}", norm.content);
        for line in ["A $r < 20$", "B $r = 20$", "C $r > 20$"] {
            assert!(norm.content.contains(line), "{:?}", norm.content);
        }
        assert!(
            !norm.content.contains("$A"),
            "option letter must not fuse into math cluster: {:?}", norm.content
        );
        assert!(
            norm.content.contains("B $r = 20$\n\nC $r > 20$"),
            "options render as separate blocks: {:?}", norm.content
        );
    }

    #[test]
    fn ghost_margin_furniture_is_dropped_mid_question() {
        let raw = "(a) The student measures the current carefully and records every reading taken.\n(ii)\nDo not write outside the box\n(b) Explain why the readings must be repeated at least once more. [4 marks]";
        let norm = normalize(raw);
        assert!(!norm.content.contains("(ii)"), "{:?}", norm.content);
        assert!(!norm.content.contains("outside"), "{:?}", norm.content);
        assert!(norm.content.contains("(a)"), "{:?}", norm.content);
        assert!(norm.content.contains("(b) Explain"), "{:?}", norm.content);
    }

    #[test]
    fn sentence_continues_after_parenthesis() {
        let raw = "The student repeats the whole experiment twice (see appendix 2)\nwhich shows the same trend for both materials used throughout.";
        let norm = normalize(raw);
        assert!(
            norm.content.contains("(see appendix 2) which shows"),
            "sentence welded across ')': {:?}", norm.content
        );
    }

    #[test]
    fn inline_media_gets_its_own_block() {
        let out = isolate_block_media("The circuit shown ![Diagram](x.png) clearly.");
        assert!(
            out.contains("The circuit shown\n\n![Diagram](x.png)\n\nclearly."),
            "text / image / text separated into blocks: {:?}",
            out
        );

        let norm = normalize("The graph shows [DIAGRAM_PLACEHOLDER] for the run.");
        assert!(
            norm.content.lines().any(|l| l.trim() == "[DIAGRAM_PLACEHOLDER]"),
            "placeholder isolated on its own line: {:?}",
            norm.content
        );
    }

    fn gate_refuses(body: &str, marks: Option<u32>, expected_reason: &str) {
        let page = format!("6. {}\n(Total for Question 6 is {} marks)", body, marks.unwrap_or(0));
        let s = span(6, 0, 0, marks);
        let carved = carve_span(&s, &[&page], false);
        let mut norm = normalize(&carved.text);
        let cfg = PipelineConfig::new("m".into(), "p".into(), "Physics".into(), "Mod".into(), None);
        let err = check_gates(&carved, &mut norm, &s, &cfg, 0).unwrap_err();
        assert_eq!(err, expected_reason);
    }

    #[test]
    fn gate_refuses_marks_mismatch() {
        gate_refuses(
            "The student measures the extension of the spring under increasing load and records every reading carefully before analysing the results in detail. [5 marks]",
            Some(3),
            "marks_checksum_mismatch",
        );
    }

    #[test]
    fn gate_refuses_subpart_gap() {
        let body = "(a) First part of the question asks for a definition stated precisely here.\n(c) Third part skips the letter b entirely which must refuse. [2 marks]";
        let page = format!("6. {}\n(Total for Question 6 is 2 marks)", body);
        let s = span(6, 0, 0, Some(2));
        let carved = carve_span(&s, &[&page], false);
        let mut norm = normalize(&carved.text);
        let cfg = PipelineConfig::new("m".into(), "p".into(), "Physics".into(), "Mod".into(), None);
        assert_eq!(
            check_gates(&carved, &mut norm, &s, &cfg, 0).unwrap_err(),
            "subpart_gap"
        );
    }

    #[test]
    fn gate_refuses_leftover_blanks() {
        gate_refuses(
            "Calculate the current drawn from the supply when both resistors conduct. ____________ [2 marks]",
            Some(2),
            "leftover_answer_line",
        );
    }

    #[test]
    fn gate_accepts_illustrative_figure_with_supply() {
        // Illustrative figure AND a detected crop is available: Tier 0 must
        // accept for free and insert a placeholder for the caller to splice.
        let body = "(a) Figure 3 shows the circuit used to investigate the resistance of a wire as its length changes in a laboratory experiment today. [4 marks]\n(b) Explain one safety precaution taken when performing this investigation with care. [2 marks]";
        let page = format!("6. {}\n(Total for Question 6 is 6 marks)", body);
        let s = span(6, 0, 0, Some(6));
        let carved = carve_span(&s, &[&page], false);
        let mut norm = normalize(&carved.text);
        let cfg = PipelineConfig::new("m".into(), "p".into(), "Physics".into(), "Mod".into(), None);
        let gate = check_gates(&carved, &mut norm, &s, &cfg, 1);
        assert!(gate.is_ok(), "illustrative figure must pass with supply: {:?}", gate.err());
        assert!(
            norm.content.contains("[DIAGRAM_PLACEHOLDER]"),
            "placeholder not inserted:\n{}",
            norm.content
        );
    }

    #[test]
    fn physics24_unmapped_fraction_glyphs_require_fallback() {
        let s = span(20, 0, 0, Some(1));
        let cfg = PipelineConfig::new("m".into(), "p".into(), "Physics".into(), "Mod".into(), None);
        let page = "20. What is the nuclear radius of an element with nucleon number y?\n[1 mark]\nA x r y\n\u{f8eb}\u{f8f6}\nB y r x\nC 3 x r y\nD y 3 r x";
        assert_eq!(transcribe_span(&cfg, &s, &[page], 0, 0).unwrap_err(), "unmapped_symbol_font_glyphs");
    }

    #[test]
    fn gate_accepts_graph_reading_with_supply() {
        for reference in ["Figure 3 shows the graph. ", ""] {
            let page = format!("6. {reference}Use the graph to determine the acceleration of the trolley during the first ten seconds of its journey. [2 marks]\n(Total for Question 6 is 2 marks)");
            let s = span(6, 0, 0, Some(2));
            let cfg = PipelineConfig::new("m".into(), "p".into(), "Physics".into(), "Mod".into(), None);
            assert!(transcribe_span(&cfg, &s, &[&page], 0, 1).is_ok());
            assert!(transcribe_span(&cfg, &s, &[&page], 0, 0).unwrap().contains("[DIAGRAM_PLACEHOLDER]"));
        }
    }

    #[test]
    fn gate_accepts_span_even_with_allowed_topics() {
        // Production imports ALWAYS carry the module taxonomy in
        // allowed_topics (commands.rs loads it unconditionally). Tier 0 must
        // still fire — untagged cards are classified later by the single
        // local `classify_topics_deferred` matcher.
        let page = format!("6. {}\n[2 marks]", "Plain question body with plenty of words to satisfy the minimum length requirement easily");
        let s = span(6, 0, 0, Some(2));
        let carved = carve_span(&s, &[&page], false);
        let mut norm = normalize(&carved.text);
        let mut cfg = PipelineConfig::new("m".into(), "p".into(), "Physics".into(), "Mod".into(), None);
        cfg.allowed_topics = vec!["Mechanics".into()];
        assert!(
            check_gates(&carved, &mut norm, &s, &cfg, 0).is_ok(),
            "populated taxonomy must not disable Tier 0"
        );
    }

    #[test]
    fn gate_refuses_min_length() {
        gate_refuses("Too short. [1 mark]", Some(1), "too_short");
    }

    #[test]
    fn happy_path_passes_all_gates() {
        let body = format!(
            "(a) {} Show that the acceleration is uniform. [4 marks]",
            "Derive the expression for the acceleration of the trolley from the definitions of velocity and acceleration,"
        );
        let page = format!("7. {}\n(Total for Question 7 is 4 marks)", body);
        let s = span(7, 0, 0, Some(4));
        let carved = carve_span(&s, &[&page], false);
        assert!(carved.found_start && carved.found_end);
        assert_eq!(carved.footer_marks, Some(4));
        let mut norm = normalize(&carved.text);
        let cfg = PipelineConfig::new("m".into(), "p".into(), "Physics".into(), "Mod".into(), None);
        assert!(check_gates(&carved, &mut norm, &s, &cfg, 0).is_ok(), "gates refused: {:?}", check_gates(&carved, &mut norm, &s, &cfg, 0));
        assert!(validate::math_delimiter_balance_errors(&norm.content).is_empty());
    }

    #[test]
    fn seam_accepts_representative_transcription() {
        let body = "(a) State Newton's second law of motion and explain each term in the equation you give with care. [3 marks]\n(b) Apply the law to derive the acceleration of the two-block system described above in full. [3 marks]";
        let page = format!("7. {}\n(Total for Question 7 is 6 marks)", body);
        let s = span(7, 0, 0, Some(6));
        let carved = carve_span(&s, &[&page], false);
        let mut norm = normalize(&carved.text);
        let cfg = PipelineConfig::new("m".into(), "p".into(), "Physics".into(), "Mod".into(), None);
        let gate = check_gates(&carved, &mut norm, &s, &cfg, 0);
        assert!(gate.is_ok(), "gates refused: {:?}", gate);
        let built = crate::pipeline::build_question_from_seam(
            norm.content.clone(),
            Some(norm.marks_total),
            &s,
            &cfg,
            0,
        );
        assert!(built.is_some(), "seam rejected the Tier-0 transcription");
        let built = built.unwrap();
        assert_eq!(built.question_number, 7);
        assert_eq!(built.marks, 6);
        assert!(built.content.contains("(a)"));
    }

    /// Fixture calibration harness: prints per-paper Tier-0 accept rates and
    /// gate-refusal reasons for physics '17–'24 and test_papers/*_qp.pdf.
    /// Manual-inspection output only — no assertions.
    
    #[test]
    fn physics24_question_5_tier0_structure() {
        let p12 = include_str!("../fixtures/physics24_text/page_12.txt");
        let p13 = include_str!("../fixtures/physics24_text/page_13.txt");
        let p14 = include_str!("../fixtures/physics24_text/page_14.txt");
        
        let raw = format!("{p12}\n{p13}\n{p14}");
        let norm = normalize(&raw);
        let cleaned = crate::marker_client::clean_marker_markdown(&norm.content);
        let sanitized = crate::sanitize::sanitize_question_content(&cleaned, 5);
        let errs = crate::validate::card_structure_errors(&sanitized, 5);
        assert!(errs.is_empty(), "Q5 has errors: {errs:?}\n{sanitized}");
        assert!(sanitized.contains("(a)"), "{sanitized}");
        assert!(sanitized.contains("(b)"), "{sanitized}");
        assert!(sanitized.contains("(c)"), "{sanitized}");
    }

#[test]
    fn diagnostic_tier0_accept_rate_on_fixtures() {
        let _guard = crate::pdf_render::pdfium_test_lock();
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
        let mut fixtures: Vec<std::path::PathBuf> = Vec::new();
        for year in 17..=24 {
            let p = manifest.join(format!("../physics '{}.pdf", year));
            if p.exists() {
                fixtures.push(p);
            }
        }
        if let Ok(entries) = std::fs::read_dir(manifest.join("../test_papers")) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.ends_with("_qp.pdf") {
                    fixtures.push(e.path());
                }
            }
        }
        if fixtures.is_empty() {
            eprintln!("[TIER0] no fixtures found");
            return;
        }
        let cfg = PipelineConfig::new("m".into(), "p".into(), "Physics".into(), "Mod".into(), None);
        for fixture in fixtures {
            let texts = match crate::pdf_render::pdf_page_texts(&fixture) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[TIER0:{}] pdfium unavailable: {}", fixture.display(), e);
                    continue;
                }
            };
            let scan = crate::doc_map::scan_text_layer(&texts);
            let figures = match crate::pdf_render::detect_pdf_figures(&fixture) {
                Ok(figures) => figures,
                Err(error) => {
                    eprintln!("[TIER0:{}] figure detection failed: {}", fixture.display(), error);
                    continue;
                }
            };
            let pages: Vec<crate::pipeline::PageInput> = texts.iter().map(|text| crate::pipeline::PageInput {
                kind: crate::pipeline::PageInputKind::TextOnly,
                text: text.clone(),
            }).collect();
            let map = crate::doc_map::build_hybrid_map_with_scan(&texts, &[], texts.len(), &scan);
            let paper_last = map.spans.iter().map(|s| s.end_page).max().unwrap_or(0);
            let mut accepted = 0usize;
            let mut reasons: std::collections::BTreeMap<&'static str, usize> = Default::default();
            for sp in &map.spans {
                let span_pages: Vec<_> = (sp.start_page..=sp.end_page)
                    .filter_map(|pi| pages.get(pi).map(|page| (pi, page))).collect();
                let text = span_pages.iter().map(|(_, p)| p.text.as_str()).collect::<Vec<_>>().join("\n");
                let refs = crate::pipeline::figure_reference_numbers(&text);
                let supplied = crate::pipeline::available_span_figures(sp, &span_pages, &figures, &refs);
                match tier0_diagnose_span(&cfg, sp, &texts, paper_last, supplied) {
                    Ok(_) => accepted += 1,
                    Err(reason) => {
                        *reasons.entry(reason).or_insert(0) += 1;
                        eprintln!(
                            "[TIER0:{}] Q{} pages {}..{} -> {}",
                            fixture.file_name().unwrap().to_string_lossy(),
                            sp.number,
                            sp.start_page + 1,
                            sp.end_page + 1,
                            reason
                        );
                    }
                }
            }
            eprintln!(
                "[TIER0:{}] {}/{} spans accepted ({:.0}%), refusals: {:?}",
                fixture.file_name().unwrap().to_string_lossy(),
                accepted,
                map.spans.len(),
                if map.spans.is_empty() { 0.0 } else { 100.0 * accepted as f32 / map.spans.len() as f32 },
                reasons
            );
        }
    }
}
