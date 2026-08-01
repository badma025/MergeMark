import re

with open('src/commands.rs', 'r', encoding='utf-8') as f:
    lines = f.readlines()

new_gen_topics = """#[tauri::command]
pub async fn generate_topics_for_module(
    app: tauri::AppHandle,
    module_id: String,
    api_key: String,
    base_url: String,
    model_name: String,
) -> Result<Vec<String>, String> {
    use crate::AppState;
    use serde_json::json;
    use tauri::Manager;

    let state: tauri::State<'_, AppState> = app.state();
    let pool = state.db.lock().await;

    // Fetch the module name and subject name
    let module: Option<(String, String)> = sqlx::query_as(
        "SELECT modules.name, subjects.name FROM modules JOIN subjects ON modules.subject_id = subjects.id WHERE modules.id = ?"
    )
    .bind(&module_id)
    .fetch_optional(&*pool)
    .await
    .map_err(|e| e.to_string())?;

    let (module_name, subject_name) = module.ok_or_else(|| "Module not found".to_string())?;

    // Drop the lock before await
    drop(pool);

    let system_prompt = "You are an educational taxonomy assistant. Your job is to output a JSON array of curriculum topics for a given module and subject. Output ONLY a valid JSON array of strings.";
    let user_prompt = format!("Subject: {}\\nModule: {}\\n\\nAct strictly according to the official syllabus and textbook chapters for this specific subject and module. Provide an exhaustive list of core topics covered in this module as a JSON array of strings.\\n\\nCRITICAL INSTRUCTIONS:\\n- Output ONLY the exact, short, high-level chapter names (e.g. \\"Complex Numbers\\", \\"Matrices\\", \\"Proof by Induction\\").\\n- Do NOT include parentheses, subtopics, or any explanatory descriptions.\\n- Output nothing but the JSON array.", subject_name, module_name);

    let request_body = json!({
        "model": model_name,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_prompt }
        ],
        "temperature": 0.2
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

    let res = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("LLM request failed: {}", e))?;

    if !res.status().is_success() {
        let err = res.text().await.unwrap_or_default();
        return Err(format!("API error: {}", err));
    }

    let json: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    let content = json["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| "Invalid LLM response format".to_string())?;

    let mut json_str = content.trim();
    if let (Some(start), Some(end)) = (json_str.find('['), json_str.rfind(']')) {
        if start <= end {
            json_str = &json_str[start..=end];
        }
    }

    let topics: Vec<String> = serde_json::from_str(json_str)
        .map_err(|e| format!("Failed to parse JSON array from LLM. Raw content: {}\\nError: {}", content, e))?;

    let pool = state.db.lock().await;
    for topic_name in &topics {
        let id = uuid::Uuid::new_v4().to_string();
        let _ = sqlx::query("INSERT INTO topics (id, module_id, name) VALUES (?, ?, ?)")
            .bind(&id)
            .bind(&module_id)
            .bind(topic_name)
            .execute(&*pool)
            .await;
    }

    Ok(topics)
}
"""

out = []
i = 0
while i < len(lines):
    # Remove old llm and pipeline imports at the top
    if lines[i].startswith("use crate::llm::"):
        i += 1
        continue
    if lines[i].startswith("use crate::pipeline::"):
        # The pipeline import spans multiple lines:
        # use crate::pipeline::{
        #     self, AnswerDraft, BuiltQuestion, ImportReport, PageInput, PipelineConfig, Progress,
        # };
        i += 3
        continue
    
    if i == 2006:
        out.append(new_gen_topics)
        break # Because it's the last function in the file!

    out.append(lines[i])
    i += 1

with open('src/commands.rs', 'w', encoding='utf-8') as f:
    f.writelines(out)
