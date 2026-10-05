/**
 * The paper behind a card: what `show_source_pages` and `relocate_source`
 * answer (src-tauri/src/source_pages.rs), and how the card says it.
 */

export type SourceView =
  | { status: "pages"; path: string; firstPage: number; lastPage: number; images: string[] }
  | { status: "missing"; path: string }
  | { status: "changed"; path: string }
  | { status: "unrecorded" };

/** "page 4", or "pages 6–7". */
export function pagesLabel(first: number, last: number): string {
  return first === last ? `page ${first}` : `pages ${first}–${last}`;
}

/**
 * Why the paper's pages cannot be shown, and whether finding the file again
 * would help; null when they can be.
 */
export function sourceProblem(view: SourceView): { message: string; canLocate: boolean } | null {
  switch (view.status) {
    case "pages":
      return null;
    case "missing":
      return {
        message: `The paper is no longer at ${view.path}: it was moved or deleted. Find it to see this question's pages; every card from the same paper follows.`,
        canLocate: true,
      };
    case "changed":
      return {
        message: `The file at ${view.path} is not the PDF this card was imported from: it has changed since. Find the original, or import the paper again to read it afresh.`,
        canLocate: true,
      };
    case "unrecorded":
      return {
        message: "This card records no paper: it was imported before MergeMark 1.0, or added by hand. Import its paper again to link it.",
        canLocate: false,
      };
  }
}
