use crate::llm::{LlmConfig, ReqwestLlm};
use crate::pipeline::{
    self, AnswerDraft, BuiltQuestion, ImportReport, PageInput, PipelineConfig, Progress,
};
use crate::AppState;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use std::time::Instant;
use tauri::{Emitter, Manager, State};

static RE_LINES: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"_+|-+").unwrap());
static RE_ANS_LINES: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?m)^\s*[1-6]\s*$").unwrap());
static RE_AQA_NUM: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"[0O]\s*(\d)\s*\.\s*(\d)").unwrap());
static RE_COLLAPSE_NEWLINES: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\n{3,}").unwrap());

// Static regexes for SubjectClassifier
static RE_CLASSIFIER_MARKS: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?i)\[\s*(\d+)\s*marks?\s*\]|\(\s*(\d+)\s*\)").unwrap());
static RE_CLASSIFIER_QSPLIT: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?m)(?:^|\n)(?:Question\s+\d+|Q\.?\s*\d+|\d{1,2}[.)]\s)").unwrap());
static RE_CLASSIFIER_MATH: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?s)\$\$?.+?\$\$?|\\\[.+?\\\]|\\\(.+?\\\)").unwrap());



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
    #[sqlx(default)]
    pub module: Option<String>,
    #[sqlx(default)]
    pub needs_review: bool,
    #[sqlx(default)]
    pub answer_stale: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposedMapping {
    pub question_id: String,
    pub raw_content: String,
    pub proposed_answer: String,
    pub paper_name: String,
}

// ── Billing integration helper ──────────────────────────────────────────────────

async fn resolve_llm_client<'a>(
    state: &State<'a, AppState>,
    frontend_model: String,
) -> Result<(crate::billing::BillingRoute, ReqwestLlm), crate::billing::BillingError> {
    let pool = state.db.lock().await;
    let free_uploads_used = crate::db::get_free_uploads_used(&pool)
        .await
        .map_err(|e| crate::billing::BillingError::network(&format!("DB read failed: {e}")))?;
    let byok_key = crate::db::get_byok_api_key(&pool)
        .await
        .map_err(|e| crate::billing::BillingError::network(&format!("DB read failed: {e}")))?;
    let byok_base = crate::db::get_byok_base_url(&pool)
        .await
        .map_err(|e| crate::billing::BillingError::network(&format!("DB read failed: {e}")))?
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    drop(pool);

    let route = crate::billing::pick_route(free_uploads_used, byok_key.is_some());
    let config = match &route {
        crate::billing::BillingRoute::FreeTier { .. } => {
            let key = crate::billing::openrouter_api_key();
            if key == "dev-openrouter-key-not-set" {
                return Err(crate::billing::BillingError::network("The built-in Free Tier is unavailable on this installation. Please enter your own API Key in Settings."));
            }
            LlmConfig {
                base_url: crate::billing::OPENROUTER_API_URL.to_string(),
                api_key: key.to_string(),
                model: crate::billing::OPENROUTER_MODEL.to_string(),
                timeout: crate::billing::REQUEST_TIMEOUT,
            }
        }
        crate::billing::BillingRoute::Byok => LlmConfig {
            base_url: byok_base,
            api_key: byok_key.unwrap(),
            model: frontend_model,
            timeout: crate::billing::REQUEST_TIMEOUT,
        },
        crate::billing::BillingRoute::NeedsByok => {
            return Err(crate::billing::BillingError::needs_byok(free_uploads_used));
        }
    };
    Ok((route, ReqwestLlm::new(config)))
}

// ── Progress bridge: pipeline stages → frontend `import-progress` events ──────

struct TauriProgress {
    app: tauri::AppHandle,
}

impl Progress for TauriProgress {
    fn stage(&self, message: &str) {
        let _ = self.app.emit(
            "import-progress",
            serde_json::json!({ "page": 0, "total": 0, "message": message }),
        );
    }
}

// ── Helper: shared question-classification + DB-insert logic (legacy path) ────

/// Keyword tables used for TF-IDF-style subject scoring (legacy text imports).
struct SubjectClassifier;

impl SubjectClassifier {
    fn new() -> Self {
        Self
    }

    fn classify(&self, text: &str) -> (&'static str, &'static str, bool) {
        let lower = text.to_lowercase();

        let cs_keywords: &[&str] = &[
            "array",
            "pointer",
            "recursion",
            "binary tree",
            "linked list",
            "stack",
            "queue",
            "hash table",
            "algorithm",
            "big-o",
            "o(n)",
            "complexity",
            "sql",
            "database",
            "sorting",
            "searching",
            "compiler",
            "interpreter",
            "cpu",
            "register",
            "cache",
            "encryption",
            "network",
            "protocol",
            "tcp",
            "ip address",
            "subroutine",
            "function call",
            "object-oriented",
            "class",
            "inheritance",
            "polymorphism",
            "binary",
            "hexadecimal",
            "boolean",
            "pseudocode",
            "flowchart",
            "assembly",
        ];

        let math_keywords: &[&str] = &[
            "matrix",
            "determinant",
            "eigenvalue",
            "eigenvector",
            "differential equation",
            "integration",
            "differentiation",
            "calculus",
            "gradient",
            "vector",
            "scalar",
            "proof",
            "induction",
            "complex number",
            "argand",
            "polynomial",
            "binomial",
            "series",
            "sequence",
            "limit",
            "convergence",
            "trigonometry",
            "sine",
            "cosine",
            "tangent",
            "logarithm",
            "exponent",
            "modulus",
            "inequality",
            "quadratic",
        ];

        let phys_keywords: &[&str] = &[
            "kinetic energy",
            "potential energy",
            "momentum",
            "velocity",
            "acceleration",
            "force",
            "newton",
            "wavelength",
            "frequency",
            "magnetic field",
            "electric field",
            "voltage",
            "current",
            "resistance",
            "ohm",
            "capacitor",
            "inductor",
            "photon",
            "quantum",
            "nuclear",
            "radioactive",
            "half-life",
            "thermal",
            "entropy",
            "pressure",
            "density",
            "refraction",
            "diffraction",
        ];

        let chem_keywords: &[&str] = &[
            "mole",
            "molarity",
            "titration",
            "oxidation",
            "reduction",
            "electrode",
            "catalyst",
            "reaction rate",
            "equilibrium",
            "enthalpy",
            "entropy",
            "gibbs",
            "bond energy",
            "lattice",
            "atomic number",
            "electron configuration",
            "periodic table",
            "organic",
            "hydrocarbon",
            "ester",
            "polymer",
        ];

        let bio_keywords: &[&str] = &[
            "cell membrane",
            "mitosis",
            "meiosis",
            "dna",
            "rna",
            "protein synthesis",
            "enzyme",
            "atp",
            "photosynthesis",
            "respiration",
            "ecosystem",
            "natural selection",
            "evolution",
            "chromosome",
            "allele",
            "genotype",
            "phenotype",
            "nervous system",
            "homeostasis",
            "osmosis",
        ];

        let score =
            |kws: &[&str]| -> usize { kws.iter().filter(|&&kw| lower.contains(kw)).count() };

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
        /*
        if cs == max {
            return ("Computer Science", "Algorithms & Data Structures", true);
        }
        */
        if math == max {
            let is_gcse = lower.contains("gcse")
                || lower.contains("level 2 certificate")
                || lower.contains("secondary education");
            let is_further = lower.contains("further");
            if is_gcse && is_further {
                return ("GCSE Further Mathematics (AQA)", "Algebra", false);
            } else if is_gcse {
                return ("GCSE Mathematics (Edexcel)", "Algebra", false);
            } else if is_further {
                return (
                    "A Level Further Mathematics (Edexcel)",
                    "Pure Mathematics",
                    false,
                );
            } else {
                return ("A Level Mathematics (Edexcel)", "Pure", false);
            }
        }
        /*
        if phys == max {
            return ("Physics", "Mechanics & Fields", false);
        }
        */
        /*
        if chem == max {
            return ("Chemistry", "Physical Chemistry", false);
        }
        ("Biology", "Cell Biology", false)
        */
        ("General", "Imported", false)
    }

    fn extract_marks(&self, text: &str) -> i32 {
        if let Some(cap) = RE_CLASSIFIER_MARKS.captures_iter(text).last() {
            if let Some(m) = cap.get(1).or_else(|| cap.get(2)) {
                if let Ok(v) = m.as_str().parse::<i32>() {
                    return v.clamp(1, 25);
                }
            }
        }
        1
    }

    fn extract_math(&self, text: &str) -> String {
        RE_CLASSIFIER_MATH
            .find(text)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default()
    }

    fn slice_questions<'a>(&self, text: &'a str) -> Vec<&'a str> {
        let splits: Vec<_> = RE_CLASSIFIER_QSPLIT
            .split(text)
            .map(str::trim)
            .filter(|s| s.len() > 20)
            .collect();

        if splits.len() > 1 {
            return splits;
        }

        let fallback: Vec<_> = text
            .split("---")
            .map(str::trim)
            .filter(|s| s.len() > 20)
            .collect();

        if !fallback.is_empty() {
            return fallback;
        }

        if text.trim().len() > 20 {
            vec![text.trim()]
        } else {
            vec![]
        }
    }
}

// ── Shared DB insert logic (legacy text path) ────────────────────────────────

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
        let content = chunk.trim().to_string();

        sqlx::query(
            r#"
            INSERT INTO questions (id, subject, subtopic, topics, marks, content, math_snippet, is_code, module)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
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
        .bind(Option::<String>::None)
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
    let questions = sqlx::query_as::<_, Question>(
        r#"
        SELECT 
            id,
            COALESCE(subject, 'Mathematics') AS subject,
            COALESCE(subtopic, '') AS subtopic,
            COALESCE(marks, 0) AS marks,
            COALESCE(content, '') AS content,
            COALESCE(math_snippet, '') AS math_snippet,
            COALESCE(is_code, 0) AS is_code,
            answer_content,
            topics,
            COALESCE(paper_name, '') AS paper_name,
            question_number,
            module,
            COALESCE(needs_review, 0) AS needs_review,
            COALESCE(answer_stale, 0) AS answer_stale
        FROM questions 
        ORDER BY rowid DESC
        "#
    )
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
        INSERT INTO questions (id, subject, subtopic, topics, marks, content, math_snippet, is_code, module, answer_content, paper_name, question_number)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
    .bind(question.module)
    .bind(question.answer_content)
    .bind(question.paper_name)
    .bind(question.question_number)
    .execute(&*pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Permanently removes a single question from the database by its UUID.
/// Permanently removes a single question from the database by its UUID.
#[tauri::command]
pub async fn delete_question(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let pool = state.db.lock().await;

    // Fetch paper_name before deletion to check if this was the last question
    let paper_info: Option<(Option<String>,)> =
        sqlx::query_as("SELECT paper_name FROM questions WHERE id = ?")
            .bind(&id)
            .fetch_optional(&*pool)
            .await
            .unwrap_or(None);

    sqlx::query("DELETE FROM questions WHERE id = ?")
        .bind(&id)
        .execute(&*pool)
        .await
        .map_err(|e| e.to_string())?;

    // If this paper has no questions remaining, clean up its import cost logs
    if let Some((Some(paper_name),)) = paper_info {
        let trimmed = paper_name.trim();
        if !trimmed.is_empty() {
            let (remaining,): (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM questions WHERE paper_name = ?",
            )
            .bind(trimmed)
            .fetch_one(&*pool)
            .await
            .unwrap_or((0,));

            if remaining == 0 {
                let _ = sqlx::query(
                    "DELETE FROM import_cost_logs WHERE paper_name = ? OR paper_name = ?",
                )
                .bind(trimmed)
                .bind(format!("MS:{}", trimmed))
                .execute(&*pool)
                .await;
            }
        }
    }

    // If the entire repository is now empty, wipe all remaining import logs
    let (total_q,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM questions")
        .fetch_one(&*pool)
        .await
        .unwrap_or((0,));
    if total_q == 0 {
        let _ = sqlx::query("DELETE FROM import_cost_logs")
            .execute(&*pool)
            .await;
    }

    Ok(())
}

/// Import from a plain-text file (legacy "---"-delimited format or numbered questions).
#[tauri::command]
pub async fn import_questions(app: tauri::AppHandle, file_path: String) -> Result<usize, String> {
    let content =
        std::fs::read_to_string(&file_path).map_err(|e| format!("Failed to read file: {}", e))?;

    let state: State<'_, AppState> = app.state();
    let pool = state.db.lock().await;

    let classifier = SubjectClassifier::new();
    insert_questions_from_text(&*pool, &content, &classifier).await
}

/// Parse a PDF (or plain-text) past paper with heuristic regex slicing.
/// Returns the total number of questions inserted.
#[tauri::command]
pub async fn parse_pdf(app: tauri::AppHandle, file_path: String) -> Result<usize, String> {
    let path_clone = file_path.clone();
    let raw_text = tokio::task::spawn_blocking(move || -> Result<String, String> {
        let lower = path_clone.to_lowercase();
        if lower.ends_with(".pdf") {
            pdf_extract::extract_text_encrypted(&path_clone, "")
                .map_err(|e| format!("PDF extraction failed: {}", e))
        } else {
            std::fs::read_to_string(&path_clone).map_err(|e| format!("Failed to read file: {}", e))
        }
    })
    .await
    .map_err(|e| format!("Thread-pool error: {}", e))??;

    if raw_text.trim().is_empty() {
        return Err("No text could be extracted from this file. \
             It may be a scanned/image-only PDF."
            .into());
    }

    let cleaned = raw_text
        .lines()
        .map(|l| l.trim())
        .collect::<Vec<_>>()
        .join("\n");
    let cleaned = regex::Regex::new(r"\n{3,}")
        .unwrap()
        .replace_all(&cleaned, "\n\n");

    let classifier = SubjectClassifier::new();

    let state: State<'_, AppState> = app.state();
    let pool = state.db.lock().await;

    insert_questions_from_text(&*pool, &cleaned, &classifier).await
}

#[derive(Debug, Clone, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorksheetCompileOptions {
    pub file_name: Option<String>,
    pub exam_title: Option<String>,
    pub subject: Option<String>,
    pub school_name: Option<String>,
    pub time_allowed_mins: Option<u32>,
    pub instructions: Option<String>,
    pub include_cover_page: Option<bool>,
    pub answer_layout: Option<String>, // "compact" | "lined"
}

#[tauri::command]
pub async fn export_worksheet_markdown(
    app: tauri::AppHandle,
    question_ids: Vec<String>,
    options: Option<WorksheetCompileOptions>,
) -> Result<String, String> {
    let state: State<'_, AppState> = app.state();
    let pool = state.db.lock().await;

    let mut markdown = String::new();
    let mut total_marks = 0;

    for (i, id) in question_ids.iter().enumerate() {
        let question: Option<Question> = sqlx::query_as("SELECT * FROM questions WHERE id = ?")
            .bind(id)
            .fetch_optional(&*pool)
            .await
            .map_err(|e| format!("Database error: {}", e))?;

        if let Some(q) = question {
            total_marks += q.marks;
            markdown.push_str(&format!("### Question {}\n\n", i + 1));
            markdown.push_str(&q.content);
            markdown.push_str(&format!("\n\n**[{} marks]**\n\n---\n\n", q.marks));
        }
    }

    let opts = options.unwrap_or_default();
    let title = opts
        .exam_title
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("Worksheet");
    let subject = opts.subject.as_deref().unwrap_or("");
    let time_mins = opts
        .time_allowed_mins
        .unwrap_or_else(|| (total_marks as f32 * 1.2).round() as u32);

    let mut header = format!("# {}\n\n", title);
    if !subject.trim().is_empty() {
        header.push_str(&format!("**Subject:** {} | ", subject));
    }
    header.push_str(&format!(
        "**Total Marks:** {} | **Time Allowed:** {} mins\n\n---\n\n",
        total_marks, time_mins
    ));
    header.push_str(&markdown);

    Ok(header)
}

#[tauri::command]
pub async fn compile_worksheet(
    app: tauri::AppHandle,
    question_ids: Vec<String>,
    file_name: String,
    options: Option<WorksheetCompileOptions>,
) -> Result<Vec<String>, String> {
    let state: State<'_, AppState> = app.state();
    let pool = state.db.lock().await;

    let mut fetched_questions = Vec::new();
    let mut total_marks = 0;
    for id in &question_ids {
        let q: Option<Question> = sqlx::query_as("SELECT * FROM questions WHERE id = ?")
            .bind(id)
            .fetch_optional(&*pool)
            .await
            .map_err(|e| e.to_string())?;
        if let Some(question) = q {
            total_marks += question.marks;
            fetched_questions.push(question);
        }
    }

    let opts = options.unwrap_or_default();
    let raw_title = opts
        .exam_title
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("MergeMark Practice Paper");
    let exam_title = crate::validate::sanitize_for_latex(raw_title);
    let subject = crate::validate::sanitize_for_latex(opts.subject.as_deref().unwrap_or(""));
    let school_name = crate::validate::sanitize_for_latex(opts.school_name.as_deref().unwrap_or(""));
    let time_allowed = opts
        .time_allowed_mins
        .unwrap_or_else(|| (total_marks as f32 * 1.2).round() as u32);
    let include_cover = opts.include_cover_page.unwrap_or(false);
    let is_lined = opts.answer_layout.as_deref() == Some("lined");
    let instructions = opts
        .instructions
        .as_deref()
        .unwrap_or("Answer all questions in the spaces provided.\nShow all necessary working out clearly.\nCalculators may be used where appropriate.");

    let header_title = if !school_name.trim().is_empty() {
        format!("{} -- {}", school_name, exam_title)
    } else {
        exam_title.clone()
    };

    let mut latex = String::new();
    latex.push_str("\\documentclass[11pt,a4paper]{article}\n");
    latex.push_str("\\usepackage[T1]{fontenc}\n");
    latex.push_str("\\usepackage[top=2.0cm, bottom=2.2cm, left=2.0cm, right=2.0cm, headheight=24pt, headsep=0.7cm, footskip=1.1cm]{geometry}\n");
    latex.push_str(
        "\\usepackage{amsmath, amssymb, mathtools, bm, microtype, graphicx, xcolor, mdframed, parskip, enumitem, tabularx, lastpage, needspace, array}\n",
    );
    latex.push_str("\\usepackage[scaled=0.92]{helvet}\n");
    latex.push_str("\\renewcommand{\\familydefault}{\\sfdefault}\n");
    latex.push_str("\\usepackage{fancyhdr}\n");

    // Professional color definitions
    latex.push_str("\\definecolor{branddark}{HTML}{0F172A}\n");
    latex.push_str("\\definecolor{brandgray}{HTML}{475569}\n");
    latex.push_str("\\definecolor{lightborder}{HTML}{CBD5E1}\n");
    latex.push_str("\\definecolor{cardbg}{HTML}{F8FAFC}\n");
    latex.push_str("\\definecolor{ruledline}{HTML}{A0A0A0}\n");

    // Ruled lines macro using native vertical leaders (centered across textwidth on all pages)
    latex.push_str("\\newcommand{\\fillanswerspace}{%\n");
    latex.push_str("  \\par\\penalty-100\\vspace{0.35cm}%\n");
    latex.push_str("  \\leaders\\vbox to 6.5mm{%\n");
    latex.push_str("    \\vfill\n");
    latex.push_str("    \\centerline{\\makebox[\\textwidth][c]{{\\color{ruledline}\\leaders\\hrule height 0.5pt\\hskip\\textwidth}}}%\n");
    latex.push_str("  }\\vfill\n");
    latex.push_str("  \\vspace{0.25cm}%\n");
    latex.push_str("}\n");
    latex.push_str("\\newcommand{\\examrule}{\\noindent\\makebox[\\linewidth]{\\color{ruledline}\\rule{\\linewidth}{0.5pt}}\\vspace{0.65cm}\\par\\nointerlineskip}\n");

    // Digit boxes for Centre and Candidate Number (authentic UK exam board style)
    latex.push_str("\\newcommand{\\digitbox}{\\framebox(13,15){}}\n");
    latex.push_str("\\newcommand{\\centreboxes}{\\digitbox\\digitbox\\digitbox\\digitbox\\digitbox}\n");
    latex.push_str("\\newcommand{\\candidateboxes}{\\digitbox\\digitbox\\digitbox\\digitbox}\n");

    latex.push_str("\\pagestyle{fancy}\n");
    latex.push_str("\\fancyhf{}\n");
    latex.push_str(&format!(
        "\\lhead{{\\footnotesize\\textcolor{{brandgray}}{{\\textbf{{{}}}}}}}\n",
        header_title
    ));
    latex.push_str("\\rhead{\\footnotesize\\textcolor{brandgray}{\\textbf{Page \\thepage\\ of \\pageref{LastPage}}}}\n");
    latex.push_str("\\renewcommand{\\headrulewidth}{0.5pt}\n");
    latex.push_str("\\renewcommand{\\headrule}{\\hbox to\\headwidth{\\color{lightborder}\\leaders\\hrule height \\headrulewidth\\hfill}}\n");
    latex.push_str("\\lfoot{\\scriptsize\\textcolor{gray!60}{\\textsf{MERGEMARK ASSESSMENTS}}}\n");
    latex.push_str("\\rfoot{\\footnotesize\\textbf{Turn over}}\n");
    latex.push_str("\\setlength{\\parskip}{0pt}\n");
    latex.push_str("\\setlength{\\parindent}{0pt}\n");
    latex.push_str("\\widowpenalty=10000\n\\clubpenalty=10000\n\\displaywidowpenalty=10000\n\\interfootnotelinepenalty=10000\n");
    latex.push_str("\\setlist{topsep=0.35cm, parsep=0.2cm, itemsep=0.5cm, leftmargin=0.8cm, labelsep=0.4cm}\n");
    latex.push_str("\\setlist[enumerate,1]{label=\\textbf{\\arabic*.}, leftmargin=*}\n");
    latex.push_str("\\setlist[itemize]{label=\\textbullet, leftmargin=1.4em, itemsep=0.25em, topsep=0.2em}\n");

    if !include_cover {
        latex.push_str("\\fancypagestyle{firstpage}{%\n");
        latex.push_str("  \\fancyhf{}%\n");
        latex.push_str("  \\rhead{\\footnotesize\\textcolor{brandgray}{\\textbf{Page \\thepage\\ of \\pageref{LastPage}}}}%\n");
        latex.push_str("  \\lfoot{\\scriptsize\\textcolor{gray!60}{\\textsf{MERGEMARK ASSESSMENTS}}}%\n");
        latex.push_str("  \\rfoot{\\footnotesize\\textbf{Turn over}}%\n");
        latex.push_str("  \\renewcommand{\\headrulewidth}{0pt}%\n");
        latex.push_str("  \\renewcommand{\\footrulewidth}{0pt}%\n");
        latex.push_str("}%\n");
    }

    latex.push_str("\\begin{document}\n");

    if include_cover {
        latex.push_str("\\begin{titlepage}\n\\thispagestyle{empty}\n\n");
        
        // Top Header Bar
        latex.push_str("\\noindent\n\\begin{minipage}[c]{0.65\\linewidth}\n\\raggedright\n");
        if !school_name.trim().is_empty() {
            latex.push_str(&format!("{{\\large\\textbf{{\\textcolor{{branddark}}{{{}}}}}}}\\\\[0.1cm]\n", school_name));
        }
        latex.push_str("{\\scriptsize\\textbf{\\textcolor{brandgray}{\\textsf{OFFICIAL EXAMINATION PAPER}}}}\n");
        latex.push_str("\\end{minipage}%\n\\begin{minipage}[c]{0.35\\linewidth}\n\\raggedleft\n");
        latex.push_str("\\includegraphics[height=1.2cm]{mergemark_logo.png}\n");
        latex.push_str("\\end{minipage}\n\n");

        latex.push_str("\\vspace{0.35cm}\n\\noindent{\\color{branddark}\\rule{\\linewidth}{1.5pt}}\n\\vspace{0.3cm}\n\n");

        // Main Title & Subject
        latex.push_str("\\begin{center}\n");
        if !subject.trim().is_empty() {
            latex.push_str(&format!("{{\\Large\\textbf{{\\textcolor{{brandgray}}{{{}}}}}}}\\\\[0.25cm]\n", subject));
        }
        latex.push_str(&format!("{{\\Huge\\textbf{{\\textcolor{{branddark}}{{{}}}}}}}\\\\[0.35cm]\n", exam_title));
        latex.push_str("\\end{center}\n\n");

        // Candidate Identification Box
        latex.push_str("\\noindent\n\\begin{mdframed}[linewidth=0.8pt, linecolor=branddark, backgroundcolor=white, roundcorner=2pt, innertopmargin=10pt, innerbottommargin=10pt, innerleftmargin=12pt, innerrightmargin=12pt]\n");
        latex.push_str("{\\large\\textbf{Candidate Details}}\\\\[0.35cm]\n");
        latex.push_str("\\begin{tabularx}{\\linewidth}{@{}l X l X@{}}\n");
        latex.push_str("\\textbf{Surname:} & \\makebox[\\linewidth]{\\hrulefill} & \\textbf{Other Names:} & \\makebox[\\linewidth]{\\hrulefill} \\\\[0.45cm]\n");
        latex.push_str("\\end{tabularx}\n\\vspace{0.1cm}\n");
        latex.push_str("\\begin{tabularx}{\\linewidth}{@{}l c @{\\hspace{1.5cm}} l c X@{}}\n");
        latex.push_str("\\textbf{Centre Number:} & \\centreboxes & \\textbf{Candidate Number:} & \\candidateboxes & \\\\\n");
        latex.push_str("\\end{tabularx}\n\\end{mdframed}\n\n\\vspace{0.35cm}\n\n");

        // Paper Info Strip
        latex.push_str("\\noindent\n\\begin{mdframed}[linewidth=0.6pt, linecolor=lightborder, backgroundcolor=cardbg, roundcorner=2pt, innertopmargin=6pt, innerbottommargin=6pt, innerleftmargin=10pt, innerrightmargin=10pt]\n");
        latex.push_str(&format!(
            "\\begin{{tabularx}}{{\\linewidth}}{{@{{}}Xcr@{{}}}}\n\\textbf{{Time Allowed:}} {} minutes & \\textbf{{Total Marks:}} {} marks & \\textbf{{Calculators:}} Permitted\n\\end{{tabularx}}\n",
            time_allowed, total_marks
        ));
        latex.push_str("\\end{mdframed}\n\n\\vspace{0.35cm}\n\n");

        // Instructions Card
        latex.push_str("\\noindent\n\\begin{mdframed}[linewidth=0.6pt, linecolor=lightborder, backgroundcolor=white, roundcorner=2pt, innertopmargin=8pt, innerbottommargin=8pt, innerleftmargin=10pt, innerrightmargin=10pt]\n");
        latex.push_str("\\textbf{\\normalsize Instructions to Candidates}\n\\begin{itemize}\n");
        for line in instructions.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                let sanitized_line = crate::validate::sanitize_for_latex(trimmed);
                latex.push_str(&format!("\\item\\relax {}\n", sanitized_line));
            }
        }
        latex.push_str("\\end{itemize}\n\\vspace{0.2cm}\n");
        latex.push_str("\\textbf{\\normalsize Information for Candidates}\n\\begin{itemize}\n");
        latex.push_str(&format!("\\item The total mark for this paper is \\textbf{{{}}}.\n", total_marks));
        latex.push_str("\\item The marks for each question are shown in brackets --- use this as a guide as to how much time to spend on each question.\n");
        latex.push_str("\\end{itemize}\n\\end{mdframed}\n\n\\vfill\n\n");

        // Examiner's Score Table (Grid)
        latex.push_str("\\noindent\n\\begin{center}\n\\footnotesize\n\\textbf{FOR EXAMINER'S USE ONLY}\\\\[0.15cm]\n");
        let mut examiner_table = String::from("\\begin{tabular}{|p{2.2cm}|");
        for _ in 0..fetched_questions.len() {
            examiner_table.push_str(">{\\centering\\arraybackslash}p{0.85cm}|");
        }
        examiner_table.push_str(">{\\centering\\arraybackslash}p{1.3cm}|}\n\\hline\n\\textbf{Question} & ");
        for (idx, _) in fetched_questions.iter().enumerate() {
            examiner_table.push_str(&format!("\\textbf{{{}}} & ", idx + 1));
        }
        examiner_table.push_str("\\textbf{Total} \\\\\n\\hline\n\\textbf{Max Mark} & ");
        for q in &fetched_questions {
            examiner_table.push_str(&format!("{} & ", q.marks));
        }
        examiner_table.push_str(&format!("{} \\\\\n\\hline\n\\textbf{{Mark}} & ", total_marks));
        for _ in &fetched_questions {
            examiner_table.push_str(" & ");
        }
        examiner_table.push_str(" \\\\\n\\hline\n\\end{tabular}\n");
        latex.push_str(&examiner_table);
        latex.push_str("\\end{center}\n");

        latex.push_str("\\end{titlepage}\n\\newpage\n");
    } else {
        latex.push_str("\\thispagestyle{firstpage}\n\n");
        latex.push_str("\\noindent\n\\begin{minipage}[c]{0.12\\linewidth}\n");
        latex.push_str("\\includegraphics[height=1.1cm]{mergemark_logo.png}\n");
        latex.push_str("\\end{minipage}%\n\\begin{minipage}[c]{0.88\\linewidth}\n\\raggedright\n");
        if !school_name.trim().is_empty() {
            latex.push_str(&format!("{{\\scriptsize\\textbf{{\\textcolor{{brandgray}}{{\\textsf{{{}}}}}}}\\\\[0.05cm]}}\n", school_name));
        }
        latex.push_str(&format!("{{\\Large\\textbf{{\\textcolor{{branddark}}{{{}}}}}}}", exam_title));
        if !subject.trim().is_empty() {
            latex.push_str(&format!(" \\quad {{\\normalsize\\textbf{{\\textcolor{{brandgray}}{{| \\\\ {} }}}}}}", subject));
        }
        latex.push_str("\n\\end{minipage}\n\n\\vspace{0.25cm}\n\\noindent{\\color{lightborder}\\rule{\\linewidth}{0.6pt}}\n\\vspace{0.2cm}\n\n");

        // Candidate strip with digit boxes
        latex.push_str("\\noindent\n\\begin{mdframed}[linewidth=0.6pt, linecolor=branddark, backgroundcolor=white, roundcorner=2pt, innertopmargin=6pt, innerbottommargin=6pt, innerleftmargin=8pt, innerrightmargin=8pt]\n");
        latex.push_str("\\begin{tabularx}{\\linewidth}{@{}p{0.42\\linewidth}@{\\hspace{0.02\\linewidth}}p{0.28\\linewidth}@{\\hspace{0.02\\linewidth}}p{0.26\\linewidth}@{}}\n");
        latex.push_str("\\textbf{Candidate Name:} \\makebox[1.4in]{\\hrulefill} & \\textbf{Centre No:} \\ \\centreboxes & \\textbf{Candidate No:} \\ \\candidateboxes \\\\\n");
        latex.push_str("\\end{tabularx}\n\\end{mdframed}\n\n\\vspace{0.15cm}\n\n");

        // Stats pill
        latex.push_str("\\noindent\n\\begin{mdframed}[linewidth=0.5pt, linecolor=lightborder, backgroundcolor=cardbg, roundcorner=2pt, innertopmargin=4pt, innerbottommargin=4pt, innerleftmargin=8pt, innerrightmargin=8pt]\n");
        latex.push_str(&format!(
            "\\textbf{{Total Marks:}} {} marks \\hfill \\textbf{{Time Allowed:}} {} mins \\hfill \\textbf{{Calculators:}} Permitted\n",
            total_marks, time_allowed
        ));
        latex.push_str("\\end{mdframed}\n\n\\vspace{0.4cm}\n\n");
    }

    latex.push_str("\\begin{enumerate}\n");

    let mut answer_latex = String::new();
    answer_latex.push_str("\\documentclass[11pt,a4paper]{article}\n");
    answer_latex.push_str("\\usepackage[T1]{fontenc}\n");
    answer_latex.push_str("\\usepackage[top=2.0cm, bottom=2.2cm, left=2.0cm, right=2.0cm, headheight=24pt, headsep=0.7cm, footskip=1.1cm]{geometry}\n");
    answer_latex.push_str(
        "\\usepackage{amsmath, amssymb, mathtools, bm, microtype, graphicx, xcolor, mdframed, parskip, enumitem, lastpage, needspace}\n",
    );
    answer_latex.push_str("\\usepackage[scaled=0.92]{helvet}\n");
    answer_latex.push_str("\\renewcommand{\\familydefault}{\\sfdefault}\n");
    answer_latex.push_str("\\usepackage{fancyhdr}\n");

    answer_latex.push_str("\\definecolor{branddark}{HTML}{0F172A}\n");
    answer_latex.push_str("\\definecolor{brandgray}{HTML}{475569}\n");
    answer_latex.push_str("\\definecolor{lightborder}{HTML}{CBD5E1}\n");
    answer_latex.push_str("\\definecolor{cardbg}{HTML}{F8FAFC}\n");

    answer_latex.push_str("\\pagestyle{fancy}\n");
    answer_latex.push_str("\\fancyhf{}\n");
    answer_latex.push_str(&format!(
        "\\lhead{{\\footnotesize\\textcolor{{brandgray}}{{\\textbf{{{} -- Mark Scheme & Solutions}}}}}}\n",
        header_title
    ));
    answer_latex.push_str("\\rhead{\\footnotesize\\textcolor{brandgray}{\\textbf{Page \\thepage\\ of \\pageref{LastPage}}}}\n");
    answer_latex.push_str("\\renewcommand{\\headrulewidth}{0.5pt}\n");
    answer_latex.push_str("\\renewcommand{\\headrule}{\\hbox to\\headwidth{\\color{lightborder}\\leaders\\hrule height \\headrulewidth\\hfill}}\n");
    answer_latex.push_str("\\lfoot{\\scriptsize\\textcolor{gray!60}{\\textsf{MERGEMARK ASSESSMENTS}}}\n");
    answer_latex.push_str("\\rfoot{\\footnotesize\\textbf{Turn over}}\n");
    answer_latex.push_str("\\setlength{\\parskip}{0pt}\n");
    answer_latex.push_str("\\setlength{\\parindent}{0pt}\n");
    answer_latex.push_str("\\widowpenalty=10000\n\\clubpenalty=10000\n\\displaywidowpenalty=10000\n\\interfootnotelinepenalty=10000\n");
    answer_latex.push_str("\\setlist{topsep=0.35cm, parsep=0.2cm, itemsep=0.5cm, leftmargin=0.8cm, labelsep=0.4cm}\n");
    answer_latex.push_str("\\setlist[enumerate,1]{label=\\textbf{\\arabic*.}, leftmargin=*}\n");
    answer_latex.push_str("\\setlist[itemize]{label=\\textbullet, leftmargin=1.4em, itemsep=0.25em, topsep=0.2em}\n");
    answer_latex.push_str("\\begin{document}\n");
    answer_latex.push_str("\\begin{enumerate}\n");

    for (i, question) in fetched_questions.iter().enumerate() {
        let question_num = i + 1;
        let mut content = crate::validate::format_markdown_for_latex(&question.content);

        let snippet = question.math_snippet.trim();
        if !snippet.is_empty() {
            let content_trim = content.trim_end();
            if content_trim.ends_with(snippet) {
                content = content_trim[..content_trim.len() - snippet.len()]
                    .trim_end()
                    .to_string();
            }
        }

        let mark_word = if question.marks == 1 { "mark" } else { "marks" };

        if is_lined {
            if i > 0 {
                latex.push_str("\\newpage\n");
            }
            latex.push_str(&format!("  \\item\\relax {}\n", content));
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

            // High mark questions get continuation page(s) fully lined from top to bottom
            let continuation_pages = if question.marks >= 11 {
                2
            } else if question.marks >= 5 {
                1
            } else {
                0
            };

            if continuation_pages == 0 {
                // Single question page: question + ruled lines filling remaining space + total mark line
                latex.push_str("  \\fillanswerspace\n");
                latex.push_str(&format!(
                    "  \\par\\nopagebreak\\null\\hfill\\textbf{{(Total for Question {} is {} {})}}\\par\\vspace{{0.15cm}}\n\n",
                    question_num, question.marks, mark_word
                ));
            } else {
                // Initial question page: question + ruled lines filling remaining space
                latex.push_str("  \\fillanswerspace\n");

                // Continuation pages
                for p in 1..=continuation_pages {
                    latex.push_str("\\newpage\n\\noindent\n");
                    latex.push_str(&format!("\\textbf{{Question {} continued}}\\\\[0.35cm]\n", question_num));
                    latex.push_str("  \\fillanswerspace\n");
                    if p == continuation_pages {
                        // Final continuation page: total marks anchored at bottom
                        latex.push_str(&format!(
                            "  \\par\\nopagebreak\\null\\hfill\\textbf{{(Total for Question {} is {} {})}}\\par\\vspace{{0.15cm}}\n\n",
                            question_num, question.marks, mark_word
                        ));
                    }
                }
            }
        } else {
            latex.push_str("  \\needspace{4.5cm}\n");
            latex.push_str(&format!("  \\item\\relax {}\n", content));
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
            latex.push_str(&format!(
                "  \\par\\nopagebreak\\vspace{{0.3cm}}\\hfill\\textbf{{(Total for Question {} is {} {})}}\\par\\vspace{{0.45cm}}\n\n",
                question_num, question.marks, mark_word
            ));
        }

        answer_latex.push_str("  \\needspace{4.5cm}\n");
        answer_latex.push_str(&format!("  \\item\\relax {}\n", content));
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
        answer_latex.push_str(&format!(
            "  \\par\\nopagebreak\\vspace{{0.3cm}}\\hfill\\textbf{{(Total for Question {} is {} {})}}\\par\\vspace{{0.35cm}}\n",
            question_num, question.marks, mark_word
        ));

        if let Some(raw_ans) = &question.answer_content {
            let mut ans_content = crate::validate::format_markdown_for_latex(raw_ans);

            let ans_snippet = question.math_snippet.trim();
            if !ans_snippet.is_empty() {
                let ans_content_trim = ans_content.trim_end();
                if ans_content_trim.ends_with(ans_snippet) {
                    ans_content = ans_content_trim[..ans_content_trim.len() - ans_snippet.len()]
                        .trim_end()
                        .to_string();
                }
            }

            answer_latex.push_str("  \\begin{mdframed}[linewidth=0.6pt, linecolor=lightborder, backgroundcolor=cardbg, roundcorner=3pt, innertopmargin=8pt, innerbottommargin=8pt, innerleftmargin=10pt, innerrightmargin=10pt]\n");
            answer_latex.push_str(&format!("  {}\n", ans_content));
            answer_latex.push_str("  \\end{mdframed}\n\n");
        } else {
            answer_latex.push_str("  \\begin{mdframed}[linewidth=0.6pt, linecolor=lightborder, backgroundcolor=cardbg, roundcorner=3pt, innertopmargin=8pt, innerbottommargin=8pt, innerleftmargin=10pt, innerrightmargin=10pt]\n");
            answer_latex.push_str("  \\textit{No mark scheme available for this question.}\n");
            answer_latex.push_str("  \\end{mdframed}\n\n");
        }
    }

    latex.push_str("\\end{enumerate}\n\n");
    latex.push_str("\\vspace{0.8cm}\n\\begin{center}\n");
    latex.push_str(&format!("\\rule{{0.5\\linewidth}}{{0.6pt}}\\\\[0.35cm]\n\\textbf{{\\large TOTAL FOR PAPER: {} MARKS}}\\\\[0.2cm]\n\\textbf{{\\small --- END OF QUESTION PAPER ---}}\n", total_marks));
    latex.push_str("\\end{center}\n");
    latex.push_str("\\rfoot{}\n");
    latex.push_str("\\end{document}\n");

    answer_latex.push_str("\\end{enumerate}\n\n");
    answer_latex.push_str("\\vspace{0.8cm}\n\\begin{center}\n");
    answer_latex.push_str(&format!("\\rule{{0.5\\linewidth}}{{0.6pt}}\\\\[0.35cm]\n\\textbf{{\\large TOTAL FOR PAPER: {} MARKS}}\\\\[0.2cm]\n\\textbf{{\\small --- END OF MARK SCHEME ---}}\n", total_marks));
    answer_latex.push_str("\\end{center}\n");
    answer_latex.push_str("\\rfoot{}\n");
    answer_latex.push_str("\\end{document}\n");

    let download_dir = app.path().download_dir().map_err(|e| e.to_string())?;

    // Embed MergeMark logo for LaTeX compilation
    let logo_bytes = include_bytes!("../icons/128x128@2x.png");
    let logo_path = download_dir.join("mergemark_logo.png");
    let _ = std::fs::write(&logo_path, logo_bytes);

    // Sanitize file name: keep alphanumeric, spaces, hyphens, underscores
    let effective_name = opts
        .file_name
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&file_name);
    let sanitized: String = effective_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim()
        .to_string();

    // Fall back to timestamp-based name if blank
    let base_name = if sanitized.is_empty() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("worksheet_{}", now)
    } else {
        sanitized.replace(' ', "_")
    };

    // Ensure unique by appending counter if file already exists
    let base_name = {
        let mut candidate = base_name.clone();
        let mut counter = 1u32;
        while download_dir.join(format!("{}.pdf", candidate)).exists() {
            candidate = format!("{}_{}", base_name, counter);
            counter += 1;
        }
        candidate
    };

    let worksheet_stem = format!("{}", base_name);
    let answer_stem = format!("{}_answers", base_name);

    let worksheet_tex = download_dir.join(format!("{}.tex", worksheet_stem));
    let answer_key_tex = download_dir.join(format!("{}.tex", answer_stem));

    std::fs::write(&worksheet_tex, &latex)
        .map_err(|e| format!("Failed to write worksheet file: {}", e))?;
    std::fs::write(&answer_key_tex, &answer_latex)
        .map_err(|e| format!("Failed to write answer key file: {}", e))?;

    let pdflatex_cmd = if std::process::Command::new("pdflatex")
        .arg("--version")
        .output()
        .is_ok()
    {
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

    // 2-Pass Compilation for Worksheet: Pass 1 generates .aux, Pass 2 resolves Page X of Y
    let _ = std::process::Command::new(&pdflatex_cmd)
        .current_dir(&download_dir)
        .arg("-interaction=nonstopmode")
        .arg("-output-directory")
        .arg(&download_dir)
        .arg(&worksheet_tex)
        .output();

    let output_worksheet = std::process::Command::new(&pdflatex_cmd)
        .current_dir(&download_dir)
        .arg("-interaction=nonstopmode")
        .arg("-output-directory")
        .arg(&download_dir)
        .arg(&worksheet_tex)
        .output()
        .map_err(|e| format!("Failed to execute pdflatex for worksheet: {}", e))?;

    let worksheet_pdf = download_dir.join(format!("{}.pdf", worksheet_stem));
    if !worksheet_pdf.exists() {
        let stdout = String::from_utf8_lossy(&output_worksheet.stdout);
        let stderr = String::from_utf8_lossy(&output_worksheet.stderr);
        return Err(format!(
            "pdflatex failed to generate worksheet PDF:\n{}\n{}",
            stdout, stderr
        ));
    }

    // 2-Pass Compilation for Answer Key
    let _ = std::process::Command::new(&pdflatex_cmd)
        .current_dir(&download_dir)
        .arg("-interaction=nonstopmode")
        .arg("-output-directory")
        .arg(&download_dir)
        .arg(&answer_key_tex)
        .output();

    let output_answer_key = std::process::Command::new(&pdflatex_cmd)
        .current_dir(&download_dir)
        .arg("-interaction=nonstopmode")
        .arg("-output-directory")
        .arg(&download_dir)
        .arg(&answer_key_tex)
        .output()
        .map_err(|e| format!("Failed to execute pdflatex for answer key: {}", e))?;

    let answer_key_pdf = download_dir.join(format!("{}.pdf", answer_stem));
    if !answer_key_pdf.exists() {
        let stdout = String::from_utf8_lossy(&output_answer_key.stdout);
        let stderr = String::from_utf8_lossy(&output_answer_key.stderr);
        return Err(format!(
            "pdflatex failed to generate answer key PDF:\n{}\n{}",
            stdout, stderr
        ));
    }

    // Clean up all intermediary files
    let _ = std::fs::remove_file(download_dir.join(format!("{}.tex", worksheet_stem)));
    let _ = std::fs::remove_file(download_dir.join(format!("{}.aux", worksheet_stem)));
    let _ = std::fs::remove_file(download_dir.join(format!("{}.log", worksheet_stem)));
    let _ = std::fs::remove_file(download_dir.join(format!("{}.tex", answer_stem)));
    let _ = std::fs::remove_file(download_dir.join(format!("{}.aux", answer_stem)));
    let _ = std::fs::remove_file(download_dir.join(format!("{}.log", answer_stem)));

    Ok(vec![
        worksheet_pdf.to_string_lossy().to_string(),
        answer_key_pdf.to_string_lossy().to_string(),
    ])
}

// ── Per-page text-layer extraction (hint text + document-map scan) ───────────

pub(crate) fn extract_page_texts(file_path: &str, num_pages: usize) -> Vec<String> {
    let mut texts = vec![String::new(); num_pages];
    if !file_path.to_lowercase().ends_with(".pdf") {
        return texts;
    }
    let mut doc = match pdf_extract::Document::load(file_path) {
        Ok(d) => d,
        Err(_) => return texts,
    };
    if doc.is_encrypted() {
        let _ = doc.decrypt("");
    }
    for page_idx in 0..num_pages {
        let mut output = HybridTextOutput::new();
        if pdf_extract::output_doc_page(&doc, &mut output, (page_idx + 1) as u32).is_ok() {
            texts[page_idx] = output.text;
        }
    }

    // Old cleanup rules, preserved: strip blank answer-line artifacts and
    // fix AQA decimal numbering in the raw hint text.
    for text in texts.iter_mut() {
        if !text.is_empty() {
            *text = RE_LINES.replace_all(text, "").to_string();
            *text = RE_ANS_LINES.replace_all(text, "").to_string();
            *text = RE_AQA_NUM.replace_all(text, "${1}.${2}").to_string();
        }
    }
    texts
}

// ── Vision question-paper ingestion (PVRV pipeline) ─────────────────────────

#[tauri::command]
pub async fn parse_pdf_vision(
    app: tauri::AppHandle,
    _api_key: String,
    file_path: String,
    pdf_base64_pages: Option<Vec<String>>,
    _base_url: String,
    model_name: String,
    subject: String,
    module_override: Option<String>,
    paper_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<Question>, String> {
    // Unambiguous build marker: if the running binary does not print this,
    // it predates the figure-fix-v3 build. Rebuild before trusting any cost
    // measurement.
    eprintln!("[BUILD] figure-fix-v5: text-first merges multi-item responses");
    let _concurrency_guard = state.extraction_in_progress.try_lock().map_err(|_| {
        "Another extraction is already in progress. Please wait for it to finish.".to_string()
    })?;

    let model_name = model_name.trim().to_string();

    state
        .cancel_flag
        .store(false, std::sync::atomic::Ordering::Relaxed);

    let pages: Vec<PageInput> = match pdf_base64_pages.filter(|p| !p.is_empty()) {
        Some(pdf_pages) => {
            let num_pages = pdf_pages.len();
            let path_clone = file_path.clone();
            let page_texts = tokio::task::spawn_blocking(move || extract_page_texts(&path_clone, num_pages))
                .await
                .map_err(|e| format!("Thread-pool error: {}", e))?;

            pdf_pages
                .into_iter()
                .enumerate()
                .map(|(i, b64)| PageInput {
                    kind: if b64.is_empty() {
                        crate::pipeline::PageInputKind::TextOnly
                    } else {
                        crate::pipeline::PageInputKind::Image { b64 }
                    },
                    text: page_texts.get(i).cloned().unwrap_or_default(),
                })
                .collect()
        },
        None => {
            let ext = std::path::Path::new(&file_path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            let is_image = ext == "png" || ext == "jpg" || ext == "jpeg" || ext == "webp";

            if ext == "pdf" {
                let path_clone = file_path.clone();
                tokio::task::spawn_blocking(move || crate::pdf_render::render_pdf_pages(std::path::Path::new(&path_clone)))
                    .await
                    .map_err(|e| format!("Thread-pool error: {}", e))??
            } else if is_image {
                let path_clone = file_path.clone();
                let page_input = tokio::task::spawn_blocking(move || crate::pdf_render::load_and_optimize_image_file(std::path::Path::new(&path_clone)))
                    .await
                    .map_err(|e| format!("Thread-pool error: {}", e))??;
                vec![page_input]
            } else {
                return Err("Unsupported file format. Please upload a PDF or image file.".into());
            }
        }
    };

    if state.cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("Import cancelled by user".to_string());
    }

    if pages.len() > crate::pdf_render::MAX_PAGES_PER_IMPORT {
        return Err(format!(
            "Document contains {} pages, which exceeds the limit of {} pages per import. Please split the file into smaller sections.",
            pages.len(),
            crate::pdf_render::MAX_PAGES_PER_IMPORT
        ));
    }

    // ── Fast path: pure-text PDF (all pages TextOnly) → heuristic extraction ──
    // If every page is TextOnly (no rendered images = no diagrams, no visual
    // elements), we can skip the expensive PVRV vision pipeline entirely and
    // use the SubjectClassifier heuristic on the combined text layer. This
    // avoids all LLM calls for pure-text PDFs while still classifying subject,
    // extracting marks, and inserting into the repository.
    let all_text_only = pages.iter().all(|p| matches!(p.kind, crate::pipeline::PageInputKind::TextOnly));
    if all_text_only && !pages.is_empty() {
        let combined_text = pages.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n\n");
        let cleaned = combined_text
            .lines()
            .map(|l| l.trim())
            .collect::<Vec<_>>()
            .join("\n");
        let cleaned = RE_COLLAPSE_NEWLINES
            .replace_all(&cleaned, "\n\n")
            .to_string();

        let classifier = SubjectClassifier::new();
        let state_clone = state.clone();
        let app_clone = app.clone();
        let pool = state_clone.db.lock().await;
        let inserted = insert_questions_from_text(&*pool, &cleaned, &classifier).await?;
        drop(pool);

        // Emit a progress event so the UI knows we're done
        let _ = app_clone.emit("import-report", &serde_json::json!({
            "paper_name": paper_name,
            "kind": "questions",
            "pages_total": pages.len(),
            "pages_processed": pages.len(),
            "questions_extracted": inserted,
            "anomalies": ["Used pure-text fast path (no vision pipeline)"]
        }));

        // Return the inserted questions (they're already in the DB; fetch them)
        let pool = state.db.lock().await;
        let questions: Vec<Question> = sqlx::query_as(
            "SELECT * FROM questions WHERE paper_name = ? ORDER BY question_number ASC"
        )
        .bind(&paper_name)
        .fetch_all(&*pool)
        .await
        .map_err(|e| e.to_string())?;
        drop(pool);
        return Ok(questions);
    }

    let diagrams_dir = app.path().app_data_dir().map(|d| d.join("diagrams")).ok();

    // ── Fetch Taxonomy ───────────────────────────────────────────
    let pool = state.db.lock().await;
    let module_id = module_override
        .clone()
        .ok_or_else(|| "A module must be selected.".to_string())?;

    let module_name: (String,) = sqlx::query_as("SELECT name FROM modules WHERE id = ?")
        .bind(&module_id)
        .fetch_optional(&*pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Selected module not found in database.".to_string())?;

    let topics_rows: Vec<(String,)> = sqlx::query_as("SELECT name FROM topics WHERE module_id = ?")
        .bind(&module_id)
        .fetch_all(&*pool)
        .await
        .map_err(|e| e.to_string())?;
    let allowed_topics = topics_rows.into_iter().map(|(n,)| n).collect();

    let subject_name: (String,) = sqlx::query_as("SELECT name FROM subjects WHERE id = ?")
        .bind(&subject)
        .fetch_optional(&*pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Selected subject not found in database.".to_string())?;
    drop(pool);

    let mut config = PipelineConfig::new(
        model_name.clone(),
        paper_name.trim().to_string(),
        subject_name.0,
        module_name.0,
        Some(std::path::PathBuf::from(&file_path)),
    );
    config.allowed_topics = allowed_topics;
    config.diagrams_dir = diagrams_dir;
    config.max_repairs = 2;
    // Phase 0: questions now get the same output budget as mark schemes.
    // Long physics questions with sub-parts (a)–(f), derivations, graph
    // descriptions, and circuit analysis routinely hit the previous 16k
    // cap, triggering truncation-salvage + repair retries that doubled
    // latency and quarantined questions. 32k gives a healthy headroom at
    // modest cost (output tokens are the expensive part, but a truncated
    // question that requires a full retry is far more expensive).
    config.max_output_tokens = 32768;

    // Text-layer-first extraction: the PDF text layer is authoritative for
    // digital papers (the vision structure pass is skipped for them), so each
    // question is transcribed from text with ZERO image tokens and vision is
    // used only when a figure is actually needed. This is the dominant cost
    // lever on pixel-billed providers (Gemini). Disable with
    // MERGEMARK_TEXT_FIRST=0.
    config.text_first = std::env::var("MERGEMARK_TEXT_FIRST")
        .map(|v| v != "0")
        .unwrap_or(true);

    let (route, client) = resolve_llm_client(&state, model_name.clone())
        .await
        .map_err(|e| e.hint.unwrap_or(e.message))?;

    // Use higher parallelism for BYOK (user controls their own rate limits);
    // conservative for the shared Free Tier key. The 429 retry loop in
    // llm.rs handles backpressure automatically if the provider throttles.
    config.parallelism = match &route {
        crate::billing::BillingRoute::FreeTier { .. } => {
            std::env::var("MERGEMARK_PARALLELISM")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .map(|v| v.clamp(1, 4))
                .unwrap_or(4)
        }
        _ => std::env::var("MERGEMARK_PARALLELISM")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .map(|v| v.clamp(1, 16))
            .unwrap_or(8),
    };

    let progress = TauriProgress { app: app.clone() };

    // ── Extraction cache: skip the pipeline if we've seen this input before ──
    // Compute a content-addressed cache key from the PDF bytes + model +
    // paper name. On cache hit, return the stored questions immediately
    // without making any API calls — re-ingestion becomes instant.
    let file_bytes = std::fs::read(&file_path).unwrap_or_default();
    let cache_key = crate::db::extraction_cache_key(
        &file_bytes,
        &model_name,
        paper_name.trim(),
    );
    let pool_check = state.db.lock().await;
    if let Ok(Some(cached_json)) = crate::db::get_cached_extraction(&pool_check, &cache_key).await {
        if let Ok(cached_questions) = serde_json::from_str::<Vec<Question>>(&cached_json) {
            progress.stage("Loaded from cache — syncing to repository.");
            for q in &cached_questions {
                let topics_json = q.topics.as_deref().unwrap_or("[]");
                let subtopic = if q.subtopic.is_empty() { "Imported" } else { &q.subtopic };
                let _ = sqlx::query(
                    r#"
                    INSERT INTO questions (id, subject, subtopic, topics, marks, content, math_snippet, is_code, paper_name, question_number, module, needs_review, answer_stale)
                    VALUES (?, ?, ?, ?, ?, ?, '', ?, ?, ?, ?, ?, 0)
                    ON CONFLICT(paper_name, question_number) DO UPDATE SET
                        subject = excluded.subject,
                        subtopic = excluded.subtopic,
                        topics = CASE WHEN excluded.topics != '[]' THEN excluded.topics ELSE questions.topics END,
                        marks = excluded.marks,
                        content = excluded.content,
                        is_code = excluded.is_code,
                        module = COALESCE(excluded.module, questions.module),
                        needs_review = excluded.needs_review
                    "#,
                )
                .bind(&q.id)
                .bind(&config.subject)
                .bind(subtopic)
                .bind(topics_json)
                .bind(q.marks)
                .bind(&q.content)
                .bind(q.is_code)
                .bind(&config.paper_name)
                .bind(q.question_number)
                .bind(&q.module)
                .bind(q.needs_review)
                .execute(&*pool_check)
                .await;
            }
            drop(pool_check);
            return Ok(cached_questions);
        }
    }
    drop(pool_check);

    // ── Deterministic figure detection (free, on-device) ───────────────────
    // A single pass over the PDF content stream locates every figure region
    // with zero AI calls. The pipeline uses these to attach figure crops to
    // text-first questions, removing the vision figure pass entirely. Only
    // runs for real PDFs; detection failures degrade to the vision path (the
    // pipeline falls back whenever a question references a figure it cannot
    // supply deterministically).
    let ext = std::path::Path::new(&file_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let page_figures: Vec<Vec<crate::pdf_render::DetectedFigure>> = if ext == "pdf" {
        let path_clone = file_path.clone();
        tokio::task::spawn_blocking(move || {
            crate::pdf_render::detect_pdf_figures(std::path::Path::new(&path_clone))
        })
        .await
        .map_err(|e| format!("Thread-pool error: {}", e))?
        .unwrap_or_else(|e| {
            eprintln!("[DETECT_FIGURES] deterministic detection failed: {}", e);
            Vec::new()
        })
    } else {
        Vec::new()
    };
    // Observable signal for the import logs: if the binary is current, this
    // ALWAYS prints (even "found 0 figures"). Its absence means the running
    // binary predates deterministic detection.
    let detected_total: usize = page_figures.iter().map(Vec::len).sum();
    eprintln!(
        "[DETECT_FIGURES] found {} figures across {} pages (free, on-device)",
        detected_total,
        page_figures.len()
    );

    let (built, mut report): (Vec<BuiltQuestion>, ImportReport) =
        pipeline::run_question_pipeline(
            &client,
            &pages,
            &page_figures,
            &config,
            &progress,
            &state.cancel_flag,
        )
        .await?;

    if state.cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("Import cancelled by user".to_string());
    }

    // Surface the report to the UI — nothing fails silently anymore.
    let _ = app.emit("import-report", &report);

    let pool = state.db.lock().await;

    // Increment free uploads if we used the Free Tier
    if matches!(route, crate::billing::BillingRoute::FreeTier { .. }) {
        let _ = crate::db::increment_free_uploads(&pool).await;
    }

    // ── Persist: idempotent upserts keyed by (paper_name, question_number) ──
    let mut final_questions = Vec::with_capacity(built.len());
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    for q in built {
        let topics_json = if q.topics.is_empty() {
            "[]".to_string()
        } else {
            serde_json::to_string(&q.topics).unwrap_or_else(|_| "[]".to_string())
        };
        let subtopic = q
            .topics
            .first()
            .cloned()
            .unwrap_or_else(|| "Imported".to_string());

        // Keep the existing row's UUID when we're refreshing it.
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM questions WHERE paper_name = ? AND question_number = ? LIMIT 1",
        )
        .bind(&config.paper_name)
        .bind(q.question_number as i64)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        let id = existing
            .map(|(i,)| i)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let db_start = Instant::now();
        sqlx::query(
            r#"
            INSERT INTO questions (id, subject, subtopic, topics, marks, content, math_snippet, is_code, paper_name, question_number, module, needs_review, answer_stale)
            VALUES (?, ?, ?, ?, ?, ?, '', ?, ?, ?, ?, ?, 0)
            ON CONFLICT(paper_name, question_number) DO UPDATE SET
                subject = excluded.subject,
                subtopic = excluded.subtopic,
                topics = CASE WHEN excluded.topics != '[]' THEN excluded.topics ELSE questions.topics END,
                marks = excluded.marks,
                content = excluded.content,
                is_code = excluded.is_code,
                module = COALESCE(excluded.module, questions.module),
                needs_review = excluded.needs_review
            "#,
        )
        .bind(&id)
        .bind(&config.subject)
        .bind(&subtopic)
        .bind(&topics_json)
        .bind(q.marks)
        .bind(&q.content)
        .bind(q.is_code)
        .bind(&config.paper_name)
        .bind(q.question_number as i64)
        .bind(&q.module)
        .bind(q.needs_review)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("DB upsert failed for question {}: {}", q.question_number, e))?;
        report.record_timing(
            "database",
            "upsert_question",
            None,
            Some(q.question_number),
            db_start.elapsed().as_millis() as u64,
        );

        final_questions.push(Question {
            id,
            subject: config.subject.clone(),
            subtopic,
            marks: q.marks,
            content: q.content,
            math_snippet: String::new(),
            is_code: q.is_code,
            answer_content: None,
            topics: Some(topics_json),
            paper_name: config.paper_name.clone(),
            question_number: Some(q.question_number as i64),
            module: Some(q.module),
            needs_review: q.needs_review,
            answer_stale: false,
        });
    }

    tx.commit().await.map_err(|e| e.to_string())?;

    // ── Store in extraction cache for instant re-ingestion ────────────────
    if !final_questions.is_empty() {
        if let Ok(questions_json) = serde_json::to_string(&final_questions) {
            let _ = crate::db::store_cached_extraction(&pool, &cache_key, &questions_json).await;
        }
    }

    // ── Log import run metrics to audit log ───────────────────────────────
    // Use the real OpenRouter usage delta if we can (before/after snapshot),
    // otherwise fall back to model-rate estimate.
    let prompt_est = (pages.len() as i64) * 1400;
    let comp_est = (final_questions.len() as i64) * 350;

    // Try to get real cost from OpenRouter usage delta
    let real_cost = {
        let api_key = crate::db::get_byok_api_key(&pool).await.unwrap_or(None);
        if let Some(ref key) = api_key {
            // Query current usage from OpenRouter
            match crate::cost::fetch_openrouter_key_info(key).await {
                Ok(info) => {
                    let after_usage = info.usage_usd;
                    // We can't do a true before/after without storing state,
                    // so use the model-rate estimate but scale it to be more
                    // accurate using higher token multipliers for reasoning models.
                    let m_lower = model_name.to_lowercase();
                    let is_reasoning = m_lower.contains("3.7") || m_lower.contains("o1") || m_lower.contains("o3");
                    let multiplier = if is_reasoning { 3.0 } else { 1.0 };
                    let est = crate::cost::calculate_cost(&model_name, prompt_est as u64, comp_est as u64) * multiplier;
                    // Use the estimate but log that we have live data available
                    eprintln!("[COST] OpenRouter live usage_usd={:.4}, estimated import cost=${:.4} (reasoning_multiplier={:.1}x)", after_usage, est, multiplier);
                    est
                },
                Err(_) => crate::cost::calculate_cost(&model_name, prompt_est as u64, comp_est as u64),
            }
        } else {
            crate::cost::calculate_cost(&model_name, prompt_est as u64, comp_est as u64)
        }
    };

    let _ = crate::db::record_import_cost(
        &pool,
        &config.paper_name,
        &model_name,
        "question_paper",
        final_questions.len() as i64,
        prompt_est,
        comp_est,
        real_cost,
        0,
    )
    .await;

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

    let url = format!("{}/models", base_url.trim_end_matches('/'));

    let res = client
        .get(&url)
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
pub async fn cancel_import(state: State<'_, AppState>) -> Result<(), String> {
    state
        .cancel_flag
        .store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub async fn delete_all_questions(state: State<'_, AppState>) -> Result<bool, String> {
    let pool = state.db.lock().await;
    sqlx::query("DELETE FROM questions")
        .execute(&*pool)
        .await
        .map_err(|e| e.to_string())?;
    let _ = sqlx::query("DELETE FROM extraction_cache")
        .execute(&*pool)
        .await;
    let _ = sqlx::query("DELETE FROM import_cost_logs")
        .execute(&*pool)
        .await;
    Ok(true)
}

#[tauri::command]
pub async fn delete_questions_by_paper(
    paper_name: String,
    state: State<'_, AppState>,
) -> Result<i64, String> {
    let name = paper_name.trim();
    if name.is_empty() {
        return Err("Cannot delete questions with an empty paper name".to_string());
    }

    let pool = state.db.lock().await;
    let result =
        sqlx::query("DELETE FROM questions WHERE paper_name = ? AND trim(paper_name) != ''")
            .bind(name)
            .execute(&*pool)
            .await
            .map_err(|e| e.to_string())?;

    let _ = sqlx::query("DELETE FROM extraction_cache")
        .execute(&*pool)
        .await;

    // Cascade-delete import cost log entries for this paper (both QP and MS records)
    let _ = sqlx::query("DELETE FROM import_cost_logs WHERE paper_name = ? OR paper_name = ?")
        .bind(name)
        .bind(format!("MS:{}", name))
        .execute(&*pool)
        .await;

    Ok(result.rows_affected() as i64)
}

#[tauri::command]
pub async fn update_question(
    app: tauri::AppHandle,
    id: String,
    new_content: String,
    new_marks: i32,
    new_answer_content: Option<String>,
    new_topics: Option<String>,
    new_module: Option<String>,
) -> Result<(), String> {
    use tauri::Manager;
    let state = app.state::<AppState>();
    let pool = state.db.lock().await;
    sqlx::query("UPDATE questions SET content = ?, marks = ?, answer_content = ?, topics = COALESCE(?, topics), module = COALESCE(?, module), math_snippet = '' WHERE id = ?")
        .bind(new_content)
        .bind(new_marks)
        .bind(new_answer_content)
        .bind(new_topics)
        .bind(new_module)
        .bind(id)
        .execute(&*pool)
        .await
        .map_err(|e| format!("Failed to update question: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn parse_mark_scheme_vision(
    app: tauri::AppHandle,
    _api_key: String,
    file_path: String,
    pdf_base64_pages: Option<Vec<String>>,
    _base_url: String,
    model_name: String,
    paper_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<ProposedMapping>, String> {
    let _concurrency_guard = state.extraction_in_progress.try_lock().map_err(|_| {
        "Another extraction is already in progress. Please wait for it to finish.".to_string()
    })?;

    let model_name = model_name.trim().to_string();

    state
        .cancel_flag
        .store(false, std::sync::atomic::Ordering::Relaxed);

    let ext = std::path::Path::new(&file_path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let is_image = ext == "png" || ext == "jpg" || ext == "jpeg";
    let has_pdf_pages = pdf_base64_pages
        .as_ref()
        .map(|p| !p.is_empty())
        .unwrap_or(false);

    // ── Build PageInput list from whatever source we have ───────────────────
    let pages: Vec<PageInput> = if has_pdf_pages {
        let raw_pages = pdf_base64_pages.unwrap();
        let num_pages = raw_pages.len();
        let path_clone = file_path.clone();
        let texts = tokio::task::spawn_blocking(move || {
            if !path_clone.to_lowercase().ends_with(".pdf") {
                return vec![String::new(); num_pages];
            }
            match pdf_extract::extract_text_by_pages_encrypted(&path_clone, "") {
                Ok(pages) => {
                    let mut out: Vec<String> = pages
                        .into_iter()
                        .map(|s| RE_LINES.replace_all(&s, "").to_string())
                        .map(|s| crate::validate::clean_ligatures(&s))
                        .collect();
                    out.resize(num_pages, String::new());
                    out
                }
                Err(_) => vec![String::new(); num_pages],
            }
        })
        .await
        .unwrap_or_else(|_| vec![String::new(); num_pages]);

        raw_pages
            .into_iter()
            .enumerate()
            .map(|(i, b64)| PageInput {
                kind: if b64.is_empty() {
                    crate::pipeline::PageInputKind::TextOnly
                } else {
                    crate::pipeline::PageInputKind::Image { b64 }
                },
                text: texts.get(i).cloned().unwrap_or_default(),
            })
            .collect()
    } else if file_path.to_lowercase().ends_with(".pdf") {
        let path_clone = file_path.clone();
        tokio::task::spawn_blocking(move || crate::pdf_render::render_pdf_pages(std::path::Path::new(&path_clone)))
            .await
            .map_err(|e| format!("Thread-pool error: {}", e))??
    } else if is_image {
        let path_clone = file_path.clone();
        let page_input = tokio::task::spawn_blocking(move || crate::pdf_render::load_and_optimize_image_file(std::path::Path::new(&path_clone)))
            .await
            .map_err(|e| format!("Thread-pool error: {}", e))??;
        vec![page_input]
    } else {
        // Plain-text source: one synthetic page carrying the whole text.
        let text = match ext.as_str() {
            "txt" => tokio::fs::read_to_string(&file_path)
                .await
                .map_err(|e| e.to_string())?,
            _ => {
                let path_clone = file_path.clone();
                tokio::task::spawn_blocking(move || {
                    pdf_extract::extract_text_encrypted(&path_clone, "")
                        .map_err(|e| format!("PDF extraction failed: {}", e))
                })
                .await
                .map_err(|e| e.to_string())??
            }
        };
        let text = crate::validate::clean_ligatures(&text);
        if text.trim().is_empty() {
            return Err("File is empty or contains only unextractable images.".to_string());
        }
        vec![PageInput {
            kind: crate::pipeline::PageInputKind::TextOnly,
            text,
        }]
    };

    if state.cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("Import cancelled by user".to_string());
    }

    if pages.len() > crate::pdf_render::MAX_PAGES_PER_IMPORT {
        return Err(format!(
            "Document contains {} pages, which exceeds the limit of {} pages per import. Please split the file into smaller sections.",
            pages.len(),
            crate::pdf_render::MAX_PAGES_PER_IMPORT
        ));
    }

    let diagrams_dir = app.path().app_data_dir().map(|d| d.join("diagrams")).ok();

    let mut config = PipelineConfig::new(
        model_name.clone(),
        paper_name.trim().to_string(),
        "MarkScheme".into(),
        "MarkScheme".into(),
        Some(std::path::PathBuf::from(&file_path)),
    );
    config.diagrams_dir = diagrams_dir;
    config.max_repairs = 2;
    config.max_output_tokens = 32768;

    let (route, client) = resolve_llm_client(&state, model_name.clone())
        .await
        .map_err(|e| e.hint.unwrap_or(e.message))?;

    // Route-aware parallelism: higher for BYOK, conservative for Free Tier.
    config.parallelism = match &route {
        crate::billing::BillingRoute::FreeTier { .. } => {
            std::env::var("MERGEMARK_PARALLELISM")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .map(|v| v.clamp(1, 4))
                .unwrap_or(4)
        }
        _ => std::env::var("MERGEMARK_PARALLELISM")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .map(|v| v.clamp(1, 16))
            .unwrap_or(8),
    };

    let progress = TauriProgress { app: app.clone() };

    // ── Mark Scheme Extraction cache: skip vision pipeline if we've seen this file before ──
    let file_bytes = std::fs::read(&file_path).unwrap_or_default();
    let cache_key = crate::db::extraction_cache_key(
        &file_bytes,
        &model_name,
        &format!("MS:{}", paper_name.trim()),
    );
    let pool_check = state.db.lock().await;
    if let Ok(Some(cached_json)) = crate::db::get_cached_extraction(&pool_check, &cache_key).await {
        if let Ok(cached_mappings) = serde_json::from_str::<Vec<ProposedMapping>>(&cached_json) {
            progress.stage("Loaded mark scheme from cache — mapping answers.");
            drop(pool_check);
            return Ok(cached_mappings);
        }
    }
    drop(pool_check);

    let (drafts, report): (Vec<AnswerDraft>, ImportReport) =
        pipeline::run_markscheme_pipeline(&client, &pages, &config, &progress, &state.cancel_flag)
            .await?;

    if state.cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("Import cancelled by user".to_string());
    }

    let _ = app.emit("import-report", &report);

    let pool = state.db.lock().await;

    // Increment free uploads if we used the Free Tier
    if matches!(route, crate::billing::BillingRoute::FreeTier { .. }) {
        let _ = crate::db::increment_free_uploads(&pool).await;
    }

    if drafts.is_empty() {
        return Err("No answers could be extracted from this document. It may be unreadable, or contain no mark-scheme content.".to_string());
    }

    // ── Match answers to DB questions for this paper ────────────────────────
    let pool = state.db.lock().await;
    let questions: Vec<Question> =
        sqlx::query_as("SELECT * FROM questions WHERE paper_name = ? ORDER BY rowid ASC")
            .bind(paper_name.trim())
            .fetch_all(&*pool)
            .await
            .map_err(|e| format!("DB error: {}", e))?;

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
                    q_by_number.entry(n).or_insert(q);
                }
            }
        }
    }

    let mut proposed_mappings: Vec<ProposedMapping> = Vec::new();
    for ans in drafts {
        let q_num = ans.question_number as i64;
        match q_by_number.get(&q_num) {
            Some(q) => {
                // If a previous DB answer exists (older import), propose the
                // fresh transcription as the replacement — the review modal
                // shows both.
                let initial_answer = if let Some(ref db_ans) = q.answer_content {
                    if !db_ans.trim().is_empty() {
                        format!("{}\n\n{}", db_ans, ans.markdown)
                    } else {
                        ans.markdown.clone()
                    }
                } else {
                    ans.markdown.clone()
                };
                proposed_mappings.push(ProposedMapping {
                    question_id: q.id.clone(),
                    raw_content: q.content.clone(),
                    proposed_answer: initial_answer,
                    paper_name: q.paper_name.clone(),
                });
            }
            None => {
                eprintln!(
                    "[MergeMark] mark scheme: no question {} in paper '{}' — answer skipped",
                    q_num, paper_name
                );
            }
        }
    }

    // ── Store mark scheme mappings in extraction cache ──────────────────────
    if !proposed_mappings.is_empty() {
        if let Ok(mappings_json) = serde_json::to_string(&proposed_mappings) {
            let pool = state.db.lock().await;
            let _ = crate::db::store_cached_extraction(&pool, &cache_key, &mappings_json).await;
        }
    }

    // ── Log mark scheme import metrics to audit log ──────────────────────────
    let prompt_est = (pages.len() as i64) * 1200;
    let comp_est = (proposed_mappings.len() as i64) * 300;
    let m_lower = model_name.to_lowercase();
    let is_reasoning = m_lower.contains("3.7") || m_lower.contains("o1") || m_lower.contains("o3");
    let multiplier = if is_reasoning { 3.0 } else { 1.0 };
    let cost_usd = crate::cost::calculate_cost(&model_name, prompt_est as u64, comp_est as u64) * multiplier;
    let pool = state.db.lock().await;
    let _ = crate::db::record_import_cost(
        &pool,
        &format!("MS:{}", paper_name.trim()),
        &model_name,
        "mark_scheme",
        proposed_mappings.len() as i64,
        prompt_est,
        comp_est,
        cost_usd,
        0,
    )
    .await;

    Ok(proposed_mappings)
}

#[tauri::command]
pub async fn commit_mark_schemes(
    mappings: Vec<ProposedMapping>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let pool = state.db.lock().await;

    for mapping in mappings {
        sqlx::query("UPDATE questions SET answer_content = ?, answer_stale = CASE WHEN answer_content IS NOT NULL THEN 1 ELSE 0 END WHERE id = ?")
            .bind(mapping.proposed_answer)
            .bind(mapping.question_id)
            .execute(&*pool)
            .await
            .map_err(|e| format!("DB update error: {}", e))?;
    }

    Ok(())
}

#[tauri::command]
pub async fn mark_question_verified(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let pool = state.db.lock().await;
    sqlx::query("UPDATE questions SET needs_review = 0, answer_stale = 0 WHERE id = ?")
        .bind(id)
        .execute(&*pool)
        .await
        .map_err(|e| format!("Failed to verify question: {}", e))?;
    Ok(())
}

// ── Hybrid text-layer device (unchanged from the previous implementation) ────

struct HybridTextOutput {
    pub text: String,
    word_buf: String,
    is_monospace_word: bool,
    last_end: f64,
    last_y: f64,
    first_char: bool,
    flip_ctm: Option<pdf_extract::Transform>,
}

impl HybridTextOutput {
    pub fn new() -> Self {
        HybridTextOutput {
            text: String::new(),
            word_buf: String::new(),
            is_monospace_word: true,
            last_end: 100000.,
            last_y: 0.,
            first_char: true,
            flip_ctm: None,
        }
    }

    fn flush_word(&mut self) {
        if !self.word_buf.is_empty() {
            if self.is_monospace_word && self.word_buf.chars().any(|c| c.is_alphanumeric()) {
                self.text.push('`');
                self.text.push_str(&self.word_buf);
                self.text.push('`');
            } else {
                self.text.push_str(&self.word_buf);
            }
            self.word_buf.clear();
        }
        self.is_monospace_word = true;
    }
}

impl pdf_extract::OutputDev for HybridTextOutput {
    fn begin_page(
        &mut self,
        _page_num: u32,
        media_box: &pdf_extract::MediaBox,
        _art_box: Option<(f64, f64, f64, f64)>,
    ) -> Result<(), pdf_extract::OutputError> {
        self.flip_ctm = Some(pdf_extract::Transform::row_major(
            1.,
            0.,
            0.,
            -1.,
            0.,
            media_box.ury - media_box.lly,
        ));
        Ok(())
    }
    fn end_page(&mut self) -> Result<(), pdf_extract::OutputError> {
        self.flush_word();
        Ok(())
    }
    fn output_character(
        &mut self,
        trm: &pdf_extract::Transform,
        width: f64,
        _spacing: f64,
        font_size: f64,
        char: &str,
    ) -> Result<(), pdf_extract::OutputError> {
        let flip_ctm = self.flip_ctm.unwrap();
        let m31 = trm.m31 * flip_ctm.m11 + trm.m32 * flip_ctm.m21 + flip_ctm.m31;
        let m32 = trm.m31 * flip_ctm.m12 + trm.m32 * flip_ctm.m22 + flip_ctm.m32;
        let transformed_font_size = (trm.m11.abs() * font_size + trm.m22.abs() * font_size) / 2.0;
        let (x, y) = (m31, m32);

        if !self.first_char {
            if (y - self.last_y).abs() > transformed_font_size * 1.5 {
                self.flush_word();
                self.text.push('\n');
            } else if x < self.last_end && (y - self.last_y).abs() > transformed_font_size * 0.5 {
                self.flush_word();
                self.text.push('\n');
            } else if x > self.last_end + transformed_font_size * 0.1 {
                self.flush_word();
                self.text.push(' ');
            }
        }

        let char_is_space = char.trim().is_empty();
        if !char_is_space {
            if !(width > 0.59 && width < 0.61) {
                self.is_monospace_word = false;
            }
            self.word_buf.push_str(char);
        } else {
            self.flush_word();
            self.text.push_str(char);
        }

        self.first_char = false;
        self.last_y = y;
        self.last_end = x + width * transformed_font_size;
        Ok(())
    }
    fn begin_word(&mut self) -> Result<(), pdf_extract::OutputError> {
        Ok(())
    }
    fn end_word(&mut self) -> Result<(), pdf_extract::OutputError> {
        Ok(())
    }
    fn end_line(&mut self) -> Result<(), pdf_extract::OutputError> {
        Ok(())
    }
}

// ── Hybrid billing command: generate_worksheet_from_pdf ──────────────────────
//
// This is the entry point the React frontend calls to ask MergeMark to
// extract a PDF, route it through the correct LLM transport (free tier or
// the user's own key), and return the structured worksheet JSON.
//
// The command implements every requirement in the spec:
//   1. Reads `usage_config.free_uploads_used` and the stored BYOK key.
//   2. Picks OpenRouter (free tier) or BYOK accordingly.
//   3. Rejects concurrent calls with a 429-style BillingError.
//   4. Drops oversize payloads locally (60 000 chars) before any HTTP.
//   5. Increments `free_uploads_used` ONLY on a 200 OK from OpenRouter.
//   6. Hard 45-second reqwest timeout + 15 000-token cap are owned by
//      `billing.rs`; this command just orchestrates them.

/// The shape React will receive on success. Wraps the raw chat-completion
/// `choices[0].message.content` plus a billing summary so the UI can show
/// "2 of 3 free uploads remaining" without an extra round-trip.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorksheetBillingSummary {
    pub route: crate::billing::BillingRoute,
    pub free_uploads_used: i64,
    pub free_uploads_remaining: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorksheetResult {
    /// Raw chat-completion payload from the LLM (provider-agnostic).
    pub completion: serde_json::Value,
    pub billing: WorksheetBillingSummary,
}

#[tauri::command]
pub async fn generate_worksheet_from_pdf(
    app: tauri::AppHandle,
    file_path: String,
    system_prompt: Option<String>,
    model: Option<String>,
) -> Result<WorksheetResult, crate::billing::BillingError> {
    use crate::billing::{
        call_byok_direct, call_openrouter_free_tier, pick_route, BillingError, BillingRoute,
        MAX_PDF_TEXT_CHARS,
    };
    use crate::AppState;
    use tauri::Manager;

    // ── 1. Concurrency lock — reject overlapping calls with 429 ───────────
    // We hold the lock for the full lifetime of the extraction. The lock
    // is non-blocking: if it can't be acquired instantly, we surface a
    // BillingError::too_many_requests() without doing any work.
    let state: tauri::State<'_, AppState> = app.state();
    let _concurrency_guard = match state.extraction_in_progress.try_lock() {
        Ok(g) => g,
        Err(_) => {
            // Another call is already running. The spec wants a 429.
            return Err(BillingError::too_many_requests());
        }
    };
    // _concurrency_guard is held until end of scope, releasing the lock
    // on any return path.

    // ── 2. Pre-flight payload cap ─────────────────────────────────────────
    // Extract the PDF text off the async runtime. We measure the cleaned
    // length BEFORE any HTTP so the bandwidth is never wasted.
    let path_clone = file_path.clone();
    let extracted_text = tokio::task::spawn_blocking(move || -> Result<String, String> {
        let lower = path_clone.to_lowercase();
        if lower.ends_with(".pdf") {
            pdf_extract::extract_text_encrypted(&path_clone, "")
                .map_err(|e| format!("PDF extraction failed: {}", e))
        } else {
            std::fs::read_to_string(&path_clone).map_err(|e| format!("Failed to read file: {e}"))
        }
    })
    .await
    .map_err(|e| BillingError::network(&format!("thread pool error: {e}")))?
    .map_err(|e| BillingError::network(&e))?;
    let extracted_text = crate::validate::clean_ligatures(&extracted_text);

    let cleaned: String = extracted_text
        .lines()
        .map(|l| l.trim())
        .collect::<Vec<_>>()
        .join("\n");
    let cleaned = regex::Regex::new(r"\n{3,}")
        .unwrap()
        .replace_all(&cleaned, "\n\n")
        .to_string();

    if cleaned.trim().is_empty() {
        return Err(BillingError::network(
            "No text could be extracted from this file. It may be a scanned/image-only PDF.",
        ));
    }

    if cleaned.chars().count() > MAX_PDF_TEXT_CHARS {
        return Err(BillingError::payload_too_large(cleaned.chars().count()));
    }

    // ── 3. Read the live billing state from SQLite ───────────────────────
    let pool = state.db.lock().await;
    let free_uploads_used = crate::db::get_free_uploads_used(&pool)
        .await
        .map_err(|e| BillingError::network(&format!("DB read failed: {e}")))?;
    let byok_key = crate::db::get_byok_api_key(&pool)
        .await
        .map_err(|e| BillingError::network(&format!("DB read failed: {e}")))?;
    let byok_key_present = byok_key.is_some();
    drop(pool);

    // ── 4. Pick the route ────────────────────────────────────────────────
    let route = pick_route(free_uploads_used, byok_key_present);
    // The free tier pins its model inside `billing::call_openrouter_free_tier`
    // (google/gemini-2.5-flash), so `model_name` is only forwarded to the
    // BYOK path. We still resolve a sensible default up front so the
    // BYOK arm has something to send if the caller didn't override.
    let model_name = model.unwrap_or_else(|| {
        if matches!(route, BillingRoute::Byok) {
            "gpt-4o-mini".to_string()
        } else {
            crate::billing::OPENROUTER_MODEL.to_string()
        }
    });
    let system = system_prompt
        .as_deref()
        .unwrap_or("You are a teacher creating a structured educational worksheet from the given source material. Return JSON only.");

    // ── 5. Make the HTTP call ────────────────────────────────────────────
    // The free-tier transport returns a raw `String` (the model's
    // `choices[0].message.content`). The BYOK transport returns the full
    // `serde_json::Value` chat-completion payload. We normalise both into
    // a `serde_json::Value` for the `WorksheetResult.completion` field so
    // the React side sees a consistent shape regardless of route.
    let completion = match &route {
        BillingRoute::FreeTier { .. } => {
            // Route through OpenRouter (Gemini 2.5 Flash, developer's
            // embedded key). The `?` here is the gating step: only when
            // this returns `Ok(_)` do we fall through to step 6 and
            // increment `free_uploads_used`. Any BillingError short-
            // circuits the counter.
            let raw = call_openrouter_free_tier(&cleaned).await?;
            serde_json::Value::String(raw)
        }
        BillingRoute::Byok => {
            // Re-read the base URL on this branch since we didn't need it
            // for the other routes.
            let pool = state.db.lock().await;
            let byok_base = crate::db::get_byok_base_url(&pool)
                .await
                .map_err(|e| BillingError::network(&format!("DB read failed: {e}")))?
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
            let byok_key = byok_key.expect("byok_key must be Some when route is Byok");
            drop(pool);
            call_byok_direct(&byok_base, &byok_key, &model_name, system, &cleaned).await?
        }
        BillingRoute::NeedsByok => {
            // 3 free uploads used and no key on file. Refuse cleanly.
            return Err(BillingError::needs_byok(free_uploads_used));
        }
    };

    // ── 6. Increment the counter — ONLY on a 200 OK from OpenRouter ────
    let (new_used, remaining) = if matches!(route, BillingRoute::FreeTier { .. }) {
        let pool = state.db.lock().await;
        let updated = crate::db::increment_free_uploads(&pool)
            .await
            .map_err(|e| BillingError::network(&format!("DB write failed: {e}")))?;
        let remaining = (crate::db::FREE_UPLOAD_LIMIT - updated).max(0);
        (updated, remaining)
    } else {
        (
            free_uploads_used,
            crate::db::FREE_UPLOAD_LIMIT - free_uploads_used,
        )
    };

    let summary = WorksheetBillingSummary {
        route,
        free_uploads_used: new_used,
        free_uploads_remaining: remaining,
    };

    Ok(WorksheetResult {
        completion,
        billing: summary,
    })
}

// ── BYOK key CRUD commands (called from React Settings page) ─────────────────

/// Returns the current free-tier counter plus a boolean indicating whether
/// a BYOK key is on file. The actual key value is never returned to the
/// frontend (it lives only in SQLite).
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStatus {
    pub free_uploads_used: i64,
    pub free_uploads_limit: i64,
    pub free_uploads_remaining: i64,
    pub byok_key_present: bool,
    pub byok_base_url: Option<String>,
}

#[tauri::command]
pub async fn get_usage_status(state: tauri::State<'_, AppState>) -> Result<UsageStatus, String> {
    let pool = state.db.lock().await;
    let used = crate::db::get_free_uploads_used(&pool)
        .await
        .map_err(|e| format!("DB read failed: {e}"))?;
    let byok = crate::db::get_byok_api_key(&pool)
        .await
        .map_err(|e| format!("DB read failed: {e}"))?;
    let base = crate::db::get_byok_base_url(&pool)
        .await
        .map_err(|e| format!("DB read failed: {e}"))?;
    Ok(UsageStatus {
        free_uploads_used: used,
        free_uploads_limit: crate::db::FREE_UPLOAD_LIMIT,
        free_uploads_remaining: (crate::db::FREE_UPLOAD_LIMIT - used).max(0),
        byok_key_present: byok.is_some(),
        byok_base_url: base,
    })
}

/// Save (or clear, with empty string) the user's BYOK key.
#[tauri::command]
pub async fn set_byok_key(
    api_key: Option<String>,
    base_url: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let pool = state.db.lock().await;
    crate::db::set_byok_api_key(&pool, api_key.as_deref(), base_url.as_deref())
        .await
        .map_err(|e| format!("DB write failed: {e}"))?;
    Ok(())
}

// ── Flashcards Export & Import ────────────────────────────────────────────────

#[tauri::command]
pub async fn export_flashcards(
    app: tauri::AppHandle,
    question_ids: Vec<String>,
    file_name: String,
) -> Result<String, String> {
    let state: State<'_, AppState> = app.state();
    let pool = state.db.lock().await;

    let download_dir = app.path().download_dir().map_err(|e| e.to_string())?;

    let sanitized: String = file_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim()
        .to_string();

    let base_name = if sanitized.is_empty() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("flashcards_{}", now)
    } else {
        sanitized.replace(' ', "_")
    };

    let base_name = {
        let mut candidate = base_name.clone();
        let mut counter = 1u32;
        while download_dir.join(format!("{}.csv", candidate)).exists() {
            candidate = format!("{}_{}", base_name, counter);
            counter += 1;
        }
        candidate
    };

    let file_path = download_dir.join(format!("{}.csv", base_name));

    // Write CSV using csv crate
    let mut wtr = csv::WriterBuilder::new()
        .from_path(&file_path)
        .map_err(|e| e.to_string())?;

    for id in question_ids {
        let q = sqlx::query_as::<_, Question>("SELECT * FROM questions WHERE id = ?")
            .bind(id)
            .fetch_optional(&*pool)
            .await
            .map_err(|e| e.to_string())?;

        if let Some(q) = q {
            let mut front = crate::validate::format_markdown_for_latex(&q.content);
            if !q.math_snippet.is_empty() {
                front = format!("{}\n\n{}", front, crate::validate::format_markdown_for_latex(&q.math_snippet));
            }
            let back = crate::validate::format_markdown_for_latex(&q.answer_content.unwrap_or_default());

            let mut tags = vec![q.subject.clone()];
            if let Some(m) = &q.module {
                if m != "Unknown" && m != "General" {
                    tags.push(m.clone());
                }
            }
            if let Some(t) = &q.topics {
                if let Ok(parsed) = serde_json::from_str::<Vec<String>>(t) {
                    tags.extend(parsed);
                }
            }
            let tags_str = tags.join(" ");

            wtr.write_record(&[&front, &back, &tags_str])
                .map_err(|e| e.to_string())?;
        }
    }

    wtr.flush().map_err(|e| e.to_string())?;

    Ok(file_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn import_flashcards(app: tauri::AppHandle, file_path: String) -> Result<usize, String> {
    let state: State<'_, AppState> = app.state();
    let pool = state.db.lock().await;

    // Determine delimiter (tab for .txt/.tsv, comma otherwise)
    let is_tsv =
        file_path.to_lowercase().ends_with(".txt") || file_path.to_lowercase().ends_with(".tsv");
    let delimiter = if is_tsv { b'\t' } else { b',' };

    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(false)
        .delimiter(delimiter)
        .from_path(&file_path)
        .map_err(|e| e.to_string())?;

    let mut count = 0;

    for result in rdr.records() {
        let record = match result {
            Ok(r) => r,
            Err(_) => continue, // Skip malformed rows
        };

        let front = record.get(0).unwrap_or("").to_string();
        let back = record.get(1).unwrap_or("").to_string();
        let tags = record.get(2).unwrap_or("").to_string();

        if front.trim().is_empty() {
            continue;
        }

        let id = uuid::Uuid::new_v4().to_string();
        let subject = "Imported".to_string();
        let subtopic = "General".to_string();
        let marks = 1;
        let is_code = false;

        // Parse tags as topics if available
        let mut topics = Vec::new();
        for tag in tags.split_whitespace() {
            topics.push(tag.to_string());
        }
        let topics_json = if topics.is_empty() {
            None
        } else {
            serde_json::to_string(&topics).ok()
        };

        sqlx::query(
            "INSERT INTO questions (id, subject, subtopic, marks, content, math_snippet, is_code, answer_content, topics, paper_name, question_number, module)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&id)
        .bind(&subject)
        .bind(&subtopic)
        .bind(marks)
        .bind(&front)
        .bind("")
        .bind(is_code)
        .bind(if back.is_empty() { None } else { Some(back) })
        .bind(topics_json)
        .bind("Imported File")
        .bind(None::<i64>)
        .bind(None::<String>)
        .execute(&*pool)
        .await
        .map_err(|e| e.to_string())?;

        count += 1;
    }

    Ok(count)
}

#[tauri::command]
pub async fn generate_topics_for_module(
    app: tauri::AppHandle,
    module_id: String,
    api_key: String,
    base_url: String,
    model_name: String,
) -> Result<Vec<String>, String> {
    use crate::llm::{LlmClient, LlmConfig, ReqwestLlm};
    use crate::AppState;
    use serde_json::json;
    use tauri::Manager;

    let state: tauri::State<'_, AppState> = app.state();
    let pool = state.db.lock().await;

    // Fetch the module name and subject name
    let module: Option<(String, String)> = sqlx::query_as(
        "SELECT modules.name, subjects.name FROM modules JOIN subjects ON modules.subject_id = subjects.id WHERE modules.id = ?"
    )
    .bind(&module_id)
    .fetch_optional(&*pool)
    .await
    .map_err(|e| e.to_string())?;

    let (module_name, subject_name) = module.ok_or_else(|| "Module not found".to_string())?;

    // Drop the lock before await
    drop(pool);

    let config = LlmConfig {
        base_url,
        api_key,
        model: model_name.clone(),
        timeout: crate::billing::REQUEST_TIMEOUT,
    };
    let client = ReqwestLlm::new(config);

    let system_prompt = "You are an educational taxonomy assistant. Your job is to output a JSON array of curriculum topics for a given module and subject. Output ONLY a valid JSON array of strings.";
    let user_prompt = format!("Subject: {}\nModule: {}\n\nAct strictly according to the official syllabus and textbook chapters for this specific subject and module. Provide an exhaustive list of core topics covered in this module as a JSON array of strings.\n\nCRITICAL INSTRUCTIONS:\n- Output ONLY the exact, short, high-level chapter names (e.g. \"Complex Numbers\", \"Matrices\", \"Proof by Induction\").\n- Do NOT include parentheses, subtopics, or any explanatory descriptions.\n- Output nothing but the JSON array.", subject_name, module_name);

    let request_body = json!({
        "model": model_name,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_prompt }
        ],
        "temperature": 0.2
    });

    let result = client
        .chat(&request_body)
        .await
        .map_err(|e| format!("LLM request failed: {:?}", e))?;

    let content = crate::llm::message_content(&result)
        .map_err(|e| format!("Failed to extract content: {:?}", e))?;

    let mut json_str = content.trim();
    
    // Find the first '[' and last ']' to extract the JSON array robustly
    if let (Some(start), Some(end)) = (json_str.find('['), json_str.rfind(']')) {
        if start <= end {
            json_str = &json_str[start..=end];
        }
    }

    let topics: Vec<String> = serde_json::from_str(json_str)
        .map_err(|e| format!("Failed to parse JSON array from LLM. Raw content: {}\nError: {}", content, e))?;

    // Save topics to DB
    let pool = state.db.lock().await;
    for topic_name in &topics {
        let id = uuid::Uuid::new_v4().to_string();
        let _ = sqlx::query("INSERT INTO topics (id, module_id, name) VALUES (?, ?, ?)")
            .bind(&id)
            .bind(&module_id)
            .bind(topic_name)
            .execute(&*pool)
            .await;
    }

    Ok(topics)
}

// ── OpenRouter Live Spend & Ingestion Cost Commands ──────────────────────────

#[tauri::command]
pub async fn get_openrouter_usage(
    api_key: Option<String>,
    state: State<'_, AppState>,
) -> Result<crate::cost::OpenRouterKeyInfo, String> {
    let key = if let Some(k) = api_key.filter(|s| !s.trim().is_empty()) {
        k
    } else {
        let pool = state.db.lock().await;
        let stored_key = crate::db::get_byok_api_key(&pool).await.unwrap_or(None);
        drop(pool);
        stored_key.unwrap_or_else(|| crate::billing::openrouter_api_key().to_string())
    };

    if key.is_empty() || key.contains("dev-openrouter-key") {
        return Err("No OpenRouter API key configured.".to_string());
    }

    crate::cost::fetch_openrouter_key_info(&key).await
}

#[tauri::command]
pub async fn get_import_cost_history(
    state: State<'_, AppState>,
) -> Result<Vec<crate::db::ImportCostRecord>, String> {
    let pool = state.db.lock().await;
    crate::db::get_import_cost_history(&pool)
        .await
        .map_err(|e| format!("Failed to read import cost history: {}", e))
}

#[tauri::command]
pub async fn clear_import_cost_history(
    state: State<'_, AppState>,
) -> Result<u64, String> {
    let pool = state.db.lock().await;
    crate::db::clear_import_cost_history(&pool)
        .await
        .map_err(|e| format!("Failed to clear import cost history: {}", e))
}

#[tauri::command]
pub async fn delete_import_cost_log(
    id: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let pool = state.db.lock().await;
    crate::db::delete_import_cost_log(&pool, &id)
        .await
        .map_err(|e| format!("Failed to delete import log: {}", e))
}

#[tauri::command]
pub async fn prune_orphaned_import_logs(
    state: State<'_, AppState>,
) -> Result<u64, String> {
    let pool = state.db.lock().await;
    crate::db::prune_orphaned_import_logs(&pool)
        .await
        .map_err(|e| format!("Failed to prune import logs: {}", e))
}

#[tauri::command]
pub async fn get_generation_cost(
    generation_id: String,
    api_key: Option<String>,
    state: State<'_, AppState>,
) -> Result<crate::cost::GenerationCostDetails, String> {
    let key = if let Some(k) = api_key.filter(|s| !s.trim().is_empty()) {
        k
    } else {
        let pool = state.db.lock().await;
        let stored_key = crate::db::get_byok_api_key(&pool).await.unwrap_or(None);
        drop(pool);
        stored_key.unwrap_or_else(|| crate::billing::openrouter_api_key().to_string())
    };

    crate::cost::fetch_openrouter_generation_cost(&generation_id, &key).await
}

