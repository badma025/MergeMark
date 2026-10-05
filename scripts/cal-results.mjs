// A triage-style results.json for a calibration run, so that
// scripts/build-audit-pack.mjs (with --ids and --papers papers/used) can
// build a targeted audit pack of fixture cards.
//   node scripts/cal-results.mjs output/zero-cost-orchestration/c-calibration/run-<stamp>
import fs from "node:fs";
import path from "node:path";
import { PAPERS } from "./lib/zero-cost-accounting.mjs";

const run = process.argv[2];
const stamp = path.basename(run).replace(/^run-/, "");
const rows = [];
for (const p of PAPERS) {
  const cardsJson = path.join(run, `${p.name}-${stamp}`, p.name, `${p.name}_cards.json`);
  if (!fs.existsSync(cardsJson)) continue;
  const cards = JSON.parse(fs.readFileSync(cardsJson, "utf8"));
  rows.push({ slug: p.name, name: p.name, file: path.basename(p.file), subject: p.subject, status: "CALIBRATION", findings: [], stats: { cards: cards.length }, cardsJson });
}
fs.writeFileSync(path.join(run, "results.json"), JSON.stringify(rows, null, 2));
console.log(`${rows.length} rows -> ${path.join(run, "results.json")}`);
