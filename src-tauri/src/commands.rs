use crate::AppState;
use image::GenericImageView;
use serde::{Deserialize, Serialize};
use tauri::{Manager, State};
use std::thread;
use std::time::Duration;

// ── Shared data model ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub id: String,
    pub subject: String,
    pub subtopic: String,
    pub marks: i32,
    pub content: String,
    pub math_snippet: String,
    pub is_code: bool,
    pub answer_content: Option<String>,
    pub topics: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposedMapping {
    pub question_id: String,
    pub raw_content: String,
    pub proposed_answer: String,
}

fn auto_close_json(s: &str) -> String {
    let mut in_string = false;
    let mut escaped = false;
    let mut stack = Vec::new();

    for c in s.chars() {
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
                    stack.pop();
                }
                _ => {}
            }
        }
    }

    let mut closed = s.to_string();
    if in_string {
        closed.push('"');
    }
    while let Some(c) = stack.pop() {
        closed.push(c);
    }
    closed
}

// ── Helper: shared question-classification + DB-insert logic ──────────────────

/// Keyword tables used for TF-IDF-style subject scoring.
struct SubjectClassifier {
    marks_re: regex::Regex,
    q_split_re: regex::Regex,
    math_re: regex::Regex,
}

impl SubjectClassifier {
    fn new() -> Self {
        Self {
            // Captures "[4 marks]", "[4 mark]", "(4)" style mark annotations
            marks_re: regex::Regex::new(
                r"(?i)\[\s*(\d+)\s*marks?\s*\]|\(\s*(\d+)\s*\)",
            )
            .unwrap(),
            // Split on a new numbered question: blank-line + digit + "." or ")"
            // Also handles "Q1", "Q.1", "Question 1"
            q_split_re: regex::Regex::new(
                r"(?m)(?:^|\n)(?:Question\s+\d+|Q\.?\s*\d+|\d{1,2}[.)]\s)",
            )
            .unwrap(),
            // LaTeX inline ($...$) or display ($$...$$, \[...\]) math
            math_re: regex::Regex::new(
                r"(?s)\$\$?.+?\$\$?|\\\[.+?\\\]|\\\(.+?\\\)",
            )
            .unwrap(),
        }
    }

    /// Score a block of text against known subject keyword sets.
    /// Returns (subject_name, subtopic, is_code).
    fn classify(&self, text: &str) -> (&'static str, &'static str, bool) {
        let lower = text.to_lowercase();

        // ── Computer Science ──────────────────────────────────────────────
        let cs_keywords: &[&str] = &[
            "array", "pointer", "recursion", "binary tree", "linked list",
            "stack", "queue", "hash table", "algorithm", "big-o", "o(n)",
            "complexity", "sql", "database", "sorting", "searching",
            "compiler", "interpreter", "cpu", "register", "cache",
            "encryption", "network", "protocol", "tcp", "ip address",
            "subroutine", "function call", "object-oriented", "class",
            "inheritance", "polymorphism", "binary", "hexadecimal",
            "boolean", "pseudocode", "flowchart", "assembly",
        ];

        // ── Further / Pure Mathematics ────────────────────────────────────
        let math_keywords: &[&str] = &[
            "matrix", "determinant", "eigenvalue", "eigenvector",
            "differential equation", "integration", "differentiation",
            "calculus", "gradient", "vector", "scalar", "proof",
            "induction", "complex number", "argand", "polynomial",
            "binomial", "series", "sequence", "limit", "convergence",
            "trigonometry", "sine", "cosine", "tangent", "logarithm",
            "exponent", "modulus", "inequality", "quadratic",
        ];

        // ── Physics ───────────────────────────────────────────────────────
        let phys_keywords: &[&str] = &[
            "kinetic energy", "potential energy", "momentum", "velocity",
            "acceleration", "force", "newton", "wavelength", "frequency",
            "magnetic field", "electric field", "voltage", "current",
            "resistance", "ohm", "capacitor", "inductor", "photon",
            "quantum", "nuclear", "radioactive", "half-life", "thermal",
            "entropy", "pressure", "density", "refraction", "diffraction",
        ];

        // ── Chemistry ────────────────────────────────────────────────────
        let chem_keywords: &[&str] = &[
            "mole", "molarity", "titration", "oxidation", "reduction",
            "electrode", "catalyst", "reaction rate", "equilibrium",
            "enthalpy", "entropy", "gibbs", "bond energy", "lattice",
            "atomic number", "electron configuration", "periodic table",
            "organic", "hydrocarbon", "ester", "polymer",
        ];

        // ── Biology ──────────────────────────────────────────────────────
        let bio_keywords: &[&str] = &[
            "cell membrane", "mitosis", "meiosis", "dna", "rna",
            "protein synthesis", "enzyme", "atp", "photosynthesis",
            "respiration", "ecosystem", "natural selection", "evolution",
            "chromosome", "allele", "genotype", "phenotype",
            "nervous system", "homeostasis", "osmosis",
        ];

        let score = |kws: &[&str]| -> usize {
            kws.iter().filter(|&&kw| lower.contains(kw)).count()
        };

        let cs = score(cs_keywords);
        let math = score(math_keywords);
        let phys = score(phys_keywords);
        let chem = score(chem_keywords);
        let bio = score(bio_keywords);

        let max = [cs, math, phys, chem, bio]
            .iter()
            .copied()
            .max()
            .unwrap_or(0);

        if max == 0 {
            return ("General", "Imported", false);
        }
        if cs == max {
            return ("Computer Science", "Algorithms & Data Structures", true);
        }
        if math == max {
            return ("Further Maths", "Pure Mathematics", false);
        }
        if phys == max {
            return ("Physics", "Mechanics & Fields", false);
        }
        if chem == max {
            return ("Chemistry", "Physical Chemistry", false);
        }
        ("Biology", "Cell Biology", false)
    }

    /// Extract the mark count from a text block.
    fn extract_marks(&self, text: &str) -> i32 {
        // Prefer the last "[N marks]" found (usually at end of question stem)
        if let Some(cap) = self.marks_re.captures_iter(text).last() {
            if let Some(m) = cap.get(1).or_else(|| cap.get(2)) {
                if let Ok(v) = m.as_str().parse::<i32>() {
                    return v.clamp(1, 25);
                }
            }
        }
        1 // default: 1 mark
    }

    /// Extract the first LaTeX math snippet present in the block, if any.
    fn extract_math(&self, text: &str) -> String {
        self.math_re
            .find(text)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default()
    }

    /// Slice a block of raw text into individual question strings.
    /// Strategy (two-pass):
    ///   1. Try splitting on numbered-question markers (Q1, 1., 1) etc.)
    ///   2. If that yields only one chunk, fall back to "---" delimiter.
    fn slice_questions<'a>(&self, text: &'a str) -> Vec<&'a str> {
        let splits: Vec<_> = self
            .q_split_re
            .split(text)
            .map(str::trim)
            .filter(|s| s.len() > 20) // ignore tiny fragments
            .collect();

        if splits.len() > 1 {
            return splits;
        }

        // Fallback: "---" delimiter (used by older import_questions)
        let fallback: Vec<_> = text
            .split("---")
            .map(str::trim)
            .filter(|s| s.len() > 20)
            .collect();

        if !fallback.is_empty() {
            return fallback;
        }

        // Last resort: treat entire text as one question
        if text.trim().len() > 20 {
            vec![text.trim()]
        } else {
            vec![]
        }
    }
}

// ── Shared DB insert logic ────────────────────────────────────────────────────

async fn insert_questions_from_text(
    pool: &sqlx::SqlitePool,
    text: &str,
    classifier: &SubjectClassifier,
) -> Result<usize, String> {
    let chunks = classifier.slice_questions(text);
    let mut inserted = 0;

    for chunk in chunks {
        let id = uuid::Uuid::new_v4().to_string();
        let (subject, subtopic, is_code) = classifier.classify(chunk);
        let marks = classifier.extract_marks(chunk);
        let math_snippet = classifier.extract_math(chunk);

        // Strip any trailing mark annotation from the display content
        // to keep the card text clean.
        let content = chunk.trim().to_string();

        sqlx::query(
            r#"
            INSERT INTO questions (id, subject, subtopic, topics, marks, content, math_snippet, is_code)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(subject)
        .bind(subtopic)
        .bind("[]")
        .bind(marks)
        .bind(&content)
        .bind(&math_snippet)
        .bind(is_code)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to insert question: {}", e))?;

        inserted += 1;
    }

    Ok(inserted)
}

// ── Tauri commands ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_all_questions(state: State<'_, AppState>) -> Result<Vec<Question>, String> {
    let pool = state.db.lock().await;
    let questions = sqlx::query_as::<_, Question>("SELECT * FROM questions")
        .fetch_all(&*pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(questions)
}

#[tauri::command]
pub async fn add_question(question: Question, state: State<'_, AppState>) -> Result<(), String> {
    let pool = state.db.lock().await;
    sqlx::query(
        r#"
        INSERT INTO questions (id, subject, subtopic, topics, marks, content, math_snippet, is_code)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(question.id)
    .bind(question.subject)
    .bind(question.subtopic)
    .bind(question.topics.unwrap_or_else(|| "[]".to_string()))
    .bind(question.marks)
    .bind(question.content)
    .bind(question.math_snippet)
    .bind(question.is_code)
    .execute(&*pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Permanently removes a single question from the database by its UUID.
#[tauri::command]
pub async fn delete_question(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let pool = state.db.lock().await;
    sqlx::query("DELETE FROM questions WHERE id = ?")
        .bind(&id)
        .execute(&*pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Import from a plain-text file (legacy "---"-delimited format or numbered questions).
#[tauri::command]
pub async fn import_questions(
    app: tauri::AppHandle,
    file_path: String,
) -> Result<usize, String> {
    let content = std::fs::read_to_string(&file_path)
        .map_err(|e| format!("Failed to read file: {}", e))?;

    let state: State<'_, AppState> = app.state();
    let pool = state.db.lock().await;

    let classifier = SubjectClassifier::new();
    insert_questions_from_text(&*pool, &content, &classifier).await
}

/// Parse a PDF (or plain-text) past paper at `file_path`, extract individual
/// questions with heuristic regex slicing, classify them by subject using
/// TF-IDF keyword scoring, and insert them into SQLite.
///
/// Returns the total number of questions inserted.
#[tauri::command]
pub async fn parse_pdf(
    app: tauri::AppHandle,
    file_path: String,
) -> Result<usize, String> {
    // ── 1. Extract text from PDF (blocking I/O → run on threadpool) ───────
    let path_clone = file_path.clone();
    let raw_text = tokio::task::spawn_blocking(move || -> Result<String, String> {
        // pdf_extract::extract_text returns the full text of all pages joined.
        // For plain-text files we fall back to std::fs::read_to_string.
        let lower = path_clone.to_lowercase();
        if lower.ends_with(".pdf") {
            pdf_extract::extract_text(&path_clone)
                .map_err(|e| format!("PDF extraction failed: {}", e))
        } else {
            std::fs::read_to_string(&path_clone)
                .map_err(|e| format!("Failed to read file: {}", e))
        }
    })
    .await
    .map_err(|e| format!("Thread-pool error: {}", e))??;

    if raw_text.trim().is_empty() {
        return Err(
            "No text could be extracted from this file. \
             It may be a scanned/image-only PDF."
                .into(),
        );
    }

    // ── 2. Pre-process: normalise whitespace artifacts from PDF extraction ─
    // pdf-extract sometimes joins words without spaces across page breaks.
    // A simple pass collapses excessive blank lines and fixes common issues.
    let cleaned = raw_text
        .lines()
        .map(|l| l.trim())
        .collect::<Vec<_>>()
        .join("\n");
    // Collapse 3+ consecutive blank lines to two (paragraph boundary)
    let cleaned = regex::Regex::new(r"\n{3,}").unwrap().replace_all(&cleaned, "\n\n");

    // ── 3. Build classifier and slice into question chunks ─────────────────
    let classifier = SubjectClassifier::new();

    // ── 4. Insert into DB ─────────────────────────────────────────────────
    let state: State<'_, AppState> = app.state();
    let pool = state.db.lock().await;

    insert_questions_from_text(&*pool, &cleaned, &classifier).await
}

#[tauri::command]
pub async fn compile_worksheet(
    app: tauri::AppHandle,
    question_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    let state: State<'_, AppState> = app.state();
    let pool = state.db.lock().await;

    let mut latex = String::new();
    latex.push_str("\\documentclass{article}\n");
    latex.push_str("\\usepackage{amsmath}\n");
    latex.push_str("\\usepackage{amssymb}\n");
    latex.push_str("\\usepackage{graphicx}\n");
    latex.push_str("\\usepackage{xcolor}\n");
    latex.push_str("\\usepackage{mdframed}\n");
    latex.push_str("\\begin{document}\n\n");
    latex.push_str("\\title{Mergemark Practice Paper}\n");
    latex.push_str("\\maketitle\n\n");
    latex.push_str("\\begin{enumerate}\n");

    let mut answer_latex = String::new();
    answer_latex.push_str("\\documentclass{article}\n");
    answer_latex.push_str("\\usepackage{amsmath}\n");
    answer_latex.push_str("\\usepackage{amssymb}\n");
    answer_latex.push_str("\\usepackage{graphicx}\n");
    answer_latex.push_str("\\usepackage{xcolor}\n");
    answer_latex.push_str("\\usepackage{mdframed}\n");
    answer_latex.push_str("\\begin{document}\n\n");
    answer_latex.push_str("\\title{Mergemark Practice Paper -- Answer Key}\n");
    answer_latex.push_str("\\maketitle\n\n");
    answer_latex.push_str("\\begin{enumerate}\n");

    for id in question_ids {
        let q: Option<Question> = sqlx::query_as("SELECT * FROM questions WHERE id = ?")
            .bind(&id)
            .fetch_optional(&*pool)
            .await
            .map_err(|e| e.to_string())?;

        if let Some(question) = q {
            let mut content = question.content.trim().to_string();
            
            // 1. Strip leading numbers (e.g., "1. ", "1)", "- ")
            let leading_num_re = regex::Regex::new(r"^\s*\d+[\.\)\-\s]*").unwrap();
            content = leading_num_re.replace(&content, "").to_string();

            // 2. Strip trailing duplicate math snippet (if the AI mistakenly appended it to the text)
            let snippet = question.math_snippet.trim();
            if !snippet.is_empty() {
                let content_trim = content.trim_end();
                if content_trim.ends_with(snippet) {
                    content = content_trim[..content_trim.len() - snippet.len()].trim_end().to_string();
                }
            }

            // 3. Fix missing inline math wrapping on bare Greek variables
            let greek_re = regex::Regex::new(r"(?x)
                (^|[\s,.\-\(])
                \\(theta|alpha|beta|gamma|pi|mu|lambda|phi|omega|sigma|delta)
                ([\s,.\-\)]|$)
            ").unwrap();
            content = greek_re.replace_all(&content, r"${1}$$\${2}$$${3}").to_string();

            // 4. Clean up markdown lists
            let list_re = regex::Regex::new(r"(?m)^[\*\-]\s+").unwrap();
            content = list_re.replace_all(&content, "\n\n").to_string();

            // Convert markdown diagram tags to LaTeX includegraphics
            while let Some(start_idx) = content.find("![Diagram](") {
                if let Some(end_idx) = content[start_idx..].find(')') {
                    let path = &content[start_idx + 11..start_idx + end_idx];
                    let latex_img = format!("\\begin{{center}}\\includegraphics[width=0.8\\linewidth]{{{}}}\\end{{center}}", path);
                    content.replace_range(start_idx..start_idx + end_idx + 1, &latex_img);
                } else {
                    break;
                }
            }

            latex.push_str(&format!("  \\item {}\n", content));
            if !question.math_snippet.is_empty() {
                if question.is_code {
                    latex.push_str(&format!(
                        "  \\begin{{verbatim}}\n{}\n  \\end{{verbatim}}\n",
                        question.math_snippet
                    ));
                } else {
                    latex.push_str(&format!("  \\[ {} \\]\n", question.math_snippet));
                }
            }
            latex.push_str(&format!("  \\hfill [{} marks]\n\n", question.marks));

            answer_latex.push_str(&format!("  \\item {}\n", content));
            if !question.math_snippet.is_empty() {
                if question.is_code {
                    answer_latex.push_str(&format!(
                        "  \\begin{{verbatim}}\n{}\n  \\end{{verbatim}}\n",
                        question.math_snippet
                    ));
                } else {
                    answer_latex.push_str(&format!("  \\[ {} \\]\n", question.math_snippet));
                }
            }
            answer_latex.push_str(&format!("  \\hfill [{} marks]\n\n", question.marks));

            if let Some(mut ans_content) = question.answer_content {
                ans_content = greek_re.replace_all(&ans_content, r"${1}$$\${2}$$${3}").to_string();
                ans_content = list_re.replace_all(&ans_content, "\n\n").to_string();

                while let Some(start_idx) = ans_content.find("![Diagram](") {
                    if let Some(end_idx) = ans_content[start_idx..].find(')') {
                        let path = &ans_content[start_idx + 11..start_idx + end_idx];
                        let latex_img = format!("\\begin{{center}}\\includegraphics[width=0.8\\linewidth]{{{}}}\\end{{center}}", path);
                        ans_content.replace_range(start_idx..start_idx + end_idx + 1, &latex_img);
                    } else {
                        break;
                    }
                }

                answer_latex.push_str("  \\vspace{0.5em}\n  \\begin{mdframed}[backgroundcolor=gray!10, linewidth=0.5pt, roundcorner=4pt]\n");
                answer_latex.push_str("  \\textbf{Mark Scheme:}\\\\[0.5em]\n");
                answer_latex.push_str(&format!("  {}\n", ans_content));
                answer_latex.push_str("  \\end{mdframed}\n\n");
            }
        }
    }

    latex.push_str("\\end{enumerate}\n");
    latex.push_str("\\end{document}\n");

    answer_latex.push_str("\\end{enumerate}\n");
    answer_latex.push_str("\\end{document}\n");

    let download_dir = app.path().download_dir().map_err(|e| e.to_string())?;
    
    let worksheet_tex = download_dir.join("worksheet.tex");
    let answer_key_tex = download_dir.join("answer_key.tex");

    std::fs::write(&worksheet_tex, &latex).map_err(|e| format!("Failed to write worksheet file: {}", e))?;
    std::fs::write(&answer_key_tex, &answer_latex).map_err(|e| format!("Failed to write answer key file: {}", e))?;

    let pdflatex_cmd = if std::process::Command::new("pdflatex").arg("--version").output().is_ok() {
        "pdflatex".to_string()
    } else if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let miktex_path = std::path::PathBuf::from(local_app_data)
            .join("Programs\\MiKTeX\\miktex\\bin\\x64\\pdflatex.exe");
        if miktex_path.exists() {
            miktex_path.to_string_lossy().to_string()
        } else {
            "pdflatex".to_string()
        }
    } else {
        "pdflatex".to_string()
    };

    let output_worksheet = std::process::Command::new(&pdflatex_cmd)
        .current_dir(&download_dir)
        .arg("-interaction=nonstopmode")
        .arg("-output-directory")
        .arg(&download_dir)
        .arg(&worksheet_tex)
        .output()
        .map_err(|e| format!("Failed to execute pdflatex for worksheet: {}", e))?;

    let worksheet_pdf = download_dir.join("worksheet.pdf");
    if !worksheet_pdf.exists() {
        let stdout = String::from_utf8_lossy(&output_worksheet.stdout);
        let stderr = String::from_utf8_lossy(&output_worksheet.stderr);
        return Err(format!("pdflatex failed to generate worksheet PDF:\n{}\n{}", stdout, stderr));
    }

    let output_answer_key = std::process::Command::new(&pdflatex_cmd)
        .current_dir(&download_dir)
        .arg("-interaction=nonstopmode")
        .arg("-output-directory")
        .arg(&download_dir)
        .arg(&answer_key_tex)
        .output()
        .map_err(|e| format!("Failed to execute pdflatex for answer key: {}", e))?;

    let answer_key_pdf = download_dir.join("answer_key.pdf");
    if !answer_key_pdf.exists() {
        let stdout = String::from_utf8_lossy(&output_answer_key.stdout);
        let stderr = String::from_utf8_lossy(&output_answer_key.stderr);
        return Err(format!("pdflatex failed to generate answer key PDF:\n{}\n{}", stdout, stderr));
    }

    let _ = std::fs::remove_file(download_dir.join("worksheet.aux"));
    let _ = std::fs::remove_file(download_dir.join("worksheet.log"));
    let _ = std::fs::remove_file(download_dir.join("answer_key.aux"));
    let _ = std::fs::remove_file(download_dir.join("answer_key.log"));

    Ok(vec![
        worksheet_pdf.to_string_lossy().to_string(),
        answer_key_pdf.to_string_lossy().to_string()
    ])
}

#[tauri::command]
pub async fn parse_pdf_vision(
    app: tauri::AppHandle,
    api_key: String,
    file_path: String,
    pdf_base64_pages: Option<Vec<String>>,
    base_url: String,
    model_name: String,
    subject: String,
    state: State<'_, AppState>,
) -> Result<Vec<Question>, String> {
    // Clean up inputs to prevent copy-paste errors
    let base_url = base_url.trim().to_string();
    let api_key = api_key.trim().to_string();
    let model_name = model_name.trim().to_string();

    // 1. Determine file type and extract content
    let ext = std::path::Path::new(&file_path).extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
    let is_image = ext == "png" || ext == "jpg" || ext == "jpeg";

    let has_pdf_pages = pdf_base64_pages.as_ref().map(|p| !p.is_empty()).unwrap_or(false);

    let text = if !is_image && !has_pdf_pages {
        match ext.as_str() {
            "txt" => tokio::fs::read_to_string(&file_path)
                .await
                .map_err(|e| e.to_string())?,
            _ => {
                let file_path_clone = file_path.clone();
                tokio::task::spawn_blocking(move || {
                    pdf_extract::extract_text(&file_path_clone)
                        .map_err(|e| format!("PDF extraction failed: {}", e))
                })
                .await
                .map_err(|e| e.to_string())??
            }
        }
    } else {
        String::new()
    };

    if !is_image && !has_pdf_pages && text.trim().is_empty() {
        return Err("File is empty or contains only unextractable images.".to_string());
    }

    // 2. Build the OpenAI prompt for structured extraction
    const EDEXCEL_MATHS_TOPICS: &[&str] = &["Proof", "Algebra and functions", "Coordinate geometry in the (x, y) plane", "Sequences and series", "Trigonometry", "Exponentials and logarithms", "Differentiation", "Integration", "Numerical methods", "Vectors", "Statistical sampling", "Data presentation and interpretation", "Probability", "Statistical distributions", "Statistical hypothesis testing", "Quantities and units in mechanics", "Kinematics", "Forces and Newton's laws", "Moments"];
    
    let system_prompt_string = format!("You are an expert exam parser. Your job is to extract EVERY question from the provided exam paper pages and return them as structured JSON. You have a generous output token budget — use it fully.

Return a JSON object with a single key 'questions' containing an array of objects. Schema for each object:
{{ \"subtopic\": string (e.g. \"Integration\"), \"topics\": [string] (1 to 3 exact matches from the list below), \"marks\": integer (sum all parts; default 1 if unknown), \"content\": string (full question text with all sub-parts), \"math_snippet\": string (extract any key LaTeX/math expression, else empty string), \"is_code\": boolean }}

You are extracting questions from a {} exam paper. Your ONLY classification task is to return a `topics` array containing 1 to 3 exact matches from this list: {:?}.
Do not invent any new tags. If a topic spans multiple areas, pick the most relevant ones.

CRITICAL — ACCURACY & GREEK LETTERS: Pay extreme attention to Greek letters (especially theta). Ensure they are properly translated to LaTeX (e.g., `$\\theta$`) and never dropped or mistakenly transcribed as a zero or the letter 'O'.
CRITICAL — NO SUMMARIES: DO NOT summarize, repeat, or list equations at the bottom of the text. You must only extract the text exactly as it flows in the document.

CRITICAL — COMPLETENESS: You MUST extract every single question. A typical A-level paper has 10–15+ questions. Do NOT stop early. Continue until 'END OF PAPER' or the last blank page. Count every numbered question. If you are running low on space, emit shorter content fields rather than omitting questions entirely.
CRITICAL: You are extracting from a high-stakes exam paper. You MUST NOT skip, summarize, or omit ANY questions.
You MUST extract EVERY SINGLE QUESTION visible in the provided pages.
Do not stop generating the array until you have perfectly transcribed every question present in the images.
If a question spans across multiple pages, ensure you capture the entirety of it into a single object.

CRITICAL — MULTI-PART QUESTIONS: Group all parts of one question (e.g. 3a, 3b, 3c) into a single JSON object. Sum their marks. Insert \\n\\n before each sub-part label like (a), (b), (i).

CRITICAL — MATH FORMATTING RULES (follow exactly):
1. Use $$ ... $$ (with a blank line before and after) for any equation that appears large, on its own line, or contains \\int, \\sum, \\prod, \\frac (as main expression), \\lim, or similar.
   BAD:  \"Show that $\\int_0^1 x^2\\,dx = \\frac{{1}}{{3}}$\"
   GOOD: \"Show that\\n\\n$$\\int_0^1 x^2\\,dx = \\frac{{1}}{{3}}$$\\n\\n\"
2. Use $ ... $ ONLY for small inline variables like $x$, $n$, $A$, $f(x)$.
   BAD:  \"The function $f(x) = 3x^2 - 2$ where $x \\in \\mathbb{{R}}$\" (if f(x)=... is a definition)
   GOOD: \"The function\\n\\n$$f(x) = 3x^2 - 2, \\quad x \\in \\mathbb{{R}}$$\\n\\n\"
3. NEVER put $ on its own line. NEVER use triple backticks for math.
4. NEVER append equation summaries at the end of questions.

CRITICAL — DIAGRAMS: If a question references a diagram/graph/figure, return diagram_bbox as [x, y, width, height] in relative 0.0–1.0 page coordinates. Also return diagram_page (0-indexed page number). Set diagram_bbox to null if there is no diagram. Do NOT flag ruled answer lines or blank spaces as diagrams.
CRITICAL: When calculating the `[x, y, width, height]` bounding box for a diagram, you MUST capture the absolute full extent of the figure.
You MUST explicitly include the 'Figure X' label (usually at the bottom) within the bounding box.
You MUST explicitly include any 'Diagram NOT accurately drawn' or 'Not to scale' warnings (usually floating in the top right corner) within the bounding box.
Ensure the bounding box stretches far enough to the edges to include all axis labels (e.g., x, y, O), curve extremes, and full geometric shapes without slicing them in half.", subject, EDEXCEL_MATHS_TOPICS);
    
    let system_prompt = system_prompt_string.as_str();

    let mut requests_to_make = Vec::new();

    if has_pdf_pages {
        let pages = pdf_base64_pages.as_ref().unwrap();
        // 2 pages per batch with a 1-page overlap.
        // This ensures every question is fully visible in at least one batch,
        // even if it straddles a page boundary.
        // Each call only needs to output ~2-4 questions, well within any token limit.
        let window_size: usize = 5;  // 4 "new" pages + 1 overlap page shown as context
        let step: usize = 4;         // advance 4 pages between batches
        let mut start: usize = 0;
        while start < pages.len() {
            let end = (start + window_size).min(pages.len());
            let chunk = &pages[start..end];
            // Tell the model exactly which pages it is looking at so it only
            // extracts questions that BEGIN on the primary (non-overlap) pages.
            let primary_start = start + 1;   // 1-indexed for human readability
            let primary_end = (start + step).min(pages.len());
            let context_note = if start == 0 {
                format!("These are pages 1\u{2013}{} of the exam paper. Extract every question that begins on any of these pages.", end)
            } else {
                format!(
                    "Page {} is shown for context (already processed in the previous batch). \
                     Extract ONLY questions that begin on page{} {}{}.",
                    start,           // 1-indexed overlap page
                    if primary_end > primary_start { "s" } else { "" },
                    primary_start,
                    if primary_end > primary_start { format!("\u{2013}{}", primary_end) } else { String::new() }
                )
            };
            let mut content_array = vec![serde_json::json!({ "type": "text", "text": context_note })];
            for page_b64 in chunk {
                content_array.push(serde_json::json!({
                    "type": "image_url",
                    "image_url": { "url": format!("data:image/jpeg;base64,{}", page_b64) }
                }));
            }
            let req_body = serde_json::json!({
                "model": &model_name,
                "messages": [
                    { "role": "system", "content": system_prompt },
                    { "role": "user", "content": content_array }
                ],
                "temperature": 0.1,
                "max_tokens": 16384,
                "response_format": { "type": "json_object" }
            });
            requests_to_make.push((req_body, start));
            if end >= pages.len() { break; }
            start += step;
        }
    } else if is_image {
        use base64::Engine;
        let image_bytes = tokio::fs::read(&file_path).await.map_err(|e| format!("Failed to read image: {}", e))?;
        let base64_image = base64::engine::general_purpose::STANDARD.encode(&image_bytes);
        let mime_type = if ext == "png" { "image/png" } else { "image/jpeg" };
        
        let req_body = serde_json::json!({
            "model": &model_name,
            "messages": [
                { "role": "system", "content": system_prompt },
                { 
                    "role": "user", 
                    "content": [
                        { "type": "text", "text": "Extract all questions from this exam paper image." },
                        { "type": "image_url", "image_url": { "url": format!("data:{};base64,{}", mime_type, base64_image) } }
                    ]
                }
            ],
            "temperature": 0.1,
            "max_tokens": 16384,
            "response_format": { "type": "json_object" }
        });
        requests_to_make.push((req_body, 0));
    } else {
        let req_body = serde_json::json!({
            "model": &model_name,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": text }
            ],
            "temperature": 0.1,
            "max_tokens": 16384,
            "response_format": { "type": "json_object" }
        });
        requests_to_make.push((req_body, 0));
    }

    let client = reqwest::Client::new();
    let pool = state.db.lock().await;
    let classifier = SubjectClassifier::new();
    let mut final_questions = Vec::new();

    #[derive(serde::Deserialize)]
    struct ExtractedQuestion {
        subject: Option<String>,
        subtopic: Option<String>,
        topics: Option<Vec<String>>,
        marks: Option<i32>,
        content: Option<String>,
        math_snippet: Option<String>,
        is_code: Option<bool>,
        diagram_bbox: Option<Vec<f32>>,
        diagram_page: Option<usize>,
    }

    #[derive(serde::Deserialize)]
    struct OpenAIResult {
        #[serde(default)]
        questions: Vec<ExtractedQuestion>,
    }

    // Tracks content fingerprints across all batches to deduplicate questions
    // that appear in the 1-page overlap between consecutive batches.
    let mut seen_fingerprints: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (req_body, base_page_offset) in requests_to_make {
        let mut retry_count = 0;
        let mut response_json: Option<serde_json::Value> = None;

        while retry_count < 3 {
            let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
            let res = client
                .post(&url)
                .header("Authorization", format!("Bearer {}", api_key))
                .json(&req_body)
                .send()
                .await
                .map_err(|e| format!("OpenAI network error: {}", e))?;

            if res.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                retry_count += 1;
                // OpenAI token rate limits can take a minute to reset.
                // Sleep for 20s, then 40s if it fails again.
                tokio::time::sleep(tokio::time::Duration::from_secs(20 * retry_count)).await;
                continue;
            }

            if !res.status().is_success() {
                let err_text = res.text().await.unwrap_or_default();
                return Err(format!("OpenAI API error: {}", err_text));
            }

            response_json = Some(res.json().await.map_err(|e| e.to_string())?);
            break;
        }

        let response_json = response_json.ok_or_else(|| "Failed to get response from OpenAI after retries (Rate Limited).".to_string())?;

        // Warn if the model hit the token limit mid-response (finish_reason == "length").
        // This means the JSON was truncated and some questions at the end of this batch
        // may be missing. The fix is to increase max_tokens or reduce batch_size.
        if let Some(finish_reason) = response_json["choices"][0]["finish_reason"].as_str() {
            if finish_reason == "length" {
                eprintln!("[MergeMark] WARNING: OpenAI response was truncated (finish_reason=length). \
                    Some questions in this batch may be missing. \
                    Consider reducing the PDF batch size or raising max_tokens.");
            }
        }

        let mut content_str = response_json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or("Invalid OpenAI response format")?
            .trim();

        // Non-OpenAI models (Claude, Gemini) often ignore the JSON mode instruction
        // and include conversational text (e.g. "Here is the JSON: ```json ... ```").
        // We find the first '{' and last '}' to extract only the JSON object.
        if let Some(start) = content_str.find('{') {
            content_str = &content_str[start..];
        }
        if content_str.ends_with("```") {
            content_str = &content_str[..content_str.len() - 3].trim_end();
        }

        // Sanitize invalid JSON escapes. Non-OpenAI models often output literal single
        // backslashes for LaTeX. We must escape them to double backslashes for serde_json,
        // EXCEPT for valid JSON escapes like \n, \", etc.
        // We also explicitly detect LaTeX commands that collide with JSON escapes (like \text vs \t).
        let chars_vec: Vec<char> = content_str.chars().collect();
        let mut sanitized = String::with_capacity(content_str.len() + 100);
        let mut i = 0;
        while i < chars_vec.len() {
            let c = chars_vec[i];
            if c == '\\' {
                if i + 1 < chars_vec.len() {
                    let next_c = chars_vec[i + 1];
                    let mut is_latex = false;
                    
                    // Lookahead to check for specific LaTeX commands that collide with JSON escapes
                    let remaining: String = chars_vec[i+1..].iter().take(6).collect();
                    if remaining.starts_with("text") ||
                       remaining.starts_with("frac") ||
                       remaining.starts_with("theta") ||
                       remaining.starts_with("tan") ||
                       remaining.starts_with("times") ||
                       remaining.starts_with("rho") ||
                       remaining.starts_with("right") ||
                       remaining.starts_with("beta") ||
                       remaining.starts_with("binom") ||
                       remaining.starts_with("nabla") ||
                       remaining.starts_with("nu") ||
                       remaining.starts_with("notin") ||
                       remaining.starts_with("ne") {
                           is_latex = true;
                    }

                    match next_c {
                        '"' | '\\' | '/' | 'u' => {
                            if next_c == '\\' {
                                sanitized.push_str("\\\\");
                                i += 1; // consume second backslash
                            } else {
                                sanitized.push('\\');
                            }
                        }
                        'n' | 'r' | 't' | 'b' | 'f' => {
                            if is_latex {
                                sanitized.push_str("\\\\");
                            } else {
                                sanitized.push('\\'); // Keep valid JSON escape
                            }
                        }
                        _ => {
                            // Any other character after a backslash is an invalid JSON escape (e.g. \int, \lim), so it must be LaTeX
                            sanitized.push_str("\\\\");
                        }
                    }
                } else {
                    sanitized.push('\\');
                }
            } else {
                sanitized.push(c);
            }
            i += 1;
        }

        let parsed: OpenAIResult = match serde_json::from_str(&sanitized) {
            Ok(p) => p,
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("trailing characters") {
                    if let Some(end) = sanitized.rfind('}') {
                        let chopped = &sanitized[..=end];
                        serde_json::from_str(chopped).map_err(|e2| format!("Failed to parse OpenAI JSON after trailing chop: {}", e2))?
                    } else {
                        return Err(format!("Failed to parse OpenAI JSON: {}\nContent starts with: {}...", err_str, sanitized.chars().take(50).collect::<String>()));
                    }
                } else {
                    let mut current = sanitized.clone();
                    let mut attempts = 0;
                    let mut recovered: Option<OpenAIResult> = None;
                    
                    while attempts < 2000 && !current.is_empty() {
                        let closed = auto_close_json(&current);
                        if let Ok(p) = serde_json::from_str::<OpenAIResult>(&closed) {
                            recovered = Some(p);
                            break;
                        }
                        current.pop();
                        attempts += 1;
                    }
                    
                    if let Some(p) = recovered {
                        p
                    } else {
                        let auto_closed = auto_close_json(&sanitized);
                        let final_err = serde_json::from_str::<OpenAIResult>(&auto_closed).err().map(|e| e.to_string()).unwrap_or_else(|| "Unknown".to_string());
                        return Err(format!("The AI model truncated the response. Attempted robust recovery failed: {}\nContent starts with: {}...", final_err, auto_closed.chars().take(50).collect::<String>()));
                    }
                }
            }
        };

        for mut q in parsed.questions {
            let mut q_content = match q.content.clone() {
                Some(c) if !c.trim().is_empty() => c,
                _ => continue, // Skip questions without content
            };

            // Deduplicate: build a fingerprint from the first ~20 words of the content.
            // Questions seen in a previous overlapping batch will have the same fingerprint.
            let fingerprint: String = q_content
                .split_whitespace()
                .take(20)
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            if !seen_fingerprints.insert(fingerprint) {
                // Already extracted from a previous (overlapping) batch — skip.
                continue;
            }

            let id = uuid::Uuid::new_v4().to_string();

            // 3. Diagram cropping and saving logic
            if let Some(bbox) = &q.diagram_bbox {
                if bbox.len() == 4 {
                    let img_result = if let Some(pages) = &pdf_base64_pages {
                        let page_index = base_page_offset + q.diagram_page.unwrap_or(0);
                        if let Some(b64) = pages.get(page_index) {
                            use base64::Engine;
                            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                                image::load_from_memory(&bytes).map_err(|_| ())
                            } else {
                                Err(())
                            }
                        } else {
                            Err(())
                        }
                    } else {
                        image::open(&file_path).map_err(|_| ())
                    };

                    if let Ok(mut img) = img_result {
                        let (width, height) = img.dimensions();
                        
                        const PADDING_PERCENT: f32 = 0.05;
                        let x_rel = bbox[0];
                        let y_rel = bbox[1];
                        let w_rel = bbox[2];
                        let h_rel = bbox[3];

                        let padded_x = (x_rel - PADDING_PERCENT).max(0.0);
                        let padded_y = (y_rel - PADDING_PERCENT).max(0.0);
                        let padded_w = (w_rel + (PADDING_PERCENT * 2.0)).min(1.0 - padded_x);
                        let padded_h = (h_rel + (PADDING_PERCENT * 2.0)).min(1.0 - padded_y);

                        let x = (padded_x * width as f32).round() as u32;
                        let y = (padded_y * height as f32).round() as u32;
                        let w = (padded_w * width as f32).max(1.0).round() as u32;
                        let h = (padded_h * height as f32).max(1.0).round() as u32;

                        // Ensure we don't crop outside image bounds
                        let crop_w = w.min(width.saturating_sub(x));
                        let crop_h = h.min(height.saturating_sub(y));

                        if crop_w > 0 && crop_h > 0 {
                            let cropped = image::imageops::crop(&mut img, x, y, crop_w, crop_h).to_image();
                            
                            if let Ok(app_data_dir) = app.path().app_data_dir() {
                                let diagrams_dir = app_data_dir.join("diagrams");
                                let _ = std::fs::create_dir_all(&diagrams_dir);
                                
                                let img_uuid = uuid::Uuid::new_v4().to_string();
                                let img_path = diagrams_dir.join(format!("{}.png", img_uuid));
                                
                                if cropped.save(&img_path).is_ok() {
                                    // Important: Convert path to a proper URL format for markdown
                                    // On Windows, absolute paths need special handling in markdown/browsers
                                    // However, requirements specifically asked for absolute local path:
                                    let link = format!("\n\n![Diagram]({})\n\n", img_path.to_string_lossy().replace('\\', "/"));
                                    q_content.push_str(&link);
                                }
                            }
                        }
                    }
                }
            }

            let (_, sys_subtopic, sys_is_code) = classifier.classify(&q_content);
            let final_subject = subject.clone();
            let final_subtopic = if sys_subtopic == "Unknown" { q.subtopic.unwrap_or_else(|| "Unknown".to_string()) } else { sys_subtopic.to_string() };
            let final_is_code = if sys_is_code { true } else { q.is_code.unwrap_or(false) };

            let final_topics = serde_json::to_string(&q.topics.unwrap_or_default()).unwrap_or_else(|_| "[]".to_string());

            let marks_val = q.marks.unwrap_or(1);
            let snippet_val = q.math_snippet.unwrap_or_default();

            sqlx::query(
                r#"
                INSERT INTO questions (id, subject, subtopic, topics, marks, content, math_snippet, is_code)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(id.clone())
            .bind(final_subject.clone())
            .bind(final_subtopic.clone())
            .bind(final_topics.clone())
            .bind(marks_val)
            .bind(q_content.clone())
            .bind(snippet_val.clone())
            .bind(final_is_code)
            .execute(&*pool)
            .await
            .map_err(|e| format!("DB error: {}", e))?;

            final_questions.push(Question {
                id,
                subject: final_subject,
                subtopic: final_subtopic,
                marks: marks_val,
                content: q_content,
                math_snippet: snippet_val,
                is_code: final_is_code,
                answer_content: None,
                topics: Some(final_topics),
            });
        }

        thread::sleep(Duration::from_millis(1500));
    }

    Ok(final_questions)
}

#[tauri::command]
pub async fn fetch_models(base_url: String, api_key: String) -> Result<Vec<String>, String> {
    let client = reqwest::Client::new();
    let base_url = base_url.trim();
    let api_key = api_key.trim();
    
    // Some endpoints use /v1, some /v1beta/openai. For models list, standard OpenAI is /models on the base url.
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    
    let res = client.get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;
        
    if !res.status().is_success() {
        let err_text = res.text().await.unwrap_or_default();
        return Err(format!("API error: {}", err_text));
    }
    
    let json: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    
    let mut models = Vec::new();
    if let Some(data) = json["data"].as_array() {
        for item in data {
            if let Some(id) = item["id"].as_str() {
                models.push(id.to_string());
            }
        }
    }
    
    models.sort();
    Ok(models)
}

#[tauri::command]
pub async fn delete_all_questions(state: State<'_, AppState>) -> Result<bool, String> {
    let pool = state.db.lock().await;
    sqlx::query("DELETE FROM questions")
        .execute(&*pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
pub async fn update_question(
    app: tauri::AppHandle,
    id: String,
    new_content: String,
    new_marks: i32,
    new_answer_content: Option<String>,
) -> Result<(), String> {
    use tauri::Manager;
    let state = app.state::<AppState>();
    let pool = state.db.lock().await;
    sqlx::query("UPDATE questions SET content = ?, marks = ?, answer_content = ? WHERE id = ?")
        .bind(new_content)
        .bind(new_marks)
        .bind(new_answer_content)
        .bind(id)
        .execute(&*pool)
        .await
        .map_err(|e| format!("Failed to update question: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn parse_mark_scheme_vision(
    _app: tauri::AppHandle,
    api_key: String,
    file_path: String,
    pdf_base64_pages: Option<Vec<String>>,
    base_url: String,
    model_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<ProposedMapping>, String> {
    let base_url = base_url.trim().to_string();
    let api_key = api_key.trim().to_string();
    let model_name = model_name.trim().to_string();

    let ext = std::path::Path::new(&file_path).extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
    let is_image = ext == "png" || ext == "jpg" || ext == "jpeg";
    let has_pdf_pages = pdf_base64_pages.as_ref().map(|p| !p.is_empty()).unwrap_or(false);

    let text = if !is_image && !has_pdf_pages {
        match ext.as_str() {
            "txt" => tokio::fs::read_to_string(&file_path).await.map_err(|e| e.to_string())?,
            _ => {
                let file_path_clone = file_path.clone();
                tokio::task::spawn_blocking(move || {
                    pdf_extract::extract_text(&file_path_clone).map_err(|e| format!("PDF extraction failed: {}", e))
                }).await.map_err(|e| e.to_string())??
            }
        }
    } else {
        String::new()
    };

    if !is_image && !has_pdf_pages && text.trim().is_empty() {
        return Err("File is empty or contains only unextractable images.".to_string());
    }

    let system_prompt = r#"You are an expert examiner. Extract the final answers and grading logic from this mark scheme. Return a JSON array containing `question_number` (String, e.g., '1' or '2') and `answer_content` (String, formatted perfectly in Markdown and LaTeX). Return a JSON object with a single key 'answers' containing this array.

CRITICAL FORMATTING RULES:
1. CRITICAL: NEVER use markdown code blocks (triple backticks) like ```latex. Return the raw text directly.
2. CRITICAL - MULTI-PART QUESTIONS: Group all parts of one question (e.g., 1a, 1b, 1c) into a SINGLE JSON object in the array. Do NOT create separate array items for each sub-part. Use the main question number (e.g., '1', '2') as the `question_number`.
3. CRITICAL FORMATTING RULE: You must structure the answer step-by-step with massive spacing. NEVER cram working out into a single line or a single inline math block.
4. Part labels MUST be bolded on their own line (e.g., **(a)**).
5. EVERY single distinct marking point, step, or line of working MUST be separated by a double newline (`\n\n`).
6. Extract the textual description of the step (e.g., 'Finds the area of $R_1$') as standard text. Only use inline math (`$`) for small variables within these sentences.
7. The main equations, substitutions, and final answers MUST be formatted as display/block math (`$$ equation $$`) so they render centered on their own distinct line.

TEMPLATE TO FOLLOW FOR EACH PART:
**(a)** Finds the area of $R_1$
\n\n
$$ R_1 = \frac{1}{2}r^2(\theta - \sin\theta) $$
\n\n
Uses the ratio $R_1 = 2R_2$
\n\n
$$ \frac{1}{2}r^2(\theta - \sin\theta) = 2 \cdot \frac{1}{2}r^2((\pi - \theta) - \sin\theta) $$"#;

    let mut requests_to_make = Vec::new();

    if has_pdf_pages {
        let pages = pdf_base64_pages.as_ref().unwrap();
        let window_size: usize = 3;
        let step: usize = 2;
        let mut start: usize = 0;
        while start < pages.len() {
            let end = (start + window_size).min(pages.len());
            let chunk = &pages[start..end];
            let primary_start = start + 1;
            let primary_end = (start + step).min(pages.len());
            let context_note = if start == 0 {
                format!("These are pages 1\u{2013}{} of the mark scheme. Extract every answer that begins on any of these pages.", end)
            } else {
                format!(
                    "Page {} is shown for context (already processed in the previous batch). \
                     Extract ONLY answers that begin on page{} {}{}.",
                    start,
                    if primary_end > primary_start { "s" } else { "" },
                    primary_start,
                    if primary_end > primary_start { format!("\u{2013}{}", primary_end) } else { String::new() }
                )
            };
            let mut content_array = vec![serde_json::json!({ "type": "text", "text": context_note })];
            for page_b64 in chunk {
                content_array.push(serde_json::json!({
                    "type": "image_url",
                    "image_url": { "url": format!("data:image/jpeg;base64,{}", page_b64) }
                }));
            }
            let req_body = serde_json::json!({
                "model": &model_name,
                "messages": [
                    { "role": "system", "content": system_prompt },
                    { "role": "user", "content": content_array }
                ],
                "temperature": 0.1,
                "max_tokens": 16384,
                "response_format": { "type": "json_object" }
            });
            requests_to_make.push((req_body, start));
            if end >= pages.len() { break; }
            start += step;
        }
    } else if is_image {
        use base64::Engine;
        let image_bytes = tokio::fs::read(&file_path).await.map_err(|e| format!("Failed to read image: {}", e))?;
        let base64_image = base64::engine::general_purpose::STANDARD.encode(&image_bytes);
        let mime_type = if ext == "png" { "image/png" } else { "image/jpeg" };
        let req_body = serde_json::json!({
            "model": &model_name,
            "messages": [
                { "role": "system", "content": system_prompt },
                { 
                    "role": "user", 
                    "content": [
                        { "type": "text", "text": "Extract all answers from this mark scheme image." },
                        { "type": "image_url", "image_url": { "url": format!("data:{};base64,{}", mime_type, base64_image) } }
                    ]
                }
            ],
            "temperature": 0.1,
            "max_tokens": 4096,
            "response_format": { "type": "json_object" }
        });
        requests_to_make.push((req_body, 0));
    } else {
        let req_body = serde_json::json!({
            "model": &model_name,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": text }
            ],
            "temperature": 0.1,
            "max_tokens": 4096,
            "response_format": { "type": "json_object" }
        });
        requests_to_make.push((req_body, 0));
    }

    let client = reqwest::Client::new();
    let pool = state.db.lock().await;

    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct ExtractedAnswer {
        question_number: Option<String>,
        answer_content: Option<String>,
    }

    #[derive(serde::Deserialize)]
    struct OpenAIAnswerResult {
        #[serde(default)]
        answers: Vec<ExtractedAnswer>,
    }

    let mut all_answers = Vec::new();
    let mut seen_fingerprints = std::collections::HashSet::new();

    for (req_body, _) in requests_to_make {
        let mut retry_count = 0;
        let mut response_json: Option<serde_json::Value> = None;

        while retry_count < 3 {
            let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
            let res = client
                .post(&url)
                .header("Authorization", format!("Bearer {}", api_key))
                .json(&req_body)
                .send()
                .await
                .map_err(|e| format!("OpenAI network error: {}", e))?;

            if res.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                retry_count += 1;
                tokio::time::sleep(tokio::time::Duration::from_secs(20 * retry_count)).await;
                continue;
            }

            if !res.status().is_success() {
                let err_text = res.text().await.unwrap_or_default();
                return Err(format!("OpenAI API error: {}", err_text));
            }

            response_json = Some(res.json().await.map_err(|e| e.to_string())?);
            break;
        }

        let response_json = response_json.ok_or_else(|| "Failed to get response from OpenAI after retries (Rate Limited).".to_string())?;

        let mut content_str = response_json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or("Invalid OpenAI response format")?
            .trim();

        if let Some(start) = content_str.find('{') {
            content_str = &content_str[start..];
        }
        if content_str.ends_with("```") {
            content_str = &content_str[..content_str.len() - 3].trim_end();
        }

        let chars_vec: Vec<char> = content_str.chars().collect();
        let mut sanitized = String::with_capacity(content_str.len() + 100);
        let mut i = 0;
        while i < chars_vec.len() {
            let c = chars_vec[i];
            if c == '\\' {
                if i + 1 < chars_vec.len() {
                    let next_c = chars_vec[i + 1];
                    let mut is_latex = false;
                    
                    let remaining: String = chars_vec[i+1..].iter().take(6).collect();
                    if remaining.starts_with("text") ||
                       remaining.starts_with("frac") ||
                       remaining.starts_with("theta") ||
                       remaining.starts_with("tan") ||
                       remaining.starts_with("times") ||
                       remaining.starts_with("rho") ||
                       remaining.starts_with("right") ||
                       remaining.starts_with("beta") ||
                       remaining.starts_with("binom") ||
                       remaining.starts_with("nabla") ||
                       remaining.starts_with("nu") ||
                       remaining.starts_with("notin") ||
                       remaining.starts_with("ne") {
                           is_latex = true;
                    }

                    match next_c {
                        '"' | '\\' | '/' | 'u' => {
                            if next_c == '\\' {
                                sanitized.push_str("\\\\");
                                i += 1;
                            } else {
                                sanitized.push('\\');
                            }
                        }
                        'n' | 'r' | 't' | 'b' | 'f' => {
                            if is_latex {
                                sanitized.push_str("\\\\");
                            } else {
                                sanitized.push('\\');
                            }
                        }
                        _ => {
                            sanitized.push_str("\\\\");
                        }
                    }
                } else {
                    sanitized.push('\\');
                }
            } else {
                sanitized.push(c);
            }
            i += 1;
        }

        let parsed: OpenAIAnswerResult = match serde_json::from_str(&sanitized) {
            Ok(p) => p,
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("trailing characters") {
                    if let Some(end) = sanitized.rfind('}') {
                        let chopped = &sanitized[..=end];
                        serde_json::from_str(chopped).map_err(|e2| format!("Failed to parse OpenAI JSON after trailing chop: {}", e2))?
                    } else {
                        return Err(format!("Failed to parse OpenAI JSON: {}\nContent starts with: {}...", err_str, sanitized.chars().take(50).collect::<String>()));
                    }
                } else {
                    let mut current = sanitized.clone();
                    let mut attempts = 0;
                    let mut recovered: Option<OpenAIAnswerResult> = None;
                    
                    while attempts < 2000 && !current.is_empty() {
                        let closed = auto_close_json(&current);
                        if let Ok(p) = serde_json::from_str::<OpenAIAnswerResult>(&closed) {
                            recovered = Some(p);
                            break;
                        }
                        current.pop();
                        attempts += 1;
                    }
                    
                    if let Some(p) = recovered {
                        p
                    } else {
                        let auto_closed = auto_close_json(&sanitized);
                        let final_err = serde_json::from_str::<OpenAIAnswerResult>(&auto_closed).err().map(|e| e.to_string()).unwrap_or_else(|| "Unknown".to_string());
                        return Err(format!("The AI model truncated the response. Attempted robust recovery failed: {}\nContent starts with: {}...", final_err, auto_closed.chars().take(50).collect::<String>()));
                    }
                }
            }
        };

        // We do not return an error here if answers is empty, as it could just be a blank page or title page.


        for ans in parsed.answers {
            let ans_content = match ans.answer_content {
                Some(ref c) if !c.trim().is_empty() => c.clone(),
                _ => continue, // Skip answers without content
            };

            let fingerprint: String = ans_content
                .split_whitespace()
                .take(20)
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            if !seen_fingerprints.insert(fingerprint) {
                continue;
            }
            
            // For now, if the question_number isn't present, we'll assign a placeholder or skip it.
            // But currently the code just uses the order of answers.
            all_answers.push(ExtractedAnswer {
                question_number: ans.question_number,
                answer_content: Some(ans_content),
            });
        }

        thread::sleep(Duration::from_millis(1500));
    }

    if all_answers.is_empty() {
        return Err("The AI model failed to return any answers from the entire document. It may have hit a safety filter, timed out, or encountered an unreadable document.".to_string());
    }

    let questions: Vec<Question> = sqlx::query_as("SELECT * FROM questions WHERE answer_content IS NULL OR trim(answer_content) = '' ORDER BY rowid ASC")
        .fetch_all(&*pool)
        .await
        .map_err(|e| format!("DB error: {}", e))?;

    let mut proposed_mappings = Vec::new();
    let mut used_q_indices = std::collections::HashSet::new();

    for (i, ans) in all_answers.into_iter().enumerate() {
        let ans_content = ans.answer_content.unwrap_or_default();
        if ans_content.trim().is_empty() {
            continue;
        }

        let mut matched_q_idx = None;

        if let Some(ref q_num) = ans.question_number {
            let q_num_clean = q_num.trim().to_lowercase();
            for (q_idx, q) in questions.iter().enumerate() {
                if used_q_indices.contains(&q_idx) {
                    continue;
                }
                
                let content_clean = q.content.trim().to_lowercase();
                
                // Strip common prefix "question " if present
                let mut content_test = content_clean.as_str();
                if content_test.starts_with("question ") {
                    content_test = &content_test["question ".len()..];
                }
                content_test = content_test.trim_start();
                
                let mut q_num_test = q_num_clean.as_str();
                if q_num_test.starts_with("question ") {
                    q_num_test = &q_num_test["question ".len()..];
                }
                q_num_test = q_num_test.trim_start();

                if content_test.starts_with(q_num_test) {
                    matched_q_idx = Some(q_idx);
                    break;
                }
            }
        }

        if matched_q_idx.is_none() {
            // Fallback to array index order
            if i < questions.len() && !used_q_indices.contains(&i) {
                matched_q_idx = Some(i);
            } else {
                for (q_idx, _) in questions.iter().enumerate() {
                    if !used_q_indices.contains(&q_idx) {
                        matched_q_idx = Some(q_idx);
                        break;
                    }
                }
            }
        }

        if let Some(q_idx) = matched_q_idx {
            used_q_indices.insert(q_idx);
            let q = &questions[q_idx];
            proposed_mappings.push(ProposedMapping {
                question_id: q.id.clone(),
                raw_content: q.content.clone(),
                proposed_answer: ans_content,
            });
        }
    }

    Ok(proposed_mappings)
}

#[tauri::command]
pub async fn commit_mark_schemes(
    mappings: Vec<ProposedMapping>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let pool = state.db.lock().await;

    for mapping in mappings {
        sqlx::query("UPDATE questions SET answer_content = ? WHERE id = ?")
            .bind(mapping.proposed_answer)
            .bind(mapping.question_id)
            .execute(&*pool)
            .await
            .map_err(|e| format!("DB update error: {}", e))?;
    }

    Ok(())
}
