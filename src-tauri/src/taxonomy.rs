use serde::{Deserialize, Serialize};
use tauri::{Manager, State};
use crate::AppState;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct Topic {
    pub id: String,
    pub module_id: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct ModuleRow {
    pub id: String,
    pub subject_id: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Module {
    pub id: String,
    pub subject_id: String,
    pub name: String,
    pub topics: Vec<Topic>,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct SubjectRow {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Subject {
    pub id: String,
    pub name: String,
    pub modules: Vec<Module>,
}

#[tauri::command]
pub async fn get_taxonomy_tree(app: tauri::AppHandle) -> Result<Vec<Subject>, String> {
    let state: State<'_, AppState> = app.state();
    let pool = state.db.lock().await;

    // Fetch all flat
    let subjects_rows = sqlx::query_as::<_, SubjectRow>("SELECT id, name FROM subjects ORDER BY name")
        .fetch_all(&*pool)
        .await
        .map_err(|e| e.to_string())?;

    let modules_rows = sqlx::query_as::<_, ModuleRow>("SELECT id, subject_id, name FROM modules ORDER BY name")
        .fetch_all(&*pool)
        .await
        .map_err(|e| e.to_string())?;

    let topics_rows = sqlx::query_as::<_, Topic>("SELECT id, module_id, name FROM topics ORDER BY name")
        .fetch_all(&*pool)
        .await
        .map_err(|e| e.to_string())?;

    let mut subjects = Vec::new();
    for s_row in subjects_rows {
        let mut modules = Vec::new();
        for m_row in modules_rows.iter().filter(|m| m.subject_id == s_row.id) {
            let mut topics = Vec::new();
            for t_row in topics_rows.iter().filter(|t| t.module_id == m_row.id) {
                topics.push(Topic {
                    id: t_row.id.clone(),
                    module_id: t_row.module_id.clone(),
                    name: t_row.name.clone(),
                });
            }
            modules.push(Module {
                id: m_row.id.clone(),
                subject_id: m_row.subject_id.clone(),
                name: m_row.name.clone(),
                topics,
            });
        }
        subjects.push(Subject {
            id: s_row.id.clone(),
            name: s_row.name.clone(),
            modules,
        });
    }

    Ok(subjects)
}

#[tauri::command]
pub async fn add_subject(app: tauri::AppHandle, name: String) -> Result<String, String> {
    let state: State<'_, AppState> = app.state();
    let pool = state.db.lock().await;
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO subjects (id, name) VALUES (?, ?)")
        .bind(&id)
        .bind(&name)
        .execute(&*pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub async fn rename_subject(app: tauri::AppHandle, id: String, name: String) -> Result<(), String> {
    let state: State<'_, AppState> = app.state();
    let pool = state.db.lock().await;
    sqlx::query("UPDATE subjects SET name = ? WHERE id = ?")
        .bind(&name)
        .bind(&id)
        .execute(&*pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn delete_subject(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let state: State<'_, AppState> = app.state();
    let pool = state.db.lock().await;
    sqlx::query("DELETE FROM subjects WHERE id = ?")
        .bind(&id)
        .execute(&*pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn add_module(app: tauri::AppHandle, subject_id: String, name: String) -> Result<String, String> {
    let state: State<'_, AppState> = app.state();
    let pool = state.db.lock().await;
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO modules (id, subject_id, name) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(&subject_id)
        .bind(&name)
        .execute(&*pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub async fn rename_module(app: tauri::AppHandle, id: String, name: String) -> Result<(), String> {
    let state: State<'_, AppState> = app.state();
    let pool = state.db.lock().await;
    sqlx::query("UPDATE modules SET name = ? WHERE id = ?")
        .bind(&name)
        .bind(&id)
        .execute(&*pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn delete_module(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let state: State<'_, AppState> = app.state();
    let pool = state.db.lock().await;
    sqlx::query("DELETE FROM modules WHERE id = ?")
        .bind(&id)
        .execute(&*pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn add_topic(app: tauri::AppHandle, module_id: String, name: String) -> Result<String, String> {
    let state: State<'_, AppState> = app.state();
    let pool = state.db.lock().await;
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO topics (id, module_id, name) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(&module_id)
        .bind(&name)
        .execute(&*pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub async fn rename_topic(app: tauri::AppHandle, id: String, name: String) -> Result<(), String> {
    let state: State<'_, AppState> = app.state();
    let pool = state.db.lock().await;
    sqlx::query("UPDATE topics SET name = ? WHERE id = ?")
        .bind(&name)
        .bind(&id)
        .execute(&*pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn delete_topic(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let state: State<'_, AppState> = app.state();
    let pool = state.db.lock().await;
    sqlx::query("DELETE FROM topics WHERE id = ?")
        .bind(&id)
        .execute(&*pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
