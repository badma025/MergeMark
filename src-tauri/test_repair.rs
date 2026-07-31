fn normalize_trace_table(text: &str) -> String {
    text.to_string()
}

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

    let hallucinated_aligned = regex::Regex::new(r"(?s)\\begin\{aligned\}(.*?)\\end\{aligned\}").unwrap();
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
                in_array = false;
            }
            continue;
        }
        if in_array {
            let closes_array = line.contains(r"\end{array}") || line.contains(r"\end{matrix}");
            array_lines.push(line);
            if closes_array {
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

    out.join("\n")
}

pub fn sanitize_extracted_latex(text: &str) -> String {
    let current = repair_latex_syntax(text);
    current.trim().to_string()
}

pub fn sanitize_markdown_math(text: &str) -> String {
    sanitize_extracted_latex(text)
}

fn main() {
    let source = r#"\left(\frac{\gamma RT}{M}\right)^{1/2}
\begin{aligned} where \\ \\ $\gamma$ is a dimensionless constant that depends on the gas \\ \\ $R$ is the molar gas constant \\ \\ $T$ is the absolute temperature \\ \\ $M$ is the molar mass of the gas. \end{aligned}"#;
    let repaired = sanitize_markdown_math(source);
    println!("REPAIRED:\n{}\n---", repaired);
    println!("starts_with $$: {}", repaired.starts_with("$$"));
}
