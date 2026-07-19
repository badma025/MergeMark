use crate::llm::{LlmConfig, ReqwestLlm};
use crate::pipeline::{
    self, AnswerDraft, BuiltQuestion, ImportReport, PageInput, PipelineConfig, Progress,
};
use crate::AppState;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager, State};

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

// ── Topic allow-lists (single source of truth for prompts + containment) ──────

const _EDEXCEL_MATHS_PURE: &[&str] = &[
    "Proof",
    "Algebra and functions",
    "Coordinate geometry in the (x, y) plane",
    "Sequences and series",
    "Trigonometry",
    "Exponentials and logarithms",
    "Differentiation",
    "Integration",
    "Numerical methods",
    "Vectors",
];
const _EDEXCEL_MATHS_STATS: &[&str] = &[
    "Statistical sampling",
    "Data presentation and interpretation",
    "Probability",
    "Statistical distributions",
    "Statistical hypothesis testing",
];
const _EDEXCEL_MATHS_MECH: &[&str] = &[
    "Quantities and units in mechanics",
    "Kinematics",
    "Forces and Newton's laws",
    "Moments",
];
const EDEXCEL_MATHS_TOPICS: &[&str] = &[
    "Proof",
    "Algebra and functions",
    "Coordinate geometry in the (x, y) plane",
    "Sequences and series",
    "Trigonometry",
    "Exponentials and logarithms",
    "Differentiation",
    "Integration",
    "Numerical methods",
    "Vectors",
    "Statistical sampling",
    "Data presentation and interpretation",
    "Probability",
    "Statistical distributions",
    "Statistical hypothesis testing",
    "Quantities and units in mechanics",
    "Kinematics",
    "Forces and Newton's laws",
    "Moments",
];

const _FM_CORE_PURE: &[&str] = &[
    "Complex numbers",
    "Argand diagrams",
    "Series",
    "Roots of polynomials",
    "Volumes of revolution",
    "Matrices",
    "Linear transformations",
    "Proof by induction",
    "Vectors",
    "Differential equations",
    "Polar coordinates",
    "Hyperbolic functions",
    "Maclaurin series",
    "Methods in calculus",
];
const _FM_FM1: &[&str] = &[
    "Momentum and impulse",
    "Work, energy and power",
    "Elastic strings and springs",
    "Elastic collisions in one dimension",
    "Elastic collisions in two dimensions",
];
const _FM_FS1: &[&str] = &[
    "Discrete probability distributions",
    "Poisson distribution",
    "Geometric and negative binomial",
    "Hypothesis testing",
    "Central Limit Theorem",
    "Chi-squared tests",
    "Probability generating functions",
    "Quality of tests",
];
const _FM_FP1: &[&str] = &[
    "Vectors (Cross product & planes)",
    "Conic sections",
    "Inequalities",
    "t-formulae",
    "Taylor series",
    "Numerical methods (Further)",
    "Reducible differential equations",
];
const _FM_D1: &[&str] = &[
    "Algorithms",
    "Graphs and networks",
    "Algorithms on graphs",
    "Route inspection",
    "Travelling Salesperson Problem",
    "Linear programming",
    "Simplex algorithm",
];
const _FM_FP2: &[&str] = &[
    "Number theory",
    "Groups",
    "Further calculus",
    "Further matrix algebra",
    "Further complex numbers",
    "Maclaurin series",
];
const _FM_FM2: &[&str] = &[
    "Circular motion",
    "Centres of mass of plane figures",
    "Further centres of mass",
    "Kinematics",
    "Dynamics",
];
const _FM_FS2: &[&str] = &[
    "Linear regression",
    "Continuous probability distributions",
    "Correlation",
    "Hypothesis testing",
];
const _FM_DM2: &[&str] = &[
    "Transportation problems",
    "Allocation (assignment) problems",
    "Flows in networks",
    "Dynamic programming",
    "Game theory",
    "Recurrence relations",
    "Decision analysis",
];
const FURTHER_MATHS_TOPICS: &[&str] = &[
    "Complex numbers",
    "Argand diagrams",
    "Series",
    "Roots of polynomials",
    "Volumes of revolution",
    "Matrices",
    "Linear transformations",
    "Proof by induction",
    "Vectors",
    "Differential equations",
    "Polar coordinates",
    "Hyperbolic functions",
    "Maclaurin series",
    "Methods in calculus",
    "Momentum and impulse",
    "Work, energy and power",
    "Elastic strings and springs",
    "Elastic collisions in one dimension",
    "Elastic collisions in two dimensions",
    "Discrete probability distributions",
    "Poisson distribution",
    "Geometric and negative binomial",
    "Hypothesis testing",
    "Central Limit Theorem",
    "Chi-squared tests",
    "Probability generating functions",
    "Quality of tests",
    "Vectors (Cross product & planes)",
    "Conic sections",
    "Inequalities",
    "t-formulae",
    "Taylor series",
    "Numerical methods (Further)",
    "Reducible differential equations",
    "Algorithms",
    "Graphs and networks",
    "Algorithms on graphs",
    "Route inspection",
    "Travelling Salesperson Problem",
    "Linear programming",
    "Simplex algorithm",
    "Number theory",
    "Groups",
    "Further calculus",
    "Further matrix algebra",
    "Further complex numbers",
    "Circular motion",
    "Centres of mass of plane figures",
    "Further centres of mass",
    "Kinematics",
    "Dynamics",
    "Linear regression",
    "Continuous probability distributions",
    "Correlation",
    "Transportation problems",
    "Allocation (assignment) problems",
    "Flows in networks",
    "Dynamic programming",
    "Game theory",
    "Recurrence relations",
    "Decision analysis",
];
const GCSE_MATHS_TOPICS: &[&str] = &[
    "Number",
    "Algebra",
    "Ratio, proportion and rates of change",
    "Geometry and measures",
    "Probability",
    "Statistics",
];
const GCSE_FM_TOPICS: &[&str] = &[
    "Number",
    "Algebra",
    "Coordinate Geometry",
    "Calculus",
    "Matrix Transformations",
    "Geometry",
];
const _PHYSICS_TOPICS: &[&str] = &[
    "Measurements and their errors",
    "Particles and radiation",
    "Waves",
    "Mechanics and materials",
    "Electricity",
    "Further mechanics",
    "Thermal physics",
    "Fields and their consequences",
    "Nuclear physics",
    "Telescopes",
    "Classification of stars",
    "Cosmology",
];
const _CS_TOPICS: &[&str] = &[
    "Fundamentals of programming",
    "Fundamentals of data structures",
    "Fundamentals of algorithms",
    "Theory of computation",
    "Fundamentals of data representation",
    "Fundamentals of computer systems",
    "Computer organisation and architecture",
    "Consequences of uses of computing",
    "Communication and networking",
    "Fundamentals of databases",
    "Big Data",
    "Fundamentals of functional programming",
];

fn allowed_topics_for_subject(subject: &str) -> Vec<String> {
    let slice: &[&str] = match subject {
        "A Level Mathematics (Edexcel)" | "A Level Mathematics" | "Mathematics" => {
            EDEXCEL_MATHS_TOPICS
        }
        "A Level Further Mathematics (Edexcel)"
        | "A Level Further Mathematics"
        | "Further Mathematics" => FURTHER_MATHS_TOPICS,
        "GCSE Mathematics (Edexcel)" | "GCSE Mathematics" => GCSE_MATHS_TOPICS,
        "GCSE Further Mathematics (AQA)" | "GCSE Further Mathematics" => GCSE_FM_TOPICS,
        // "Physics" => PHYSICS_TOPICS,
        // "Computer Science" => CS_TOPICS,
        _ => &[],
    };
    slice.iter().map(|s| s.to_string()).collect()
}

// ── Helper: shared question-classification + DB-insert logic (legacy path) ────

/// Keyword tables used for TF-IDF-style subject scoring (legacy text imports).
struct SubjectClassifier {
    marks_re: regex::Regex,
    q_split_re: regex::Regex,
    math_re: regex::Regex,
}

impl SubjectClassifier {
    fn new() -> Self {
        Self {
            marks_re: regex::Regex::new(r"(?i)\[\s*(\d+)\s*marks?\s*\]|\(\s*(\d+)\s*\)").unwrap(),
            q_split_re: regex::Regex::new(
                r"(?m)(?:^|\n)(?:Question\s+\d+|Q\.?\s*\d+|\d{1,2}[.)]\s)",
            )
            .unwrap(),
            math_re: regex::Regex::new(r"(?s)\$\$?.+?\$\$?|\\\[.+?\\\]|\\\(.+?\\\)").unwrap(),
        }
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
        if let Some(cap) = self.marks_re.captures_iter(text).last() {
            if let Some(m) = cap.get(1).or_else(|| cap.get(2)) {
                if let Ok(v) = m.as_str().parse::<i32>() {
                    return v.clamp(1, 25);
                }
            }
        }
        1
    }

    fn extract_math(&self, text: &str) -> String {
        self.math_re
            .find(text)
            .map(|m| m.as_str().to_string())
            .unwrap_or_default()
    }

    fn slice_questions<'a>(&self, text: &'a str) -> Vec<&'a str> {
        let splits: Vec<_> = self
            .q_split_re
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
            pdf_extract::extract_text(&path_clone)
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

#[tauri::command]
pub async fn compile_worksheet(
    app: tauri::AppHandle,
    question_ids: Vec<String>,
    file_name: String,
) -> Result<Vec<String>, String> {
    let state: State<'_, AppState> = app.state();
    let pool = state.db.lock().await;

    let mut latex = String::new();
    latex.push_str("\\documentclass[11pt]{article}\n");
    latex.push_str("\\usepackage[margin=1in]{geometry}\n");
    latex.push_str(
        "\\usepackage{amsmath, amssymb, graphicx, xcolor, mdframed, parskip, enumitem}\n",
    );
    latex.push_str("\\usepackage{fancyhdr}\n");
    latex.push_str("\\renewcommand{\\familydefault}{\\sfdefault}\n");
    latex.push_str(
        "% Pin all vertical rhythm at the document level so it cannot vary per question.\n",
    );
    latex.push_str(
        "\\setlist{topsep=0.4cm, parsep=0.2cm, itemsep=0.2cm, leftmargin=1.2cm, labelsep=0.4cm}\n",
    );
    latex.push_str("\\setlist[1]{label=\\textbf{\\arabic*.}, leftmargin=*}\n");
    latex.push_str("\\setlist[2]{label=\\textbf{(\\alph*)}, leftmargin=0.8cm}\n");
    latex.push_str("\\setlist[3]{label=\\textbf{(\\roman*)}, leftmargin=1.8cm}\n");
    latex.push_str("\\setlength{\\parskip}{0pt}\n");
    latex.push_str("\\setlength{\\parindent}{0pt}\n");
    latex.push_str("\\fancypagestyle{firstpage}{%\n");
    latex.push_str("  \\fancyhf{}%\n");
    latex.push_str("  \\lhead{\\textbf{Name:}\\hspace{0.4cm}\\makebox[2.5in]{\\hrulefill}}%\n");
    latex.push_str("  \\chead{\\textbf{Date:}\\hspace{0.4cm}\\makebox[1.5in]{\\hrulefill}}%\n");
    latex.push_str(
        "  \\rhead{\\textbf{Score:}\\hspace{0.4cm}\\makebox[1in]{\\hrulefill} / Total}%\n",
    );
    latex.push_str("}%\n");
    latex.push_str("\\pagestyle{empty}\n");
    latex.push_str("\\setlength{\\headheight}{28pt}\n");
    latex.push_str("\\setlength{\\headsep}{0.5cm}\n");
    latex.push_str("\\begin{document}\n");
    latex.push_str("\\thispagestyle{firstpage}\n\n");
    latex.push_str("\\begin{enumerate}\n");

    let mut answer_latex = String::new();
    answer_latex.push_str("\\documentclass[11pt]{article}\n");
    answer_latex.push_str("\\usepackage[margin=1in]{geometry}\n");
    answer_latex.push_str(
        "\\usepackage{amsmath, amssymb, graphicx, xcolor, mdframed, parskip, enumitem}\n",
    );
    answer_latex.push_str("\\usepackage{fancyhdr}\n");
    answer_latex.push_str("\\renewcommand{\\familydefault}{\\sfdefault}\n");
    answer_latex.push_str(
        "\\setlist{topsep=0.4cm, parsep=0.2cm, itemsep=0.2cm, leftmargin=1.2cm, labelsep=0.4cm}\n",
    );
    answer_latex.push_str("\\setlist[1]{label=\\textbf{\\arabic*.}, leftmargin=*}\n");
    answer_latex.push_str("\\setlist[2]{label=\\textbf{(\\alph*)}, leftmargin=0.8cm}\n");
    answer_latex.push_str("\\setlist[3]{label=\\textbf{(\\roman*)}, leftmargin=1.8cm}\n");
    answer_latex.push_str("\\setlength{\\parskip}{0pt}\n");
    answer_latex.push_str("\\setlength{\\parindent}{0pt}\n");
    answer_latex.push_str("\\fancypagestyle{firstpage}{%\n");
    answer_latex.push_str("  \\fancyhf{}%\n");
    answer_latex.push_str("  \\chead{\\Large\\textbf{Mergemark Practice Paper -- Answer Key}}%\n");
    answer_latex.push_str("}%\n");
    answer_latex.push_str("\\pagestyle{empty}\n");
    answer_latex.push_str("\\setlength{\\headheight}{28pt}\n");
    answer_latex.push_str("\\begin{document}\n");
    answer_latex.push_str("\\thispagestyle{firstpage}\n\n");
    answer_latex.push_str("\\begin{enumerate}\n");

    let mut question_num: usize = 0;
    for id in question_ids {
        let q: Option<Question> = sqlx::query_as("SELECT * FROM questions WHERE id = ?")
            .bind(&id)
            .fetch_optional(&*pool)
            .await
            .map_err(|e| e.to_string())?;

        if let Some(question) = q {
            question_num += 1;
            let mut content = question.content.trim().to_string();
            content = content.replace("\r\n", "\n");

            // Format markdown to LaTeX
            let bold_re = regex::Regex::new(r"\*\*(.+?)\*\*").unwrap();
            content = bold_re.replace_all(&content, r"\textbf{${1}}").to_string();
            let italic_re = regex::Regex::new(r"\*([^\*]+?)\*").unwrap();
            content = italic_re
                .replace_all(&content, r"\textit{${1}}")
                .to_string();
            let multiple_nl_re = regex::Regex::new(r"\n+").unwrap();
            content = multiple_nl_re.replace_all(&content, "\n\n").to_string();

            // Format inline marks
            let inline_marks_re = regex::Regex::new(r"\[(\d+)\s*marks?\]").unwrap();
            content = inline_marks_re
                .replace_all(&content, r"\null\hfill \textbf{[${1} marks]}")
                .to_string();

            // Format list indents for parts (a) and subparts (i).
            // Document-level \setlist already sets leftmargin / labelsep / topsep / parsep / itemsep
            // for all list depths, so we just emit plain itemize blocks and let the preamble handle it.
            let subpart_re =
                regex::Regex::new(r"(?m)^[ \t]*\((i|ii|iii|iv|v|vi|vii|viii|ix|x)\)[ \t]+(.*)")
                    .unwrap();
            content = subpart_re
                .replace_all(
                    &content,
                    "\\begin{itemize}\n\\item[\\textbf{(${1})}] ${2}\n\\end{itemize}",
                )
                .to_string();
            let part_re = regex::Regex::new(r"(?m)^[ \t]*\(([a-z])\)[ \t]+(.*)").unwrap();
            content = part_re
                .replace_all(
                    &content,
                    "\\begin{itemize}\n\\item[\\textbf{(${1})}] ${2}\n\\end{itemize}",
                )
                .to_string();

            // 1. Strip leading numbers (e.g., "1. ", "1)", "- ")
            let leading_num_re = regex::Regex::new(r"^\s*\d+[\.\)\-\s]*").unwrap();
            content = leading_num_re.replace(&content, "").to_string();

            // 2. Strip trailing duplicate math snippet
            let snippet = question.math_snippet.trim();
            if !snippet.is_empty() {
                let content_trim = content.trim_end();
                if content_trim.ends_with(snippet) {
                    content = content_trim[..content_trim.len() - snippet.len()]
                        .trim_end()
                        .to_string();
                }
            }

            // 3. Fix missing inline math wrapping on bare Greek variables
            let greek_re = regex::Regex::new(
                r"(?x)
                (^|[\s,.\-\(])
                \\(theta|alpha|beta|gamma|pi|mu|lambda|phi|omega|sigma|delta)
                ([\s,.\-\)]|$)
            ",
            )
            .unwrap();
            content = greek_re
                .replace_all(&content, r"${1}$\$${2}$${3}")
                .to_string();

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
            // Inline "Total for question" footer — does not introduce a \par or \hfill that would
            // disturb the ruled-line rhythm below it.
            latex.push_str(&format!(
                "  \\hfill\\textbf{{[Total for Question {} is {} marks]}}\\par\\vspace{{0.4cm}}\n",
                question_num, question.marks
            ));

            // Ruled lines: exact 0.9cm pitch, full A4 page coverage.
            //
            // A4 at 1in margins has ~24.13cm of usable vertical height. We compute the number of
            // 0.9cm lines needed to fill the page from the current cursor down to the bottom margin.
            // \pagetotal + \textheight - \baselineskip gives the remaining space, but the
            // simplest robust approach is to draw (marks * 3) lines or enough to cover one full
            // page (27 lines at 0.9cm = 24.3cm, which exactly fills the area) — whichever is
            // larger. We use a tabbing-free \rule + \vspace pattern that gives a guaranteed
            // 0.9cm pitch independent of \baselineskip.
            //
            // 0.3pt line + 0.9cm - 0.3pt = 0.89cm-ish of gap. We use \vspace* to prevent the gap
            // collapsing at page breaks, and \nointerlineskip so the ruled lines don't
            // inherit a \baselineskip between them.
            latex.push_str("  \\nointerlineskip\n");
            let lines_to_draw = (question.marks * 3).max(27);
            for _ in 0..lines_to_draw {
                // \rule with width=\linewidth draws a 0.3pt line across the full text width.
                // The \vspace{0.9cm} is the gap ABOVE the next line, which combined with the
                // 0.3pt line below gives an exact 0.9cm pitch.
                latex.push_str("  \\vspace{0.9cm}\\par\\noindent{\\color{gray!60}\\rule{\\linewidth}{0.3pt}}\\nointerlineskip\n");
            }
            latex.push_str("  \\newpage\n\n");

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
            // Same inline "Total" footer treatment as the worksheet (no \par + \hfill disruption).
            answer_latex.push_str(&format!(
                "  \\hfill\\textbf{{[Total for Question {} is {} marks]}}\\par\n",
                question_num, question.marks
            ));

            if let Some(mut ans_content) = question.answer_content {
                ans_content = ans_content.replace("\r\n", "\n");
                ans_content = bold_re
                    .replace_all(&ans_content, r"\textbf{${1}}")
                    .to_string();
                ans_content = italic_re
                    .replace_all(&ans_content, r"\textit{${1}}")
                    .to_string();
                ans_content = multiple_nl_re.replace_all(&ans_content, "\n\n").to_string();
                ans_content = inline_marks_re
                    .replace_all(&ans_content, r"\null\hfill \textbf{[${1} marks]}")
                    .to_string();
                ans_content = subpart_re
                    .replace_all(
                        &ans_content,
                        "\\begin{itemize}\n\\item[\\textbf{(${1})}] ${2}\n\\end{itemize}",
                    )
                    .to_string();
                ans_content = part_re
                    .replace_all(
                        &ans_content,
                        "\\begin{itemize}\n\\item[\\textbf{(${1})}] ${2}\n\\end{itemize}",
                    )
                    .to_string();

                ans_content = greek_re
                    .replace_all(&ans_content, r"${1}$\$${2}$${3}")
                    .to_string();
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

    // Sanitize file name: keep alphanumeric, spaces, hyphens, underscores
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

fn extract_page_texts(file_path: &str, num_pages: usize) -> Vec<String> {
    let mut texts = vec![String::new(); num_pages];
    if !file_path.to_lowercase().ends_with(".pdf") {
        return texts;
    }
    let doc = match pdf_extract::Document::load(file_path) {
        Ok(d) => d,
        Err(_) => return texts,
    };
    for page_idx in 0..num_pages {
        let mut output = HybridTextOutput::new();
        if pdf_extract::output_doc_page(&doc, &mut output, (page_idx + 1) as u32).is_ok() {
            texts[page_idx] = output.text;
        }
    }

    // Old cleanup rules, preserved: strip blank answer-line artifacts and
    // fix AQA decimal numbering in the raw hint text.
    let re_lines = regex::Regex::new(r"_+|-+").unwrap();
    let re_ans_lines = regex::Regex::new(r"(?m)^\s*[1-6]\s*$").unwrap();
    let re_aqa_num = regex::Regex::new(r"[0O]\s*(\d)\s*\.\s*(\d)").unwrap();
    for text in texts.iter_mut() {
        if !text.is_empty() {
            *text = re_lines.replace_all(text, "").to_string();
            *text = re_ans_lines.replace_all(text, "").to_string();
            *text = re_aqa_num.replace_all(text, "${1}.${2}").to_string();
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
    let _concurrency_guard = state.extraction_in_progress.try_lock().map_err(|_| {
        "Another extraction is already in progress. Please wait for it to finish.".to_string()
    })?;

    let model_name = model_name.trim().to_string();

    state
        .cancel_flag
        .store(false, std::sync::atomic::Ordering::Relaxed);

    let pdf_pages = match pdf_base64_pages {
        Some(p) if !p.is_empty() => p,
        _ => return Err("No rasterized PDF pages provided.".into()),
    };
    let num_pages = pdf_pages.len();

    // Per-page raw text (for the document map + OCR hints), off the async loop.
    let path_clone = file_path.clone();
    let page_texts =
        tokio::task::spawn_blocking(move || extract_page_texts(&path_clone, num_pages))
            .await
            .map_err(|e| format!("Thread-pool error: {}", e))?;

    let pages: Vec<PageInput> = pdf_pages
        .into_iter()
        .enumerate()
        .map(|(i, b64)| PageInput {
            b64,
            text: page_texts.get(i).cloned().unwrap_or_default(),
        })
        .collect();

    let diagrams_dir = app.path().app_data_dir().map(|d| d.join("diagrams")).ok();

    let mut config = PipelineConfig::new(
        model_name.clone(),
        paper_name.trim().to_string(),
        subject.clone(),
    );
    config.module_override = module_override
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    config.allowed_topics = allowed_topics_for_subject(&subject);
    config.diagrams_dir = diagrams_dir;
    config.max_repairs = 2;
    config.max_output_tokens = 16384;
    config.parallelism = std::env::var("MERGEMARK_PARALLELISM")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .map(|v| v.clamp(1, 8))
        .unwrap_or(4);

    let (route, client) = resolve_llm_client(&state, model_name.clone())
        .await
        .map_err(|e| e.hint.unwrap_or(e.message))?;

    let progress = TauriProgress { app: app.clone() };
    let (built, mut report): (Vec<BuiltQuestion>, ImportReport) =
        pipeline::run_question_pipeline(&client, &pages, &config, &progress, &state.cancel_flag)
            .await?;

    // Surface the report to the UI — nothing fails silently anymore.
    let _ = app.emit("import-report", &report);

    let pool = state.db.lock().await;

    // Increment free uploads if we used the Free Tier
    if matches!(route, crate::billing::BillingRoute::FreeTier { .. }) {
        let _ = crate::db::increment_free_uploads(&pool).await;
    }

    // ── Persist: idempotent upserts keyed by (paper_name, question_number) ──
    let mut final_questions = Vec::with_capacity(built.len());

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
        .fetch_optional(&*pool)
        .await
        .map_err(|e| e.to_string())?;

        let id = existing
            .map(|(i,)| i)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let db_start = Instant::now();
        sqlx::query(
            r#"
            INSERT INTO questions (id, subject, subtopic, topics, marks, content, math_snippet, is_code, paper_name, question_number, module)
            VALUES (?, ?, ?, ?, ?, ?, '', ?, ?, ?, ?)
            ON CONFLICT(paper_name, question_number) DO UPDATE SET
                subject = excluded.subject,
                subtopic = excluded.subtopic,
                topics = CASE WHEN excluded.topics != '[]' THEN excluded.topics ELSE questions.topics END,
                marks = excluded.marks,
                content = excluded.content,
                is_code = excluded.is_code,
                module = COALESCE(excluded.module, questions.module)
            "#,
        )
        .bind(&id)
        .bind(&subject)
        .bind(&subtopic)
        .bind(&topics_json)
        .bind(q.marks)
        .bind(&q.content)
        .bind(q.is_code)
        .bind(&config.paper_name)
        .bind(q.question_number as i64)
        .bind(&q.module)
        .execute(&*pool)
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
            subject: subject.clone(),
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
        });
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
            match pdf_extract::extract_text_by_pages(&path_clone) {
                Ok(pages) => {
                    let re_lines = regex::Regex::new(r"_+|-+").unwrap();
                    let mut out: Vec<String> = pages
                        .into_iter()
                        .map(|s| re_lines.replace_all(&s, "").to_string())
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
                b64,
                text: texts.get(i).cloned().unwrap_or_default(),
            })
            .collect()
    } else if is_image {
        use base64::Engine;
        let image_bytes = tokio::fs::read(&file_path)
            .await
            .map_err(|e| format!("Failed to read image: {}", e))?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&image_bytes);
        vec![PageInput {
            b64,
            text: String::new(),
        }]
    } else {
        // Plain-text source: one synthetic page carrying the whole text.
        let text = match ext.as_str() {
            "txt" => tokio::fs::read_to_string(&file_path)
                .await
                .map_err(|e| e.to_string())?,
            _ => {
                let path_clone = file_path.clone();
                tokio::task::spawn_blocking(move || {
                    pdf_extract::extract_text(&path_clone)
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
            b64: String::new(),
            text,
        }]
    };

    let diagrams_dir = app.path().app_data_dir().map(|d| d.join("diagrams")).ok();

    let mut config = PipelineConfig::new(
        model_name.clone(),
        paper_name.trim().to_string(),
        "MarkScheme".into(),
    );
    config.diagrams_dir = diagrams_dir;
    config.max_repairs = 2;
    config.max_output_tokens = 32768;

    let (route, client) = resolve_llm_client(&state, model_name.clone())
        .await
        .map_err(|e| e.hint.unwrap_or(e.message))?;

    let progress = TauriProgress { app: app.clone() };
    let (drafts, report): (Vec<AnswerDraft>, ImportReport) =
        pipeline::run_markscheme_pipeline(&client, &pages, &config, &progress, &state.cancel_flag)
            .await?;

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
            pdf_extract::extract_text(&path_clone)
                .map_err(|e| format!("PDF extraction failed: {e}"))
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
