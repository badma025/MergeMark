//! Corpus papers for tests.
//!
//! The exam papers the tests read live in the repository's `papers/` folder,
//! which git does not track (`fixtures/corpus/README.md` says how to restore
//! the ones the tests need). A test whose paper is missing has nothing to
//! check. By default it says so and returns. With
//! `MERGEMARK_REQUIRE_FIXTURES=1`, which CI and `scripts/verify.sh` set, it
//! fails instead, and so does a test whose paper PDFium cannot open: a
//! passing run then means every test checked what it names.

use std::io::Write;
use std::path::PathBuf;

/// Whether a missing fixture fails its test (`MERGEMARK_REQUIRE_FIXTURES=1`).
pub(crate) fn fixtures_required() -> bool {
    std::env::var("MERGEMARK_REQUIRE_FIXTURES").is_ok_and(|v| v.trim() == "1")
}

/// The path of a corpus paper named relative to the repository root
/// (`papers/used/…`), or `None` when this machine does not have it.
///
/// With `MERGEMARK_FIXTURE_LOG=<file>` set (a relative path is the
/// repository root's), the name is appended to that file:
/// `scripts/fixtures.mjs record` makes the committed fixture list from it.
pub(crate) fn corpus_paper(rel: &str) -> Option<PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    if let Ok(log) = std::env::var("MERGEMARK_FIXTURE_LOG") {
        static LOG: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _one_writer = LOG.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let log = root.join(log);
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&log).unwrap_or_else(|e| panic!("{}: {e}", log.display()));
        writeln!(file, "{rel}").unwrap_or_else(|e| panic!("{}: {e}", log.display()));
    }
    let path = root.join(rel);
    if path.exists() {
        Some(path)
    } else {
        unavailable(&format!("fixture missing: {rel}"))
    }
}

/// What PDFium gave, or `None` when it could not open or read a paper.
pub(crate) fn pdfium<T, E: std::fmt::Display>(result: Result<T, E>, what: &str) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => unavailable(&format!("PDFium could not read {what}: {error}")),
    }
}

/// A test has nothing to check for want of `what`: `None` after saying so,
/// or a failure when fixtures are required.
pub(crate) fn unavailable<T>(what: &str) -> Option<T> {
    if fixtures_required() {
        panic!("{what} (MERGEMARK_REQUIRE_FIXTURES=1: a missing fixture fails its test)");
    }
    eprintln!("[FIXTURE] {what}: this test checked nothing");
    None
}
