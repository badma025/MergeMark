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
    #[sqlx(default)]
    pub paper_name: String,
    #[sqlx(default)]
    pub question_number: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposedMapping {
    pub question_id: String,
    pub raw_content: String,
    pub proposed_answer: String,
    pub paper_name: String,
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
    paper_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<Question>, String> {
    let base_url = base_url.trim().to_string();
    let api_key = api_key.trim().to_string();
    let model_name = model_name.trim().to_string();
    
    // Check if we have pages
    let has_pdf_pages = pdf_base64_pages.as_ref().map(|p| !p.is_empty()).unwrap_or(false);
    if !has_pdf_pages {
        return Err("No rasterized PDF pages provided.".into());
    }

    let pdf_pages = pdf_base64_pages.unwrap();
    let num_pages = pdf_pages.len();
    
    // --- DUAL-VERIFICATION FIREWALL (ASYNC) ---
    let mut effective_num_pages = num_pages;
    let client = reqwest::Client::new();
    
    for page_idx in 0..num_pages {
        let file_path_clone = file_path.clone();
        
        let raw_text = tokio::task::spawn_blocking(move || {
            let doc = match pdf_extract::Document::load(&file_path_clone) {
                Ok(d) => d,
                Err(_) => return String::new(),
            };
            let mut output = BBoxOutput { text_data: Vec::new(), pdf_page_height: 0.0 };
            if pdf_extract::output_doc_page(&doc, &mut output, (page_idx + 1) as u32).is_ok() {
                output.text_data.iter().map(|(c, _, _)| c.as_str()).collect::<String>()
            } else {
                String::new()
            }
        }).await.unwrap_or_default();
        
        let norm_text = raw_text.to_lowercase().replace(|c: char| c.is_whitespace(), "");
        if norm_text.contains("totalforpaper") || 
           norm_text.contains("endofquestionpaper") || 
           (norm_text.contains("answerbooklet") && norm_text.contains("centrenumber")) {
            effective_num_pages = page_idx + 1;
            break;
        }
        
        let text_len = raw_text.trim().len();
        if text_len > 250 {
            continue; 
        }
        
        let b64_data_str = &pdf_pages[page_idx];
        let b64_data = if b64_data_str.starts_with("data:image") {
            b64_data_str.split(',').nth(1).unwrap_or(b64_data_str)
        } else {
            b64_data_str
        };
        
        let prompt = "You are an exam document classifier. If this page is explicitly an 'Answer Booklet' (e.g., contains lined paper for answers, 'Do not write in this area' margins, or a secondary cover page with 'Candidate Number'), return EXACTLY 'ANSWER_BOOKLET'. If it is a normal question, or just a page that says 'BLANK PAGE', return EXACTLY 'QUESTION_PAPER'.";
        let req_body = serde_json::json!({
            "model": &model_name,
            "messages": [
                { "role": "user", "content": [
                    { "type": "text", "text": prompt },
                    { "type": "image_url", "image_url": { "url": format!("data:image/jpeg;base64,{}", b64_data) } }
                ]}
            ],
            "temperature": 0.1,
            "max_tokens": 10
        });
        
        let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
        let res = client.post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .timeout(std::time::Duration::from_secs(30))
            .json(&req_body)
            .send()
            .await;
            
        if let Ok(response) = res {
            if response.status().is_success() {
                if let Ok(json) = response.json::<serde_json::Value>().await {
                    let ai_resp = json["choices"][0]["message"]["content"].as_str().unwrap_or_default().trim();
                    if ai_resp.contains("ANSWER_BOOKLET") {
                        effective_num_pages = page_idx + 1;
                        break;
                    }
                }
            }
        }
    }

    let classifier = SubjectClassifier::new();
    let mut aggregated_questions: std::collections::BTreeMap<String, Question> = std::collections::BTreeMap::new();
    
    const EDEXCEL_MATHS_TOPICS: &[&str] = &["Proof", "Algebra and functions", "Coordinate geometry in the (x, y) plane", "Sequences and series", "Trigonometry", "Exponentials and logarithms", "Differentiation", "Integration", "Numerical methods", "Vectors", "Statistical sampling", "Data presentation and interpretation", "Probability", "Statistical distributions", "Statistical hypothesis testing", "Quantities and units in mechanics", "Kinematics", "Forces and Newton's laws", "Moments"];
    const FURTHER_MATHS_TOPICS: &[&str] = &["Complex numbers", "Argand diagrams", "Series", "Roots of polynomials", "Volumes of revolution", "Matrices", "Linear transformations", "Proof by induction", "Vectors", "Differential equations", "Polar coordinates", "Hyperbolic functions", "Maclaurin series", "Methods in calculus", "Momentum and impulse", "Work, energy and power", "Elastic strings and springs", "Elastic collisions in one dimension", "Elastic collisions in two dimensions", "Discrete probability distributions", "Poisson distribution", "Geometric and negative binomial", "Hypothesis testing", "Central Limit Theorem", "Chi-squared tests", "Probability generating functions", "Quality of tests", "Vectors (Cross product & planes)", "Conic sections", "Inequalities", "t-formulae", "Taylor series", "Numerical methods (Further)", "Reducible differential equations", "Algorithms", "Graphs and networks", "Algorithms on graphs", "Route inspection", "Travelling Salesperson Problem", "Linear programming", "Simplex algorithm"];
    const PHYSICS_TOPICS: &[&str] = &["Measurements and their errors", "Particles and radiation", "Waves", "Mechanics and materials", "Electricity", "Further mechanics", "Thermal physics", "Fields and their consequences", "Nuclear physics", "Telescopes", "Classification of stars", "Cosmology"];
    const CS_TOPICS: &[&str] = &["Fundamentals of programming", "Fundamentals of data structures", "Fundamentals of algorithms", "Theory of computation", "Fundamentals of data representation", "Fundamentals of computer systems", "Computer organisation and architecture", "Consequences of uses of computing", "Communication and networking", "Fundamentals of databases", "Big Data", "Fundamentals of functional programming"];

    let allowed_topics: &[&str] = match subject.as_str() {
        "Mathematics" => EDEXCEL_MATHS_TOPICS,
        "Further Mathematics" => FURTHER_MATHS_TOPICS,
        "Physics" => PHYSICS_TOPICS,
        "Computer Science" => CS_TOPICS,
        _ => &[],
    };

    let mut current_question_num = String::from("Unknown");

    #[derive(serde::Deserialize)]
    struct ExtractedQuestion {
        question_number: Option<serde_json::Value>,
        subject: Option<String>,
        subtopic: Option<String>,
        topics: Option<Vec<String>>,
        marks: Option<i32>,
        content: Option<String>,
        math_snippet: Option<String>,
        is_code: Option<bool>,
        diagram_bbox: Option<Vec<f32>>,
        is_continuation: Option<bool>,
    }

    #[derive(serde::Deserialize)]
    struct OpenAIResult {
        #[serde(default)]
        extracted_questions: Vec<ExtractedQuestion>,
    }

    for page_idx in 0..effective_num_pages {
        let system_prompt = format!(r#"You are a mathematical OCR engine. You MUST output a valid JSON object starting with {{ and ending with }}. Do NOT output a bare array.

RULE 1 (JSON SCHEMA):
The root object MUST be: {{ "extracted_questions": [ ... ] }}. Every question object inside the array MUST include:
- "question_number": integer
- "marks": integer (Find the marks for this specific part, e.g., (4). If the question is multi-part, sum the marks for all parts (a)+(b)+(c). Never return 0.)
- "content": string (Put ALL text, math, and tables here chronologically. If there is a graphical diagram, insert the exact string "[DIAGRAM_PLACEHOLDER]" exactly where the diagram appears in the text).
- "math_snippet": string (LEAVE THIS EMPTY. Put all math directly inside the "content" string where it belongs chronologically).
- "is_code": boolean
- "topics": array of strings (Select relevant topics from the provided list below)
- "diagram_bbox": [x, y, w, h] normalized floats between 0.0 and 1.0 (e.g., 0.12, 0.45). If you return pixel integers (e.g., 86, 229), the code will crash. You must calculate these as a fraction of the total image width/height.
- "is_continuation": boolean

RULE 2 (SUBJECT TAGS):
You are STRICTLY FORBIDDEN from inventing topics. You MUST only use the provided topics list: {:?}.

RULE 3 (PUNCTUATION & FORMATTING):
- Preserve ALL original punctuation (commas, periods, colons). Do not strip them.
- Preserve spatial formatting: If there is a list or lines of working (e.g., Bin 1: ..., Bin 2: ...), separate them with double newlines (\n\n) so they do not become a wall of text.
- Use double newlines (\n\n) to separate sub-parts like (a), (b), (i). 
- If a question has multiple parts, include all parts in ONE JSON object for that question number.
- WRAP ALL MATH: You MUST wrap all numbers, variables, expressions, and mathematical operators in single `$` for inline math or `$$` for display math.
- STRICT LaTeX RULE: NEVER put `$` or `$$` delimiters INSIDE a LaTeX environment like `\begin{{array}}` or `\begin{{aligned}}`. Environments must stand alone inside `$$...$$`.

RULE 4 (TABLES & DIAGRAMS):
- TABLES: ONLY true data tables, matrices, or Simplex tableaus should be formatted as LaTeX block math using the `array` environment. Do NOT draw image bounding boxes around pure data tables.
- DIAGRAMS: You MUST use `diagram_bbox` for all visual and scheduling elements including Gantt charts, scheduling diagrams, timelines, graphs, plots, charts, illustrations, networks, and trees. Do NOT attempt to recreate scheduling diagrams or timelines using LaTeX arrays. Provide the bounding box [x, y, w, h] as relative coordinates (0.0 to 1.0).
- Example: If the image is 1000px wide and the diagram starts at 100px, x should be 0.1.
- If you cannot calculate the relative coordinate, return null.
RULE 5 (EXCLUSIONS & CLEANUP):
- EXCLUDE MARKS: Do NOT extract the mark counts that appear next to question parts (e.g., "(4)", "[3]"). Do NOT extract the footer text that says "Total for Question X is Y marks". Just sum the marks mentally and put the total integer in the "marks" JSON field.
- EXCLUDE BLANK PAGES: If a page just says "BLANK PAGE" or "Turn over", completely ignore it. Do NOT include this text in the content. If the entire page is blank or just says "BLANK PAGE", return an empty array `[]`.

CONTEXT: The previous page was processing Question {}. If the current image does not explicitly start with a new question number, you MUST assume it is a continuation of Question {}, use that number in JSON, and set 'is_continuation': true."#, allowed_topics, current_question_num, current_question_num);

        let b64_data_str = &pdf_pages[page_idx];
        let b64_data = if b64_data_str.starts_with("data:image") {
            b64_data_str.split(',').nth(1).unwrap_or(b64_data_str)
        } else {
            b64_data_str
        };
        
        let req_body = serde_json::json!({
            "model": &model_name,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": [
                    { "type": "image_url", "image_url": { "url": format!("data:image/jpeg;base64,{}", b64_data) } }
                ]}
            ],
            "temperature": 0.1,
            "max_tokens": 16384,
            "response_format": { "type": "json_object" }
        });
        
        let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
        let mut res = client.post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .timeout(std::time::Duration::from_secs(300))
            .json(&req_body)
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;
            
        if res.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
            res = client.post(&url)
                .header("Authorization", format!("Bearer {}", api_key))
                .timeout(std::time::Duration::from_secs(300))
                .json(&req_body)
                .send()
                .await
                .map_err(|e| format!("Network error: {}", e))?;
        }
            
        if !res.status().is_success() {
            let err_text = res.text().await.unwrap_or_default();
            println!("API Error on page {}: {}", page_idx, err_text);
            continue; 
        }
        
        let response_json: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
        let content_str = response_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_string();
            
        let cleaned_json = if content_str.starts_with("```json") {
            content_str.trim_start_matches("```json").trim_end_matches("```").trim()
        } else if content_str.starts_with("```") {
            content_str.trim_start_matches("```").trim_end_matches("```").trim()
        } else {
            &content_str
        };
        
        let parsed_page: OpenAIResult = match serde_json::from_str(cleaned_json) {
            Ok(v) => v,
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("EOF") || err_str.contains("expected value") || err_str.contains("trailing characters") {
                    let mut current = cleaned_json.to_string();
                    let mut attempts = 0;
                    let mut recovered: Option<OpenAIResult> = None;
                    while attempts < 2000 && !current.is_empty() {
                        let closed = auto_close_json(&current);
                        if let Ok(v) = serde_json::from_str::<OpenAIResult>(&closed) {
                            recovered = Some(v);
                            break;
                        }
                        current.pop();
                        attempts += 1;
                    }
                    if let Some(v) = recovered {
                        v
                    } else {
                        println!("Failed to parse JSON on page {}: {} - Raw: {}", page_idx, e, cleaned_json);
                        continue;
                    }
                } else {
                    println!("Failed to parse JSON on page {}: {} - Raw: {}", page_idx, e, cleaned_json);
                    continue;
                }
            }
        };

        for q in parsed_page.extracted_questions {
            let q_num_val = q.question_number.unwrap_or(serde_json::json!(""));
            let mut q_num_str = if q_num_val.is_number() {
                q_num_val.as_i64().unwrap_or(0).to_string()
            } else {
                q_num_val.as_str().unwrap_or("").to_string()
            };
            
            // Strictly sanitize to pure digits
            q_num_str = q_num_str.chars().filter(|c| c.is_ascii_digit()).collect();
            if q_num_str.is_empty() {
                q_num_str = "0".to_string();
            }
            
            let mut q_content = q.content.unwrap_or_default();
            let is_cont = q.is_continuation.unwrap_or(false);
            
            if q_num_str != "0" {
                current_question_num = q_num_str.clone();
            }
            
            let target_q_num = if is_cont {
                current_question_num.clone()
            } else if !q_num_str.is_empty() {
                q_num_str.clone()
            } else {
                current_question_num.clone()
            };

            // Diagram cropping logic
            if let Some(bbox) = &q.diagram_bbox {
                if bbox.len() == 4 {
                    use base64::Engine;
                    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64_data) {
                        if let Ok(mut img) = image::load_from_memory(&bytes) {
                            let (width, height) = img.dimensions();
                            let x = (bbox[0] * width as f32) as u32;
                            let y = (bbox[1] * height as f32) as u32;
                            let w = (bbox[2] * width as f32) as u32;
                            let h = (bbox[3] * height as f32) as u32;

                            let padding: u32 = 40;
                            let safe_x = x.saturating_sub(padding);
                            let safe_y = y.saturating_sub(padding);
                            let safe_width = (w + (x - safe_x) + padding).min(width - safe_x);
                            let safe_height = (h + (y - safe_y) + padding).min(height - safe_y);

                            if safe_width > 0 && safe_height > 0 {
                                let cropped = image::imageops::crop(&mut img, safe_x, safe_y, safe_width, safe_height).to_image();
                                if let Ok(app_data_dir) = app.path().app_data_dir() {
                                    let diagrams_dir = app_data_dir.join("diagrams");
                                    let _ = std::fs::create_dir_all(&diagrams_dir);
                                    let img_uuid = uuid::Uuid::new_v4().to_string();
                                    let img_path = diagrams_dir.join(format!("{}.png", img_uuid));
                                    
                                    if cropped.save(&img_path).is_ok() {
                                        let link = format!("\n\n![Diagram]({})\n\n", img_path.to_string_lossy().replace('\\', "/"));
                                        if q_content.contains("[DIAGRAM_PLACEHOLDER]") {
                                            q_content = q_content.replace("[DIAGRAM_PLACEHOLDER]", &link);
                                        } else {
                                            q_content.push_str(&link);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let existing = aggregated_questions.entry(target_q_num.clone()).or_insert_with(|| {
                let (_, sys_subtopic, sys_is_code) = classifier.classify(&q_content);
                Question {
                    id: uuid::Uuid::new_v4().to_string(),
                    subject: subject.clone(),
                    subtopic: if sys_subtopic == "Unknown" { q.subtopic.clone().unwrap_or_else(|| "Unknown".to_string()) } else { sys_subtopic.to_string() },
                    marks: q.marks.unwrap_or(1),
                    content: String::new(),
                    math_snippet: q.math_snippet.clone().unwrap_or_default(),
                    is_code: if subject == "Computer Science" { q.is_code.unwrap_or(sys_is_code) } else { false },
                    answer_content: None,
                    topics: Some(serde_json::to_string(&q.topics.clone().unwrap_or_default()).unwrap_or_else(|_| "[]".to_string())),
                    paper_name: paper_name.clone(),
                    question_number: q_num_val.as_i64().or_else(|| target_q_num.parse::<i64>().ok()),
                }
            });

            if !existing.content.is_empty() {
                existing.content.push_str("\n\n");
            }
            existing.content.push_str(&q_content);
        }
    }

    let pool = state.db.lock().await;
    let mut final_questions = Vec::new();
    
    for (q_num_str, mut question_obj) in aggregated_questions {
        let q_num_val = q_num_str.parse::<i64>().unwrap_or(0);
        let mut final_id = question_obj.id.clone();
        let mut final_content = question_obj.content.clone();
        let mut was_updated = false;

        if q_num_val > 0 && !paper_name.trim().is_empty() {
            let existing: Option<(String, String)> = sqlx::query_as(
                "SELECT id, content FROM questions WHERE paper_name = ? AND question_number = ? LIMIT 1"
            )
            .bind(&paper_name)
            .bind(q_num_val)
            .fetch_optional(&*pool)
            .await
            .unwrap_or(None);

            if let Some((existing_id, existing_content)) = existing {
                final_id = existing_id.clone();
                final_content = format!("{}\n\n{}", existing_content, final_content);
                
                let new_topics_str = question_obj.topics.clone().unwrap_or_else(|| "[]".to_string());
                let new_marks = question_obj.marks;
                
                sqlx::query("UPDATE questions SET content = ?, topics = CASE WHEN ? != '[]' THEN ? ELSE topics END, marks = CASE WHEN ? > 1 THEN ? ELSE marks END WHERE id = ?")
                    .bind(&final_content)
                    .bind(&new_topics_str)
                    .bind(&new_topics_str)
                    .bind(new_marks)
                    .bind(new_marks)
                    .bind(&existing_id)
                    .execute(&*pool)
                    .await
                    .map_err(|e| format!("DB error updating existing question: {}", e))?;
                was_updated = true;
            }
        }

        if !was_updated {
            sqlx::query(
                r#"
                INSERT INTO questions (id, subject, subtopic, topics, marks, content, math_snippet, is_code, paper_name, question_number)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&final_id)
            .bind(&question_obj.subject)
            .bind(&question_obj.subtopic)
            .bind(&question_obj.topics.clone().unwrap_or_else(|| "[]".to_string()))
            .bind(question_obj.marks)
            .bind(&final_content)
            .bind(&question_obj.math_snippet)
            .bind(question_obj.is_code)
            .bind(&paper_name)
            .bind(q_num_val)
            .execute(&*pool)
            .await
            .map_err(|e| format!("DB error inserting new question: {}", e))?;
        }
        
        question_obj.id = final_id;
        question_obj.content = final_content;
        question_obj.question_number = Some(q_num_val);
        question_obj.topics = Some(question_obj.topics.unwrap_or_else(|| "[]".to_string()));
        final_questions.push(question_obj);
    }

    Ok(final_questions)
}

#[tauri::command]
pub async fn get_paper_names(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let pool = state.db.lock().await;
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT paper_name FROM questions WHERE paper_name IS NOT NULL AND trim(paper_name) != '' ORDER BY paper_name ASC"
    )
    .fetch_all(&*pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(|(name,)| name).collect())
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
    new_topics: Option<String>,
) -> Result<(), String> {
    use tauri::Manager;
    let state = app.state::<AppState>();
    let pool = state.db.lock().await;
    sqlx::query("UPDATE questions SET content = ?, marks = ?, answer_content = ?, topics = COALESCE(?, topics), math_snippet = '' WHERE id = ?")
        .bind(new_content)
        .bind(new_marks)
        .bind(new_answer_content)
        .bind(new_topics)
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
    paper_name: String,
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

    let system_prompt = r#"STEP 1: Look at the page headers. If you see 'General Instructions for Marking', 'General Marking Guidance', 'General Principles for Mechanics Marking', or 'Abbreviations', you MUST immediately return an empty array `[]`. DO NOT invent math. DO NOT extract numbered lists from these pages.

ESCAPE HATCH: Exams contain front covers, formula booklets, and 'General Marking Guidance' pages. If the provided images DO NOT contain any actual exam questions or mark scheme answers, you MUST return a completely empty JSON array: `[]`.
CRITICAL: Do NOT hallucinate, invent, or generate example math questions to fill the array. If the page is just instructions, return `[]`.

STRICT ANTI-HALLUCINATION: You are a transcriber, not a solver. Do NOT invent, solve, or hallucinate generic physics problems (e.g., resolving forces for a block) just because you see physics keywords on a page. If you do not see a clear question number header (e.g., '1(a)') next to mathematical steps, return `[]`.

EXTRACTION GUARDRAIL — MARK SCHEME STRUCTURE REQUIRED: Before extracting any content from a page, confirm it contains explicit mark-scheme structure. Valid indicators are: a question number header in the form '1', '1(a)', '2(b)(i)', etc. appearing in a dedicated question-number column, AND at least one mark label such as 'M1', 'A1', 'B1', 'dM1', or 'ft' in the adjacent marks column. If you do NOT see this structure on the page, do NOT extract anything from it — return `[]`. Numbered bullet points, grammar rules, or abbreviation lists that happen to resemble math are NOT valid mark scheme entries.

IGNORE EXAMINER NOTES: Discard any text explaining mark allocations (e.g., M1, A1, B1, dM1). Extract pure mathematics only.
LIMIT ALTERNATIVES: If a question has multiple alternative methods, extract the main scheme and a MAXIMUM of ONE Alternative Method. Discard the rest.

You are an expert examiner. Extract the final answers and grading logic from this mark scheme.
Return a JSON object with a single key 'answers' containing an array of objects. Do NOT rely on array indices. Each object MUST contain a `question_number` (Integer) read directly from the page, and `answer_markdown` (String).

QAB RULE: Preserve sub-question letters exactly as printed (e.g., (g), (h)). Do not reset them to (a) on a new page.

CRITICAL — QUESTION NUMBER RULE: Look at the main question number column printed in the mark scheme. Use only that explicit number. If a question spans multiple pages or contains alternative methods, group them all under the SAME `question_number` object — do not create a second object with the same number.

CRITICAL FORMATTING RULES:
1. CRITICAL: Your output array length must perfectly match the number of unique main question numbers on the page. All 'Alternative Methods' for a single question MUST be merged into that ONE question's `answer_markdown` string, separated by a Markdown divider (`---`).
2. CRITICAL: NEVER use markdown code blocks (triple backticks) like ```latex. Return the raw text directly.
3. CRITICAL - MULTI-PART QUESTIONS: Group all parts of one question (e.g., 1a, 1b, 1c) into a SINGLE JSON object. Do NOT create separate array items for each sub-part. The `question_number` must be the main integer (e.g., 1, 2) from the exam paper — not a sub-part letter.
4. CRITICAL ANCHORING RULE: Parse the mark scheme by its official printed question numbers. One array item corresponds to exactly ONE unique main question number.
5. HANDLING ALTERNATIVE METHODS — LIMIT TO ONE: If a question or subquestion contains 'Alternative Method', 'Alt 1', 'Alternative Scheme', or similar, you MUST NOT create a new JSON object for it. Instead, append it inside the single `answer_markdown` for that `question_number`, separated by a Markdown horizontal rule (`---`) and a bold header.
   STRICT LIMIT: You MUST extract at most ONE alternative method per question or subquestion. If the mark scheme provides multiple alternatives (e.g., 'Alt 1', 'Alt 2', 'Alternative Method 2'), extract ONLY the FIRST one. Completely ignore and discard any second, third, or subsequent alternative methods — do not include them in the Markdown output at all.
   The goal is a concise solution: the main scheme, and at most ONE alternative. If there is only a main scheme, extract only that.
6. CRITICAL FORMATTING RULE: Structure the answer step-by-step with generous spacing. NEVER cram working out into a single line or a single inline math block.
7. Part labels MUST be bolded on their own line (e.g., **(a)**).
8. EVERY single distinct marking point, step, or line of working MUST be separated by a double newline (`\n\n`).
9. Extract the textual description of the step (e.g., 'Finds the area of $R_1$') as standard text. Only use inline math (`$`) for small variables within these sentences.
10. The main equations, substitutions, and final answers MUST be formatted as display/block math (`$$ equation $$`) so they render centered on their own distinct line.

DECISION MATHS QAB PROTOCOL:
THE REPRINT BAN: If a page reprints a previous question's text or initial tableau for convenience, IGNORE the reprinted text and only extract the new sub-questions.
MATH TABLES: Simplex tableaus and data tables MUST be formatted as LaTeX block math using the `array` environment. Do NOT draw image bounding boxes around them.
GRAPHICAL DIAGRAMS: Activity networks, Gantt charts, and trees MUST be captured via image bounding box coordinates. Do NOT try to build them with LaTeX.
THE EMPTY GRID BAN: NEVER capture empty working grids, blank lines, or unpopulated tracing tables. Return nothing for these.

TEMPLATE TO FOLLOW FOR EACH PART:
**(a)** Main Method working...
\n\n
$$ R_1 = \frac{1}{2}r^2(\theta - \sin\theta) $$
\n\n
---
\n\n
**ALTERNATIVE METHOD 1** (include this block only if the mark scheme provides an alternative — and only this one, never a second)
\n\n
**(a)** Alternative step-by-step working...
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
    struct ExtractedAnswer {
        /// Integer question number parsed directly from the mark scheme page (e.g., 1, 2, 3).
        question_number: Option<i64>,
        /// Formatted LaTeX/Markdown solution steps for this question.
        answer_markdown: Option<String>,
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

        if let Some(start) = content_str.find(|c| c == '{' || c == '[') {
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
                // ── Case 1: model used the escape hatch and returned `[]` ──
                if let Ok(serde_json::Value::Array(ref arr)) = serde_json::from_str(&sanitized) {
                    if arr.is_empty() {
                        continue;
                    }
                }

                // ── Case 2: model returned a bare array `[{...}]` instead
                //            of the expected `{"answers": [{...}]}` wrapper.
                //            Parse directly as Vec<ExtractedAnswer> and wrap it.
                if sanitized.trim_start().starts_with('[') {
                    if let Ok(answers) = serde_json::from_str::<Vec<ExtractedAnswer>>(&sanitized) {
                        OpenAIAnswerResult { answers }
                    } else {
                        // Try auto-close on the bare array (truncated case)
                        let mut current = sanitized.clone();
                        let mut attempts = 0;
                        let mut recovered: Option<OpenAIAnswerResult> = None;
                        while attempts < 2000 && !current.is_empty() {
                            let closed = auto_close_json(&current);
                            if let Ok(answers) = serde_json::from_str::<Vec<ExtractedAnswer>>(&closed) {
                                recovered = Some(OpenAIAnswerResult { answers });
                                break;
                            }
                            current.pop();
                            attempts += 1;
                        }
                        if let Some(p) = recovered {
                            p
                        } else {
                            let auto_closed = auto_close_json(&sanitized);
                            let final_err = serde_json::from_str::<OpenAIAnswerResult>(&auto_closed)
                                .err()
                                .map(|e| e.to_string())
                                .unwrap_or_else(|| "Unknown".to_string());
                            return Err(format!(
                                "The AI model truncated the response. Attempted robust recovery failed: {}\nContent starts with: {}...",
                                final_err,
                                auto_closed.chars().take(50).collect::<String>()
                            ));
                        }
                    }
                } else {
                    // ── Case 3: standard {"answers": [...]} object, but truncated/malformed ──
                    let err_str = e.to_string();
                    if err_str.contains("trailing characters") {
                        if let Some(end) = sanitized.rfind('}') {
                            let chopped = &sanitized[..=end];
                            serde_json::from_str(chopped)
                                .map_err(|e2| format!("Failed to parse OpenAI JSON after trailing chop: {}", e2))?
                        } else {
                            return Err(format!(
                                "Failed to parse OpenAI JSON: {}\nContent starts with: {}...",
                                err_str,
                                sanitized.chars().take(50).collect::<String>()
                            ));
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
                            let final_err = serde_json::from_str::<OpenAIAnswerResult>(&auto_closed)
                                .err()
                                .map(|e| e.to_string())
                                .unwrap_or_else(|| "Unknown".to_string());
                            return Err(format!(
                                "The AI model truncated the response. Attempted robust recovery failed: {}\nContent starts with: {}...",
                                final_err,
                                auto_closed.chars().take(50).collect::<String>()
                            ));
                        }
                    }
                }
            }
        };


        // We do not return an error here if answers is empty — it could be a blank/title page.

        for ans in parsed.answers {
            let ans_content = match ans.answer_markdown {
                Some(ref c) if !c.trim().is_empty() => c.clone(),
                _ => continue, // Skip answers without content
            };

            // Deduplicate across overlapping batches using a content fingerprint.
            let fingerprint: String = ans_content
                .split_whitespace()
                .take(20)
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            if !seen_fingerprints.insert(fingerprint) {
                continue;
            }

            // Only accept entries that carry an explicit question_number; skip otherwise.
            if let Some(q_num) = ans.question_number {
                all_answers.push(ExtractedAnswer {
                    question_number: Some(q_num),
                    answer_markdown: Some(ans_content),
                });
            } else {
                eprintln!("[MergeMark] WARNING: AI returned an answer without a question_number — skipping.");
            }
        }

        thread::sleep(Duration::from_millis(1500));
    }

    if all_answers.is_empty() {
        return Err("The AI model failed to return any answers from the entire document. It may have hit a safety filter, timed out, or encountered an unreadable document.".to_string());
    }

    // Fetch all questions belonging to this paper that still need a mark-scheme answer.
    // Matching on paper_name prevents answers from one paper polluting another paper's questions.
    let paper_name_filter = paper_name.trim().to_string();
    // Fetch ALL questions for this paper, not just unanswered ones.
    // This ensures that if a hallucinated answer was written for an instruction page,
    // the real answer extracted from actual question pages will overwrite it.
    let questions: Vec<Question> = sqlx::query_as(
        "SELECT * FROM questions WHERE paper_name = ? ORDER BY rowid ASC"
    )
    .bind(&paper_name_filter)
    .fetch_all(&*pool)
    .await
    .map_err(|e| format!("DB error: {}", e))?;

    // Build a lookup map: extracted leading integer from question content → Question.
    // For example, content starting with "1. Find..." or "Question 1\n..." → key 1.
    let leading_num_re = regex::Regex::new(r"^(?:Question\s+)?(\d+)").unwrap();
    let mut q_by_number: std::collections::HashMap<i64, &Question> =
        std::collections::HashMap::new();
    for q in &questions {
        if let Some(n) = q.question_number {
            q_by_number.entry(n).or_insert(q);
        } else {
            let trimmed = q.content.trim();
            if let Some(cap) = leading_num_re.captures(trimmed) {
                if let Ok(n) = cap[1].parse::<i64>() {
                    // Only insert the first question for each number to avoid overwriting.
                    q_by_number.entry(n).or_insert(q);
                }
            }
        }
    }

    let mut proposed_mappings: Vec<ProposedMapping> = Vec::new();
    let mut q_id_to_mapping_index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for ans in all_answers {
        let ans_content = match ans.answer_markdown {
            Some(c) if !c.trim().is_empty() => c,
            _ => continue,
        };
        let q_num = match ans.question_number {
            Some(n) => n,
            None => continue,
        };

        if let Some(q) = q_by_number.get(&q_num) {
            // STITCHING LOGIC: If a mapping for this exact question_number and paper_name already exists,
            // fetch its current proposed_answer, append a double newline \n\n, and append the new markdown.
            if let Some(&idx) = q_id_to_mapping_index.get(&q.id) {
                let existing = &proposed_mappings[idx].proposed_answer;
                proposed_mappings[idx].proposed_answer = format!("{}\n\n{}", existing, ans_content);
            } else {
                let idx = proposed_mappings.len();
                q_id_to_mapping_index.insert(q.id.clone(), idx);
                
                // If there's already an answer in the DB from a previous run, stitch it too
                let initial_answer = if let Some(ref db_ans) = q.answer_content {
                    if !db_ans.trim().is_empty() {
                        format!("{}\n\n{}", db_ans, ans_content)
                    } else {
                        ans_content.clone()
                    }
                } else {
                    ans_content.clone()
                };

                proposed_mappings.push(ProposedMapping {
                    question_id: q.id.clone(),
                    raw_content: q.content.clone(),
                    proposed_answer: initial_answer,
                    paper_name: q.paper_name.clone(),
                });
            }
        } else {
            eprintln!(
                "[MergeMark] WARNING: No DB question found matching question_number {} — skipping.",
                q_num
            );
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

// ── Hybrid Pipeline ──────────────────────────────────────────────────────────

struct BBoxOutput {
    pub text_data: Vec<(String, f64, f64)>,
    pub pdf_page_height: f64,
}

impl pdf_extract::OutputDev for BBoxOutput {
    fn begin_page(&mut self, _page_num: u32, media_box: &pdf_extract::MediaBox, _art_box: Option<(f64, f64, f64, f64)>) -> Result<(), pdf_extract::OutputError> {
        self.pdf_page_height = media_box.ury - media_box.lly;
        Ok(())
    }
    fn end_page(&mut self) -> Result<(), pdf_extract::OutputError> {
        Ok(())
    }
    fn output_character(&mut self, trm: &pdf_extract::Transform, _width: f64, _spacing: f64, _font_size: f64, char: &str) -> Result<(), pdf_extract::OutputError> {
        self.text_data.push((char.to_string(), trm.m31, trm.m32));
        Ok(())
    }
    fn begin_word(&mut self) -> Result<(), pdf_extract::OutputError> { Ok(()) }
    fn end_word(&mut self) -> Result<(), pdf_extract::OutputError> { Ok(()) }
    fn end_line(&mut self) -> Result<(), pdf_extract::OutputError> { Ok(()) }
}

#[derive(Debug, serde::Serialize)]
pub struct QuestionSlice {
    pub question_number: u32,
    pub y_start: f64,
    pub y_end: f64,
}

