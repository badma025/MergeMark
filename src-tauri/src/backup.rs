//! Backup & restore: export the question library (rows + diagram images) to a
//! single zip archive, and import it back with merge/replace semantics.
//!
//! Archive layout:
//!   manifest.json   — formatVersion, app version, export time, counts
//!   questions.json  — JSON array of `Question` rows; diagram references inside
//!                     `content` / `answer_content` rewritten from absolute local
//!                     paths to archive-relative `images/<name>` so the backup is
//!                     portable across machines and OSes
//!   images/<name>   — the referenced diagram PNGs, included exactly once each

use crate::commands::Question;
use crate::AppState;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

const FORMAT_VERSION: u32 = 1;

// ── IPC report types ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    format_version: u32,
    app_version: String,
    exported_at: u64,
    question_count: usize,
    image_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupPreview {
    format_version: u32,
    app_version: String,
    exported_at: u64,
    question_count: usize,
    image_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    questions: usize,
    images: usize,
    missing_images: usize,
    path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    added: i64,
    updated: i64,
    images_copied: usize,
    replaced: bool,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn image_ref_regex() -> Regex {
    // Matches Markdown image embeds: ![alt](target)
    Regex::new(r"!\[[^\]]*\]\(([^)\s]+)\)").expect("valid image-ref regex")
}

/// Normalise a path the way `pipeline.rs` embeds it: forward slashes.
fn slashy(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn diagrams_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|d| d.join("diagrams"))
        .map_err(|e| format!("Failed to resolve app data directory: {}", e))
}

/// Rewrite one markdown field for export: swap absolute image paths for
/// archive-relative names, queueing each existing file for inclusion exactly
/// once. Returns the rewritten field; bumps `missing` for broken references.
fn rewrite_for_export(
    text: &str,
    image_map: &mut HashMap<String, String>, // absolute path -> images/<name>
    used_names: &mut std::collections::HashSet<String>,
    missing: &mut usize,
) -> String {
    let re = image_ref_regex();
    let mut out = text.to_string();
    for cap in re.captures_iter(text) {
        let target = cap.get(1).unwrap().as_str();
        if target.starts_with("images/") {
            continue; // already archive-relative (shouldn't happen, but safe)
        }
        let path = PathBuf::from(target);
        if !path.is_file() {
            *missing += 1;
            continue; // leave the reference as-is; report it
        }
        let base = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("{}.png", Uuid::new_v4()));
        let entry_name = image_map.entry(target.to_string()).or_insert_with(|| {
            let mut name = base.clone();
            let mut n = 1u32;
            while !used_names.insert(name.clone()) {
                // basename collision between two different source files
                name = format!("{}-{}", n, base);
                n += 1;
            }
            format!("images/{}", name)
        });
        out = out.replacen(target, entry_name, 1);
    }
    out
}

/// Rewrite one markdown field for import: swap `images/<name>` refs for the
/// freshly-created absolute path of the extracted copy on this machine.
fn rewrite_for_import(text: &str, extracted: &HashMap<String, String>) -> String {
    let mut out = text.to_string();
    for (name, new_abs) in extracted {
        out = out.replace(&format!("images/{}", name), new_abs);
    }
    out
}

// ── Commands ──────────────────────────────────────────────────────────────────

/// Reads a backup archive and returns its manifest + question count without
/// touching the database. The UI uses this to show a confirmation dialog.
#[tauri::command]
pub async fn preview_backup(src_path: String) -> Result<BackupPreview, String> {
    let file =
        std::fs::File::open(&src_path).map_err(|e| format!("Could not open backup file: {}", e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| "That file is not a valid MergeMark backup (not a zip archive)".to_string())?;

    let mut manifest_file = archive
        .by_name("manifest.json")
        .map_err(|_| "That file is not a valid MergeMark backup (missing manifest)".to_string())?;
    let mut manifest_str = String::new();
    manifest_file
        .read_to_string(&mut manifest_str)
        .map_err(|e| format!("Backup manifest is unreadable: {}", e))?;
    drop(manifest_file);
    let manifest: Manifest = serde_json::from_str(&manifest_str)
        .map_err(|_| "That file is not a valid MergeMark backup (corrupt manifest)".to_string())?;

    if manifest.format_version > FORMAT_VERSION {
        return Err(format!(
            "This backup was made by a newer version of MergeMark (backup format v{}, this app understands up to v{}). Please update the app first.",
            manifest.format_version, FORMAT_VERSION
        ));
    }

    let questions: Vec<serde_json::Value> = {
        let mut qf = archive.by_name("questions.json").map_err(|_| {
            "That file is not a valid MergeMark backup (missing questions)".to_string()
        })?;
        let mut qs = String::new();
        qf.read_to_string(&mut qs)
            .map_err(|e| format!("Backup questions are unreadable: {}", e))?;
        serde_json::from_str(&qs).map_err(|_| {
            "That file is not a valid MergeMark backup (corrupt questions)".to_string()
        })?
    };

    Ok(BackupPreview {
        format_version: manifest.format_version,
        app_version: manifest.app_version,
        exported_at: manifest.exported_at,
        question_count: questions.len(),
        image_count: manifest.image_count,
    })
}

/// Exports every question plus its referenced diagram images to a zip archive
/// at `dest_path`.
#[tauri::command]
pub async fn export_backup(
    app: AppHandle,
    dest_path: String,
    state: State<'_, AppState>,
) -> Result<ExportReport, String> {
    let pool = state.db.lock().await;
    let mut questions = sqlx::query_as::<_, Question>("SELECT * FROM questions")
        .fetch_all(&*pool)
        .await
        .map_err(|e| format!("Failed to read questions: {}", e))?;
    drop(pool);

    // Rewrite image references to archive-relative paths and collect the files.
    let mut image_map: HashMap<String, String> = HashMap::new();
    let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut missing: usize = 0;
    for q in &mut questions {
        q.content = rewrite_for_export(&q.content, &mut image_map, &mut used_names, &mut missing);
        if let Some(ans) = q.answer_content.clone() {
            q.answer_content = Some(rewrite_for_export(
                &ans,
                &mut image_map,
                &mut used_names,
                &mut missing,
            ));
        }
    }

    let dest = PathBuf::from(&dest_path);
    let file = std::fs::File::create(&dest)
        .map_err(|e| format!("Could not create backup file at {}: {}", dest_path, e))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let result: Result<(), String> = (|zip: &mut zip::ZipWriter<std::fs::File>| {
        let manifest = Manifest {
            format_version: FORMAT_VERSION,
            app_version: app.package_info().version.to_string(),
            exported_at: unix_now(),
            question_count: questions.len(),
            image_count: image_map.len(),
        };
        zip.start_file("manifest.json", options)
            .map_err(|e| format!("Failed to start manifest: {}", e))?;
        zip.write_all(
            serde_json::to_string_pretty(&manifest)
                .map_err(|e| format!("Failed to serialise manifest: {}", e))?
                .as_bytes(),
        )
        .map_err(|e| format!("Failed to write manifest: {}", e))?;

        let questions_json = serde_json::to_string_pretty(&questions)
            .map_err(|e| format!("Failed to serialise questions: {}", e))?;
        zip.start_file("questions.json", options)
            .map_err(|e| format!("Failed to add questions to backup: {}", e))?;
        zip.write_all(questions_json.as_bytes())
            .map_err(|e| format!("Failed to write questions: {}", e))?;

        for (abs_path, entry_name) in &image_map {
            let mut f = std::fs::File::open(abs_path)
                .map_err(|e| format!("Failed to read image {}: {}", abs_path, e))?;
            zip.start_file(entry_name.clone(), options)
                .map_err(|e| format!("Failed to add image to backup: {}", e))?;
            std::io::copy(&mut f, zip)
                .map_err(|e| format!("Failed to write image {}: {}", abs_path, e))?;
        }
        Ok(())
    })(&mut zip);

    match result.and_then(|_| {
        zip.finish()
            .map(|_| ())
            .map_err(|e| format!("Failed to finalise backup: {}", e))
    }) {
        Ok(()) => Ok(ExportReport {
            questions: questions.len(),
            images: image_map.len(),
            missing_images: missing,
            path: dest_path,
        }),
        Err(e) => {
            // Don't leave a half-written archive behind.
            let _ = std::fs::remove_file(&dest);
            Err(e)
        }
    }
}

/// Imports a backup archive created by `export_backup`.
///
/// `mode` is "merge" (default) or "replace":
///  - merge:   upserts each question on (paper_name, question_number) — the same
///             composite identity the ingestion pipeline uses. Existing rows are
///             updated in place (keeping their ids), genuinely-new rows are
///             inserted (with a fresh id if the backup's id is already taken).
///  - replace: wipes the whole table first (behind a typed confirmation in the UI).
///
/// Diagram images are extracted to this machine's diagrams directory under fresh
/// names, references in content are rewritten, and all DB writes happen in a
/// single transaction — a bad file can never leave a half-imported library.
#[tauri::command]
pub async fn import_backup(
    app: AppHandle,
    src_path: String,
    mode: String,
    state: State<'_, AppState>,
) -> Result<ImportSummary, String> {
    let replace = match mode.as_str() {
        "merge" => false,
        "replace" => true,
        _ => return Err(format!("Unknown import mode '{}'", mode)),
    };

    // 1. Parse and validate the archive (no DB touched yet).
    let file =
        std::fs::File::open(&src_path).map_err(|e| format!("Could not open backup file: {}", e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| "That file is not a valid MergeMark backup (not a zip archive)".to_string())?;

    let manifest: Manifest = {
        let mut mf = archive.by_name("manifest.json").map_err(|_| {
            "That file is not a valid MergeMark backup (missing manifest)".to_string()
        })?;
        let mut s = String::new();
        mf.read_to_string(&mut s)
            .map_err(|e| format!("Backup manifest is unreadable: {}", e))?;
        serde_json::from_str(&s).map_err(|_| {
            "That file is not a valid MergeMark backup (corrupt manifest)".to_string()
        })?
    };
    if manifest.format_version > FORMAT_VERSION {
        return Err(format!(
            "This backup was made by a newer version of MergeMark (backup format v{}, this app understands up to v{}). Please update the app first.",
            manifest.format_version, FORMAT_VERSION
        ));
    }

    let mut questions: Vec<Question> = {
        let mut qf = archive.by_name("questions.json").map_err(|_| {
            "That file is not a valid MergeMark backup (missing questions)".to_string()
        })?;
        let mut s = String::new();
        qf.read_to_string(&mut s)
            .map_err(|e| format!("Backup questions are unreadable: {}", e))?;
        serde_json::from_str(&s).map_err(|_| {
            "That file is not a valid MergeMark backup (corrupt questions)".to_string()
        })?
    };

    // 2. Extract images to fresh filenames and build the reference rewrite map.
    let dir = diagrams_dir(&app)?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create diagrams directory: {}", e))?;

    let mut extracted: HashMap<String, String> = HashMap::new(); // entry name -> new abs path
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Backup archive is unreadable: {}", e))?;
        let entry_path = match entry.enclosed_name() {
            Some(p) => p,
            None => continue, // skip suspicious paths
        };
        let mut comps = entry_path.components();
        if comps.next().map(|c| c.as_os_str()) != Some(std::ffi::OsStr::new("images")) {
            continue;
        }
        let name = match entry_path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };
        if name.is_empty() || entry_path.components().count() > 2 {
            continue; // only flat images/<name> entries
        }
        let ext = Path::new(&name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_string();
        let new_abs = slashy(&dir.join(format!("{}.{}", Uuid::new_v4(), ext)));
        let mut out = std::fs::File::create(&new_abs)
            .map_err(|e| format!("Failed to restore image {}: {}", name, e))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|e| format!("Failed to restore image {}: {}", name, e))?;
        extracted.insert(name, new_abs);
    }

    for q in &mut questions {
        q.content = rewrite_for_import(&q.content, &extracted);
        if let Some(ans) = q.answer_content.clone() {
            q.answer_content = Some(rewrite_for_import(&ans, &extracted));
        }
    }

    // 3. Write the rows in a single transaction.
    let pool = state.db.lock().await;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| format!("Failed to start import transaction: {}", e))?;

    let result: Result<(i64, i64), String> = async {
        if replace {
            sqlx::query("DELETE FROM questions")
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("Failed to clear existing questions: {}", e))?;
        }

        let mut added = 0i64;
        let mut updated = 0i64;

        for q in &questions {
            let has_composite = !q.paper_name.trim().is_empty() && q.question_number.is_some();

            // Merge mode: if a row with the same (paper_name, question_number)
            // exists, update it in place and keep its existing id.
            if !replace && has_composite {
                let existing_id: Option<String> = sqlx::query_scalar(
                    "SELECT id FROM questions WHERE paper_name = ? AND question_number = ?",
                )
                .bind(&q.paper_name)
                .bind(q.question_number)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| format!("Import failed while checking duplicates: {}", e))?;

                if let Some(id) = existing_id {
                    sqlx::query(
                        r#"
                        UPDATE questions
                        SET subject = ?, subtopic = ?, marks = ?, content = ?,
                            math_snippet = ?, is_code = ?, answer_content = ?,
                            topics = ?, module = ?
                        WHERE id = ?
                        "#,
                    )
                    .bind(&q.subject)
                    .bind(&q.subtopic)
                    .bind(q.marks)
                    .bind(&q.content)
                    .bind(&q.math_snippet)
                    .bind(q.is_code)
                    .bind(&q.answer_content)
                    .bind(&q.topics)
                    .bind(&q.module)
                    .bind(&id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| format!("Import failed while updating a question: {}", e))?;
                    updated += 1;
                    continue;
                }
            }

            // Insert path: in merge mode, dodge id collisions with a fresh id.
            let mut id = q.id.clone();
            if !replace {
                loop {
                    let taken: Option<String> =
                        sqlx::query_scalar("SELECT id FROM questions WHERE id = ?")
                            .bind(&id)
                            .fetch_optional(&mut *tx)
                            .await
                            .map_err(|e| {
                                format!("Import failed while checking duplicates: {}", e)
                            })?;
                    if taken.is_none() {
                        break;
                    }
                    id = Uuid::new_v4().to_string();
                }
            }

            sqlx::query(
                r#"
                INSERT INTO questions (id, subject, subtopic, marks, content, math_snippet,
                                       is_code, answer_content, topics, paper_name,
                                       question_number, module)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&id)
            .bind(&q.subject)
            .bind(&q.subtopic)
            .bind(q.marks)
            .bind(&q.content)
            .bind(&q.math_snippet)
            .bind(q.is_code)
            .bind(&q.answer_content)
            .bind(&q.topics)
            .bind(&q.paper_name)
            .bind(q.question_number)
            .bind(&q.module)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("Import failed while inserting a question: {}", e))?;
            added += 1;
        }

        Ok((added, updated))
    }
    .await;

    match result {
        Ok((added, updated)) => {
            tx.commit()
                .await
                .map_err(|e| format!("Failed to finalise import: {}", e))?;
            Ok(ImportSummary {
                added,
                updated,
                images_copied: extracted.len(),
                replaced: replace,
            })
        }
        Err(e) => {
            // Roll back — the library stays exactly as it was.
            let _ = tx.rollback().await;
            Err(e)
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_rewrite_swaps_abs_paths_and_dedupes() {
        let mut map = HashMap::new();
        let mut used = std::collections::HashSet::new();
        let mut missing = 0usize;

        // Create real files so the `is_file()` check passes.
        let dir = std::env::temp_dir().join(format!("mm-export-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p1 = dir.join("a.png");
        std::fs::write(&p1, b"one").unwrap();

        let text = format!("before\n\n![Diagram]({})\n\nafter", slashy(&p1));
        let out = rewrite_for_export(&text, &mut map, &mut used, &mut missing);
        assert_eq!(missing, 0);
        assert!(
            out.contains("](images/a.png)"),
            "rewrote to relative: {}",
            out
        );

        // Same file referenced again -> same archive name, still one entry.
        let out2 = rewrite_for_export(&text, &mut map, &mut used, &mut missing);
        assert!(out2.contains("](images/a.png)"));
        assert_eq!(map.len(), 1);

        // Missing file -> counted, reference left untouched.
        let ghost = format!("![Diagram]({}/ghost.png)", slashy(&dir));
        let out3 = rewrite_for_export(&ghost, &mut map, &mut used, &mut missing);
        assert_eq!(missing, 1);
        assert_eq!(out3, ghost);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn import_rewrite_swaps_relative_for_extracted_paths() {
        let mut m = HashMap::new();
        m.insert("a.png".to_string(), "C:/data/diagrams/u1.png".to_string());
        let out = rewrite_for_import("x ![Diagram](images/a.png) y", &m);
        assert_eq!(out, "x ![Diagram](C:/data/diagrams/u1.png) y");
    }
}
