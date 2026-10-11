/**
 * Shrink each rendered equation that is wider than its line until it fits,
 * down to `MIN_SCALE` of its size; past that it scrolls sideways as before.
 * KaTeX lays an equation out as one unbreakable box, so a long row such as
 * `E(10,−1,−6)  F(7,−2,7)  G(10,2,9)` otherwise overflows a narrow card.
 */
const MIN_SCALE = 0.6;
const FITTED = "data-fitted";

export function fitWideMath(root: HTMLElement): void {
  const equations = [...root.querySelectorAll<HTMLElement>(".katex")];
  // Undo earlier fits first (all writes), then measure (all reads), then fit
  for (const k of equations) {
    if (k.hasAttribute(FITTED)) {
      k.style.removeProperty("font-size");
      k.removeAttribute(FITTED);
    }
  }
  const fits: [HTMLElement, number][] = [];
  for (const k of equations) {
    const box = k.querySelector<HTMLElement>(".katex-html");
    const line = k.closest<HTMLElement>(".katex-display") ?? k.parentElement?.closest<HTMLElement>("p, li, div");
    if (!box || !line || line.closest("table")) continue;
    // A display equation's box is a block as wide as its line: its content's width is the scroll width
    const width = Math.max(box.scrollWidth, box.getBoundingClientRect().width);
    const available = line.clientWidth;
    if (width <= available || available <= 0) continue;
    const size = parseFloat(getComputedStyle(k).fontSize);
    fits.push([k, size * Math.max(MIN_SCALE, (available - 1) / width)]);
  }
  for (const [k, px] of fits) {
    // The display rule sets its size with !important
    k.style.setProperty("font-size", `${px}px`, "important");
    k.setAttribute(FITTED, "");
  }
}
