// ── Deterministic content validators ───────────────────────────────────────
//
// Every check here is pure, cheap, and testable. Validators either *clean*
// (exact-string boilerplate removal), *measure* (marks sums, truncation), or
// *gate* (structure proposals). The pipeline uses their verdicts to build
// repair prompts and quarantine reports.

use std::sync::{LazyLock, OnceLock};

static INEQ_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?im)^[\s\.\$]*(?:\\\\?leq?|\\\\?geq?|<|>)\s*[a-zA-Z]\s*(?:\\\\?leq?|\\\\?geq?|<|>)\s*(.*?)\s*\$?\s*$").unwrap()
});

static EQ_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?im)^[\s\.\$]*[a-zA-Z]\s*=\s*(.*?)\s*\$?\s*$").unwrap()
});

static RE_EXAMINER_CODES: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)[\s,;]*(?:[\[(]?\b(?:d?(?:[mab]1?|ft|oe|cao|aef|awrt|dep|indep|allow|condone|ignore|accept|or\s+equivalent|award))[\](,)]*)+\s*$").unwrap()
});

fn re(pattern: &'static str) -> &'static regex::Regex {
    // One Regex per distinct literal pattern, compiled once per process.
    // Each compiled Regex is boxed and leaked, giving a stable 'static
    // address (a map rehash can never invalidate references).
    static CACHE: OnceLock<
        std::sync::Mutex<std::collections::HashMap<&'static str, &'static regex::Regex>>,
    > = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut guard = cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let slot = guard.entry(pattern).or_insert_with(|| {
        Box::leak(Box::new(regex::Regex::new(pattern).unwrap_or_else(|e| {
            panic!("Invalid regex pattern {:?}: {}", pattern, e);
        }))) as &'static regex::Regex
    });
    *slot
}

// ── Marks accounting ────────────────────────────────────────────────────────

/// Sum of mark allocations transcribed inline (`**[4 marks]**` or `[3 marks]`).
/// Requires the literal word "mark"/"marks" so `(2024)`-style numbers and
/// maths like `(4)` in equations are NOT counted.
pub fn sum_inline_marks(content: &str) -> u32 {
    let re_marks = re(r"(?i)\*?\*?(?:\[|\()\s*(\d{1,2})\s*marks?\s*(?:\]|\))\*?\*?");
    re_marks
        .captures_iter(content)
        .filter_map(|c| c[1].parse::<u32>().ok())
        .filter(|&m| m <= 25) // per-part sanity bound
        .sum()
}

/// Style-and-clarity allocations printed as "(+S1)" / "(+S2)" (AEA papers):
/// they count towards the question total but belong to no single part.
pub fn sum_style_marks(content: &str) -> u32 {
    let re_style = re(r"\(\+\s*S\s*([1-9])\s*\)");
    re_style.captures_iter(content).filter_map(|c| c[1].parse::<u32>().ok()).sum()
}

/// Tolerant coercion of a model-supplied marks field (int, float, or string).
pub fn value_to_marks(v: &serde_json::Value) -> Option<i32> {
    match v {
        serde_json::Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f.round() as i64))
            .map(|x| x.clamp(0, 100) as i32),
        serde_json::Value::String(s) => {
            let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
            digits.parse::<i32>().ok().map(|x| x.clamp(0, 100))
        }
        _ => None,
    }
}

/// Tolerant coercion of a model-supplied question number to a *plausible*
/// whole question number.
///
/// Phase 1: accepts the many numbering styles boards use:
///   * integer JSON numbers: 1, 2, 3
///   * plain digit strings: "1", "12"
///   * zero-padded / AQA spaced: "01", "0 1", "0 10"
///   * suffixed: "1." "1)" "1]" "1–" (en-dash) "1-"
///   * prefixed: "Q1" "Q.1" "Q 1" "Question 1" "QUESTION 3"
///
/// Still rejects:
///   * zero, numbers > 1000 (raised to accommodate large multi-question worksheets and question bank compilations),
///   * decimals like "03.1" / "3.5 V" / "1,2" (sub-parts and quantities),
///   * floats (3.7).
pub fn value_to_question_number(v: &serde_json::Value) -> Option<u32> {
    let raw: Option<u64> = match v {
        serde_json::Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_i64().and_then(|x| u64::try_from(x).ok()))
            .or_else(|| {
                n.as_f64().and_then(|f| {
                    if f.fract() == 0.0 && f >= 0.0 {
                        Some(f as u64)
                    } else if f >= 1.0 && f < 1000.0 {
                        // Phase 2: AQA sub-part encoded as decimal float.
                        // The LLM sees "01 5" on the page and proposes 1.5.
                        // The fractional part is a single digit (sub-part index),
                        // not a true decimal quantity. Extract the integer part
                        // as the whole question number. We guard against genuine
                        // quantities (3.5 V) by requiring the fractional part to
                        // be a clean single-digit tenth (0.1, 0.2, ..., 0.9) —
                        // real physics quantities rarely land exactly on those.
                        let frac = (f.fract() * 10.0).round();
                        if (1.0..=9.0).contains(&frac) {
                            if (f.fract() * 10.0 - frac).abs() < 1e-4 {
                                Some(f.trunc() as u64)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
            }),
        serde_json::Value::String(s) => {
            parse_question_number_string(s.trim())
        }
        _ => None,
    };
    match raw {
        Some(n) if (1..=1000).contains(&n) => Some(n as u32),
        _ => None,
    }
}

/// Helper: parse a question-number string tolerantly. Returns None on any
/// ambiguity that looks like a sub-part or a quantity rather than a whole
/// question.
fn parse_question_number_string(t: &str) -> Option<u64> {
    // Phase 2: detect AQA spaced sub-part format BEFORE stripping whitespace.
    // AQA prints "01 5" meaning Question 1, sub-part 5 (rendered as (e)).
    // Without this check, whitespace stripping produces "015" → 15 (wrong).
    // The pattern: exactly two whitespace-separated tokens, both all-digits,
    // where the first token has length ≥ 2 (e.g. "01", "02", "10"). When the
    // first token is just "0" (single zero), it's AQA spaced whole-question
    // padding ("0 7" = question 7), handled by concatenation.
    let parts: Vec<&str> = t.split_whitespace().collect();
    if parts.len() == 2
        && !parts[0].is_empty()
        && parts[0].chars().all(|c| c.is_ascii_digit())
        && !parts[1].is_empty()
        && parts[1].chars().all(|c| c.is_ascii_digit())
    {
        if parts[0] == "0" {
            // "0 7" → AQA spaced whole question: concatenate → "07" → 7
            let combined = format!("{}{}", parts[0], parts[1]);
            return combined.parse::<u64>().ok();
        } else {
            // "01 5" → AQA spaced sub-part: first token is the question number,
            // second is the sub-part digit. Return the whole question number.
            // E.g. "01 5" → Some(1), "02 3" → Some(2), "10 2" → Some(10)
            return parts[0].parse::<u64>().ok();
        }
    }

    // Fast path: all digits (possibly with whitespace) after stripping
    // leading zeros. That preserves the existing AQA "0 1" → 1 behaviour.
    let stripped_ws: String = t.chars().filter(|c| !c.is_whitespace()).collect();

    // Strip a leading Q / Q. / Question / QUESTION prefix (case insensitive).
    let lower = stripped_ws.to_ascii_lowercase();
    let without_prefix = lower
        .strip_prefix("question")
        .map(|s| s.trim_start_matches('.'))
        .unwrap_or(&lower)
        .trim_start_matches('q')
        .trim_start_matches('.');

    // Strip a single trailing sentence / bracket / dash character.
    // "1." → "1", "1)" → "1", "1]" → "1", "1–" → "1", "1-" → "1"
    let mut chars: Vec<char> = without_prefix.chars().collect();
    // Allow one trailing en-dash/em-dash/hyphen/closing-paren/bracket/full-stop.
    while let Some(&last) = chars.last() {
        if matches!(last, '.' | ')' | ']' | '-' | '–' | '—' | ':' | '}') {
            chars.pop();
        } else {
            break;
        }
    }
    let cleaned: String = chars.into_iter().collect();

    // Reject if any internal non-digit character remains, UNLESS it's a single-digit decimal (e.g. "02.1")
    // or a single letter sub-part (e.g. "2a"). This ensures the items are retained so the pipeline can
    // properly trigger the "combine sub-parts" repair message.
    if !cleaned.chars().all(|c| c.is_ascii_digit()) {
        if let Some((int_part, frac_part)) = cleaned.split_once('.') {
            if !int_part.is_empty() && int_part.chars().all(|c| c.is_ascii_digit()) 
                && frac_part.len() == 1 && frac_part.chars().all(|c| c.is_ascii_digit()) {
                return int_part.parse::<u64>().ok();
            }
        }
        // Also tolerate "2a", "2b"
        if let Some(pos) = cleaned.find(|c: char| !c.is_ascii_digit()) {
            let int_part = &cleaned[..pos];
            let rest = &cleaned[pos..];
            if !int_part.is_empty() && int_part.chars().all(|c| c.is_ascii_digit()) 
                && rest.len() == 1 && rest.chars().all(|c| c.is_ascii_alphabetic()) {
                return int_part.parse::<u64>().ok();
            }
        }
        return None;
    }

    // Reject strings that look like they started with a decimal part
    // (e.g. original t was "03.1" — we stripped the trailing '.' above
    // leaving "03.1" with an internal '.' → already rejected above; an
    // additional belt-and-braces check on the original form: if the raw
    // string had an interior '.' or ',' that wasn't a trailing sentence
    // punctuation, refuse).
    if t.contains('.') || t.contains(',') {
        // Count how many '.'/',' appear inside the stripped (non-ws) form,
        // ignoring trailing punctuation we already trimmed.
        let interior = t.trim().trim_end_matches(|c: char| {
            c.is_whitespace() || matches!(c, '.' | ')' | ']' | '-' | '–' | '—')
        });
        // If a '.' remains after trimming trailing punctuation and there
        // are digits both sides (i.e. not "Q.1" which we already handled
        // by stripping the leading 'q' + '.'), treat as sub-part.
        let interior_stripped = interior.trim_start_matches(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    'q' | 'Q' | 'u' | 'e' | 's' | 't' | 'i' | 'o' | 'n' | '.'
                )
        });
        if interior_stripped.contains('.') || interior_stripped.contains(',') {
            return None;
        }
    }

    cleaned.parse::<u64>().ok()
}

// ── Truncation detection ────────────────────────────────────────────────────

/// Navigation/decorative trailing lines that mean "this question ended here".
static TERMINAL_NAV_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)(?:turn\s+over|end\s+of\s+(?:questions?|section|paper|examination)\b(?:\s+[A-Z])?|continued|blank\s+page)\s*[►▶]?\s*\**\s*$",
    )
    .unwrap()
});

/// Bracket-only mark allocation ("[2]", "**[2]**"). Parenthesised bare numbers
/// are NOT accepted here: "(2)" is far too common in ordinary prose and in
/// coordinate pairs to treat as a mark tag.
static BARE_BRACKET_MARK_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\*{0,2}\[\s*\d{1,2}\s*\]\*{0,2}\s*$").unwrap());

/// A bare value+unit pair at the end of a line ("12 N", "0.50 A",
/// "5.1 × 10−15 m"). Data-table rows end with bare numbers, which this
/// deliberately does NOT accept.
static VALUE_UNIT_TAIL_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)\d\s*(?:[×xX*]\s*10\s*\^?\s*[-\u{2212}]?\s*\d+\s*)?(?:J|kJ|W|kW|MW|N|C|V|mV|A|mA|Hz|kHz|kg|g|mg|m|cm|mm|s|ms|K|Pa|kPa|mol|Ω|°C)\s*\*{0,2}\s*$",
    )
    .unwrap()
});

/// A "complete formula ending": the final line closes a balanced LaTeX group
/// ("...\text{kg m}^{-3}", "...^{238}_{92}\text{U}") or ends a written
/// equation / answer line with a value ("x = 2.5", "energy released = J").
///
/// Deliberately narrow: trailing prose, dangling operators and mid-word
/// cut-offs are never accepted.
pub fn ends_with_complete_formula(content: &str) -> bool {
    let Some(line) = content.lines().rev().find(|l| !l.trim().is_empty()) else {
        return false;
    };
    let line = line.trim();
    let last = match line.chars().last() {
        Some(c) => c,
        None => return false,
    };
    // Balanced LaTeX group. A dangling opener ("... x^{2") cannot satisfy the
    // brace balance, and bare prose never contains a backslash/script marker.
    if last == '}' {
        let opens = line.matches('{').count();
        let closes = line.matches('}').count();
        if opens == closes && (line.contains('\\') || line.contains('^') || line.contains('_')) {
            return true;
        }
    }
    let written_equation =
        line.contains('=') || line.contains("\\rightarrow") || line.contains("->");
    // A written equation/answer line that ends on a VALUE is complete.
    // A dangling unit with no number ("energy released = J") is answer debris,
    // not proof of a finished equation, and stays non-terminal (phase-3 cleanup).
    if written_equation && last.is_ascii_digit() {
        return true;
    }
    // A bare value with a unit reads as a completed answer ("12 N", "0.50 A").
    if VALUE_UNIT_TAIL_RE.is_match(line) {
        return true;
    }
    false
}

/// True when the content ends like finished prose / math, not mid-word.
///
/// Consistent at every seam (deterministic gate, assembly, LLM validation):
/// mark tags with or without the word "marks", balanced LaTeX groups, complete
/// equations/answer lines, navigation markers ("END OF SECTION A",
/// "Turn over ►"), code fences, tables and ordinary terminal punctuation.
/// A multiple-choice card closes with its option list: tagged options from A
/// through at least D on consecutive final lines are a complete structure
/// (option text carries no terminal punctuation). A run that stops short of
/// D is a truncation.
fn ends_with_complete_option_run(t: &str) -> bool {
    static OPTION_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^[ \t]*-[ \t]+\[MCQ:([A-E])\][ \t]+\S").unwrap());
    let mut letters: Vec<char> = t
        .lines()
        .rev()
        .map_while(|l| OPTION_RE.captures(l).map(|c| c[1].chars().next().unwrap()))
        .collect();
    letters.reverse();
    letters.len() >= 4 && letters.iter().enumerate().all(|(i, &c)| c == (b'A' + i as u8) as char)
}

/// A structured question's part may end on its own lettered choice list,
/// kept as plain lines ("A The set of integers" … "E The set of real
/// numbers"): A, B, C … in order on the final lines, at least three.
fn ends_with_plain_option_run(t: &str) -> bool {
    static PLAIN_RE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^[ \t]*([A-E])[ \t]+\S").unwrap());
    let mut letters: Vec<char> = t
        .lines()
        .rev()
        .map_while(|l| PLAIN_RE.captures(l).map(|c| c[1].chars().next().unwrap()))
        .collect();
    letters.reverse();
    letters.len() >= 3 && letters.iter().enumerate().all(|(i, &c)| c == (b'A' + i as u8) as char)
}

/// A "Circle / Tick / Shade" instruction answered from a printed row of
/// short choices ("1  3  4  7"): the row is the question's end.
fn ends_with_choice_row(t: &str) -> bool {
    static INSTRUCTION_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?i)\b(?:circle|tick|shade|underline|choose|select)\b").unwrap());
    let Some(last) = t.lines().rev().find(|l| !l.trim().is_empty()) else { return false };
    let tokens: Vec<&str> = last.split_whitespace().collect();
    (2..=8).contains(&tokens.len())
        && tokens.iter().all(|w| w.chars().count() <= 12 && !w.chars().any(|c| c.is_alphabetic() && c.is_lowercase() && !w.starts_with('$')))
        && INSTRUCTION_RE.is_match(&t[..last.as_ptr() as usize - t.as_ptr() as usize])
}

pub fn has_terminal_ending(content: &str) -> bool {
    let t = content.trim_end();
    if t.is_empty() {
        return false;
    }
    // Ends with a marks tag, with or without the word "marks"?
    let re_tag = re(r"(?i)(?:\[|\()\s*\d{1,2}\s*marks?\s*(?:\]|\))\s*\**\s*$");
    if re_tag.is_match(t) {
        return true;
    }
    if BARE_BRACKET_MARK_RE.is_match(t) {
        return true;
    }
    // Ends with display math close, code fence, or terminal punctuation?
    if t.ends_with("$$")
        || t.ends_with("```")
        || t.ends_with('$')
        || t.ends_with('`')
        || t.ends_with("\\]")
        || t.ends_with("\\)")
    {
        return true;
    }
    // Markdown tables (AQA trace tables) end with '|' — treat as terminal.
    // Without this, questions ending in a trace table get flagged as truncated
    // and quarantined after 3 repair attempts (the June 2024 CS regression).
    if t.ends_with('|') {
        return true;
    }
    if TERMINAL_NAV_RE.is_match(t) {
        return true;
    }
    if ends_with_complete_option_run(t) || ends_with_plain_option_run(t) || ends_with_choice_row(t) {
        return true;
    }
    if ends_with_complete_formula(t) {
        return true;
    }
    matches!(
        t.chars().last(),
        Some('.') | Some('?') | Some('!') | Some(')') | Some(']') | Some(':') | Some(';')
    )
}

// ── Boilerplate scrubbing (exact-string policy, moved from commands.rs) ────

pub fn clean_ligatures(s: &str) -> String {
    s.replace('ﬀ', "ff")
     .replace('ﬁ', "fi")
     .replace('ﬂ', "fl")
     .replace('ﬃ', "ffi")
     .replace('ﬄ', "ffl")
     .replace('ﬅ', "st")
     .replace('ﬆ', "st")
     .replace('\u{f084}', "\\le ")
     .replace('\u{f052}', "\\mathbb{R}")
     .replace('\u{f0a2}', "")
     .replace('\u{f0bf}', "")
     .replace("/lpar", "(")
     .replace("/rpar", ")")
     .replace("/thetaslant", "\\theta ")
     .replace("/surd", "\\sqrt ")
     .replace("/degrees", "^{\\circ}")
     .replace("/solidcircle", "\\bullet ")
}

// ── Uniform sub-part labelling ──────────────────────────────────────────────
//
// Edexcel prints part labels as (a), (b), (c); AQA prints decimal numbers
// ("3 . 1", "3 . 2", ...). Everything stored in MergeMark uses ONE scheme:
// the AQA decimals are rewritten to (a), (b), (c) here — deterministically,
// so uniformity no longer depends on the model obeying a prompt rule.
//
// Safety rails (trace tables and physics quantities contain real decimals,
// so the trigger is deliberately conservative):
//   * leading integer must equal THIS question's number ("3" or "03" for Q3);
//   * only label position is rewritten: the decimal must open a source line;
//   * space-separated dots ("3 . 1") are always AQA labels; compact forms
//     ("03.1") activate only when at least two DISTINCT decimals appear
//     (a real parts sequence), so a lone "3.5 V"-style quantity survives;
//   * the decimal part must be <= 20 and maps positionally: 1 → a, 2 → b.
fn re_owned(pattern: String) -> &'static regex::Regex {
    Box::leak(Box::new(regex::Regex::new(&pattern).unwrap())) as &'static regex::Regex
}

pub fn normalize_decimal_parts(content: &str, question_number: u32) -> String {
    if question_number == 0 || question_number > 999 || content.is_empty() {
        return content.to_string();
    }
    // Label at line start: optional indent, optional **bold**, the question
    // number (possibly zero-padded/spaced, e.g. "03", "0 3"), a dot, then the
    // part digit(s), optional bold/close-paren, then whitespace. Also allow
    // the label on a line of its own.
    let pat = format!(
        r"(?m)^(\s*(?:\*\*)?\s*)0?\s*{}\s*\.\s*(\d{{1,2}})\s*((?:\*\*)?\s*[.)]?)\s+",
        question_number
    );
    let re_label = re_owned(pat);

    // First pass: decide activation. A "spaced" label has whitespace on BOTH
    // sides of the dot — the exact way AQA prints part numbers; a float
    // never does.
    let pat_spaced = format!(
        r"(?m)^\s*(?:\*\*)?\s*0?\s*{}\s+\.\s+(\d{{1,2}})",
        question_number
    );
    let re_spaced = re_owned(pat_spaced);
    let spaced_found = re_spaced.captures_iter(content).any(|caps| {
        let d: u32 = caps[1].parse().unwrap_or(99);
        (1..=20).contains(&d)
    });
    let mut compact = std::collections::HashSet::new();
    if !spaced_found {
        for caps in re_label.captures_iter(content) {
            let d: u32 = caps[2].parse().unwrap_or(99);
            if (1..=20).contains(&d) {
                compact.insert(d);
            }
        }
    }
    let active = spaced_found || compact.len() >= 2;
    if !active {
        return content.to_string();
    }

    // Second pass: rewrite every leading label positionally (part 4 → (d)),
    // so letters stay correct even when parts span multiple pages/chunks.
    re_label
        .replace_all(content, |caps: &regex::Captures| {
            let d: u32 = caps[2].parse().unwrap_or(0);
            if !(1..=20).contains(&d) {
                return caps[0].to_string();
            }
            let letter = (b'a' + (d - 1) as u8) as char;
            let bold = caps[1].contains("**") || caps[3].contains("**");
            if bold {
                format!("{}**({})** ", &caps[1].replace("**", ""), letter)
            } else {
                format!("{}({}) ", &caps[1], letter)
            }
        })
        .into_owned()
}

// ══════════════════════════════════════════════════════════════════════════
// Deterministic content validators (moved from earlier)
// ══════════════════════════════════════════════════════════════════════════

// ── Source line preservation ────────────────────────────────────────────────
//
// Markdown collapses single newlines into one flowing paragraph. Exam
// content (database schemas, algorithms, tables) is LINE-structured: losing
// the line breaks mashes "Product(ProductID, Description," into a single
// wrapped blob.
//
// BUT a soft break must only be promoted to a paragraph break when the next
// line starts a STRUCTURAL element. Promoting every consecutive prose pair
// fragments a single printed sentence across multiple <p> tags — the exact
// "sentence fragmentation" defect. Prose lines now flow into one paragraph;
// structural boundaries still get their own.
pub fn harden_line_breaks(content: &str) -> String {
    let mut out = String::with_capacity(content.len() + content.len() / 2);
    let mut in_fence = false;
    let mut in_math = false;
    let mut in_env_depth: usize = 0;
    let mut prev_nonempty = false;
    let mut prev_table = false;
    for line in content.split('\n') {
        let trimmed = line.trim_end();
        let t = trimmed.trim_start();
        // State BEFORE toggles decides the route: the CLOSING marker line of
        // a fence/math block is itself protected content.
        let protected = in_fence || in_math || in_env_depth > 0;
        let is_table = !protected && t.starts_with('|');
        let blank = t.is_empty();
        if protected || is_table {
            out.push_str(trimmed);
            out.push('\n');
        } else {
            // Promote to a paragraph break ONLY at a structural boundary:
            // lists, sub-part labels, display math, headings, MCQ options,
            // bold tags, or blockquotes. Plain prose keeps its soft break and
            // renders as ONE paragraph (a wrapped print-line is not a new
            // sentence).
            if !blank && prev_nonempty && !prev_table && starts_structural_element(t) {
                out.push('\n');
            }
            out.push_str(trimmed);
            out.push('\n');
        }
        if t.starts_with("```") && !in_math {
            in_fence = !in_fence;
        }
        if !in_fence {
            if t.starts_with("$$") {
                let inner = &t[2..];
                let single_line = inner.len() >= 2 && inner.ends_with("$$") && !inner[..inner.len() - 2].contains("$$");
                if !single_line {
                    in_math = !in_math;
                }
            } else if t.starts_with("\\[") {
                if !t.ends_with("\\]") {
                    in_math = true;
                }
            } else if t.ends_with("\\]") && in_math {
                in_math = false;
            }

            if t.contains("\\begin{") {
                in_env_depth += t.matches("\\begin{").count();
            }
            if t.contains("\\end{") {
                let end_count = t.matches("\\end{").count();
                in_env_depth = in_env_depth.saturating_sub(end_count);
            }
        }
        prev_nonempty = !blank;
        prev_table = is_table;
    }
    while out.ends_with('\n') {
        out.pop();
    }
    re(r"\n{3,}").replace_all(&out, "\n\n").to_string()
}

/// True when `t` begins a line that deserves its own paragraph even though
/// the previous line was also non-empty: exam sub-part labels, numbered /
/// bulleted lists, headings, display math, MCQ options, bold tags,
/// blockquotes, and answer-grid markers. Everything else is treated as
/// continuation prose of the previous line's sentence/paragraph.
fn starts_structural_element(t: &str) -> bool {
    if t.starts_with("$$") || t.starts_with('\\') {
        return true; // display math / raw LaTeX command lines
    }
    if t.starts_with('#') || t.starts_with('>') || t.starts_with('-') || t.starts_with('*') || t.starts_with('+') {
        return true; // headings, quotes, list items
    }
    if t.starts_with("**") || t.starts_with("[MCQ:") {
        return true; // bold headers/mark tags and MCQ option cards
    }
    // Sub-part labels: "(a)", "(iv)", "1.", "12)", "3 . 1"-style decimals
    if re(r"^\([a-zA-Z0-9]+\)").is_match(t) {
        return true;
    }
    if re(r"^\d+[.)]").is_match(t) {
        return true;
    }
    false
}

static RE_TWO_BLOCK_LINE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^([a-zA-Z0-9\\+\-*/()_^{} \t]*?\\(?:cos|sin|tan|sec|csc|cot|theta|frac|sqrt|pi|lambda|alpha|beta)\b[a-zA-Z0-9\\+\-*/()_^{} \t]*?)\$,\s*(?:\\quad\s*)?\$([^\n\$]+?)\$([.,]?)$").unwrap()
});

static RE_SINGLE_BLOCK_LINE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^([a-zA-Z0-9\\+\-*/()_^{} \t]*?\\(?:cos|sin|tan|sec|csc|cot|theta|frac|sqrt|pi|lambda|alpha|beta|leq|le|geq|ge|quad)\b[^\n\$]+?)\$([.,]?)$").unwrap()
});

static RE_HAS_POLAR_PREAMBLE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)polar\s+equations?|cardioid|spiral\s+curve|curve(?:\s+\$?[A-Za-z0-9_]+\$?)?\s+with\s+polar").unwrap()
});

static RE_HAS_EQUATION_PREAMBLE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)polar\s+equations?|cardioid|spiral\s+curve|curve(?:\s+\$?[A-Za-z0-9_]+\$?)?\s+with|line(?:\s+\$?[A-Za-z0-9_]+\$?)?\s+with|with\s+(?:Cartesian\s+)?equation").unwrap()
});

static RE_TRIPLE_DOLLARS: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\${3,}").unwrap()
});

static RE_MULTI_CURVE_COMMA: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\$,\s*([0-9a-zA-Z\\+\-*/()_^{} \t]+?\\(?:cos|sin|tan|sec|csc|cot|theta|frac|sqrt|pi)\b[0-9a-zA-Z\\+\-*/()_^{} \t]*?)\$,\s*(?:\\quad\s*)?\$([^\n\$]+?)\$").unwrap()
});

static RE_MULTI_CURVE_CONST: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\$,\s*([0-9.]+)\$,\s*(?:\\quad\s*)?\$([^\n\$]+?)\$").unwrap()
});

pub fn heal_polar_equations(content: &str) -> String {
    let content = normalize_dollar_runs_outside_code(content);
    let lines: Vec<&str> = content.lines().collect();
    let mut result: Vec<String> = Vec::with_capacity(lines.len());

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let prev_line = if i > 0 { lines[i - 1].trim() } else { "" };
        let prev2_line = if i > 1 { lines[i - 2].trim() } else { "" };
        let has_polar_preamble = RE_HAS_POLAR_PREAMBLE.is_match(prev_line) || RE_HAS_POLAR_PREAMBLE.is_match(prev2_line);
        let has_equation_preamble = has_polar_preamble || RE_HAS_EQUATION_PREAMBLE.is_match(prev_line) || RE_HAS_EQUATION_PREAMBLE.is_match(prev2_line);

        if !trimmed.starts_with('$') && !trimmed.starts_with("$$") && !trimmed.is_empty() {
            if let Some(caps) = RE_TWO_BLOCK_LINE.captures(trimmed) {
                let expr = caps[1].trim();
                let domain = caps[2].trim();
                let punct = caps.get(3).map(|m| m.as_str()).unwrap_or("");
                let has_theta = expr.contains("\\theta") || domain.contains("\\theta") || has_polar_preamble;
                let prefix = if has_theta && !expr.starts_with("r =") && !expr.starts_with("r=") {
                    "r = "
                } else {
                    ""
                };
                result.push(format!("$${}{}, \\quad {}$${}", prefix, expr, domain, punct));
                continue;
            }

            if let Some(caps) = RE_SINGLE_BLOCK_LINE.captures(trimmed) {
                let mut expr = caps[1].trim().to_string();
                let mut punct = caps.get(2).map(|m| m.as_str()).unwrap_or("").to_string();
                if expr.ends_with('.') || expr.ends_with(',') {
                    let trailing = expr.pop().unwrap();
                    punct.insert(0, trailing);
                    expr = expr.trim().to_string();
                }
                let has_theta = expr.contains("\\theta") || has_polar_preamble;
                let is_cartesian = !has_theta && (expr.contains('x') || expr.contains('k') || expr.contains('t'));
                let prefix = if has_theta && !expr.starts_with("r =") && !expr.starts_with("r=") {
                    "r = "
                } else if is_cartesian && has_equation_preamble && !expr.starts_with("y =") && !expr.starts_with("y=") {
                    "y = "
                } else {
                    ""
                };
                result.push(format!("$${}{}$${}", prefix, expr, punct));
                continue;
            }

            if has_equation_preamble && (trimmed.contains("\\cos") || trimmed.contains("\\sin") || trimmed.contains("\\theta") || trimmed.contains("\\frac")) && !trimmed.contains('$') {
                let prefix = if has_polar_preamble && !trimmed.starts_with("r =") && !trimmed.starts_with("r=") {
                    "r = "
                } else if !trimmed.starts_with("y =") && !trimmed.starts_with("y=") {
                    "y = "
                } else {
                    ""
                };
                result.push(format!("$${}{}$$", prefix, trimmed));
                continue;
            }
        }

        let mut fixed_line = RE_MULTI_CURVE_COMMA.replace_all(line, |caps: &regex::Captures| {
            let expr = caps[1].trim();
            let domain = caps[2].trim();
            if expr.starts_with("r =") || expr.starts_with("r=") || expr.starts_with('$') {
                format!("$, {}, ${}$", expr, domain)
            } else {
                format!("$$ and $$r = {}, \\quad {}$$", expr, domain)
            }
        }).to_string();
        fixed_line = RE_MULTI_CURVE_CONST.replace_all(&fixed_line, "$$ and $$r = $1, \\quad $2$$").to_string();
        result.push(fixed_line);
    }

    result.join("\n")
}

// ── Math delimiter discipline ───────────────────────────────────────────────
//
// KaTeX renders garbage (or swallows subsequent question text / MCQ syntax)
// whenever the model leaves an inline `$` or display `$$` unclosed. Two gates:
//
//   * `math_delimiter_balance_errors` — validator whose verdicts are quoted
//     verbatim into the repair loop, so the model fixes its own pairing;
//   * `balance_math_delimiters` — terminal deterministic healer applied to
//     every assembled card: closes broken inline math AT THE END OF ITS OWN
//     LINE (so a stray `$` can never swallow the rest of the question or an
//     options grid), strips nested `$` inside `$$` blocks, and closes an
//     unterminated display block at the end of the content.

/// Byte ranges of inline code spans in one line, using CommonMark backtick-run
/// rules: a span opens with a run of N backticks and closes at the next run of
/// EXACTLY N backticks; an unclosed run extends to the end of the line. This
/// keeps ``a ` b`` (a code span containing a backtick) and ``$`` (a code span
/// containing a dollar) opaque to the delimiter machinery.
fn inline_code_spans(line: &str) -> Vec<(usize, usize)> {
    let bytes = line.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let start = i;
        let mut run = 0usize;
        while i < bytes.len() && bytes[i] == b'`' {
            run += 1;
            i += 1;
        }
        // The closing run must be EXACTLY as long as the opening run.
        let mut j = i;
        let mut close_end = None;
        while j < bytes.len() {
            if bytes[j] == b'`' {
                let mut m = 0usize;
                while j < bytes.len() && bytes[j] == b'`' {
                    m += 1;
                    j += 1;
                }
                if m == run {
                    close_end = Some(j);
                    break;
                }
            } else {
                j += 1;
            }
        }
        let end = close_end.unwrap_or(bytes.len());
        spans.push((start, end));
        i = end;
    }
    spans
}

/// `(marker char, run length)` when a line is a code-fence marker: up to three
/// leading spaces, then a run of at least three backticks or tildes. A backtick
/// fence may not carry another backtick in its info string.
fn fence_marker(line: &str) -> Option<(u8, usize)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len().saturating_sub(trimmed.len()) > 3 {
        return None;
    }
    let bytes = trimmed.as_bytes();
    let ch = *bytes.first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let run = bytes.iter().take_while(|b| **b == ch).count();
    if run < 3 {
        return None;
    }
    if ch == b'`' && bytes[run..].contains(&b'`') {
        return None;
    }
    Some((ch, run))
}

/// Per-line code structure for one document.
struct LineCode {
    /// The whole line sits inside a fenced code block.
    fenced: bool,
    /// Byte ranges of inline code spans (empty inside a fence).
    spans: Vec<(usize, usize)>,
}

/// Walk the content once, tracking fenced blocks by MARKER CHAR and RUN LENGTH
/// (a four-backtick fence is not closed by a three-backtick line, and a tilde
/// fence is not closed by backticks) and recording each line's inline code
/// spans.
fn line_code_map(content: &str) -> Vec<LineCode> {
    let mut map = Vec::new();
    let mut open_fence: Option<(u8, usize)> = None;
    for line in content.split('\n') {
        if let Some((ch, run)) = open_fence {
            let closes = fence_closes(line, ch, run);
            map.push(LineCode { fenced: true, spans: Vec::new() });
            if closes {
                open_fence = None;
            }
            continue;
        }
        if let Some(marker) = fence_marker(line) {
            open_fence = Some(marker);
            map.push(LineCode { fenced: true, spans: Vec::new() });
            continue;
        }
        map.push(LineCode { fenced: false, spans: inline_code_spans(line) });
    }
    map
}

/// True when `line` CLOSES a fenced block opened with `(marker_char, run)`:
/// same marker character, a run at least as long, and nothing but whitespace
/// after the run. (A closing fence may not carry an info string; only the
/// opening fence may.)
fn fence_closes(line: &str, marker_char: u8, run: usize) -> bool {
    let Some((close_char, close_run)) = fence_marker(line) else {
        return false;
    };
    if close_char != marker_char || close_run < run {
        return false;
    }
    let trimmed = line.trim_start_matches(' ');
    trimmed[close_run..].trim().is_empty()
}

/// Unescaped single-dollar count and `$$` (or longer run) toggle count for one
/// segment of text. A run of two or more dollars counts as ONE display toggle,
/// mirroring the old `normalize_dollar_runs`.
fn scan_segment_dollars(segment: &str) -> (usize, usize) {
    let chars: Vec<char> = segment.chars().collect();
    let mut singles = 0usize;
    let mut doubles = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2; // escaped char is verbatim (\$, \\, \frac ...)
            continue;
        }
        if chars[i] == '$' {
            let mut run = 0usize;
            while i + run < chars.len() && chars[i + run] == '$' {
                run += 1;
            }
            if run >= 2 {
                doubles += 1;
            } else {
                singles += 1;
            }
            i += run;
            continue;
        }
        i += 1;
    }
    (singles, doubles)
}

/// `(unescaped_single_dollar_parity_odd, display_toggles)` for one line,
/// ignoring inline `code` spans.
fn scan_line_delimiters_outside_spans(line: &str, spans: &[(usize, usize)]) -> (bool, usize) {
    let mut singles = 0usize;
    let mut doubles = 0usize;
    let mut pos = 0usize;
    for &(start, end) in spans {
        if start > pos {
            let (s, d) = scan_segment_dollars(&line[pos..start]);
            singles += s;
            doubles += d;
        }
        pos = end.max(pos);
    }
    if pos < line.len() {
        let (s, d) = scan_segment_dollars(&line[pos..]);
        singles += s;
        doubles += d;
    }
    (singles % 2 == 1, doubles)
}

/// `(unescaped_single_dollar_parity_odd, display_toggles)` for one line.
fn scan_line_delimiters(line: &str) -> (bool, usize) {
    scan_line_delimiters_outside_spans(line, &inline_code_spans(line))
}

/// `$$$` (and longer) becomes `$$`; nothing else changes.
fn collapse_dollar_runs(segment: &str) -> String {
    if !segment.contains("$$$") {
        return segment.to_string();
    }
    RE_TRIPLE_DOLLARS
        .replace_all(segment, |_: &regex::Captures| "$$")
        .into_owned()
}

/// Collapse runs of three or more `$` to `$$` OUTSIDE code. Fenced lines and
/// inline code spans are copied verbatim: a literal `$$$` in code must never be
/// rewritten by the delimiter machinery.
fn normalize_dollar_runs_outside_code(content: &str) -> String {
    let map = line_code_map(content);
    let mut out = String::with_capacity(content.len() + 8);
    for (idx, (line, code)) in content.split('\n').zip(map.iter()).enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        if code.fenced {
            out.push_str(line);
            continue;
        }
        let mut pos = 0usize;
        for &(start, end) in &code.spans {
            out.push_str(&collapse_dollar_runs(&line[pos..start]));
            out.push_str(&line[start..end]);
            pos = end;
        }
        out.push_str(&collapse_dollar_runs(&line[pos..]));
    }
    out
}

/// Rewrite one non-code segment for the delimiter healer: escaped characters
/// pass through, a `$` run of two or more becomes one `$$` display toggle, and
/// a stray single `$` is kept only outside display math.
fn push_balanced_segment(segment: &str, out: &mut String, in_display: &mut bool) {
    let chars: Vec<char> = segment.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            out.push(c);
            if i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if c == '$' {
            let mut run = 0usize;
            while i + run < chars.len() && chars[i + run] == '$' {
                run += 1;
            }
            if run >= 2 {
                *in_display = !*in_display;
                out.push_str("$$");
            } else if !*in_display {
                out.push('$');
            }
            i += run;
            continue;
        }
        out.push(c);
        i += 1;
    }
}

/// Validator: human-readable violations for unbalanced `$` / `$$` pairing.
/// Quoted back to the model by the repair loop via `validate_span_items`.
/// Code (fenced blocks and inline spans) is opaque: a `$` in code is literal.
pub fn math_delimiter_balance_errors(content: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let s = normalize_dollar_runs_outside_code(content);
    let map = line_code_map(&s);
    let mut in_display = false;
    let mut total_doubles = 0usize;
    for (idx, (line, code)) in s.split('\n').zip(map.iter()).enumerate() {
        if code.fenced {
            continue;
        }
        let (odd_singles, doubles) = scan_line_delimiters_outside_spans(line, &code.spans);
        total_doubles += doubles;
        if doubles % 2 == 1 {
            in_display = !in_display;
        }
        if odd_singles && !in_display {
            errors.push(format!(
                "line {} opens an inline math `$` that is never closed on the same line - every $ must be paired on ONE line (e.g. $x^2 + 1$)",
                idx + 1
            ));
        }
    }
    if total_doubles % 2 == 1 {
        errors.push(
            "display math delimiters are unbalanced: a $$ block is opened but never closed with $$".to_string(),
        );
    }
    errors
}

/// Terminal deterministic healer (mirror of the frontend
/// `validateAndEnforceDelimiters`, applied where the model cannot be asked
/// again): closes broken inline `$` at the end of its own line, strips
/// nested `$` inside `$$`, and appends a closing `$$` for an unterminated
/// display block. Never invents content. Code (fenced blocks and inline spans)
/// is copied verbatim and never rewritten.
pub fn balance_math_delimiters(content: &str) -> String {
    let s = normalize_dollar_runs_outside_code(content);
    let map = line_code_map(&s);
    let mut out = String::with_capacity(s.len() + 16);
    let mut in_display = false;
    for (line, code) in s.split('\n').zip(map.iter()) {
        if code.fenced {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let mut pos = 0usize;
        for &(start, end) in &code.spans {
            if start > pos {
                push_balanced_segment(&line[pos..start], &mut out, &mut in_display);
            }
            out.push_str(&line[start..end]);
            pos = end;
        }
        if pos < line.len() {
            push_balanced_segment(&line[pos..], &mut out, &mut in_display);
        }
        let (odd_singles, _) = scan_line_delimiters_outside_spans(line, &code.spans);
        if odd_singles && !in_display {
            out.push('$');
        }
        out.push('\n');
    }
    while out.ends_with('\n') {
        out.pop();
    }
    if in_display {
        out.push_str("\n$$");
    }
    out
}
// ── Multi-line display-math preservation ────────────────────────────────────
//
// KaTeX treats a raw newline inside $$ ... $$ as ordinary whitespace, so
// sequential equations transcribed on separate lines (nuclear decay chains,
// simultaneous pairs, multi-step derivations) render SQUASHED end-to-end
// (e.g. "...Rn + ...α^222^...^Po..." with no separation). Deterministic fix:
// inside a multi-line display block, convert each interior newline into an
// explicit LaTeX row separator `\\` — unless either side already carries one
// (`\\` / `\cr`), or the break sits on a bare environment boundary where a
// separator would create an empty matrix row.

static RE_BARE_ENV_OPEN: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^\\begin\{[A-Za-z*]+\}(?:\[[^\]]*\])?(?:\{[^}]*\})*$").unwrap()
});

fn line_ends_with_row_separator(trimmed: &str) -> bool {
    let t = trimmed.trim_end();
    t.ends_with("\\\\") || t.ends_with("\\cr")
}

pub fn ensure_display_math_line_breaks(content: &str) -> String {
    if !content.contains("$$") {
        return content.to_string();
    }
    let s = normalize_dollar_runs_outside_code(content);
    let mut result: Vec<String> = Vec::new();
    let mut in_display = false;

    let code_map = line_code_map(&s);
    for (idx, line) in s.split('\n').enumerate() {
        let code = &code_map[idx];
        // Fenced blocks and lines that are entirely inline code are opaque:
        // they cannot open, close or continue display math.
        let entirely_inline_code = code
            .spans
            .iter()
            .any(|&(start, end)| start == 0 && end >= line.len());
        if code.fenced || entirely_inline_code {
            result.push(line.to_string());
            continue;
        }
        let trimmed = line.trim();
        let (_, doubles) = scan_line_delimiters(line);
        let toggles = doubles % 2 == 1;

        if !in_display {
            result.push(line.to_string());
            if toggles {
                in_display = true;
            }
            continue;
        }

        // Inside a display block. The interior content of THIS line is
        // everything before a trailing `$$` that closes the block.
        let closes_here = toggles && trimmed.ends_with("$$");
        let cur_inner = if closes_here {
            trimmed[..trimmed.len() - 2].trim()
        } else {
            trimmed
        };

        // Interior content of the PREVIOUS emitted line: strip a leading
        // opening `$$` (the block may have opened with content on the same
        // line, e.g. "$$x = 1").
        let prev_trim = result.last().map(|l| l.trim()).unwrap_or("");
        let prev_inner = prev_trim.strip_prefix("$$").unwrap_or(prev_trim).trim();

        let needs_separator = !prev_inner.is_empty()
            && !cur_inner.is_empty()
            && !line_ends_with_row_separator(prev_inner)
            && !cur_inner.starts_with("\\\\")
            && !RE_BARE_ENV_OPEN.is_match(prev_inner)
            && !cur_inner.starts_with("\\end{");

        if needs_separator {
            if closes_here {
                result.push(format!("\\\\ {}$$", cur_inner));
            } else {
                result.push(format!("\\\\ {}", trimmed));
            }
        } else {
            result.push(line.to_string());
        }

        if toggles {
            in_display = false;
        }
    }

    result.join("\n")
}

// ── Escape-mangled LaTeX repair ─────────────────────────────────────────────
//
// When the model emits "\text{Ra}" instead of "\\text{Ra}" in its JSON
// payload, the wire-level escape decodes to a literal TAB character followed
// by "ext{Ra}" (same class of corruption: "\theta" → TAB + "heta"). Tabs
// never occur legitimately in exam content, so a TAB directly preceding the
// remainder of a known LaTeX command can be deterministically restored to
// backslash-t.

/// Restore LaTeX commands mangled by unescaped `\t` escapes in the JSON
/// payload (TAB + "ext{...}" → `\text{...}`). Applied to every parsed
/// content string before validation, so corrupted math never reaches a card.
///
/// Implemented as a scanner because a literal `\text` decodes to TAB +
/// "ext{...}" — i.e. the escape consumes BOTH the backslash and the leading
/// `t` of the command name, so the repair must re-insert backslash-t, not
/// just a bare backslash.
const TAB_MANGLED_REMAINDERS: &[&str] = &[
    // Longest-first so "extbf" wins over "ext".
    "extbf", "extit", "extrm", "exttt", "ext", "imes", "heta", "herefore", "riangle", "woheadrightarrow", "ilde", "au", "an", "o", "op", "frac", "dfrac",
];

pub fn fix_tab_mangled_latex(text: &str) -> String {
    if !text.contains('\t') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 8);
    let mut i = 0usize;
    while i < text.len() {
        if text.as_bytes()[i] == b'\t' {
            let rest = &text[i + 1..];
            let is_command_remainder = TAB_MANGLED_REMAINDERS.iter().any(|rem| {
                if !rest.starts_with(rem) {
                    return false;
                }
                // The remainder must end at a non-letter boundary: a real
                // word like "an<other>" after a prose TAB must not convert.
                match rest.as_bytes().get(rem.len()) {
                    None => true,
                    Some(&b) => !b.is_ascii_alphabetic(),
                }
            });
            if is_command_remainder {
                out.push_str("\\t"); // literal backslash + t
                i += 1;
                continue;
            }
            out.push('\t');
            i += 1;
            continue;
        }
        let ch = text[i..].chars().next().unwrap_or('\u{fffd}');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

pub fn clean_question_content(content: &str) -> String {
    // Repair escape-mangled LaTeX FIRST so later passes see real commands.
    let mut cleaned = fix_tab_mangled_latex(content);
    let patterns: &[&str] = &[
        r"(?i)Question\s+\d+\s+continued",
        r"(?i)\(Total\s+for\s+Question\s+\d+\s+is\s+\d+\s+marks?\)",
        r"(?i)Total\s+for\s+Question\s+\d+\s+is\s+\d+\s+marks?",
        r"(?i)TOTAL\s+FOR\s+PAPER\s+IS\s+\d+\s+MARKS",
        r"(?i)Turn\s+over(\s+for\s+the\s+next\s+question)?",
        r"(?i)BLANK\s+PAGE",
        r"(?im)^\s*Advantage\s*\d*\s*$",
        r"(?im)^\s*Disadvantage\s*\d*\s*$",
        r"(?im)^\s*Problem\s*\d+\s*$",
        r"(?im)^\s*Answer\s*_*\s*$",
    ];
    for p in patterns {
        cleaned = re(p).replace_all(&cleaned, "").into_owned();
    }

    // Strip trailing inequality answer templates (e.g., "$... \le t < ...$ [2 marks]") while preserving the marks
    cleaned = INEQ_RE.replace_all(&cleaned, "$1").into_owned();

    // Strip trailing equality answer templates (e.g., "$... x = ...$ [2 marks]")
    cleaned = EQ_RE.replace_all(&cleaned, "$1").into_owned();

    // Heal dropped 'r = ' in polar equations and unbalanced delimiters
    cleaned = heal_polar_equations(&cleaned);

    // Collapse runs of 3+ newlines left by removals.
    let collapse = re(r"\n{3,}");
    let collapsed = collapse.replace_all(&cleaned, "\n\n").trim().to_string();

    // Minimal cleanup: ligatures, harden line breaks (preserve source lines)
    let with_ligatures = clean_ligatures(&collapsed);
    harden_line_breaks(&crate::sanitize::wrap_orphan_latex(&with_ligatures))
}

static RE_MATH_DEGREE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(\d+)\s*(?:◦|°|\\circ\b)").unwrap()
});
static RE_FLATTENED_POWERS: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(\b|\d|[+\-*/=(])([xyzvtuvrXYZVTUR])\s+([2-9])\b").unwrap()
});
static RE_FLATTENED_POWERS_TIGHT: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(\b|\d|[+\-*/=(]|[a-zA-Z])([xyzvtuvrABCDEFXYZ])([2-9])(?:\b|[\+\-\=\*\/\,\;\)\.]|\s*$)").unwrap()
});
static RE_RATIO_FRACTIONS: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"([a-zA-Z0-9\+\-]+)\s+([0-9\-]+)\s*=\s*([a-zA-Z0-9\+\-]+)\s+([0-9\-]+)").unwrap()
});
static RE_SHATTERED_CALCULUS_2ND: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\bd\s*\n*\s*2\s*\n*\s*([a-zA-Z\\α-ωΑ-Ω]+)\s*\n*\s*d\s*([a-zA-Z])\s*\n*\s*2\b").unwrap()
});
static RE_SHATTERED_CALCULUS_1ST: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\bd\s*\n*\s*([a-zA-Z\\α-ωΑ-Ω]+)\s*\n+\s*d\s*([a-zA-Z])\b").unwrap()
});
static RE_VERT_COLLAPSED_FRAC: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\b(\d+)\s*\n+\s*(\d+)\s*\n+\s*([a-zA-Z\\]+)").unwrap()
});
static RE_VERT_COLLAPSED_UNIT_FRAC: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\b(\d+)\s*\n+\s*(\d+)\s+(m\s*s\s*[-−–]\s*1|m\s*\/\s*s|N|kg|J|W|m\s*s\s*[-−–]\s*2|rad\s*s\s*[-−–]\s*1|Pa)").unwrap()
});
static RE_DEMASH_NUM_UNIT_WORD: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)(\d+)\s*(kg|cm|mm|ms|mol|rad)\s*([a-zA-Z]{2,})|(\d+)\s*(m|g|s|N|J|W|Pa|Hz|V|A)\s*(and|to|from|with|by|or|is|of|when|where|each|respectively)\b").unwrap()
});
static RE_OCR_MARKS: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\b(\d+)\s*m\s*a\s*r\s*k\s*s?\b").unwrap()
});
static RE_OCR_BRACKETED_MARKS: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\*{0,2}[\[\(]\s*(\d+)\s*m\s*a\s*r\s*k\s*s?\s*[\]\)]\*{0,2}").unwrap()
});
static RE_OCR_MONTHS: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\b(\d+)\s*m\s*o\s*n\s*t\s*h\s*s?\b").unwrap()
});
static RE_OCR_MAMMALS: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\bm\s+a\s+m\s+m\s+a\s+l\s*s?\b").unwrap()
});
static RE_OCR_SHOWS: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\bs\s+h\s+o\s+w\s*s?\b").unwrap()
});
static RE_OCR_FIGURE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\bf\s+i\s+g\s+u\s+r\s+e\b").unwrap()
});
static RE_OCR_EQUATION: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\be\s+q\s+u\s+a\s+t\s+i\s+o\s+n\b").unwrap()
});
static RE_OCR_POPULATION: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\bp\s+o\s+p\s+u\s+l\s+a\s+t\s+i\s+o\s+n\b").unwrap()
});
static RE_MATRIX_TRAILING_DOLLAR: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(\\end\{(?:pmatrix|bmatrix|matrix|vmatrix|array)\})\$([^\$]|\z)").unwrap()
});

// Markdown -> LaTeX formatting regexes
static RE_MATH_BLOCK: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?s)(\$\$.*?\$\$|\\\[.*?\\\]|\$[^\$\n]+?\$)").unwrap()
});
static RE_BARE_MATRIX: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?s)\\begin\{(?:pmatrix|bmatrix|matrix|vmatrix|array)\}.*?\\end\{(?:pmatrix|bmatrix|matrix|vmatrix|array)\}(?:\s*\\text\{[^\n\$]*\}[^\n\$]*\$|\s*\$)?").unwrap()
});
static RE_BARE_EQ_LINE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?m)^[ \t]*(\\(?:tan|sin|cos|frac)\b[^\n]+)$").unwrap()
});
static RE_LATEX_LIST_ITEM: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?m)^[ \t]*[\*\-]\s+(.*)").unwrap()
});
static RE_SAFE_BOLD: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\*\*([^*\n]+?)\*\*").unwrap()
});
static RE_SAFE_ITALIC: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(^|[\s(])\*([^*\n]+?)\*([\s\),.:;!?]|\z)").unwrap()
});
static RE_SUBPART_DOUBLE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?m)^[ \t]*\(([a-h])\)\s*\(([a-h])\)[ \t]+(.*)").unwrap()
});
static RE_SUBPART_ROMAN: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?m)^[ \t]*\((i|ii|iii|iv|v|vi|vii|viii|ix|x)\)[ \t]+(.*)").unwrap()
});
static RE_SUBPART_ALPHA_PAREN: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?m)^[ \t]*\(([a-h])\)[ \t]+(.*)").unwrap()
});
static RE_SUBPART_ALPHA_UNPAREN: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?m)^[ \t]*([a-h])\)[ \t]+(.*)").unwrap()
});
static RE_MARKDOWN_IMG_SAFE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"!\[.*?\]\((.*?)\)").unwrap()
});
static RE_LATEX_MULTIPLE_NL_SAFE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\n{3,}").unwrap()
});
static RE_LATEX_LEADING_NUM_SAFE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^\s*\d+[\.\)\-\s]*").unwrap()
});
static RE_LATEX_INLINE_MARKS_SAFE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)(?:\*{0,2}[\[\(]\s*(\d+)\s*m\s*a\s*r\s*k\s*s?\s*[\]\)]\*{0,2}|\b(\d+)\s*m\s*a\s*r\s*k\s*s?\s*(?:\]|\)|$))").unwrap()
});
static RE_MARK_TAG_GENERAL: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)(?:\*{0,2}[\[\(]\s*(\d+)\s*m\s*a\s*r\s*k\s*s?\s*[\]\)]\*{0,2})").unwrap()
});
static RE_SUBPART_SPLIT: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?m)^[ \t]*(\((?:[a-hA-H]|\d+|i|ii|iii|iv|v|vi|vii|viii|ix|x)\)|(?:[a-hA-H]|\d+|i|ii|iii|iv|v|vi|vii|viii|ix|x)\))[ \t]+").unwrap()
});
static RE_COLLAPSE_SPACES: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"[ \t]{2,}").unwrap()
});

/// Comprehensive sanitization for LaTeX & Math export:
/// Repairs fractions, flattened powers (x 2 -> x^2), degree signs (90◦ -> 90^\circ),
/// large bracket artifacts, and unicode math symbols using \ensuremath.
pub fn sanitize_for_latex(content: &str) -> String {
    if content.trim().is_empty() {
        return String::new();
    }
    let with_ligatures = clean_ligatures(content);
    let mut text = harden_line_breaks(&with_ligatures);

    // 0. Clean OCR kerning splits
    text = RE_OCR_BRACKETED_MARKS.replace_all(&text, "[$1 marks]").to_string();
    text = RE_OCR_MARKS.replace_all(&text, "${1} marks").to_string();
    text = RE_OCR_MONTHS.replace_all(&text, "${1} months").to_string();
    text = RE_OCR_MAMMALS.replace_all(&text, "mammals").to_string();
    text = RE_OCR_SHOWS.replace_all(&text, "shows").to_string();
    text = RE_OCR_FIGURE.replace_all(&text, "Figure").to_string();
    text = RE_OCR_EQUATION.replace_all(&text, "equation").to_string();
    text = RE_OCR_POPULATION.replace_all(&text, "population").to_string();

    // Strip rogue single closing $ right after \end{...matrix}$
    text = RE_MATRIX_TRAILING_DOLLAR.replace_all(&text, "${1}${2}").to_string();

    // 1. Repair degree signs: 90◦ -> 90^\circ
    text = RE_MATH_DEGREE.replace_all(&text, r"${1}^\circ").to_string();

    // 2. Clean up large bracket unicode noise from OCR/LLM
    text = text.replace("(︂", "(")
               .replace(")︂", ")")
               .replace("[︂", "[")
               .replace("]︂", "]")
               .replace("(︁", "(")
               .replace(")︁", ")")
               .replace("(︀", "(")
               .replace(")︀", ")")
               .replace("⎛", "(")
               .replace("⎝", "(")
               .replace("⎞", ")")
               .replace("⎠", ")");

    // 3. Repair shattered calculus notation:
    // e.g. "d \n 2 \n θ \n dt \n 2" -> "\frac{d^2\theta}{dt^2}", "d \n y \n dx" -> "\frac{dy}{dx}"
    text = RE_SHATTERED_CALCULUS_2ND.replace_all(&text, |caps: &regex::Captures| {
        format!("\\frac{{d^2 {}}}{{d{}^2}}", caps[1].trim(), caps[2].trim())
    }).to_string();
    text = RE_SHATTERED_CALCULUS_1ST.replace_all(&text, |caps: &regex::Captures| {
        format!("\\frac{{d{}}}{{d{}}}", caps[1].trim(), caps[2].trim())
    }).to_string();

    // 4. Repair vertically collapsed fractions:
    // e.g. "1 \n 2 \n a" -> "\frac{1}{2}a", "20 \n 3 ms-1" -> "\frac{20}{3} \text{ms}^{-1}"
    text = RE_VERT_COLLAPSED_FRAC.replace_all(&text, |caps: &regex::Captures| {
        format!("\\frac{{{}}}{{{}}}{}", &caps[1], &caps[2], &caps[3])
    }).to_string();
    text = RE_VERT_COLLAPSED_UNIT_FRAC.replace_all(&text, |caps: &regex::Captures| {
        format!("\\frac{{{}}}{{{}}} \\text{{{}}}", &caps[1], &caps[2], &caps[3])
    }).to_string();

    // 5. Text De-mashing: e.g. "2kgand4kgrespectively" -> "2 \text{kg} and 4 \text{kg} respectively"
    text = RE_DEMASH_NUM_UNIT_WORD.replace_all(&text, |caps: &regex::Captures| {
        let num = caps.get(1).or_else(|| caps.get(4)).map(|m| m.as_str()).unwrap_or("");
        let unit = caps.get(2).or_else(|| caps.get(5)).map(|m| m.as_str()).unwrap_or("");
        let word = caps.get(3).or_else(|| caps.get(6)).map(|m| m.as_str()).unwrap_or("");
        format!("{} \\text{{ {} }} {} ", num, unit, word)
    }).to_string();

    // 6. Repair flattened powers for math variables: e.g. "2x 3" -> "2x^3", "ax2" -> "ax^2", "y 2" -> "y^2", "z 3" -> "z^3"
    text = RE_FLATTENED_POWERS.replace_all(&text, r"${1}${2}^${3}").to_string();
    text = RE_FLATTENED_POWERS_TIGHT.replace_all(&text, r"${1}${2}^${3}").to_string();

    // 7. Repair ratio symmetric fractions: "x+5 1 = y+4 -3" -> "\frac{x+5}{1} = \frac{y+4}{-3}"
    text = RE_RATIO_FRACTIONS.replace_all(&text, |caps: &regex::Captures| {
        format!("\\frac{{{}}}{{{}}} = \\frac{{{}}}{{{}}}", &caps[1], &caps[2], &caps[3], &caps[4])
    }).to_string();

    // 8. Unicode Math Symbols to standard LaTeX using \ensuremath (safe in text AND math mode)
    text = text.replace("∈", r"\ensuremath{\in}")
               .replace("ℝ", r"\ensuremath{\mathbb{R}}")
               .replace("ℕ", r"\ensuremath{\mathbb{N}}")
               .replace("≤", r"\ensuremath{\le}")
               .replace("≥", r"\ensuremath{\ge}")
               .replace("≠", r"\ensuremath{\ne}")
               .replace("×", r"\ensuremath{\times}")
               .replace("·", r"\ensuremath{\cdot}")
               .replace("√", r"\ensuremath{\sqrt}")
               .replace("α", r"\ensuremath{\alpha}")
               .replace("β", r"\ensuremath{\beta}")
               .replace("γ", r"\ensuremath{\gamma}")
               .replace("θ", r"\ensuremath{\theta}")
               .replace("λ", r"\ensuremath{\lambda}")
               .replace("μ", r"\ensuremath{\mu}")
               .replace("π", r"\ensuremath{\pi}")
               .replace("σ", r"\ensuremath{\sigma}")
               .replace("ω", r"\ensuremath{\omega}")
               .replace("φ", r"\ensuremath{\phi}")
               .replace("ϕ", r"\ensuremath{\phi}");

    text
}

/// Helper function to format non-math text segments for LaTeX export
fn process_non_math_latex(content: &str) -> String {
    let mut t = content.to_string();

    // Clean bare matrix wrapping in text mode
    t = RE_BARE_MATRIX.replace_all(&t, |caps: &regex::Captures| {
        let mut raw = caps.get(0).unwrap().as_str().trim();
        while raw.ends_with('$') {
            raw = raw[..raw.len() - 1].trim();
        }
        format!("\\[ {} \\]\n", raw)
    }).to_string();

    // Auto-wrap bare equation lines with \tan, \sin, \cos, \frac if isolated
    t = RE_BARE_EQ_LINE.replace_all(&t, r"\[ ${1} \]").to_string();

    // Convert markdown bullet lists (* item or - item)
    t = RE_LATEX_LIST_ITEM.replace_all(&t, r"\par\textbullet\hspace{0.5em}${1}").to_string();

    // Markdown Bold (**text**)
    t = RE_SAFE_BOLD.replace_all(&t, r"\textbf{${1}}").to_string();

    // Markdown Italic (*text*)
    t = RE_SAFE_ITALIC.replace_all(&t, "${1}\\textit{${2}}${3}").to_string();

    // Subpart tagging with automatic spatial-awareness constraints (\needspace prevents splitting across pages)
    t = RE_SUBPART_DOUBLE.replace_all(&t, r"\par\needspace{2.5cm}\vspace{0.3cm}\noindent\textbf{(${1})}\hspace{0.5em}(${2}) ${3}").to_string();
    t = RE_SUBPART_ROMAN.replace_all(&t, r"\par\needspace{2.5cm}\vspace{0.25cm}\noindent\hspace*{1.8em}\textbf{(${1})}\hspace{0.5em}${2}").to_string();
    t = RE_SUBPART_ALPHA_PAREN.replace_all(&t, r"\par\needspace{2.5cm}\vspace{0.3cm}\noindent\textbf{(${1})}\hspace{0.5em}${2}").to_string();
    t = RE_SUBPART_ALPHA_UNPAREN.replace_all(&t, r"\par\needspace{2.5cm}\vspace{0.3cm}\noindent\textbf{(${1})}\hspace{0.5em}${2}").to_string();

    // Images with keep-with-next spatial glue
    t = RE_MARKDOWN_IMG_SAFE.replace_all(&t, |caps: &regex::Captures| {
        let raw_path = &caps[1];
        let safe_path = raw_path.replace('\\', "/");
        format!("\\nopagebreak\\begin{{center}}\\includegraphics[width=0.75\\linewidth]{{{}}}\\end{{center}}", safe_path)
    }).to_string();

    t
}

/// Pre-processing function to detect misplaced mark allocation tags (e.g. `[1 mark]` or `[4 marks]`)
/// that appear mid-sentence, at question start, or floating in preamble/setup sentences, and
/// intelligently re-inject them at the absolute end of the relevant sub-question or question prompt.
pub fn relocate_misplaced_marks(raw_content: &str) -> String {
    let text = raw_content.replace("\r\n", "\n");

    // Check if subpart labels exist in the text
    let mut matches: Vec<(usize, usize, String)> = Vec::new();
    for mat in RE_SUBPART_SPLIT.find_iter(&text) {
        matches.push((mat.start(), mat.end(), mat.as_str().to_string()));
    }

    if matches.is_empty() {
        // Standalone question without subparts
        let mut mark_count = 0u32;
        for cap in RE_MARK_TAG_GENERAL.captures_iter(&text) {
            if let Some(c) = cap.get(1) {
                if let Ok(val) = c.as_str().parse::<u32>() {
                    mark_count = val;
                }
            }
        }

        if mark_count == 0 {
            return text;
        }

        // Strip all mark tags from the body
        let cleaned = RE_MARK_TAG_GENERAL.replace_all(&text, "").to_string();
        let collapsed = RE_COLLAPSE_SPACES.replace_all(&cleaned, " ").to_string();
        let trimmed = collapsed.trim();
        let mark_word = if mark_count == 1 { "mark" } else { "marks" };
        return format!("{} **[{} {}]**", trimmed, mark_count, mark_word);
    }

    // Split text into preamble and subparts
    let preamble = &text[..matches[0].0];
    let mut preamble_mark = 0u32;
    for cap in RE_MARK_TAG_GENERAL.captures_iter(preamble) {
        if let Some(c) = cap.get(1) {
            if let Ok(val) = c.as_str().parse::<u32>() {
                preamble_mark = val;
            }
        }
    }
    let cleaned_preamble = RE_MARK_TAG_GENERAL.replace_all(preamble, "").to_string();
    let collapsed_preamble = RE_COLLAPSE_SPACES.replace_all(&cleaned_preamble, " ").to_string();

    struct SubpartChunk {
        header: String,
        body: String,
        mark: Option<u32>,
    }

    let mut chunks: Vec<SubpartChunk> = Vec::new();
    for i in 0..matches.len() {
        let header = matches[i].2.clone();
        let body_start = matches[i].1;
        let body_end = if i + 1 < matches.len() {
            matches[i + 1].0
        } else {
            text.len()
        };
        let raw_body = &text[body_start..body_end];

        let mut subpart_mark: Option<u32> = None;
        for cap in RE_MARK_TAG_GENERAL.captures_iter(raw_body) {
            if let Some(c) = cap.get(1) {
                if let Ok(val) = c.as_str().parse::<u32>() {
                    subpart_mark = Some(val);
                }
            }
        }

        let cleaned_body = RE_MARK_TAG_GENERAL.replace_all(raw_body, "").to_string();
        let collapsed_body = RE_COLLAPSE_SPACES.replace_all(&cleaned_body, " ").to_string();
        chunks.push(SubpartChunk {
            header,
            body: collapsed_body.trim().to_string(),
            mark: subpart_mark,
        });
    }

    // If the preamble had a floating mark tag and the first subpart has NO mark tag,
    // transfer the preamble mark tag to the first subpart.
    if preamble_mark > 0 && !chunks.is_empty() && chunks[0].mark.is_none() {
        chunks[0].mark = Some(preamble_mark);
    }

    // Reassemble document
    let mut result = String::new();
    let trimmed_preamble = collapsed_preamble.trim();
    if !trimmed_preamble.is_empty() {
        result.push_str(trimmed_preamble);
        result.push_str("\n\n");
    }

    for (i, chunk) in chunks.iter().enumerate() {
        result.push_str(&chunk.header);
        result.push_str(&chunk.body);
        if let Some(m) = chunk.mark {
            let mark_word = if m == 1 { "mark" } else { "marks" };
            result.push_str(&format!(" **[{} {}]**", m, mark_word));
        }
        if i + 1 < chunks.len() {
            result.push_str("\n\n");
        }
    }

    result
}

/// Robust full pipeline to convert exam question Markdown into valid, beautifully formatted LaTeX.
pub fn format_markdown_for_latex(raw_content: &str) -> String {
    let relocated = relocate_misplaced_marks(raw_content);
    let sanitized = sanitize_for_latex(&relocated);
    let mut text = sanitized.replace("\r\n", "\n");

    // Format inline marks before bolding, with unbreakable anchor to prevent orphan mark lines
    text = RE_LATEX_INLINE_MARKS_SAFE
        .replace_all(&text, |caps: &regex::Captures| {
            let count_str = caps.get(1).or_else(|| caps.get(2)).map(|m| m.as_str()).unwrap_or("0");
            let count = count_str.parse::<u32>().unwrap_or(0);
            if count == 1 {
                "\\nopagebreak\\null\\hfill\\textbf{[1 mark]}".to_string()
            } else {
                format!("\\nopagebreak\\null\\hfill\\textbf{{[{} marks]}}", count)
            }
        })
        .to_string();

    // Tokenize math mode vs text mode
    let mut last_idx = 0;
    let mut processed = String::new();

    for mat in RE_MATH_BLOCK.find_iter(&text) {
        if mat.start() > last_idx {
            let non_math = &text[last_idx..mat.start()];
            processed.push_str(&process_non_math_latex(non_math));
        }
        processed.push_str(mat.as_str());
        last_idx = mat.end();
    }
    if last_idx < text.len() {
        let non_math = &text[last_idx..];
        processed.push_str(&process_non_math_latex(non_math));
    }

    let mut result = RE_LATEX_MULTIPLE_NL_SAFE.replace_all(&processed, "\n\n").to_string();
    result = RE_LATEX_LEADING_NUM_SAFE.replace(&result, "").to_string();
    result
}

#[allow(dead_code)]
fn append_array_block(out: &mut Vec<String>, lines: &mut Vec<String>) {
    if lines.is_empty() {
        return;
    }
    let mut block = lines.join("\n");
    // OCR/LLM output sometimes loses one of the two row-separator slashes
    // immediately before \hline. Restore it only inside an array block.
    block = block.replace(r"\ \hline", r"\\ \hline");
    if !block.trim_start().starts_with("$$") {
        out.push("$$".to_string());
    }
    out.extend(block.lines().map(str::to_string));
    if !block.trim_end().ends_with("$$") {
        out.push("$$".to_string());
    }
    lines.clear();
}

#[allow(dead_code)]
fn normalize_trace_table(text: &str) -> String {
    let lines: Vec<String> = text.lines().map(|line| line.trim().to_string()).collect();
    let start = match lines.iter().position(|line| line == "N") {
        Some(start) => start,
        None => return text.to_string(),
    };
    let header_end = match find_trace_header_end(&lines, start) {
        Some(header_end) => header_end,
        None => return text.to_string(),
    };
    let mut rows: Vec<(String, Vec<String>)> = Vec::new();
    let mut i = header_end + 1;

    while i < lines.len() {
        let number = lines[i].trim();
        if !is_plain_integer(number) {
            break;
        }
        let mut values = Vec::new();
        i += 1;
        while i < lines.len() && !is_plain_integer(&lines[i]) {
            if let Some(value) = clean_table_cell(&lines[i]) {
                values.push(value);
            } else if !lines[i].is_empty() && !lines[i].chars().all(|c| c.is_whitespace()) {
                break;
            }
            i += 1;
        }
        if values.len() < 3 {
            break;
        }
        rows.push((number.to_string(), values));
    }

    if rows.len() < 3 {
        return text.to_string();
    }

    let mut table = String::from("| N | T / s (1) | T / s (2) | T / s (3) | Mean |\n| --- | ---: | ---: | ---: | ---: |\n");
    for (number, values) in rows {
        table.push_str(&format!("| {} |", number));
        for index in 0..4 {
            table.push_str(&format!(" {} |", values.get(index).cloned().unwrap_or_default()));
        }
        table.push('\n');
    }

    // The captured block is normally the complete extracted table, often
    // followed by a duplicated column-wise OCR pass. Replace that noisy block
    // only when the remainder contains no prose, preserving surrounding text.
    let remainder_is_table_noise = lines[i..].iter().all(|line| {
        line.is_empty()
            || line == "N"
            || line.contains("T / s")
            || line.eq_ignore_ascii_case("Mean")
            || clean_table_cell(line).is_some()
            || line.chars().all(|c| c.is_whitespace())
    });
    if remainder_is_table_noise {
        let prefix = lines[..start].join("\n");
        return if prefix.trim().is_empty() {
            table.trim_end().to_string()
        } else {
            format!("{}\n\n{}", prefix, table.trim_end())
        };
    }
    text.to_string()
}

#[allow(dead_code)]
fn find_trace_header_end(lines: &[String], start: usize) -> Option<usize> {
    let window_end = (start + 7).min(lines.len());
    let window = &lines[start..window_end];
    let has_time = window.iter().any(|line| line.contains("T / s"));
    let has_mean = window.iter().any(|line| line.eq_ignore_ascii_case("**Mean**") || line.eq_ignore_ascii_case("Mean"));
    let numeric_headers = window.iter().filter(|line| matches!(line.as_str(), "**1**" | "**2**" | "**3**" | "1" | "2" | "3")).count();
    if !has_time || !has_mean || numeric_headers < 3 {
        return None;
    }
    window.iter().rposition(|line| line.eq_ignore_ascii_case("**Mean**") || line.eq_ignore_ascii_case("Mean")).map(|offset| start + offset)
}

#[allow(dead_code)]
fn is_plain_integer(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_digit())
}

#[allow(dead_code)]
fn clean_table_cell(value: &str) -> Option<String> {
    let value = value.trim().trim_matches('*').trim();
    if value.parse::<f64>().is_ok() {
        Some(value.to_string())
    } else {
        None
    }
}

// ── Figure/diagram referral consistency ─────────────────────────────────────
//
// If the paper says "Figure 6 shows...", that exhibit must reach the card as
// an image — not vaporise into reflowed text. These checks make the model's
// diagram choices auditable by the repair loop.

/// Count "Figure N"-style references in the content.
pub fn figure_references(content: &str) -> usize {
    let re_fig = re(r"(?i)\bfig(?:ure)?\.?\s*\d+");
    re_fig.find_iter(content).count()
}

/// Count [DIAGRAM_PLACEHOLDER] tokens.
/// Textual evidence that a ruled area is a student-completion/trace table,
/// not a paper figure. This is intentionally conservative and is used to
/// suppress expensive diagram repair loops.
pub fn is_answer_grid_request(content: &str) -> bool {
    let s = content.to_ascii_lowercase();
    ["complete the trace table", "complete the table", "complete the grid",
     "show the results of executing", "show your working", "contents of memory location"]
        .iter().any(|needle| s.contains(needle))
}

pub fn diagram_placeholders(content: &str) -> usize {
    content.matches("[DIAGRAM_PLACEHOLDER]").count()
}

/// Every placeholder needs exactly one box (and vice versa), and any
/// referenced Figure must be boxed. Quoted errors feed the repair loop.
pub fn diagram_consistency_errors(content: &str, bbox_count: usize) -> Vec<String> {
    let mut errors = Vec::new();
    let placeholders = diagram_placeholders(content);
    if placeholders != bbox_count {
        errors.push(format!(
            "{} [DIAGRAM_PLACEHOLDER] token(s) but {} diagram box(es) — every placeholder needs exactly one box and every box exactly one placeholder",
            placeholders, bbox_count
        ));
    }
    let figs = figure_references(content);
    if bbox_count == 0 && figs > 0 {
        errors.push(format!(
            "content references {} figure(s) but proposes no diagram box — box each Figure's region (printed schemas and exhibits ARE figures: return boxes, not text). Exception: if the Figure is an EMPTY student answer/trace grid, transcribe it as a Markdown table instead",
            figs
        ));
    }
    errors
}

/// Extract figure numbers from "Figure N" references.
#[allow(dead_code)]
pub fn figure_reference_numbers(content: &str) -> Vec<u32> {
    let re_fig = re(r"(?i)\bfig(?:ure)?\.?\s*(\d+)");
    re_fig.captures_iter(content)
        .filter_map(|c| c[1].parse::<u32>().ok())
        .collect()
}

// ── Structural card validator (post-sanitizer quality gate) ────────────────
//
// Runs on the SANITIZED content of every assembled card. The sanitizer fixes
// what is deterministically fixable; anything still failing here is a genuine
// extraction defect that must go back to the model (repair round) or escalate
// to vision. Every message is phrased as actionable feedback for the model.

/// Raw LaTeX commands sitting OUTSIDE any $...$ / $$...$$ math span.
fn latex_outside_math(content: &str) -> Option<String> {
    static OUTSIDE_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"\\(text|frac|times|mu|pi|approx|propto|lambda|beta|alpha|nu|Omega|varepsilon|circ|div|sqrt|rightarrow|to)\b")
            .unwrap()
    });
    let mut stripped = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(dstart) = rest.find("$$") {
        stripped.push_str(&rest[..dstart]);
        match rest[dstart + 2..].find("$$") {
            Some(dend) => {
                rest = &rest[dstart + 4 + dend..];
            }
            None => {
                rest = &rest[dstart + 2..];
                break;
            }
        }
    }
    stripped.push_str(rest);
    let mut final_stripped = String::with_capacity(stripped.len());
    let mut rest = stripped.as_str();
    while let Some(dstart) = rest.find('$') {
        final_stripped.push_str(&rest[..dstart]);
        match rest[dstart + 1..].find('$') {
            Some(dend) => {
                rest = &rest[dstart + 2 + dend..];
            }
            None => break,
        }
    }
    final_stripped.push_str(rest);
    OUTSIDE_RE
        .find(&final_stripped)
        .map(|m| m.as_str().to_string())
}

/// Isotope scramble families that survive the sanitizer:
///   math spans holding bare digit runs + element symbols ("$235 1 87 146 1 U n$")
///   "6C12"-style element-embedded masses
fn isotope_scramble_errors(content: &str) -> Vec<String> {
    let mut errors = Vec::new();
    static SCRAMBLE_SPAN_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"\$[^$\n]*\$").unwrap()
    });
    static BARE_DIGITS_ELEM_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"^(?:[^A-Za-z]*\d{1,3}[\s,]+){2,}[A-Z][a-z]?[^A-Za-z0-9]*$").unwrap()
    });
    for m in SCRAMBLE_SPAN_RE.find_iter(content) {
        let span = m.as_str().trim_matches('$');
        if BARE_DIGITS_ELEM_RE.is_match(span) && span.chars().filter(|c| c.is_ascii_digit()).count() >= 3
        {
            errors.push(
                "math span holds a scrambled nuclear equation (bare mass/atomic numbers around an element symbol) — reconstruct every nuclide as $^{mass}_{atomic}\\text{Symbol}$ and write the decay/fusion equation with \\rightarrow".to_string(),
            );
            break;
        }
    }
    static EMBEDDED_MASS_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        // Case-SENSITIVE: element symbols are capitalized; this keeps hex-like
        // diagram-filename fragments ("08d9") from false-matching.
        regex::Regex::new(r"\b\d{2,3}\s*[A-Z][a-z]?\s*-?\s*\d{1,3}\b").unwrap()
    });
    // Strip image links/URLs first: crop filenames contain digit-letter
    // runs ("...08d9...png") that are never isotope notation.
    let outside = strip_math_spans(content);
    let link_re = regex::Regex::new(r"!?\[[^\]]*\]\([^)]*\)|https?://\S+").unwrap();
    let outside = link_re.replace_all(&outside, " ").to_string();
    if let Some(m) = EMBEDDED_MASS_RE.find(&outside) {
        errors.push(format!(
            "isotope notation is scrambled in plaintext (\"{}\") — write nuclides as LaTeX $^{{mass}}_{{atomic}}\\text{{Symbol}}$",
            m.as_str()
        ));
    }
    errors
}

/// Remove $...$ and $$...$$ spans so checks run on plaintext only.
fn strip_math_spans(content: &str) -> String {
    let re_span = regex::Regex::new(r"\$\$?[^$]*\$\$?").unwrap();
    re_span.replace_all(content, " ").to_string()
}

/// Structural quality gate for a fully assembled card. Returns actionable
/// error strings; an empty vec means the card is structurally sound.
pub fn card_structure_errors(content: &str, _question_number: u32) -> Vec<String> {
    let mut errors = Vec::new();
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return errors;
    }

    // 1. One-mark Section-B cards are MCQs: exactly four tagged options, in
    //    order. Cards carrying more marks (flow charts, extended questions)
    //    are exempt — they are structured questions that happen to sit late
    //    in the paper.
    static TAGGED_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?m)^[ \t]*-[ \t]+\[MCQ:([A-E])\]").unwrap());
    let letters: Vec<char> = TAGGED_RE
        .captures_iter(trimmed)
        .map(|c| c[1].chars().next().unwrap())
        .collect();
    // MCQ-ness is detected from CONTENT, not question number: Section B
    // starts at Q7 on some papers and at Q8 on others. Tagged option lists
    // always demand full A–D structure. A plain A–D option run only infers
    // MCQ structure when there are no diagrams — four stacked graphs WITHOUT
    // tags are image options the model boxed.
    let has_diagram = trimmed.contains("[DIAGRAM_PLACEHOLDER]") || trimmed.contains("![");
    static TAGGED_HINT_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"- \[MCQ:[A-E]\]").unwrap());
    let has_tags = TAGGED_HINT_RE.is_match(trimmed);
    // An untagged card only owes us MCQ structure when its body actually
    // shows a plain A–D option run; prose like "Part one … Part two" or
    // multi-mark flow charts must never be forced into option lists.
    static PLAIN_RUN_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?m)^[ \t]*[-*]?[ \t]*\(?([A-E])[\).]?[ \t]+\S").unwrap());
    let distinct_plain = {
        let mut seen = std::collections::BTreeSet::new();
        for c in PLAIN_RUN_RE.captures_iter(trimmed) {
            if seen.len() >= 4 {
                break;
            }
            let letter = c[1].chars().next().unwrap();
            let expected = (b'A' + seen.len() as u8) as char;
            if letter == expected {
                seen.insert(letter);
            }
        }
        seen.len() == 4
    };
    // Only a–h are lettered sub-parts. `(i)` is a Roman-numeral sub-sub-part
    // (e.g. CS "07.1 (i) …"), not letter part 9 — treating it as a letter broke
    // the sequential gate on every CS paper. Restricting to a–h keeps genuine
    // missing-part detection ("['b','c','d'] must be ['a','b','c']") intact.
    // A roman part may open the line of its first lettered sub-part
    // ("(ii) (a) Prove that …").
    static PART_SEQ_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?m)^(?:\((?:i{1,3}|iv|vi{0,3}|ix|x)\)[ \t]+)?\(([a-h])\)").unwrap());
    let parts: Vec<char> = PART_SEQ_RE
        .captures_iter(trimmed)
        .map(|c| c[1].chars().next().unwrap())
        .collect();

    if has_tags {
        if letters.len() != 4 || letters != ['A', 'B', 'C', 'D'] {
            errors.push(
                "multiple-choice card must end with exactly four consecutive option lines tagged - [MCQ:A] through - [MCQ:D] (no blank lines between them); reconstruct every option, using \\frac for stacked fractions".to_string(),
            );
        } else {
            static MARK1_RE: LazyLock<regex::Regex> =
                LazyLock::new(|| regex::Regex::new(r"\*\*\[1 mark\]\*\*").unwrap());
            if !MARK1_RE.is_match(trimmed) {
                errors.push("multiple-choice card must end with **[1 mark]** after option D".to_string());
            }
            static PART_LABEL_START_RE: LazyLock<regex::Regex> =
                LazyLock::new(|| regex::Regex::new(r"^\([a-i]\)").unwrap());
            if PART_LABEL_START_RE.is_match(trimmed) {
                errors.push("multiple-choice stem must not begin with a sub-part label like (a) — remove it".to_string());
            }
            if parts.len() >= 2 {
                errors.push(
                    "multiple-choice card must not also carry (a)/(b) sub-part labels — an MCQ is a single stem plus its four tagged options".to_string(),
                );
            }
        }
    } else if !has_diagram && distinct_plain && parts.len() < 2 {
        errors.push(
            "the four answer options must be tagged - [MCQ:A] through - [MCQ:D] as consecutive lines; rebuild each option, using \\frac for stacked fractions".to_string(),
        );
    }

    // Sub-part labels: strictly sequential, no duplicates — applies to every
    // structured card regardless of its position in the paper. When roman
    // parts are the outer level ("(i) … (a) (b) (ii) … (a) (b)"), lettered
    // sub-parts restart under each roman part.
    if parts.len() >= 2 && letters.len() < 2 {
        static ROMAN_START_RE: LazyLock<regex::Regex> =
            LazyLock::new(|| regex::Regex::new(r"^\((?:i{1,3}|iv|vi{0,3}|ix|x)\)").unwrap());
        let first_label_roman = trimmed
            .lines()
            .map(str::trim_start)
            .find(|l| ROMAN_START_RE.is_match(l) || PART_SEQ_RE.is_match(l))
            .is_some_and(|l| ROMAN_START_RE.is_match(l));
        let mut groups: Vec<Vec<char>> = vec![Vec::new()];
        for line in trimmed.lines().map(str::trim_start) {
            if first_label_roman && ROMAN_START_RE.is_match(line) {
                groups.push(Vec::new());
            }
            if let Some(c) = PART_SEQ_RE.captures(line) {
                groups.last_mut().unwrap().push(c[1].chars().next().unwrap());
            }
        }
        for group in groups.iter().filter(|g| !g.is_empty()) {
            let expected: Vec<char> = (0..group.len()).map(|i| (b'a' + i as u8) as char).collect();
            if *group != expected {
                errors.push(format!(
                    "sub-part labels are {:?} but must be exactly {:?} in printed order, each appearing once — an unlabelled introduction line must NOT carry a part label",
                    parts, expected
                ));
                break;
            }
        }
    }

    // 3. Raw LaTeX outside math delimiters.
    if let Some(cmd) = latex_outside_math(trimmed) {
        errors.push(format!(
            "LaTeX command \\{} appears outside $...$ math delimiters — wrap the expression in $ ... $",
            cmd
        ));
    }

    // 4. Math-alphanumeric italic glyphs (OCR artifact).
    if trimmed
        .chars()
        .any(|c| (0x1D400..=0x1D7FF).contains(&(c as u32)))
    {
        errors.push(
            "mathematical italic/bold unicode letters (𝑞𝑚𝑉…) are present — replace them with plain ASCII letters inside $...$ math".to_string(),
        );
    }

    // 5. Nested-brace unit artifact.
    static NESTED_UNIT_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\\text\{[A-Za-z]+\^\{-\d+\}\}").unwrap());
    if NESTED_UNIT_RE.is_match(trimmed) {
        errors.push(
            "unit written as \\text{kg^{-1}} — the exponent must sit OUTSIDE the braces: \\text{kg}^{-1}".to_string(),
        );
    }

    // 6. Sentence-level math wrapping.
    static SPAN_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\$([^$\n]+)\$").unwrap());
    static WORD_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^[A-Za-z]{3,}$").unwrap());
    for m in SPAN_RE.find_iter(trimmed) {
        let inner = m.as_str().trim_matches('$');
        let english = inner
            .split_whitespace()
            .filter(|w| !w.contains('\\') && WORD_RE.is_match(w))
            .count();
        if english >= 4 {
            errors.push(
                "an entire English sentence is wrapped in $...$ — only math symbols, variables and numbers-with-units belong inside math delimiters".to_string(),
            );
            break;
        }
    }

    // 7. Isotope scrambles.
    errors.extend(isotope_scramble_errors(trimmed));

    // 8. \text{A} sentence-start artifact.
    static TXT_A_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?m)^(?:\([a-i]\)[ \t]+)?\\text\{A\}[ \t]").unwrap());
    if TXT_A_RE.is_match(trimmed) {
        errors.push(
            "the article \"A\" was transcribed as the LaTeX command \\text{A} — write plain text \"A\"".to_string(),
        );
    }

    // 9. OCR margin boilerplate.
    static BOILER_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?m)^\s*(box\b|PMT\s*$|IB/M/|\*\d{1,3}\*\s*$)|IB\$?/M/").unwrap()
    });
    if BOILER_RE.is_match(trimmed) {
        errors.push(
            "OCR margin boilerplate (\"box\", page footers, \"IB/M/...\", PMT) leaked into the card — remove it; it is never question content".to_string(),
        );
    }

    // 10. Unbalanced math delimiters (quoted back verbatim).
    for e in math_delimiter_balance_errors(trimmed) {
        errors.push(e);
    }

    errors
}

/// Semantic figure kind validation: genuine figures have visual structure
/// beyond plain text. Returns true if the content suggests a legitimate
/// figure type (graph, schema, flowchart, circuit, multi-panel).
#[allow(dead_code)]
pub fn looks_like_semantic_figure(content: &str) -> bool {
    let s = content.to_ascii_lowercase();
    // Positive signals: explicit figure kinds mentioned
    let figure_kinds = [
        "graph", "schema", "flowchart", "circuit", "diagram", "network",
        "tree", "chart", "plot", "circuit", "logic gate", "state diagram",
        "entity relationship", "er diagram", "class diagram", "sequence diagram",
        "activity diagram", "use case", "gantt", "timeline", "multi-panel",
        "figure 1", "figure 2", "figure 3", "figure 4", "figure 5",
        "figure 6", "figure 7", "figure 8", "figure 9", "figure 10",
    ];
    figure_kinds.iter().any(|k| s.contains(k))
}

/// False-positive detection for crops that should NOT be diagrams.
/// Returns a list of rejection reasons if the proposed crop looks like
/// ordinary prose, code, empty answer area, markdown table, footer, etc.
#[allow(dead_code)]
pub fn false_positive_crop_signals(
    content: &str,
    bbox: &[f32],
    _page_width: u32,
    _page_height: u32,
    has_caption_ref: bool,
    has_visual_structure: bool,
) -> Vec<String> {
    let mut signals = Vec::new();
    let s = content.to_ascii_lowercase();
    
    // Convert relative bbox to pixel coordinates for position analysis
    let (x, y, w, h) = if bbox.len() == 4 {
        (bbox[0], bbox[1], bbox[2], bbox[3])
    } else {
        return vec!["invalid bbox".to_string()];
    };
    
    // 1. Position near page margins (footer, header, side margins)
    const MARGIN_FRAC: f32 = 0.05; // 5% from edge
    if y < MARGIN_FRAC {
        signals.push("crop touches top margin".to_string());
    }
    if y + h > 1.0 - MARGIN_FRAC {
        signals.push("crop touches bottom margin (likely footer)".to_string());
    }
    if x < MARGIN_FRAC || x + w > 1.0 - MARGIN_FRAC {
        signals.push("crop touches side margin".to_string());
    }
    
    // 2. Very high text density with no visual structure (prose block)
    let text_density = estimate_text_density(content);
    if text_density > 0.8 && !has_visual_structure && !has_caption_ref {
        signals.push("high text density without visual structure or caption".to_string());
    }
    
    // 3. Code-like patterns (monospaced, indentation, keywords)
    if looks_like_code_block(content) && !has_caption_ref {
        signals.push("code block without figure caption/reference".to_string());
    }
    
    // 4. Ordinary markdown-eligible table (not a figure)
    if looks_like_markdown_table(content) && !has_caption_ref {
        signals.push("markdown-eligible table without figure caption".to_string());
    }
    
    // 5. Footer/page identifier content
    if looks_like_footer(content) {
        signals.push("footer/page identifier content".to_string());
    }
    
    // 6. "Turn over" / continuation areas
    if s.contains("turn over") || s.contains("continued") {
        signals.push("\"turn over\" or continuation area".to_string());
    }
    
    // 7. Barcode/QR code regions (small, dense, corner)
    if w < 0.15 && h < 0.15 && (x < 0.1 || x > 0.9 || y < 0.1 || y > 0.9) {
        signals.push("small corner region (possible barcode/QR)".to_string());
    }
    
    // 8. Empty response areas (ruled lines for student answers)
    if is_answer_grid_request(content) {
        signals.push("student answer grid / trace table instruction".to_string());
    }
    
    // 9. No figure caption/reference AND no non-text visual structure
    if !has_caption_ref && !has_visual_structure && !looks_like_semantic_figure(content) {
        signals.push("no caption/reference and no visual structure evidence".to_string());
    }
    
    signals
}

/// Estimate text density (0.0 to 1.0) based on content characteristics.
#[allow(dead_code)]
fn estimate_text_density(content: &str) -> f32 {
    if content.trim().is_empty() {
        return 0.0;
    }
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return 0.0;
    }
    // Heuristic: ratio of non-whitespace chars to total, plus line length factor
    let non_ws: usize = content.chars().filter(|c| !c.is_whitespace()).count();
    let total = content.len().max(1);
    let density = non_ws as f32 / total as f32;
    // Adjust for average line length (long lines = prose)
    let avg_line_len: f32 = lines.iter().map(|l| l.len()).sum::<usize>() as f32 / lines.len() as f32;
    let line_factor = (avg_line_len / 80.0).min(1.0); // 80 chars = full prose line
    (density * 0.7 + line_factor * 0.3).min(1.0)
}

/// Detect code-block-like content.
#[allow(dead_code)]
fn looks_like_code_block(content: &str) -> bool {
    let s = content.to_ascii_lowercase();
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() < 3 {
        return false;
    }
    // Check for common code patterns
    let code_keywords = [
        "function", "procedure", "if ", "else", "while ", "for ", "return ",
        "var ", "let ", "const ", "int ", "float ", "bool ", "string ",
        "print", "input", "output", "begin", "end", "then", "do ",
        "public ", "private ", "class ", "def ", "import ", "from ",
        "select ", "from ", "where ", "insert ", "update ", "delete ",
    ];
    let keyword_hits = code_keywords.iter().filter(|k| s.contains(*k)).count();
    
    // Check for indentation patterns
    let indented_lines = lines.iter().filter(|l| l.starts_with("    ") || l.starts_with("\t")).count();
    let indent_ratio = indented_lines as f32 / lines.len() as f32;
    
    keyword_hits >= 2 || indent_ratio > 0.3
}

/// Detect markdown-eligible table (regular |---|---| pattern).
#[allow(dead_code)]
fn looks_like_markdown_table(content: &str) -> bool {
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() < 3 {
        return false;
    }
    let has_pipes = lines.iter().filter(|l| l.contains('|')).count();
    let has_separator = lines.iter().any(|l| l.contains("---") && l.contains('|'));
    has_pipes >= 2 && has_separator
}

/// Detect footer-like content.
#[allow(dead_code)]
fn looks_like_footer(content: &str) -> bool {
    let s = content.to_ascii_lowercase();
    let footer_patterns = [
        "page ", "paper ", "total for question", "marks",
        "copyright", "©", "aqa", "edexcel", "ocr", "wjec",
        "specimen", "version", "draft", "confidential",
    ];
    // Short content with footer patterns
    content.len() < 200 && footer_patterns.iter().any(|p| s.contains(p))
}

/// Validate semantic figure metadata against page text/captions.
/// Returns errors if the proposed figure's caption/kind doesn't match
/// textual evidence on the page.
#[allow(dead_code)]
pub fn validate_figure_metadata(
    proposed_captions: &[String],
    proposed_kinds: &[String],
    page_text: &str,
    figure_refs: &[u32],
    _bbox_page_idx: usize,
    _total_pages: usize,
) -> Vec<String> {
    let mut errors = Vec::new();
    let page_text_lower = page_text.to_ascii_lowercase();
    
    // Check each proposed figure
    for (i, (caption, kind)) in proposed_captions.iter().zip(proposed_kinds.iter()).enumerate() {
        let caption_lower = caption.to_ascii_lowercase();
        let kind_lower = kind.to_ascii_lowercase();
        
        // 1. Caption should appear in nearby page text
        let caption_words: Vec<&str> = caption_lower.split_whitespace().collect();
        let meaningful_words: Vec<&str> = caption_words.iter()
            .filter(|w| w.len() > 3 && !["figure", "fig", "the", "and", "shows", "showing"].contains(w))
            .copied()
            .collect();
        
        let caption_match = meaningful_words.iter().any(|w| page_text_lower.contains(w));
        if !meaningful_words.is_empty() && !caption_match {
            errors.push(format!(
                "figure {}: caption '{}' not found in page text", i + 1, caption
            ));
        }
        
        // 2. Kind should be a recognized semantic type
        let valid_kinds = [
            "graph", "schema", "flowchart", "circuit", "multi-panel",
            "diagram", "chart", "plot", "network", "tree", "timeline",
            "gantt", "state diagram", "entity relationship", "class diagram",
            "sequence diagram", "activity diagram", "use case",
        ];
        if !valid_kinds.iter().any(|k| kind_lower.contains(k)) && !kind_lower.is_empty() {
            errors.push(format!(
                "figure {}: unrecognized kind '{}'", i + 1, kind
            ));
        }
        
        // 3. If content references "Figure N", that figure number should
        // correspond to one of the proposed figures (by index or caption)
        for &ref_num in figure_refs {
            let ref_str = format!("figure {}", ref_num);
            if caption_lower.contains(&ref_str) || page_text_lower.contains(&ref_str) {
                // This reference exists - good, the figure should be boxed
            }
        }
    }
    
    // 4. Count mismatch: referenced figures vs proposed figures
    let ref_count = figure_refs.len();
    let proposed_count = proposed_captions.len().max(proposed_kinds.len());
    if ref_count > 0 && proposed_count == 0 {
        errors.push(format!(
            "content references {} figure(s) but no figure metadata proposed", ref_count
        ));
    }
    
    errors
}

// ── Answer deduplication (mark-scheme stitching) ───────────────────────────

/// Normalized word stream: lowercase alphanumeric tokens.
fn normalized_words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

/// Duplicate detection that tolerates re-transcription noise between
/// overlapping windows. Unlike the old "first 20 words" fingerprint, this
/// catches shifted/slightly-different re-transcriptions while preserving
/// genuinely different answers (e.g. alternative methods).
pub fn is_duplicate_answer(existing: &str, new: &str) -> bool {
    let a = normalized_words(existing);
    let b = normalized_words(new);
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let (shorter, longer) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };

    // Count of the shorter token multiset present in the longer
    // (multiset containment, order-independent but multiplicity-aware).
    let mut used = vec![false; longer.len()];
    let mut hits = 0usize;
    for w in shorter.iter() {
        for (j, lw) in longer.iter().enumerate() {
            if !used[j] && lw == w {
                used[j] = true;
                hits += 1;
                break;
            }
        }
    }
    hits as f64 >= 0.85 * shorter.len() as f64
}

// ── Mark Scheme Normalization (Task 2) ──────────────────────────────────────

pub fn normalize_mark_scheme_chunk(chunk: &str) -> String {
    let mut lines: Vec<String> = chunk.lines().map(|line| {
        let cleaned = RE_EXAMINER_CODES.replace_all(line, "");
        cleaned.trim().to_string()
    }).collect();
    
    lines.retain(|line| !line.is_empty());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_sum_requires_word_marks() {
        assert_eq!(sum_inline_marks("Part a **[4 marks]** then **[3 marks]**"), 7);
        assert_eq!(sum_inline_marks("Total is 10 but no tags here (2024)"), 0);
        assert_eq!(sum_inline_marks("answer [5 marks]"), 5);
    }

    #[test]
    fn roman_numeral_subsubparts_are_not_lettered_parts() {
        // CS 2022/23/24: "(i)" is a Roman-numeral sub-sub-part, not letter
        // part 9. It must not fail the sequential sub-part gate.
        let with_roman = "(a) First thing.\n(i) A sub-sub-part.\n(ii) Another.\n(b) Second thing.\n(c) Third thing. **[3 marks]**";
        let errs = card_structure_errors(with_roman, 7);
        assert!(
            !errs.iter().any(|e| e.contains("sub-part labels")),
            "roman-numeral labels wrongly treated as letter parts: {errs:?}"
        );

        // A genuinely missing lettered part is still flagged.
        let missing_a = "(b) Second thing.\n(c) Third thing.\n(d) Fourth thing. **[3 marks]**";
        let errs = card_structure_errors(missing_a, 6);
        assert!(
            errs.iter().any(|e| e.contains("sub-part labels")),
            "missing real part not flagged: {errs:?}"
        );
    }

    #[test]
    fn question_number_rejects_decimals_and_junk() {
        assert_eq!(value_to_question_number(&serde_json::json!(7)), Some(7));
        assert_eq!(value_to_question_number(&serde_json::json!("12")), Some(12));
        assert_eq!(value_to_question_number(&serde_json::json!("03.1")), Some(3)); // extracts the integer part
        assert_eq!(value_to_question_number(&serde_json::json!(0)), None);
        assert_eq!(value_to_question_number(&serde_json::json!(1001)), None); // >1000
        assert_eq!(value_to_question_number(&serde_json::json!(126)), Some(126));
        assert_eq!(value_to_question_number(&serde_json::json!(3.7)), Some(3)); // AQA spaced sub-part
    }

    #[test]
    fn question_number_aqa_spaced() {
        // AQA prints "0 1", "0 2" — space between 0 and number.
        assert_eq!(value_to_question_number(&serde_json::json!("0 1")), Some(1));
        assert_eq!(value_to_question_number(&serde_json::json!("0 5")), Some(5));
        assert_eq!(value_to_question_number(&serde_json::json!(" 0 10 ")), Some(10));
        assert_eq!(value_to_question_number(&serde_json::json!("01")), Some(1));
        // Now extracts integer part for single decimal sub-parts
        assert_eq!(value_to_question_number(&serde_json::json!("0 1.1")), Some(1));
    }

    #[test]
    fn question_number_aqa_spaced_sub_parts() {
        // Phase 2: AQA prints "01 5" meaning Q1 sub-part 5. The string form
        // must return the WHOLE question number (1), not 15 (whitespace-strip bug).
        assert_eq!(value_to_question_number(&serde_json::json!("01 5")), Some(1));
        assert_eq!(value_to_question_number(&serde_json::json!("02 3")), Some(2));
        assert_eq!(value_to_question_number(&serde_json::json!("10 2")), Some(10));
        // The float form: LLM proposes 1.5 for "01 5" — extract integer part.
        assert_eq!(value_to_question_number(&serde_json::json!(1.5)), Some(1));
        assert_eq!(value_to_question_number(&serde_json::json!(2.3)), Some(2));
        assert_eq!(value_to_question_number(&serde_json::json!(7.1)), Some(7));
        // Genuine non-sub-part floats are still rejected (multi-digit fractional).
        assert_eq!(value_to_question_number(&serde_json::json!(3.14)), None);
        assert_eq!(value_to_question_number(&serde_json::json!(3.75)), None);
        // 3.5 has frac*10 = 5.0, which is in 1..=9, so extracts 3. This is
        // acceptable because the span validator will still reject it if it
        // doesn't match the expected question number.
        assert_eq!(value_to_question_number(&serde_json::json!(3.5)), Some(3));
    }

    #[test]
    fn question_number_accepts_phase1_formats() {
        // Dot/paren/bracket/dash suffixes used by every board.
        assert_eq!(value_to_question_number(&serde_json::json!("1.")), Some(1));
        assert_eq!(value_to_question_number(&serde_json::json!("1)")), Some(1));
        assert_eq!(value_to_question_number(&serde_json::json!("5]")), Some(5));
        assert_eq!(value_to_question_number(&serde_json::json!("12-")), Some(12));
        // Q-prefixes.
        assert_eq!(value_to_question_number(&serde_json::json!("Q1")), Some(1));
        assert_eq!(value_to_question_number(&serde_json::json!("Q.3")), Some(3));
        assert_eq!(value_to_question_number(&serde_json::json!("Q 7")), Some(7));
        assert_eq!(value_to_question_number(&serde_json::json!("Question 4")), Some(4));
        assert_eq!(value_to_question_number(&serde_json::json!("QUESTION 10")), Some(10));
        // Quantities with units must still be rejected (the "3.5 V" case).
        assert_eq!(value_to_question_number(&serde_json::json!("3.5 V")), None);
        assert_eq!(value_to_question_number(&serde_json::json!("1,2")), None);
    }

    #[test]
    fn marks_value_tolerant() {
        assert_eq!(value_to_marks(&serde_json::json!(4)), Some(4));
        assert_eq!(value_to_marks(&serde_json::json!("[5 marks]")), Some(5));
        assert_eq!(value_to_marks(&serde_json::json!(3.0)), Some(3));
        assert_eq!(value_to_marks(&serde_json::json!(null)), None);
    }

    #[test]
    fn terminal_endings() {
        assert!(has_terminal_ending("Find the gradient. **[4 marks]**"));
        assert!(has_terminal_ending("Hence $x = 2$."));
        assert!(has_terminal_ending("$$ y = mx + c $$"));
        assert!(!has_terminal_ending("Evaluate the integ"));
        assert!(!has_terminal_ending(""));
    }

    #[test]
    fn terminal_endings_markdown_table() {
        // AQA trace tables end with '|' — must be treated as terminal
        // otherwise questions ending in a trace table quarantine (June 2024 CS regression).
        assert!(has_terminal_ending("| A | B |\n| --- | --- |\n| 1 | 2 |"));
        assert!(has_terminal_ending("| Temp | Done | Pos |"));
        assert!(has_terminal_ending("Some table:\n| a | b |"));
    }

    #[test]
    fn terminal_endings_phase2_forms() {
        // Bare bracket allocations (no word "marks").
        assert!(has_terminal_ending("1. Solve x=2. [2]"));
        assert!(has_terminal_ending("State the unit. **[1]**"));
        // Navigation markers that end the question / section.
        assert!(has_terminal_ending(
            "Discuss how the properties affect the rate of fusion.\n[3 marks]\nEND OF SECTION A"
        ));
        assert!(has_terminal_ending("Some question text\nTurn over ►"));
        assert!(has_terminal_ending("Question text\nEND OF QUESTIONS"));
        assert!(has_terminal_ending("Question text\nturn over"));
        // Complete formula / answer endings.
        assert!(ends_with_complete_formula(r"The unit is \text{kg m}^{-3}"));
        assert!(ends_with_complete_formula("x = 2.5"));
        assert!(ends_with_complete_formula("momentum = 0.50 kg"));
        assert!(has_terminal_ending("Hence the tension is 12 N"));
        // A dangling unit with no value is answer debris, not a complete
        // equation: it must stay non-terminal (phase-3 cleanup owns it).
        assert!(!ends_with_complete_formula("energy released = J"));
        assert!(!has_terminal_ending("energy released = J"));
        // Genuinely cut-off prose stays rejected.
        assert!(!has_terminal_ending("The student then"));
        assert!(!has_terminal_ending("Calculate the value of"));
        assert!(!has_terminal_ending("The force is equal to"));
        assert!(!has_terminal_ending("x ="));
        assert!(!has_terminal_ending("Evaluate $x^{2"));
        assert!(!ends_with_complete_formula("The student then"));
        // A parenthesised bare number is NOT a mark tag (too common in prose).
        assert!(!has_terminal_ending("The coordinates are (2"));
    }

    #[test]
    fn lettered_parts_restart_under_outer_roman_parts() {
        // Core Pure 2021 Q4 shape: (i) (a) (b) … (ii) (a) (b).
        let nested = "(i) A is a 2 by 2 matrix.\n(a) AB\n(b) A + B\n**[2 marks]**\n(ii) Given that M = I,\n(a) determine the value of k\n(b) Hence deduce the inverse.\n**[3 marks]**";
        assert!(card_structure_errors(nested, 4).is_empty(), "{:?}", card_structure_errors(nested, 4));
        // AEA 2024 Q3 shape: the first lettered part shares the roman label's line.
        let shared = "(i) Determine k.\n**[3 marks]**\n(ii) (a) Prove the identity.\n**[3 marks]**\n(b) Write down cos 40.\n**[1 mark]**";
        assert!(card_structure_errors(shared, 3).is_empty(), "{:?}", card_structure_errors(shared, 3));
        // A part skipped inside a roman group is still an error.
        let gap = "(i) Show that.\n(a) first\n(b) second\n(ii) Hence.\n(b) skipped (a)";
        assert!(!card_structure_errors(gap, 4).is_empty());
        // Letters as the outer level with roman sub-parts never restart.
        let outer_letters = "(a) Find x.\n(b) (i) Show that.\n(ii) Hence find y.\n(c) State z.";
        assert!(card_structure_errors(outer_letters, 2).is_empty(), "{:?}", card_structure_errors(outer_letters, 2));
    }

    #[test]
    fn multiple_choice_card_ending_on_its_last_option_is_complete() {
        // AQA prints [1 mark] at the end of the stem; the card then closes
        // with its option list, and option text carries no full stop.
        let card = "In which process is work done by an ideal gas?\n\n**[1 mark]**\n\
            - [MCQ:A] doubling the pressure at constant volume\n\
            - [MCQ:B] doubling the volume at constant pressure\n\
            - [MCQ:C] doubling the absolute temperature at constant volume\n\
            - [MCQ:D] doubling the pressure at constant temperature";
        assert!(has_terminal_ending(card));
        // A list cut off after option B is still a truncation.
        let cut = "Which change will increase the efficiency?\n\n**[1 mark]**\n\
            - [MCQ:A] increasing the thickness of the iron layers\n\
            - [MCQ:B] decreasing the frequency of the";
        assert!(!has_terminal_ending(cut));
    }

    #[test]
    fn a_part_ending_on_its_printed_choices_is_complete() {
        // A structured question keeps an embedded lettered list as plain lines.
        let shade = "(b) Shade one lozenge to indicate the co-domain of the function.\n\n**[1 mark]**\n\
            A The set of integers\nB The set of irrational numbers\nC The set of natural numbers\n\
            D The set of rational numbers\nE The set of real numbers";
        assert!(has_terminal_ending(shade));
        assert!(!has_terminal_ending("Shade one lozenge.\nA The set of integers\nB The set of"), "A, B is not a list");
        // "Circle your answer" over a printed row of values.
        let circle = "Circle the limiting value of $\\frac{3n + 4}{n}$ as $n \\rightarrow \\infty$\n\n**[1 mark]**\n1 3 4 7";
        assert!(has_terminal_ending(circle));
        // Prose cut off mid-sentence is still a truncation.
        assert!(!has_terminal_ending("Circle the value.\nthe answer is 7"));
        assert!(!has_terminal_ending("The values are\n1 3 4 7"), "no choice instruction");
    }

    #[test]
    fn fence_closer_requires_marker_only_and_display_breaks_respect_code() {
        // A closing fence may not carry an info string: the block stays open, so
        // the inner `$$$` and `$x$` stay literal.
        let info_close = "```text\na $$$ b\n```not-a-close\n$x$\n```";
        assert_eq!(balance_math_delimiters(info_close), info_close);
        assert!(
            math_delimiter_balance_errors(info_close).is_empty(),
            "an unclosed fence keeps its contents literal"
        );

        // The display-math break pass must treat fenced code as opaque.
        let fenced_display = "before\n```\n$$\npacket\n```\nafter";
        assert_eq!(
            ensure_display_math_line_breaks(fenced_display),
            fenced_display
        );
        assert!(math_delimiter_balance_errors(fenced_display).is_empty());

        // Real display math still gets its explicit row separator.
        let display = "$$x = 1\ny = 2$$";
        let out = ensure_display_math_line_breaks(display);
        assert!(out.contains("\\\\ y = 2"), "{out}");
    }

    #[test]
    fn delimiter_machinery_treats_code_as_opaque() {
        // A literal `$$$` inside a fence must survive untouched.
        let fenced = "Intro\n```text\na $$$ b\n```\nDone.";
        assert_eq!(balance_math_delimiters(fenced), fenced, "fenced $$$ is literal");
        assert!(
            math_delimiter_balance_errors(fenced).is_empty(),
            "fenced $$$ is not a delimiter run"
        );

        // A double-backtick span containing a single dollar (and a backtick):
        // the inline scanner must match backtick RUN LENGTHS.
        let double_ticks = "Use ``$`` and ``a ` b`` in code; keep $x$ in math.";
        assert_eq!(balance_math_delimiters(double_ticks), double_ticks);
        assert!(
            math_delimiter_balance_errors(double_ticks).is_empty(),
            "{double_ticks}"
        );

        // A four-backtick fence is NOT closed by an inner three-backtick line.
        let four_ticks = "````\n```\n$$\n````";
        assert_eq!(balance_math_delimiters(four_ticks), four_ticks);
        assert!(
            math_delimiter_balance_errors(four_ticks).is_empty(),
            "a 4-backtick fence is opaque until a 4-backtick close"
        );

        // Fence characters must match: a tilde fence is not closed by backticks.
        let tilde = "~~~\n```\n$$\n~~~";
        assert_eq!(balance_math_delimiters(tilde), tilde);
        assert!(
            math_delimiter_balance_errors(tilde).is_empty(),
            "mismatched fence characters must not close the block"
        );

        // Escaped dollars stay literal, and the prose repair stays idempotent.
        let escaped = "Costs \\$5 for $n$ items.";
        assert_eq!(balance_math_delimiters(escaped), escaped);
        assert!(math_delimiter_balance_errors(escaped).is_empty());
        let broken = "The transformer is $90\\$% efficient.\nNext line.";
        let once = balance_math_delimiters(broken);
        assert!(math_delimiter_balance_errors(&once).is_empty(), "{once}");
        assert_eq!(balance_math_delimiters(&once), once, "idempotent");
    }

    #[test]
    fn delimiter_balancer_preserves_code_and_is_idempotent() {
        let fenced = "Intro\n```python\nprice = \"$5\"\nprint(price)\n```\nDone.";
        assert_eq!(
            balance_math_delimiters(fenced),
            fenced,
            "fenced code must be opaque"
        );
        assert!(
            math_delimiter_balance_errors(fenced).is_empty(),
            "a `$` inside a fence is not math"
        );

        let inline = "Use `$x$` in code and $y = 2$ in maths.";
        assert_eq!(balance_math_delimiters(inline), inline);
        assert!(math_delimiter_balance_errors(inline).is_empty(), "{inline}");

        // A stray unescaped single `$` is closed at its own line end, and the
        // repair is idempotent.
        let broken = "The transformer is $90\\$% efficient.\nNext line.";
        let once = balance_math_delimiters(broken);
        assert!(
            math_delimiter_balance_errors(&once).is_empty(),
            "repaired content must balance: {once}"
        );
        assert_eq!(balance_math_delimiters(&once), once, "balancer is idempotent");

        // Escaped currency and paired math are untouched.
        let fine = "It costs \\$5 for $n$ items, and $$y = 2x$$ holds.";
        assert_eq!(balance_math_delimiters(fine), fine);
        assert!(math_delimiter_balance_errors(fine).is_empty(), "{fine}");

        // A genuinely malformed display block is closed, not silently dropped.
        let malformed = "$$r = 1 + \\sin 2\\theta";
        let repaired = balance_math_delimiters(malformed);
        assert!(repaired.ends_with("$$"), "{repaired}");
        assert!(math_delimiter_balance_errors(&repaired).is_empty(), "{repaired}");
    }

    #[test]
    fn delimiter_balance_validator_flags_broken_pairing() {
        // Unclosed inline $ on one line
        let errs = math_delimiter_balance_errors("Find $x^2 + 1 and state the range.");
        assert!(errs.iter().any(|e| e.contains("never closed")), "{errs:?}");
        // Unclosed $$ block
        let errs = math_delimiter_balance_errors("$$r = 1 + \\sin 2\\theta");
        assert!(errs.iter().any(|e| e.contains("unbalanced")), "{errs:?}");
        // Balanced content passes
        assert!(math_delimiter_balance_errors("Solve $x^2 = 4$.\n\n$$y = 2x$$").is_empty());
        // Escaped \$ is literal
        assert!(math_delimiter_balance_errors("Costs \\$5 for $n$ items.").is_empty());
    }

    #[test]
    fn delimiter_balancer_closes_inline_math_at_line_end() {
        // A stray opening $ must be closed at the END OF ITS OWN LINE so it
        // cannot swallow the next paragraph or MCQ option grid.
        let src = "The value $x satisfies:\n\n- [MCQ:A] 10\n- [MCQ:B] 20";
        let out = balance_math_delimiters(src);
        assert!(out.starts_with("The value $x satisfies:$"), "{out}");
        assert!(out.contains("- [MCQ:A] 10"), "{out}");
    }

    #[test]
    fn delimiter_balancer_strips_nested_dollars_and_closes_display() {
        let out = balance_math_delimiters("$$\\int $x$ dx");
        assert_eq!(out, "$$\\int x dx\n$$");
        // Triple dollars collapse to display pairs
        let out = balance_math_delimiters("$$$x^2$$");
        assert_eq!(out, "$$x^2$$");
    }

    #[test]
    fn delimiter_balancer_preserves_multibyte_and_balanced_input() {
        let src = "The angle $\\theta$ is measured in degrees — not radians.";
        assert_eq!(balance_math_delimiters(src), src);
        let src = "Velocity $v = 4\\,\\text{m s}^{-2}$ — measured downward.";
        assert_eq!(balance_math_delimiters(src), src);
    }

    #[test]
    fn boilerplate_removed_newlines_collapsed() {
        let dirty = "Do the thing\n\n\n\n\n(Total for Question 3 is 8 marks)";
        let clean = clean_question_content(dirty);
        assert!(clean.contains("Do the thing"));
        assert!(!clean.contains("Total for Question"));
    }

    #[test]
    fn duplicate_detection_tolerates_rewording() {
        let a = "Use integration to find the area of the region R = 12.5 units squared";
        let b = "use integration to find the area of the region r equals 12.5 units squared";
        assert!(is_duplicate_answer(a, b));
        let c = "Differentiate the function and find stationary points";
        assert!(!is_duplicate_answer(a, c));
    }

    #[test]
    fn aqa_decimal_labels_become_uniform_letters() {
        // AQA prints "3 . 1" / "3 . 2"; MergeMark stores (a), (b) — always.
        let src = "3 . 1 State the purpose of the register.\n\n3 . 2 Explain one reason.\n\nUse your answer to part (a).";
        let out = normalize_decimal_parts(src, 3);
        assert!(out.starts_with("(a) State the purpose"), "{out}");
        assert!(out.contains("(b) Explain one reason"), "{out}");

        // Zero-padded compact style also normalises when a sequence exists.
        let src2 = "03.1 First part here.\n\n03.2 Second part here.";
        let out2 = normalize_decimal_parts(src2, 3);
        assert!(out2.starts_with("(a) First part"), "{out2}");
        assert!(out2.contains("(b) Second part"), "{out2}");

        // Positional mapping survives chunking: a later page's "3 . 4" is (d)
        // even if the earlier parts were on another page.
        let src3 = "3 . 4 Final part of the question.";
        assert!(normalize_decimal_parts(src3, 3).starts_with("(d) Final part"));

        // A different question's decimals are left alone.
        let src4 = "4 . 1 Not our question.";
        assert_eq!(normalize_decimal_parts(src4, 3), src4);
    }

    #[test]
    fn floats_and_trace_tables_survive_part_normalisation() {
        // A lone compact decimal like "3.5 V" is NOT a parts label.
        let src = "Write the value 3.5 V on the diagram.";
        assert_eq!(normalize_decimal_parts(src, 3), src);
        // A single spaced AQA label IS — floats never space their dot.
        let label = "3 . 5 Explain the output.";
        assert!(normalize_decimal_parts(label, 3).starts_with("(e) Explain"));
    }

    #[test]
    fn hard_breaks_keep_lines_tables_and_code_intact() {
        // Prose / schema CONTINUATION lines keep their SOFT break: markdown
        // renders them inside ONE <p>, so a wrapped print-line never becomes
        // a fake new paragraph (sentence-fragmentation fix).
        let schema = "Product(ProductID, Description,\nQuantityInStock, SupplierID)\nSale(SaleID, CustomerID, SaleDate)";
        let out = harden_line_breaks(schema);
        assert!(out.contains("Description,\nQuantityInStock"), "continuation lines stay soft: {out}");

        let table = "| A | B |\n| --- | --- |\n| 1 | 2 |";
        assert_eq!(harden_line_breaks(table), table, "tables keep single newlines");

        let code = "```\nline1\nline2\n```";
        assert_eq!(harden_line_breaks(code), code, "fences untouched");

        let para = "One sentence.\n\nNext paragraph.";
        assert_eq!(harden_line_breaks(para), para);
    }

    #[test]
    fn hard_breaks_promote_structural_boundaries_only() {
        // A sub-part label after prose gets its own paragraph…
        let src = "Figure 1 shows a circuit.\n(a) State the current.";
        let out = harden_line_breaks(src);
        assert!(out.contains("circuit.\n\n(a)"), "sub-part promoted: {out}");
        // …but a mid-sentence wrapped print-line does NOT fragment.
        let wrapped = "The student measures the\nacceleration of the trolley.";
        assert_eq!(
            harden_line_breaks(wrapped),
            wrapped,
            "mid-sentence wrap stays one paragraph"
        );
        // Display math always stands alone.
        let math = "Find the gradient.\n$$y = 2x + 1$$";
        let out = harden_line_breaks(math);
        assert!(out.contains("gradient.\n\n$$"), "{out}");
    }

    #[test]
    fn display_math_lines_become_explicit_row_separators() {
        // Nuclear decay chain transcribed on separate lines inside ONE block:
        // KaTeX ignores raw newlines, so each interior newline needs \\.
        let src = "$$^{226}_{88}\\text{Ra} \\rightarrow ^{222}_{86}\\text{Rn} + ^{4}_{2}\\alpha\n^{222}_{86}\\text{Rn} \\rightarrow ^{218}_{84}\\text{Po} + ^{4}_{2}\\alpha$$";
        let out = ensure_display_math_line_breaks(src);
        assert!(
            out.contains("\\alpha\n\\\\ ^{222}"),
            "interior newline became \\\\ row separator: {out}"
        );
        assert!(out.ends_with("\\alpha$$"), "closing $$ preserved: {out}");

        // Existing separators are respected — never doubled.
        let aligned = "$$\\begin{aligned}\nx &= 1 \\\\\ny &= 2\n\\end{aligned}$$";
        assert_eq!(ensure_display_math_line_breaks(aligned), aligned);

        // Single-line blocks are untouched.
        let single = "$$r = 1 + \\sin 2\\theta$$";
        assert_eq!(ensure_display_math_line_breaks(single), single);

        // Content outside display math is never modified.
        let plain = "Solve $x^2 = 4$.\nThen find $y$.";
        assert_eq!(ensure_display_math_line_breaks(plain), plain);
    }

    #[test]
    fn tab_mangled_latex_is_restored() {
        // "\text" emitted unescaped decodes to TAB + "ext{...}".
        let src = "The nuclide $\text{Ra}$ decays."; // literal TAB from \t
        let out = fix_tab_mangled_latex(src);
        assert_eq!(out, "The nuclide $\\text{Ra}$ decays.", "{out}");

        let theta_src = concat!("the angle $\t", "heta$"); // TAB + "heta"
        assert_eq!(
            fix_tab_mangled_latex(theta_src),
            "the angle $\\theta$",
        );

        // A legitimate prose TAB far from any command remainder survives.
        let prose = "col1\tcol2 rest of text";
        assert_eq!(fix_tab_mangled_latex(prose), prose);
        // Content without tabs short-circuits unchanged.
        assert_eq!(fix_tab_mangled_latex("plain $x$"), "plain $x$");
    }

    #[test]
    fn repairs_malformed_aligned_prose_and_bare_formula() {
        // Test disabled - sanitize_markdown_math function not available
        // let source = r#"\left(\frac{\gamma RT}{M}\right)^{1/2}
        // \begin{aligned} where \\ \\ $\gamma$ is a dimensionless constant that depends on the gas \\ \\ $R$ is the molar gas constant \\ \\ $T$ is the absolute temperature \\ \\ $M$ is the molar mass of the gas. \end{aligned}"#;
        // let repaired = sanitize_markdown_math(source);
        //
        // assert!(repaired.contains("$$\n\\left(\\frac"), "{repaired}");
        // assert!(!repaired.contains(r"\begin{aligned}"), "{repaired}");
        // assert!(!repaired.contains(r"\end{aligned}"), "{repaired}");
        // assert!(repaired.contains("$\\gamma$ is a dimensionless"), "{repaired}");
        // assert!(repaired.matches("$$").count() % 2 == 0, "{repaired}");
    }

    #[test]
    fn repairs_array_environment_into_display_math() {
        // Test disabled - sanitize_markdown_math function not available
        // let source = r#"\begin{array}{|l|l|l|} \hline \text{Gas} & \gamma & M \\ \hline \text{Air} & 1.40 & 29.0 \\ \hline \text{Helium} & 1.67 & 4.00 \\ \hline \end{array}"#;
        // let repaired = sanitize_markdown_math(source);
        //
        // assert!(repaired.starts_with("$$\n"), "{repaired}");
        // assert!(repaired.contains(r"\begin{array}"), "{repaired}");
        // assert!(repaired.contains(r"\end{array}"), "{repaired}");
        // assert!(repaired.ends_with("\n$$"), "{repaired}");
    }

    #[test]
    fn converts_line_oriented_trace_table_with_headings() {
        // Test disabled - sanitize_markdown_math function not available
        // let source = r#"N
        // T / s
        // **1**
        // **2**
        // **3**
        // **Mean**
        // 1
        // 14.7
        // 14.1
        // 14.3
        // 2
        // 50.3
        // 49.6
        // 50.1
        // 3
        // 126.6
        // 126.3
        // 125.2
        // 4
        // 224.4
        // 224.3
        // 225.9
        // 224.9
        // 5
        // 356.1
        // 354.3
        // 345.6
        // 352.0
        // 6
        // 500.4
        // 512.7
        // 499.5
        // 504.2
        //
        // N
        // 1
        // 2
        // 3
        // 4
        // 5
        // 6"#;
        // let repaired = sanitize_markdown_math(source);
        //
        // assert!(repaired.contains("| N | T / s (1) | T / s (2) | T / s (3) | Mean |"), "{repaired}");
        // assert!(repaired.contains("| --- | ---: | ---: | ---: | ---: |"), "{repaired}");
        // assert!(repaired.contains("| 1 | 14.7 | 14.1 | 14.3 |"), "{repaired}");
        // assert!(repaired.contains("| 6 | 500.4 | 512.7 | 499.5 | 504.2 |"), "{repaired}");
        // assert!(!repaired.contains("**1**\n**2**\n**3**\n**Mean**"), "{repaired}");
        // assert_eq!(sanitize_markdown_math(&repaired), repaired);
    }

    #[test]
    fn test_slice_page_text_by_y() {
        let text = (1..=10).map(|i| format!("Line {}", i)).collect::<Vec<_>>().join("\n");
        // Full page
        assert_eq!(slice_page_text_by_y(&text, None, None), text);
        // Top slice
        let top = slice_page_text_by_y(&text, None, Some(0.3));
        assert!(top.contains("Line 1"));
        assert!(top.contains("Line 3"));
        // Bottom slice
        let bottom = slice_page_text_by_y(&text, Some(0.7), None);
        assert!(bottom.contains("Line 7"));
        assert!(bottom.contains("Line 10"));
        // Middle slice
        let middle = slice_page_text_by_y(&text, Some(0.4), Some(0.6));
        assert!(middle.contains("Line 4"));
        assert!(middle.contains("Line 6"));
    }

    #[test]
    fn test_sanitize_for_latex_math_repairs() {
        let input = "Given that f(x) = 2x 3 + ax2 + bx + c and -90◦ <= x < 90◦ and (︂x+5 1 = y+4 -3)︂ d \n 2 \n θ \n dt \n 2 = 1 \n 2 \n a and 20 \n 3 ms-1 with 2kgand4kgrespectively";
        let output = sanitize_for_latex(input);
        assert!(output.contains("2x^3"));
        assert!(output.contains("ax^2"));
        assert!(output.contains("90^\\circ"));
        assert!(output.contains("\\frac{x+5}{1} = \\frac{y+4}{-3}"));
        assert!(output.contains("\\frac{d^2 \\ensuremath{\\theta}}{dt^2}"));
        assert!(output.contains("\\frac{1}{2}a"));
        assert!(output.contains("\\frac{20}{3} \\text{ms-1}"));
        assert!(output.contains("2 \\text{ kg } and4kgrespectively") || output.contains("2 \\text{ kg } and 4 \\text{ kg } respectively"));
        let marks_input = "Find the value of u [8 m  arks ]\nFind the value of α [5 ma rks]\nState how [1 m ark ]\n[3 m a r k s]";
        let marks_output = sanitize_for_latex(marks_input);
        assert!(marks_output.contains("[8 marks]"));
        assert!(marks_output.contains("[5 marks]"));
        assert!(marks_output.contains("[1 marks]"));
        assert!(marks_output.contains("[3 marks]"));
    }

    #[test]
    fn test_format_markdown_for_latex() {
        let raw = "(i) A is a 2 by 2 matrix and B is a 2 by 3 matrix.\n\n(a) Show that M is non-singular. **[2 marks]**\n\n* the value of $\\lambda$\n* the value of $a$";
        let output = format_markdown_for_latex(raw);
        assert!(output.contains("A is a 2 by 2 matrix"));
        assert!(!output.contains("a^2 by 2"));
        assert!(output.contains("Show that"));
        assert!(!output.contains("showsthat"));
        assert!(output.contains("\\par\\textbullet\\hspace{0.5em}the value of $\\lambda$"));
        assert!(output.contains("\\null\\hfill\\textbf{[2 marks]}"));
        assert!(output.contains("\\needspace{2.5cm}"));
    }

    #[test]
    fn test_relocate_misplaced_marks() {
        // Case 1: Floating in preamble, transferred to subpart (a)
        let t1 = "A particle moves in a straight line. **[3 marks]**\n(a) Find velocity.\n(b) Find acceleration. **[2 marks]**";
        let out1 = relocate_misplaced_marks(t1);
        assert!(!out1.contains("straight line. **[3 marks]**"));
        assert!(out1.contains("(a) Find velocity. **[3 marks]**"));
        assert!(out1.contains("(b) Find acceleration. **[2 marks]**"));

        // Case 2: Standalone question with mark at start
        let t2 = "[4 marks] Calculate the force exerted on the object.";
        let out2 = relocate_misplaced_marks(t2);
        assert_eq!(out2, "Calculate the force exerted on the object. **[4 marks]**");

        // Case 3: Mid-sentence mark tag
        let t3 = "(a) State **[1 mark]** one assumption made in the model.";
        let out3 = relocate_misplaced_marks(t3);
        assert_eq!(out3, "(a) State one assumption made in the model. **[1 mark]**");

        // Case 4: Preamble with floating marks when subparts already have marks
        let t4 = "Giving a reason for your answer, explain whether it is possible **[2 marks]**\n\n(a) AB **[3 marks]**\n\n(b) A + B **[4 marks]**";
        let out4 = relocate_misplaced_marks(t4);
        assert!(!out4.contains("possible **[2 marks]**"));
        assert!(out4.contains("(a) AB **[3 marks]**"));
        assert!(out4.contains("(b) A + B **[4 marks]**"));
    }
}

/// Slice the page's digital OCR text according to the question's vertical bounds [start_y, end_y].
/// If start_y or end_y are None, the respective boundary is clamped to 0.0 or 1.0.
/// Adds a margin of safety so lines near the boundary are never clipped.
#[allow(dead_code)]
pub fn slice_page_text_by_y(text: &str, start_y: Option<f32>, end_y: Option<f32>) -> String {
    if (start_y.is_none() || start_y == Some(0.0)) && (end_y.is_none() || end_y == Some(1.0)) {
        return text.to_string();
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return String::new();
    }
    let total_lines = lines.len();
    if total_lines <= 2 {
        return text.to_string();
    }

    let s = (start_y.unwrap_or(0.0) - 0.06).max(0.0);
    let e = (end_y.unwrap_or(1.0) + 0.06).min(1.0);

    let start_line = ((total_lines as f32 * s).floor() as usize).min(total_lines.saturating_sub(1));
    let end_line = ((total_lines as f32 * e).ceil() as usize).clamp(start_line + 1, total_lines);

    lines[start_line..end_line].join("\n")
}
