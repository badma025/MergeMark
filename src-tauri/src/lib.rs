use sqlx::SqlitePool;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::Mutex;

/// Shared application state — holds the SQLite connection pool.
/// Wrapped in Arc<Mutex<>> so it can safely be accessed from concurrent Tauri commands.
mod db;

mod commands;
mod doc_map;
mod geometry;
mod json_salvage;
mod llm;
mod pipeline;
mod validate;

pub struct AppState {
    pub db: Arc<Mutex<SqlitePool>>,
    pub cancel_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

// ---------------------------------------------------------------------------
// App entry point
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
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
            commands::get_paper_names,
            commands::cancel_import
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
