// Status and key counts per run on the papers every run contains.
//   node scripts/trajectory.mjs unused-001 unused-c1 … unused-c<N>   (stamps of output/triage/run-<stamp>)
import { readFileSync, existsSync } from "node:fs";
const runs = process.argv.slice(2);
const load = (r) => new Map(JSON.parse(readFileSync(`output/triage/run-${r}/results.json`, "utf8")).map((x) => [x.file, x]));
const R = runs.filter((r) => existsSync(`output/triage/run-${r}/results.json`)).map((r) => [r, load(r)]);
const common = [...R[0][1].keys()].filter((f) => R.every(([, m]) => m.has(f)));
console.log(`papers in every run: ${common.length}`);
console.log("run\tPASS\tREVIEW\tFLAGGED\tFAIL\ttotalMATCH\tmarksUnder\tzeroMarkCards\tglyphPapers\tglyphFindings\tcards\tflaggedCards\tnoCards");
for (const [r, m] of R) {
  const rows = common.map((f) => m.get(f));
  const st = (s) => rows.filter((x) => x.status === s).length;
  const papers = (c) => rows.filter((x) => x.findings.some((f) => f.code === c)).length;
  const finds = (c) => rows.reduce((a, x) => a + x.findings.filter((f) => f.code === c).length, 0);
  console.log([r, st("PASS"), st("REVIEW"), st("FLAGGED"), st("FAIL"), rows.filter((x) => x.stats.marksTotal === "MATCH").length, papers("marks-total-under"), finds("no-marks"), papers("unidentified-glyph"), finds("unidentified-glyph"), rows.reduce((a, x) => a + (x.stats.cards ?? 0), 0), rows.reduce((a, x) => a + (x.stats.flaggedCards ?? 0), 0), papers("no-cards")].join("\t"));
}
