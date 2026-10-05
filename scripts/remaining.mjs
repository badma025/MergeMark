// For a triage run: every hard finding, and whether the card it names is flagged for review.
//   node scripts/remaining.mjs unused-c<N>
import { readFileSync } from "node:fs";
const run = process.argv[2];
const rows = JSON.parse(readFileSync(`output/triage/run-${run}/results.json`, "utf8"));
const out = new Map();
for (const r of rows) {
  let cards = [];
  try { cards = JSON.parse(readFileSync(r.cardsJson, "utf8")); } catch {}
  for (const f of r.findings.filter((x) => x.severity === "fail")) {
    const k = f.code;
    if (!out.has(k)) out.set(k, { findings: 0, cardLevel: 0, flagged: 0, papers: new Set(), paperFlagged: 0 });
    const o = out.get(k);
    o.findings += 1;
    o.papers.add(r.file);
    if (f.question != null) {
      o.cardLevel += 1;
      if (cards.find((c) => Number(c.question_number) === Number(f.question))?.needs_review) o.flagged += 1;
    }
  }
}
for (const [k, o] of out) {
  const paperFlagged = [...o.papers].filter((f) => { const r = rows.find((x) => x.file === f); try { return JSON.parse(readFileSync(r.cardsJson, "utf8")).some((c) => c.needs_review); } catch { return false; } }).length;
  console.log(`${k}\tpapers=${o.papers.size}\tfindings=${o.findings}\tcard-level=${o.cardLevel}\tcard flagged=${o.flagged}\tpapers with any flagged card=${paperFlagged}`);
}
