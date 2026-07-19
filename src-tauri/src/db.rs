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
        .create_if_missing(true);

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
            module TEXT
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

