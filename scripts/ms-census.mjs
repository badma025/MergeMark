// The distribution of the local mark-scheme reader's refusals over a census.
//   node scripts/ms-census.mjs <dir of *_ms_census.json> [--pages]
// Each census (e2e_import --mark-scheme --census) lists every reason the
// reader refuses its scheme, page by page, as read and with rotated pages
// turned upright. A page is clear when no reason fires on it; a reason is a
// page's sole reason when it is the only one there, so the pages it alone
// blocks are those that clearing it would open.
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";

const [dir, ...flags] = process.argv.slice(2);
const showPages = flags.includes("--pages");
if (!dir) {
  console.error("usage: node scripts/ms-census.mjs <dir of *_ms_census.json> [--pages]");
  process.exit(2);
}
const censuses = readdirSync(dir)
  .filter((f) => f.endsWith("_ms_census.json"))
  .sort()
  .map((f) => JSON.parse(readFileSync(path.join(dir, f), "utf8")));

/// A reason without the detail that varies within it (a glyph's index, a key).
function category(reason) {
  return reason
    .replace(/^unmapped glyph \d+$/, "unmapped glyph")
    .replace(/^invalid question key: .*$/s, "invalid question key")
    .replace(/^.*: missing answer or marks$/s, "missing answer or marks");
}

function byPage(refusals) {
  const pages = new Map();
  const whole = new Set();
  for (const r of refusals) {
    if (r.page == null) whole.add(category(r.reason));
    else pages.set(r.page, new Set([...(pages.get(r.page) ?? []), category(r.reason)]));
  }
  return { pages, whole };
}

function tally(reading) {
  const rows = new Map();
  for (const c of censuses) {
    const { pages, whole } = byPage(c[reading]);
    for (const [page, cats] of pages) {
      for (const cat of cats) {
        const row = rows.get(cat) ?? { schemes: new Set(), pages: 0, sole: 0, where: [] };
        row.schemes.add(c.paper);
        row.pages += 1;
        if (cats.size === 1) row.sole += 1;
        row.where.push(`${c.paper} p${page}`);
        rows.set(cat, row);
      }
    }
    for (const cat of whole) {
      const row = rows.get(cat) ?? { schemes: new Set(), pages: 0, sole: 0, where: [] };
      row.schemes.add(c.paper);
      row.where.push(`${c.paper} (document)`);
      rows.set(cat, row);
    }
  }
  return [...rows].sort((a, b) => b[1].pages - a[1].pages || a[0].localeCompare(b[0]));
}

console.log("| scheme | pages | rotated | pages refused, as read | pages refused, rotated pages turned | document-level reasons |");
console.log("|---|---|---|---|---|---|");
for (const c of censuses) {
  const asRead = byPage(c.asRead);
  const turned = byPage(c.rotatedPagesTurned);
  console.log(`| ${c.paper} | ${c.pages} | ${c.rotatedPages.length} | ${asRead.pages.size} | ${turned.pages.size} | ${[...turned.whole].join("; ") || "none"} |`);
}

for (const [reading, title] of [["asRead", "as read"], ["rotatedPagesTurned", "rotated pages turned upright"]]) {
  console.log(`\n**Reasons, ${title}** (pages a reason fires on; sole = pages where it is the only reason)\n`);
  console.log("| reason | schemes | pages | sole |");
  console.log("|---|---|---|---|");
  for (const [cat, row] of tally(reading)) {
    console.log(`| ${cat} | ${row.schemes.size} | ${row.pages} | ${row.sole} |`);
    if (showPages) console.log(`|  | ${row.where.join(", ")} | | |`);
  }
}
