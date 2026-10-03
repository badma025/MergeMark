// node --test scripts/tests/triage-invariants.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { checkPaper } from "../lib/triage-invariants.mjs";

// AQA Economics Jun 2021 prints a total of 80: Context 1 (Q1-4) or Context 2
// (Q5-8), 40 marks each, and one of three 40-mark essays (Q9-10, Q11-12,
// Q13-14). Its cards carry every alternative: 200 marks.
const pdf = "papers/used/Economics_A-level_Past-Papers_AQA__June_2021_QP.pdf";
const marks = { 1: 2, 2: 4, 3: 9, 4: 25, 5: 2, 6: 4, 7: 9, 8: 25, 9: 15, 10: 25, 11: 15, 12: 25, 13: 15, 14: 25 };
const cards = Object.entries(marks).map(([q, m]) => ({ question_number: Number(q), marks: m, content: `Question ${q}. [${m} marks]`, needs_review: false }));
const choiceGroups = [[[1, 2, 3, 4], [5, 6, 7, 8]], [[9, 10], [11, 12], [13, 14]]];

test("a choose-one paper is compared with the questions a candidate answers", { skip: !existsSync(pdf) }, () => {
  const { stats, findings } = checkPaper({ cards, sourcePdf: pdf, cardsDir: ".", cert: { choiceGroups } });
  assert.equal(stats.marksSum, 200);
  assert.equal(stats.marksAnswered, 80);
  assert.equal(stats.marksTotal, "MATCH");
  assert.ok(!findings.some((f) => /marks-total/.test(f.code)), JSON.stringify(findings));
});

test("without its choices the same paper is over its printed total", { skip: !existsSync(pdf) }, () => {
  const { stats, findings } = checkPaper({ cards, sourcePdf: pdf, cardsDir: ".", cert: {} });
  assert.equal(stats.marksTotal, "MISMATCH");
  assert.ok(findings.some((f) => f.code === "marks-total-over"));
});

test("parts printed as EITHER … OR … count once against the card's marks", () => {
  // Edexcel A Economics Jun 2018 Q6: (a)-(e) and EITHER (f) OR (g), 60 marks.
  const content = "(a) Explain.\n**[4 marks]**\n(b) Assess.\n**[10 marks]**\n(c) Explain.\n**[5 marks]**\n(d) Explain.\n**[6 marks]**\n(e) Discuss.\n**[15 marks]**\nEITHER\n(f) Evaluate.\n**[20 marks]**\nOR\n(g) Evaluate.\n**[20 marks]**";
  const { findings } = checkPaper({ cards: [{ question_number: 6, marks: 60, content, needs_review: false }], sourcePdf: null, cardsDir: ".", cert: {} });
  assert.ok(!findings.some((f) => /marks-tag/.test(f.code)), JSON.stringify(findings));
});

test("alternatives carrying different marks fail the paper", { skip: !existsSync(pdf) }, () => {
  const uneven = cards.map((c) => (c.question_number === 12 ? { ...c, marks: 20 } : c));
  const { findings } = checkPaper({ cards: uneven, sourcePdf: pdf, cardsDir: ".", cert: { choiceGroups } });
  assert.ok(findings.some((f) => f.code === "choice-alternatives-differ" && f.severity === "fail"), JSON.stringify(findings));
});

test("a part printed after the alternatives counts on its own", () => {
  const content = "EITHER\n(f) Evaluate.\n**[25 marks]**\nOR\n(g) Evaluate.\n**[20 marks]**\n(h) Explain.\n**[5 marks]**";
  const { findings } = checkPaper({ cards: [{ question_number: 6, marks: 30, content, needs_review: false }], sourcePdf: null, cardsDir: ".", cert: {} });
  assert.ok(!findings.some((f) => /marks-tag/.test(f.code)), JSON.stringify(findings));
});

// OCR A Chemistry Jun 2022 prints no choice instruction.
const noChoicePdf = "papers/used/Chemistry_A-level_Past-Papers_OCR-A__June_2022_QP.pdf";
test("alternatives the paper does not print are not counted", { skip: !existsSync(noChoicePdf) }, () => {
  const { stats, findings } = checkPaper({ cards, sourcePdf: noChoicePdf, cardsDir: ".", cert: { choiceGroups } });
  assert.equal(stats.marksAnswered, undefined);
  assert.ok(findings.some((f) => f.code === "choice-not-printed"), JSON.stringify(findings));
});
