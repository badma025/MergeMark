// Compare two triage runs on the papers both contain.
//   node scripts/compare.mjs output/triage/run-unused-001 output/triage/run-unused-c1
import { readFileSync } from "node:fs";

const [a, b] = process.argv.slice(2);
const load = (d) => new Map(JSON.parse(readFileSync(`${d}/results.json`, "utf8")).map((r) => [r.file, r]));
const A = load(a);
const B = load(b);
const common = [...A.keys()].filter((f) => B.has(f)).sort();
const statuses = ["PASS", "REVIEW", "FLAGGED", "FAIL"];
const count = (M) => Object.fromEntries(statuses.map((s) => [s, common.filter((f) => M.get(f).status === s).length]));
console.log(`papers in both runs: ${common.length} (only in before: ${[...A.keys()].filter((f) => !B.has(f)).length}, only in after: ${[...B.keys()].filter((f) => !A.has(f)).length})`);
console.log("before", JSON.stringify(count(A)));
console.log("after ", JSON.stringify(count(B)));

const codes = (M) => {
  const m = new Map();
  for (const f of common) {
    for (const x of M.get(f).findings) {
      const k = `${x.severity}:${x.code}`;
      if (!m.has(k)) m.set(k, { papers: new Set(), cards: 0 });
      m.get(k).papers.add(f);
      m.get(k).cards += 1;
    }
  }
  return m;
};
const ca = codes(A);
const cb = codes(B);
console.log("\ncode\tpapers before->after\tfindings before->after");
for (const k of [...new Set([...ca.keys(), ...cb.keys()])].sort()) {
  const x = ca.get(k) ?? { papers: new Set(), cards: 0 };
  const y = cb.get(k) ?? { papers: new Set(), cards: 0 };
  console.log(`${k}\t${x.papers.size} -> ${y.papers.size}\t${x.cards} -> ${y.cards}`);
}

const trans = new Map();
for (const f of common) {
  const k = `${A.get(f).status} -> ${B.get(f).status}`;
  trans.set(k, [...(trans.get(k) ?? []), f]);
}
console.log("\ntransitions");
for (const [k, fs] of [...trans.entries()].sort()) {
  console.log(`${k}: ${fs.length}`);
  if (!k.startsWith("PASS -> PASS") && k.split(" -> ")[0] !== k.split(" -> ")[1] && fs.length <= 25) for (const f of fs) console.log(`    ${f}`);
}
const sum = (M) => common.reduce((s, f) => s + (M.get(f).stats.marksTotal === "MATCH" ? 1 : 0), 0);
console.log(`\nprinted total MATCH: ${sum(A)} -> ${sum(B)}`);
const flagged = (M) => common.reduce((s, f) => s + (M.get(f).stats.flaggedCards ?? 0), 0);
const cards = (M) => common.reduce((s, f) => s + (M.get(f).stats.cards ?? 0), 0);
console.log(`cards: ${cards(A)} -> ${cards(B)}; flagged cards: ${flagged(A)} -> ${flagged(B)}`);
