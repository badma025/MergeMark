// Source-independent invariants for triaging an import of a paper that has no
// hand-made expected-IDs entry. Nothing here knows about a specific paper.
//
// Findings have a severity:
//   fail  - the import is wrong or untrustworthy (duplicate IDs, U+FFFD,
//           missing figure file, mark total that contradicts the printed total)
//   flag  - the pipeline itself marked the card for review (needs_review,
//           quarantine, placeholder). Honest, but not a clean import.
//   warn  - a heuristic worth a human look (ID gap, low text coverage)
//
// The module only reads: the cards JSON, the source PDF text (via pdftotext)
// and the figure files the cards reference.

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, statSync } from "node:fs";
import path from "node:path";

export function pdfPages(pdfPath) {
  try {
    const raw = execFileSync("pdftotext", ["-layout", pdfPath, "-"], {
      encoding: "utf8",
      maxBuffer: 256 * 1024 * 1024,
    });
    return raw.split("\f");
  } catch {
    return null;
  }
}

/// Totals printed on the paper itself. Several wordings exist across boards;
/// every distinct number found is a candidate. Section totals are collected
/// too, so a paper whose sections sum correctly is not failed for lacking one
/// overall figure.
export function printedTotals(pages) {
  const text = pages.join("\n");
  const paper = new Set();
  const section = [];
  const grab = (re, into) => {
    for (const m of text.matchAll(re)) into(Number(m[1]), m);
  };
  const addPaper = (n) => Number.isFinite(n) && n > 0 && n <= 400 && paper.add(n);
  grab(/TOTAL\s+FOR\s+(?:THIS\s+)?PAPER\s*(?:IS|=|:)?\s*(\d{1,3})\s*MARKS?/gi, addPaper);
  grab(/total\s+(?:mark|marks)\s+for\s+(?:this|the)\s+(?:question\s+)?paper\s+(?:is|are)\s*(\d{1,3})/gi, addPaper);
  grab(/maximum\s+mark\s+for\s+this\s+paper\s+is\s*(\d{1,3})/gi, addPaper);
  grab(/total\s+number\s+of\s+marks\s+(?:for\s+this\s+paper\s+)?(?:is|are)\s*(\d{1,3})/gi, addPaper);
  grab(/Total\s+marks?\s*[:=]?\s*(\d{1,3})\s*$/gim, addPaper);
  grab(/^\s*TOTAL\s+(\d{1,3})\s*$/gm, addPaper);
  grab(/\[Total:?\s*(\d{1,3})\]/gi, addPaper);
  grab(/total\s+mark\s+for\s+this\s+paper\s+is\s*(\d{1,3})/gi, addPaper);
  grab(/marks?\s+for\s+this\s+paper\s+(?:is|are)\s*(\d{1,3})/gi, addPaper);
  grab(/TOTAL\s+FOR\s+SECTION\s+[A-Z]\s*(?:IS|=|:)?\s*(\d{1,3})\s*MARKS?/gi, (n) => section.push(n));
  grab(/Total\s+for\s+Section\s+[A-Z]\s*(?:is|=|:)?\s*(\d{1,3})\s*marks?/gi, (n) => section.push(n));
  const sectionSum = section.length >= 2 ? section.reduce((a, b) => a + b, 0) : null;
  if (sectionSum) addPaper(sectionSum);
  return { candidates: [...paper].sort((a, b) => a - b), sections: section };
}

const MARK_TAG = /\*\*\[(\d+)\s*marks?\]\*\*/gi;

// A choice instruction as papers print it ("In Section A, answer EITHER
// Context 1 OR Context 2", "answer ONE essay").
const CHOICE_INSTRUCTION = /\banswer\s+(?:either\b|one\s+(?:essay|question|context|section)\b)/i;

// A card's tagged marks, with parts printed "EITHER (f) … OR (g) …" counted
// once (the largest): a candidate answers one of them. The choice ends at
// the next part of the alternatives' own kind ("(h)" after "(f)" and "(g)");
// a sub-part ("(ii)" inside "(f)") stays in its alternative.
function taggedMarks(content) {
  const marks = (line) => [...line.matchAll(MARK_TAG)].reduce((a, m) => a + Number(m[1]), 0);
  const label = (line) => {
    const m = /^\((?:([a-h])|i{1,3}|iv|vi{0,3}|ix|x)\)\s/.exec(line);
    return m ? (m[1] ? "letter" : "roman") : null;
  };
  const largest = (c) => Math.max(...c.answered, c.current);
  let total = 0;
  let choice = null;
  for (const raw of content.split("\n")) {
    const line = raw.trim();
    if (line === "EITHER") {
      if (choice) total += largest(choice);
      choice = { answered: [], current: 0, kind: null, opening: true };
    } else if (!choice) {
      total += marks(line);
    } else if (line === "OR") {
      choice.answered.push(choice.current);
      choice.current = 0;
      choice.opening = true;
    } else if (choice.opening && line) {
      choice.opening = false;
      choice.kind ??= label(line);
      choice.current += marks(line);
    } else if (label(line) && choice.kind && (label(line) === choice.kind || (choice.kind === "roman" && label(line) === "letter"))) {
      total += largest(choice) + marks(line);
      choice = null;
    } else {
      choice.current += marks(line);
    }
  }
  return choice ? total + largest(choice) : total;
}

function pngSize(file) {
  try {
    const b = readFileSync(file);
    if (b.length < 24 || b.readUInt32BE(0) !== 0x89504e47) return null;
    return { w: b.readUInt32BE(16), h: b.readUInt32BE(20), bytes: b.length };
  } catch {
    return null;
  }
}

const tokenize = (s) =>
  new Set(
    s
      .toLowerCase()
      .replace(/\\[a-z]+/g, " ")
      .match(/[a-z]{5,}/g) ?? [],
  );

/// Share of the source's longer words that appear somewhere in the cards.
/// Cover pages and instructions are excluded by dropping words that appear on
/// most pages (running furniture) and the first page.
export function textCoverage(pages, cards) {
  const body = pages.slice(1);
  const pageSets = body.map((p) => tokenize(p));
  const df = new Map();
  for (const set of pageSets) for (const t of set) df.set(t, (df.get(t) ?? 0) + 1);
  const limit = Math.max(3, Math.floor(body.length * 0.4));
  const source = new Set();
  for (const set of pageSets) for (const t of set) if ((df.get(t) ?? 0) < limit) source.add(t);
  if (source.size === 0) return null;
  const got = tokenize(cards.map((c) => c.content ?? "").join("\n"));
  let hit = 0;
  for (const t of source) if (got.has(t)) hit += 1;
  return hit / source.size;
}

export function checkPaper({ cards, sourcePdf, cardsDir, cert }) {
  const findings = [];
  const add = (severity, code, detail, question = null) =>
    findings.push({ severity, code, detail, question });
  const stats = {};

  if (!Array.isArray(cards) || cards.length === 0) {
    // Say why when the source looks like page images with only a watermark
    // for a text layer: the importer treats any text as a digital paper.
    const srcPages = sourcePdf ? pdfPages(sourcePdf) : null;
    let why = "";
    if (srcPages && srcPages.length) {
      const perPage = srcPages.reduce((a, p) => a + p.replace(/\s+/g, "").length, 0) / srcPages.length;
      stats.charsPerPage = Math.round(perPage);
      if (perPage < 150) why = ` (source has only ~${stats.charsPerPage} text characters per page: likely scanned with a watermark text layer)`;
    }
    add("fail", "no-cards", `the import produced no cards${why}`);
    return { findings, stats };
  }
  stats.cards = cards.length;

  // 1. Question IDs: unique, and (as a warning) contiguous.
  const ids = cards.map((c) => Number(c.question_number));
  const dupes = ids.filter((id, i) => ids.indexOf(id) !== i);
  if (dupes.length) add("fail", "duplicate-id", `question numbers repeated: ${[...new Set(dupes)].join(", ")}`);
  const uniq = [...new Set(ids)].sort((a, b) => a - b);
  const max = uniq[uniq.length - 1];
  const gaps = [];
  for (let n = 1; n <= max; n += 1) if (!uniq.includes(n)) gaps.push(n);
  stats.ids = uniq.length;
  stats.maxId = max;
  if (gaps.length) add("warn", "id-gap", `no card for question(s) ${gaps.slice(0, 12).join(", ")}${gaps.length > 12 ? "..." : ""}`);

  // 2. Per-card content checks.
  let flagged = 0;
  for (const c of cards) {
    const q = c.question_number;
    const content = String(c.content ?? "");
    if (c.needs_review) flagged += 1;
    if (content.trim().length < 20) add("fail", "empty-card", `content is ${content.trim().length} characters`, q);
    if (content.includes("�")) add("fail", "unidentified-glyph", "contains U+FFFD", q);
    if (/[-]/u.test(content)) add("fail", "private-use-glyph", "contains an unmapped private-use glyph", q);
    if (!Number.isFinite(c.marks) || c.marks <= 0) add("warn", "no-marks", `marks=${JSON.stringify(c.marks)}`, q);
    const tagged = [...content.matchAll(MARK_TAG)].map((m) => Number(m[1]));
    if (tagged.length && Number.isFinite(c.marks)) {
      const sum = taggedMarks(content);
      if (sum > c.marks) add("fail", "marks-tag-overcount", `card marks=${c.marks} but tags sum to ${sum}`, q);
      else if (sum < c.marks) add("warn", "marks-tag-undercount", `card marks=${c.marks} but tags sum to ${sum}; a part's mark tag may be missing`, q);
    }
    // Figures: every referenced file must exist and not be a sliver.
    for (const m of content.matchAll(/!\[[^\]]*\]\(([^)]+)\)/g)) {
      const ref = m[1];
      const file = path.isAbsolute(ref) || existsSync(ref) ? ref : path.join(cardsDir, ref);
      if (!existsSync(file)) {
        add("fail", "figure-missing", `image file not found: ${ref}`, q);
        continue;
      }
      const size = pngSize(file);
      if (size && (size.w < 8 || size.h < 8)) add("fail", "figure-blank", `figure is ${size.w}x${size.h}px`, q);
      else if (!size && statSync(file).size < 300) add("fail", "figure-blank", "figure file is almost empty", q);
    }
    if (/\bFigure\s+\d+\b/i.test(content) && !/!\[/.test(content)) {
      add("warn", "figure-mentioned-no-image", "text mentions a figure but the card has no image", q);
    }
    if (c.needs_review) add("flag", "needs-review", "pipeline flagged this card for review", q);
  }
  stats.flaggedCards = flagged;

  if (cert) {
    if ((cert.quarantined ?? 0) > 0) add("flag", "quarantined", `${cert.quarantined} card(s) quarantined`);
    if ((cert.placeholderFallbackCards ?? 0) > 0) add("flag", "placeholder", `${cert.placeholderFallbackCards} placeholder card(s)`);
  }

  // 3. Printed paper total vs sum of card marks.
  const marksSum = cards.reduce((a, c) => a + (Number.isFinite(c.marks) ? c.marks : 0), 0);
  stats.marksSum = marksSum;
  // A choose-one paper prints the total of the questions a candidate
  // answers: count one alternative of each choice the import recorded, when
  // the paper's own text prints a choice instruction. Alternatives must carry
  // the same marks; marks lost from one of them fail the paper.
  let compared = marksSum;
  const pages = sourcePdf ? pdfPages(sourcePdf) : null;
  const choiceGroups = cert?.choiceGroups ?? [];
  if (choiceGroups.length && !(pages && CHOICE_INSTRUCTION.test(pages.join(" ")))) {
    add("warn", "choice-not-printed", `the import recorded alternatives ${JSON.stringify(choiceGroups)}, but the paper's text prints no choice instruction`);
  } else if (choiceGroups.length) {
    const byQuestion = new Map(cards.map((c) => [Number(c.question_number), Number.isFinite(c.marks) ? c.marks : 0]));
    for (const group of choiceGroups) {
      const sums = group.map((alt) => alt.reduce((a, q) => a + (byQuestion.get(q) ?? 0), 0));
      compared -= sums.reduce((a, b) => a + b, 0) - Math.max(...sums);
      if (new Set(sums).size > 1) add("fail", "choice-alternatives-differ", `alternatives ${JSON.stringify(group)} carry ${sums.join(" / ")} marks`);
    }
    stats.marksAnswered = compared;
  }
  if (!pages) {
    add("warn", "no-source-text", "could not read the source PDF text; source checks skipped");
  } else {
    stats.pages = pages.length;
    const totals = printedTotals(pages);
    stats.printedTotals = totals.candidates;
    if (totals.candidates.length === 0) {
      stats.marksTotal = "UNKNOWN";
    } else if (totals.candidates.includes(compared)) {
      stats.marksTotal = "MATCH";
    } else {
      stats.marksTotal = "MISMATCH";
      // Cards above the printed total can be a choose-one-question paper
      // whose choice was not recognised; cards below it mean marks or whole
      // questions were lost.
      const direction = compared > Math.max(...totals.candidates) ? "over" : "under";
      stats.marksTotalDirection = direction;
      const answered = compared === marksSum ? "" : ` (${compared} with one alternative of each choice)`;
      add("fail", `marks-total-${direction}`, `cards sum to ${marksSum}${answered}; the paper prints ${totals.candidates.join(" / ")}`);
    }
    // Papers that print "(Total for Question N is M marks)" give an exact
    // per-question oracle for the card's mark value.
    const perQuestion = new Map();
    for (const m of pages.join(" ").matchAll(/Total\s+for\s+Question\s+(\d+)\s+is\s+(\d+)\s+marks?/gi)) {
      perQuestion.set(Number(m[1]), Number(m[2]));
    }
    stats.printedQuestionTotals = perQuestion.size;
    for (const c of cards) {
      const printed = perQuestion.get(Number(c.question_number));
      if (printed !== undefined && printed !== c.marks) {
        add("fail", "question-marks-mismatch", `card marks=${c.marks}; the paper prints ${printed}`, c.question_number);
      }
    }
    const coverage = textCoverage(pages, cards);
    stats.textCoverage = coverage === null ? null : Number(coverage.toFixed(3));
    if (coverage !== null && coverage < 0.6) add("warn", "low-text-coverage", `only ${(coverage * 100).toFixed(0)}% of the source's distinctive words appear in the cards`);
  }

  return { findings, stats };
}

export function classify(findings, runFailures = []) {
  if (runFailures.length || findings.some((f) => f.severity === "fail")) return "FAIL";
  if (findings.some((f) => f.severity === "flag")) return "FLAGGED";
  if (findings.some((f) => f.severity === "warn")) return "REVIEW";
  return "PASS";
}
