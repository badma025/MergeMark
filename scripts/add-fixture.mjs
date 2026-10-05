// node scripts/add-fixture.mjs name lo hi "note"  -> adds a reviewed expected-IDs entry tied to the file hash
import { readFileSync, writeFileSync } from "node:fs";
import { PAPERS, sha256File } from "./lib/zero-cost-accounting.mjs";
const [name, lo, hi, note] = process.argv.slice(2);
const fx = JSON.parse(readFileSync("scripts/fixtures/source-expected-ids.json", "utf8"));
const p = PAPERS.find((x) => x.name === name);
if (!p) throw new Error(`no paper ${name}`);
const ids = Array.from({ length: Number(hi) - Number(lo) + 1 }, (_, i) => Number(lo) + i);
fx.papers[name] = { file: p.file, fileHash: sha256File(p.file), cohort: p.cohort, style: "numbered", confidence: "MEDIUM", expectedIds: ids, candidateIds: ids.map(String), note };
writeFileSync("scripts/fixtures/source-expected-ids.json", JSON.stringify(fx, null, 2) + "\n");
console.log(name, ids.length);
