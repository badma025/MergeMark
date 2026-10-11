/**
 * Importing several papers at once: which of the chosen files can be
 * imported, and what the batch says when it ends. The files import one after
 * another, since the backend runs one extraction at a time.
 */

const IMPORTABLE = /\.(pdf|txt|png|jpe?g)$/i;

/** A paper's name, as its cards record it: the file's name without its extension. */
export function paperNameOf(filePath: string): string {
  return filePath.replace(/\\/g, "/").split("/").pop()?.replace(/\.[^.]+$/, "") || filePath;
}

/** The files MergeMark can import, in the order given, each once. */
export function importablePaths(paths: string[]): { accepted: string[]; refused: string[] } {
  const accepted: string[] = [];
  const refused: string[] = [];
  const seen = new Set<string>();
  for (const path of paths) {
    const key = path.replace(/\\/g, "/").toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    (IMPORTABLE.test(path) ? accepted : refused).push(path);
  }
  return { accepted, refused };
}

export type PaperResult =
  | { path: string; ok: true; questions: number }
  | { path: string; ok: false; error: string };

/**
 * The toast a finished batch shows: how many papers and questions, how many
 * items the checks flagged (each paper's own report is in Import Logs), and
 * which papers failed and why.
 */
export function batchSummary(results: PaperResult[], cancelled: boolean, flagged = 0): { title: string; description: string; failed: boolean } {
  const done = results.filter((r): r is Extract<PaperResult, { ok: true }> => r.ok);
  const failed = results.filter((r): r is Extract<PaperResult, { ok: false }> => !r.ok);
  const questions = done.reduce((sum, r) => sum + r.questions, 0);
  const papers = (n: number) => (n === 1 ? "1 paper" : `${n} papers`);
  const title = cancelled
    ? `Import stopped after ${papers(done.length)}`
    : failed.length === 0
      ? `${papers(done.length)} imported`
      : `${papers(done.length)} imported, ${failed.length} failed`;
  const parts = [`${questions} ${questions === 1 ? "question" : "questions"} extracted.`];
  if (flagged > 0) parts.push(`${flagged} ${flagged === 1 ? "item" : "items"} flagged for review: see Import Logs.`);
  for (const f of failed) parts.push(`${paperNameOf(f.path)}: ${f.error}`);
  return { title, description: parts.join(" "), failed: failed.length > 0 };
}
