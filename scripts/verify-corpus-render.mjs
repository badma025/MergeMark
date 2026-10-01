// Render every imported card through the frontend's exam-Markdown
// preprocessing, react-markdown (GFM + math) and KaTeX in strict mode, and
// report anything that would not display cleanly.
//
// Usage: node scripts/verify-corpus-render.mjs <run-dir-or-cards.json> [...]
//   A run directory is scanned for *_cards.json up to two folders deep (one
//   folder per paper, as the r1b loop writes, or <paper>-<stamp>/<paper>/ as
//   the calibration runner does). Finding no cards at all is a failure.
// Writes render-audit.json next to the first input and exits non-zero when
// any card fails.
import fs from 'node:fs/promises';
import path from 'node:path';
import { build } from 'esbuild';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import Markdown from 'react-markdown';
import remarkMath from 'remark-math';
import remarkGfm from 'remark-gfm';
import rehypeKatex from 'rehype-katex';

const inputs = process.argv.slice(2);
if (!inputs.length) throw new Error('Usage: node scripts/verify-corpus-render.mjs <run-dir|cards.json> [...]');

const bundle = await build({ entryPoints: ['src/lib/preprocess-math.ts'], bundle: true, write: false, format: 'esm', platform: 'node' });
const { preprocessExamMarkdown } = await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString('base64')}`);

async function cardFiles(p, depth = 2) {
  const stat = await fs.stat(p);
  if (stat.isFile()) return [p];
  const out = [];
  for (const entry of await fs.readdir(p, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const dir = path.join(p, entry.name);
    for (const f of await fs.readdir(dir, { withFileTypes: true })) {
      if (f.isFile() && f.name.endsWith('_cards.json')) out.push(path.join(dir, f.name));
    }
    if (depth > 1) out.push(...(await cardFiles(dir, depth - 1)).filter((f) => path.dirname(f) !== dir));
  }
  return [...new Set(out)].sort();
}

// KaTeX reports problems through console warnings in non-strict modes and
// through .katex-error spans when it cannot parse; capture both.
const warnings = [];
const originalWarn = console.warn;
console.warn = (...args) => { warnings.push(args.join(' ')); };

const render = (text) => renderToStaticMarkup(React.createElement(Markdown, {
  remarkPlugins: [remarkGfm, remarkMath],
  rehypePlugins: [[rehypeKatex, { strict: 'error', throwOnError: false }]],
  children: text,
}));

const report = [];
let failures = 0;
for (const input of inputs) {
  for (const file of await cardFiles(input)) {
    const cards = JSON.parse(await fs.readFile(file, 'utf8'));
    const paper = path.basename(file).replace(/_cards\.json$/, '');
    const paperRows = [];
    for (const card of cards) {
      const errors = [];
      warnings.length = 0;
      const content = preprocessExamMarkdown(card.content);
      // The app renders the stem as Markdown and each tagged option body in
      // its own option card (as verify-physics24-render.mjs does).
      const options = [...content.matchAll(/^- \[MCQ:([A-E])\] (.*)$/gm)];
      const stem = content.slice(0, options[0]?.index ?? content.length);
      let html = '';
      try {
        html = render(stem) + options.map(m => render(m[2])).join('');
      } catch (e) {
        errors.push(`render threw: ${e.message}`);
      }
      for (const m of html.matchAll(/<span class="katex-error"[^>]*title="([^"]*)"/g)) errors.push(`KaTeX: ${m[1].replace(/&#x27;/g, "'").slice(0, 160)}`);
      for (const w of warnings) errors.push(`KaTeX warning: ${w.slice(0, 160)}`);
      if (html.includes('[MCQ:')) errors.push('Leaked MCQ marker');
      const repeated = content.search(/\${3,}/);
      if (repeated >= 0) errors.push(`Repeated math delimiters near ${JSON.stringify(content.slice(Math.max(0, repeated - 60), repeated + 60))}`);
      if (/[-]/u.test(content)) errors.push('Unmapped private-use glyph');
      if (content.includes('�')) errors.push('Unidentified source glyph (U+FFFD)');
      // A Markdown table must render as a <table>.
      if (/^\|.*\|\s*$/m.test(content) && /^\|\s*-{3}/m.test(content) && !html.includes('<table')) errors.push('Markdown table did not render as a table');
      if (errors.length) failures += 1;
      paperRows.push({ question: card.question_number, needs_review: card.needs_review, errors });
    }
    const bad = paperRows.filter(r => r.errors.length);
    report.push({ paper, file, cards: paperRows.length, failing: bad.length, rows: bad });
    console.log(`${paper}: ${paperRows.length - bad.length}/${paperRows.length} render cleanly`);
    for (const r of bad) console.log(`  Q${r.question}: ${r.errors.join('; ')}`);
  }
}
console.warn = originalWarn;
// (A run directory with no cards in it proves nothing.)
const checked = report.reduce((n, r) => n + r.cards, 0);
if (checked === 0) {
  console.error(`no *_cards.json found under ${inputs.join(', ')}`);
  failures += 1;
}
const outPath = path.join((await fs.stat(inputs[0])).isFile() ? path.dirname(inputs[0]) : inputs[0], 'render-audit.json');
await fs.writeFile(outPath, JSON.stringify({ failures, papers: report }, null, 2));
console.log(`\n${checked} card(s) checked, ${failures} failing; audit written to ${outPath}`);
if (failures) process.exitCode = 1;
