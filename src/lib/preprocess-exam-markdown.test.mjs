import assert from 'node:assert/strict';
import test from 'node:test';
import { healLatexDelimiters, stripOrphanedDollars } from './preprocess-exam-markdown.ts';

test('a minus touching an opening dollar moves inside the maths', () => {
  const out = healLatexDelimiters(String.raw`A −$7.1\times10^{7}$ C`);
  assert.ok(out.includes(String.raw`A $-7.1`) && !out.includes('−-'), out);
  assert.ok(healLatexDelimiters(String.raw`(−$x$)`).includes(String.raw`($-x$)`));
  // A spaced dash is prose punctuation: it is left as printed.
  const prose = healLatexDelimiters(String.raw`the force – $F$ – acts`);
  assert.ok(prose.includes(String.raw`force – $F$`), prose);
  // The fixture physics '18 Q17 card (before the layout put the minus in the
  // maths): the option line must stay an option line.
  const option = String.raw`- [MCQ:A] – $6.76 \times 10^{14}$ J`;
  assert.ok(healLatexDelimiters(option).includes(option), healLatexDelimiters(option));
  // A dash before a CLOSING dollar is maths inside its span.
  assert.ok(healLatexDelimiters(String.raw`$a -$ b $c$`).includes(String.raw`$a -$ b $c$`));
});

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
