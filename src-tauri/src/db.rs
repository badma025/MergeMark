use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Why the library could not be opened, and the newest backup of it there is
/// (`mergemark.db.bak-<version>`, written when a version first opens it).
#[derive(Debug)]
pub struct InitFailure {
    pub message: String,
    pub backup: Option<PathBuf>,
}

/// The library's schema version (`PRAGMA user_version`). Version 1 ran the
/// one-off repairs of libraries written before 1.0 (`repair_old_library`):
/// they run once, so a card edited by hand is never rewritten at a later
/// launch.
const SCHEMA_VERSION: i64 = 1;

pub async fn init_db(app_data_dir: PathBuf) -> Result<SqlitePool, InitFailure> {
    let fail = |message: String| InitFailure { message, backup: newest_backup(&app_data_dir) };
    std::fs::create_dir_all(&app_data_dir).map_err(|e| fail(format!("its folder {} could not be created: {e}", app_data_dir.display())))?;
    let db_path = app_data_dir.join("mergemark.db");
    let existed = db_path.exists();
    open_library(&db_path, existed).await.map_err(|e| fail(e.to_string()))
}

/// The newest backup beside the library (`mergemark.db.bak-*`).
pub fn newest_backup(app_data_dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(app_data_dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("mergemark.db.bak-"))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
}

/// The backup a version writes when it first opens a library.
pub fn backup_path(db_path: &Path) -> PathBuf {
    db_path.with_file_name(format!("mergemark.db.bak-{}", env!("CARGO_PKG_VERSION")))
}

async fn open_library(db_path: &Path, existed: bool) -> Result<SqlitePool, sqlx::Error> {
    // Using `mode=rwc` ensures the file is created if it doesn't exist
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());

    let options = SqliteConnectOptions::from_str(&db_url)?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_secs(5))
        // What is deleted is overwritten, not left in free pages.
        .pragma("secure_delete", "ON");
    let pool = SqlitePool::connect_with(options).await?;

    // Earlier versions kept an AI provider's API key here in plain text
    // (usage_config) and a log of AI spend (import_cost_logs). Nothing reads
    // either: both are erased, before any backup can copy them.
    sqlx::query("DROP TABLE IF EXISTS usage_config").execute(&pool).await?;
    sqlx::query("DROP TABLE IF EXISTS import_cost_logs").execute(&pool).await?;
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)").execute(&pool).await?;

    // A copy of the library as it was before this version first changed it.
    // (A failed copy is reported and the library still opens: every change
    // below adds, or sets aside, and deletes nothing.)
    if existed {
        let backup = backup_path(db_path);
        if !backup.exists() {
            if let Err(e) = sqlx::query("VACUUM INTO ?").bind(backup.to_string_lossy().into_owned()).execute(&pool).await {
                eprintln!("[DB] could not back up the library to {}: {e}", backup.display());
            }
        }
    }
    let version: i64 = sqlx::query_scalar("PRAGMA user_version").fetch_one(&pool).await?;

    // Run the migration to create the questions table if it doesn't exist
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
    // The paper a card was imported from (path, SHA-256 of its bytes) and
    // the pages its question is on, 1-based: what the card is checked
    // against (source_pages.rs).
    for column in ["source_path TEXT", "source_sha256 TEXT", "source_first_page INTEGER", "source_last_page INTEGER"] {
        let _ = sqlx::query(&format!("ALTER TABLE questions ADD COLUMN {column}")).execute(&pool).await;
    }

    // NULL sanitization
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

    if version < 1 {
        repair_old_library(&pool).await?;
    }

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

    // Cached imports of an earlier extraction version are never read again
    // (the version is part of their key): they go when the version changes.
    sqlx::query("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)").execute(&pool).await?;
    let cached: Option<String> = sqlx::query_scalar("SELECT value FROM meta WHERE key = 'extraction_cache_version'").fetch_optional(&pool).await?;
    if cached.as_deref() != Some(EXTRACTION_CACHE_VERSION.to_string().as_str()) {
        sqlx::query("DELETE FROM extraction_cache").execute(&pool).await?;
        sqlx::query("INSERT OR REPLACE INTO meta (key, value) VALUES ('extraction_cache_version', ?)")
            .bind(EXTRACTION_CACHE_VERSION.to_string())
            .execute(&pool)
            .await?;
    }

    // Seed taxonomy if completely empty
    let count: (i64,) = sqlx::query_as("SELECT count(*) FROM subjects").fetch_one(&pool).await?;
    if count.0 == 0 {
        seed_taxonomy(&pool).await?;
    }

    sqlx::query(&format!("PRAGMA user_version = {SCHEMA_VERSION}")).execute(&pool).await?;
    Ok(pool)
}

/// The one-off repairs of a library written before 1.0 (schema version 0).
async fn repair_old_library(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    // The math snippet is no longer kept: the card's content holds it all.
    sqlx::query("UPDATE questions SET math_snippet = '' WHERE math_snippet IS NULL OR math_snippet != ''").execute(pool).await?;

    // Heal polar equations and unbalanced delimiters written by the old
    // AI pipeline.
    let rows = sqlx::query("SELECT id, content FROM questions WHERE content LIKE '%polar%' OR content LIKE '%cardioid%' OR content LIKE '%theta%' OR content LIKE '%cos%' OR content LIKE '%sin%'")
        .fetch_all(pool)
        .await?;
    {
        use sqlx::Row;
        for row in rows {
            if let (Ok(id), Ok(content)) = (row.try_get::<String, _>("id"), row.try_get::<String, _>("content")) {
                let healed = crate::validate::heal_polar_equations(&content);
                if healed != content {
                    sqlx::query("UPDATE questions SET content = ? WHERE id = ?").bind(healed).bind(id).execute(pool).await?;
                }
            }
        }
    }

    // Before the unique index can exist, duplicate (paper_name,
    // question_number) rows from older builds collapse to the most recently
    // written one. The others are set aside in questions_superseded, not
    // deleted: one may hold a user's edits (and the backup holds them all).
    let duplicate = "trim(COALESCE(paper_name, '')) != '' AND question_number IS NOT NULL AND rowid NOT IN (\
        SELECT MAX(rowid) FROM questions WHERE trim(COALESCE(paper_name, '')) != '' AND question_number IS NOT NULL \
        GROUP BY paper_name, question_number)";
    let mut tx = pool.begin().await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS questions_superseded AS SELECT * FROM questions WHERE 0").execute(&mut *tx).await?;
    let set_aside = sqlx::query(&format!("INSERT INTO questions_superseded SELECT * FROM questions WHERE {duplicate}")).execute(&mut *tx).await?;
    sqlx::query(&format!("DELETE FROM questions WHERE {duplicate}")).execute(&mut *tx).await?;
    tx.commit().await?;
    if set_aside.rows_affected() > 0 {
        eprintln!("[DB] {} duplicate question rows set aside in questions_superseded", set_aside.rows_affected());
    }
    Ok(())
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
/// v8: 1.0 W1 round 2 (stems shared by several questions, a first line set
/// beside a bare question number, an operator before a radical read as its
/// index) and W3's first fixes (WJEC's "a)" part labels and raised decimal
/// points, a question opening with a figure, a bracket against the word
/// before it read as marks, MathTechnical's and MT Extra's symbols, units
/// repaired inside maths): a cached v7 card can lack its stem, hold the next
/// question's, or hold one of those defects.
/// v9: 1.0 W1 round 3 (AES-encrypted papers' fonts read, Advent's and MT
/// Extra's symbols, a superscript at the text's size, an integral's fraction
/// limit, glyphs clipped out of sight or white on white, an expression set in
/// its label row's gap, an enlarged sign on its own row, an allocation beside
/// a drawing, a paper with no question numbers, array indices in code, a
/// drawing of boxes and arrows): a cached v8 card can hold one of those
/// defects.
/// v10: 1.0 W1 round 4 (a script left behind when its row's items moved one
/// by one, a drawing or a dimension line beside a figure, a detector box over
/// two drawings): a cached v9 card can hold one of those defects.
pub const EXTRACTION_CACHE_VERSION: u32 = 10;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir() -> PathBuf {
        std::env::temp_dir().join(format!("mm_db_{}", uuid::Uuid::new_v4()))
    }

    const OLD_KEY: &str = "sk-or-v1-plain-text-key-0123456789abcdef";

    /// A library written before 1.0: an AI provider key in plain text, a log
    /// of AI spend, and one question imported twice (the older row edited).
    async fn old_library(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        let url = format!("sqlite://{}?mode=rwc", dir.join("mergemark.db").display());
        let pool = SqlitePool::connect(&url).await.unwrap();
        for sql in [
            "CREATE TABLE questions (id TEXT PRIMARY KEY, subject TEXT DEFAULT 'Mathematics' NOT NULL, subtopic TEXT NOT NULL, marks INTEGER NOT NULL, content TEXT NOT NULL, math_snippet TEXT NOT NULL, is_code BOOLEAN NOT NULL, paper_name TEXT DEFAULT '', question_number INTEGER)",
            "INSERT INTO questions VALUES ('edited', 'Physics', '', 3, 'Edited by hand.', '', 0, 'Paper', 1)",
            "INSERT INTO questions VALUES ('again', 'Physics', '', 3, 'Imported again.', '', 0, 'Paper', 1)",
            "CREATE TABLE usage_config (id INTEGER PRIMARY KEY, byok_api_key TEXT)",
            "CREATE TABLE import_cost_logs (id TEXT PRIMARY KEY, paper_name TEXT)",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO usage_config VALUES (1, ?)").bind(OLD_KEY).execute(&pool).await.unwrap();
        pool.close().await;
    }

    fn file_holds(path: &Path, needle: &str) -> bool {
        std::fs::read(path).is_ok_and(|bytes| bytes.windows(needle.len()).any(|w| w == needle.as_bytes()))
    }

    /// The first launch of a version backs the library up, once, after
    /// erasing the old AI key, and sets aside (never deletes) a duplicate.
    #[tokio::test]
    async fn an_old_library_is_backed_up_once_without_its_ai_key() {
        let dir = scratch_dir();
        old_library(&dir).await;
        let db_path = dir.join("mergemark.db");
        let pool = init_db(dir.clone()).await.unwrap();
        let tables: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE name IN ('usage_config', 'import_cost_logs')").fetch_one(&pool).await.unwrap();
        assert_eq!(tables, 0);
        let kept: Vec<String> = sqlx::query_scalar("SELECT id FROM questions").fetch_all(&pool).await.unwrap();
        assert_eq!(kept, ["again"]);
        let aside: Vec<String> = sqlx::query_scalar("SELECT content FROM questions_superseded").fetch_all(&pool).await.unwrap();
        assert_eq!(aside, ["Edited by hand."]);
        let version: i64 = sqlx::query_scalar("PRAGMA user_version").fetch_one(&pool).await.unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        pool.close().await;

        let backup = backup_path(&db_path);
        assert!(backup.exists(), "no backup at {}", backup.display());
        assert_eq!(newest_backup(&dir).as_deref(), Some(backup.as_path()));
        for file in [db_path.clone(), backup.clone(), dir.join("mergemark.db-wal")] {
            assert!(!file_holds(&file, OLD_KEY), "{} still holds the old key", file.display());
        }
        // The backup is the library before this version changed it.
        let copy = SqlitePool::connect(&format!("sqlite://{}?mode=ro", backup.display())).await.unwrap();
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM questions ORDER BY id").fetch_all(&copy).await.unwrap();
        assert_eq!(ids, ["again", "edited"]);
        copy.close().await;

        // A later launch of the same version writes no second backup.
        let written = std::fs::metadata(&backup).unwrap().modified().unwrap();
        init_db(dir.clone()).await.unwrap().close().await;
        assert_eq!(std::fs::metadata(&backup).unwrap().modified().unwrap(), written);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The one-off repairs of old libraries run once: a card edited by hand
    /// afterwards is never rewritten at a later launch.
    #[tokio::test]
    async fn a_card_is_not_rewritten_at_a_later_launch() {
        let content = "The curve C has polar equation\n2\\cos\\theta + 1$.";
        assert_ne!(crate::validate::heal_polar_equations(content), content, "the old repair would rewrite this card");
        let dir = scratch_dir();
        let pool = init_db(dir.clone()).await.unwrap();
        sqlx::query("INSERT INTO questions (id, subtopic, marks, content, math_snippet, is_code, paper_name, question_number) VALUES ('q', '', 2, ?, '', 0, 'Paper', 1)")
            .bind(content)
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        let pool = init_db(dir.clone()).await.unwrap();
        let stored: String = sqlx::query_scalar("SELECT content FROM questions WHERE id = 'q'").fetch_one(&pool).await.unwrap();
        assert_eq!(stored, content);
        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A library that cannot be opened is reported with its newest backup.
    #[tokio::test]
    async fn a_library_that_cannot_be_opened_names_its_backup() {
        let dir = scratch_dir();
        std::fs::create_dir_all(dir.join("mergemark.db")).unwrap(); // a folder where the file should be
        std::fs::write(dir.join("mergemark.db.bak-0.9.4"), b"").unwrap();
        let failure = init_db(dir.clone()).await.expect_err("a folder is no library");
        assert_eq!(failure.backup, Some(dir.join("mergemark.db.bak-0.9.4")));
        assert!(!failure.message.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Cached imports of an earlier extraction version go when it changes.
    #[tokio::test]
    async fn cached_imports_of_an_earlier_version_are_swept() {
        let dir = scratch_dir();
        let pool = init_db(dir.clone()).await.unwrap();
        store_cached_extraction(&pool, "old-key", "[]").await.unwrap();
        sqlx::query("UPDATE meta SET value = '1' WHERE key = 'extraction_cache_version'").execute(&pool).await.unwrap();
        pool.close().await;
        let pool = init_db(dir.clone()).await.unwrap();
        assert_eq!(get_cached_extraction(&pool, "old-key").await.unwrap(), None);
        store_cached_extraction(&pool, "new-key", "[]").await.unwrap();
        pool.close().await;
        let pool = init_db(dir.clone()).await.unwrap();
        assert_eq!(get_cached_extraction(&pool, "new-key").await.unwrap().as_deref(), Some("[]"));
        pool.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
