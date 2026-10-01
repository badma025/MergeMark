// ── Deterministic content sanitizer (last line of defense) ─────────────────
//
// Runs on EVERY question card immediately before persistence, regardless of
// whether the content came from the LLM, Tier-0 deterministic extraction, or
// the extraction cache. The extraction prompts already mandate correct LaTeX /
// Markdown, but models violate them under long-context pressure; when they do,
// the corruption is deterministic and fixable after the fact. Every fix here
// is conservative: it only fires on patterns that are ALWAYS wrong in exam
// card content (unbalanced `$`, unicode minus in units, OCR margin debris,
// blank lines between MCQ options, ...).
//
// Trap classes covered (see import postmortem):
//   T1  scrambled isotope notation        -> reconstruct_isotope_notation
//   T2  rogue/asymmetric math delimiters  -> balance_math_delimiters
//   T3  sentence-level `$...$` wrapping   -> unwrap_sentence_math
//   T4  unicode minus / plain exponents   -> normalize_unicode_math
//   T6  MCQ list syntax violations        -> tighten_mcq_lists
//   (boilerplate)                         -> strip_ocr_boilerplate

use std::sync::LazyLock;

/// Strip only this question's exact heading, never a following numeric value.
pub fn strip_question_heading(content: &str, number: u32) -> String {
    // Legacy cards may have mistaken the last heading digit plus the article
    // "A" for an ampere quantity. Require this question's exact split number.
    if (10..100).contains(&number) {
        let wrapped = regex::Regex::new(&format!(
            r"\A[ \t]*{}[ \t]+\${}\\[ \t]+\\text\{{A\}}\$[ \t]+", number / 10, number % 10
        )).unwrap();
        if wrapped.is_match(content) { return wrapped.replace(content, "A ").to_string(); }
    }
    let digits = number.to_string().chars().map(|c| c.to_string())
        .collect::<Vec<_>>().join(r"[ \t]*");
    let subpart = regex::Regex::new(&format!(
        r"\A\s*0?[ \t]*{}[ \t]*(?:box[ \t]*)?\.[ \t]*[1-9]\b", digits
    )).unwrap();
    if subpart.is_match(content) { return content.to_string(); }
    let re = regex::Regex::new(&format!(
        r"\A[ \t]*(?:box[ \t]+)?(?:Q(?:uestion)?\.?[ \t]*)?0?[ \t]*{}(?:[.)])?(?:[ \t]+|[ \t]*\r?\n)(?:box[ \t]+)?", digits
    )).unwrap();
    re.replace(content, "").to_string()
}

/// Wrap orphan text commands while preserving existing inline/display math
/// and code. Nested braces are consumed as a balanced group.
pub fn wrap_orphan_latex(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut i = 0;
    let mut math = 0;
    let mut code = false;
    while i < content.len() {
        let rest = &content[i..];
        if rest.starts_with('`') { code = !code; }
        if !code && rest.starts_with('$') && (i == 0 || content.as_bytes()[i-1] != b'\\') {
            let width = if rest.starts_with("$$") { 2 } else { 1 };
            if math == 0 { math = width; } else if math == width { math = 0; }
            out.push_str(&rest[..width]); i += width; continue;
        }
        if math == 0 && !code && rest.starts_with(r"\text{") {
            let mut depth = 0;
            let mut end = None;
            for (offset, ch) in rest.char_indices().skip(5) {
                if ch == '{' { depth += 1; }
                if ch == '}' {
                    depth -= 1;
                    if depth == 0 { end = Some(offset + 1); break; }
                }
            }
            if let Some(end) = end {
                out.push('$'); out.push_str(&rest[..end]); out.push('$');
                i += end; continue;
            }
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch); i += ch.len_utf8();
    }
    out
}

// ── Boilerplate ─────────────────────────────────────────────────────────────

static BOILER_LINE_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)^[ \t]*(?:\*+\d{1,3}\*+|(?:turn[ \t]+over[ \t]+)?►|PMT|IB[/​$]?M[/​$]?[A-Za-z0-9$/]*|do[ \t]+not[ \t]+write|outside[ \t]+the|box[ \t]*[.:]?|question[ \t]+\d{1,2}[ \t]+continues[ \t]+on[ \t]+the[ \t]+next[ \t]+page[ \t]*\.?|extraction[ \t]+incomplete.*|\(extraction[ \t]+incomplete.*)[ \t]*$",
    )
    .unwrap()
});

static BARE_PAGE_NO_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^[ \t]*\d{1,2}[ \t]*$").unwrap());

static INLINE_BOX_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"[ \t]+box[ \t]*\.").unwrap());

static INLINE_IB_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"IB\$?/M/\$?[A-Za-z0-9$/]*").unwrap());

pub fn strip_ocr_boilerplate(content: &str) -> String {
    let symbol_margin = regex::Regex::new(r"\b([A-Z])[ \t]+box[ \t]*\r?\n[ \t]*([0-9])\b").unwrap();
    let content = symbol_margin.replace_all(content, |c: &regex::Captures| format!("${}_{}$", &c[1], &c[2]));
    let mut kept: Vec<&str> = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if BOILER_LINE_RE.is_match(t) {
            continue;
        }
        if BARE_PAGE_NO_RE.is_match(t) {
            continue; // bare page-number debris lines
        }
        kept.push(line);
    }
    let out = kept.join("\n");
    let out = INLINE_IB_RE.replace_all(&out, "").to_string();
    let out = INLINE_BOX_RE.replace_all(&out, " .").to_string();
    // margin debris "box" glued after a part label or at line start:
    //   "(i) box Three molecules..." -> "(i) Three molecules..."
    // "Tick the correct box" prose is NOT touched (box not at line-start/label).
    let box_label = regex::Regex::new(
        r"(?m)^((?:\([a-z]\)|\([ivx]+\))?[ \t]*)box[ \t]+([A-Z0-9$])",
    )
    .unwrap();
    let out = box_label.replace_all(&out, "${1}${2}").to_string();
    let out = regex::Regex::new(r"\bbox[ \t]+([A-Z$\\])").unwrap()
        .replace_all(&out, "${1}").to_string();
    let out = regex::Regex::new(r"(?m)([?.!])[ \t]+box[ \t]*$").unwrap()
        .replace_all(&out, "${1}").to_string();
    collapse_blank_lines(out.trim())
}

// ── T4: unicode minus, plain negative exponents, bare \text artifacts ──────

/// En dashes that read as minus signs (inside maths, or before a number,
/// a single-letter variable or a bracket after a space, operator or opening
/// bracket) become "-". Before a word it is a dash ("Table 3 – Standard").
fn en_dash_minus(t: &str) -> String {
    let chars: Vec<char> = t.chars().collect();
    let mut out = String::with_capacity(t.len());
    let mut math = false;
    for (i, &c) in chars.iter().enumerate() {
        if c == '$' {
            math = !math;
        }
        if c != '\u{2013}' {
            out.push(c);
            continue;
        }
        let prev = i.checked_sub(1).map(|k| chars[k]);
        let next = chars[i + 1..].iter().copied().find(|c| *c != ' ');
        let next_letters = chars[i + 1..].iter().skip_while(|c| **c == ' ').take_while(|c| c.is_alphabetic()).count();
        let joins = prev.is_some_and(|p| p.is_alphanumeric()) && chars.get(i + 1).is_some_and(|n| n.is_alphanumeric());
        let minus_like = !joins
            && prev.map_or(true, |p| p == ' ' || "=+-×(<>≤≥[{".contains(p))
            && next.is_some_and(|n| n.is_ascii_digit() || n == '(' || n.is_ascii_alphabetic() && next_letters == 1);
        out.push(if math || minus_like { '-' } else { c });
    }
    out
}

pub fn normalize_unicode_math(content: &str) -> String {
    let symbols = regex::Regex::new(r"\\text\{[ \t]*\\(mu|Omega)[ \t]*([A-Za-z]*)[ \t]*\}").unwrap();
    let content = symbols.replace_all(content, |c: &regex::Captures| {
        if c[2].is_empty() { format!("\\{}", &c[1]) }
        else { format!("\\{}\\text{{{}}}", &c[1], &c[2]) }
    });
    let t = content.replace('\u{2212}', "-"); // U+2212 MINUS SIGN
    // An en dash used as a minus (OCR, "x – 3") becomes one; a dash joining
    // words or a range ("east–west", "0–12") is punctuation and stays.
    let t = en_dash_minus(&t);
    // plain compound units with negative exponents: "J kg-1 K-1", "V div-1",
    // "m s-1", "kg m-3" -> $\text{...}^{-N}$. Only known unit words fire.
    const U: &str = "(?:kg|mg|ms|mm|cm|km|kJ|MJ|kN|kV|mV|nC|mC|mA|kA|MA|pF|mT|kW|MW|mW|kHz|MHz|kBq|MBq|GBq|kPa|MPa|keV|MeV|GeV|kg|m|g|s|h|J|N|V|C|A|K|F|T|W|Hz|mol|rad|div|throw|eV|u|Bq|Pa)";
    // "\text{kg} \text{m}-3" -> "\text{kg} \text{m}^{-3}"; a minus after
    // any other word ("\text{for} -1") is a sign, not an exponent.
    let unit_exp = regex::Regex::new(&format!(r"\\text\{{({U})\}}[ ]?-(\d)")).unwrap();
    let t = unit_exp.replace_all(&t, r"\text{${1}^{-${2}}}").to_string();
    if let Ok(re) = regex::Regex::new(&format!(
        r"(?m)(^|[^$\w{{-])((?:{U})(?:[ ](?:{U}))*)[ ]?-(\d)([^\w}}]|$)"
    )) {
        let t = re
            .replace_all(&t, |caps: &regex::Captures| {
                format!(
                    "{}$\\text{{{}}}^{{-{}}}${}",
                    &caps[1],
                    &caps[2],
                    &caps[3],
                    caps.get(4).map(|m| m.as_str()).unwrap_or("")
                )
            })
            .to_string();
        return normalize_unicode_math_rest(&t);
    }
    normalize_unicode_math_rest(&t)
}

fn normalize_unicode_math_rest(t: &str) -> String {
    // "x × 10-15" / "x \times 10-15" -> "x \times 10^{-15}"
    let sci = regex::Regex::new(r"(?:×|\\times)[ \t]*10-(\d+)").unwrap();
    let t = sci.replace_all(t, |caps: &regex::Captures| {
        format!("\\times 10^{{-{}}}", &caps[1])
    });
    // bare "10-15 m" -> "$10^{-15}$ m" (not already inside math; the trailing
    // char is consumed and re-emitted because the regex crate has no lookahead)
    let bare = regex::Regex::new(r"(?m)(^|[^$\w^{])10-(\d+)([^\w}]|$)").unwrap();
    let t = bare.replace_all(&t, |caps: &regex::Captures| {
        let start = caps.get(0).unwrap().start();
        let line_start = t[..start].rfind('\n').map_or(0, |i| i+1);
        if t[line_start..start].matches('$').count() % 2 == 1
            || t[..start].matches("$$").count() % 2 == 1 {
            format!("{}10^{{-{}}}{}", &caps[1], &caps[2], &caps[3])
        } else {
            format!("{}$10^{{-{}}}${}", &caps[1], &caps[2], &caps[3])
        }
    });
    // "\text{A} room..." -> "A room..." (OCR artifact at sentence start),
    // including after a part label: "(c) \text{A} slow-moving..." -> "(c) A slow-moving..."
    let txt_a = regex::Regex::new(r"(?m)^\\text\{A\}[ \t]+").unwrap();
    let t = txt_a.replace_all(&t, "A ").to_string();
    let txt_a_label = regex::Regex::new(r"(?m)^(\([a-i]\)[ \t]+)\\text\{A\}[ \t]+").unwrap();
    let t = txt_a_label.replace_all(&t, "${1}A ").to_string();
    // Nested-brace unit artifact from earlier repair passes:
    //   "\text{kg^{-1}}" -> "\text{kg}^{-1}"
    let nested = regex::Regex::new(r"\\text\{([A-Za-z]+)\^\{?(-\d+)\}?\}").unwrap();
    let t = nested.replace_all(&t, "\\text{${1}}^{${2}}").to_string();
    // Orphan \text{...} unit runs outside math: "200 \text{g}",
    // "4200 \text{J} \text{kg}^{-1}" -> "$200\ \text{g}$", "$4200\ \text{J}\ \text{kg}^{-1}$".
    // Only fires when the run is NOT already inside a $...$ span on its line.
    let u = r"\\text\s*\{[^}]*\}(?:\^\{-?\d+\})?";
    let unit_run = regex::Regex::new(&format!(
        r"((?:\d[\d.,]*)?[ \t]*(?:{u})(?:[ \t]+(?:{u}))*)"
    ))
    .unwrap();
    let mut out = String::with_capacity(t.len());
    for line in t.split('\n') {
        if line.contains("\\text") {
            let mut rebuilt = String::with_capacity(line.len());
            let mut rest = line;
            loop {
                match unit_run.find(rest) {
                    Some(m) => {
                        rebuilt.push_str(&rest[..m.start()]);
                        if rebuilt.matches('$').count() % 2 == 0 {
                            let s = m.as_str().trim();
                            let num_end = s
                                .find(|c: char| !c.is_ascii_digit() && c != '.' && c != ',')
                                .unwrap_or(s.len());
                            let num = &s[..num_end];
                            let units = s[num_end..].split_whitespace().collect::<Vec<_>>();
                            if units.is_empty() {
                                rebuilt.push_str(m.as_str());
                            } else if num.is_empty() {
                                rebuilt.push_str(&format!("${}$", units.join("\\ ")));
                            } else {
                                rebuilt.push_str(&format!(
                                    "${}\\ {}$",
                                    num,
                                    units.join("\\ ")
                                ));
                            }
                        } else {
                            rebuilt.push_str(m.as_str());
                        }
                        rest = &rest[m.end()..];
                    }
                    None => {
                        rebuilt.push_str(rest);
                        break;
                    }
                }
            }
            out.push_str(&rebuilt);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    let mut t = out;
    if t.ends_with('\n') {
        t.pop();
    }
    // Math-alphanumeric italic letters (U+1D400 block) used by some OCR
    // pipelines: "𝑞𝑞" -> "q". Doubled identical italic letters collapse to one
    // (the doubling is a bold/italic dual-rendering artifact, never semantics).
    let t = normalize_math_alphanumerics(&t);
    t
}

/// Map Mathematical Alphanumeric Symbols (the italic/bold runs observed in
/// OCR output) to plain ASCII and collapse doubled letters produced by
/// bold-italic double rendering ("𝑞𝑞" -> "q").
fn normalize_math_alphanumerics(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut prev_math: Option<char> = None;
    for ch in t.chars() {
        let cp = ch as u32;
        let mapped = match cp {
            // Mathematical Bold Capital A–Z (U+1D400) / Small a–z (U+1D41A)
            0x1D400..=0x1D419 => Some((b'A' + (cp - 0x1D400) as u8) as char),
            0x1D41A..=0x1D433 => Some((b'a' + (cp - 0x1D41A) as u8) as char),
            // Mathematical Italic Capital A–Z (U+1D434) / Small a–z (U+1D44E)
            0x1D434..=0x1D44D => Some((b'A' + (cp - 0x1D434) as u8) as char),
            0x1D44E..=0x1D467 => Some((b'a' + (cp - 0x1D44E) as u8) as char),
            _ => None,
        };
        match mapped {
            Some(c) => {
                if prev_math == Some(c) {
                    continue; // collapse doubled bold/italic rendering
                }
                out.push(c);
                prev_math = Some(c);
            }
            None => {
                out.push(ch);
                prev_math = None;
            }
        }
    }
    out
}

// ── T2: rogue / asymmetric math delimiters ─────────────────────────────────

pub fn balance_math_delimiters(content: &str) -> String {
    let mut out_lines: Vec<String> = Vec::new();
    for line in content.split('\n') {
        if line.contains("$$") {
            // `$$` is ONLY valid as a standalone display equation: ^$$...$$$
            let t = line.trim();
            let display = regex::Regex::new(r"^\$\$[^$]+\$\$$").unwrap();
            if display.is_match(t) {
                out_lines.push(line.to_string());
                continue;
            }
            // Demote every other `$$` usage to inline `$` pairs, then rebalance.
            let stripped = line.replace("$$", "$");
            out_lines.push(balance_inline(&stripped));
        } else if line.matches('$').count() % 2 == 1 {
            out_lines.push(balance_inline(line));
        } else {
            out_lines.push(line.to_string());
        }
    }
    out_lines.join("\n")
}

/// Make the number of `$` in a line even by dropping the last unpaired one.
/// Dropping (rather than appending a closer) is the safe direction: an
/// unclosed `$` before a marks tag is the observed corruption, and appending
/// would wrap the trailing prose in math instead.
fn balance_inline(line: &str) -> String {
    let n = line.matches('$').count();
    if n % 2 == 0 {
        return line.to_string();
    }
    match line.rfind('$') {
        Some(idx) => {
            let mut s = line.to_string();
            s.remove(idx);
            s.trim_end().to_string()
        }
        None => line.to_string(),
    }
}

// ── T3: sentence-level math wrapping ───────────────────────────────────────

static MATH_SPAN_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\$([^$\n]+)\$").unwrap());

static LONG_ENGLISH_WORD_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^[A-Za-z]{3,}$").unwrap());

pub fn unwrap_sentence_math(content: &str) -> String {
    MATH_SPAN_RE
        .replace_all(content, |caps: &regex::Captures| {
            let inner = &caps[1];
            let english_words = inner
                .split_whitespace()
                .filter(|w| !w.contains('\\') && LONG_ENGLISH_WORD_RE.is_match(w))
                .count();
            if english_words >= 4 {
                // A "math" span holding 6+ plain English words is a wrapping
                // error: emit the prose without delimiters.
                inner.to_string()
            } else {
                caps[0].to_string()
            }
        })
        .to_string()
}

// ── T1: scrambled isotope notation ─────────────────────────────────────────

/// Reconstructs the common OCR scramble families for nuclides:
///   "235U 92" / "92 235U"-adjacent forms -> $^{235}_{92}\text{U}$
///   "13C 6", "14N 7"                     -> $^{13}_{6}\text{C}$
///   "146La 57"                           -> $^{146}_{57}\text{La}$
/// Only fires OUTSIDE existing math spans and only for element-symbol tokens
/// followed by a trailing mass/atomic pair, which never occurs in valid prose.
/// Rebuild a column-flattened fusion/reaction chain. The text layer prints the
/// three mass numbers, then the three element symbols, then the three atomic
/// numbers:
///
///   `3 17 20 He + O Ne 2 8 10 \rightarrow`
///
/// This is the real source order (it is what pdfium emits for the printed
/// equation), so pairing the i-th mass / symbol / atomic number reconstructs
/// `^{3}_{2}\text{He} + ^{17}_{8}\text{O} \rightarrow ^{20}_{10}\text{Ne}`.
/// Only the exact three-term signature is matched; anything else is left for a
/// flagged recovery.
fn reconstruct_flattened_nuclide_chain(content: &str) -> String {
    static RE_CHAIN: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?m)^[ \t]*(\d{1,3})[ \t]+(\d{1,3})[ \t]+(\d{1,3})[ \t]+\$?([A-Z][a-z]?)\$?[ \t]+\$?\+[ \t]*\$?([A-Z][a-z]?)\$?[ \t]+\$?([A-Z][a-z]?)\$?[ \t]*\$?(\d{1,3})[ \t]+(\d{1,3})[ \t]+(\d{1,3})[ \t]*(?:\\rightarrow|->|\u{2192})?\$?",
        )
        .unwrap()
    });
    RE_CHAIN
        .replace_all(content, |c: &regex::Captures| {
            let masses = [&c[1], &c[2], &c[3]];
            let syms = [&c[4], &c[5], &c[6]];
            let atomics = [&c[7], &c[8], &c[9]];
            let parse = |s: &str| s.parse::<u32>().unwrap_or(0);
            let valid = (0..3).all(|i| {
                let a = parse(masses[i]);
                let z = parse(atomics[i]);
                z > 0 && z <= a
            });
            if !valid {
                return c[0].to_string();
            }
            format!(
                "$^{{{}}}_{{{}}}\\text{{{}}} + ^{{{}}}_{{{}}}\\text{{{}}} \\rightarrow ^{{{}}}_{{{}}}\\text{{{}}}$",
                masses[0], atomics[0], syms[0],
                masses[1], atomics[1], syms[1],
                masses[2], atomics[2], syms[2],
            )
        })
        .to_string()
}

pub fn reconstruct_isotope_notation(content: &str) -> String {
    // Only source-order reconstruction remains (values paired from the text
    // layer itself); memorised whole-equation substitutions were removed.
    let mut content = content.to_string();
    content = reconstruct_flattened_nuclide_chain(&content);
    // Split out math spans so we never touch valid LaTeX.
    let mut parts: Vec<String> = Vec::new();
    let mut last = 0;
    for m in MATH_SPAN_RE.find_iter(&content) {
        parts.push(apply_isotope_fixes(&content[last..m.start()]));
        parts.push(m.as_str().to_string());
        last = m.end();
    }
    parts.push(apply_isotope_fixes(&content[last..]));
    parts.join("")
}

fn apply_isotope_fixes(text: &str) -> String {
    // "235U 92" style: mass, symbol, space, atomic number (word boundary).
    let re = regex::Regex::new(r"\b(\d{1,3})[ \t]*([A-Z][a-z]?)\s+(\d{1,3})\b").unwrap();
    let t = re.replace_all(text, |c: &regex::Captures| {
        let a: u32 = c[1].parse().unwrap_or(0);
        let z: u32 = c[3].parse().unwrap_or(0);
        if z == 0 || z > a { return c[0].to_string(); }
        format!("$^{{{}}}_{{{}}}\\text{{{}}}$", &c[1], &c[3], &c[2])
    });
    t.to_string()
}

// ── T6: MCQ list syntax ────────────────────────────────────────────────────

static MARKS_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\*\*\[\d+[ \t]+marks?\]\*\*").unwrap());

static TAGGED_OPT_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^-[ \t]+\[MCQ:([A-E])\][ \t]*(.*)$").unwrap());

static PLAIN_OPT_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^-?[ \t]*\(?([A-E])[\).]?[ \t]+(\S.*)$").unwrap());

fn parse_plain_opt(line: &str) -> Option<(char, &str)> {
    let t = line.trim();
    let c = PLAIN_OPT_RE.captures(t)?;
    let letter = c[1].chars().next().unwrap();
    Some((letter, c.get(2).map(|m| m.as_str()).unwrap_or("")))
}

/// A stacked-fraction option printed as two lines collapses in the text layer
/// to "A 2" followed by "$9 d$" (numerator on the letter line, denominator in
/// a math span). Reconstruct: `$\frac{2}{9}d$`. "GMm" / "$2R$" ->
/// `$\frac{GMm}{2R}$`.
/// Split text that ENDS on a bare integer into (head, number). Anything glued
/// to the digits (a decimal point, a letter) is not a bare number, so ordinary
/// values like "0.50" or "x9" never qualify.
#[allow(dead_code)] // retained for the future glyph-evidence fraction path
fn split_trailing_number(body: &str) -> Option<(&str, &str)> {
    let trimmed = body.trim_end();
    let digits_start = trimmed
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_digit())
        .last()
        .map(|(i, _)| i)?;
    if digits_start == 0 {
        return None;
    }
    if trimmed[..digits_start]
        .chars()
        .last()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '.')
    {
        return None;
    }
    Some((trimmed[..digits_start].trim_end(), &trimmed[digits_start..]))
}

/// Two flattened-column artefacts produced by the text layer plus the math
/// wrapper:
///   * "Secondary voltage $/ $\text{V}$$ Secondary current /$$\text{A}$$" —
///     a "Label / unit" header split into stray dollar fragments;
///   * "$90\$%" — a percentage wrapped with a stray dollar.
/// Both are rebuilt as plain text: words, values and units survive, the broken
/// delimiters do not. Only the BROKEN (doubled-dollar) shapes are matched, so a
/// complete math span is never touched.
fn repair_flattened_layout_artifacts(content: &str) -> String {
    static RE_UNIT_BROKEN_A: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"\$\s*/\s*\$\s*\\text\{([^}]*)\}\s*\$\$").unwrap()
    });
    static RE_UNIT_BROKEN_B: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"/\s*\$\$\s*\\text\{([^}]*)\}\s*\$\$?").unwrap()
    });
    static RE_PERCENT_BROKEN: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\$(\d+(?:\.\d+)?)\\\$%").unwrap());
    let t = RE_UNIT_BROKEN_A.replace_all(content, |c: &regex::Captures| {
        format!(" / {}", &c[1])
    });
    let t = RE_UNIT_BROKEN_B.replace_all(&t, |c: &regex::Captures| {
        format!(" / {}", &c[1])
    });
    RE_PERCENT_BROKEN
        .replace_all(&t, |c: &regex::Captures| format!("{}%", &c[1]))
        .to_string()
}

fn merge_stacked_fraction(body: &str, math_line: &str) -> Option<String> {
    let tok = body.trim();
    if tok.is_empty() || tok.len() > 8 || !tok.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let t = math_line.trim();
    let inner = t.strip_prefix('$')?.strip_suffix('$')?.trim();
    if inner.is_empty() || inner.contains('$') {
        return None;
    }
    // Split the denominator span into leading factor + trailing alpha tail:
    //   "9 d" -> ("9", "d")   "2R" -> ("2R", "")   "mqd" -> ("mqd", "")
    let (den, tail) = match inner.find(' ') {
        Some(pos) => {
            let (a, b) = inner.split_at(pos);
            (a.trim(), b.trim())
        }
        None => (inner, ""),
    };
    if den.is_empty() || tail.contains(' ') || tail.contains('$') {
        return None;
    }
    Some(format!("$\\frac{{{}}}{{{}}}$ {}", tok, den, tail).trim_end().to_string())
}

static STANDALONE_MATH_LINE_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^\$([^$]+)\$[ \t]*(\*\*\[\d+[ \t]+marks?\]\*\*)?$").unwrap());

static PART_LABEL_LINE_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^\([a-h]\)").unwrap());

pub fn tighten_mcq_lists(content: &str) -> String {
    let content = content.replace('\r', "");
    let lines: Vec<&str> = content.lines().collect();
    let has_tagged = lines.iter().any(|l| TAGGED_OPT_RE.is_match(l.trim()));

    let mut out_lines: Vec<String>;
    if has_tagged {
        // Remove blank/whitespace-only lines BETWEEN consecutive MCQ items.
        let is_mcq = |l: &str| l.trim_start().starts_with("- [MCQ:");
        let is_blank = |l: &str| l.trim().is_empty();
        out_lines = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            if is_blank(line)
                && i > 0
                && i + 1 < lines.len()
                && is_mcq(lines[i - 1])
                && is_mcq(lines[i + 1])
            {
                continue;
            }
            out_lines.push((*line).to_string());
        }
        // Span-merge duplication debris: a page-boundary merge can re-append
        // the tail options as PLAIN lines after the complete tagged run
        // ("...- [MCQ:D] 4.6 kW **[1 mark]**\nC 1.1 kW\n\nD 4.6 kW ...").
        // Drop trailing plain option lines that duplicate a tagged option.
        if let Some(_last_tagged) = out_lines
            .iter()
            .rposition(|l| l.trim_start().starts_with("- [MCQ:"))
        {
            let tagged_bodies: Vec<String> = out_lines
                .iter()
                .filter_map(|l| {
                    TAGGED_OPT_RE.captures(l.trim()).map(|c| {
                        normalize_ws(c.get(2).map(|m| m.as_str()).unwrap_or(""))
                    })
                })
                .collect();
            let scan_from = last_tagged_position(&out_lines) + 1;
            let mut cut = None;
            for (i, l) in out_lines.iter().enumerate().skip(scan_from) {
                let t = l.trim();
                if t.is_empty() {
                    continue;
                }
                match parse_plain_opt(t) {
                    Some((letter, body)) => {
                        let norm_body = normalize_ws(body);
                        let norm_body_no_mark = normalize_ws(
                            body.trim_end_matches("**[1 mark]**"),
                        );
                        let dup = tagged_bodies.iter().any(|b| {
                            b == &norm_body
                                || b == &norm_body_no_mark
                                || b.trim_end_matches("**[1 mark]**").trim() == norm_body
                        });
                        if dup || letter != 'A' {
                            if cut.is_none() {
                                cut = Some(i); // first debris line — truncate here
                            }
                            continue;
                        }
                    }
                    None => {}
                }
                break;
            }
            if let Some(cut) = cut {
                out_lines.truncate(cut);
            }
        }
    } else if lines.iter().filter(|l| PART_LABEL_LINE_RE.is_match(l.trim_start())).count() >= 2 {
        // A structured question with lettered parts keeps an embedded choice
        // list ("Shade one lozenge …" in one part) as plain lines: it is not
        // a multiple-choice card.
        return content.to_string();
    } else {
        // Convert a consecutive plain "A ... B ... C ... D ..." run into the
        // strict tagged list. The run must be IN ORDER (A then B then C then
        // D) so prose like "A room contains..." is never mistaken for an
        // option. Anything before the run is stem; anything after is kept.
        let mut chain_start = None;
        let mut chain_end = 0;
        let mut chain: Vec<(char, String)> = Vec::new();
        'outer: for i in 0..lines.len() {
            let mut j = i;
            let mut expected = b'A';
            let mut collected: Vec<(char, String)> = Vec::new();
            while j < lines.len() {
                if lines[j].trim().is_empty() && !collected.is_empty() {
                    j += 1; // tolerate blank lines inside the option run
                    continue;
                }
                let parsed = parse_plain_opt(lines[j]);
                match parsed {
                    Some((letter, body)) if letter as u8 == expected => {
                        // Stacked-fraction merge: "A 2" + "$9 d$" is ONE option.
                        if let Some(k) = (j + 1..lines.len())
                            .find(|k| !lines[*k].trim().is_empty())
                        {
                            if !body.contains('$')
                                && STANDALONE_MATH_LINE_RE.is_match(lines[k].trim())
                            {
                                if let Some(merged) =
                                    merge_stacked_fraction(body, lines[k])
                                {
                                    collected.push((letter, merged));
                                    expected += 1;
                                    j = k + 1;
                                    continue;
                                }
                            }
                            // NOTE: a "number at the end of one line / short
                            // bare symbol on the next" rule used to fabricate
                            // `\frac{9}{F}` here. That shape does not establish
                            // division or its orientation (it may be `9F`, a
                            // value+unit, or a flattened fraction). It is no
                            // longer guessed; real stacked fractions require
                            // fraction-bar glyph evidence from the layout
                            // evidence path.
                        }
                        collected.push((letter, body.to_string()));
                        expected += 1;
                        j += 1;
                    }
                    _ => break,
                }
            }
            if collected.len() == 4 {
                chain_start = Some(i);
                chain_end = j;
                chain = collected;
                break 'outer;
            }
        }
        if let (Some(start), chain_nonempty) = (chain_start, !chain.is_empty()) {
            let _ = chain_nonempty;
            let mut rebuilt: Vec<String> = Vec::new();
            for line in &lines[..start] {
                rebuilt.push(stem_label_re().replace_all(line, "").to_string());
            }
            for (letter, body) in &chain {
                rebuilt.push(format!("- [MCQ:{}] {}", letter, body));
            }
            for line in &lines[chain_end..] {
                rebuilt.push((*line).to_string());
            }
            out_lines = rebuilt;
        } else if let Some(run) = letter_only_option_run(&lines) {
            // Options printed as bare letters (answer bubbles naming points
            // or graphs drawn above) are options with no text of their own.
            out_lines = lines
                .iter()
                .enumerate()
                .map(|(i, l)| match run.iter().position(|&k| k == i) {
                    // The option's text is its letter (it names a point or a
                    // drawing): an empty option would read as no option.
                    Some(n) => format!("- [MCQ:{0}] {0}", (b'A' + n as u8) as char),
                    None => (*l).to_string(),
                })
                .collect();
        } else {
            out_lines = lines.iter().map(|l| (*l).to_string()).collect();
        }
    }

    let mut out = out_lines.join("\n");
    let is_mcq_card = out.contains("- [MCQ:");
    // Strip stray part labels from the MCQ stem: "(i) ", "(a) ", "(iv) ".
    // MCQ cards only — structured questions legitimately use (a)/(b) labels.
    if is_mcq_card {
        let mut lines: Vec<String> = out.split('\n').map(|s| s.to_string()).collect();
        for line in lines.iter_mut() {
            if line.starts_with("- [MCQ:") {
                break;
            }
            if stem_label_re().is_match(line) {
                *line = stem_label_re().replace_all(line, "").to_string();
            }
        }
        out = lines.join("\n");
    }
    // Ensure the card carries a marks tag (AQA MCQs are always 1 mark).
    // ONLY for cards that are actually MCQs — never decorate structured
    // questions whose per-part marks the model already supplied.
    if is_mcq_card && !MARKS_RE.is_match(&out) {
        out = format!("{} **[1 mark]**", out.trim_end());
    }
    // Ensure the marks tag is not INSIDE the last option's math span.
    let fix = regex::Regex::new(r"(\$[^$\n]*)\*\*\[1 mark\]\*\*").unwrap();
    let out = fix.replace_all(&out, "${1}**[1 mark]**").to_string();
    out
}

/// The last run of lines reading exactly "A", "B", "C", "D" (blank lines
/// between allowed) with only blank lines after it: the line indices.
fn letter_only_option_run(lines: &[&str]) -> Option<Vec<usize>> {
    let filled: Vec<usize> = (0..lines.len()).filter(|&i| !lines[i].trim().is_empty()).collect();
    if filled.len() < 5 {
        return None;
    }
    let tail = &filled[filled.len() - 4..];
    let in_order = tail.iter().enumerate().all(|(n, &i)| lines[i].trim() == ((b'A' + n as u8) as char).to_string());
    // A stem comes first (not another bare letter).
    let before = lines[filled[filled.len() - 5]].trim();
    (in_order && before.chars().count() > 1).then(|| tail.to_vec())
}

fn last_tagged_position(lines: &[String]) -> usize {
    lines
        .iter()
        .rposition(|l| l.trim_start().starts_with("- [MCQ:"))
        .unwrap_or(0)
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn stem_label_re() -> &'static regex::Regex {
    static RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"^(?:(?:\([a-i]\)|\([iivx]+\))[ \t]+)+").unwrap()
    });
    &RE
}

// ── Orchestrator ───────────────────────────────────────────────────────────

pub fn sanitize_question_content(content: &str, question_number: u32) -> String {
    let t = content.replace("\r\n", "\n");
    let t = strip_question_heading(&t, question_number);
    let t = strip_ocr_boilerplate(&t);
    let t = normalize_unicode_math(&t);
    let t = unwrap_sentence_math(&t);
    let mut t = balance_math_delimiters(&t);
    t = wrap_orphan_latex(&t);
    // tighten_mcq_lists is self-gating: it only fires on cards that carry
    // (or plainly contain) an A–D option run.
    let t = bind_standalone_mcq_images(&t);
    let t = tighten_mcq_lists(&t);
    let t = crate::marker_client::ensure_diagrams_above_mcq(&t);
    let t = repair_flattened_layout_artifacts(&t);
    let t = unwrap_math_wrapped_table_rows(&t);
    collapse_blank_lines(t.trim()).to_string()
}

/// A Markdown table delimiter row accidentally wrapped in `$ … $`.
fn unwrap_math_wrapped_table_rows(content: &str) -> String {
    let table_delim = regex::Regex::new(r"(?m)^\$([ \t]*\|[\s\-:|]+\|[ \t]*)\$$").unwrap();
    table_delim.replace_all(content, "$1").to_string()
}

/// Four trailing figures following a choice stem are visual answer options.
fn bind_standalone_mcq_images(content: &str) -> String {
    if content.contains("[MCQ:") { return content.to_string(); }
    let image = regex::Regex::new(r"^!\[[^\]]*\]\(([^)]+)\)$").unwrap();
    let lines: Vec<_> = content.lines().collect();
    let mut start = lines.len();
    let mut urls = Vec::new();
    while start > 0 {
        let line = lines[start-1].trim();
        if line.is_empty() { start -= 1; continue; }
        let Some(c) = image.captures(line) else { break; };
        urls.push(c[1].to_string());
        start -= 1;
    }
    let stem = lines[..start].join("\n");
    if urls.len() != 4 || !regex::Regex::new(r"(?i)\bwhich\b").unwrap().is_match(&stem) {
        return content.to_string();
    }
    urls.reverse();
    format!("{}\n\n{}", stem.trim_end(), urls.iter().enumerate().map(|(i, url)| {
        let letter = (b'A' + i as u8) as char;
        format!("- [MCQ:{letter}] ![Option {letter}]({url})")
    }).collect::<Vec<_>>().join("\n"))
}

fn collapse_blank_lines(t: &str) -> String {
    let re = regex::Regex::new(r"\n{3,}").unwrap();
    re.replace_all(t, "\n\n").to_string()
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #[test]
    fn bare_negative_power_does_not_nest_math_delimiters() {
        assert_eq!(normalize_unicode_math(r"$r \approx 10-15 \text{m}$"), r"$r \approx 10^{-15} \text{m}$");
        assert_eq!(normalize_unicode_math(r"$r < 10^{-14} \text{m}$"), r"$r < 10^{-14} \text{m}$");
        assert_eq!(normalize_unicode_math(r"$$r = 10-15$$"), r"$$r = 10^{-15}$$");
    }
    #[test]
    fn flattened_text_layer_maths_is_never_rewritten_into_remembered_expressions() {
        // The text layer of a MathType paper prints stacked maths as runs of
        // bare numbers and letters. Without glyph geometry there is nothing
        // to reconstruct from: the sanitizer must not substitute expressions
        // it "knows" belong to the paper (the layout engine rebuilds them from
        // the source glyphs, rules and paths instead).
        let page29 = include_str!("../fixtures/physics24_text/page_29.txt");
        let out = sanitize_question_content(page29, 21);
        assert!(!out.contains(r"\sqrt[3]{16}") && !out.contains(r"\frac{R}"), "{out}");
        let out = sanitize_question_content(r"14 Cp $N 61 7 + \rightarrow$", 17);
        assert!(!out.contains("^{13}"), "13 is not in the source text: {out}");
        let out = sanitize_question_content("238Un $X 92 + \\rightarrow\nXY $v + + e \\rightarrow \\beta$-\nHow many neutrons?", 31);
        assert!(!out.contains(r"\nu"), "no remembered antineutrino: {out}");
        assert!(!sanitize_question_content("A 10 2\n m\nB 10 3\n m", 22).contains("10^{2}"), "no guessed exponent");
    }

    #[test]
    fn audit_margin_subparts_survive_cleanup() {
        for (n, raw) in [(4, "0 4 box . 1 One purpose **[1 mark]**\n0 4 . 2 State two **[2 marks]**"), (6, "0 6 box . 1 The electric potential **[3 marks]**\n06.2 Determine **[4 marks]**\n06.3 Calculate **[2 marks]**\n06.4 Discuss **[3 marks]**")] {
            let clean = strip_ocr_boilerplate(raw);
            assert!(clean.contains(". 1"), "{clean}");
            assert_eq!(strip_question_heading(&clean, n), clean);
            let out = crate::marker_client::clean_marker_markdown(&clean);
            assert!(out.contains("(a)"), "{out}");
            assert!(out.contains("(b)"), "{out}");
            assert_eq!(out.matches("marks]").count() + out.matches("mark]").count(), if n == 4 { 2 } else { 4 });
        }
    }

    #[test]
    fn options_printed_as_bare_letters_are_tagged_with_their_letter() {
        // The options name points on the diagram: each option is its letter.
        let raw = "The diagram shows gravitational equipotentials.\nWhich point has the greatest gravitational field strength?\n\n**[1 mark]**\nA\nB\nC\nD";
        let out = tighten_mcq_lists(raw);
        assert!(out.contains("- [MCQ:A] A\n- [MCQ:B] B\n- [MCQ:C] C\n- [MCQ:D] D"), "{out}");
        // A lone letter after prose is not an option run.
        let prose = "Point A is shown.\nB";
        assert_eq!(tighten_mcq_lists(prose), prose);
    }

    #[test]
    fn heading_number_alone_on_its_line_is_stripped() {
        // Edexcel prints "2." above a figure that opens the question.
        assert_eq!(strip_question_heading("2.\nFigure 1\nThe network shown", 2), "Figure 1\nThe network shown");
        assert_eq!(strip_question_heading("10.\r\nFigure 3", 10), "Figure 3");
        // A different number, or a decimal, is content.
        assert_eq!(strip_question_heading("3.\nFigure 1", 2), "3.\nFigure 1");
        assert_eq!(strip_question_heading("2.5 m of rope", 2), "2.5 m of rope");
    }

    #[test]
    fn audit_unit_repairs() {
        assert_eq!(normalize_unicode_math(r"$31.0 \text{ \mu F}$ and $2.4 \times 10^5 \text{ \Omega}$"), r"$31.0 \mu\text{F}$ and $2.4 \times 10^5 \Omega$");
    }
    #[test]
    fn physics24_orphan_commands_and_partial_reaction() {
        let raw = r"A value \text{kg} and $\text{N}$ with $$\text{C}$$ and `\text{code}`.";
        let out = wrap_orphan_latex(raw);
        assert_eq!(out, r"A value $\text{kg}$ and $\text{N}$ with $$\text{C}$$ and `\text{code}`.");
        assert_eq!(wrap_orphan_latex(&out), out);
        assert_eq!(wrap_orphan_latex(r"\text{a {nested} label}"), r"$\text{a {nested} label}$");
        let repaired = sanitize_question_content(r"The particle is \text{n}$ **[1 mark]**", 31);
        assert!(crate::validate::card_structure_errors(&repaired, 31).is_empty(), "{repaired}");
    }
    #[test]
    fn contextual_furniture_removed_without_touching_legitimate_content() {
        // Sanitize must NOT delete valued equations or code/table content: the
        // answer-line labels are removed only by source answer-rule adjacency
        // in the evidence engine (pdf_render::answer_prompt_edits).
        let keep1 = sanitize_question_content("The total cost = 10 units.", 5);
        assert!(keep1.contains("cost = 10 units"), "{keep1}");
        let keep2 = sanitize_question_content("```\nconst cost = 10; // left right\n```", 5);
        assert!(keep2.contains("const cost = 10"), "{keep2}");
        assert!(keep2.contains("left right"), "{keep2}");
        let keep3 = sanitize_question_content("| left right |\n| --- |\n| 1 |", 5);
        assert!(keep3.contains("left right"), "{keep3}");
        let keep4 = sanitize_question_content("cost =\nnext line", 5);
        assert!(keep4.contains("cost ="), "{keep4}");
    }

    #[test]
    fn flattened_fusion_chain_is_reconstructed_from_source_order() {
        let raw = "3 17 20 He $+ O$ Ne $2 8 10 \\rightarrow$";
        let out = reconstruct_isotope_notation(raw);
        assert_eq!(
            out,
            "$^{3}_{2}\\text{He} + ^{17}_{8}\\text{O} \\rightarrow ^{20}_{10}\\text{Ne}$",
            "{out}"
        );
        // Atomic number greater than mass number is not a real nuclide: leave
        // the source untouched so the card becomes a flagged recovery.
        let bad = "3 17 20 He $+ O$ Ne $2 8 99 \\rightarrow$";
        assert!(reconstruct_isotope_notation(bad).contains("He"));
        assert!(!reconstruct_isotope_notation(bad).contains("\\text{Ne}"));
    }

    #[test]
    fn physics24_repairs() {
        let q21 = sanitize_question_content("2 1 A synchronous orbit of the Earth.\nA 3\nR\nB 4\nR\nC 5\nR\nD 6\nR", 21);
        assert!(q21.starts_with("A synchronous"), "{q21}");
        assert_eq!(sanitize_question_content(r"2 $1\ \text{A}$ synchronous orbit.", 21), "A synchronous orbit.");
        // The stacked R/number option cannot be reconstructed from text order
        // alone (no fraction-bar evidence here), so nothing is fabricated.
        assert!(!q21.contains("\\frac"), "must not guess a stacked fraction: {q21}");
        assert_eq!(strip_ocr_boilerplate("potential at box P. Tick the correct box"), "potential at P. Tick the correct box");
        assert_eq!(strip_ocr_boilerplate("Which change increases efficiency? box"), "Which change increases efficiency?");
    }
    use super::*;

    #[test]
    fn strips_margin_boilerplate() {
        let raw = "0 1 box . 4 A room contains moist air.\n*03*\n ►\nIB$/M/$Jun$24/7408/2$ **[3 marks]**\nPMT";
        let out = strip_ocr_boilerplate(raw);
        assert!(out.contains("A room contains moist air."));
        assert!(!out.contains("IB/"));
        assert!(!out.contains("*03*"));
        assert!(!out.contains("PMT"));
    }

    #[test]
    fn normalizes_unicode_minus_and_exponents() {
        let raw = "density is 1.25 kg m\u{2212}3 and speed 3\u{d7}10\u{2212}8 m s\u{2212}1";
        let out = normalize_unicode_math(raw);
        assert!(out.contains("^{-3}"), "{out}");
        assert!(out.contains("\\times 10^{-8}"), "{out}");
        assert!(out.contains("^{-1}"), "{out}");
        assert!(!out.contains('\u{2212}'));
    }

    #[test]
    fn a_dash_before_a_prose_word_stays_a_dash() {
        // "Table 3 – Standard AQA …", "… recorded – the system": punctuation.
        assert_eq!(en_dash_minus("Table 3 – Standard AQA"), "Table 3 – Standard AQA");
        assert_eq!(en_dash_minus("• `#` – use the decimal value"), "• `#` – use the decimal value");
        assert_eq!(en_dash_minus("are recorded – the system"), "are recorded – the system");
        // Before a number, a single-letter variable or a bracket it is a minus.
        assert_eq!(en_dash_minus("x – 3"), "x - 3");
        assert_eq!(en_dash_minus("2 – y"), "2 - y");
        assert_eq!(en_dash_minus("a – (b + c)"), "a - (b + c)");
    }

    #[test]
    fn a_minus_after_a_word_that_is_not_a_unit_stays_a_minus() {
        // "for −1 ⩽ x ⩽ 1": the word is prose, the −1 a signed bound.
        let raw = r"$\begin{cases}3x - 2 \text{for} -1 \leqslant x \leqslant 1\end{cases}$";
        assert_eq!(normalize_unicode_math(raw), raw);
        // A detached exponent after a unit word is still that unit's.
        assert!(normalize_unicode_math(r"$1.25 \text{kg} \text{m}-3$").contains(r"\text{m}^{-3}"));
    }

    #[test]
    fn balances_rogue_dollars_before_marks() {
        let raw = "- [MCQ:B] r < 10^{-15} m $$";
        let out = balance_math_delimiters(raw);
        assert!(!out.contains("$$"), "{out}");
        assert!(out.matches('$').count() % 2 == 0, "{out}");
    }

    #[test]
    fn keeps_standalone_display_math() {
        let raw = "$$T^2 \\propto r^3$$";
        let out = balance_math_delimiters(raw);
        assert_eq!(out, raw);
    }

    #[test]
    fn unwraps_sentence_level_math() {
        let raw = "$the charge stored on a capacitor of area $1\\ \\text{m}^2$ when p.d. is $1\\ \\text{V}$$";
        // Outer span holds >= 6 english words -> unwrapped; inner math kept.
        let out = unwrap_sentence_math(raw);
        assert!(!out.starts_with("$the"), "{out}");
    }

    #[test]
    fn reconstructs_scrambled_isotopes() {
        let raw = "The mass of 235U 92 is greater than that of 146La 57.";
        let out = reconstruct_isotope_notation(raw);
        assert!(out.contains("$^{235}_{92}\\text{U}$"), "{out}");
        assert!(out.contains("$^{146}_{57}\\text{La}$"), "{out}");
    }

    #[test]
    fn leaves_valid_math_spans_untouched() {
        let raw = "$^{235}_{92}\\text{U}$ already fine";
        let out = reconstruct_isotope_notation(raw);
        assert_eq!(out, raw);
    }

    #[test]
    fn tightens_mcq_lists_and_appends_mark() {
        let raw = "Stem?\n\n- [MCQ:A] 3.50v\n\n- [MCQ:B] 3.67v\n\n- [MCQ:C] 3.87v\n\n- [MCQ:D] 26.0v";
        let out = tighten_mcq_lists(raw);
        assert!(!out.contains("\n\n- [MCQ:B]"), "{out}");
        assert!(out.ends_with("**[1 mark]**"));
    }

    #[test]
    fn mcq_mark_never_lands_inside_math() {
        let raw = "- [MCQ:D] $7.24\\times10^{6}\\ \\text{V m}^{-1}$ downwards";
        let out = tighten_mcq_lists(raw);
        let last = out.lines().last().unwrap();
        let math_count = last.matches('$').count();
        assert_eq!(math_count % 2, 0, "{last}");
    }

    #[test]
    fn orchestrator_end_to_end() {
        let raw = "(i) box Three molecules have speeds.\n\nA 3.50v\n\nB 3.67v\n\nC 3.87v\n\nD 26.0v";
        let out = sanitize_question_content(raw, 9);
        assert!(out.contains("- [MCQ:A] 3.50v"), "{out}");
        assert!(out.ends_with("**[1 mark]**"), "{out}");
        assert!(!out.contains("box"), "{out}");
    }

    #[test]
    fn regression_q9_physics17_reimport() {
        // Exact content observed in the post-re-import database: \text{A}
        // artifact, nested-brace units, duplicated option tail from a
        // page-boundary span merge.
        let raw = "\\text{A} student measures the power of a microwave oven. He places 200 \\text{g} of water at $23 ^{\\circ}C$ into the microwave and heats it on full power for 1 minute. When he removes it, the temperature of the water is $79 ^{\\circ}C$.\nThe specific heat capacity of water is 4200 \\text{J} \\text{kg^{-1}} \\text{K^{-1}}.\nWhat is the average rate at which thermal energy is gained by the water?\n\n- [MCQ:A] 780 W\n- [MCQ:B] 840 W\n- [MCQ:C] 1.1 kW\n- [MCQ:D] 4.6 kW **[1 mark]**\nC 1.1 kW\n\nD 4.6 kW **[1 mark]**";
        let out = sanitize_question_content(raw, 9);
        assert!(out.starts_with("A student measures"), "\\text{{A}} not unwrapped:\n{}", out);
        assert!(
            out.contains("$4200\\ \\text{J}\\ \\text{kg}^{-1}\\ \\text{K}^{-1}$"),
            "unit run not wrapped in math:\n{}",
            out
        );
        assert!(
            !out.contains("\nC 1.1 kW"),
            "duplicated plain option tail not removed:\n{}",
            out
        );
        assert_eq!(out.matches("[MCQ:").count(), 4, "{}", out);
    }

    #[test]
    fn stacked_fraction_options_reconstructed() {
        let raw = "Charon is a moon of Pluto.\nWhat is the distance of X from the centre of Pluto?\n\nA 2\n$9 d$\nB 2\n$3 d$\nC 3\n$4 d$\nD 8\n$9 d$ **[1 mark]**";
        let out = sanitize_question_content(raw, 12);
        assert!(out.contains("- [MCQ:A] $\\frac{2}{9}$ d") || out.contains("- [MCQ:A] $\\frac{2}{9}$d"), "{out}");
        assert!(out.contains("- [MCQ:B] $\\frac{2}{3}$ d") || out.contains("- [MCQ:B] $\\frac{2}{3}$d"), "{out}");
        assert_eq!(out.matches("- [MCQ:").count(), 4, "{out}");
    }

    #[test]
    fn section_a_cards_skip_mcq_tightening() {
        let raw = "(a) Define the tesla. **[1 mark]**";
        let out = sanitize_question_content(raw, 3);
        assert_eq!(out, raw);
    }

    #[test]
    fn flattened_unit_header_and_percent_are_repaired() {
        // Exact broken shapes seen on physics '21 Q25.
        let raw = "The transformer is $90\\$% efficient.\n\
                   Secondary voltage $/ $\\text{V}$$ Secondary current /$$\\text{A}$$";
        let out = repair_flattened_layout_artifacts(raw);
        assert!(out.contains("90%"), "{out}");
        assert!(!out.contains("\\text"), "{out}");
        assert!(out.contains("/ V"), "{out}");
        assert!(out.contains("/ A"), "{out}");
        assert_eq!(repair_flattened_layout_artifacts(&out), out, "idempotent");

        // Complete math and complete " / " headers are untouched.
        let clean = "The charge is $Q = 2\\,\\text{C}$ and the unit is $\\text{V}$.";
        assert_eq!(repair_flattened_layout_artifacts(clean), clean);

        // The repaired line passes the structural gate that flagged it before.
        let sanitized = sanitize_question_content(raw, 25);
        assert!(
            crate::validate::card_structure_errors(&sanitized, 25).is_empty(),
            "{sanitized}"
        );
    }

    #[test]
    fn stacked_fraction_is_not_guessed_from_two_lines() {
        // Physics '21 Q14: the text layer splits a two-line expression
        // ("9" then "F"). We must NOT invent `\frac{9}{F}` from that shape —
        // it could be a product, a value+unit, or a flattened fraction missing
        // entries. The raw tokens survive and no fraction is fabricated.
        let raw = "1 What is the force that now acts between the particles?\n[1 mark]\n\
                   A an attractive force of 9\nF\n\
                   B an attractive force of 9\nF\n\
                   C a repulsive force of 3\nF\n\
                   D a repulsive force of 3\nF";
        let out = sanitize_question_content(raw, 1);
        assert!(!out.contains("\\frac{9}{F}"), "must not guess a fraction: {out}");
        assert!(!out.contains("\\frac{3}{F}"), "must not guess a fraction: {out}");
        assert!(out.contains("attractive force of 9"), "source tokens preserved: {out}");
        assert!(out.contains("repulsive force of 3"), "source tokens preserved: {out}");

        // Negative cases: wrapped value+unit pairs and prose never become
        // fractions, and a bare pair outside an option run is left alone.
        for plain in ["The value is 9\nN", "The force is 3\nF", "The value is 9\nF"] {
            let s = sanitize_question_content(plain, 40);
            assert!(!s.contains("\\frac"), "no fabricated fraction: {s}");
            assert_eq!(s, plain, "plain text unchanged: {s}");
        }
    }

    #[test]
    fn scratch_debug_merged_items() {
        // Regression guard: the sanitizer must never decorate non-MCQ cards.
        let content = "Part one: factorise $x^2 - 5x + 6$.\n\nPart two: hence solve $x^2 - 5x + 6 = 0$.";
        let s = sanitize_question_content(content, 30);
        assert!(!s.contains("**["), "sanitizer injected a mark tag: {}", s);
        // Section A keeps its part labels...
        let s2 = sanitize_question_content("(a) Define the tesla. **[1 mark]**", 3);
        assert_eq!(s2, "(a) Define the tesla. **[1 mark]**");
        // A high question number alone is not evidence of an MCQ section.
        let s3 = sanitize_question_content("(a) Two parallel metal plates carry charge.", 15);
        assert!(s3.starts_with("(a) Two parallel"), "{s3}");
    }
}
