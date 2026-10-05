// Blind slices of an audit pack, and the merged verdict table.
//   node scripts/blind.mjs split <pack dir> [slices=4]
//   node scripts/blind.mjs merge <pack dir>
// A slice carries only what an auditor needs: no stratum, paper status or
// automatic findings.
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import path from "node:path";

const [cmd, dir, n = "4"] = process.argv.slice(2);
const manifest = JSON.parse(readFileSync(path.join(dir, "manifest.json"), "utf8"));
const items = manifest.items;

if (cmd === "split") {
  const k = Number(n);
  // Interleave so each slice mixes strata.
  const slices = Array.from({ length: k }, () => []);
  items.forEach((it, i) => slices[i % k].push({
    id: it.id, paper: it.paper, question: it.question, cardMarksField: it.marks,
    cardFile: it.cardFile, pagePngs: it.pagePngs, figurePngs: it.figurePngs,
  }));
  slices.forEach((s, i) => writeFileSync(path.join(dir, `blind-slice-${i + 1}.json`), JSON.stringify(s, null, 2)));
  console.log(slices.map((s) => s.length).join(" + "), "items");
} else if (cmd === "merge") {
  const verdicts = new Map();
  for (let i = 1; existsSync(path.join(dir, `verdicts-${i}.json`)); i += 1) {
    for (const v of JSON.parse(readFileSync(path.join(dir, `verdicts-${i}.json`), "utf8"))) verdicts.set(v.id, v);
  }
  const merged = items.map((it) => ({ ...it, ...(verdicts.get(it.id) ?? { verdict: "MISSING" }) }));
  writeFileSync(path.join(dir, "merged.json"), JSON.stringify(merged, null, 2));
  const strata = [...new Set(merged.map((m) => m.stratum))];
  console.log("| sample group | cards | CLEAN | MINOR | MAJOR | missing |\n|---|---|---|---|---|---|");
  const row = (name, ms) => {
    const c = (v) => ms.filter((m) => m.verdict === v).length;
    console.log(`| ${name} | ${ms.length} | ${c("CLEAN")} | ${c("MINOR")} | ${c("MAJOR")} | ${c("MISSING")} |`);
  };
  for (const s of strata) row(s, merged.filter((m) => m.stratum === s));
  row("**all**", merged);
  const fam = new Map();
  for (const m of merged.filter((x) => x.verdict === "MAJOR")) {
    for (const t of new Set((m.issues ?? []).filter((i) => i.severity === "MAJOR").map((i) => i.type))) fam.set(t, (fam.get(t) ?? 0) + 1);
  }
  console.log("\nMAJOR issue types (cards):", JSON.stringify(Object.fromEntries([...fam].sort((a, b) => b[1] - a[1]))));
}
