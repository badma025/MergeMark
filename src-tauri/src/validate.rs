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
/// whole question number. Rejects: missing, zero, > 60, and multi-part
/// decimals like "03.1" (which digit-mashing used to turn into "31").
pub fn value_to_question_number(v: &serde_json::Value) -> Option<u32> {
    let raw: Option<u64> = match v {
        serde_json::Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_i64().and_then(|x| u64::try_from(x).ok()))
            .or_else(|| {
                n.as_f64().and_then(|f| {
                    if f.fract() == 0.0 && f >= 0.0 {
                        Some(f as u64)
                    } else {
                        None
                    }
                })
            }),
        serde_json::Value::String(s) => {
            let t = s.trim();
            if t.contains('.') || t.contains(',') {
                // "03.1" or "1,2" are sub-part styles — refuse to guess.
                None
            } else {
                t.parse::<u64>().ok()
            }
        }
        _ => None,
    }?;
    match raw {
        Some(n) if (1..=60).contains(&n) => Some(n as u32),
        _ => None,
    }
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
    matches!(
        t.chars().last(),
        Some('.') | Some('?') | Some('!') | Some(')') | Some(']') | Some(':') | Some(';')
    )
}

// ── Boilerplate scrubbing (exact-string policy, moved from commands.rs) ────

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
    // Collapse runs of 3+ newlines left by removals.
    let collapse = re(r"\n{3,}");
    collapse.replace_all(&cleaned, "\n\n").trim().to_string()
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
        assert_eq!(value_to_question_number(&serde_json::json!("03.1")), None); // not "31"!
        assert_eq!(value_to_question_number(&serde_json::json!(0)), None);
        assert_eq!(value_to_question_number(&serde_json::json!(99)), None);
        assert_eq!(value_to_question_number(&serde_json::json!(3.7)), None);
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
}
