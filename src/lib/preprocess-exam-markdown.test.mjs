import assert from 'node:assert/strict';
import test from 'node:test';
import { healLatexDelimiters, stripOrphanedDollars } from './preprocess-exam-markdown.ts';

test('an inline formula ending a line keeps its span before a display block', () => {
  // Core Pure 2023 Q4: the sentence ends in $n \in \mathbb{N}$ and the matrix
  // identity follows as its own (display) line.
  const card = String.raw`Prove by induction that for $n \in \mathbb{N}$` + '\n' +
    String.raw`$\begin{pmatrix}1 & -2 \\ 0 & 1\end{pmatrix}^{n} = \begin{pmatrix}1 & -2n \\ 0 & 1\end{pmatrix}$`;
  const out = healLatexDelimiters(card);
  assert.ok(!out.includes('$$$'), out);
  assert.ok(out.includes(String.raw`$n \in \mathbb{N}$`), out);
  assert.ok(/\$\$\\begin\{pmatrix\}1 & -2 \\\\ 0 & 1\\end\{pmatrix\}\^\{n\}/.test(out), out);
});

test('a stray single dollar beside a double dollar on one line is still repaired', () => {
  assert.equal(stripOrphanedDollars('$$ x = 1 $$ $'), '$$ x = 1 $$');
  assert.equal(stripOrphanedDollars('$ $$ x = 1 $$'), '$$ x = 1 $$');
});
