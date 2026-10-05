//! The paper behind a card: the pages its question was imported from,
//! rendered so the card can be checked against them.
//!
//! An import records the PDF's path, the SHA-256 of its bytes and the
//! question's page span (`questions.source_*`). Showing the pages re-reads
//! the file: a file gone from its path, or a different file now at it, is
//! said to be so and never shown as if it were the paper. The user can point
//! MergeMark at the file's new place, accepted only when it is the same PDF.

use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::path::Path;

/// The hex SHA-256 of a file's bytes: how a moved paper is known again.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// What showing a card's source finds.
#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SourceView {
    /// The question's pages (1-based, inclusive), rendered as PNG files.
    Pages { path: String, first_page: i64, last_page: i64, images: Vec<String> },
    /// There is no file at the recorded path: the paper moved or was deleted.
    Missing { path: String },
    /// The file at the recorded path is not the one the card came from.
    Changed { path: String },
    /// The card records no paper: imported before 1.0, or typed in.
    Unrecorded,
}

/// The pages of the paper `id` was imported from, rendered into
/// `pages_dir/<sha256>/` (kept, for the papers shown most recently).
pub async fn view(pool: &SqlitePool, pages_dir: &Path, id: &str) -> Result<SourceView, String> {
    let recorded: Option<(Option<String>, Option<String>, Option<i64>, Option<i64>)> =
        sqlx::query_as("SELECT source_path, source_sha256, source_first_page, source_last_page FROM questions WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;
    let Some(recorded) = recorded else { return Err(GONE.to_string()) };
    let (Some(path), Some(sha), Some(first), Some(last)) = recorded else { return Ok(SourceView::Unrecorded) };
    if first < 1 || last < first {
        return Err(format!("This card records pages {first} to {last} of its paper, which cannot be shown."));
    }
    let pages_dir = pages_dir.to_path_buf();
    tokio::task::spawn_blocking(move || show(path, &sha, first, last, &pages_dir)).await.map_err(|e| e.to_string())?
}

fn show(path: String, sha: &str, first: i64, last: i64, pages_dir: &Path) -> Result<SourceView, String> {
    let pdf = Path::new(&path);
    if !pdf.is_file() {
        return Ok(SourceView::Missing { path });
    }
    let bytes = std::fs::read(pdf).map_err(|e| format!("{path} could not be read: {e}"))?;
    if sha256_hex(&bytes) != sha {
        return Ok(SourceView::Changed { path });
    }
    let dir = pages_dir.join(sha);
    let images = render(&bytes, first, last, &dir)?;
    mark_shown(&dir);
    prune(pages_dir, PAPERS_KEPT);
    Ok(SourceView::Pages { path, first_page: first, last_page: last, images })
}

/// The PNG of each page `first..=last`, rendering those not already kept.
fn render(bytes: &[u8], first: i64, last: i64, dir: &Path) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{} could not be created: {e}", dir.display()))?;
    let png = |page: i64| dir.join(format!("page-{page}.png"));
    let to_draw: Vec<usize> = (first..=last).filter(|&page| !png(page).is_file()).map(|page| (page - 1) as usize).collect();
    if !to_draw.is_empty() {
        crate::pdf_render::render_pages_at_300dpi(bytes, &to_draw, |index, image| {
            let file = png(index as i64 + 1);
            // Written whole, then moved into place: no page is shown half-written.
            let partial = dir.join(format!("page-{}.{}.partial", index + 1, uuid::Uuid::new_v4()));
            image.to_rgb8().save_with_format(&partial, image::ImageFormat::Png).map_err(|e| format!("A page could not be saved: {e}"))?;
            std::fs::rename(&partial, &file).map_err(|e| format!("A page could not be saved: {e}"))
        })?;
    }
    Ok((first..=last).map(|page| png(page).to_string_lossy().into_owned()).collect())
}

/// Points every card imported from the same PDF as `id` at `path`, if the
/// file there is that PDF. Returns how many cards it moved.
pub async fn relocate(pool: &SqlitePool, id: &str, path: &Path) -> Result<u64, String> {
    let sha: Option<Option<String>> = sqlx::query_scalar("SELECT source_sha256 FROM questions WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    let sha = match sha {
        None => return Err(GONE.to_string()),
        Some(None) => return Err("This card records no paper to look for.".to_string()),
        Some(Some(sha)) => sha,
    };
    let shown = path.display().to_string();
    let bytes = tokio::fs::read(path).await.map_err(|e| format!("{shown} could not be read: {e}"))?;
    if sha256_hex(&bytes) != sha {
        return Err(format!(
            "{shown} is not the PDF this card was imported from: its contents differ. Choose the original file, or import this one as a paper of its own."
        ));
    }
    let path = std::path::absolute(path).map_err(|e| e.to_string())?.to_string_lossy().into_owned();
    let moved = sqlx::query("UPDATE questions SET source_path = ? WHERE source_sha256 = ?")
        .bind(&path)
        .bind(&sha)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(moved.rows_affected())
}

const GONE: &str = "This card is no longer in the library.";

/// How many papers keep their rendered pages: those shown most recently.
const PAPERS_KEPT: usize = 12;
/// The file in a paper's folder that records when it was last shown.
const SHOWN: &str = ".shown";

fn mark_shown(dir: &Path) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let _ = std::fs::write(dir.join(SHOWN), now.to_string());
}

/// Removes the rendered pages of all but the `keep` papers shown last.
fn prune(pages_dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(pages_dir) else { return };
    let mut papers: Vec<(u128, std::path::PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .map(|p| (std::fs::read_to_string(p.join(SHOWN)).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0), p))
        .collect();
    papers.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, dir) in papers.into_iter().skip(keep) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::corpus_paper;

    const PAPER: &str = "papers/used/Maths_A-level_Papers_CIE__June_2019_(v1)_QP_-_S1.pdf";
    const OTHER: &str = "papers/used/Maths_A-level_M2_Papers-Edexcel__January_2009_QP.pdf";

    async fn library(dir: &Path) -> SqlitePool {
        crate::db::init_db(dir.join("app")).await.unwrap()
    }

    async fn card(pool: &SqlitePool, id: &str, number: i64, source: Option<(&Path, &str, i64, i64)>) {
        sqlx::query("INSERT INTO questions (id, subject, subtopic, marks, content, math_snippet, is_code, paper_name, question_number, source_path, source_sha256, source_first_page, source_last_page) VALUES (?, 'Mathematics', '', 2, 'A question.', '', 0, 'CIE S1 June 2019', ?, ?, ?, ?, ?)")
            .bind(id)
            .bind(number)
            .bind(source.map(|s| s.0.to_string_lossy().into_owned()))
            .bind(source.map(|s| s.1.to_string()))
            .bind(source.map(|s| s.2))
            .bind(source.map(|s| s.3))
            .execute(pool)
            .await
            .unwrap();
    }

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mm_source_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A card's pages are rendered from its paper at 300 DPI. When the paper
    /// moves, the card says so; another PDF is not taken for it, and the same
    /// file found in its new place serves every card imported from it.
    #[tokio::test]
    async fn a_moved_paper_is_missing_until_it_is_found_again() {
        let _pdfium = crate::pdf_render::pdfium_test_lock();
        let (Some(paper), Some(other)) = (corpus_paper(PAPER), corpus_paper(OTHER)) else { return };
        let dir = temp_dir();
        let pool = library(&dir).await;
        let pdf = dir.join("paper.pdf");
        std::fs::copy(&paper, &pdf).unwrap();
        let sha = sha256_hex(&std::fs::read(&pdf).unwrap());
        card(&pool, "q4", 4, Some((&pdf, &sha, 6, 7))).await;
        card(&pool, "q2", 2, Some((&pdf, &sha, 4, 4))).await;
        let pages_dir = dir.join("source-pages");

        let SourceView::Pages { path, first_page, last_page, images } = view(&pool, &pages_dir, "q4").await.unwrap() else {
            panic!("the paper is where the card says")
        };
        assert_eq!((path.as_str(), first_page, last_page), (pdf.to_string_lossy().as_ref(), 6, 7));
        assert_eq!(images.len(), 2, "{images:?}");
        for image in &images {
            assert!(image.starts_with(&*pages_dir.join(&sha).to_string_lossy()), "{image}");
            let png = image::open(image).unwrap();
            assert_eq!(png.width(), 2480, "an A4 page at 300 DPI");
        }

        let moved = dir.join("moved");
        std::fs::create_dir_all(&moved).unwrap();
        let moved = moved.join("paper.pdf");
        std::fs::rename(&pdf, &moved).unwrap();
        let missing = SourceView::Missing { path: pdf.to_string_lossy().into_owned() };
        assert_eq!(view(&pool, &pages_dir, "q4").await.unwrap(), missing);

        let refused = relocate(&pool, "q4", &other).await.expect_err("another paper is not this card's");
        assert!(refused.contains("not the PDF"), "{refused}");
        assert_eq!(view(&pool, &pages_dir, "q4").await.unwrap(), missing);

        assert_eq!(relocate(&pool, "q4", &moved).await.unwrap(), 2, "both cards from the paper follow it");
        let SourceView::Pages { path, first_page: 4, last_page: 4, images } = view(&pool, &pages_dir, "q2").await.unwrap() else {
            panic!("the paper was found again")
        };
        assert_eq!(path, moved.to_string_lossy());
        assert_eq!(images.len(), 1);
    }

    /// A different file now at the paper's path is not shown as the paper.
    #[tokio::test]
    async fn a_different_file_at_the_papers_path_is_not_shown_as_it() {
        let _pdfium = crate::pdf_render::pdfium_test_lock();
        let (Some(paper), Some(other)) = (corpus_paper(PAPER), corpus_paper(OTHER)) else { return };
        let dir = temp_dir();
        let pool = library(&dir).await;
        let pdf = dir.join("paper.pdf");
        std::fs::copy(&paper, &pdf).unwrap();
        let sha = sha256_hex(&std::fs::read(&pdf).unwrap());
        card(&pool, "q2", 2, Some((&pdf, &sha, 4, 4))).await;
        std::fs::copy(&other, &pdf).unwrap();
        let pages_dir = dir.join("source-pages");
        assert_eq!(view(&pool, &pages_dir, "q2").await.unwrap(), SourceView::Changed { path: pdf.to_string_lossy().into_owned() });
        assert!(!pages_dir.join(&sha).exists(), "nothing was rendered");
    }

    /// A card imported before 1.0, or typed in, records no paper.
    #[tokio::test]
    async fn a_card_with_no_recorded_paper_says_so() {
        let dir = temp_dir();
        let pool = library(&dir).await;
        card(&pool, "typed", 1, None).await;
        assert_eq!(view(&pool, &dir.join("source-pages"), "typed").await.unwrap(), SourceView::Unrecorded);
        assert!(relocate(&pool, "typed", &dir.join("any.pdf")).await.is_err());
    }

    /// Rendered pages are kept for the papers shown most recently.
    #[test]
    fn only_the_papers_shown_last_keep_their_pages() {
        let dir = temp_dir();
        for k in 0..(PAPERS_KEPT + 2) {
            let paper = dir.join(format!("paper-{k}"));
            std::fs::create_dir_all(&paper).unwrap();
            std::fs::write(paper.join(SHOWN), (1000 + k).to_string()).unwrap();
        }
        prune(&dir, PAPERS_KEPT);
        assert!(!dir.join("paper-0").exists() && !dir.join("paper-1").exists(), "the two shown longest ago go");
        for k in 2..(PAPERS_KEPT + 2) {
            assert!(dir.join(format!("paper-{k}")).exists(), "paper-{k} stays");
        }
    }
}
