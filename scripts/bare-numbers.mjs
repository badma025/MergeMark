// Lines holding only a number that a card gains between two runs, with the
// line before and after: options printed as numbers, or page numbers that
// leaked into a card. Works on triage and calibration run directories.
//   node scripts/bare-numbers.mjs <run dir A> <run dir B>
import fs from "node:fs";
import path from "node:path";

const [a, b] = process.argv.slice(2).map((p) => path.resolve(p));
const stampOf = (dir) => path.basename(dir).replace(/^run-/, "");

function cards(run) {
  const stamp = stampOf(run);
  const out = new Map();
  for (const e of fs.readdirSync(run, { withFileTypes: true })) {
    if (!e.isDirectory() || !e.name.endsWith(`-${stamp}`)) continue;
    const paper = e.name.slice(0, -(stamp.length + 1));
    const file = path.join(run, e.name, paper, `${paper}_cards.json`);
    if (fs.existsSync(file)) out.set(paper, JSON.parse(fs.readFileSync(file, "utf8")));
  }
  return out;
}

const bare = /^\s*\d{1,3}\s*$/;
const count = (lines) => lines.reduce((m, l) => (bare.test(l) ? m.set(l.trim(), (m.get(l.trim()) ?? 0) + 1) : m), new Map());
const A = cards(a), B = cards(b);
let gained = 0;
for (const [paper, bc] of B) {
  const ac = A.get(paper);
  if (!ac) continue;
  for (const card of bc) {
    const before = ac.find((c) => c.question_number === card.question_number);
    if (!before) continue;
    const lines = card.content.split("\n");
    const had = count(before.content.split("\n"));
    const seen = new Map();
    lines.forEach((l, i) => {
      if (!bare.test(l)) return;
      const k = l.trim();
      seen.set(k, (seen.get(k) ?? 0) + 1);
      if (seen.get(k) <= (had.get(k) ?? 0)) return;
      gained++;
      console.log(`${paper} Q${card.question_number} (strict=${!card.needs_review}): ${JSON.stringify(lines[i - 1] ?? "")} | ${k} | ${JSON.stringify(lines[i + 1] ?? "")}`);
    });
  }
}
console.log(`bare-number lines gained: ${gained}`);
