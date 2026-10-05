// Per changed card between two runs: marks, review flag, and the lines it
// lost and gained (diagram links masked). Works on triage and calibration
// run directories.
//   node scripts/card-changes.mjs <run dir A> <run dir B> [max lines per card]
import fs from "node:fs";
import path from "node:path";

const [a, b, maxArg] = process.argv.slice(2);
const max = Number(maxArg ?? 6);
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
const mask = (s) => String(s).replace(/!\[Diagram\]\([^)]*\)/g, "![Diagram](<D>)");
const lines = (s) => mask(s).split("\n").map((l) => l.trim()).filter(Boolean);
function minus(x, y) {
  const left = new Map();
  for (const l of y) left.set(l, (left.get(l) ?? 0) + 1);
  return x.filter((l) => (left.get(l) ?? 0) > 0 ? (left.set(l, left.get(l) - 1), false) : true);
}
const A = cards(path.resolve(a)), B = cards(path.resolve(b));
let changed = 0;
for (const [paper, bc] of B) {
  const ac = A.get(paper);
  if (!ac) continue;
  const numbers = new Set([...ac, ...bc].map((c) => c.question_number));
  for (const n of [...numbers].sort((x, y) => x - y)) {
    const x = ac.find((c) => c.question_number === n), y = bc.find((c) => c.question_number === n);
    if (!x || !y) {
      changed++;
      console.log(`${paper} Q${n}: ${x ? "only before" : "only after"}`);
      continue;
    }
    if (mask(x.content) === mask(y.content) && x.marks === y.marks && x.needs_review === y.needs_review) continue;
    changed++;
    const lost = minus(lines(x.content), lines(y.content)), got = minus(lines(y.content), lines(x.content));
    const head = `${paper} Q${n}: marks ${x.marks}->${y.marks} review ${x.needs_review}->${y.needs_review}`;
    console.log(head);
    for (const l of lost.slice(0, max)) console.log(`   - ${l.slice(0, 150)}`);
    if (lost.length > max) console.log(`   - … ${lost.length - max} more`);
    for (const l of got.slice(0, max)) console.log(`   + ${l.slice(0, 150)}`);
    if (got.length > max) console.log(`   + … ${got.length - max} more`);
  }
}
console.log(`changed cards: ${changed}`);
