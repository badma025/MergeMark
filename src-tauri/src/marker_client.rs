use regex::{Captures, Regex};
use std::sync::OnceLock;

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
// Post-Processing & Markdown / LaTeX Repair
// ══════════════════════════════════════════════════════════════════════════

static RE_PMATRIX: OnceLock<Regex> = OnceLock::new();
static RE_BMATRIX: OnceLock<Regex> = OnceLock::new();
static RE_VMATRIX: OnceLock<Regex> = OnceLock::new();
static RE_MATRIX: OnceLock<Regex> = OnceLock::new();
static RE_ARRAY: OnceLock<Regex> = OnceLock::new();

static RE_FRACTURED_EQUALS: OnceLock<Regex> = OnceLock::new();
static RE_FRACTURED_EXP_N_EQ: OnceLock<Regex> = OnceLock::new();
static RE_FRACTURED_EXP_INV_EQ: OnceLock<Regex> = OnceLock::new();
static RE_FRACTURED_EXP_INV: OnceLock<Regex> = OnceLock::new();
static RE_FRACTURED_HTML_EQ: OnceLock<Regex> = OnceLock::new();
static RE_FRACTURED_EQ_OPERATOR: OnceLock<Regex> = OnceLock::new();
static RE_FRACTURED_EQ_BLOCK: OnceLock<Regex> = OnceLock::new();

static RE_RAW_HLINE_ARRAY: OnceLock<Regex> = OnceLock::new();
static RE_RAW_HLINE_MATCH: OnceLock<Regex> = OnceLock::new();

static RE_SUBQUESTION_MARKER: OnceLock<Regex> = OnceLock::new();
static RE_MULTI_NEWLINES: OnceLock<Regex> = OnceLock::new();
static RE_DISPLAY_MATH: OnceLock<Regex> = OnceLock::new();

static RE_EMPTY_BULLET: OnceLock<Regex> = OnceLock::new();
static RE_EMPTY_NUMBERED: OnceLock<Regex> = OnceLock::new();
static RE_EMPTY_HTML_LI: OnceLock<Regex> = OnceLock::new();

static RE_ORPHANED_SUBPARTS: OnceLock<Regex> = OnceLock::new();
static RE_ORPHANED_SUBPARTS_BOLD: OnceLock<Regex> = OnceLock::new();

fn get_matrix_regexes() -> [(&'static str, &'static Regex); 5] {
    let pmat = RE_PMATRIX.get_or_init(|| Regex::new(r"\\begin\{pmatrix\}([\s\S]*?)\\end\{pmatrix\}").unwrap());
    let bmat = RE_BMATRIX.get_or_init(|| Regex::new(r"\\begin\{bmatrix\}([\s\S]*?)\\end\{bmatrix\}").unwrap());
    let vmat = RE_VMATRIX.get_or_init(|| Regex::new(r"\\begin\{vmatrix\}([\s\S]*?)\\end\{vmatrix\}").unwrap());
    let mat = RE_MATRIX.get_or_init(|| Regex::new(r"\\begin\{matrix\}([\s\S]*?)\\end\{matrix\}").unwrap());
    let arr = RE_ARRAY.get_or_init(|| Regex::new(r"\\begin\{array\}(?:\{[^}]*\})?([\s\S]*?)\\end\{array\}").unwrap());

    [
        ("pmatrix", pmat),
        ("bmatrix", bmat),
        ("vmatrix", vmat),
        ("matrix", mat),
        ("array", arr),
    ]
}

/// Helper to fix row separators inside matrix environments without lookarounds.
fn fix_matrix_content(inner: &str, is_bracketed: bool) -> String {
    let mut fixed_lines = Vec::new();

    for line in inner.lines() {
        let mut line_str = line.to_string();
        if is_bracketed {
            line_str = line_str.replace(r"\hline", "");
        }

        let trimmed = line_str.trim_end();
        if trimmed.ends_with(r"\\") {
            // Already correctly terminated with double backslash
            fixed_lines.push(line_str);
        } else if trimmed.ends_with('\\') {
            // Single backslash at line end -> replace with \\
            let prefix = &trimmed[..trimmed.len() - 1].trim_end();
            fixed_lines.push(format!(r"{} \\", prefix));
        } else {
            // Check for single backslash inline row separator (e.g. " 1 & 2 \ 3 & 4 ")
            // If we have " \ " where the backslash is followed by whitespace, convert to " \\ "
            let mut processed = String::with_capacity(line_str.len() + 8);
            let chars: Vec<char> = line_str.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                if chars[i] == '\\' {
                    let is_escaped = i > 0 && chars[i - 1] == '\\';
                    let next_is_bs = i + 1 < chars.len() && chars[i + 1] == '\\';
                    let next_is_space = i + 1 < chars.len() && chars[i + 1].is_whitespace();

                    if !is_escaped && !next_is_bs && next_is_space {
                        // Lone backslash followed by whitespace -> row break
                        processed.push_str(r"\\");
                    } else {
                        processed.push(chars[i]);
                    }
                } else {
                    processed.push(chars[i]);
                }
                i += 1;
            }
            fixed_lines.push(processed);
        }
    }

    fixed_lines.join("\n")
}

/// Repair matrix row separators inside `\begin{pmatrix}` ... `\end{pmatrix}`,
/// `\begin{vmatrix}` ... `\end{vmatrix}`, and other matrix environments.
/// Replaces single stray backslashes (`\`) separating elements/rows with double backslashes (`\\`).
pub fn fix_matrix_rows(text: &str) -> String {
    let mut result = text.to_string();

    for (env, re_env) in get_matrix_regexes() {
        result = re_env.replace_all(&result, |caps: &Captures| {
            let inner = &caps[1];
            let is_bracketed = env == "pmatrix" || env == "bmatrix" || env == "matrix" || env == "vmatrix";
            let fixed_inner = fix_matrix_content(inner, is_bracketed);
            format!(r"\begin{{{}}}{}\end{{{}}}", env, fixed_inner, env)
        }).to_string();
    }

    result
}

/// Fix broken array tables: target `\begin{array}` blocks containing the literal word `hline`
/// (where unescaped) and prepend it with a backslash so it becomes `\hline`.
pub fn fix_broken_array_tables(text: &str) -> String {
    let re_array = RE_RAW_HLINE_ARRAY.get_or_init(|| {
        Regex::new(r"\\begin\{array\}(?:\{[^}]*\})?([\s\S]*?)\\end\{array\}").unwrap()
    });
    let re_raw_hline = RE_RAW_HLINE_MATCH.get_or_init(|| {
        Regex::new(r"(\A|[^\\])\bhline\b").unwrap()
    });

    re_array.replace_all(text, |caps: &Captures| {
        let full = &caps[0];
        let fixed = re_raw_hline.replace_all(full, r"${1}\hline").to_string();
        fixed
    }).to_string()
}

/// Catch and merge fractured display equations split across multiple display blocks.
/// 1. Replace `$$\s*=\s*$$` with `=`
/// 2. Replace `$$\s*\^n\s*=\s*$$` with `^n = `
/// 3. Replace `$$\s*\^{-1}\s*=\s*$$` with `^{-1} = `
/// 4. Replace `$$\s*\^{-1}\s*$$` with `^{-1}`
/// 5. Merge general adjacent display math blocks split by operators or HTML `<p>=</p>` tags.
pub fn merge_fractured_equations(text: &str) -> String {
    let re_equals = RE_FRACTURED_EQUALS.get_or_init(|| {
        Regex::new(r"\$\$\s*=\s*\$\$").unwrap()
    });
    let re_exp_n_eq = RE_FRACTURED_EXP_N_EQ.get_or_init(|| {
        Regex::new(r"\$\$\s*\^n\s*=\s*\$\$").unwrap()
    });
    let re_exp_inv_eq = RE_FRACTURED_EXP_INV_EQ.get_or_init(|| {
        Regex::new(r"\$\$\s*\^\{\s*-1\s*\}\s*=\s*\$\$").unwrap()
    });
    let re_exp_inv = RE_FRACTURED_EXP_INV.get_or_init(|| {
        Regex::new(r"\$\$\s*\^\{\s*-1\s*\}\s*\$\$").unwrap()
    });
    let re_html_eq = RE_FRACTURED_HTML_EQ.get_or_init(|| {
        Regex::new(r"\$\$\s*<p>\s*=\s*</p>\s*\$\$").unwrap()
    });
    let re_op = RE_FRACTURED_EQ_OPERATOR.get_or_init(|| {
        Regex::new(r"\$\$\s*([\s\S]*?)\s*\$\$\s*(?:<p>\s*)?([=+\-~]|\\approx|\\ne|\\leq|\\geq|\\times|\\div|\^\{?[a-zA-Z0-9\+\-]+\}?|\w+\^n)\s*(?:</p>\s*)?\s*\$\$\s*([\s\S]*?)\s*\$\$").unwrap()
    });
    let re_blk = RE_FRACTURED_EQ_BLOCK.get_or_init(|| {
        Regex::new(r"\$\$\s*([\s\S]*?)\s*\$\$\s*\$\$\s*([=+\-~]|\\approx|\\ne|\\leq|\\geq)\s*\$\$\s*\$\$\s*([\s\S]*?)\s*\$\$").unwrap()
    });

    let mut current = text.to_string();

    // Pass 1: Direct literal pattern replacements
    current = re_equals.replace_all(&current, " = ").to_string();
    current = re_exp_n_eq.replace_all(&current, " ^n = ").to_string();
    current = re_exp_inv_eq.replace_all(&current, " ^{-1} = ").to_string();
    current = re_exp_inv.replace_all(&current, " ^{-1} ").to_string();
    current = re_html_eq.replace_all(&current, " = ").to_string();

    // Pass 2: Iterative merge for adjacent display blocks
    for _ in 0..5 {
        let mut changed = false;

        let pass1 = re_op.replace_all(&current, |caps: &Captures| {
            changed = true;
            let left = caps[1].trim();
            let op = caps[2].trim();
            let right = caps[3].trim();
            format!("$$\n{} {} {}\n$$", left, op, right)
        }).to_string();

        let pass2 = re_blk.replace_all(&pass1, |caps: &Captures| {
            changed = true;
            let left = caps[1].trim();
            let op = caps[2].trim();
            let right = caps[3].trim();
            format!("$$\n{} {} {}\n$$", left, op, right)
        }).to_string();

        current = pass2;
        if !changed {
            break;
        }
    }

    current
}

/// Identifies text inside display math blocks `$$ ... $$` and removes any stray
/// inline math delimiters `$` contained within them to prevent KaTeX ParseError crashes.
/// Example: `$$\mathbf{M} = $ \begin{pmatrix}... $$` -> `$$\mathbf{M} = \begin{pmatrix}... $$`
pub fn clean_delimiter_nesting(text: &str) -> String {
    let re_display_math = RE_DISPLAY_MATH.get_or_init(|| {
        Regex::new(r"\$\$([\s\S]*?)\$\$").unwrap()
    });

    re_display_math.replace_all(text, |caps: &Captures| {
        let inner = &caps[1];
        // Strip nested inline math $ inside display $$ blocks
        let cleaned_inner = inner.replace('$', "");
        format!("$${}$$", cleaned_inner)
    }).to_string()
}

/// Removes empty list markers like `- \n` or `* \n` or `1. \n` or `<li></li>`
/// so they don't produce empty <li> elements in the rendered output.
pub fn strip_empty_list_items(text: &str) -> String {
    let re_bullet = RE_EMPTY_BULLET.get_or_init(|| {
        Regex::new(r"(?m)^\s*[-*+]\s*$\r?\n?").unwrap()
    });
    let re_num = RE_EMPTY_NUMBERED.get_or_init(|| {
        Regex::new(r"(?m)^\s*\d+\.\s*$\r?\n?").unwrap()
    });
    let re_li = RE_EMPTY_HTML_LI.get_or_init(|| {
        Regex::new(r"<li>\s*</li>").unwrap()
    });

    let mut result = re_bullet.replace_all(text, "").to_string();
    result = re_num.replace_all(&result, "").to_string();
    result = re_li.replace_all(&result, "").to_string();
    result
}

/// Merges orphaned sub-part headers like `\n\n(a)\n\n(i)` into `\n\n(a) (i)`
/// so that (a) and (i) stay on the same line.
pub fn merge_orphaned_subparts(text: &str) -> String {
    let re_subparts = RE_ORPHANED_SUBPARTS.get_or_init(|| {
        Regex::new(r"(?:\r?\n){2,}\(([a-zA-Z])\)(?:\r?\n){2,}\(([ivxlcdmIVXLCDM]+)\)").unwrap()
    });
    let re_subparts_bold = RE_ORPHANED_SUBPARTS_BOLD.get_or_init(|| {
        Regex::new(r"(?:\r?\n){2,}\*\*\(([a-zA-Z])\)\*\*(?:\r?\n){2,}\*\*\(([ivxlcdmIVXLCDM]+)\)\*\*").unwrap()
    });

    let mut result = re_subparts.replace_all(text, "\n\n($1) ($2)").to_string();
    result = re_subparts_bold.replace_all(&result, "\n\n**($1)** **($2)**").to_string();
    result
}

/// Aggressively enforce hierarchical spacing: any marker like (a), (b), (i), (ii)
/// at the start of a sub-question / sentence must be preceded by a double newline `\n\n`.
pub fn fix_hierarchical_spacing(text: &str) -> String {
    let re_marker = RE_SUBQUESTION_MARKER.get_or_init(|| {
        Regex::new(r"([^\n\s])\s*(?:\r?\n)?\s*(\((?:[a-zA-Z]|\d{1,2}|[ivxlcdmIVXLCDM]+)\)|\*\*\((?:[a-zA-Z]|\d{1,2}|[ivxlcdmIVXLCDM]+)\)\*\*|\*\*[a-zA-Z]\*\*|\*\*[ivxlcdmIVXLCDM]+\*\*)\s+([A-Z0-9\$`'\\\(])").unwrap()
    });
    let re_newlines = RE_MULTI_NEWLINES.get_or_init(|| {
        Regex::new(r"\n{3,}").unwrap()
    });

    // Process line breaks outside of code blocks and display math
    let mut out = String::with_capacity(text.len() + 64);
    let mut in_fence = false;
    let mut in_display_math = false;

    for line in text.split('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            out.push_str(line);
            out.push('\n');
            continue;
        }

        if trimmed == "$$" || (trimmed.starts_with("$$") && trimmed.ends_with("$$") && trimmed.len() > 4) {
            if trimmed == "$$" {
                in_display_math = !in_display_math;
            }
            out.push_str(line);
            out.push('\n');
            continue;
        }

        if in_fence || in_display_math {
            out.push_str(line);
            out.push('\n');
            continue;
        }

        // Apply hierarchical sub-question break insertion
        let line_with_breaks = re_marker.replace_all(line, |caps: &Captures| {
            let prev_char = &caps[1];
            let marker = &caps[2];
            let next_char = &caps[3];
            format!("{}\n\n{} {}", prev_char, marker, next_char)
        });

        out.push_str(&line_with_breaks);
        out.push('\n');
    }

    // Collapse any excessive runs of newlines
    let collapsed = re_newlines.replace_all(&out, "\n\n").to_string();
    collapsed.trim().to_string()
}

/// Comprehensive LaTeX and Markdown sanitizer for organic VLM extraction.
/// Applies fractured equation merging, array table repair, matrix row normalization,
/// delimiter nesting cleanup, empty list stripping, subpart merging, and hierarchical spacing.
pub fn sanitize_latex(content: &str) -> String {
    if content.trim().is_empty() {
        return String::new();
    }

    // 1. Fix fractured display equations (merging display blocks around operators and exponents)
    let mut cleaned = merge_fractured_equations(content);

    // 2. Fix broken array tables with unescaped hline
    cleaned = fix_broken_array_tables(&cleaned);

    // 3. Fix single backslashes in pmatrix, vmatrix, bmatrix, matrix, array
    cleaned = fix_matrix_rows(&cleaned);

    // 4. Fix nested delimiters (strip inner $ inside $$...$$)
    cleaned = clean_delimiter_nesting(&cleaned);

    // 5. Strip empty list items to avoid empty <li> elements
    cleaned = strip_empty_list_items(&cleaned);

    // 6. Enforce clean hierarchical spacing for sub-questions
    cleaned = fix_hierarchical_spacing(&cleaned);

    // 7. Merge orphaned sub-part headers e.g. (a)\n\n(i) into (a) (i)
    cleaned = merge_orphaned_subparts(&cleaned);

    cleaned
}

/// Main post-processing entry point for organic VLM extraction markdown.
pub fn clean_marker_markdown(content: &str) -> String {
    sanitize_latex(content)
}

// ══════════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matrix_row_fix() {
        let input = r#"$$
\begin{pmatrix}
1 & 2 \
3 & 4
\end{pmatrix}
$$"#;
        let fixed = fix_matrix_rows(input);
        assert!(fixed.contains(r"1 & 2 \\"));
        assert!(fixed.contains(r"3 & 4"));
    }

    #[test]
    fn test_vmatrix_row_fix() {
        let input = r#"$$
\begin{vmatrix}
a & b \
c & d
\end{vmatrix}
$$"#;
        let fixed = fix_matrix_rows(input);
        assert!(fixed.contains(r"a & b \\"));
        assert!(fixed.contains(r"c & d"));
    }

    #[test]
    fn test_inline_matrix_row_fix() {
        let input = r#"$$\begin{pmatrix} a & b \ c & d \end{pmatrix}$$"#;
        let fixed = fix_matrix_rows(input);
        assert!(fixed.contains(r"a & b \\ c & d"));
    }

    #[test]
    fn test_orphaned_equals_merge() {
        let input = "$$\n\\mathbf{M}^n\n$$\n=\n$$\n\\begin{pmatrix} 2^n & 0 \\\\ 0 & 3^n \\end{pmatrix}\n$$";
        let merged = merge_fractured_equations(input);
        assert!(!merged.contains("$$\n=\n$$"));
        assert!(merged.contains(r"\mathbf{M}^n = \begin{pmatrix}"));
    }

    #[test]
    fn test_fractured_literal_replacements() {
        let input1 = "$$ A $$ = $$ B $$";
        assert_eq!(merge_fractured_equations(input1), "$$\nA = B\n$$");

        let input2 = "$$\\mathbf{M}$$^n = $$\\begin{pmatrix} 1 & 0 \\\\ 0 & 1 \\end{pmatrix}$$";
        let res2 = merge_fractured_equations(input2);
        assert!(res2.contains(r"\mathbf{M} ^n = \begin{pmatrix}"));

        let input3 = "$$\\mathbf{M}$$$$\\^{-1}$$$$\\mathbf{N}$$";
        let res3 = merge_fractured_equations(input3);
        assert!(res3.contains(r"\mathbf{M} ^{-1} \mathbf{N}"));
    }

    #[test]
    fn test_broken_array_table_hline() {
        let input = r#"$$\begin{array}{cc}
1 & 2 \\
hline
3 & 4
\end{array}$$"#;
        let fixed = fix_broken_array_tables(input);
        assert!(fixed.contains(r"\hline"));
        assert!(!fixed.contains("\nhline"));
    }

    #[test]
    fn test_hierarchical_spacing() {
        let input = "Show that the equation has real roots. (a) Find the discriminant. (b) Hence deduce the range of k.";
        let spaced = fix_hierarchical_spacing(input);
        assert!(spaced.contains("Show that the equation has real roots.\n\n(a) Find"));
        assert!(spaced.contains("Find the discriminant.\n\n(b) Hence"));
    }

    #[test]
    fn test_delimiter_nesting_strip() {
        let input = "$$\nf(x) = $x^2 + 2x$\n$$";
        let cleaned = clean_delimiter_nesting(input);
        assert_eq!(cleaned, "$$\nf(x) = x^2 + 2x\n$$");

        let input2 = "$$\\mathbf{M} = $ \\begin{pmatrix} 1 & 2 \\\\ 3 & 4 \\end{pmatrix}$$";
        let cleaned2 = clean_delimiter_nesting(input2);
        assert_eq!(cleaned2, "$$\\mathbf{M} =  \\begin{pmatrix} 1 & 2 \\\\ 3 & 4 \\end{pmatrix}$$");
    }

    #[test]
    fn test_strip_empty_list_items() {
        let input = "Some instructions:\n-\n- Valid item\n*\n1.\n2. Valid numbered item\n<li></li>";
        let cleaned = strip_empty_list_items(input);
        assert!(!cleaned.contains("\n-\n"));
        assert!(!cleaned.contains("\n*\n"));
        assert!(!cleaned.contains("\n1.\n"));
        assert!(!cleaned.contains("<li></li>"));
        assert!(cleaned.contains("- Valid item"));
        assert!(cleaned.contains("2. Valid numbered item"));
    }

    #[test]
    fn test_merge_orphaned_subparts() {
        let input = "Question introductory text.\n\n(a)\n\n(i) Find the value of k.";
        let merged = merge_orphaned_subparts(input);
        assert!(merged.contains("(a) (i) Find the value of k."));

        let input_bold = "Question text.\n\n**(b)**\n\n**(ii)** Hence prove that.";
        let merged_bold = merge_orphaned_subparts(input_bold);
        assert!(merged_bold.contains("**(b)** **(ii)** Hence prove that."));
    }
}
