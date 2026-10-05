//! Headless end-to-end import harness.
//!
//! Runs the EXACT production import pipeline (layout map → local
//! transcriber → deterministic figure splicing → sanitizer) against a real
//! PDF and writes the resulting cards to disk as markdown for verification —
//! no GUI, no Tauri state, no network.
//!
//! Usage:
//!   cargo run --example e2e_import -- "physics '24.pdf" "physics '24" --out dir
//!   cargo run --example e2e_import -- "<scheme>.pdf" "<name>" --mark-scheme --out dir
//!   cargo run --example e2e_import -- "<scheme>.pdf" "<name>" --mark-scheme --census --out dir
//!
//! `--offline` is accepted for older scripts: every run is offline. The
//! certification it writes records zero model requests, because MergeMark
//! has no model client to make one.

use mergemark_lib::pipeline::{
    run_question_pipeline, NullProgress, PageInput, PipelineConfig,
};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

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
    // Reject flags that require a value but were given none, or were followed
    // by another option token (`--subject --offline` must NOT silently treat
    // `--offline` as the subject). Never silently fall back to a default for an
    // explicit flag.
    for flag in ["--out", "--subject", "--module"] {
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
    // Developer view of the geometry layout: every page's lines (furniture
    // tagged) and the question map with each mapped body.
    if args.iter().any(|a| a == "--dump-layout") {
        std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
        let pages = mergemark_lib::pdf_render::load_layout_pages(std::path::Path::new(&pdf_path)).ok_or("layout unavailable")?;
        let mut s = String::new();
        for lp in &pages {
            s.push_str(&format!("===== page {} ({}x{})\nfigures {:?}\ndrawings {:?}\n{}", lp.page + 1, lp.width, lp.height, lp.figures, lp.drawings, lp.debug()));
        }
        std::fs::write(out_dir.join("layout_pages.txt"), s).map_err(|e| e.to_string())?;
        let map = mergemark_lib::layout::build_layout_map(&pages);
        let mut s = format!("anomalies: {:?}\n", map.anomalies);
        for q in &map.questions {
            let body = mergemark_lib::layout::question_body(&pages, q);
            s.push_str(&format!("===== Q{} pages {}-{} footer={:?}\n{}\n", q.number, q.start_page + 1, q.end_page + 1, body.footer_marks, body.text));
        }
        std::fs::write(out_dir.join("layout_map.txt"), s).map_err(|e| e.to_string())?;
        let unknown = mergemark_lib::pdf_render::unknown_glyph_report(std::path::Path::new(&pdf_path)).unwrap_or_default();
        std::fs::write(out_dir.join("unknown_glyphs.txt"), unknown).map_err(|e| e.to_string())?;
        let glyphs = mergemark_lib::pdf_render::glyph_report(std::path::Path::new(&pdf_path)).unwrap_or_default();
        std::fs::write(out_dir.join("glyphs.txt"), glyphs).map_err(|e| e.to_string())?;
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
    // Mark-scheme census: every reason the local reader refuses the scheme,
    // not only the first, as read and with rotated pages turned upright.
    if args.iter().any(|a| a == "--mark-scheme") && args.iter().any(|a| a == "--census") {
        let path = PathBuf::from(&pdf_path);
        let as_read = mergemark_lib::mark_scheme_deterministic::census(&path, false)?;
        let turned = mergemark_lib::mark_scheme_deterministic::census(&path, true)?;
        let refusals = |c: &mergemark_lib::mark_scheme_deterministic::Census| {
            c.refusals.iter().map(|r| serde_json::json!({ "page": r.page.map(|p| p + 1), "reason": r.reason })).collect::<Vec<_>>()
        };
        let census = serde_json::json!({
            "paper": paper_name,
            "file": pdf_path,
            "pages": as_read.pages,
            "rotatedPages": as_read.rotated_pages.iter().map(|p| p + 1).collect::<Vec<_>>(),
            "asRead": refusals(&as_read),
            "rotatedPagesTurned": refusals(&turned),
        });
        std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
        let slug = paper_name.replace(' ', "_").replace('\'', "");
        std::fs::write(out_dir.join(format!("{slug}_ms_census.json")), serde_json::to_string_pretty(&census).unwrap()).map_err(|e| e.to_string())?;
        println!("[E2E][MS-CENSUS] pages={} refusals as read={} with rotated pages turned={}", as_read.pages, as_read.refusals.len(), turned.refusals.len());
        return Ok(());
    }
    // Mark-scheme import: what the app's local reader makes of the scheme.
    if args.iter().any(|a| a == "--mark-scheme") {
        let path = PathBuf::from(&pdf_path);
        let pages: Vec<PageInput> = mergemark_lib::pdf_render::render_pdf_pages(&path)?;
        let config = PipelineConfig::new(paper_name.clone(), "MarkScheme".into(), "MarkScheme".into(), Some(path.clone()));
        let (drafts, report) = mergemark_lib::pipeline::run_markscheme_pipeline(&pages, &config, &NullProgress, &AtomicBool::new(false)).await?;
        std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
        let slug = paper_name.replace(' ', "_").replace('\'', "");
        let md: String = drafts.iter().map(|d| format!("\n\n===== Q{} =====\n{}\n", d.question_number, d.markdown)).collect();
        std::fs::write(out_dir.join(format!("{slug}_answers.md")), md).map_err(|e| e.to_string())?;
        let certification = serde_json::json!({
            "mode": "offline-mark-scheme",
            "paper": paper_name,
            "pages": pages.len(),
            "modelClient": "none",
            "cloudAttempts": 0,
            "answers": drafts.len(),
            "localAnswers": report.deterministic,
            "anomalies": &report.anomalies,
            "quarantined": report.quarantined.iter().map(|q| &q.reason).collect::<Vec<_>>(),
        });
        std::fs::write(out_dir.join(format!("{slug}_offline_certification.json")), serde_json::to_string_pretty(&certification).unwrap()).map_err(|e| e.to_string())?;
        println!("[E2E][MS] answers={} local={}", drafts.len(), report.deterministic);
        for a in &report.anomalies {
            println!("  - {a}");
        }
        return Ok(());
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
        paper_name.clone(),
        subject,
        module_name,
        Some(path.clone()),
    );
    // Same per-import layout evidence the production path attaches.
    config.layout_evidence =
        mergemark_lib::pdf_render::load_import_evidence(&path).map(Arc::new);
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
    let (built, report) = run_question_pipeline(
        &pages,
        &page_figures,
        &config,
        &NullProgress,
        &cancel,
    )
    .await?;

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
        "[E2E] tier0={} quarantined={} structural_flags={}/{}",
        report.deterministic,
        report.quarantined.len(),
        flagged,
        built.len()
    );
    println!(
        "[E2E] text_layer={} strict_tier0={} local_recoveries={} quarantined={}",
        report.text_layer,
        report.deterministic,
        report.recovered,
        report.quarantined.len()
    );
    {
        let slug = paper_name.replace(' ', "_").replace('\'', "");
        // MergeMark has no model client: no request can be attempted, so the
        // request and token counts the certification records are zero by
        // construction, not by measurement. What enforces it is the test
        // `mergemark_has_no_way_to_reach_an_ai_service` (no HTTP client in
        // Cargo.toml, no outside host in the window's CSP); the calibration
        // still checks the fields, so an older build's certification fails.
        let usable_cards = built.len().saturating_sub(placeholder_fallbacks);
        let certification = serde_json::json!({
            "mode": "offline",
            "paper": paper_name,
            "pages": pages.len(),
            "textLayer": report.text_layer,
            "modelClient": "none",
            "cloudAttempts": 0,
            "imageAttempts": 0,
            "textOnlyAttempts": 0,
            "promptTokens": 0,
            "completionTokens": 0,
            "costUsd": "0.0000",
            "tokensZero": true,
            "zeroAttemptPolicyPassed": true,
            "questionsExpected": report.questions_expected,
            "questionsExtracted": report.questions_extracted,
            "strictTier0": report.deterministic,
            "localRecoveries": report.recovered,
            "quarantined": report.quarantined.len(),
            "structuralFlags": flagged,
            "placeholderFallbackCards": placeholder_fallbacks,
            "usableCards": usable_cards,
            "pagesNotTranscribed": &report.pages_not_transcribed,
            "anomalies": &report.anomalies,
            "choiceGroups": &report.choice_groups,
        });
        let cert_file = out_dir.join(format!("{}_offline_certification.json", slug));
        std::fs::write(&cert_file, serde_json::to_string_pretty(&certification).unwrap())
            .map_err(|e| e.to_string())?;
        println!(
            "[E2E][OFFLINE] strict_tier0={} local_recoveries={} placeholder_fallbacks={} usable_cards={} (no model client: zero requests possible)",
            report.deterministic,
            report.recovered,
            placeholder_fallbacks,
            usable_cards
        );
        println!("[E2E][OFFLINE] certification -> {}", cert_file.display());
    }
    if !report.anomalies.is_empty() {
        println!("[E2E] anomalies:");
        for a in &report.anomalies {
            println!("  - {}", a);
        }
    }
    Ok(())
}
