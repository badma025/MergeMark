// Independent source-question-ID derivation from raw `--inspect-source` page
// text dumps. This is deliberately SEPARATE from the production extraction
// parser: it exists only to produce a human-reviewable expectation fixture so a
// calibration percentage is not measured against the parser's own count.
//
// The output of this module is frozen into
// `scripts/fixtures/source-expected-ids.json` (tied to each PDF's SHA256).
// The production ingestion pipeline never reads this fixture.

import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";

/// Reads page_*.txt files from an inspect-source dump, ordered by page number.
export function readPageTexts(dumpDir) {
  const files = readdirSync(dumpDir)
    .filter((f) => /^page_\d+\.txt$/.test(f))
    .sort((a, b) => Number(a.match(/\d+/)[0]) - Number(b.match(/\d+/)[0]));
  return files.map((f) => ({
    page: Number(f.match(/\d+/)[0]),
    text: readFileSync(path.join(dumpDir, f), "utf8"),
  }));
}

/// AQA-style boxed question IDs render as `0 1`, `0 2 . 1`, ... Parent question
/// IDs are the leading two-digit `0N` (N in 1..40). The mandatory whitespace
/// keeps this from matching unboxed digit runs such as binary literals.
export function deriveAqaBoxedIds(pages) {
  const ids = new Set();
  const evidence = new Map();
  for (const { page, text } of pages) {
    for (const m of text.matchAll(/(?:^|[^0-9])0\s+(\d{1,2})(?:\s*\.\s*\d)?(?![0-9])/g)) {
      const n = Number(m[1]);
      if (n >= 1 && n <= 40) {
        ids.add(String(n).padStart(2, "0"));
        if (!evidence.has(n)) evidence.set(n, page);
      }
    }
  }
  return { ids: [...ids].sort((a, b) => Number(a) - Number(b)), pages: evidence };
}

/// Numbered question headings: `1.`, `12)`, `3 ` at the start of a line,
/// plus explicit `Question N continued` / `(Total for Question N ...)`.
export function deriveNumberedIds(pages) {
  const ids = new Set();
  const evidence = new Map();
  const add = (n, page) => {
    if (Number.isInteger(n) && n >= 1 && n <= 40) {
      ids.add(String(n));
      if (!evidence.has(n)) evidence.set(n, page);
    }
  };
  for (const { page, text } of pages) {
    for (const line of text.split(/\r?\n/)) {
      const t = line.trim();
      let m = t.match(/^Question\s+(\d{1,2})\b/i);
      if (m) add(Number(m[1]), page);
      m = t.match(/^\(?Total for Question\s+(\d{1,2})\b/i);
      if (m) add(Number(m[1]), page);
      if (/^(?:Question\s+\d+\s+continued|.*continued on next page)/i.test(t)) continue;
      // Leading heading `N.` / `N)` / `N (a) ...` with real content after it.
      m = t.match(/^(\d{1,2})\s*[\.\)]\s*\S/) || t.match(/^(\d{1,2})\s+\S/);
      if (m) add(Number(m[1]), page);
    }
  }
  return { ids: [...ids].sort((a, b) => Number(a) - Number(b)), pages: evidence };
}

/// `Question N (*****)` (Madas / Naiker practice papers).
export function deriveQuestionHeadingIds(pages) {
  const ids = new Set();
  const evidence = new Map();
  for (const { page, text } of pages) {
    for (const m of text.matchAll(/^\s*Question\s+(\d{1,2})\b/gim)) {
      const n = Number(m[1]);
      if (n >= 1 && n <= 40) {
        ids.add(String(n));
        if (!evidence.has(n)) evidence.set(n, page);
      }
    }
  }
  return { ids: [...ids].sort((a, b) => Number(a) - Number(b)), pages: evidence };
}

/// The front-matter self-declaration, e.g. "There are 16 questions in this
/// question paper". Independent corroboration for the heading scan.
export function deriveDeclaredCount(pages) {
  for (const { page, text } of pages) {
    const m = text.match(/There are\s+(\d{1,3})\s+questions/i);
    if (m) return { count: Number(m[1]), page };
  }
  return { count: null, page: null };
}

export const STYLES = {
  aqa: deriveAqaBoxedIds,
  examiner: deriveAqaBoxedIds,
  numbered: deriveNumberedIds,
  heading: deriveQuestionHeadingIds,
};

/// AQA-style "For Examiner's Use" front table, e.g.
///   For Examiner’s Use | Question Mark | 1 | 2 | ... | 6 | 7–31 | TOTAL
/// The table explicitly declares the full set of question IDs (ranges are
/// written with an en-dash). This is the strongest independent source evidence
/// for AQA-style papers.
export function deriveExaminerTableIds(pages) {
  const ids = new Set();
  let page = null;
  for (const { page: p, text } of pages) {
    const start = text.search(/For Examiner/);
    if (start < 0) continue;
    const end = text.indexOf("TOTAL", start);
    const segment = text.slice(start, end < 0 ? start + 400 : end);
    // Ranges first (`7–31`).
    for (const m of segment.matchAll(/(\d{1,2})\s*[–—-]\s*(\d{1,2})/g)) {
      const lo = Number(m[1]);
      const hi = Number(m[2]);
      if (lo >= 1 && hi >= lo && hi <= 40) {
        for (let n = lo; n <= hi; n += 1) ids.add(String(n));
      }
    }
    // Then standalone integers.
    for (const m of segment.matchAll(/(?<![0-9])(\d{1,2})(?![0-9–—-])/g)) {
      const n = Number(m[1]);
      if (n >= 1 && n <= 40) ids.add(String(n));
    }
    if (ids.size > 0 && page === null) page = p;
  }
  const evidence = new Map();
  for (const id of ids) evidence.set(Number(id), page);
  return { ids: [...ids].sort((a, b) => Number(a) - Number(b)), pages: evidence };
}

const EXAMINER_STYLES = new Set(["aqa", "examiner"]);

function contiguousFromOne(ids) {
  const arr = [...ids].map(Number).sort((a, b) => a - b);
  if (arr.length === 0) return null;
  for (let i = 0; i < arr.length; i += 1) {
    if (arr[i] !== i + 1) return null;
  }
  return arr[arr.length - 1];
}

/// Derives the expected parent IDs for one paper given its inspect dump and
/// style. Returns `{ ids, style, pages, declaredCount, confidence, candidates }`.
///
/// `confidence`:
///   * "HIGH"   — front matter declares the count and the scan covers it.
///   * "MEDIUM" — no declaration, but the scan is contiguous 1..N.
///   * "UNKNOWN"— cannot resolve independent IDs; `ids` is null and the raw
///                candidates are retained for human review instead of being
///                silently reported as verified coverage.
export function deriveExpectedIds(dumpDir, style) {
  const pages = readPageTexts(dumpDir);
  const fn = STYLES[style];
  if (!fn) throw new Error(`unknown source-id style: ${style}`);
  let { ids, pages: evidence } = fn(pages);
  const declared = deriveDeclaredCount(pages);
  // AQA-style papers declare their question IDs in the front examiner table;
  // prefer that explicit statement over the (lossy) boxed-label scan.
  if (EXAMINER_STYLES.has(style)) {
    const table = deriveExaminerTableIds(pages);
    if (table.ids.length > 0) {
      ids = table.ids;
      evidence = table.pages;
    }
  }
  const numeric = ids.map(Number);
  let expectedIds = null;
  let confidence = "UNKNOWN";
  if (EXAMINER_STYLES.has(style) && ids.length > 0) {
    expectedIds = ids.map(Number);
    confidence = "HIGH";
  } else if (declared.count && Number.isInteger(declared.count)) {
    const want = Array.from({ length: declared.count }, (_, i) => i + 1);
    const covered = want.every((n) => numeric.includes(n));
    if (covered) {
      expectedIds = want;
      confidence = "HIGH";
    }
  }
  if (!expectedIds) {
    const max = contiguousFromOne(ids);
    if (max) {
      expectedIds = Array.from({ length: max }, (_, i) => i + 1);
      confidence = "MEDIUM";
    }
  }
  return {
    ids: expectedIds,
    style,
    confidence,
    candidates: ids,
    pages: Object.fromEntries([...evidence.entries()].map(([id, page]) => [id, page])),
    declaredCount: declared.count,
    declaredPage: declared.page,
  };
}
