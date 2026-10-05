use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
use std::path::PathBuf;
use std::str::FromStr;

pub async fn init_db(app_data_dir: PathBuf) -> Result<SqlitePool, sqlx::Error> {
    // 1. Ensure the app data directory exists
    if !app_data_dir.exists() {
        std::fs::create_dir_all(&app_data_dir).expect("Failed to create app data directory");
    }

    // 2. Define the path to the database file
    let db_path = app_data_dir.join("mergemark.db");
    
    // Using `mode=rwc` ensures the file is created if it doesn't exist
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());

    let options = SqliteConnectOptions::from_str(&db_url)?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_secs(5));

    // 3. Connect to the SQLite database
    let pool = SqlitePool::connect_with(options).await?;

    // 4. Run the migration to create the questions table if it doesn't exist
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS questions (
            id TEXT PRIMARY KEY,
            subject TEXT DEFAULT 'Mathematics' NOT NULL,
            subtopic TEXT NOT NULL,
            marks INTEGER NOT NULL,
            content TEXT NOT NULL,
            math_snippet TEXT NOT NULL,
            is_code BOOLEAN NOT NULL,
            answer_content TEXT,
            topics TEXT,
            paper_name TEXT DEFAULT '',
            question_number INTEGER,
            module TEXT,
            needs_review BOOLEAN NOT NULL DEFAULT 0,
            answer_stale BOOLEAN NOT NULL DEFAULT 0
        );
        "#
    )
    .execute(&pool)
    .await?;

    // Migrate existing table by adding new columns. Ignore error if the column already exists.
    let _ = sqlx::query("ALTER TABLE questions ADD COLUMN answer_content TEXT")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE questions ADD COLUMN topics TEXT")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE questions ADD COLUMN paper_name TEXT DEFAULT ''")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE questions ADD COLUMN question_number INTEGER")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE questions ADD COLUMN module TEXT")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE questions ADD COLUMN needs_review BOOLEAN NOT NULL DEFAULT 0")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE questions ADD COLUMN answer_stale BOOLEAN NOT NULL DEFAULT 0")
        .execute(&pool)
        .await;

    // Task 4: Legacy cleanup & NULL sanitization
    let _ = sqlx::query("UPDATE questions SET math_snippet = '' WHERE math_snippet IS NULL OR math_snippet != ''")
        .execute(&pool)
        .await;
    let _ = sqlx::query("UPDATE questions SET subject = 'Mathematics' WHERE subject IS NULL")
        .execute(&pool)
        .await;
    let _ = sqlx::query("UPDATE questions SET subtopic = '' WHERE subtopic IS NULL")
        .execute(&pool)
        .await;
    let _ = sqlx::query("UPDATE questions SET marks = 0 WHERE marks IS NULL")
        .execute(&pool)
        .await;
    let _ = sqlx::query("UPDATE questions SET content = '' WHERE content IS NULL")
        .execute(&pool)
        .await;
    let _ = sqlx::query("UPDATE questions SET is_code = 0 WHERE is_code IS NULL")
        .execute(&pool)
        .await;
    let _ = sqlx::query("UPDATE questions SET paper_name = '' WHERE paper_name IS NULL")
        .execute(&pool)
        .await;
    let _ = sqlx::query("UPDATE questions SET needs_review = 0 WHERE needs_review IS NULL")
        .execute(&pool)
        .await;
    let _ = sqlx::query("UPDATE questions SET answer_stale = 0 WHERE answer_stale IS NULL")
        .execute(&pool)
        .await;

    // Heal existing polar equations and unbalanced delimiters directly in SQLite
    if let Ok(rows) = sqlx::query("SELECT id, content FROM questions WHERE content LIKE '%polar%' OR content LIKE '%cardioid%' OR content LIKE '%theta%' OR content LIKE '%cos%' OR content LIKE '%sin%'")
        .fetch_all(&pool)
        .await
    {
        use sqlx::Row;
        for row in rows {
            if let (Ok(id), Ok(content)) = (row.try_get::<String, _>("id"), row.try_get::<String, _>("content")) {
                let healed = crate::validate::heal_polar_equations(&content);
                if healed != content {
                    let _ = sqlx::query("UPDATE questions SET content = ? WHERE id = ?")
                        .bind(healed)
                        .bind(id)
                        .execute(&pool)
                        .await;
                }
            }
        }
    }

    // ── Idempotency migration ────────────────────────────────────────────────
    // Before the unique index can exist, collapse any duplicate
    // (paper_name, question_number) rows produced by older builds, keeping
    // the most recently written row.
    let _ = sqlx::query(
        r#"
        DELETE FROM questions
        WHERE trim(COALESCE(paper_name, '')) != ''
          AND question_number IS NOT NULL
          AND rowid NOT IN (
              SELECT MAX(rowid) FROM questions
              WHERE trim(COALESCE(paper_name, '')) != ''
                AND question_number IS NOT NULL
              GROUP BY paper_name, question_number
          );
        "#,
    )
    .execute(&pool)
    .await;

    // Composite-key uniqueness — the old architecture's invariant, now
    // enforced by the database itself so re-imports upsert instead of
    // duplicating (NULL question_numbers stay insertable for legacy rows).
    let _ = sqlx::query(
        r#"
        CREATE UNIQUE INDEX IF NOT EXISTS ux_questions_paper_qnum
        ON questions(paper_name, question_number);
        "#,
    )
    .execute(&pool)
    .await;

    let _ = sqlx::query("CREATE INDEX IF NOT EXISTS idx_questions_subject ON questions(subject);")
        .execute(&pool)
        .await;
    let _ = sqlx::query("CREATE INDEX IF NOT EXISTS idx_questions_module ON questions(module);")
        .execute(&pool)
        .await;
    let _ = sqlx::query("CREATE INDEX IF NOT EXISTS idx_questions_paper_name ON questions(paper_name);")
        .execute(&pool)
        .await;

    // ── Taxonomy Tables (BYOT) ─────────────────────────────────────────────
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS subjects (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE
        );
        "#
    ).execute(&pool).await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS modules (
            id TEXT PRIMARY KEY,
            subject_id TEXT NOT NULL,
            name TEXT NOT NULL,
            FOREIGN KEY(subject_id) REFERENCES subjects(id) ON DELETE CASCADE,
            UNIQUE(subject_id, name)
        );
        "#
    ).execute(&pool).await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS topics (
            id TEXT PRIMARY KEY,
            module_id TEXT NOT NULL,
            name TEXT NOT NULL,
            FOREIGN KEY(module_id) REFERENCES modules(id) ON DELETE CASCADE,
            UNIQUE(module_id, name)
        );
        "#
    ).execute(&pool).await?;

    // ── Extraction cache for instant re-ingestion ──────────────────────────
    // Stores completed extraction results keyed by (file_content_hash,
    // model, paper_name, cache_version). When a user re-ingests the same
    // paper with the same model, the pipeline is skipped entirely and the
    // cached questions are returned directly.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS extraction_cache (
            cache_key    TEXT PRIMARY KEY,
            questions    TEXT NOT NULL,
            created_at   INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .execute(&pool)
    .await?;

    // ── Import Cost & Spend Audit Log ─────────────────────────────────────
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS import_cost_logs (
            id                  TEXT PRIMARY KEY,
            paper_name          TEXT NOT NULL,
            model_name          TEXT NOT NULL,
            paper_type          TEXT NOT NULL DEFAULT 'question_paper',
            questions_count     INTEGER NOT NULL DEFAULT 0,
            prompt_tokens       INTEGER NOT NULL DEFAULT 0,
            completion_tokens   INTEGER NOT NULL DEFAULT 0,
            total_tokens        INTEGER NOT NULL DEFAULT 0,
            cost_usd            REAL NOT NULL DEFAULT 0.0,
            duration_ms         INTEGER NOT NULL DEFAULT 0,
            created_at          INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .execute(&pool)
    .await?;

    // Phase 2: per-stage token breakdown (JSON array of
    // {stage, promptTokens, completionTokens}). Idempotent — the ALTER
    // fails with "duplicate column" on databases that already have it.
    if let Err(e) = sqlx::query(
        "ALTER TABLE import_cost_logs ADD COLUMN stage_breakdown TEXT",
    )
    .execute(&pool)
    .await
    {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            return Err(e);
        }
    }

    // Seed taxonomy if completely empty
    let count: (i64,) = sqlx::query_as("SELECT count(*) FROM subjects").fetch_one(&pool).await?;
    if count.0 == 0 {
        seed_taxonomy(&pool).await?;
    }

    Ok(pool)
}

// ── Extraction cache helpers ─────────────────────────────────────────────
//
// Content-addressed cache for completed extraction results. When a user
// re-ingests the same PDF, the pipeline is skipped entirely and cached
// questions are returned instantly.
//
// Bump EXTRACTION_CACHE_VERSION when the extraction logic or validation
// rules change in a way that would produce different output for the same
// input.

/// Bump this to invalidate all cached extraction results after logic changes.
/// v3: formatting-rules pipeline (sanitize seam, structural validator,
/// Tier-0 figure lever) — cached v2 entries hold unformatted cards.
/// v4: imports are local only; entries cached from AI-model runs are dropped.
/// v5: import fixes round 2 (question ends, marks, symbol fonts, numbers
/// as options): a cached v4 card can lack options, allocations or parts.
/// v6: import fixes round 3 (fractions under a line of text, drawings on
/// graph paper, the end of the last question): a cached v5 card can hold a
/// stacked fraction read as scripts, a grid cropped to its drawing without
/// the axes, or the back page's logo as the last question's figure.
/// v7: 1.0 W1 round 1 (small fractions on a line of text, a bar over a
/// letter and its square, a key underlined above close text, a question
/// ending at its own total, "box" after a possessive, tables drawn cell by
/// cell): a cached v6 card can hold one of those defects.
pub const EXTRACTION_CACHE_VERSION: u32 = 7;

/// Compute a deterministic cache key from file content + parameters.
/// Uses a fast FNV-1a hash of the file bytes (not cryptographic, just
/// collision-resistant enough for a local cache).
pub fn extraction_cache_key(file_bytes: &[u8], paper_name: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    file_bytes.hash(&mut hasher);
    paper_name.hash(&mut hasher);
    EXTRACTION_CACHE_VERSION.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Look up a cached extraction result. Returns `Some(json_string)` if found.
pub async fn get_cached_extraction(
    pool: &SqlitePool,
    cache_key: &str,
) -> Result<Option<String>, sqlx::Error> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT questions FROM extraction_cache WHERE cache_key = ?",
    )
    .bind(cache_key)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(q,)| q))
}

/// Store a completed extraction result in the cache.
pub async fn store_cached_extraction(
    pool: &SqlitePool,
    cache_key: &str,
    questions_json: &str,
) -> Result<(), sqlx::Error> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    sqlx::query(
        r#"
        INSERT OR REPLACE INTO extraction_cache (cache_key, questions, created_at)
        VALUES (?, ?, ?)
        "#,
    )
    .bind(cache_key)
    .bind(questions_json)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}


async fn seed_taxonomy(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let seed_data = vec![
        (
            "A Level Mathematics (Edexcel)",
            vec![
                ("Pure", vec![
                    "Proof", "Algebra and functions", "Coordinate geometry in the (x, y) plane", 
                    "Sequences and series", "Trigonometry", "Exponentials and logarithms", 
                    "Differentiation", "Integration", "Numerical methods", "Vectors"
                ]),
                ("Statistics", vec![
                    "Statistical sampling", "Data presentation and interpretation", "Probability", 
                    "Statistical distributions", "Statistical hypothesis testing"
                ]),
                ("Mechanics", vec![
                    "Quantities and units in mechanics", "Kinematics", "Forces and Newton's laws", "Moments"
                ])
            ]
        ),
        (
            "A Level Further Mathematics (Edexcel)",
            vec![
                ("Core Pure", vec![
                    "Complex numbers", "Argand diagrams", "Series", "Roots of polynomials", 
                    "Volumes of revolution", "Matrices", "Linear transformations", 
                    "Proof by induction", "Vectors", "Differential equations", 
                    "Polar coordinates", "Hyperbolic functions", "Maclaurin series", 
                    "Methods in calculus"
                ]),
                ("Further Mechanics 1", vec![
                    "Momentum and impulse", "Work, energy and power", "Elastic strings and springs", 
                    "Elastic collisions in one dimension", "Elastic collisions in two dimensions"
                ]),
                ("Further Statistics 1", vec![
                    "Discrete probability distributions", "Poisson distribution", 
                    "Geometric and negative binomial", "Hypothesis testing", 
                    "Central Limit Theorem", "Chi-squared tests", "Probability generating functions", 
                    "Quality of tests"
                ]),
                ("Further Pure 1", vec![
                    "Vectors (Cross product & planes)", "Conic sections", "Inequalities", 
                    "t-formulae", "Taylor series", "Numerical methods (Further)", 
                    "Reducible differential equations"
                ]),
                ("Decision Mathematics 1", vec![
                    "Algorithms", "Graphs and networks", "Algorithms on graphs", 
                    "Route inspection", "Travelling Salesperson Problem", 
                    "Linear programming", "Simplex algorithm"
                ]),
                ("Further Pure 2", vec![
                    "Number theory", "Groups", "Further calculus", "Further matrix algebra", 
                    "Further complex numbers", "Maclaurin series"
                ]),
                ("Further Mechanics 2", vec![
                    "Circular motion", "Centres of mass of plane figures", "Further centres of mass", 
                    "Kinematics", "Dynamics"
                ]),
                ("Further Statistics 2", vec![
                    "Linear regression", "Continuous probability distributions", 
                    "Correlation", "Hypothesis testing"
                ]),
                ("Decision Mathematics 2", vec![
                    "Transportation problems", "Allocation (assignment) problems", "Flows in networks", 
                    "Dynamic programming", "Game theory", "Recurrence relations", "Decision analysis"
                ])
            ]
        ),
        (
            "GCSE Mathematics (Edexcel)",
            vec![
                ("GCSE Mathematics", vec![
                    "Number", "Algebra", "Ratio, proportion and rates of change", 
                    "Geometry and measures", "Probability", "Statistics"
                ])
            ]
        ),
        (
            "GCSE Further Mathematics (AQA)",
            vec![
                ("GCSE Further Mathematics", vec![
                    "Number", "Algebra", "Coordinate Geometry", "Calculus", 
                    "Matrix Transformations", "Geometry"
                ])
            ]
        )
    ];

    for (subject_name, modules) in seed_data {
        let subject_id = uuid::Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO subjects (id, name) VALUES (?, ?)")
            .bind(&subject_id)
            .bind(subject_name)
            .execute(pool).await?;

        for (module_name, topics) in modules {
            let module_id = uuid::Uuid::new_v4().to_string();
            sqlx::query("INSERT INTO modules (id, subject_id, name) VALUES (?, ?, ?)")
                .bind(&module_id)
                .bind(&subject_id)
                .bind(module_name)
                .execute(pool).await?;

            for topic_name in topics {
                let topic_id = uuid::Uuid::new_v4().to_string();
                sqlx::query("INSERT INTO topics (id, module_id, name) VALUES (?, ?, ?)")
                    .bind(&topic_id)
                    .bind(&module_id)
                    .bind(topic_name)
                    .execute(pool).await?;
            }
        }
    }

    Ok(())
}

// ── Import Cost Log Helpers ──────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ImportCostRecord {
    pub id: String,
    pub paper_name: String,
    pub model_name: String,
    pub paper_type: String,
    pub questions_count: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub cost_usd: f64,
    pub duration_ms: i64,
    pub created_at: i64,
    /// JSON array of per-stage {stage, promptTokens, completionTokens};
    /// `None` for rows logged before Phase 2.
    #[sqlx(default)]
    pub stage_breakdown: Option<String>,
}

pub async fn record_import_cost(
    pool: &SqlitePool,
    paper_name: &str,
    model_name: &str,
    paper_type: &str,
    questions_count: i64,
) -> Result<ImportCostRecord, sqlx::Error> {
    // A local import: no tokens, no cost (the columns stay for the records
    // earlier versions wrote).
    let (prompt_tokens, completion_tokens, cost_usd, duration_ms, stage_breakdown): (i64, i64, f64, i64, Option<&str>) =
        (0, 0, 0.0, 0, None);
    let id = uuid::Uuid::new_v4().to_string();
    let total_tokens = prompt_tokens + completion_tokens;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    sqlx::query(
        r#"
        INSERT INTO import_cost_logs (
            id, paper_name, model_name, paper_type,
            questions_count, prompt_tokens, completion_tokens, total_tokens,
            cost_usd, duration_ms, created_at, stage_breakdown
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(paper_name)
    .bind(model_name)
    .bind(paper_type)
    .bind(questions_count)
    .bind(prompt_tokens)
    .bind(completion_tokens)
    .bind(total_tokens)
    .bind(cost_usd)
    .bind(duration_ms)
    .bind(now)
    .bind(stage_breakdown)
    .execute(pool)
    .await?;

    Ok(ImportCostRecord {
        id,
        paper_name: paper_name.to_string(),
        model_name: model_name.to_string(),
        paper_type: paper_type.to_string(),
        questions_count,
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cost_usd,
        duration_ms,
        created_at: now,
        stage_breakdown: stage_breakdown.map(str::to_string),
    })
}

pub async fn prune_orphaned_import_logs(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    // If no questions exist in the repository at all, clear all import logs
    let (total_q,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM questions")
        .fetch_one(pool)
        .await?;

    if total_q == 0 {
        let res = sqlx::query("DELETE FROM import_cost_logs")
            .execute(pool)
            .await?;
        return Ok(res.rows_affected());
    }

    // Delete logs for papers that have no questions remaining in the repository.
    // Handles both standard paper names and mark scheme prefix "MS:".
    let res = sqlx::query(
        r#"
        DELETE FROM import_cost_logs
        WHERE id IN (
            SELECT l.id FROM import_cost_logs l
            WHERE NOT EXISTS (
                SELECT 1 FROM questions q
                WHERE q.paper_name = l.paper_name
                   OR (l.paper_name LIKE 'MS:%' AND q.paper_name = SUBSTR(l.paper_name, 4))
            )
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(res.rows_affected())
}

pub async fn clear_import_cost_history(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    let res = sqlx::query("DELETE FROM import_cost_logs")
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

pub async fn delete_import_cost_log(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
    let res = sqlx::query("DELETE FROM import_cost_logs WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn get_import_cost_history(
    pool: &SqlitePool,
) -> Result<Vec<ImportCostRecord>, sqlx::Error> {
    // Always prune logs of deleted papers / empty repo first
    let _ = prune_orphaned_import_logs(pool).await;

    let rows = sqlx::query_as::<_, ImportCostRecord>(
        r#"
        SELECT
            id, paper_name, model_name, paper_type,
            questions_count, prompt_tokens, completion_tokens, total_tokens,
            cost_usd, duration_ms, created_at, stage_breakdown
        FROM import_cost_logs
        ORDER BY created_at DESC
        LIMIT 100
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

