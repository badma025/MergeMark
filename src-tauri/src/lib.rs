use sqlx::SqlitePool;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::Mutex;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};
use std::sync::OnceLock;

/// Shared application state — holds the SQLite connection pool.
/// Wrapped in Arc<Mutex<>> so it can safely be accessed from concurrent Tauri commands.
mod db;

static TRACING_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();

mod backup;
mod billing;
mod commands;
mod doc_map;
mod geometry;
mod json_salvage;
mod llm;
mod pdfium;
mod pdf_render;
mod pipeline;
mod taxonomy;
mod validate;
pub mod marker_client;

pub struct AppState {
    pub db: Arc<Mutex<SqlitePool>>,
    pub cancel_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Concurrency guard for the hybrid-billing command. Only one
    /// `generate_worksheet_from_pdf` call may be in flight at a time; the
    /// command attempts a non-blocking `try_lock` and rejects overlapping
    /// calls with a 429-style `BillingError` instead of queueing them.
    pub extraction_in_progress: tokio::sync::Mutex<()>,
}

// ---------------------------------------------------------------------------
// App entry point
// ---------------------------------------------------------------------------

fn init_tracing() {
    // Initialize tracing with JSON file output for production
    let file_appender = tracing_appender::rolling::daily("logs", "mergemark.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // Store the guard to keep the file appender alive for the app lifetime
    let _ = TRACING_GUARD.set(guard);

    // EnvFilter: default to INFO, but allow RUST_LOG to override
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(
            fmt::layer()
                .json()
                .with_current_span(true)
                .with_span_list(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_writer(non_blocking),
        )
        .try_init();
    // Ignore error if already initialized (tests may run multiple times)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize tracing early
    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_handle = app.handle().clone();

            // Resolve a writable path for the SQLite database inside the OS app-data directory.
            let data_dir = app_handle
                .path()
                .app_data_dir()
                .expect("Failed to resolve app data directory");

            // Initialise the connection pool and run migrations on a blocking thread
            let pool = tauri::async_runtime::block_on(async {
                db::init_db(data_dir)
                    .await
                    .expect("Failed to initialize SQLite database")
            });

            app.manage(AppState {
                db: Arc::new(Mutex::new(pool)),
                cancel_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                extraction_in_progress: tokio::sync::Mutex::new(()),
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            backup::export_backup,
            backup::preview_backup,
            backup::import_backup,
            commands::get_all_questions,
            commands::add_question,
            commands::delete_question,
            commands::delete_all_questions,
            commands::delete_questions_by_paper,
            commands::import_questions,
            commands::compile_worksheet,
            commands::parse_pdf,
            commands::parse_pdf_vision,
            commands::parse_mark_scheme_vision,
            commands::fetch_models,
            commands::update_question,
            commands::commit_mark_schemes,
            commands::mark_question_verified,
            commands::get_paper_names,
            commands::cancel_import,
            commands::generate_worksheet_from_pdf,
            commands::export_worksheet_markdown,
            commands::get_usage_status,
            commands::set_byok_key,
            commands::export_flashcards,
            commands::import_flashcards,
            taxonomy::get_taxonomy_tree,
            taxonomy::add_subject,
            taxonomy::rename_subject,
            taxonomy::delete_subject,
            taxonomy::add_module,
            taxonomy::rename_module,
            taxonomy::delete_module,
            taxonomy::add_topic,
            taxonomy::rename_topic,
            taxonomy::delete_topic,
            commands::generate_topics_for_module
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
