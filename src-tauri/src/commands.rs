use crate::pipeline::{
    self, BuiltQuestion, ImportReport, PageInput, PipelineConfig, Progress,
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

/// Why an import read no questions, in words for the import screen.
fn not_transcribed_message(report: &ImportReport, file: &std::path::Path) -> String {
    const OFFLINE: &str = "MergeMark works offline and sends nothing to AI services.";
    let picture = file
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["png", "jpg", "jpeg", "webp"].contains(&e.to_ascii_lowercase().as_str()));
    if picture {
        format!("This picture was not transcribed: MergeMark reads scanned PDFs with local OCR, but not pictures imported on their own. Import the paper as a PDF. {OFFLINE}")
    } else if !report.pages_not_transcribed.is_empty() {
        let why = if crate::ocr::available() { "local OCR could not read them" } else { "this computer has no local OCR engine to read them" };
        format!("This paper was not transcribed: pages {:?} are scanned, and {why}. {OFFLINE}", report.pages_not_transcribed)
    } else {
        format!("No questions were found in this paper: its text gave no question headings MergeMark could place. {OFFLINE}")
    }
}

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
    /// The PDF the card was imported from, the SHA-256 of its bytes, and the
    /// pages its question is on (1-based): what it is checked against.
    #[sqlx(default)]
    pub source_path: Option<String>,
    #[sqlx(default)]
    pub source_sha256: Option<String>,
    #[sqlx(default)]
    pub source_first_page: Option<i64>,
    #[sqlx(default)]
    pub source_last_page: Option<i64>,
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
            COALESCE(answer_stale, 0) AS answer_stale,
            source_path,
            source_sha256,
            source_first_page,
            source_last_page
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

    let worksheet_stem = base_name.clone();
    let answer_stem = format!("{}_answers", base_name);

    // pdflatex leaves its .tex, .aux and .log (and the logo it reads) beside
    // what it writes: it works in a folder of its own, and only the two PDFs
    // come out. After a failure the folder stays, with the log the error names.
    let pdflatex = find_pdflatex().ok_or_else(|| PDFLATEX_NOT_FOUND.to_string())?;
    let work = std::env::temp_dir().join(format!("mergemark-compile-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&work).map_err(|e| format!("Failed to create a folder to compile in: {}", e))?;
    // Embed MergeMark logo for LaTeX compilation
    std::fs::write(work.join("mergemark_logo.png"), include_bytes!("../icons/128x128@2x.png"))
        .map_err(|e| format!("Failed to write the logo: {}", e))?;
    let worksheet = compile_latex(&pdflatex, &work, &worksheet_stem, &latex, "worksheet")?;
    let answers = compile_latex(&pdflatex, &work, &answer_stem, &answer_latex, "answer key")?;

    let worksheet_pdf = download_dir.join(format!("{}.pdf", worksheet_stem));
    let answer_key_pdf = download_dir.join(format!("{}.pdf", answer_stem));
    std::fs::copy(&worksheet, &worksheet_pdf).map_err(|e| format!("Failed to save {}: {}", worksheet_pdf.display(), e))?;
    std::fs::copy(&answers, &answer_key_pdf).map_err(|e| format!("Failed to save {}: {}", answer_key_pdf.display(), e))?;
    let _ = std::fs::remove_dir_all(&work);

    Ok(vec![
        worksheet_pdf.to_string_lossy().to_string(),
        answer_key_pdf.to_string_lossy().to_string(),
    ])
}

/// What compiling answers when no pdflatex can run: the worksheet screen
/// then offers to install MiKTeX.
const PDFLATEX_NOT_FOUND: &str = "PDFLATEX_NOT_FOUND";

/// pdflatex on the PATH, or where MiKTeX (for this user or all users) or
/// MacTeX installs it, since an app started from the desktop may not have
/// TeX on its PATH; `None` when none runs.
fn find_pdflatex() -> Option<std::path::PathBuf> {
    let runs = |exe: &std::path::Path| pdflatex_command(exe).arg("--version").output().is_ok_and(|o| o.status.success());
    let on_path = std::path::PathBuf::from("pdflatex");
    if runs(&on_path) {
        return Some(on_path);
    }
    let mut installed = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        installed.push(std::path::PathBuf::from(local).join("Programs\\MiKTeX\\miktex\\bin\\x64\\pdflatex.exe"));
    }
    if let Some(programs) = std::env::var_os("ProgramFiles") {
        installed.push(std::path::PathBuf::from(programs).join("MiKTeX\\miktex\\bin\\x64\\pdflatex.exe"));
    }
    installed.push(std::path::PathBuf::from("/Library/TeX/texbin/pdflatex"));
    installed.into_iter().find(|exe| exe.exists() && runs(exe))
}

/// A pdflatex command that opens no console window: a release build has none
/// of its own, so Windows would flash one for every run.
fn pdflatex_command(exe: &std::path::Path) -> std::process::Command {
    #[allow(unused_mut)]
    let mut command = std::process::Command::new(exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Two pdflatex passes over `source` in `work` (the second resolves "Page X
/// of Y"): the finished PDF there, or why there is none.
fn compile_latex(pdflatex: &std::path::Path, work: &std::path::Path, stem: &str, source: &str, what: &str) -> Result<std::path::PathBuf, String> {
    let tex = work.join(format!("{}.tex", stem));
    std::fs::write(&tex, source).map_err(|e| format!("Failed to write {} file: {}", what, e))?;
    let run = || pdflatex_command(pdflatex).current_dir(work).arg("-interaction=nonstopmode").arg("-output-directory").arg(work).arg(&tex).output();
    let _ = run();
    let output = run().map_err(|e| format!("Failed to execute pdflatex for {}: {}", what, e))?;
    let pdf = work.join(format!("{}.pdf", stem));
    if !pdf.exists() {
        return Err(format!(
            "pdflatex failed to generate {} PDF:\n{}\n{}",
            what,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    reject_unfinished_pdf(&pdf, work, stem)?;
    Ok(pdf)
}

/// A PDF pdflatex finished writing ends with `%%EOF`. pdflatex's exit code
/// can't tell us that: it is non-zero for any recoverable LaTeX error.
fn pdf_is_complete(bytes: &[u8]) -> bool {
    let tail = &bytes[bytes.len().saturating_sub(1024)..];
    tail.windows(5).any(|w| w == b"%%EOF")
}

/// Deletes a PDF that pdflatex stopped writing part-way (seen when MiKTeX
/// installs packages during the run) and reports it, keeping the log.
fn reject_unfinished_pdf(pdf: &std::path::Path, dir: &std::path::Path, stem: &str) -> Result<(), String> {
    let bytes = std::fs::read(pdf).map_err(|e| format!("Failed to read {}: {}", pdf.display(), e))?;
    if pdf_is_complete(&bytes) {
        return Ok(());
    }
    let _ = std::fs::remove_file(pdf);
    Err(format!(
        "pdflatex did not finish writing {}.pdf. If MiKTeX was installing packages, compile again. Log: {}",
        stem,
        dir.join(format!("{}.log", stem)).display()
    ))
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

// ── Question-paper import ─────────────────────────────────────────────────────

/// The import screen's question-paper import, read on this machine.
#[tauri::command]
pub async fn import_paper(
    app: tauri::AppHandle,
    file_path: String,
    pdf_base64_pages: Option<Vec<String>>,
    subject: String,
    module_override: Option<String>,
    paper_name: String,
    state: State<'_, AppState>,
) -> Result<Vec<Question>, String> {
    let host = TauriProgress { app };
    import_question_paper(
        &state,
        &host,
        QuestionImportRequest { file_path, pdf_base64_pages, subject, module_override, paper_name },
    )
    .await
}

/// One question-paper import, as the import screen asks for it.
pub(crate) struct QuestionImportRequest {
    pub file_path: String,
    pub pdf_base64_pages: Option<Vec<String>>,
    pub subject: String,
    pub module_override: Option<String>,
    pub paper_name: String,
}

/// Where an import reports to: its progress stages, its finished report, and
/// the folder its figure crops are saved in. The app sends both to the
/// window; tests collect them.
pub(crate) trait ImportHost: Progress {
    fn report<T: Serialize>(&self, report: &T);
    fn diagrams_dir(&self) -> Option<std::path::PathBuf>;
}

impl ImportHost for TauriProgress {
    fn report<T: Serialize>(&self, report: &T) {
        let _ = self.app.emit("import-report", report);
    }
    fn diagrams_dir(&self) -> Option<std::path::PathBuf> {
        self.app.path().app_data_dir().map(|d| d.join("diagrams")).ok()
    }
}

pub(crate) async fn import_question_paper<H: ImportHost>(
    state: &AppState,
    host: &H,
    request: QuestionImportRequest,
) -> Result<Vec<Question>, String> {
    let QuestionImportRequest { file_path, pdf_base64_pages, subject, module_override, paper_name } = request;
    let _concurrency_guard = state.extraction_in_progress.try_lock().map_err(|_| {
        "Another extraction is already in progress. Please wait for it to finish.".to_string()
    })?;

    state
        .cancel_flag
        .store(false, std::sync::atomic::Ordering::Relaxed);

    let ext = std::path::Path::new(&file_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let is_pdf = ext == "pdf";

    // ── Deterministic figure detection (free, on-device) ───────────────────
    // Started UP-FRONT so it overlaps page assembly — but ONLY when page
    // assembly does not touch pdfium itself: the frontend path extracts text
    // with pdf-extract (a separate library) while detection uses pdfium, so
    // they overlap safely. When assembly renders the PDF with pdfium itself
    // (the None branch), detection must wait — pdfium is not thread-safe, so
    // two of its calls must never run concurrently.
    let has_frontend_pages = pdf_base64_pages.as_ref().map_or(false, |p| !p.is_empty());
    let detection_future = if is_pdf && has_frontend_pages {
        let path_clone = file_path.clone();
        Some(tokio::task::spawn_blocking(move || {
            crate::pdf_render::detect_pdf_figures(std::path::Path::new(&path_clone))
        }))
    } else {
        None
    };

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

    // ── Plain-text path: no page carries a picture or a drawing ──────────────
    // Such a file is split into questions by the legacy text heuristic
    // (SubjectClassifier) instead of the layout map.
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
        let pool = state.db.lock().await;
        let inserted = insert_questions_from_text(&*pool, &cleaned, &classifier).await?;
        drop(pool);

        // Emit a progress event so the UI knows we're done
        host.report(&serde_json::json!({
            "paper_name": paper_name,
            "kind": "questions",
            "pages_total": pages.len(),
            "pages_processed": pages.len(),
            "questions_extracted": inserted,
            "anomalies": ["Used the plain-text path (no layout map)"]
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

    let diagrams_dir = host.diagrams_dir();

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
        paper_name.trim().to_string(),
        subject_name.0,
        module_name.0,
        Some(std::path::PathBuf::from(&file_path)),
    );
    config.allowed_topics = allowed_topics;
    config.diagrams_dir = diagrams_dir;
    // Load the document's character/run/rule geometry ONCE for this import so
    // the deterministic layout repairs (stacked fractions, formulas, nuclides)
    // have real source evidence without reloading the PDF per span.
    config.layout_evidence = crate::pdf_render::load_import_evidence(
        std::path::Path::new(&file_path),
    )
    .map(std::sync::Arc::new);
    config.parallelism = std::env::var("MERGEMARK_PARALLELISM")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .map(|v| v.clamp(1, 16))
        .unwrap_or(8);


    // ── Extraction cache: skip the pipeline if we've seen this input before ──
    // Compute a content-addressed cache key from the PDF bytes + paper name.
    // On cache hit, return the stored questions immediately: re-ingestion
    // becomes instant.
    let file_bytes = std::fs::read(&file_path).unwrap_or_default();
    let cache_key = crate::db::extraction_cache_key(&file_bytes, paper_name.trim());
    // The paper each card records, to be checked against (source_pages.rs).
    let source_path = std::path::absolute(&file_path).map_or_else(|_| file_path.clone(), |p| p.to_string_lossy().into_owned());
    let source_sha256 = (!file_bytes.is_empty()).then(|| crate::source_pages::sha256_hex(&file_bytes));
    let pool_check = state.db.lock().await;
    if let Ok(Some(cached_json)) = crate::db::get_cached_extraction(&pool_check, &cache_key).await {
        if let Ok(cached_questions) = serde_json::from_str::<Vec<Question>>(&cached_json) {
            host.stage("Loaded from cache — syncing to repository.");
            for q in &cached_questions {
                let topics_json = q.topics.as_deref().unwrap_or("[]");
                let subtopic = if q.subtopic.is_empty() { "Imported" } else { &q.subtopic };
                // Same sanitizer gate as the fresh-import path so cached cards
                // can never reintroduce formatting corruption.
                let content =
                    crate::sanitize::sanitize_question_content(&q.content, q.question_number.unwrap_or(0) as u32);
                let _ = sqlx::query(
                    r#"
                    INSERT INTO questions (id, subject, subtopic, topics, marks, content, math_snippet, is_code, paper_name, question_number, module, needs_review, answer_stale, source_path, source_sha256, source_first_page, source_last_page)
                    VALUES (?, ?, ?, ?, ?, ?, '', ?, ?, ?, ?, ?, 0, ?, ?, ?, ?)
                    ON CONFLICT(paper_name, question_number) DO UPDATE SET
                        subject = excluded.subject,
                        subtopic = excluded.subtopic,
                        topics = CASE WHEN excluded.topics != '[]' THEN excluded.topics ELSE questions.topics END,
                        marks = excluded.marks,
                        content = excluded.content,
                        is_code = excluded.is_code,
                        module = COALESCE(excluded.module, questions.module),
                        needs_review = excluded.needs_review,
                        source_path = excluded.source_path,
                        source_sha256 = excluded.source_sha256,
                        source_first_page = excluded.source_first_page,
                        source_last_page = excluded.source_last_page
                    "#,
                )
                .bind(&q.id)
                .bind(&config.subject)
                .bind(subtopic)
                .bind(topics_json)
                .bind(q.marks)
                .bind(&content)
                .bind(q.is_code)
                .bind(&config.paper_name)
                .bind(q.question_number)
                .bind(&q.module)
                .bind(q.needs_review)
                .bind(&source_path)
                .bind(&source_sha256)
                .bind(q.source_first_page)
                .bind(q.source_last_page)
                .execute(&*pool_check)
                .await;
            }
            drop(pool_check);
            let mut sanitized = cached_questions;
            for q in &mut sanitized {
                q.source_path = Some(source_path.clone());
                q.source_sha256 = source_sha256.clone();
                q.content = crate::sanitize::sanitize_question_content(
                    &q.content,
                    q.question_number.unwrap_or(0) as u32,
                );
            }
            return Ok(sanitized);
        }
    }
    drop(pool_check);

    // ── Await figure detection (started concurrently with page assembly) ──
    // Observable signal for the import logs: if the binary is current, this
    // ALWAYS prints (even "found 0 figures"). Its absence means the running
    // binary predates deterministic detection.
    let page_figures: Vec<Vec<crate::pdf_render::DetectedFigure>> = if let Some(future) = detection_future
    {
        future
            .await
            .map_err(|e| format!("Thread-pool error: {}", e))?
            .unwrap_or_else(|e| {
                eprintln!("[DETECT_FIGURES] deterministic detection failed: {}", e);
                Vec::new()
            })
    } else if is_pdf {
        // Not overlapped (page assembly used pdfium): run detection now that
        // the pdfium page rendering above has finished.
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
    let detected_total: usize = page_figures.iter().map(Vec::len).sum();
    eprintln!(
        "[DETECT_FIGURES] found {} figures across {} pages (free, on-device)",
        detected_total,
        page_figures.len()
    );

    let (built, mut report): (Vec<BuiltQuestion>, ImportReport) =
        pipeline::run_question_pipeline(
            &pages,
            &page_figures,
            &config,
            host,
            &state.cancel_flag,
        )
        .await?;

    if state.cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("Import cancelled by user".to_string());
    }

    // Surface the report to the UI — nothing fails silently anymore.
    host.report(&report);

    // A paper nothing could be read from is not an import: say why instead
    // of reporting zero questions as a success.
    if built.is_empty() {
        return Err(not_transcribed_message(&report, std::path::Path::new(&file_path)));
    }

    let pool = state.db.lock().await;

    // ── Persist: idempotent upserts keyed by (paper_name, question_number) ──
    let mut final_questions = Vec::with_capacity(built.len());
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    for q in built {
        // ── Deterministic content sanitizer (last line of defense) ─────────
        // Runs on every card regardless of origin (strict / recovered / fallback):
        // strips OCR margin boilerplate, repairs unicode-minus exponents,
        // balances rogue `$` delimiters, unwraps sentence-level math,
        // reconstructs scrambled isotope notation, and tightens MCQ lists.
        let content = crate::sanitize::sanitize_question_content(&q.content, q.question_number);
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

        let (first_page, last_page) = (*q.pages.start() as i64 + 1, *q.pages.end() as i64 + 1);
        let db_start = Instant::now();
        sqlx::query(
            r#"
            INSERT INTO questions (id, subject, subtopic, topics, marks, content, math_snippet, is_code, paper_name, question_number, module, needs_review, answer_stale, source_path, source_sha256, source_first_page, source_last_page)
            VALUES (?, ?, ?, ?, ?, ?, '', ?, ?, ?, ?, ?, 0, ?, ?, ?, ?)
            ON CONFLICT(paper_name, question_number) DO UPDATE SET
                subject = excluded.subject,
                subtopic = excluded.subtopic,
                topics = CASE WHEN excluded.topics != '[]' THEN excluded.topics ELSE questions.topics END,
                marks = excluded.marks,
                content = excluded.content,
                is_code = excluded.is_code,
                module = COALESCE(excluded.module, questions.module),
                needs_review = excluded.needs_review,
                source_path = excluded.source_path,
                source_sha256 = excluded.source_sha256,
                source_first_page = excluded.source_first_page,
                source_last_page = excluded.source_last_page
            "#,
        )
        .bind(&id)
        .bind(&config.subject)
        .bind(&subtopic)
        .bind(&topics_json)
        .bind(q.marks)
        .bind(&content)
        .bind(q.is_code)
        .bind(&config.paper_name)
        .bind(q.question_number as i64)
        .bind(&q.module)
        .bind(q.needs_review)
        .bind(&source_path)
        .bind(&source_sha256)
        .bind(first_page)
        .bind(last_page)
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
            content,
            math_snippet: String::new(),
            is_code: q.is_code,
            answer_content: None,
            topics: Some(topics_json),
            paper_name: config.paper_name.clone(),
            question_number: Some(q.question_number as i64),
            module: Some(q.module),
            needs_review: q.needs_review,
            answer_stale: false,
            source_path: Some(source_path.clone()),
            source_sha256: source_sha256.clone(),
            source_first_page: Some(first_page),
            source_last_page: Some(last_page),
        });
    }

    tx.commit().await.map_err(|e| e.to_string())?;

    // ── Store in extraction cache for instant re-ingestion ────────────────
    if !final_questions.is_empty() {
        if let Ok(questions_json) = serde_json::to_string(&final_questions) {
            let _ = crate::db::store_cached_extraction(&pool, &cache_key, &questions_json).await;
        }
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

/// The pages of the paper a card was imported from, rendered so the card
/// can be checked against them; or why they cannot be shown (the paper
/// moved, changed, or was never recorded).
#[tauri::command]
pub async fn show_source_pages(
    id: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::source_pages::SourceView, String> {
    let pages_dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("source-pages");
    // Rendering takes a moment: the library stays free meanwhile.
    let pool = state.db.lock().await.clone();
    crate::source_pages::view(&pool, &pages_dir, &id).await
}

/// Points a card's paper at the PDF's new place, if the file chosen is the
/// one the card was imported from (every card from it follows), and shows
/// the card's pages from there.
#[tauri::command]
pub async fn relocate_source(
    id: String,
    path: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::source_pages::SourceView, String> {
    {
        let pool = state.db.lock().await;
        crate::source_pages::relocate(&pool, &id, std::path::Path::new(&path)).await?;
    }
    show_source_pages(id, app, state).await
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
            // A card file carries the card's own Markdown, never the
            // worksheet's print layout or this computer's file paths.
            let mut front = crate::validate::format_markdown_for_card(&q.content);
            if !q.math_snippet.is_empty() {
                front = format!("{}\n\n{}", front, crate::validate::format_markdown_for_card(&q.math_snippet));
            }
            let back = crate::validate::format_markdown_for_card(&q.answer_content.unwrap_or_default());

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

#[cfg(test)]
mod tests {
    use super::*;

    // ── Worksheet compile ───────────────────────────────────────────────────

    /// A worksheet compiles in a folder of its own, which holds the PDF and
    /// pdflatex's other files. (Runs where pdflatex is installed.)
    #[test]
    fn a_worksheet_compiles_in_a_folder_of_its_own() {
        let Some(pdflatex) = find_pdflatex() else {
            eprintln!("[COMPILE] pdflatex is not installed: test skipped");
            return;
        };
        let work = std::env::temp_dir().join(format!("mm_compile_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&work).unwrap();
        let source = "\\documentclass{article}\n\\begin{document}\nPage one.\n\\end{document}\n";
        let pdf = compile_latex(&pdflatex, &work, "sheet", source, "worksheet").expect("a plain document compiles");
        assert_eq!(pdf, work.join("sheet.pdf"));
        assert!(pdf_is_complete(&std::fs::read(&pdf).unwrap()));
        assert!(work.join("sheet.log").exists() && work.join("sheet.tex").exists());
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn a_pdf_pdflatex_stopped_writing_is_not_complete() {
        // A finished PDF ends with %%EOF (and maybe a newline).
        let finished = b"%PDF-1.5\n...\nstartxref\n2390771\n%%EOF\n";
        assert!(pdf_is_complete(finished));
        assert!(pdf_is_complete(b"%PDF-1.5\n...\n%%EOF"));
        // Practice_Examination_Paper_12.pdf: pdflatex died while MiKTeX was
        // installing packages, leaving the file cut off mid-stream.
        let cut_off = b"%PDF-1.5\n%\xd0\xd4\xc5\xd8\n5 0 obj\n<< /Length 6 >>\nstream\n\xeb$5=10\xa4\x89H";
        assert!(!pdf_is_complete(cut_off));
        assert!(!pdf_is_complete(b""));
    }

    // ── No request leaves the machine ───────────────────────────────────────

    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Every HTTP(S) request this process makes goes to a local listener that
    /// counts and refuses it: the proxy settings HTTP clients read point at
    /// it. A test sees any request, whatever host it was meant for.
    struct NetworkTrap {
        connections: Arc<AtomicUsize>,
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
        _serial: std::sync::MutexGuard<'static, ()>,
    }

    impl NetworkTrap {
        fn start() -> Self {
            static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let proxy = format!("http://{}", listener.local_addr().unwrap());
            let connections = Arc::new(AtomicUsize::new(0));
            let counter = Arc::clone(&connections);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    counter.fetch_add(1, Ordering::SeqCst);
                    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));
                    let _ = stream.read(&mut [0u8; 4096]);
                    let _ = stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                }
            });
            let mut saved = Vec::new();
            for var in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"] {
                saved.push((var, std::env::var_os(var)));
                if var == "NO_PROXY" {
                    std::env::remove_var(var);
                } else {
                    std::env::set_var(var, &proxy);
                }
            }
            Self { connections, saved, _serial: serial }
        }

        fn connections(&self) -> usize {
            self.connections.load(Ordering::SeqCst)
        }
    }

    impl Drop for NetworkTrap {
        fn drop(&mut self) {
            for (var, value) in self.saved.drain(..) {
                match value {
                    Some(value) => std::env::set_var(var, value),
                    None => std::env::remove_var(var),
                }
            }
        }
    }

    /// Collects what an import reports instead of sending it to a window.
    struct TestHost {
        dir: std::path::PathBuf,
        reports: std::sync::Mutex<Vec<serde_json::Value>>,
    }

    impl Progress for TestHost {
        fn stage(&self, _message: &str) {}
    }

    impl ImportHost for TestHost {
        fn report<T: Serialize>(&self, report: &T) {
            self.reports.lock().unwrap().push(serde_json::to_value(report).unwrap());
        }
        fn diagrams_dir(&self) -> Option<std::path::PathBuf> {
            Some(self.dir.join("diagrams"))
        }
    }

    /// A fresh app database in a temporary folder, with an AI provider key
    /// and base URL saved, as an earlier version of MergeMark stored them.
    async fn app_with_a_provider_key() -> (AppState, TestHost) {
        let dir = std::env::temp_dir().join(format!("mm_offline_{}", uuid::Uuid::new_v4()));
        let pool = crate::db::init_db(dir.clone()).await.unwrap();
        sqlx::query("CREATE TABLE usage_config (id INTEGER PRIMARY KEY CHECK (id = 1), free_uploads_used INTEGER NOT NULL DEFAULT 0, byok_api_key TEXT, byok_base_url TEXT, updated_at INTEGER NOT NULL DEFAULT 0)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO usage_config (id, byok_api_key, byok_base_url) VALUES (1, 'sk-test-not-a-real-key', 'http://ai.mergemark.invalid/v1')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO subjects (id, name) VALUES ('offline-subject', 'Offline Test Subject')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO modules (id, subject_id, name) VALUES ('offline-module', 'offline-subject', 'Offline Test Module')")
            .execute(&pool)
            .await
            .unwrap();
        let state = AppState {
            db: Arc::new(tokio::sync::Mutex::new(pool)),
            cancel_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            extraction_in_progress: tokio::sync::Mutex::new(()),
        };
        (state, TestHost { dir, reports: Default::default() })
    }

    /// Imports a paper as the app does, into the offline test subject and
    /// module.
    async fn import_offline(state: &AppState, host: &TestHost, file_path: String, paper_name: &str) -> Result<Vec<Question>, String> {
        import_question_paper(
            state,
            host,
            QuestionImportRequest {
                file_path,
                pdf_base64_pages: None,
                subject: "offline-subject".into(),
                module_override: Some("offline-module".into()),
                paper_name: paper_name.into(),
            },
        )
        .await
    }

    fn fixture(rel: &str) -> Option<String> {
        crate::test_support::corpus_paper(rel).map(|path| path.to_string_lossy().into_owned())
    }

    /// A scanned paper on a computer with no local OCR engine is reported as
    /// not transcribed. Nothing is sent anywhere, even with a key saved.
    #[tokio::test]
    async fn a_scan_with_no_local_ocr_is_not_transcribed_and_sends_nothing() {
        let _pdfium = crate::pdf_render::pdfium_test_lock();
        let Some(file_path) = fixture("papers/used/Maths_A-level_M3_Papers-Edexcel__June_2004_QP.pdf") else { return };
        let _no_ocr = crate::ocr::unavailable_for_test::guard();
        let trap = NetworkTrap::start();
        let (state, host) = app_with_a_provider_key().await;
        let result = import_offline(&state, &host, file_path, "Edexcel M3 June 2004")
        .await;
        assert_eq!(trap.connections(), 0, "no request may leave the machine");
        let error = result.expect_err("a paper nobody transcribed is not an import");
        assert!(error.contains("not transcribed"), "{error}");
        let reports = host.reports.lock().unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0]["pagesNotTranscribed"], serde_json::json!([1, 2, 3, 4, 5]), "{}", reports[0]);
    }

    /// Tick-box options printed as bare numbers are part of the question:
    /// the imported card keeps them as the paper prints them.
    #[tokio::test]
    async fn tick_box_options_printed_as_numbers_survive_the_import() {
        let _pdfium = crate::pdf_render::pdfium_test_lock();
        let Some(file_path) = fixture("papers/used/Chemistry_GCSE_Past-Papers_AQA__June_2024_QP.pdf") else { return };
        let (state, host) = app_with_a_provider_key().await;
        let questions = import_offline(&state, &host, file_path, "AQA GCSE Chemistry June 2024")
        .await
        .expect("the paper is read locally");
        let q1 = questions.iter().find(|q| q.question_number == Some(1)).expect("Q1 imported");
        assert!(q1.content.contains("Tick (✓) one box.\n2\n6\n14\n20\n"), "{}", q1.content);
    }

    /// "box" after a possessive is the noun of its sentence, through the
    /// whole import (CIE S1 Jun 2019 Q2: "… at random from Rosa’s box.").
    #[tokio::test]
    async fn a_box_after_a_possessive_survives_the_import() {
        let _pdfium = crate::pdf_render::pdfium_test_lock();
        let Some(file_path) = fixture("papers/used/Maths_A-level_Papers_CIE__June_2019_(v1)_QP_-_S1.pdf") else { return };
        let (state, host) = app_with_a_provider_key().await;
        let questions = import_offline(&state, &host, file_path, "CIE S1 June 2019")
        .await
        .expect("the paper is read locally");
        let q2 = questions.iter().find(|q| q.question_number == Some(2)).expect("Q2 imported");
        assert!(q2.content.contains("at random from Rosa’s box."), "{}", q2.content);
    }

    /// A stem printed for two questions ("Questions 3 and 4 refer to the
    /// following information.") reaches both cards with its drawing, and the
    /// card before ends at its own total, without either (edexcel '21).
    #[tokio::test]
    async fn a_shared_stem_and_its_drawing_reach_each_question_it_names() {
        let _pdfium = crate::pdf_render::pdfium_test_lock();
        let Some(file_path) = fixture("papers/used/edexcel '21 edexcel.pdf") else { return };
        let (state, host) = app_with_a_provider_key().await;
        let questions = import_offline(&state, &host, file_path, "edexcel 21").await.expect("the paper is read locally");
        let card = |n: i64| questions.iter().find(|q| q.question_number == Some(n)).unwrap_or_else(|| panic!("Q{n} imported"));
        for n in [3, 4] {
            let q = card(n);
            assert!(q.content.starts_with("Questions 3 and 4 refer to the following information."), "Q{n}: {}", q.content);
            assert!(q.content.contains("horizontal spring") && q.content.contains("![Diagram]("), "Q{n}: {}", q.content);
        }
        let q2 = card(2);
        assert!(!q2.content.contains("refer to") && !q2.content.contains("![Diagram]("), "{}", q2.content);
    }

    /// What the layout reads as maths reaches the card as maths:
    /// - Edexcel Oct 2021 Q12's "2(u − 1)³/u": a letter, a minus and a digit
    ///   inside maths are a subtraction, not a unit with an exponent;
    /// - AQA Physics Jun 2022 Q5's fraction of two phrases: words inside
    ///   \text{…} are maths' own text, not a sentence wrapped in maths.
    #[tokio::test]
    async fn maths_the_layout_reads_reaches_the_card_whole() {
        let _pdfium = crate::pdf_render::pdfium_test_lock();
        let (Some(edexcel), Some(aqa)) = (
            fixture("papers/used/Maths_A-level_Papers_Edexcel__October_2021_QP.pdf"),
            fixture("papers/used/Physics_A-level_Past-Papers_AQA__June_2022_QP.pdf"),
        ) else { return };
        let (state, host) = app_with_a_provider_key().await;
        let questions = import_offline(&state, &host, edexcel, "Edexcel Oct 2021").await.expect("the paper is read locally");
        let q12 = questions.iter().find(|q| q.question_number == Some(12)).expect("Q12 imported");
        assert!(q12.content.contains(r"\frac{2(u -1)^{3}}{u}") && !q12.content.contains("$$"), "{}", q12.content);
        let questions = import_offline(&state, &host, aqa, "AQA Physics Jun 2022").await.expect("the paper is read locally");
        let q5 = questions.iter().find(|q| q.question_number == Some(5)).expect("Q5 imported");
        assert!(
            q5.content.contains(r"$\frac{\text{specific charge of the particle in column } \mathbf{X}}{\text{specific charge of the particle in column } \mathbf{Y}}$"),
            "{}",
            q5.content
        );
    }

    /// A bracketed number set against the word before it ("Work out P(5)",
    /// AQA GCSE Maths Jun 2024 Q4; "Find P(3) and P(10)", AQA A-level Maths
    /// Jun 2019 Q8) is the question's text, not a mark allocation: the card
    /// keeps it, and its marks are those printed.
    #[tokio::test]
    async fn a_bracketed_number_against_its_word_is_no_allocation() {
        let _pdfium = crate::pdf_render::pdfium_test_lock();
        let (Some(gcse), Some(alevel)) = (
            fixture("papers/used/Maths_GCSE_Past-Papers_AQA__June_2024_QP.pdf"),
            fixture("papers/used/Maths_A-level_Papers_AQA__June_2019_QP.pdf"),
        ) else { return };
        let (state, host) = app_with_a_provider_key().await;
        let questions = import_offline(&state, &host, gcse, "AQA GCSE Maths Jun 2024").await.expect("the paper is read locally");
        let q4 = questions.iter().find(|q| q.question_number == Some(4)).expect("Q4 imported");
        assert!(q4.content.contains("Work out P(5)"), "{}", q4.content);
        assert_eq!(q4.marks, 4, "{}", q4.content);
        let questions = import_offline(&state, &host, alevel, "AQA A-level Maths Jun 2019").await.expect("the paper is read locally");
        let q8 = questions.iter().find(|q| q.question_number == Some(8)).expect("Q8 imported");
        assert!(q8.content.contains("Find P(3) and P(10)"), "{}", q8.content);
        assert_eq!(q8.marks, 4, "{}", q8.content);
    }

    /// An allocation set against the full stop before it ("… John's age at
    /// this time.(2)", naiker M16 Q1) is still one: the card carries the
    /// paper's 11 marks.
    #[tokio::test]
    async fn an_allocation_against_the_full_stop_before_it_is_still_one() {
        let _pdfium = crate::pdf_render::pdfium_test_lock();
        let Some(file_path) = fixture("papers/used/naikermaths_paper_m16_pure.pdf") else { return };
        let (state, host) = app_with_a_provider_key().await;
        let questions = import_offline(&state, &host, file_path, "naiker M16").await.expect("the paper is read locally");
        let q1 = questions.iter().find(|q| q.question_number == Some(1)).expect("Q1 imported");
        assert!(q1.content.contains("John's age at this time.") && !q1.content.contains("time.(2)"), "{}", q1.content);
        assert_eq!(q1.marks, 11, "{}", q1.content);
    }

    /// Each card records the paper it was read from, the SHA-256 of the file
    /// and the pages its question is on, so it can be checked against them:
    /// read fresh, and again from the cache.
    #[tokio::test]
    async fn an_imported_card_records_its_paper_and_its_pages() {
        let _pdfium = crate::pdf_render::pdfium_test_lock();
        let Some(file_path) = fixture("papers/used/Maths_A-level_Papers_CIE__June_2019_(v1)_QP_-_S1.pdf") else { return };
        let path = std::path::absolute(&file_path).unwrap().to_string_lossy().into_owned();
        let sha = crate::source_pages::sha256_hex(&std::fs::read(&file_path).unwrap());
        let (state, host) = app_with_a_provider_key().await;
        for read in ["fresh", "cached"] {
            let questions = import_offline(&state, &host, file_path.clone(), "CIE S1 June 2019").await.expect("the paper is read locally");
            assert_eq!(host.reports.lock().unwrap().len(), 1, "{read}: only a fresh read reports");
            for (number, pages) in [(2, (4, 4)), (4, (6, 7))] {
                let q = questions.iter().find(|q| q.question_number == Some(number)).expect("imported");
                assert_eq!((q.source_path.as_deref(), q.source_sha256.as_deref()), (Some(path.as_str()), Some(sha.as_str())), "{read} Q{number}");
                assert_eq!((q.source_first_page, q.source_last_page), (Some(pages.0), Some(pages.1)), "{read} Q{number}");
                let pool = state.db.lock().await;
                let saved: (Option<String>, Option<String>, Option<i64>, Option<i64>) = sqlx::query_as("SELECT source_path, source_sha256, source_first_page, source_last_page FROM questions WHERE id = ?")
                    .bind(&q.id)
                    .fetch_one(&*pool)
                    .await
                    .unwrap();
                assert_eq!(saved, (Some(path.clone()), Some(sha.clone()), Some(pages.0), Some(pages.1)), "{read} Q{number}");
            }
        }
    }

    /// What the last page prints after "END OF QUESTION PAPER" (the board's
    /// logo over its copyright notice) is no figure of the last question.
    #[tokio::test]
    async fn the_back_page_logo_is_no_figure_of_the_last_question() {
        let _pdfium = crate::pdf_render::pdfium_test_lock();
        let Some(file_path) = fixture("papers/used/Maths_A-level_Papers_OCR-MEI-Further__June_2019_QP.pdf") else { return };
        let (state, host) = app_with_a_provider_key().await;
        let questions = import_offline(&state, &host, file_path, "OCR MEI Further Modelling with Algorithms June 2019")
        .await
        .expect("the paper is read locally");
        // Q5 is the last question, on the page that ends with the logo.
        let q5 = questions.iter().find(|q| q.question_number == Some(5)).expect("Q5 imported");
        assert!(q5.content.trim_end().ends_with("**[8 marks]**"), "{}", q5.content);
        assert!(!q5.content.contains("![Diagram]"), "{}", q5.content);
        // Q2's network (Fig. 2.1) is still its figure.
        let q2 = questions.iter().find(|q| q.question_number == Some(2)).expect("Q2 imported");
        assert!(q2.content.contains("![Diagram]"), "{}", q2.content);
    }

    /// A question printed twice, ending "EITHER (f) … OR (g) …", is imported
    /// once, and its tags agree with the total it prints.
    #[tokio::test]
    async fn a_question_with_alternative_parts_keeps_its_printed_total() {
        let _pdfium = crate::pdf_render::pdfium_test_lock();
        let Some(file_path) = fixture("papers/used/Economics_A-level_Past-Papers_Edexcel-A__June_2018_QP_-_Paper_2_Edexcel_(A)_Economics_AS-level.pdf") else { return };
        let (state, host) = app_with_a_provider_key().await;
        let questions = import_offline(&state, &host, file_path, "Edexcel A Economics June 2018 Paper 2")
        .await
        .expect("the paper is read locally");
        // "(Total for Question 6 = 60 marks)": (a)-(e) 40, then EITHER (f) OR (g), 20 each.
        let q6 = questions.iter().find(|q| q.question_number == Some(6)).expect("Q6 imported");
        assert_eq!(q6.marks, 60, "{}", q6.content);
        assert_eq!(crate::validate::sum_inline_marks(&q6.content), 60, "{}", q6.content);
    }
}

