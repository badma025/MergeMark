// ── JSON boundary discipline ────────────────────────────────────────────────
//
// Policy (see docs/INGESTION_RELIABILITY_PLAN.md §3.2):
//   1. Parse the model's output VERBATIM. We never rewrite escapes — any
//      "fixer" on a broken wire format is guessing, and `fix_json_escapes`
//      provably mangled `\nabla`/`\tan`/`\frac` into control characters.
//   2. On parse failure the error is ROUND-TRIPPED to the model by the
//      caller ("your output was invalid JSON: <serde error>"), so the author
//      of the content resolves ambiguities a regex never can.
//   3. Last-resort salvage for the *truncation* case only: cut at the end of
//      the last COMPLETE top-level item and close the structure. Salvage
//      never invents content and never edits inside a string.
//
// Anything that survives here is guaranteed to be exactly what the model
// emitted — recovered or rejected, never silently rewritten.

use serde::de::DeserializeOwned;

/// The result of interpreting one LLM response body.
pub enum ParseOutcome<T> {
    /// Parsed verbatim (after harmless fence/preamble stripping).
    Clean(T),
    /// Valid JSON recovered by dropping a tail: either junk after the first
    /// complete value (`dropped_tail == false`) or a truncated final item
    /// (`dropped_tail == true` — the content may be incomplete; caller should
    /// flag or repair).
    Salvaged { value: T, dropped_tail: bool },
    /// Not interpretable. `error` is suitable for quoting back to the model
    /// in a repair prompt.
    Malformed { error: String },
}

/// Strip a leading ```json / ``` fence and trailing ``` if present.
pub fn strip_code_fence(s: &str) -> &str {
    let t = s.trim();
    let t = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```JSON"))
        .or_else(|| t.strip_prefix("```"))
        .unwrap_or(t);
    t.strip_suffix("```").unwrap_or(t).trim()
}

/// Skip conversational filler before the first '{' or '['.
fn skip_preamble(s: &str) -> &str {
    match s.find(|c| c == '{' || c == '[') {
        Some(i) => &s[i..],
        None => s,
    }
}

struct ScanResult {
    /// Byte index just past the end of the first complete root value.
    first_value_end: Option<usize>,
    /// Byte index just past the last `}` that closed a top-level *item*
    /// (i.e. returned the depth to the array holding the items).
    last_item_boundary: Option<usize>,
    /// Expected closers (innermost first is already handled by reversal at
    /// the call site) to balance the structure at `last_item_boundary`.
    closers_at_boundary: Vec<char>,
}

/// Single pass tracking string/escape state and the opening-char stack.
fn scan(s: &str) -> ScanResult {
    let mut stack: Vec<char> = Vec::new(); // expected closers: '}' or ']'
    let mut in_string = false;
    let mut escaped = false;
    let mut first_value_end = None;
    let mut last_item_boundary = None;
    let mut closers_at_boundary: Vec<char> = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        let c = bytes[i] as char;
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else {
            match c {
                '"' => in_string = true,
                '{' => stack.push('}'),
                '[' => stack.push(']'),
                '}' | ']' => {
                    let depth_before = stack.len();
                    let was_item_close = c == '}'
                        && depth_before >= 2
                        && stack.get(depth_before.saturating_sub(2)) == Some(&']');
                    if stack.last() == Some(&c) {
                        stack.pop();
                    }
                    if stack.is_empty() && first_value_end.is_none() {
                        first_value_end = Some(i + 1);
                    }
                    if was_item_close {
                        last_item_boundary = Some(i + 1);
                        closers_at_boundary = stack.clone();
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }

    ScanResult {
        first_value_end,
        last_item_boundary,
        closers_at_boundary,
    }
}

/// Interpret one LLM response. See the module docs for the policy.
pub fn parse_llm_json<T: DeserializeOwned>(wire: &str) -> ParseOutcome<T> {
    let prepared = skip_preamble(strip_code_fence(wire));

    // 1. Verbatim.
    match serde_json::from_str::<T>(prepared) {
        Ok(v) => return ParseOutcome::Clean(v),
        Err(e) => {
            // 2. Trailing junk (commentary or a second object appended):
            // accept the first complete value only.
            if e.is_data() || e.to_string().contains("trailing characters") {
                let scan = scan(prepared);
                if let Some(end) = scan.first_value_end {
                    if end < prepared.len() {
                        if let Ok(v) = serde_json::from_str::<T>(&prepared[..end]) {
                            return ParseOutcome::Salvaged { value: v, dropped_tail: false };
                        }
                    }
                }
            }

            // 3. Truncation salvage: cut at the last complete item and close.
            let scan = scan(prepared);
            if let Some(end) = scan.last_item_boundary {
                if end < prepared.len() {
                    let mut candidate = prepared[..end].to_string();
                    for closer in scan.closers_at_boundary.iter().rev() {
                        candidate.push(*closer);
                    }
                    if let Ok(v) = serde_json::from_str::<T>(&candidate) {
                        return ParseOutcome::Salvaged { value: v, dropped_tail: true };
                    }
                }
            }

            ParseOutcome::Malformed {
                error: format!("{} (at line {}, column {})", e, e.line(), e.column()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, serde::Deserialize, PartialEq)]
    struct Item {
        n: u32,
    }

    #[derive(Debug, serde::Deserialize, PartialEq)]
    struct Page {
        #[serde(default)]
        extracted_questions: Vec<Item>,
    }

    #[test]
    fn clean_verbatim() {
        let wire = r#"{"extracted_questions":[{"n":1},{"n":2}]}"#;
        match parse_llm_json::<Page>(wire) {
            ParseOutcome::Clean(p) => assert_eq!(p.extracted_questions.len(), 2),
            _ => panic!("expected Clean"),
        }
    }

    #[test]
    fn clean_after_fence_and_preamble() {
        let wire = "Sure! Here is the JSON:\n```json\n{\"extracted_questions\":[{\"n\":7}]}\n```";
        match parse_llm_json::<Page>(wire) {
            ParseOutcome::Clean(p) => assert_eq!(p.extracted_questions[0].n, 7),
            _ => panic!("expected Clean"),
        }
    }

    #[test]
    fn truncation_salvages_complete_prefix_only() {
        // Second item is cut mid-string: only item 1 may come back.
        let wire = r#"{"extracted_questions":[{"n":1},{"n":2,"content":"Evaluate \nabla f an"#;
        match parse_llm_json::<Page>(wire) {
            ParseOutcome::Salvaged { value, dropped_tail } => {
                assert!(dropped_tail);
                assert_eq!(value.extracted_questions.len(), 1);
                assert_eq!(value.extracted_questions[0].n, 1);
            }
            _ => panic!("expected Salvaged"),
        }
    }

    #[test]
    fn truncation_with_nested_structures() {
        // Item close is detected through nested arrays/objects.
        let wire = r#"{"extracted_questions":[{"n":1},{"n":2,"b":[[0.1,0.2,0.3,0.4]]},{"n":3,"b":[[0.5"#;
        match parse_llm_json::<Page>(wire) {
            ParseOutcome::Salvaged { value, dropped_tail } => {
                assert!(dropped_tail);
                assert_eq!(value.extracted_questions.len(), 2);
            }
            _ => panic!("expected Salvaged"),
        }
    }

    #[test]
    fn trailing_junk_accepted_first_value() {
        let wire = r#"{"extracted_questions":[{"n":1}]} I hope this helps!"#;
        match parse_llm_json::<Page>(wire) {
            ParseOutcome::Salvaged { value, dropped_tail } => {
                assert!(!dropped_tail);
                assert_eq!(value.extracted_questions.len(), 1);
            }
            ParseOutcome::Clean(v) => assert_eq!(v.extracted_questions.len(), 1),
            _ => panic!("expected Salvaged or Clean"),
        }
    }

    #[test]
    fn properly_escaped_latex_stays_verbatim() {
        // The old `fix_json_escapes` mangled LaTeX even when the model did
        // everything right. We must pass it through untouched.
        #[derive(Debug, serde::Deserialize)]
        struct Row {
            c: String,
        }
        let wire = r#"{"c":"$\nabla f$ and $\tan \theta$"}"#;
        match parse_llm_json::<Row>(wire) {
            ParseOutcome::Clean(r) => assert_eq!(r.c, "$\\nabla f$ and $\\tan \\theta$"),
            _ => panic!("expected Clean"),
        }
    }

    #[test]
    fn invalid_escape_goes_to_repair_not_guessing() {
        // `\uZ` is NOT a valid escape: old code would guess, we must reject.
        let wire = "{\"extracted_questions\":[{\"n\":1,\"c\":\"bad \\uZoo\"}]}";
        match parse_llm_json::<Page>(wire) {
            ParseOutcome::Malformed { .. } => {}
            ParseOutcome::Salvaged { dropped_tail, .. } => {
                // salvage may only succeed by dropping the content entirely;
                // truncated-item recovery would remove the offending item.
                assert!(dropped_tail);
            }
            ParseOutcome::Clean(_) => panic!("must not silently accept invalid escapes"),
        }
    }

    #[derive(Debug, serde::Deserialize, PartialEq)]
    #[serde(untagged)]
    enum AnswerEnvelope {
        Wrapped { #[serde(default)] answers: Vec<Item> },
        Bare(Vec<Item>),
    }

    #[test]
    fn bare_array_supported_for_mark_scheme() {
        let wire = r#"[{"n":1},{"n":2}]"#;
        match parse_llm_json::<AnswerEnvelope>(wire) {
            ParseOutcome::Clean(AnswerEnvelope::Bare(v)) => assert_eq!(v.len(), 2),
            _ => panic!("expected Clean bare array"),
        }
    }
}
