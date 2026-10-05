use sqlx::SqlitePool;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::Mutex;

/// Shared application state — holds the SQLite connection pool.
/// Wrapped in Arc<Mutex<>> so it can safely be accessed from concurrent Tauri commands.
mod db;

mod backup;
mod commands;
mod deterministic;
mod doc_map;
mod geometry;
pub mod layout;
pub mod mark_scheme_deterministic;
pub mod ocr;
pub mod pdf_render;
pub mod pipeline;
pub mod sanitize;
mod source_pages;
pub(crate) mod stroke_census;
mod taxonomy;
#[cfg(test)]
mod test_support;
pub mod validate;
pub mod marker_client;

pub struct AppState {
    pub db: Arc<Mutex<SqlitePool>>,
    pub cancel_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Only one import runs at a time: an import attempts a non-blocking
    /// `try_lock` and rejects an overlapping one instead of queueing it.
    pub extraction_in_progress: tokio::sync::Mutex<()>,
}

// ---------------------------------------------------------------------------
// App entry point
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init());
    // Updates from the project's GitHub releases, and the restart into one.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build()).plugin(tauri_plugin_process::init());
    builder
        .setup(|app| {
            let app_handle = app.handle().clone();

            // Resolve a writable path for the SQLite database inside the OS app-data directory.
            let data_dir = app_handle
                .path()
                .app_data_dir()
                .expect("Failed to resolve app data directory");

            // Initialise the connection pool and run migrations on a blocking thread
            let pool = match tauri::async_runtime::block_on(db::init_db(data_dir.clone())) {
                Ok(pool) => pool,
                Err(failure) => {
                    // Say why, and where the backup is, instead of panicking.
                    // (Blocking on the dialog here would deadlock: it runs on
                    // this thread's event loop.)
                    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.hide();
                    }
                    let mut message = format!(
                        "MergeMark could not open your library at {}:\n\n{}",
                        data_dir.join("mergemark.db").display(),
                        failure.message
                    );
                    if let Some(backup) = &failure.backup {
                        message.push_str(&format!("\n\nA copy of your library from before this version first opened it is at {}.", backup.display()));
                    }
                    app.dialog()
                        .message(message)
                        .title("MergeMark could not start")
                        .kind(MessageDialogKind::Error)
                        .show(|_| std::process::exit(1));
                    return Ok(());
                }
            };

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
            commands::import_paper,
            commands::import_scheme,
            commands::update_question,
            commands::commit_mark_schemes,
            commands::mark_question_verified,
            commands::show_source_pages,
            commands::relocate_source,
            commands::get_paper_names,
            commands::cancel_import,
            commands::export_worksheet_markdown,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
