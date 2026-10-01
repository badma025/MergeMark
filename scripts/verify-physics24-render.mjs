import fs from 'node:fs/promises';
import path from 'node:path';
import { build } from 'esbuild';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import Markdown from 'react-markdown';
import remarkMath from 'remark-math';
import remarkGfm from 'remark-gfm';
import rehypeKatex from 'rehype-katex';

const input = process.argv[2];
if (!input) throw new Error('Usage: node scripts/verify-physics24-render.mjs <cards.json>');
const bundle = await build({ entryPoints: ['src/lib/preprocess-math.ts'], bundle: true, write: false, format: 'esm', platform: 'node' });
const { preprocessExamMarkdown } = await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString('base64')}`);
const cards = JSON.parse(await fs.readFile(input, 'utf8'));
const results = [];
const sections = [];
const expectedMarks = [9, 9, 7, 5, 8, 12, 10];
for (const card of cards) {
  const q = card.question_number;
  const content = preprocessExamMarkdown(card.content).replace(/(!\[[^\]]*\]\()([^)]*[/\\]diagrams[/\\])([^)]*\))/g, '$1diagrams/$3');
  const errors = [];
  const options = [...content.matchAll(/^- \[MCQ:([A-E])\] (.*)$/gm)];
  const images = [...content.matchAll(/!\[[^\]]*\]\(([^)]+)\)/g)].map(m => m[1]);
  if (q === 2 && (images.length !== 4 || new Set(images).size !== 4)) errors.push('Expected four distinct capacitor figures');
  if (q === 14 && images.length !== 1) errors.push('Expected only the oscilloscope diagram');
  if (q === 26 && images.length !== 4) errors.push('Expected four charge diagrams');
  if (q >= 8 && options.map(m => m[1]).join('') !== 'ABCD') errors.push('Expected exactly A, B, C, D');
  if (q >= 8 && new Set(options.map(m => m[2])).size !== options.length) errors.push('Duplicate option bodies');
  const expected = {
    1: ['\\frac{\\text{mass of water}}{\\text{mass of air}}'],
    14: ['0.30\\pi', '0.15\\pi'],
    16: ['\\frac{2\\pi m_e}{Be}', '\\frac{v}{\\pi r}', '\\frac{Be}{2\\pi m_e}'],
    21: ['\\frac{R}{\\sqrt[3]{2}}', '\\frac{R}{\\sqrt[3]{16}}', '\\frac{R}{2}', '\\frac{\\sqrt{2}R}{8}'],
    22: ['10^{3}', '10^{4}', '10^{5}', '10^{6}'],
    // The source prints the antineutrino with a Latin upright "v" (U+0076,
    // TimesNewRomanPSMT; visually a serif v, not a Greek nu), so the
    // source-faithful transcription is accepted alongside the semantic one.
    31: ['^{238}_{92}', ['\\bar{\\nu}_e', '\\bar{\\mathrm{v}}_e']],
  };
  for (const required of expected[q] ?? []) {
    const alternatives = Array.isArray(required) ? required : [required];
    if (!alternatives.some(r => content.includes(r))) errors.push(`Missing source expression: ${alternatives.join(' or ')}`);
  }
  if (q <= 7) {
    const expectedParts = [4,4,2,3,3,4,4][q-1];
    for (let i=0; i<expectedParts; i++) if (!content.includes(`(${String.fromCharCode(97+i)})`)) errors.push(`Missing subpart ${i+1}`);
    const expectedPartMarks = [[1,3,2,3],[2,2,3,2],[3,4],[1,2,2],[3,2,3],[3,4,2,3],[1,5,2,2]][q-1];
    const partMarks = [...content.matchAll(/\*\*\[(\d+) marks?\]\*\*/g)].map(m => Number(m[1]));
    if (partMarks.join(',') !== expectedPartMarks.join(',')) errors.push(`Subpart marks ${partMarks}, expected ${expectedPartMarks}`);
  }
  for (const m of content.matchAll(/!\[[^\]]*\]\((diagrams\/[^)]+)\)/g)) {
    try { await fs.access(path.join(path.dirname(input), m[1])); } catch { errors.push(`Missing image ${m[1]}`); }
  }
  if (q <= 7 && card.marks !== expectedMarks[q-1]) errors.push(`Marks ${card.marks}, expected ${expectedMarks[q-1]}`);
  if (card.needs_review) errors.push('Import flagged needs_review');
  if (/[\uE000-\uF8FF]/u.test(content)) errors.push('Unmapped symbol-font glyphs');
  const render = text => renderToStaticMarkup(React.createElement(Markdown, { remarkPlugins: [remarkGfm, remarkMath], rehypePlugins: [[rehypeKatex, { strict: 'error' }]], children: text }));
  const start = options[0]?.index ?? content.length;
  const stem = content.slice(0, start);
  const html = render(stem) + '<div class="options">' + options.map(m => `<div><strong>${m[1]}</strong>${render(m[2])}</div>`).join('') + '</div>';
  if (html.includes('katex-error')) errors.push('KaTeX error');
  if (html.includes('[MCQ:')) errors.push('Leaked MCQ marker');
  if (/\${3,}/.test(content)) errors.push('Repeated math delimiters');
  if (q >= 8 && /!\[/.test(content.slice(start).replace(/^- \[MCQ:[A-E]\].*$/gm, ''))) errors.push('Unbound trailing diagram');
  results.push({ question: q, errors, content });
  sections.push(`<article><h2>Question ${q} · ${card.marks} marks · ${errors.length ? errors.join('; ') : 'automated checks pass'}</h2>${html}</article>`);
}
const numbers = cards.map(c => c.question_number).sort((a,b) => a-b);
const complete = numbers.join(',') === Array.from({length:32}, (_,i) => i+1).join(',');
const passed = results.filter(r => !r.errors.length).length;
const out = path.dirname(input);
const css = await fs.readFile('node_modules/katex/dist/katex.min.css', 'utf8');
await fs.cp('node_modules/katex/dist/fonts', path.join(out, 'fonts'), { recursive: true });
await fs.writeFile(path.join(out, 'render-verification.html'), `<!doctype html><meta charset="utf-8"><title>Physics 2024 verification</title><style>${css}\nbody{max-width:1000px;margin:40px auto;font:17px system-ui;line-height:1.6}article{border-bottom:2px solid #aaa;padding:24px}img{max-width:100%;max-height:500px}.options{display:grid;grid-template-columns:1fr 1fr;gap:16px}.options>div{padding:16px;border:1px solid #bbb}table{border-collapse:collapse}td,th{border:1px solid #bbb;padding:8px}</style><h1>Physics 2024: ${passed}/${cards.length} automated checks pass</h1><p>Automated structural and math checks do not certify visual or scientific fidelity.</p>${sections.join('\n')}`);
await fs.writeFile(path.join(out, 'render-verification.json'), JSON.stringify({ complete, passed, total: cards.length, results }, null, 2));
console.log(JSON.stringify({ complete, passed, total: cards.length, failures: results.filter(r => r.errors.length).map(({question, errors}) => ({question, errors})) }, null, 2));
if (!complete || passed !== 32) process.exitCode = 1;
