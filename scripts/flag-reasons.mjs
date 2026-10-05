// Per paper: the gate reasons that flagged its cards, and its hard findings.
//   node scripts/flag-reasons.mjs unused-c5
import fs from "node:fs";
const p = `output/triage/run-${process.argv[2]}/`;
const rows = JSON.parse(fs.readFileSync(p + "results.json", "utf8"));
const total = {}, combos = {};
for (const r of rows) {
  let log = "";
  try { log = fs.readFileSync(p + "logs/" + r.slug + ".log", "utf8"); } catch {}
  const reasons = [...log.matchAll(/TIER0_FALLBACK\] question=\d+ reason=([a-z_]+)/g)].map((m) => m[1]);
  for (const x of reasons) total[x] = (total[x] || 0) + 1;
  if (r.status === "PASS") continue;
  const fails = [...new Set(r.findings.filter((f) => f.severity === "fail").map((f) => f.code))];
  const key = [...new Set(reasons)].sort().join("+") + (fails.length ? " |FAIL:" + fails.join(",") : "");
  combos[key] = (combos[key] || 0) + 1;
}
console.log("cards flagged, by gate reason:");
Object.entries(total).sort((a, b) => b[1] - a[1]).forEach(([k, v]) => console.log(`  ${v}\t${k}`));
console.log("\nnon-PASS papers, by reason combination:");
Object.entries(combos).sort((a, b) => b[1] - a[1]).forEach(([k, v]) => console.log(`  ${v}\t${k || "(no gate reason)"}`));
