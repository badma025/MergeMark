import assert from 'node:assert/strict';
import test from 'node:test';
import { batchSummary, importablePaths, paperNameOf } from './import-batch.ts';

test('a paper is named by its file, without folder or extension', () => {
  assert.equal(paperNameOf('C:\\exams\\2024_June_P1.pdf'), '2024_June_P1');
  assert.equal(paperNameOf('/Users/a/Maths M2.v2.pdf'), 'Maths M2.v2');
});

test('a batch keeps the files MergeMark can import, in order, each once', () => {
  const { accepted, refused } = importablePaths([
    'C:\\p\\a.pdf', 'C:\\p\\notes.docx', 'C:/p/b.PNG', 'C:\\p\\A.pdf', 'C:\\p\\c.jpeg', 'C:\\p\\d.txt',
  ]);
  assert.deepEqual(accepted, ['C:\\p\\a.pdf', 'C:/p/b.PNG', 'C:\\p\\c.jpeg', 'C:\\p\\d.txt']);
  assert.deepEqual(refused, ['C:\\p\\notes.docx']);
});

test('a finished batch counts its papers and questions and names each failure', () => {
  const all = batchSummary([
    { path: 'C:\\p\\a.pdf', ok: true, questions: 9 },
    { path: 'C:\\p\\b.pdf', ok: true, questions: 1 },
  ], false);
  assert.equal(all.title, '2 papers imported');
  assert.equal(all.description, '10 questions extracted.');
  assert.equal(all.failed, false);

  const some = batchSummary([
    { path: 'C:\\p\\a.pdf', ok: true, questions: 9 },
    { path: 'C:\\p\\scan.pdf', ok: false, error: 'No questions were found in this paper.' },
  ], false);
  assert.equal(some.title, '1 paper imported, 1 failed');
  assert.match(some.description, /^9 questions extracted\. scan: No questions were found/);
  assert.ok(some.failed);

  assert.equal(batchSummary([{ path: 'a.pdf', ok: true, questions: 1 }], true).title, 'Import stopped after 1 paper');
  assert.equal(batchSummary([{ path: 'a.pdf', ok: true, questions: 4 }], false, 2).description, '4 questions extracted. 2 items flagged for review: see Import Logs.');
});
