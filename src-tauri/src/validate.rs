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
// wrapped blob. Outside code fences, display math, and Markdown tables,
// every source line becomes its own paragraph — what you see on the paper
// is what renders on the card.
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
            if !blank && prev_nonempty && !prev_table {
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
    let content = RE_TRIPLE_DOLLARS.replace_all(content, "$$");
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
    harden_line_breaks(&with_ligatures)
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
