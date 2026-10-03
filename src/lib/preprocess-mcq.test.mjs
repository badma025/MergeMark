import assert from 'node:assert/strict';
import test from 'node:test';
import { normalizeMCQOptions, formatMcqOptionBody } from './preprocess-mcq.ts';

test('preserves prose with inline math and normalizes repeated dollars', () => {
  for (const body of [String.raw`The mass of $^{235}_{92}\text{U}$ is greater than the products.`, String.raw`$\alpha$ emission.`, String.raw`1.38 $\times 10^{-7}$ V m$^{-1}$ upwards`, String.raw`the charge stored on a capacitor separated by $1\ \text{m}$`]) {
    assert.equal(formatMcqOptionBody(body), body);
  }
  assert.equal(formatMcqOptionBody('$$$x$$$ is larger'), '$x$ is larger');
  assert.equal(formatMcqOptionBody(String.raw`The quantity has a factor of \mu`), String.raw`The quantity has a factor of \mu`);
});

test('closes option math before the next option and promotes trailing stem images', () => {
  const result = normalizeMCQOptions('Stem\n- [MCQ:A] $x\n- [MCQ:B] $y$\n- [MCQ:C] no\n- [MCQ:D] yes\n\n![Graph](graph.png)');
  assert.ok(result.includes('- [MCQ:A] $x$\n- [MCQ:B] $y$'), result);
  assert.ok(result.indexOf('![Graph]') < result.indexOf('- [MCQ:A]'), result);
});

test('binds four trailing diagrams as distinct visual choices', () => {
  const result = normalizeMCQOptions('Which distribution gives zero field?\n\n' + [...'ABCD'].map(l => `![Diagram](image_${l}.png)`).join('\n\n'));
  for (const l of 'ABCD') assert.ok(result.includes(`- [MCQ:${l}] ![Option ${l}](image_${l}.png)`), result);
});

test('never invents a stacked fraction from flattened option rows', () => {
  // Text order alone ("A 3" then "R") cannot say whether this is R/3, 3R or
  // a value and a unit; the backend rebuilds real fractions from fraction-bar
  // geometry, so the frontend leaves such rows as printed.
  const raw = 'What is the radius?\nA 3\n\nR\nB 4\nR\nC 5\nR\nD 6\nR';
  const result = normalizeMCQOptions(raw);
  assert.ok(!result.includes('\\frac'), result);
  for (const [i, letter] of [...'ABCD'].entries()) assert.ok(result.includes(`[MCQ:${letter}] ${i + 3} R`), result);
  assert.equal(normalizeMCQOptions(result), result);
});

test('the parts printed after an option list stay outside its last option', () => {
  // Edexcel A Biology Jun 2021 Q5: part (iii) closes with options A-D, then
  // the card goes on with parts (iv) and (c).
  const raw = 'Which part of the cell is the location for this process?\n\n**[1 mark]**\nA cytoplasm\nB mitochondrion\nC nucleus\nD rough endoplasmic reticulum\n\n(iv) Describe the role of the tRNA.\n\n**[2 marks]**\n\n(c) Give two differences.';
  const result = normalizeMCQOptions(raw);
  assert.ok(result.includes('- [MCQ:D] rough endoplasmic reticulum\n'), result);
  assert.ok(result.includes('\n(iv) Describe the role of the tRNA.'), result);
  assert.ok(result.includes('\n(c) Give two differences.'), result);
  assert.ok(!/\[MCQ:D\][^\n]*\(iv\)/.test(result), result);
});

test('leaves a number and variable in ordinary prose unchanged', () => {
  const raw = 'A 3\nR is the radius of the orbit.';
  assert.equal(normalizeMCQOptions(raw), raw);
});

test('keeps flattened radical and variable rows as printed', () => {
  const raw = 'Radius?\r\r\nA \\sqrt[3]{2}\r\r\nR\r\r\nB \\sqrt[3]{16}\r\r\nR\r\r\nC 2\r\r\nR\r\r\nD x\r\r\ny';
  const out = normalizeMCQOptions(raw);
  assert.ok(!out.includes('\\frac'), out);
  assert.ok(out.includes('\\sqrt[3]{2}') && out.includes('\\sqrt[3]{16}'), out);
});
