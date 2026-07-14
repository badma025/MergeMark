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
            topics TEXT
        );
        "#
    )
    .execute(&pool)
    .await?;

    // Migrate existing table by adding the new column. Ignore error if it already exists.
    let _ = sqlx::query("ALTER TABLE questions ADD COLUMN answer_content TEXT")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE questions ADD COLUMN topics TEXT")
        .execute(&pool)
        .await;

    // 5. Seed the database with mock data if it's empty
    seed_database_if_empty(&pool).await?;

    Ok(pool)
}

async fn seed_database_if_empty(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM questions")
        .fetch_one(pool)
        .await?;

    if count.0 == 0 {
        let mock_data = vec![
            ("q1", "Mathematics", "Calculus", "[\"Differentiation\"]", 4, "Find the derivative of f(x) with respect to x, and determine all critical points in the interval [0, 2π].", "f(x) = 3x³ - 2sin(x) + e^(2x)", false),
            ("q2", "Physics", "Mechanics", "[]", 6, "A particle of mass m moves under a conservative force. Show that the total mechanical energy is conserved and find the equilibrium positions.", "F(x) = -dV/dx,   V(x) = ½kx² - mgx", false),
            ("q3", "Computer Science", "Algorithms", "[]", 3, "Analyse the time complexity of the following recursive function and express your answer using Big-O notation.", "def fib(n):\n    if n <= 1:\n        return n\n    return fib(n-1) + fib(n-2)", true),
            ("q4", "Mathematics", "Statistics", "[\"Statistical distributions\"]", 5, "Given a normal distribution X ~ N(μ, σ²), find the probability P(X > 72) given that μ = 65 and σ = 8.", "P(X > 72) = P(Z > (72 - μ) / σ)", false),
            ("q5", "Chemistry", "Thermodynamics", "[]", 4, "Calculate the Gibbs free energy change for the reaction at 298 K. State whether the reaction is spontaneous.", "ΔG° = ΔH° - TΔS°\n     = -120 kJ - (298)(0.250 kJ/K)", false),
            ("q6", "Computer Science", "Data Structures", "[]", 3, "Write a function that reverses a singly linked list in-place and returns the new head node. Analyse its space complexity.", "def reverse(head):\n    prev = None\n    curr = head\n    while curr:\n        nxt = curr.next\n        curr.next = prev\n        prev, curr = curr, nxt\n    return prev", true),
        ];

        for (id, subject, subtopic, topics, marks, content, math_snippet, is_code) in mock_data {
            sqlx::query(
                r#"
                INSERT INTO questions (id, subject, subtopic, topics, marks, content, math_snippet, is_code)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(id)
            .bind(subject)
            .bind(subtopic)
            .bind(topics)
            .bind(marks)
            .bind(content)
            .bind(math_snippet)
            .bind(is_code)
            .execute(pool)
            .await?;
        }
    }

    Ok(())
}
