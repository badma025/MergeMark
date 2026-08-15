use regex::Regex;
use std::sync::LazyLock;

// ══════════════════════════════════════════════════════════════════════════
// VLM System Prompts & Strict Formatting Rules
// ══════════════════════════════════════════════════════════════════════════

pub const VLM_RULE_INLINE_IMAGE_PLACEMENT: &str =
    "If an image or diagram is present, insert its placeholder [DIAGRAM_PLACEHOLDER] (or ![Diagram](image_key)) IMMEDIATELY after the sentence or paragraph that references it (e.g., directly after 'shown in Figure 3'). Never place diagrams at the end of the question if they were referenced earlier.";

pub const VLM_RULE_DELIMITER_DISCIPLINE: &str =
    "NEVER place inline math delimiters $ inside a display math $$ block. Display math must start with $$ and end with $$ with NO inner $ signs.";

pub const VLM_RULE_CHARACTER_PRECISION: &str =
    "Pay close attention to function notation. Do not confuse the italic function symbol $f$ in $f(x)$ or $f(t)$ with the number 1. Pay extreme attention to Greek symbols: do not confuse \\theta with the number 1, or \\alpha with a. Accurately transcribe all complex number forms, e.g., r(\\cos \\theta + \\text{i}\\sin \\theta).";

pub const VLM_RULE_LIST_CLEANUP: &str =
    "Do not output empty list bullets or empty numbered prefixes.";

pub const VLM_RULE_TABLE_FORMATTING: &str =
    "If a grid or table contains standard text or numbers, you MUST format it as a standard Markdown table using pipes | and dashes -. NEVER use LaTeX array environments or \\hline for data tables.";

pub const VLM_RULE_EQUATION_COHESION: &str =
    "A mathematical equation MUST remain inside a single, cohesive display math block $$ ... $$. Never split an equation into multiple blocks. Operators like =, +, or exponents like ^n and ^{-1} must remain inside the same block as the matrices or variables they belong to.";

pub const VLM_RULE_SYMBOL_PRESERVATION: &str =
    "Pay extreme attention to Greek symbols. Do not confuse \\theta with the number 1, or \\alpha with a. Accurately transcribe all complex number forms, e.g., r(\\cos \\theta + \\text{i}\\sin \\theta).";

pub const VLM_RULE_DELIMITER_NESTING: &str =
    "Never wrap regular prose or full sentences in $$ display math delimiters. Only use $$ for standalone mathematical equations. Never nest inline math $ inside a display math $$ block.";

pub const VLM_FEW_SHOT_INLINE_FRACTURE: &str =
    "CRITICAL: Never fracture inline math. WRONG: $r(\\cos$ \\theta $). RIGHT: $r(\\cos \\theta)$.";

pub const VLM_FEW_SHOT_PROSE_WRAPPING: &str =
    "CRITICAL: Never wrap English sentences in $$. Use $ for variables inside text.";

// ══════════════════════════════════════════════════════════════════════════
// Compiled Regex Patterns for Post-Processing Fixes
// ══════════════════════════════════════════════════════════════════════════

static RE_ARTIFACT_BOILERPLATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^\s*(?:MARK\s+SCHEME|GCSE|A[-\s]?LEVEL|AS\s+LEVEL|BLANK\s+PAGE|DO\s+NOT\s+WRITE|TURN\s+OVER|TOTAL\s+FOR\s+(?:QUESTION|PAPER)\s+\d+|QUESTION\s+PAPER|EXAMINER|CENTRE\s+NUMBER|CANDIDATE\s+NUMBER|SPECIMEN|PRACTICE\s+PAPER|MOCK\s+EXAM|INSERT|FORMULAE?\s+SHEET|DATA\s+BOOKLET|COPYRIGHT|ACKNOWLEDGE|BLANK|PAGE\s+\d+\s+OF\s+\d+|\d+\s+OF\s+\d+)\b.*$").unwrap()
});

static RE_AQA_DECIMAL_PART: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|\n)\s*(\d+)(?:[.\s])([1-9])\b").unwrap()
});

static RE_SUBPART_LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)(^|\n)(\s*)(\([a-z]\)|\([ivx]+\))(\s)").unwrap()
});

static RE_VISUAL_MCQ_LEADIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(Which\s+(?:graph|diagram|figure|image|chart|plot|sketch|drawing)\s+(?:is|shows|represents|matches|corresponds\s+to)\s+(?:correct|the\s+correct|best|accurate)\s*\w*\??|Which\s+of\s+the\s+following\s+(?:graphs?|diagrams?|figures?|images?|charts?|plots?|sketches?)\s+(?:is|shows|represents)\??)\s*").unwrap()
});

static RE_MARK_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\*\*\[\s*(\d+)\s+marks?\s*\]\*\*").unwrap()
});

static RE_TABLE_MCQ_ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\|?\s*([A-D]|\([A-D]\)|\d+)[\.)]?\s*\|([^|]*)\|").unwrap()
});

static RE_LATEX_SPACED_CMD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\\ +(begin|end|frac|dfrac|cfrac|sqrt|text|textbf|textit|mathbf|mathit|mathrm|mathbb|mathcal|operatorname|pmatrix|bmatrix|vmatrix|matrix|array|cases|aligned|theta|lambda|alpha|beta|gamma|delta|Delta|pi|mu|sigma|omega|Omega|phi|Phi|psi|Psi|times|div|pm|mp|leq|geq|neq|approx|sim|equiv|subset|supset|subseteq|supseteq|in|notin|forall|exists|infty|partial|nabla|cos|sin|tan|sec|csc|cot|cosh|sinh|tanh|ln|log|exp|int|iint|iiint|oint|sum|prod|lim|vec|hat|bar|dot|ddot|tilde|binom|quad|qquad|hline|hlinex|cline|multicolumn|multirow)\b").unwrap()
});

static RE_EXCESSIVE_NEWLINES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\n{3,}").unwrap()
});

static RE_PAGE_ARTIFACTS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^\s*(?:Page\s+\d+|Pg\.\s*\d+|\d+\s*/\s*\d+|\[\s*Barcode\s*\]|\[\s*Registration\s*\]|Printer\s*Mark).*$").unwrap()
});

static RE_LEADING_NUMBER_ARTIFACTS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)(^|\n\n)\s*(?:Q\d+\s*[.:]?|\d{1,3}(?:\s+[A-Za-z\d]){1,3}\s+)").unwrap()
});

static RE_VISUAL_MCQ_GIBBERISH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:[A-D]\s*){4,}$").unwrap()
});

static RE_FOOTER_CODES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^\s*[A-Z]{2}/[A-Z]/[A-Za-z]{3}\d{2}/\d+/\d+\s*$").unwrap()
});

static RE_MARGIN_ARTIFACTS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^\s*Do\s+not\s+write\s+(?:outside\s+the\s+box|in\s+this\s+area)\b.*$").unwrap()
});

static RE_SERIAL_CODES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^\s*(?:[A-Z]{2,4}\d{2,5}[A-Z]?(?:/\d+)?|\d{4,6}/\d+|[A-Z]\d{2,4}[A-Z]?(?:/\d+)?|\d{5,6})\s*$").unwrap()
});

static RE_NAVIGATION_MARKERS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^\s*(?:TURN\s+OVER\s*[\p{So}]*|TURN\s+OVER\s+FOR\s+THE\s+NEXT\s+QUESTION|END\s+OF\s+QUESTIONS?|CONTINUE\s+ON\s+NEXT\s+PAGE|GO\s+TO\s+NEXT\s+PAGE)\b.*$").unwrap()
});

static RE_TABULAR_MCQ_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*[A-Za-z][A-Za-z/\s().]*(?:/\s*[A-Za-z])\b.*$").unwrap()
});

static RE_TABULAR_MCQ_ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*[A-D][\).]?\s+\d").unwrap()
});

static RE_INLINE_MCQ: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)([A-D]\)[^A-D\n]*(?:\s+[A-D]\)[^A-D\n]*)+)").unwrap()
});

static RE_SPLIT_MCQ: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([A-D]\)[^A-D\n]*)").unwrap()
});

static RE_PAREN_MCQ: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)(\([A-D]\)[^(\n]*(?:\s+\([A-D]\)[^(\n]*)+)").unwrap()
});

static RE_SPLIT_PAREN_MCQ: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(\([A-D]\)[^(\n]*)").unwrap()
});

static RE_TABLE_ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^(\|.*?\|)\s*$").unwrap()
});

static RE_OPTION_ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\|\s*([A-D]|\([A-D]\))").unwrap()
});

static RE_HEADER_COLS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\s{2,}|\t+").unwrap()
});

static RE_OPT_ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([A-D])[\).]?\s+(.+)$").unwrap()
});

static RE_VISUAL_OPTS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)([A-D]\))\s*(\[DIAGRAM_PLACEHOLDER\])").unwrap()
});

static RE_DIAGRAM_OPT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(\[DIAGRAM_PLACEHOLDER\])").unwrap()
});

static RE_OPT_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*([A-D]\))\s+(.+)$").unwrap()
});

static RE_PAREN_OPT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(\([A-D]\))\s+(.+)$").unwrap()
});

static RE_SUBPART: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)((?:^|\n\n)\s*)(\([a-z]\)|\([ivx]+\))\s+").unwrap()
});

static RE_MCQ_BOUNDARY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)([A-D]\)|\([A-D]\))").unwrap()
});

static RE_MARK_TAG_MATCH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\*\*\[\s*\d+\s+marks?\s*\]\*\*").unwrap()
});

static RE_MCQ_START: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)((?:^|\n\n?))([A-D]\)|\([A-D]\))").unwrap()
});

static RE_REMOVE_MARKS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r" \*\*\[\s*\d+\s+marks?\s*\]\*\*").unwrap()
});

static RE_NAV_TEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^\s*(?:TURN\s+OVER|END\s+OF\s+QUESTIONS?|CONTINUE\s+ON\s+NEXT\s+PAGE|GO\s+TO\s+NEXT\s+PAGE)[\s\p{So}]*$").unwrap()
});

// ══════════════════════════════════════════════════════════════════════════
// Minimal Post-Processing — Only basic typo fixes that are safe as string ops
// ══════════════════════════════════════════════════════════════════════════

/// Ligature cleanup (exact replacements, zero false positives)
pub fn clean_ligatures(s: &str) -> String {
    s.replace('\u{fb00}', "ff")    // ﬀ
     .replace('\u{fb03}', "ffi")   // ﬃ
     .replace('\u{fb01}', "fi")    // ﬁ
     .replace('\u{fb02}', "fl")    // ﬂ
     .replace('\u{fb04}', "ffl")   // ﬄ
     .replace('\u{fb05}', "st")    // ﬅ
     .replace('\u{fb06}', "st")    // ﬆ
}

/// Fix space after backslash for known LaTeX commands: "\ begin" -> "\begin", "\ sqrt" -> "\sqrt"
/// Uses a single compiled regex for all known commands. Safe, fast, no look-arounds.
pub fn fix_basic_latex_typos(text: &str) -> String {
    RE_LATEX_SPACED_CMD.replace_all(text, |caps: &regex::Captures| {
        format!("\\{}", &caps[1])
    }).to_string()
}

/// Remove exam-board boilerplate: MARK SCHEME, GCSE, A-LEVEL, BLANK PAGE, DO NOT WRITE, Turn over, etc.
fn remove_artifact_boilerplate(text: &str) -> String {
    RE_ARTIFACT_BOILERPLATE.replace_all(text, "").to_string()
}

/// Remove margin artifact warnings ("Do not write outside the box", "Do not write in this area")
fn remove_margin_artifacts(text: &str) -> String {
    RE_MARGIN_ARTIFACTS.replace_all(text, "").to_string()
}

/// Remove alphanumeric paper/serial codes (exam board codes, session identifiers)
fn remove_serial_codes(text: &str) -> String {
    RE_SERIAL_CODES.replace_all(text, "").to_string()
}

/// Remove navigation markers ("Turn over ►", "Turn over for the next question", "END OF QUESTIONS")
fn remove_navigation_markers(text: &str) -> String {
    RE_NAVIGATION_MARKERS.replace_all(text, "").to_string()
}

/// Remove page artifacts: page numbers, barcodes, registration marks, footer codes
fn remove_page_artifacts(text: &str) -> String {
    let cleaned = RE_PAGE_ARTIFACTS.replace_all(text, "").to_string();
    RE_FOOTER_CODES.replace_all(&cleaned, "").to_string()
}

/// Remove leading question numbers and OCR artifacts from start of question text
/// e.g., "1 9 O box is the centre..." -> "box is the centre..."
/// "3 1 27Mg 12 can decay..." -> "27Mg 12 can decay..."
/// "Q5 Find the value of x." -> "Find the value of x."
/// Does NOT remove "Question N:" headers or AQA decimal parts like "02.1"
/// Does NOT remove sub-part identifiers like "(a)", "(i)", "1.1"
fn remove_leading_number_artifacts(text: &str) -> String {
    RE_LEADING_NUMBER_ARTIFACTS.replace_all(text, |caps: &regex::Captures| {
        let prefix = &caps[1];
        if prefix == "\n\n" {
            "\n\n".to_string()
        } else {
            String::new()
        }
    }).to_string()
}

/// Convert AQA decimal sub-parts: "02.1" -> "(a)", "02.2" -> "(b)", "02 5" -> "(e)"
/// Only applies when the pattern appears as a sub-part label (start of line or after whitespace)
fn convert_aqa_decimal_parts(text: &str) -> String {
    RE_AQA_DECIMAL_PART.replace_all(text, |caps: &regex::Captures| {
        let _whole: u32 = caps[1].parse().unwrap_or(0);
        let part: u32 = caps[2].parse().unwrap_or(0);
        if part >= 1 && part <= 26 {
            let letter = (b'a' + (part - 1) as u8) as char;
            format!("\n\n({})", letter)
        } else {
            caps[0].to_string() // fallback, shouldn't happen
        }
    }).to_string()
}

/// Ensure sub-part labels (a), (b), (i), (ii) have double newline before them
fn ensure_subpart_spacing(text: &str) -> String {
    RE_SUBPART_LABEL.replace_all(text, |caps: &regex::Captures| {
        let _prefix = &caps[1];
        let whitespace = &caps[2];
        let label = &caps[3];
        let following_space = &caps[4];
        format!("\n\n{}{}{}", label, whitespace, following_space)
    }).to_string()
}

/// Fix MCQ option flattening: ensure each option A) B) C) D) or (A) (B) (C) (D) is on its own line
fn fix_mcq_option_flattening(text: &str) -> String {
    // First pass: handle uppercase A) B) C) D) format
    let result = RE_INLINE_MCQ.replace_all(text, |caps: &regex::Captures| {
        let matched = &caps[1];
        let parts: Vec<String> = RE_SPLIT_MCQ.find_iter(matched)
            .map(|m| m.as_str().trim().to_string())
            .collect();
        format!("\n\n{}", parts.join("\n\n"))
    }).to_string();

    // Second pass: handle (A) (B) (C) (D) format
    RE_PAREN_MCQ.replace_all(&result, |caps: &regex::Captures| {
        let matched = &caps[1];
        let parts: Vec<String> = RE_SPLIT_PAREN_MCQ.find_iter(matched)
            .map(|m| m.as_str().trim().to_string())
            .collect();
        format!("\n\n{}", parts.join("\n\n"))
    }).to_string()
}

/// Fix tabular MCQ options: if options appear in a table, convert to clean list or proper markdown table
fn fix_tabular_mcq_options(text: &str) -> String {
    if RE_TABLE_MCQ_ROW.is_match(text) {
        // Ensure existing markdown tables are well-formatted
        let cleaned = RE_TABLE_ROW.replace_all(text, "$1\n").to_string();
        return add_table_header_if_missing(&cleaned);
    }

    let lines: Vec<&str> = text.lines().collect();
    if lines.len() >= 2 {
        let mut has_header = false;
        let mut has_option_rows = false;

        let non_empty: Vec<&str> = lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();

        if non_empty.len() >= 2 {
            if RE_TABULAR_MCQ_HEADER.is_match(non_empty[0]) {
                has_header = true;
            }
            for line in &non_empty[1..] {
                if RE_TABULAR_MCQ_ROW.is_match(line) {
                    has_option_rows = true;
                    break;
                }
            }
        }

        if has_header && has_option_rows {
            return convert_columnar_to_markdown_table(text);
        }
    }

    text.to_string()
}

/// Add header row to markdown table if missing (detect option letters in first data row)
fn add_table_header_if_missing(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() { return text.to_string(); }

    let first_line = lines[0].trim();
    if first_line.starts_with('|') && first_line.contains("---") {
        return text.to_string(); // Already has header
    }

    if RE_OPTION_ROW.is_match(first_line) {
        let col_count = first_line.matches('|').count().saturating_sub(1);
        if col_count >= 2 {
            let mut header = String::from("| Option");
            for i in 1..col_count {
                header.push_str(&format!(" | Column {}", i));
            }
            header.push_str(" |");

            let sep = "|".repeat(col_count + 1).replace("|", "|---");
            let mut result = String::new();
            result.push_str(&header);
            result.push('\n');
            result.push_str(&sep);
            result.push('\n');
            for line in &lines {
                result.push_str(line);
                result.push('\n');
            }
            return result.trim().to_string();
        }
    }

    text.to_string()
}

/// Convert columnar data (header + option rows without pipes) to markdown table
fn convert_columnar_to_markdown_table(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    if lines.len() < 2 { return text.to_string(); }

    let header_text = lines[0];
    let option_lines = &lines[1..];

    let header_cols: Vec<&str> = RE_HEADER_COLS
        .split(header_text)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    if header_cols.len() < 2 { return text.to_string(); }

    let mut rows = Vec::new();
    for line in option_lines {
        if let Some(caps) = RE_OPT_ROW.captures(line) {
            let opt = caps[1].to_string();
            let rest = caps[2].trim();
            let vals: Vec<&str> = rest.split_whitespace().collect();
            let mut row = vec![format!("**{}**", opt)];
            row.extend(vals.iter().map(|v| v.to_string()));
            rows.push(row);
        }
    }

    if rows.is_empty() { return text.to_string(); }

    let mut result = String::new();

    let mut header = String::from("| Option");
    for col in &header_cols {
        header.push_str(&format!(" | {}", col));
    }
    header.push_str(" |");
    result.push_str(&header);
    result.push('\n');

    let mut sep = String::from("|");
    for _ in 0..header_cols.len() {
        sep.push_str("---|");
    }
    result.push_str(&sep);
    result.push('\n');

    for row in rows {
        let mut row_str = String::from("|");
        for cell in row {
            row_str.push_str(&format!(" {} |", cell));
        }
        result.push_str(&row_str);
        result.push('\n');
    }

    result.trim().to_string()
}

/// Fix visual MCQ gibberish: "Which graph is correct?" followed by [DIAGRAM_PLACEHOLDER] options
fn fix_visual_mcq_gibberish(text: &str) -> String {
    let result = RE_VISUAL_MCQ_LEADIN.replace_all(text, |caps: &regex::Captures| {
        format!("{}\n\n", caps[1].trim())
    }).to_string();

    let result = RE_VISUAL_OPTS.replace_all(&result, "$1 $2\n\n").to_string();
    let result = RE_DIAGRAM_OPT.replace_all(&result, "[VISUAL_MCQ_PLACEHOLDER]").to_string();
    let result = RE_VISUAL_MCQ_GIBBERISH.replace_all(&result, "").to_string();

    let result = remove_margin_artifacts(&result);
    let result = remove_serial_codes(&result);
    remove_navigation_markers(&result)
}

/// Format MCQ options as markdown list items with bold keys: "A) 4N" -> "- **A** 4N"
fn format_mcq_options_bold(text: &str) -> String {
    let result = RE_OPT_LINE.replace_all(text, "- **$1** $2").to_string();
    RE_PAREN_OPT.replace_all(&result, "- **$1** $2").to_string()
}

/// Fix mark allocation misplacement: ensure **[X marks]** appears at end of sub-part, before MCQ options
fn fix_mark_allocation(text: &str) -> String {
    let mut marks: Vec<String> = Vec::new();
    for cap in RE_MARK_TAG.find_iter(text) {
        marks.push(cap.as_str().to_string());
    }

    if marks.is_empty() {
        return text.to_string();
    }

    let result = RE_MARK_TAG.replace_all(text, "").to_string();

    let parts: Vec<String> = RE_SUBPART.split(&result).map(|s| s.to_string()).collect();

    let label_data: Vec<(String, String)> = RE_SUBPART.captures_iter(&result).map(|caps| {
        let prefix = caps[1].to_string();
        let label = caps[2].to_string();
        (prefix, label)
    }).collect();

    if !parts.is_empty() && !marks.is_empty() && parts.len() == label_data.len() + 1 {
        let mut rebuilt = String::new();
        if !parts[0].trim().is_empty() {
            rebuilt.push_str(parts[0].trim());
        }

        for (i, (prefix, label)) in label_data.iter().enumerate() {
            rebuilt.push_str(prefix);
            rebuilt.push_str(label);
            rebuilt.push_str(" ");
            rebuilt.push_str(parts[i + 1].trim());
            if i < marks.len() {
                rebuilt.push_str(" ");
                rebuilt.push_str(&marks[i]);
            }
            rebuilt.push_str("\n\n");
        }
        let result = rebuilt.trim().to_string();

        let mut mark_after_mcq = false;
        if RE_MCQ_BOUNDARY.is_match(&result) {
            if let Some(mcq_pos) = RE_MCQ_BOUNDARY.find(&result).map(|m| m.start()) {
                if let Some(mark_pos) = RE_MARK_TAG_MATCH.find(&result).map(|m| m.start()) {
                    if mark_pos > mcq_pos {
                        mark_after_mcq = true;
                    }
                }
            }
        }

        if !mark_after_mcq {
            return result;
        }
    }

    if RE_MCQ_START.is_match(&result) && !marks.is_empty() {
        let first_mark = marks[0].clone();
        let mut modified = RE_MCQ_START.replace(&result, |caps: &regex::Captures| {
            format!("{}{} {}", &caps[1], first_mark, &caps[2])
        }).to_string();
        if marks.len() > 1 {
            for mark in &marks[1..] {
                modified.push_str(" ");
                modified.push_str(mark);
            }
        }
        modified = RE_REMOVE_MARKS.replace_all(&modified, "").to_string();
        return modified;
    }

    let re_fix_spacing = RE_MARK_TAG.replace_all(&result, " **[${1} marks]**");
    re_fix_spacing.to_string()
}

/// Normalize excessive newlines (more than 2 consecutive) to exactly 2
fn normalize_newlines(text: &str) -> String {
    RE_EXCESSIVE_NEWLINES.replace_all(text, "\n\n").to_string()
}

/// Remove navigational text like "Turn over ►", "END OF QUESTIONS"
fn remove_navigational_text(text: &str) -> String {
    RE_NAV_TEXT.replace_all(text, "").to_string()
}

/// Main post-processing entry point - comprehensive fixes for 6 error categories
pub fn clean_marker_markdown(content: &str) -> String {
    if content.trim().is_empty() {
        return String::new();
    }

    let mut cleaned = clean_ligatures(content);
    cleaned = fix_basic_latex_typos(&cleaned);

    // 1. ARTIFACT BLEED: Remove exam-board boilerplate and page artifacts
    // PRE-CLEAN: Remove margin warnings, header/footer text, and navigation boilerplate BEFORE card-splitting logic runs
    cleaned = remove_artifact_boilerplate(&cleaned);
    cleaned = remove_page_artifacts(&cleaned);
    cleaned = remove_navigational_text(&cleaned);
    cleaned = remove_margin_artifacts(&cleaned);
    cleaned = remove_serial_codes(&cleaned);
    cleaned = remove_navigation_markers(&cleaned);

    // 2. NUMBER-AGNOSTIC FAILURE: Convert AQA decimals FIRST (before leading artifact removal),
    // then remove leading question numbers/OCR artifacts, ensure sub-part spacing
    cleaned = convert_aqa_decimal_parts(&cleaned);
    cleaned = remove_leading_number_artifacts(&cleaned);
    cleaned = ensure_subpart_spacing(&cleaned);

    // 3. MCQ OPTION FLATTENING: Separate inline options onto own lines
    cleaned = fix_mcq_option_flattening(&cleaned);

    // 4. TABULAR OPTION DESTRUCTION: Handle table-formatted options
    cleaned = fix_tabular_mcq_options(&cleaned);

    // 5. VISUAL MCQ GIBBERISH: Format image-based MCQs properly, remove OCR gibberish
    cleaned = fix_visual_mcq_gibberish(&cleaned);

    // 6. MARK ALLOCATION MISPLACEMENT: Move marks to end of sub-parts, before MCQ options
    cleaned = fix_mark_allocation(&cleaned);

    // 7. BOLD OPTION FORMATTING: Format MCQ options as markdown list items with bold keys
    cleaned = format_mcq_options_bold(&cleaned);

    // Final cleanup: normalize excessive newlines
    cleaned = normalize_newlines(&cleaned);

    cleaned.trim().to_string()
}

// ══════════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_ligatures() {
        // Test input: ff (fb00), fi (fb01), fl (fb02), ffi (fb03), ffl (fb04), st (fb05), st (fb06)
        let input = "\u{fb00}\u{fb01}\u{fb02}\u{fb03}\u{fb04}\u{fb05}\u{fb06}";
        let expected = "fffiflffifflstst";
        assert_eq!(clean_ligatures(input), expected);
    }

    #[test]
    fn test_fix_basic_latex_typos_spaced_commands() {
        let input = r"\ begin{pmatrix} 1 & 2 \ end{pmatrix}";
        let expected = r"\begin{pmatrix} 1 & 2 \end{pmatrix}";
        assert_eq!(fix_basic_latex_typos(input), expected);
    }

    #[test]
    fn test_fix_basic_latex_typos_multiple_commands() {
        let input = r"\ text{hello} \ frac{1}{2} \ hline";
        let expected = r"\text{hello} \frac{1}{2} \hline";
        assert_eq!(fix_basic_latex_typos(input), expected);
    }

    #[test]
    fn test_fix_basic_latex_typos_preserves_real_commands() {
        let input = r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}";
        assert_eq!(fix_basic_latex_typos(input), input);
    }

    #[test]
    fn test_clean_marker_markdown_basic() {
        let input = r"\ begin{pmatrix} 1 & 2 \ end{pmatrix}";
        let output = clean_marker_markdown(input);
        assert!(output.contains(r"\begin{pmatrix}"));
        assert!(output.contains(r"\end{pmatrix}"));
    }

    #[test]
    fn test_clean_marker_markdown_empty() {
        assert_eq!(clean_marker_markdown(""), "");
        assert_eq!(clean_marker_markdown("   "), "");
    }

    // === ARTIFACT BLEED TESTS ===
    #[test]
    fn test_remove_mark_scheme_boilerplate() {
        let input = "MARK SCHEME\n\nQuestion 1: Prove that...\n\nGCSE MATHEMATICS";
        let output = clean_marker_markdown(input);
        assert!(!output.contains("MARK SCHEME"));
        assert!(!output.contains("GCSE"));
        assert!(output.contains("Question 1"));
    }

    #[test]
    fn test_remove_blank_page_and_turn_over() {
        let input = "Question 1 content.\n\nBLANK PAGE\n\nTURN OVER\n\nQuestion 2 content.";
        let output = clean_marker_markdown(input);
        assert!(!output.contains("BLANK PAGE"));
        assert!(!output.contains("TURN OVER"));
        assert!(output.contains("Question 1"));
        assert!(output.contains("Question 2"));
    }

    #[test]
    fn test_remove_page_numbers_and_barcodes() {
        let input = "Question 1\n\nPage 3\n\n[Barcode]\n\nQuestion 2";
        let output = clean_marker_markdown(input);
        assert!(!output.contains("Page 3"));
        assert!(!output.contains("[Barcode]"));
    }

    #[test]
    fn test_remove_footer_codes() {
        let input = "Question 1\n\nIB/M/Jun21/7408/2\n\nQuestion 2";
        let output = clean_marker_markdown(input);
        assert!(!output.contains("IB/M/Jun21/7408/2"));
    }

    #[test]
    fn test_remove_navigational_text() {
        let input = "Question 1\n\nTurn over ►\n\nEND OF QUESTIONS\n\nQuestion 2";
        let output = clean_marker_markdown(input);
        assert!(!output.contains("Turn over"));
        assert!(!output.contains("END OF QUESTIONS"));
    }

    // === NUMBER-AGNOSTIC FAILURE TESTS ===
    #[test]
    fn test_convert_aqa_decimal_parts() {
        let input = "Question 2\n\n02.1 First part\n\n02.2 Second part";
        let output = clean_marker_markdown(input);
        assert!(output.contains("(a)"));
        assert!(output.contains("(b)"));
        assert!(!output.contains("02.1"));
        assert!(!output.contains("02.2"));
    }

    #[test]
    fn test_convert_aqa_spaced_parts() {
        let input = "Question 3\n\n03 5 Fifth part";
        let output = clean_marker_markdown(input);
        assert!(output.contains("(e)"));
    }

    #[test]
    fn test_ensure_subpart_spacing() {
        let input = "Intro text\n(a) Part A\n(b) Part B";
        let output = clean_marker_markdown(input);
        assert!(output.contains("\n\n(a)"));
        assert!(output.contains("\n\n(b)"));
    }

    #[test]
    fn test_roman_numeral_subparts() {
        let input = "Text\n(i) First\n(ii) Second";
        let output = clean_marker_markdown(input);
        assert!(output.contains("\n\n(i)"));
        assert!(output.contains("\n\n(ii)"));
    }

    #[test]
    fn test_remove_leading_question_number() {
        // "1 9 O box is the centre..." -> "box is the centre..."
        let input = "1 9 O box is the centre of rotation.";
        let output = clean_marker_markdown(input);
        assert!(!output.starts_with("1 9 O"));
        assert!(output.contains("box is the centre"));
    }

    #[test]
    fn test_remove_leading_question_number_with_physics() {
        // "3 1 27Mg 12 can decay..." -> "27Mg 12 can decay..."
        let input = "3 1 27Mg 12 can decay by beta-plus emission.";
        let output = clean_marker_markdown(input);
        assert!(!output.starts_with("3 1"));
        assert!(output.contains("27Mg 12 can decay"));
    }

    #[test]
    fn test_remove_leading_q_prefix() {
        let input = "Q5 Find the value of x.";
        let output = clean_marker_markdown(input);
        assert!(!output.starts_with("Q5"));
        assert!(output.contains("Find the value of x"));
    }

    // === MCQ OPTION FLATTENING TESTS ===
    #[test]
    fn test_fix_mcq_inline_options_uppercase() {
        let input = "Which is correct? A) Option one B) Option two C) Option three D) Option four";
        let output = clean_marker_markdown(input);
        // Now they're separated AND bolded
        assert!(output.contains("- **A)** Option one"));
        assert!(output.contains("- **B)** Option two"));
        assert!(output.contains("- **C)** Option three"));
        assert!(output.contains("- **D)** Option four"));
    }

    #[test]
    fn test_fix_mcq_inline_options_parenthesized() {
        let input = "Choose: (A) First (B) Second (C) Third (D) Fourth";
        let output = clean_marker_markdown(input);
        // Now they're separated AND bolded
        assert!(output.contains("- **(A)** First"));
        assert!(output.contains("- **(B)** Second"));
        assert!(output.contains("- **(C)** Third"));
        assert!(output.contains("- **(D)** Fourth"));
    }

    // === TABULAR OPTION DESTRUCTION TESTS ===
    #[test]
    fn test_fix_tabular_mcq_options() {
        let input = "| A | 200 | 0.50 | B | 200 | 0.45 |\n| C | 150 | 0.40 | D | 150 | 0.35 |";
        let output = clean_marker_markdown(input);
        // Should preserve as table with proper newlines
        assert!(output.contains("A | 200 | 0.50"));
        assert!(output.contains("B | 200 | 0.45"));
    }

    // === VISUAL MCQ GIBBERISH TESTS ===
    #[test]
    fn test_fix_visual_mcq_lead_in() {
        let input = "Which graph is correct? A)\n[DIAGRAM_PLACEHOLDER]\nB)\n[DIAGRAM_PLACEHOLDER]";
        let output = clean_marker_markdown(input);
        assert!(output.contains("Which graph is correct?"));
        // Should have bold formatted options as list items
        assert!(output.contains("- **A)** [VISUAL_MCQ_PLACEHOLDER]"));
        assert!(output.contains("- **B)** [VISUAL_MCQ_PLACEHOLDER]"));
    }

    #[test]
    fn test_fix_visual_mcq_diagram_options() {
        let input = "Which diagram shows the correct shape? A) [DIAGRAM_PLACEHOLDER] B) [DIAGRAM_PLACEHOLDER]";
        let output = clean_marker_markdown(input);
        // Now uses VISUAL_MCQ_PLACEHOLDER and bold formatting
        eprintln!("OUTPUT: {:?}", output);
        assert!(output.contains("- **A)** [VISUAL_MCQ_PLACEHOLDER]"));
        assert!(output.contains("- **B)** [VISUAL_MCQ_PLACEHOLDER]"));
    }

    #[test]
    fn test_remove_visual_mcq_gibberish() {
        // OCR produces "A B C D A B C D" for visual options
        let input = "Which graph is correct?\nA B C D A B C D\nA)\n[DIAGRAM_PLACEHOLDER]";
        let output = clean_marker_markdown(input);
        assert!(!output.contains("A B C D A B C D"));
    }

    #[test]
    fn test_visual_mcq_replaces_diagram_placeholder() {
        let input = "Which graph is correct?\nA) [DIAGRAM_PLACEHOLDER]\nB) [DIAGRAM_PLACEHOLDER]";
        let output = clean_marker_markdown(input);
        assert!(output.contains("[VISUAL_MCQ_PLACEHOLDER]"));
        assert!(!output.contains("[DIAGRAM_PLACEHOLDER]"));
    }

    #[test]
    fn test_visual_mcq_strips_margin_artifacts() {
        let input = "Which graph is correct?\nDo not write outside the 2 1 box\nA) [DIAGRAM_PLACEHOLDER]";
        let output = clean_marker_markdown(input);
        assert!(!output.contains("Do not write outside the"));
        assert!(output.contains("[VISUAL_MCQ_PLACEHOLDER]"));
    }

    // === BOLD OPTION FORMATTING TESTS ===
    #[test]
    fn test_format_mcq_options_bold_uppercase() {
        let input = "A) 4N\nB) 8N\nC) 12N\nD) 16N";
        let output = clean_marker_markdown(input);
        assert!(output.contains("- **A)** 4N"));
        assert!(output.contains("- **B)** 8N"));
        assert!(output.contains("- **C)** 12N"));
        assert!(output.contains("- **D)** 16N"));
    }

    #[test]
    fn test_format_mcq_options_bold_parenthesized() {
        let input = "(A) Option one\n(B) Option two";
        let output = clean_marker_markdown(input);
        assert!(output.contains("- **(A)** Option one"));
        assert!(output.contains("- **(B)** Option two"));
    }

    // === TABULAR MCQ OPTIONS TESTS ===
    #[test]
    fn test_tabular_mcq_with_header() {
        let input = "Secondary voltage / V  Secondary current / A\nA 200 0.50\nB 200 0.45\nC 150 0.40\nD 150 0.35";
        let output = clean_marker_markdown(input);
        assert!(output.contains("| Option"));
        assert!(output.contains("| Secondary voltage / V"));
        assert!(output.contains("| Secondary current / A"));
        assert!(output.contains("|---"));
        assert!(output.contains("**A**"));
        assert!(output.contains("200"));
        assert!(output.contains("0.50"));
    }

    #[test]
    fn test_tabular_mcq_existing_markdown_table_with_header() {
        let input = "| Option | Voltage (V) | Current (A) |\n| A | 200 | 0.50 |\n| B | 200 | 0.45 |";
        let output = clean_marker_markdown(input);
        // Should keep existing header
        assert!(output.contains("Option"));
        assert!(output.contains("Voltage"));
        assert!(output.contains("Current"));
    }

    // === MARK ALLOCATION MISPLACEMENT TESTS ===
    #[test]
    fn test_fix_marks_at_subpart_end() {
        let input = "Question 1\n\n(a) First part **[2 marks]**\n\n(b) Second part **[3 marks]**";
        let output = clean_marker_markdown(input);
        // Marks should be at end of each sub-part
        assert!(output.contains("(a) First part **[2 marks]**"));
        assert!(output.contains("(b) Second part **[3 marks]**"));
    }

    #[test]
    fn test_marks_not_at_question_start() {
        // Marks shouldn't float at the very beginning
        let input = "**[5 marks]**\n\nQuestion 1: Do something.";
        let output = clean_marker_markdown(input);
        // The mark tag should not be at the start
        assert!(!output.starts_with("**[5 marks]**"));
    }

    #[test]
    fn test_marks_before_mcq_options() {
        // Marks should appear before MCQ options, not after
        let input = "(a) Calculate the force. **[2 marks]**\nA) 10 N B) 20 N C) 30 N D) 40 N";
        let output = clean_marker_markdown(input);
        // Marks should be before options
        let marks_pos = output.find("**[2 marks]**").unwrap_or(usize::MAX);
        let option_pos = output.find("A) 10 N").unwrap_or(usize::MAX);
        assert!(marks_pos < option_pos, "Marks should appear before MCQ options");
    }

    // === NEWLINE NORMALIZATION TESTS ===
    #[test]
    fn test_normalize_excessive_newlines() {
        let input = "Part 1\n\n\n\nPart 2\n\n\n\n\nPart 3";
        let output = clean_marker_markdown(input);
        assert!(output.contains("Part 1\n\nPart 2"));
        assert!(output.contains("Part 2\n\nPart 3"));
        assert!(!output.contains("\n\n\n"));
    }

    // === INTEGRATION TESTS ===
    #[test]
    fn test_full_pipeline_complex_question() {
        let input = r#"MARK SCHEME - GCSE MATHEMATICS

BLANK PAGE

Question 2

02.1 Prove that x^2 + 1 = 0 has no real solutions. **[2 marks]**

02.2 Find the complex roots. **[3 marks]**

A) Root 1 B) Root 2 C) Root 3 D) Root 4

TURN OVER

Page 2"#;
        let output = clean_marker_markdown(input);

        // No boilerplate
        assert!(!output.contains("MARK SCHEME"));
        assert!(!output.contains("GCSE"));
        assert!(!output.contains("BLANK PAGE"));
        assert!(!output.contains("TURN OVER"));
        assert!(!output.contains("Page 2"));

        // AQA decimals converted
        assert!(output.contains("(a)"));
        assert!(output.contains("(b)"));

        // Sub-part spacing
        assert!(output.contains("\n\n(a)"));
        assert!(output.contains("\n\n(b)"));

        // MCQ options separated AND bolded
        assert!(output.contains("- **A)** Root 1"));
        assert!(output.contains("- **B)** Root 2"));
        assert!(output.contains("- **C)** Root 3"));
        assert!(output.contains("- **D)** Root 4"));

        // Marks at end of sub-parts
        assert!(output.contains("(a)"));
        assert!(output.contains("**[2 marks]**"));
    }

    #[test]
    fn test_visual_mcq_with_gibberish_cleanup() {
        let input = "Which graph represents the function?\nA B C D A B C D\nA) [DIAGRAM_PLACEHOLDER]\nB) [DIAGRAM_PLACEHOLDER]\nC) [DIAGRAM_PLACEHOLDER]\nD) [DIAGRAM_PLACEHOLDER]";
        let output = clean_marker_markdown(input);
        assert!(!output.contains("A B C D A B C D"));
        // Now uses VISUAL_MCQ_PLACEHOLDER and bold formatting
        assert!(output.contains("- **A)** [VISUAL_MCQ_PLACEHOLDER]"));
        assert!(output.contains("- **B)** [VISUAL_MCQ_PLACEHOLDER]"));
        assert!(output.contains("- **C)** [VISUAL_MCQ_PLACEHOLDER]"));
        assert!(output.contains("- **D)** [VISUAL_MCQ_PLACEHOLDER]"));
    }

    #[test]
    fn test_tabular_mcq_preserved() {
        let input = "Select the correct voltage and current:\n| Option | Voltage (V) | Current (A) |\n| A | 200 | 0.50 |\n| B | 200 | 0.45 |\n| C | 150 | 0.40 |\n| D | 150 | 0.35 |";
        let output = clean_marker_markdown(input);
        // Should preserve table structure
        assert!(output.contains("Voltage"));
        assert!(output.contains("Current"));
        assert!(output.contains("Option"));
    }
}