use regex::Regex;
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
// Minimal Post-Processing — Only basic typo fixes that are safe as string ops
// ══════════════════════════════════════════════════════════════════════════

/// Ligature cleanup (exact replacements, zero false positives)
pub fn clean_ligatures(s: &str) -> String {
    s.replace('ﬀ', "ff")
     .replace('ﬁ', "fi")
     .replace('ﬂ', "fl")
     .replace('ﬃ', "ffi")
     .replace('ﬄ', "ffl")
     .replace('ﬅ', "st")
     .replace('ﬆ', "st")
}

/// Fix space after backslash for known LaTeX commands: "\ begin" -> "\begin", "\ sqrt" -> "\sqrt"
/// Uses a single compiled regex for all known commands. Safe, fast, no look-arounds.
static RE_LATEX_SPACED_CMD: OnceLock<Regex> = OnceLock::new();

pub fn fix_basic_latex_typos(text: &str) -> String {
    let re = RE_LATEX_SPACED_CMD.get_or_init(|| {
        Regex::new(r"\\ +(begin|end|frac|dfrac|cfrac|sqrt|text|textbf|textit|mathbf|mathit|mathrm|mathbb|mathcal|operatorname|pmatrix|bmatrix|vmatrix|matrix|array|cases|aligned|theta|lambda|alpha|beta|gamma|delta|Delta|pi|mu|sigma|omega|Omega|phi|Phi|psi|Psi|times|div|pm|mp|leq|geq|neq|approx|sim|equiv|subset|supset|subseteq|supseteq|in|notin|forall|exists|infty|partial|nabla|cos|sin|tan|sec|csc|cot|cosh|sinh|tanh|ln|log|exp|int|iint|iiint|oint|sum|prod|lim|vec|hat|bar|dot|ddot|tilde|binom|quad|qquad|hline|hlinex|cline|multicolumn|multirow)\b").unwrap()
    });

    re.replace_all(text, r"\\$1").to_string()
}

/// Main post-processing entry point - minimal, fast, only basic typo fixes
pub fn clean_marker_markdown(content: &str) -> String {
    if content.trim().is_empty() {
        return String::new();
    }

    let mut cleaned = clean_ligatures(content);
    cleaned = fix_basic_latex_typos(&cleaned);

    cleaned
}

// ══════════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_ligatures() {
        let input = "ﬀﬁﬂﬃﬄﬅﬆ";
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
}