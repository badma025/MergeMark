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

    // ── Billing / usage_config table ───────────────────────────────────────
    // A single-row table (id = 1) that tracks the beta-launch hybrid billing
    // model:
    //   * `free_uploads_used`  — count of successful 200 OK responses through
    //                            the OpenRouter free tier (Gemini 2.5 Flash).
    //                            Capped at 3 by the Tauri command; we never
    //                            auto-reset it here.
    //   * `byok_api_key`       — user-supplied personal LLM key. When present
    //                            the OpenRouter free-tier route is bypassed
    //                            entirely and requests go direct to the
    //                            upstream provider.
    //   * `byok_base_url`      — optional override of the LLM base URL used
    //                            when the user supplies a BYOK key. Defaults
    //                            to OpenAI's compatible endpoint if NULL.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS usage_config (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            free_uploads_used INTEGER NOT NULL DEFAULT 0,
            byok_api_key TEXT,
            byok_base_url TEXT,
            updated_at INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .execute(&pool)
    .await?;

    // Make sure the singleton row exists. INSERT OR IGNORE is safe across
    // re-opens because of the CHECK (id = 1) primary key.
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO usage_config (id, free_uploads_used, byok_api_key, byok_base_url, updated_at)
        VALUES (1, 0, NULL, NULL, ?);
        "#,
    )
    .bind(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    )
    .execute(&pool)
    .await?;

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

    // Seed taxonomy if completely empty
    let count: (i64,) = sqlx::query_as("SELECT count(*) FROM subjects").fetch_one(&pool).await?;
    if count.0 == 0 {
        seed_taxonomy(&pool).await?;
    }

    Ok(pool)
}

// ── usage_config helpers ─────────────────────────────────────────────────────
//
// These are the only sanctioned entry points for the rest of the crate to
// touch the billing table. Keeping the SQL in one place means the schema can
// evolve without grepping the whole codebase.

/// Free-tier ceiling. Once `free_uploads_used >= FREE_UPLOAD_LIMIT` the
/// command will refuse to route through the OpenRouter free tier.
pub const FREE_UPLOAD_LIMIT: i64 = 3;

/// Read the current `free_uploads_used` counter.
pub async fn get_free_uploads_used(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as("SELECT free_uploads_used FROM usage_config WHERE id = 1")
        .fetch_one(pool)
        .await?;
    Ok(row.0)
}

/// Increment `free_uploads_used` by one. Returns the new value.
pub async fn increment_free_uploads(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    sqlx::query(
        r#"
        UPDATE usage_config
        SET free_uploads_used = free_uploads_used + 1,
            updated_at = ?
        WHERE id = 1
        "#,
    )
    .bind(now)
    .execute(pool)
    .await?;
    get_free_uploads_used(pool).await
}

/// Read the user-supplied BYOK key. `None` means no key is stored.
pub async fn get_byok_api_key(pool: &SqlitePool) -> Result<Option<String>, sqlx::Error> {
    let row: (Option<String>,) =
        sqlx::query_as("SELECT byok_api_key FROM usage_config WHERE id = 1")
            .fetch_one(pool)
            .await?;
    Ok(row.0.and_then(|s| {
        let trimmed = s.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }))
}

/// Read the optional BYOK base URL override.
pub async fn get_byok_base_url(pool: &SqlitePool) -> Result<Option<String>, sqlx::Error> {
    let row: (Option<String>,) = sqlx::query_as("SELECT byok_base_url FROM usage_config WHERE id = 1")
        .fetch_one(pool)
        .await?;
    Ok(row.0.and_then(|s| {
        let trimmed = s.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }))
}

/// Persist (or clear) the user's BYOK key. Empty/whitespace strings clear it.
pub async fn set_byok_api_key(
    pool: &SqlitePool,
    key: Option<&str>,
    base_url: Option<&str>,
) -> Result<(), sqlx::Error> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let key = key.map(|k| k.trim()).filter(|k| !k.is_empty());
    let base_url = base_url.map(|b| b.trim()).filter(|b| !b.is_empty());
    sqlx::query(
        r#"
        UPDATE usage_config
        SET byok_api_key = ?,
            byok_base_url = ?,
            updated_at = ?
        WHERE id = 1
        "#,
    )
    .bind(key)
    .bind(base_url)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

// ── Extraction cache helpers ─────────────────────────────────────────────
//
// Content-addressed cache for completed extraction results. When a user
// re-ingests the same PDF with the same model, the pipeline is skipped
// entirely and cached questions are returned instantly.
//
// Bump EXTRACTION_CACHE_VERSION when the extraction logic, validation
// rules, or prompt templates change in a way that would produce different
// output for the same input.

/// Bump this to invalidate all cached extraction results after logic changes.
pub const EXTRACTION_CACHE_VERSION: u32 = 2;

/// Compute a deterministic cache key from file content + parameters.
/// Uses a fast FNV-1a hash of the file bytes (not cryptographic, just
/// collision-resistant enough for a local cache).
pub fn extraction_cache_key(
    file_bytes: &[u8],
    model: &str,
    paper_name: &str,
) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    file_bytes.hash(&mut hasher);
    model.hash(&mut hasher);
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
}

pub async fn record_import_cost(
    pool: &SqlitePool,
    paper_name: &str,
    model_name: &str,
    paper_type: &str,
    questions_count: i64,
    prompt_tokens: i64,
    completion_tokens: i64,
    cost_usd: f64,
    duration_ms: i64,
) -> Result<ImportCostRecord, sqlx::Error> {
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
            cost_usd, duration_ms, created_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
            cost_usd, duration_ms, created_at
        FROM import_cost_logs
        ORDER BY created_at DESC
        LIMIT 100
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

