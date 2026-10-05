import assert from 'node:assert/strict';
import test from 'node:test';
import { pagesLabel, sourceProblem } from './source-pages.ts';

test('a span of pages reads as the paper prints it', () => {
  assert.equal(pagesLabel(4, 4), 'page 4');
  assert.equal(pagesLabel(6, 7), 'pages 6–7');
});

test('a paper that cannot be shown says why, and offers to find it only when it can be found', () => {
  assert.equal(sourceProblem({ status: 'pages', path: 'C:/p.pdf', firstPage: 4, lastPage: 4, images: [] }), null);

  const missing = sourceProblem({ status: 'missing', path: 'C:/papers/s1.pdf' });
  assert.ok(missing.canLocate);
  assert.match(missing.message, /C:\/papers\/s1\.pdf/);
  assert.match(missing.message, /moved or deleted/);

  const changed = sourceProblem({ status: 'changed', path: 'C:/papers/s1.pdf' });
  assert.ok(changed.canLocate);
  assert.match(changed.message, /not the PDF this card was imported from/);

  const unrecorded = sourceProblem({ status: 'unrecorded' });
  assert.ok(!unrecorded.canLocate, 'there is no file to look for');
  assert.match(unrecorded.message, /import/);
});
