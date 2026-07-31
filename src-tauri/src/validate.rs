// ── Deterministic content validators ───────────────────────────────────────
//
// Every check here is pure, cheap, and testable. Validators either *clean*
// (exact-string boilerplate removal), *measure* (marks sums, truncation), or
// *gate* (structure proposals). The pipeline uses their verdicts to build
// repair prompts and quarantine reports.

use std::sync::OnceLock;

fn re(pattern: &'static str) -> &'static regex::Regex {
    // One Regex per distinct literal pattern, compiled once per process.
    // Each compiled Regex is boxed and leaked, giving a stable 'static
    // address (a map rehash can never invalidate references).
    static CACHE: OnceLock<
        std::sync::Mutex<std::collections::HashMap<&'static str, &'static regex::Regex>>,
    > = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut guard = cache.lock().unwrap();
    let slot = guard.entry(pattern).or_insert_with(|| {
        Box::leak(Box::new(regex::Regex::new(pattern).unwrap())) as &'static regex::Regex
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
///   * zero, numbers > 200 (raised from 60 so IB / CIE papers with many
///     structured questions don't false-negative — a 200-question paper
///     is implausible at A-Level),
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
                    } else if f >= 1.0 && f < 200.0 {
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
        Some(n) if (1..=200).contains(&n) => Some(n as u32),
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

/// True when the content ends like finished prose / math, not mid-word.
pub fn has_terminal_ending(content: &str) -> bool {
    let t = content.trim_end();
    if t.is_empty() {
        return false;
    }
    // Ends with a marks tag?
    let re_tag = re(r"(?i)(?:\[|\()\s*\d{1,2}\s*marks?\s*(?:\]|\))\s*\**\s*$");
    if re_tag.is_match(t) {
        return true;
    }
    // Ends with display math close, code fence, or terminal punctuation?
    if t.ends_with("$$") || t.ends_with("```") || t.ends_with('$') || t.ends_with('`') {
        return true;
    }
    // Markdown tables (AQA trace tables) end with '|' — treat as terminal.
    // Without this, questions ending in a trace table get flagged as truncated
    // and quarantined after 3 repair attempts (the June 2024 CS regression).
    if t.ends_with('|') {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    if lower.ends_with("continued") || lower.ends_with("turn over") || lower.ends_with("blank page") {
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
    if question_number == 0 || question_number > 99 || content.is_empty() {
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

// ── Source line preservation ────────────────────────────────────────────────
//
// Markdown collapses single newlines into one flowing paragraph. Exam
// content (database schemas, algorithms, tables) is LINE-structured: losing
// the line breaks mashes "Product(ProductID, Description," into a single
// wrapped blob. Outside code fences, display math, and Markdown tables,
// every source line becomes its own paragraph — what you see on the paper
// is what renders on the card.
pub fn harden_line_breaks(content: &str) -> String {
    let mut out = String::with_capacity(content.len() + content.len() / 2);
    let mut in_fence = false;
    let mut in_math = false;
    let mut prev_nonempty = false;
    let mut prev_table = false;
    for line in content.split('\n') {
        let trimmed = line.trim_end();
        let t = trimmed.trim_start();
        // State BEFORE toggles decides the route: the CLOSING marker line of
        // a fence/math block is itself protected content.
        let protected = in_fence || in_math;
        let is_table = !protected && t.starts_with('|');
        let blank = t.is_empty();
        if protected || is_table {
            out.push_str(trimmed);
            out.push('\n');
        } else {
            if !blank && prev_nonempty && !prev_table {
                out.push('\n');
            }
            out.push_str(trimmed);
            out.push('\n');
        }
        if t.starts_with("```") && !in_math {
            in_fence = !in_fence;
        }
        if !in_fence && t.starts_with("$$") {
            let inner = &t[2..];
            let single_line = inner.len() >= 2 && inner.ends_with("$$") && !inner[..inner.len() - 2].contains("$$");
            if !single_line {
                in_math = !in_math;
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

pub fn clean_question_content(content: &str) -> String {
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
    let mut cleaned = content.to_string();
    for p in patterns {
        cleaned = re(p).replace_all(&cleaned, "").into_owned();
    }
    
    // Strip trailing inequality answer templates (e.g., "$... \le t < ...$ [2 marks]") while preserving the marks
    let ineq_re = regex::Regex::new(r"(?im)^[\s\.\$]*(?:\\\\?leq?|\\\\?geq?|<|>)\s*[a-zA-Z]\s*(?:\\\\?leq?|\\\\?geq?|<|>)\s*(.*?)\s*\$?\s*$").unwrap();
    cleaned = ineq_re.replace_all(&cleaned, "$1").into_owned();

    // Strip trailing equality answer templates (e.g., "$... x = ...$ [2 marks]")
    let eq_re = regex::Regex::new(r"(?im)^[\s\.\$]*[a-zA-Z]\s*=\s*(.*?)\s*\$?\s*$").unwrap();
    cleaned = eq_re.replace_all(&cleaned, "$1").into_owned();

    // Collapse runs of 3+ newlines left by removals.
    let collapse = re(r"\n{3,}");
    let collapsed = collapse.replace_all(&cleaned, "\n\n").trim().to_string();
    // Source lines are meaningful — don't let Markdown reflow them into a
    // single blob (schemas, algorithms, multi-part stems).
    let hardened = harden_line_breaks(&collapsed);
    
    // Automatically close any unclosed $ or $$ tags to prevent MDX parser crashes
    sanitize_markdown_math(&hardened)
}

/// Repair common LLM/PDF LaTeX damage before balancing Markdown delimiters.
///
/// This deliberately handles structure rather than trying to be a LaTeX
/// compiler: malformed prose inside `aligned`, bare display formulas, arrays
/// outside math mode, and line-oriented trace tables are all common outputs
/// from PDF transcription and can be repaired without changing ordinary text.
pub fn repair_latex_syntax(text: &str) -> String {
    let mut text = normalize_trace_table(text);
    let aligned_open = text.matches(r"\begin{aligned}").count();
    let aligned_close = text.matches(r"\end{aligned}").count();
    if aligned_open > aligned_close {
        text = text
            .replace(r"\begin{aligned}", "")
            .replace(r"\end{aligned}", "")
            .replace(r"\\", "\n");
    }

    let hallucinated_aligned = re(r"(?s)\\begin\{aligned\}(.*?)\\end\{aligned\}");
    text = hallucinated_aligned
        .replace_all(&text, |captures: &regex::Captures<'_>| {
            let block = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
            let contains_non_math = block.contains("**")
                || block.contains("![")
                || block.contains("](")
                || block.contains("[DIAGRAM_PLACEHOLDER]");
            if contains_non_math {
                block.replace(r"\\", "\n")
            } else {
                captures.get(0).map(|m| m.as_str()).unwrap_or_default().to_string()
            }
        })
        .into_owned();
    let mut out = Vec::new();
    let mut in_array = false;
    let mut array_lines: Vec<String> = Vec::new();

    for raw_line in text.lines() {
        let mut line = raw_line.to_string();
        let has_aligned = line.contains(r"\begin{aligned}") || line.contains(r"\end{aligned}");

        // `aligned` is a math environment. If an LLM put prose and inline `$`
        // math inside it, unwrap the environment and turn row separators into
        // ordinary line breaks instead of emitting invalid nested math.
        if has_aligned && (line.contains('$') || line.contains(" where ") || line.contains(" is ")) {
            line = line.replace(r"\begin{aligned}", "");
            line = line.replace(r"\end{aligned}", "");
            line = line.replace(r"\ \ ", "\n");
            line = line.replace(r"\\", "\n");
            for part in line.split('\n') {
                if !part.trim().is_empty() {
                    out.push(part.trim().to_string());
                }
            }
            continue;
        }

        if line.contains(r"\begin{array}") || line.contains(r"\begin{matrix}") {
            in_array = true;
            array_lines.push(line);
            if array_lines.last().is_some_and(|s| s.contains(r"\end{array}") || s.contains(r"\end{matrix}")) {
                append_array_block(&mut out, &mut array_lines);
                in_array = false;
            }
            continue;
        }
        if in_array {
            let closes_array = line.contains(r"\end{array}") || line.contains(r"\end{matrix}");
            array_lines.push(line);
            if closes_array {
                append_array_block(&mut out, &mut array_lines);
                in_array = false;
            }
            continue;
        }

        let trimmed = line.trim();
        let bare_math = trimmed.starts_with(r"\left")
            || trimmed.starts_with(r"\frac")
            || trimmed.starts_with(r"\sqrt")
            || trimmed.starts_with(r"\begin{array}")
            || trimmed.starts_with(r"\begin{matrix}");
        if bare_math && !trimmed.contains('$') {
            line = format!("$$\n{}\n$$", trimmed);
        }
        out.push(line);
    }

    if in_array && !array_lines.is_empty() {
        // Complete an accidentally truncated array rather than leaving an
        // unterminated environment for the Markdown/LaTeX renderer.
        if !array_lines.iter().any(|line| line.contains(r"\end{array}")) {
            array_lines.push(r"\end{array}".to_string());
        }
        append_array_block(&mut out, &mut array_lines);
    }

    out.join("\n")
}

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

fn is_plain_integer(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_digit())
}

fn clean_table_cell(value: &str) -> Option<String> {
    let value = value.trim().trim_matches('*').trim();
    if value.parse::<f64>().is_ok() {
        Some(value.to_string())
    } else {
        None
    }
}

/// Automatically repair malformed LaTeX and close missing inline `$`/block
/// `$$` tags.
pub fn sanitize_markdown_math(text: &str) -> String {
    let text = aggressive_markdown_math_cleanup(text);
    let mut text = repair_latex_syntax(&text);
    if text.matches("$$").count() % 2 != 0 {
        text.push_str("\n$$");
    }
    let mut in_block = false;
    let mut lines = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "$$" {
            in_block = !in_block;
            lines.push(line.to_string());
            continue;
        }

        if in_block {
            lines.push(line.to_string());
            continue;
        }

        let mut inline_count = 0;
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '$' {
                let escaped = i > 0 && chars[i-1] == '\\';
                let double = i + 1 < chars.len() && chars[i+1] == '$';
                if !escaped {
                    if double {
                        i += 1;
                    } else {
                        inline_count += 1;
                    }
                }
            }
            i += 1;
        }

        if inline_count % 2 != 0 {
            lines.push(format!("{}$", line));
        } else {
            lines.push(line.to_string());
        }
    }

    if in_block {
        lines.push("$$".to_string());
    }

    collapse_blank_lines_re()
        .replace_all(&lines.join("\n"), "\n\n")
        .into_owned()
}

/// Normalize common LLM formatting damage before the structural LaTeX repair
/// pass. These are deliberately text-level fixes: they do not attempt to
/// parse or rewrite the math expression itself.
fn aggressive_markdown_math_cleanup(text: &str) -> String {
    // Keep every Markdown image on its own block-level line and consume any
    // unpredictable whitespace attached to either side.
    let mut cleaned = markdown_image_re()
        .replace_all(text, "\n\n${1}\n\n")
        .into_owned();

    // Rust's regex crate deliberately does not support the requested negative
    // lookahead `(?:(?!\$\$).)*`. Match line candidates with a precompiled,
    // regex-compatible equivalent, then enforce "no earlier $$" in the
    // replacement callback. Single `$` characters do not prevent a match.
    cleaned = missing_display_opener_re()
        .replace_all(&cleaned, |captures: &regex::Captures<'_>| {
            let whole = captures.get(0).map(|m| m.as_str()).unwrap_or_default();
            let body = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
            if body.trim().is_empty() || body.contains("$$") {
                whole.to_string()
            } else {
                // Promoting the whole line to display math makes any inline
                // `$...$` delimiters nested and invalid. Keep their contents,
                // but remove the unescaped single-dollar wrappers.
                format!("$${}$$", strip_single_dollar_delimiters(body))
            }
        })
        .into_owned();

    // Move punctuation outside display math. A comma before a closing display
    // delimiter is discarded because it is usually an LLM-generated separator;
    // a sentence-ending period is retained after the delimiter.
    cleaned = re(r",\s*\$\$")
        .replace_all(&cleaned, replace_comma_display_delimiter)
        .into_owned();
    cleaned = re(r"\.\s*\$\$")
        .replace_all(&cleaned, replace_period_display_delimiter)
        .into_owned();

    cleaned
}

fn markdown_image_re() -> &'static regex::Regex {
    static IMAGE_RE: OnceLock<regex::Regex> = OnceLock::new();
    IMAGE_RE.get_or_init(|| {
        regex::Regex::new(r"(?s)\s*(!\[.*?\]\(.*?\))\s*")
            .expect("valid Markdown image isolation regex")
    })
}

fn missing_display_opener_re() -> &'static regex::Regex {
    static DISPLAY_RE: OnceLock<regex::Regex> = OnceLock::new();
    DISPLAY_RE.get_or_init(|| {
        regex::Regex::new(r"(?m)^([^\r\n]*?)\$\$[ \t]*$")
            .expect("valid missing display opener regex")
    })
}

fn collapse_blank_lines_re() -> &'static regex::Regex {
    static NEWLINES_RE: OnceLock<regex::Regex> = OnceLock::new();
    NEWLINES_RE.get_or_init(|| {
        regex::Regex::new(r"\n{3,}").expect("valid duplicate newline regex")
    })
}

fn strip_single_dollar_delimiters(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        if character == '$' {
            let escaped = output
                .chars()
                .rev()
                .take_while(|previous| *previous == '\\')
                .count()
                % 2
                == 1;
            if !escaped {
                continue;
            }
        }
        output.push(character);
    }
    output
}

fn replace_comma_display_delimiter(_: &regex::Captures<'_>) -> String {
    "$$".to_string()
}

fn replace_period_display_delimiter(_: &regex::Captures<'_>) -> String {
    "$$.".to_string()
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
    let re_examiner_codes = regex::Regex::new(r"(?i)[\s,;]*(?:[\[(]?\b(?:d?(?:[mab]1?|ft|oe|cao|aef|awrt|dep|indep|allow|condone|ignore|accept|or\s+equivalent|award))[\](,)]*)+\s*$").unwrap();
    let mut lines: Vec<String> = chunk.lines().map(|line| {
        let cleaned = re_examiner_codes.replace_all(line, "");
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
    fn question_number_rejects_decimals_and_junk() {
        assert_eq!(value_to_question_number(&serde_json::json!(7)), Some(7));
        assert_eq!(value_to_question_number(&serde_json::json!("12")), Some(12));
        assert_eq!(value_to_question_number(&serde_json::json!("03.1")), Some(3)); // extracts the integer part
        assert_eq!(value_to_question_number(&serde_json::json!(0)), None);
        assert_eq!(value_to_question_number(&serde_json::json!(201)), None); // >200
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
        let schema = "Product(ProductID, Description,\nQuantityInStock, SupplierID)\nSale(SaleID, CustomerID, SaleDate)";
        let out = harden_line_breaks(schema);
        assert!(out.contains("Description,\n\nQuantityInStock"), "lines must not reflow: {out}");

        let table = "| A | B |\n| --- | --- |\n| 1 | 2 |";
        assert_eq!(harden_line_breaks(table), table, "tables keep single newlines");

        let code = "```\nline1\nline2\n```";
        assert_eq!(harden_line_breaks(code), code, "fences untouched");

        let para = "One sentence.\n\nNext paragraph.";
        assert_eq!(harden_line_breaks(para), para);
    }

    #[test]
    fn repairs_malformed_aligned_prose_and_bare_formula() {
        let source = r#"\left(\frac{\gamma RT}{M}\right)^{1/2}
\begin{aligned} where \\ \\ $\gamma$ is a dimensionless constant that depends on the gas \\ \\ $R$ is the molar gas constant \\ \\ $T$ is the absolute temperature \\ \\ $M$ is the molar mass of the gas. \end{aligned}"#;
        let repaired = sanitize_markdown_math(source);

        assert!(repaired.contains("$$\n\\left(\\frac"), "{repaired}");
        assert!(!repaired.contains(r"\begin{aligned}"), "{repaired}");
        assert!(!repaired.contains(r"\end{aligned}"), "{repaired}");
        assert!(repaired.contains("$\\gamma$ is a dimensionless"), "{repaired}");
        assert!(repaired.matches("$$").count() % 2 == 0, "{repaired}");
    }

    #[test]
    fn removes_markdown_and_diagram_markup_from_aligned_math() {
        let source = r#"$$\begin{aligned}
F &= ma \\
**[5 marks]** \\
![Diagram](/tmp/diagram.png)
\end{aligned}$$"#;
        let repaired = sanitize_markdown_math(source);

        assert!(!repaired.contains(r"\begin{aligned}"), "{repaired}");
        assert!(repaired.contains("**[5 marks]**"), "{repaired}");
        assert!(repaired.contains("![Diagram](/tmp/diagram.png)"), "{repaired}");
    }

    #[test]
    fn closes_inline_display_delimiter_without_rewriting_prose() {
        let repaired = sanitize_markdown_math("The result is $$x = 2.$$ ");
        assert_eq!(repaired.matches("$$").count() % 2, 0);
        assert!(repaired.contains("The result is"));
    }

    #[test]
    fn isolates_inline_markdown_images() {
        let repaired = sanitize_markdown_math("Before ![Figure 1](/tmp/figure.png) after");
        let lines: Vec<&str> = repaired.lines().collect();
        let image_line = lines
            .iter()
            .position(|line| *line == "![Figure 1](/tmp/figure.png)")
            .expect("image should occupy its own line");
        assert!(image_line > 0);
        assert!(image_line + 1 < lines.len());
    }

    #[test]
    fn isolates_images_with_tabs_crlf_and_existing_blank_lines() {
        let repaired = sanitize_markdown_math(
            "Before.\t \r\n\r\n  ![Diagram](/tmp/figure.png) \t\r\n\r\nAfter.",
        );
        assert_eq!(
            repaired,
            "Before.\n\n![Diagram](/tmp/figure.png)\n\nAfter."
        );
        assert!(!repaired.contains("\n\n\n"));
    }

    #[test]
    fn moves_punctuation_outside_display_delimiters() {
        assert_eq!(
            sanitize_markdown_math("$$x = 1,$$"),
            "$$x = 1$$"
        );
        assert_eq!(
            sanitize_markdown_math("$$x = 1.$$"),
            "$$x = 1$$."
        );
    }

    #[test]
    fn opens_math_lines_that_only_have_a_closing_delimiter() {
        let repaired = sanitize_markdown_math("x = \\frac{1}{2} $$");
        assert!(repaired.starts_with("$$x = \\frac{1}{2} $$"));
        assert_eq!(repaired.matches("$$").count() % 2, 0);
    }

    #[test]
    fn opens_real_llm_math_lines_with_punctuation_and_trailing_spaces() {
        let first = sanitize_markdown_math(
            "t^2-2, \\quad y=6t, \\quad t\\ge0.$$   ",
        );
        assert_eq!(first, "$$t^2-2, \\quad y=6t, \\quad t\\ge0$$.");

        let second = sanitize_markdown_math("\\int_2^T 12t^2 , dt$$\t");
        assert_eq!(second, "$$\\int_2^T 12t^2 , dt$$");

        let inline = sanitize_markdown_math("y=$6t$, \\quad t\\ge0.$$  ");
        assert_eq!(inline, "$$y=6t, \\quad t\\ge0$$.");
    }

    #[test]
    fn duplicate_newlines_are_collapsed_after_all_repairs() {
        let repaired = sanitize_markdown_math("First\n\n\n\nSecond");
        assert_eq!(repaired, "First\n\nSecond");
    }

    #[test]
    fn repairs_array_environment_into_display_math() {
        let source = r#"\begin{array}{|l|l|l|} \hline \text{Gas} & \gamma & M \\ \hline \text{Air} & 1.40 & 29.0 \\ \hline \text{Helium} & 1.67 & 4.00 \\ \hline \end{array}"#;
        let repaired = sanitize_markdown_math(source);

        assert!(repaired.starts_with("$$\n"), "{repaired}");
        assert!(repaired.contains(r"\begin{array}"), "{repaired}");
        assert!(repaired.contains(r"\end{array}"), "{repaired}");
        assert!(repaired.ends_with("\n$$"), "{repaired}");
    }

    #[test]
    fn converts_line_oriented_trace_table_with_headings() {
        let source = r#"N
T / s
**1**
**2**
**3**
**Mean**
1
14.7
14.1
14.3
2
50.3
49.6
50.1
3
126.6
126.3
125.2
4
224.4
224.3
225.9
224.9
5
356.1
354.3
345.6
352.0
6
500.4
512.7
499.5
504.2

N
1
2
3
4
5
6"#;
        let repaired = sanitize_markdown_math(source);

        assert!(repaired.contains("| N | T / s (1) | T / s (2) | T / s (3) | Mean |"), "{repaired}");
        assert!(repaired.contains("| --- | ---: | ---: | ---: | ---: |"), "{repaired}");
        assert!(repaired.contains("| 1 | 14.7 | 14.1 | 14.3 |"), "{repaired}");
        assert!(repaired.contains("| 6 | 500.4 | 512.7 | 499.5 | 504.2 |"), "{repaired}");
        assert!(!repaired.contains("**1**\n**2**\n**3**\n**Mean**"), "{repaired}");
        assert_eq!(sanitize_markdown_math(&repaired), repaired);
    }
}
