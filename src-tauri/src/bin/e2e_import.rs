//! Headless end-to-end import harness.
//!
//! Runs the EXACT production extraction pipeline (doc map → Tier-0 →
//! text-first → crop-first → vision repair loop → deterministic figure
//! splicing → sanitizer) against a real PDF and writes the resulting cards
//! to disk as markdown for verification — no GUI, no Tauri state.
//!
//! Usage:
//!   cargo run --bin e2e_import -- "physics '17.pdf" "physics '17" --out dir
//!
//!   # Offline certification: no credentials, no network. Every attempted
//!   # model request is refused and counted, and the run exits non-zero if
//!   # anything at all was attempted.
//!   cargo run --bin e2e_import -- "physics '24.pdf" "physics '24" --offline --out dir
//!
//! Environment:
//!   OPENROUTER_API_KEY   (required unless MERGEMARK_E2E_MODEL is a groq key)
//!   OPENROUTER_BASE_URL  (optional, default https://openrouter.ai/api/v1/)
//!   MERGEMARK_E2E_MODEL  (default google/gemini-2.5-flash)

use mergemark_lib::llm::{LlmConfig, RefusingLlm, ReqwestLlm};
use mergemark_lib::pipeline::{
    run_question_pipeline, NullProgress, PageInput, PipelineConfig,
};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        return Err(
            "usage: e2e_import <pdf-path> <paper-name> [--out <dir>]".to_string(),
        );
    }
    let pdf_path = args[1].clone();
    let paper_name = args[2].clone();
    let out_dir = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("e2e_output"));
    // Offline certification mode: refuse + count every request instead of
    // calling a provider. No credentials, no network, no spend.
    let offline = args.iter().any(|a| a == "--offline");
    // Reject flags that require a value but were given none, or were followed
    // by another option token (`--subject --offline` must NOT silently treat
    // `--offline` as the subject). Never silently fall back to a default for an
    // explicit flag.
    for flag in ["--out", "--subject", "--module", "--config-db"] {
        if let Some(i) = args.iter().position(|a| a == flag) {
            match args.get(i + 1) {
                None => return Err(format!("{flag} requires a value")),
                Some(next) if next.starts_with("--") => {
                    return Err(format!("{flag} requires a value (got '{next}')"))
                }
                Some(_) => {}
            }
        }
    }

    if args.iter().any(|a| a == "--inspect-source") {
        std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
        let path = std::path::Path::new(&pdf_path);
        for (i, page) in mergemark_lib::pdf_render::render_pdf_pages(path)?.iter().enumerate() {
            std::fs::write(out_dir.join(format!("page_{}.txt", i+1)), &page.text).map_err(|e| e.to_string())?;
        }
        let figures = mergemark_lib::pdf_render::detect_pdf_figures(path)?;
        let json: Vec<_> = figures.iter().enumerate().map(|(i, figures)| serde_json::json!({
            "page": i+1, "figures": figures.iter().map(|f| serde_json::json!({
                "bbox": f.bbox, "caption": f.caption, "kind": f.kind, "confidence": f.seg_confidence
            })).collect::<Vec<_>>()
        })).collect();
        std::fs::write(out_dir.join("figures.json"), serde_json::to_string_pretty(&json).unwrap()).map_err(|e| e.to_string())?;
        return Ok(());
    }
    if args.iter().any(|a| a == "--render-source") {
        std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
        for page in [15, 25, 27, 28, 31, 34] {
            mergemark_lib::pdf_render::render_pdf_page_at_300dpi(std::path::Path::new(&pdf_path), page)?
                .resize(1000, 1500, image::imageops::FilterType::Lanczos3)
                .save(out_dir.join(format!("source_page_{}.png", page + 1))).map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    let model =
        std::env::var("MERGEMARK_E2E_MODEL").unwrap_or_else(|_| "google/gemini-2.5-flash".into());
    // Explicit opt-in to the app's configured provider; open SQLite read-only
    // and never print credentials or modify the user's imports. Offline mode
    // needs no credentials at all, so it skips this entirely.
    let mut api_key: Option<String> = None;
    let mut base_url: Option<String> = None;
    if !offline {
        let mut stored_key = None;
        let mut stored_url = None;
        if let Some(db_path) = args.iter().position(|a| a == "--config-db").and_then(|i| args.get(i+1)) {
            let options = sqlx::sqlite::SqliteConnectOptions::new().filename(db_path).read_only(true);
            let pool = sqlx::SqlitePool::connect_with(options).await.map_err(|e| e.to_string())?;
            let row: (Option<String>, Option<String>) = sqlx::query_as("SELECT byok_api_key, byok_base_url FROM usage_config WHERE id = 1")
                .fetch_one(&pool).await.map_err(|e| e.to_string())?;
            stored_key = row.0.filter(|s| !s.trim().is_empty());
            stored_url = row.1.filter(|s| !s.trim().is_empty());
            pool.close().await;
        }
        api_key = std::env::var("OPENROUTER_API_KEY").ok().or(stored_key);
        base_url = std::env::var("OPENROUTER_BASE_URL").ok().or(stored_url);
    }

    let path = PathBuf::from(&pdf_path);
    let pages: Vec<PageInput> = mergemark_lib::pdf_render::render_pdf_pages(&path)?;
    println!(
        "[E2E] {} pages rendered; detecting figures…",
        pages.len()
    );
    let page_figures = mergemark_lib::pdf_render::detect_pdf_figures(&path)?;
    println!(
        "[E2E] {} figures detected (free, on-device)",
        page_figures.iter().map(Vec::len).sum::<usize>()
    );

    // Backward-compatible subject/module overrides so CS/maths calibration is
    // not forced through the Physics taxonomy.
    let subject = args
        .iter()
        .position(|a| a == "--subject")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "Physics".to_string());
    let module_name = args
        .iter()
        .position(|a| a == "--module")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "A-Level Physics".to_string());
    let mut config = PipelineConfig::new(
        model.clone(),
        paper_name.clone(),
        subject,
        module_name,
        Some(path.clone()),
    );
    // Production values (mirror commands.rs parse_pdf_vision).
    config.max_repairs = 2;
    config.max_output_tokens = 32768;
    config.text_first = true;
    config.ms_text_first = true;
    config.deterministic = true;
    // Same per-import layout evidence the production path attaches.
    config.layout_evidence =
        mergemark_lib::pdf_render::load_import_evidence(&path).map(Arc::new);
    // Free-tier routes (Google AI Studio) enforce per-minute quotas — run
    // serially there via MERGEMARK_E2E_PARALLELISM=1; default mirrors prod.
    config.parallelism = std::env::var("MERGEMARK_E2E_PARALLELISM")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(4);
    // Subject-appropriate syllabus topics (not a gate). Never force the Physics
    // taxonomy onto CS/maths calibration runs.
    config.allowed_topics = if config.subject.eq_ignore_ascii_case("Physics") {
        ["Thermal physics", "Capacitors", "Electric fields", "Gravitational fields",
         "Magnetic fields", "Nuclear physics", "Waves", "Mechanics", "Electricity",
         "Required Practical"].into_iter().map(str::to_string).collect()
    } else if config.subject.eq_ignore_ascii_case("Computer Science") {
        ["Algorithms", "Data structures", "Programming", "Databases", "Networks",
         "Boolean logic", "Computer systems", "Software"].into_iter().map(str::to_string).collect()
    } else {
        ["Algebra", "Calculus", "Vectors", "Matrices", "Complex numbers",
         "Statistics", "Mechanics", "Proof", "Sequences"].into_iter().map(str::to_string).collect()
    };
    // Write diagram crops next to the output so cards reference real files.
    let diagrams_dir = out_dir.join("diagrams");
    std::fs::create_dir_all(&diagrams_dir).map_err(|e| e.to_string())?;
    config.diagrams_dir = Some(diagrams_dir);

    let cancel = Arc::new(AtomicBool::new(false));
    // `attempts` is (total, image, text) and only exists in offline mode, where
    // the refusing client is the source of truth for the $0 certification.
    let (built, report, attempts): (_, _, Option<(usize, usize, usize)>) = if offline {
        let client = RefusingLlm::new();
        let (built, report) = run_question_pipeline(
            &client,
            &pages,
            &page_figures,
            &config,
            &NullProgress,
            &cancel,
        )
        .await?;
        (
            built,
            report,
            Some((client.calls(), client.image_calls(), client.text_calls())),
        )
    } else {
        let api_key = api_key.ok_or(
            "No API key: set OPENROUTER_API_KEY or supply --config-db with a configured provider",
        )?;
        let client = ReqwestLlm::new(LlmConfig {
            base_url: base_url.unwrap_or_else(|| "https://openrouter.ai/api/v1/".to_string()),
            api_key,
            model: model.clone(),
            timeout: Duration::from_secs(300),
        });
        let (built, report) = run_question_pipeline(
            &client,
            &pages,
            &page_figures,
            &config,
            &NullProgress,
            &cancel,
        )
        .await?;
        (built, report, None)
    };

    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let mut md = String::new();
    let mut flagged = 0usize;
    let mut placeholder_fallbacks = 0usize;
    for q in &built {
        // The never-lose-a-question fallback card is a review placeholder, not
        // usable extracted content: count it separately so certification never
        // reports it as coverage.
        if q.notes
            .iter()
            .any(|n| n.contains("Fallback card created due to incomplete extraction"))
        {
            placeholder_fallbacks += 1;
        }
        // Same sanitizer gate as the persistence seam.
        let content =
            mergemark_lib::sanitize::sanitize_question_content(&q.content, q.question_number);
        let errs = mergemark_lib::validate::card_structure_errors(&content, q.question_number);
        if !errs.is_empty() {
            flagged += 1;
        }
        md.push_str(&format!(
            "\n\n===== Q{} | {}m | needs_review={} | issues={} =====\n{}{}\n",
            q.question_number,
            q.marks,
            q.needs_review,
            if errs.is_empty() { "clean".into() } else { errs.join("; ") },
            if q.notes.is_empty() { String::new() } else { format!("notes: {}\n", q.notes.join(" | ")) },
            content
        ));
    }
    let out_file = out_dir.join(format!(
        "{}_cards.md",
        paper_name.replace(' ', "_").replace('\'', "")
    ));
    std::fs::write(&out_file, md).map_err(|e| e.to_string())?;

    // Machine-readable export for downstream import into the app database.
    let cards: Vec<serde_json::Value> = built
        .iter()
        .map(|q| {
            serde_json::json!({
                "question_number": q.question_number,
                "marks": q.marks,
                "topics": q.topics,
                "needs_review": q.needs_review,
                "content": mergemark_lib::sanitize::sanitize_question_content(
                    &q.content, q.question_number,
                ),
            })
        })
        .collect();
    let json_file = out_dir.join(format!(
        "{}_cards.json",
        paper_name.replace(' ', "_").replace('\'', "")
    ));
    std::fs::write(&json_file, serde_json::to_string_pretty(&cards).unwrap())
        .map_err(|e| e.to_string())?;

    println!("\n[E2E] extracted {} questions -> {}", built.len(), out_file.display());
    println!(
        "[E2E] tier0={} text_first={} crop_first={} vision_repairs={} quarantined={}",
        report.deterministic,
        report.text_first,
        report.crop_first,
        report.repairs,
        report.quarantined.len()
    );
    println!(
        "[E2E] prompt_tokens={} completion_tokens={} structural_flags={}/{}",
        report.prompt_tokens, report.completion_tokens, flagged, built.len()
    );
    println!(
        "[E2E] text_layer={} strict_tier0={} local_recoveries={} quarantined={}",
        report.text_layer,
        report.deterministic,
        report.recovered,
        report.quarantined.len()
    );
    if let Some((calls, image_calls, text_calls)) = attempts {
        let slug = paper_name.replace(' ', "_").replace('\'', "");
        // A refused request makes no HTTP call, so offline spend is always
        // exactly zero. The zero-ATTEMPT policy is a separate, stricter gate:
        // it is what proves the digital circuit breaker held.
        let tokens_zero = report.prompt_tokens == 0 && report.completion_tokens == 0;
        let zero_attempt_policy = calls == 0;
        let usable_cards = built.len().saturating_sub(placeholder_fallbacks);
        let certification = serde_json::json!({
            "mode": "offline",
            "paper": paper_name,
            "pages": pages.len(),
            "textLayer": report.text_layer,
            "cloudAttempts": calls,
            "imageAttempts": image_calls,
            "textOnlyAttempts": text_calls,
            "promptTokens": report.prompt_tokens,
            "completionTokens": report.completion_tokens,
            "costUsd": "0.0000",
            "tokensZero": tokens_zero,
            "zeroAttemptPolicyPassed": zero_attempt_policy,
            "questionsExpected": report.questions_expected,
            "questionsExtracted": report.questions_extracted,
            "strictTier0": report.deterministic,
            "localRecoveries": report.recovered,
            "textFirst": report.text_first,
            "cropFirst": report.crop_first,
            "repairs": report.repairs,
            "quarantined": report.quarantined.len(),
            "structuralFlags": flagged,
            "placeholderFallbackCards": placeholder_fallbacks,
            "usableCards": usable_cards,
            "anomalies": &report.anomalies,
        });
        let cert_file = out_dir.join(format!("{}_offline_certification.json", slug));
        std::fs::write(&cert_file, serde_json::to_string_pretty(&certification).unwrap())
            .map_err(|e| e.to_string())?;
        println!(
            "[E2E][OFFLINE] cloud_attempts={} (text={} image={}) prompt_tokens={} completion_tokens={} spend=$0.0000 (refused requests never leave the machine)",
            calls,
            text_calls,
            image_calls,
            report.prompt_tokens,
            report.completion_tokens
        );
        println!(
            "[E2E][OFFLINE] zero_attempt_policy={} tokens_zero={} strict_tier0={} local_recoveries={} placeholder_fallbacks={} usable_cards={}",
            zero_attempt_policy,
            tokens_zero,
            report.deterministic,
            report.recovered,
            placeholder_fallbacks,
            usable_cards
        );
        println!("[E2E][OFFLINE] certification -> {}", cert_file.display());
        if !zero_attempt_policy || !tokens_zero {
            return Err(format!(
                "zero-cost certification FAILED: attempted={} (text {}, image {}), prompt_tokens={}, completion_tokens={}",
                calls, text_calls, image_calls, report.prompt_tokens, report.completion_tokens
            ));
        }
    }
    if !report.anomalies.is_empty() {
        println!("[E2E] anomalies:");
        for a in &report.anomalies {
            println!("  - {}", a);
        }
    }
    Ok(())
}
