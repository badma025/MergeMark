/**
 * KaTeX Delimiter Healing & LaTeX Syntax Sanitization
 *
 * Runs on raw markdown string before it enters ReactMarkdown:
 * 1. Normalizes Markdown tables (injects missing GFM delimiters & blank lines)
 * 2. Strips orphaned trailing $ symbols (e.g. "[2 marks]$", "answer: $x$.$", "$$ $")
 * 3. Balances unclosed LaTeX math environments (\begin{pmatrix} -> \end{pmatrix})
 * 4. Balances inline ($) and block ($$) math delimiters — broken inline math is
 *    closed at the END OF ITS OWN LINE so a stray $ can never swallow the next
 *    paragraph or an MCQ options grid
 * 5. Fixes spaced LaTeX commands ("\ frac" -> "\frac")
 * 6. Strips backend placeholder tokens ([DIAGRAM_PLACEHOLDER],
 *    [VISUAL_MCQ_PLACEHOLDER]) and phantom/empty display math blocks
 */

// LaTeX environments that require balancing and wrapping
export const MATH_ENVS = [
  'pmatrix', 'bmatrix', 'vmatrix', 'Vmatrix', 'matrix',
  'array', 'cases', 'aligned', 'gathered', 'align', 'align*',
  'alignat', 'alignat*', 'flalign', 'flalign*',
  'eqnarray', 'eqnarray*', 'multline', 'multline*',
  'split', 'subequations'
];

/**
 * Backend placeholder tokens that must never reach the DOM as literal text.
 */
export const PLACEHOLDER_TOKENS = ['[DIAGRAM_PLACEHOLDER]', '[VISUAL_MCQ_PLACEHOLDER]'] as const;

/**
 * Strip backend placeholder tokens so they can never reach the DOM as literal text.
 */
export function stripPlaceholderTokens(text: string): string {
  let s = text;
  for (const token of PLACEHOLDER_TOKENS) {
    s = s.split(token).join('');
  }
  return s;
}

/**
 * Remove phantom/empty display math blocks ($$ with no content between the
 * delimiters, including across blank lines) before the markdown parser sees
 * them — an empty $$ pair renders as a stray KaTeX artifact / blank gap.
 */
export function stripEmptyDisplayMath(text: string): string {
  let s = text.replace(/\$\$[\s]*\$\$/g, '');
  // Collapse blank runs the removal leaves behind.
  return s.replace(/\n{3,}/g, '\n\n');
}

/**
 * Common LaTeX command names frequently broken by OCR / LLMs with a space after the backslash
 */
const LATEX_COMMANDS = [
  'begin', 'end', 'frac', 'dfrac', 'cfrac', 'sqrt', 'text', 'textbf', 'textit',
  'mathbf', 'mathit', 'mathrm', 'mathbb', 'mathcal', 'operatorname',
  'pmatrix', 'bmatrix', 'vmatrix', 'matrix', 'array', 'cases', 'aligned',
  'hline', 'hlinex', 'cline', 'multicolumn', 'multirow',
  'theta', 'lambda', 'alpha', 'beta', 'gamma', 'delta', 'Delta', 'pi', 'mu',
  'sigma', 'omega', 'Omega', 'phi', 'Phi', 'psi', 'Psi',
  'times', 'div', 'pm', 'mp', 'leq', 'geq', 'neq', 'approx', 'sim', 'equiv',
  'subset', 'supset', 'subseteq', 'supseteq', 'in', 'notin', 'forall', 'exists',
  'infty', 'partial', 'nabla', 'cos', 'sin', 'tan', 'sec', 'csc', 'cot',
  'cosh', 'sinh', 'tanh', 'ln', 'log', 'exp', 'int', 'iint', 'iiint', 'oint',
  'sum', 'prod', 'lim', 'vec', 'hat', 'bar', 'dot', 'ddot', 'tilde', 'binom',
  'quad', 'qquad', 'overrightarrow', 'overleftarrow', 'overbrace', 'underbrace',
  'widehat', 'widetilde', 'overline', 'underline', 'left', 'right'
];

/**
 * Fix spaced commands: "\ frac{1}{2}" -> "\frac{1}{2}"
 */
export function fixSpacedCommands(text: string): string {
  const pattern = new RegExp(`\\\\ +(${LATEX_COMMANDS.join('|')})\\b`, 'g');
  return text.replace(pattern, '\\$1');
}

/**
 * Restore LaTeX commands mangled by unescaped `\t` escapes in stored JSON
 * payloads: "\text{Ra}" decodes to a literal TAB character followed by
 * "ext{Ra}" (same class as \theta → TAB + "heta"). Tabs never occur
 * legitimately in exam content, so a TAB directly preceding the remainder of
 * a known LaTeX command is deterministically restored to backslash-t.
 */
const TAB_MANGLED_LATEX_RE = /\t(?=(?:extbf|extit|extrm|exttt|ext|imes|heta|herefore|au|ilde|an|o|op|riangle|woheadrightarrow|frac|dfrac)(?![A-Za-z]))/g;

export function fixTabMangledLatex(text: string): string {
  if (!text.includes('\t')) return text;
  return text.replace(TAB_MANGLED_LATEX_RE, '\\t');
}

/**
 * Normalizes Markdown tables produced by OCR / LLMs:
 * 1. Reassembles multi-line cells (split across lines) into valid single-line GFM table rows.
 * 2. Bridges blank lines between header and delimiter rows or within table bodies.
 * 3. Deduplicates consecutive delimiter rows (|:---|:---| followed by | --- | --- |).
 * 4. Ensures delimiter column count matches header column count.
 * 5. Guarantees clean blank lines before and after table blocks.
 */
export function normalizeMarkdownTables(text: string): string {
  if (!text || !text.includes('|')) return text;

  // Un-wrap any table rows or delimiter lines accidentally wrapped in math delimiters ($| ... |$)
  let cleaned = text
    .replace(/^[ \t]*\$(\|(?:\s*:?-+:?\s*\|)+)\$?[ \t]*$/gm, '$1')
    .replace(/^[ \t]*\$?(\|(?:\s*:?-+:?\s*\|)+)\$[ \t]*$/gm, '$1')
    .replace(/^[ \t]*\$(\|.*\|)\$[ \t]*$/gm, '$1');

  const rawLines = cleaned.split('\n');
  const lines: string[] = [];

  // Pass 1: Fuse multiline table rows.
  // e.g. Line 1: "| | mass of water"
  //      Line 2: "mass of air |"
  // -> Fuse to "| | mass of water <br> mass of air |"
  for (let i = 0; i < rawLines.length; i++) {
    let line = rawLines[i];
    const trimmed = line.trim();

    if (trimmed.startsWith('|') && !trimmed.endsWith('|') && trimmed.length > 1) {
      // Look ahead for continuation lines that finish this row
      let fused = trimmed;
      let j = i + 1;
      while (j < rawLines.length) {
        const nextTrimmed = rawLines[j].trim();
        if (nextTrimmed === '') break;
        if (nextTrimmed.startsWith('|') && nextTrimmed.endsWith('|')) {
          // New row begins, break
          break;
        }
        fused += (nextTrimmed.endsWith('|') ? ' <br> ' : ' ') + nextTrimmed;
        if (nextTrimmed.endsWith('|')) {
          j++;
          break;
        }
        j++;
      }
      lines.push(fused);
      i = j - 1;
    } else {
      lines.push(line);
    }
  }

  const result: string[] = [];
  let inTable = false;
  let tableLines: string[] = [];

  const isTableLine = (l: string) => {
    const trimmed = l.trim();
    return trimmed.startsWith('|') && trimmed.endsWith('|') && trimmed.length > 1;
  };

  const isDelimiterRow = (l: string) => /^\|(?:\s*:?-+:?\s*\|)+$/.test(l.trim());

  const getColCount = (row: string) => {
    const cells = row.split('|').map(c => c.trim()).filter((_, idx, arr) => idx > 0 && idx < arr.length - 1);
    return Math.max(cells.length, 1);
  };

  const flushTable = (tbl: string[]) => {
    if (tbl.length === 0) return;

    // Filter out duplicate or redundant delimiter rows
    const cleanedRows: string[] = [];
    for (let r = 0; r < tbl.length; r++) {
      const row = tbl[r].trim();
      if (isDelimiterRow(row)) {
        // If previous row in cleanedRows was already a delimiter, skip this one
        if (cleanedRows.length > 0 && isDelimiterRow(cleanedRows[cleanedRows.length - 1])) {
          continue;
        }
      }
      cleanedRows.push(row);
    }

    if (cleanedRows.length === 0) return;

    let repaired: string[] = [];

    // Case A: First row is already a delimiter row (missing header row)
    if (isDelimiterRow(cleanedRows[0])) {
      const colCount = getColCount(cleanedRows[0]);
      const headerRow = '|' + '   |'.repeat(colCount);
      repaired = [headerRow, cleanedRows[0], ...cleanedRows.slice(1)];
    }
    // Case B: Second row is NOT a delimiter row (or only 1 row total)
    else if (cleanedRows.length === 1 || !isDelimiterRow(cleanedRows[1])) {
      const colCount = getColCount(cleanedRows[0]);
      const delimiterRow = '|' + ' --- |'.repeat(colCount);
      repaired = [cleanedRows[0], delimiterRow, ...cleanedRows.slice(1)];
    } else {
      // Delimiter row exists at index 1 — ensure its column count matches header
      const headerCols = getColCount(cleanedRows[0]);
      const delimCols = getColCount(cleanedRows[1]);
      if (headerCols !== delimCols) {
        cleanedRows[1] = '|' + ' --- |'.repeat(headerCols);
      }
      repaired = cleanedRows;
    }

    // Ensure blank line before table if needed
    if (result.length > 0 && result[result.length - 1].trim() !== '') {
      result.push('');
    }
    result.push(...repaired);
    // Ensure blank line after table
    result.push('');
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();

    if (isTableLine(trimmed)) {
      if (!inTable) {
        inTable = true;
        tableLines = [trimmed];
      } else {
        tableLines.push(trimmed);
      }
    } else if (trimmed === '' && inTable) {
      // Lookahead: is next non-blank line also a table line?
      let nextIsTable = false;
      for (let j = i + 1; j < lines.length; j++) {
        const look = lines[j].trim();
        if (look === '') continue;
        if (isTableLine(look)) nextIsTable = true;
        break;
      }
      if (!nextIsTable) {
        flushTable(tableLines);
        inTable = false;
        tableLines = [];
        result.push(line);
      }
      // If next is table, we bridge across the blank line without flushing
    } else {
      if (inTable) {
        flushTable(tableLines);
        inTable = false;
        tableLines = [];
      }
      result.push(line);
    }
  }

  if (inTable) {
    flushTable(tableLines);
  }

  return result.join('\n');
}

/**
 * Ensures display math blocks ($$...$$) have blank lines before and after
 * so CommonMark/ReactMarkdown never squashes display equations into adjacent paragraphs.
 */
export function isolateDisplayMathBlocks(text: string): string {
  if (!text || !text.includes('$$')) return text;

  // 1. Isolate single-line $$...$$ from surrounding text:
  // e.g. "Text\n$$eq$$" -> "Text\n\n$$eq$$"
  // e.g. "$$eq$$\nText" -> "$$eq$$\n\nText"
  let s = text.replace(/([^\n])\n([ \t]*\$\$[^\n]+\$\$)/g, '$1\n\n$2');
  s = s.replace(/(\$\$[^\n]+\$\$)[ \t]*\n([^\n])/g, '$1\n\n$2');

  // 2. Isolate multi-line $$ ... $$ blocks from surrounding text:
  s = s.replace(/([^\n])\n([ \t]*\$\$\s*$)/gm, '$1\n\n$2');
  s = s.replace(/(^\s*\$\$)[ \t]*\n([^\n])/gm, '$1\n\n$2');

  return s;
}

/**
 * Strip orphaned dollar signs that break KaTeX delimiter pairing:
 * E.g., "[2 marks]$" -> "[2 marks]", "answer: $x$.$" -> "answer: $x$."
 */
export function stripOrphanedDollars(text: string): string {
  let s = text;

  // 1. Triple or more dollars -> $$
  s = s.replace(/\${3,}/g, '$$$$');

  // 2. Strip trailing $ immediately following mark allocations: [4 marks]$ or (3 marks)$
  s = s.replace(/((?:\[|\()\s*(?:Total:?\s*)?\d+\s*marks?\s*(?:\]|\)))\$/gi, '$1');

  // 3. Strip trailing $ right after punctuation at end of line: "value of $x$.$" -> "value of $x$."
  s = s.replace(/([.?!,;:])\$(?=\s|$)/g, '$1');

  // 4. Strip stray double-dollar with an extra single dollar: "$$ $" or "$ $$"
  //    on ONE line. Across a line break these are two spans (an inline
  //    formula closing a sentence, then a display block) and must survive.
  s = s.replace(/\$\$[ \t]*\$(?!\$)/g, '$$$$');
  s = s.replace(/(?<!\$)\$[ \t]*\$\$/g, '$$$$');

  // 5. Strip solitary dollar signs on their own line
  s = s.replace(/^[ \t]*\$[ \t]*$/gm, '');

  return s;
}

/**
 * Auto-close unclosed LaTeX environments (\begin{pmatrix} -> \end{pmatrix})
 */
export function balanceMathEnvironments(text: string): string {
  let s = text;
  for (const env of MATH_ENVS) {
    const beginMatches = (s.match(new RegExp(`\\\\begin\\{${env}\\}`, 'g')) || []).length;
    const endMatches = (s.match(new RegExp(`\\\\end\\{${env}\\}`, 'g')) || []).length;
    const missing = beginMatches - endMatches;
    if (missing > 0) {
      s = s + '\n' + `\\end{${env}}\n`.repeat(missing);
    }
  }
  return s;
}

/**
 * Heal mismatched single vs double dollar blocks ($...$$ or $$...$)
 * and lines starting with raw LaTeX commands ending with $ (missing opening delimiter).
 */
export function healMismatchedAndMissingDelimiters(text: string): string {
  let s = text;

  // 1. Fix mismatched $ ... $$ (single start, double end) -> $$ ... $$ on a single line
  s = s.replace(/(^|[\n \t])\$(?!\$)([^\n\$]+?)\$\$(?=[ \t]|$)/g, '$1$$$$$2$$$$');

  // 2. Fix mismatched $$ ... $ (double start, single end) -> $$ ... $$ on a single line
  s = s.replace(/(^|[\n \t])\$\$([^\n\$]+?)\$(?!\$)(?=[ \t]|$)/g, '$1$$$$$2$$$$');

  // 3. Fix lines starting with bare LaTeX command and ending with a single $ (e.g. "\frac{...}$" -> "$$\frac{...}$$")
  s = s.replace(
    /^[ \t]*(\\(?:frac|dfrac|cfrac|sin|cos|tan|sec|csc|cot|sinh|cosh|tanh|sqrt|sum|int|iint|iiint|oint|lim|begin|mathbf|mathit|mathrm|mathbb|mathcal|operatorname|theta|alpha|beta|gamma|delta|Delta|pi|mu|sigma|omega|Omega|phi|Phi|psi|Psi|left)\b[^\n\$]+)\$[ \t]*$/gm,
    '$$$1$$'
  );

  return s;
}

/**
 * Heals bare matrix environments and matrix blocks concatenated with display math:
 * e.g. "\begin{pmatrix} 7 & 6 \\ 6 & 2 \end{pmatrix}$$\lambda = -2...$$"
 * -> "$$\begin{pmatrix} 7 & 6 \\ 6 & 2 \end{pmatrix}$$\n\n$$\lambda = -2...$$"
 * e.g. "$\begin{pmatrix} 1 & 2 \\ 2 & -4 \end{pmatrix} \quad \text{and} \quad B = \begin{pmatrix}...$"
 * -> "$$\begin{pmatrix} 1 & 2 \\ 2 & -4 \end{pmatrix} \quad \text{and} \quad B = \begin{pmatrix}...$$"
 */
export function healMatrixEnvironments(text: string): string {
  if (!text || !text.includes('\\begin{')) return text;

  let s = text;

  // 1. Bare \begin{matrix} ... \end{matrix}$$equation$$ -> $$\begin{matrix} ... \end{matrix}$$\n\n$$equation$$
  s = s.replace(
    /(?:^|\n)([ \t]*\\begin\{(?:pmatrix|bmatrix|vmatrix|Vmatrix|matrix|cases|aligned)\}[\s\S]*?\\end\{(?:pmatrix|bmatrix|vmatrix|Vmatrix|matrix|cases|aligned)\})\$\$([\s\S]*?\$\$)(?=\n|$)/g,
    (_match, matrixBlock, eqBlock) => {
      return `\n\n$$${matrixBlock.trim()}$$\n\n$$${eqBlock.trim()}\n\n`;
    }
  );

  // 2. Single $ containing matrix environments -> elevate to $$ ... $$
  s = s.replace(
    /(?:^|[\n \t])\$(?!\$)([^\n\$]*?\\begin\{(?:pmatrix|bmatrix|vmatrix|Vmatrix|matrix|cases|aligned)\}[\s\S]*?\\end\{(?:pmatrix|bmatrix|vmatrix|Vmatrix|matrix|cases|aligned)\}[^\n\$]*?)\$(?!\$)/g,
    (_match, content) => {
      return `\n\n$$${content.trim()}$$\n\n`;
    }
  );

  // 3. Isolated bare \begin{matrix} ... \end{matrix} without any $ on its own lines
  s = s.replace(
    /(?:^|\n)([ \t]*\\begin\{(?:pmatrix|bmatrix|vmatrix|Vmatrix|matrix|cases|dcases)\}[\s\S]*?\\end\{(?:pmatrix|bmatrix|vmatrix|Vmatrix|matrix|cases|dcases)\}[ \t]*)(?=\n|$)/g,
    (match, block) => {
      const trimmed = block.trim();
      if (!trimmed.startsWith('$') && !trimmed.endsWith('$')) {
        return `\n\n$$${trimmed}$$\n\n`;
      }
      return match;
    }
  );

  return s;
}

/**
 * Close broken inline math at the END OF ITS OWN LINE.
 *
 * The old repair appended the closing `$` at the end of the whole document
 * segment, which silently swallowed every paragraph (and any MCQ options
 * grid) between the stray `$` and the next delimiter into one math node.
 * Closing per line guarantees a broken boundary can only ever eat its own
 * sentence, never the content below it.
 */
function closeBrokenInlineMathPerLine(part: string): string {
  const lines = part.split('\n');
  let inMath = false;
  return lines
    .map((line) => {
      let s = '';
      for (let i = 0; i < line.length; i++) {
        const ch = line[i];
        if (ch === '\\') {
          // Consume the escaped char verbatim so \$ stays literal.
          s += ch + (line[i + 1] ?? '');
          i++;
          continue;
        }
        if (ch === '$') {
          inMath = !inMath;
        }
        s += ch;
      }
      if (inMath) {
        s += '$';
        inMath = false;
      }
      return s;
    })
    .join('\n');
}

/**
 * Strict final delimiter validator and repair pass
 */
export function validateAndEnforceDelimiters(text: string): string {
  if (!text) return '';

  let s = text;

  // 1. Triple or more dollars -> $$
  s = s.replace(/\${3,}/g, '$$$$');

  // 2. Mismatched single vs double
  s = healMismatchedAndMissingDelimiters(s);

  // 3. Count unescaped block math delimiters ($$)
  const doubleMatches = s.match(/(?<!\\)\$\$/g) || [];
  if (doubleMatches.length % 2 !== 0) {
    s += '\n$$';
  }

  // 4. Split by display math $$ blocks
  const parts = s.split('$$');
  const healedParts = parts.map((part, index) => {
    // Even indices are OUTSIDE display math ($$)
    if (index % 2 === 0) {
      // Close any broken inline $ boundary at its OWN line end so it can
      // never swallow subsequent text or MCQ syntax.
      return closeBrokenInlineMathPerLine(part);
    } else {
      // Odd indices are INSIDE display math ($$)
      // Strip nested single $ inside display math to prevent KaTeX parse errors
      return part.replace(/(?<!\\)\$/g, '');
    }
  });

  return healedParts.join('$$');
}

/**
 * Heals dropped variable prefixes (e.g. 'r = ', 'y = ') and missing opening '$' delimiters
 * in polar equations, cardioids, Cartesian lines, and curves without ever consuming preambles.
 */
export function healPolarAndDroppedEquations(text: string): string {
  if (!text) return '';
  const lines = text.split('\n');
  const result: string[] = [];

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();

    const prevLine = i > 0 ? lines[i - 1].trim() : '';
    const prev2Line = i > 1 ? lines[i - 2].trim() : '';
    const hasPolarPreamble =
      /polar\s+equations?|cardioid|spiral\s+curve|curve(?:\s+\$?[A-Za-z0-9_]+\$?)?\s+with\s+polar/i.test(prevLine) ||
      /polar\s+equations?|cardioid|spiral\s+curve|curve(?:\s+\$?[A-Za-z0-9_]+\$?)?\s+with\s+polar/i.test(prev2Line);

    const hasEquationPreamble =
      hasPolarPreamble ||
      /(?:straight\s+)?line(?:\s+\$?[A-Za-z0-9_]+\$?)?\s+with(?:\s+(?:polar|Cartesian)?\s+equation)?|curve(?:\s+\$?[A-Za-z0-9_]+\$?)?\s+with(?:\s+(?:polar|Cartesian)?\s+equation)?|(?:Cartesian\s+)?equation\s+of\s+the\s+(?:line|curve)|with\s+(?:Cartesian\s+)?equation/i.test(prevLine) ||
      /(?:straight\s+)?line(?:\s+\$?[A-Za-z0-9_]+\$?)?\s+with(?:\s+(?:polar|Cartesian)?\s+equation)?|curve(?:\s+\$?[A-Za-z0-9_]+\$?)?\s+with(?:\s+(?:polar|Cartesian)?\s+equation)?|(?:Cartesian\s+)?equation\s+of\s+the\s+(?:line|curve)|with\s+(?:Cartesian\s+)?equation/i.test(prev2Line);

    if (!trimmed.startsWith('$') && !trimmed.startsWith('$$') && trimmed.length > 0) {
      // Case 1: Two dollar blocks on this line: `expr$, $domain$` or `expr$, \quad $domain$`
      const twoBlockMatch = trimmed.match(
        /^([a-zA-Z0-9\\+\-*/()_^{} \t]*?\\(?:cos|sin|tan|sec|csc|cot|theta|frac|sqrt|pi|lambda|alpha|beta)\b[a-zA-Z0-9\\+\-*/()_^{} \t]*?)\$,\s*(?:\\quad\s*)?\$([^\n\$]+?)\$([.,]?)$/
      );
      if (twoBlockMatch) {
        const expr = twoBlockMatch[1].trim();
        const domain = twoBlockMatch[2].trim();
        const punct = twoBlockMatch[3] || '';
        const hasTheta = expr.includes('\\theta') || domain.includes('\\theta') || hasPolarPreamble;
        const prefix = hasTheta && !/^[a-zA-Z]\s*=/.test(expr) ? 'r = ' : '';
        result.push(`$$${prefix}${expr}, \\quad ${domain}$$${punct}`);
        continue;
      }

      // Case 2: Single un-opened block ending with a single `$`:
      // e.g. "a\theta, \quad 0 \leq \theta \leq 2\pi$,"
      // e.g. "4(\cos\theta + \sin\theta) \quad 0 \le \theta < 2\pi$."
      // e.g. "x + k$"
      // e.g. "2x^2 + 3x - 1$."
      const singleBlockMatch = trimmed.match(
        /^([a-zA-Z0-9\\+\-*/()_^{} \t]+?)\$([.,]?)$/
      );
      if (singleBlockMatch && hasEquationPreamble) {
        let expr = singleBlockMatch[1].trim();
        let punct = singleBlockMatch[2] || '';
        const trailingPunctMatch = expr.match(/([.,])$/);
        if (trailingPunctMatch) {
          punct = trailingPunctMatch[1] + punct;
          expr = expr.slice(0, -1).trim();
        }
        const hasTheta = expr.includes('\\theta') || hasPolarPreamble;
        const isCartesianLineOrCurve = !hasTheta && /[xkt]/i.test(expr);
        let prefix = '';
        if (!/^[a-zA-Z]\s*=/.test(expr)) {
          if (hasTheta) {
            prefix = 'r = ';
          } else if (isCartesianLineOrCurve) {
            prefix = 'y = ';
          }
        }
        result.push(`$$${prefix}${expr}$$${punct}`);
        continue;
      }

      // Case 3: Completely unbracketed formula line directly following polar or curve preamble
      if (hasEquationPreamble && /\\(?:cos|sin|tan|sec|csc|cot|theta|frac|sqrt|pi)\b/.test(trimmed) && !trimmed.includes('$')) {
        let expr = trimmed;
        const prefix = !/^[a-zA-Z]\s*=/.test(expr) ? (hasPolarPreamble ? 'r = ' : 'y = ') : '';
        result.push(`$$${prefix}${expr}$$`);
        continue;
      }
    }

    // Case 4: Multiple curves on one line e.g. "... $,4\sin 2\theta..." or "... $,1.5$, $0 \le \theta..."
    let fixedLine = line.replace(
      /\$,\s*(?!\$?\s*r\s*=|\$\$)([0-9a-zA-Z\\+\-*/()_^{} \t]+?\\(?:cos|sin|tan|sec|csc|cot|theta|frac|sqrt|pi)\b[0-9a-zA-Z\\+\-*/()_^{} \t]*?)\$,\s*(?:\\quad\s*)?\$([^\n\$]+?)\$/g,
      '$$ and $$r = $1, \\quad $2$$'
    );
    fixedLine = fixedLine.replace(
      /\$,\s*([0-9.]+)\$,\s*(?:\\quad\s*)?\$([^\n\$]+?)\$/g,
      '$$ and $$r = $1, \\quad $2$$'
    );

    result.push(fixedLine);
  }

  return result.join('\n');
}

/**
 * Deduplicate accidental verbatim repeated question paragraphs
 */
export function deduplicateRepeatedParagraphs(text: string): string {
  if (!text) return '';
  const paragraphs = text.split(/\n{2,}/);
  if (paragraphs.length <= 1) return text;

  const half = Math.floor(paragraphs.length / 2);
  if (paragraphs.length >= 2 && paragraphs.length % 2 === 0) {
    const firstHalf = paragraphs.slice(0, half).join('\n\n').trim();
    const secondHalf = paragraphs.slice(half).join('\n\n').trim();
    if (firstHalf === secondHalf && firstHalf.length > 40) {
      return firstHalf;
    }
  }

  const deduped: string[] = [];
  for (let i = 0; i < paragraphs.length; i++) {
    const p = paragraphs[i].trim();
    if (p.length > 30 && deduped.length > 0 && deduped[deduped.length - 1].trim() === p) {
      continue;
    }
    deduped.push(paragraphs[i]);
  }
  return deduped.join('\n\n');
}

/**
 * Convert interior newlines inside multi-line $$ ... $$ blocks into explicit
 * LaTeX row separators (\\).
 *
 * KaTeX treats a raw newline inside display math as ordinary whitespace, so
 * sequential equations transcribed on separate lines (nuclear decay chains,
 * simultaneous pairs) render squashed end-to-end (e.g.
 * "...Rn + α^{222}...Po"). Each interior newline becomes \\ unless either
 * side already carries a separator, or the break sits on a bare environment
 * boundary where a separator would create an empty matrix row.
 */
const BARE_ENV_OPEN_RE = /^\\begin\{[A-Za-z*]+\}(?:\[[^\]]*\])?(?:\{[^}]*\})*$/;

function endsWithRowSeparator(trimmed: string): boolean {
  return /(?:\\\\|\\cr)\s*$/.test(trimmed);
}

/**
 * Shared code map for the delimiter machinery (the Rust `validate::line_code_map`
 * mirror). Tracks fenced code blocks by MARKER CHAR and RUN LENGTH so a
 * four-backtick fence is not closed by a three-backtick line and a tilde fence
 * is not closed by backticks. Inline code spans follow CommonMark backtick-RUN
 * rules (a span opens with a run of N backticks and closes at the next run of
 * EXACTLY N backticks), so ``a ` b`` and ``$`` stay opaque.
 */
interface LineCode {
  /** The whole line sits inside a fenced code block. */
  fenced: boolean;
  /** [start, end) char ranges of inline code spans (empty inside a fence). */
  spans: Array<[number, number]>;
}

function fenceMarker(line: string): { ch: string; run: number } | null {
  const trimmed = line.replace(/^ {0,3}/, '');
  if (trimmed.length < 3) return null;
  const ch = trimmed[0];
  if (ch !== '`' && ch !== '~') return null;
  let run = 0;
  while (run < trimmed.length && trimmed[run] === ch) run++;
  if (run < 3) return null;
  // A backtick fence may not carry another backtick in its info string.
  if (ch === '`' && trimmed.slice(run).includes('`')) return null;
  return { ch, run };
}

/** True when `line` CLOSES a fence opened with `ch`/`run`: same marker char, a
 * run at least as long, and nothing but whitespace after the run. */
function fenceCloses(line: string, ch: string, run: number): boolean {
  const marker = fenceMarker(line);
  if (!marker || marker.ch !== ch || marker.run < run) return false;
  const trimmed = line.replace(/^ {0,3}/, '');
  return trimmed.slice(marker.run).trim() === '';
}

function inlineCodeSpans(line: string): Array<[number, number]> {
  const spans: Array<[number, number]> = [];
  let i = 0;
  while (i < line.length) {
    if (line[i] !== '`') {
      i++;
      continue;
    }
    const start = i;
    let run = 0;
    while (i < line.length && line[i] === '`') {
      run++;
      i++;
    }
    let j = i;
    let closeEnd = -1;
    while (j < line.length) {
      if (line[j] === '`') {
        let m = 0;
        while (j < line.length && line[j] === '`') {
          m++;
          j++;
        }
        if (m === run) {
          closeEnd = j;
          break;
        }
      } else {
        j++;
      }
    }
    const end = closeEnd === -1 ? line.length : closeEnd;
    spans.push([start, end]);
    i = end;
  }
  return spans;
}

export function lineCodeMap(content: string): LineCode[] {
  const map: LineCode[] = [];
  let open: { ch: string; run: number } | null = null;
  for (const line of content.split('\n')) {
    if (open) {
      const closes = fenceCloses(line, open.ch, open.run);
      map.push({ fenced: true, spans: [] });
      if (closes) open = null;
      continue;
    }
    const marker = fenceMarker(line);
    if (marker) {
      open = marker;
      map.push({ fenced: true, spans: [] });
      continue;
    }
    map.push({ fenced: false, spans: inlineCodeSpans(line) });
  }
  return map;
}

/**
 * Mask fenced code blocks and inline code spans with opaque sentinels so the
 * (otherwise code-blind) delimiter passes copy them verbatim. Newline structure
 * is preserved (one sentinel per code line), so line-anchored repairs still see
 * the same number of lines. `restore` puts the exact original text back.
 *
 * The sentinel uses U+0001 control chars and digits only — it contains none of
 * the characters the passes match (`$`, backtick, `|`, `\begin{`, punctuation).
 */
export function protectCode(content: string): { masked: string; restore: (s: string) => string } {
  const codeMap = lineCodeMap(content);
  const store: string[] = [];
  // Collision-safe sentinel: pick a control marker the source does not already
  // contain, so an existing U+0001+digits sequence can never restore as another
  // code span (or as `undefined`).
  let marker = '';
  for (let len = 1; len <= 8 && !marker; len++) {
    for (const c of ['\u0001', '\u0002', '\u0003', '\u0000']) {
      const candidate = c.repeat(len);
      if (!content.includes(candidate)) {
        marker = candidate;
        break;
      }
    }
  }
  if (!marker) {
    throw new Error('protectCode: content already contains every sentinel marker');
  }
  const token = (text: string) => {
    store.push(text);
    return `${marker}${store.length - 1}${marker}`;
  };
  const masked = content
    .split('\n')
    .map((line, i) => {
      const code = codeMap[i] ?? { fenced: false, spans: [] as Array<[number, number]> };
      if (code.fenced) return token(line);
      if (code.spans.length === 0) return line;
      let out = '';
      let pos = 0;
      for (const [start, end] of code.spans) {
        out += line.slice(pos, start) + token(line.slice(start, end));
        pos = end;
      }
      out += line.slice(pos);
      return out;
    })
    .join('\n');
  const restore = (s: string) =>
    s.replace(new RegExp(`${marker}(\\d+)${marker}`, 'g'), (_match, n: string) => store[Number(n)]);
  return { masked, restore };
}

/** Display `$$` toggles in one line, counting each run of two-or-more dollars
 * as ONE toggle and ignoring inline code spans. */
function countDisplayToggles(line: string, spans: Array<[number, number]>): number {
  let toggles = 0;
  let pos = 0;
  const scan = (segment: string) => {
    let i = 0;
    while (i < segment.length) {
      if (segment[i] === '\\') {
        i += 2;
        continue;
      }
      if (segment[i] === '$') {
        let run = 0;
        while (i + run < segment.length && segment[i + run] === '$') run++;
        if (run >= 2) toggles++;
        i += run;
        continue;
      }
      i++;
    }
  };
  for (const [start, end] of spans) {
    if (start > pos) scan(line.slice(pos, start));
    pos = Math.max(end, pos);
  }
  if (pos < line.length) scan(line.slice(pos));
  return toggles;
}

export function ensureDisplayMathLineBreaks(text: string): string {
  if (!text.includes('$$')) return text;
  const lines = text.split('\n');
  const codeMap = lineCodeMap(text);
  const out: string[] = [];
  let inDisplay = false;

  for (let idx = 0; idx < lines.length; idx++) {
    const line = lines[idx];
    const code = codeMap[idx];
    // Fenced blocks and lines that are entirely inline code are opaque: they
    // neither open, close nor continue display math.
    const entirelyInlineCode = code.spans.some(([start, end]) => start === 0 && end >= line.length);
    if (code.fenced || entirelyInlineCode) {
      out.push(line);
      continue;
    }
    const trimmed = line.trim();
    const toggles = countDisplayToggles(line, code.spans) % 2 === 1;

    if (!inDisplay) {
      out.push(line);
      if (toggles) inDisplay = true;
      continue;
    }

    // Inside a display block: this line's interior content is everything
    // before a trailing $$ that closes the block.
    const closesHere = toggles && trimmed.endsWith('$$');
    const curInner = closesHere ? trimmed.slice(0, -2).trim() : trimmed;

    // Previous line's interior content (the block may have opened with
    // content on the same line, e.g. "$$x = 1").
    const prevTrim = (out[out.length - 1] ?? '').trim();
    const prevInner = prevTrim.startsWith('$$') ? prevTrim.slice(2).trim() : prevTrim;

    const needsSeparator =
      prevInner.length > 0 &&
      curInner.length > 0 &&
      !endsWithRowSeparator(prevInner) &&
      !curInner.startsWith('\\\\') &&
      !BARE_ENV_OPEN_RE.test(prevInner) &&
      !curInner.startsWith('\\end{');

    if (needsSeparator) {
      out.push(closesHere ? `\\\\ ${curInner}$$` : `\\\\ ${trimmed}`);
    } else {
      out.push(line);
    }

    if (toggles) inDisplay = false;
  }

  return out.join('\n');
}

/**
 * Heals fractured math expressions and exponents split across dollar delimiters or OCR:
 * e.g. "$4.0 \times 10$−6 \text{kg}" -> "$4.0 \times 10^{-6}\ \text{kg}$"
 * e.g. "$10$−6" -> "$10^{-6}$"
 * e.g. "−$7.1\times10^{7}" -> "$-7.1\times10^{7}$"
 * e.g. "$7.1\times10^{7}\text{C} \text{kg}$−1" -> "$7.1\times10^{7}\text{ C kg}^{-1}$"
 * e.g. "$4.0 \times 10^{-6}$ \text{kg}" -> "$4.0 \times 10^{-6}\ \text{kg}$"
 */
export function healFracturedMath(text: string): string {
  if (!text) return '';
  let s = text;

  // 1. Minus sign outside opening $: −$7.1 \times 10^7 -> $-7.1 \times 10^7$
  s = s.replace(/(?:^|[\s(])([−–\-])\s*\$([^\$\n]+)\$/g, '$1-$2$');

  // 2. Exponent and unit outside closing $: `$4.0 \times 10$−6 \text{kg}` or `$4.0 \times 10$-6 kg`
  s = s.replace(
    /\$([^\$\n]+?)\$[ \t]*[−–\-^](\d+)[ \t]*(\\text\{[^\}]+\}|[a-zA-Z]+)(?=[ \t.,;:\n]|$)/g,
    '$$$1^{-\$2}\\ \$3$$'
  );

  // 3. Simple power outside closing $: `$10$−6` or `$10$-6` or `$10$^6`
  s = s.replace(
    /\$([^\$\n]+?)\$[ \t]*[−–\-^](\d+)(?=[ \t.,;:\n]|$)/g,
    '$$$1^{-\$2}$$'
  );

  // 4. `\text{...}` directly following closing $: `$4.0 \times 10^{-6}$ \text{kg}` -> `$4.0 \times 10^{-6}\ \text{kg}$`
  s = s.replace(
    /\$([^\$\n]+?)\$[ \t]*(\\text\{[^\}]+\})/g,
    '$$$1\\ \$2$$'
  );

  // 5. Trailing exponent outside closing $: `$7.1\times10^{7}\text{C} \text{kg}$−1` -> `$7.1\times10^{7}\text{C} \text{kg}^{-1}$`
  s = s.replace(
    /\$([^\$\n]+?)\$[ \t]*[−–\-^](\d+)/g,
    '$$$1^{-\$2}$$'
  );

  // 6. Strip multi-line AQA margin warnings, watermark PMT and stray box tokens
  s = s.replace(/(?:^|\r?\n)[ \t]*(?:Do not write[ \t]*\r?\n+[ \t]*outside the[ \t]*\r?\n+[ \t]*box|outside the[ \t]*\r?\n+[ \t]*box|Do not write outside the box|\bdo not write in this area\b|\bdo not write on this page\b)[ \t]*(?=\r?\n|$)/gi, '');
  s = s.replace(/(?:^|\r?\n)[ \t]*box[ \t]*(?=\r?\n+[ \t]*(?:0\s*)?\d)/gmi, '');
  s = s.replace(/^[ \t]*(?:box[ \t]+(?:0\s*)?\d+(?:[ \t]*\d+)?(?:[ \t]*\.[ \t]*\d+)?|\([a-z]\)[ \t]*box[ \t]*|box[ \t]+)/gmi, '');
  s = s.replace(/(?:^|\r?\n)[ \t]*PMT[ \t]*(?=\r?\n|$)/gmi, '');

  return s;
}

/**
 * Main delimiter and table healing function
 */
export function healLatexDelimiters(raw: string): string {
  if (!raw || !raw.trim()) return '';

  let s = raw;

  // Fenced code blocks and inline code spans are opaque to EVERY pass below —
  // including tab-mangled repair, placeholder stripping and the empty-display
  // strip. Mask them first, run the code-blind healers, then restore the exact
  // source text. Without this a literal `$$$` / `$$ $$` / placeholder-looking
  // string / TAB-mangled text inside code would be rewritten.
  const { masked, restore } = protectCode(s);
  s = masked;

  // -1. Escape-mangled LaTeX from stored payloads must be restored before
  //     any structural pass sees TAB + "ext" instead of \text.
  s = fixTabMangledLatex(s);

  // 0. Backend placeholder tokens must never reach the parser or DOM.
  s = stripPlaceholderTokens(s);
  // 0b. Phantom/empty display math blocks are stripped before healing so
  //     they cannot confuse delimiter pairing.
  s = stripEmptyDisplayMath(s);

  s = healFracturedMath(s);
  s = deduplicateRepeatedParagraphs(s);
  s = normalizeMarkdownTables(s);
  s = fixSpacedCommands(s);
  s = healMatrixEnvironments(s);
  s = healPolarAndDroppedEquations(s);
  s = stripOrphanedDollars(s);
  s = balanceMathEnvironments(s);
  s = validateAndEnforceDelimiters(s);
  // Multi-line display blocks need explicit \\ row separators — KaTeX
  // ignores raw newlines and would squash sequential equations end-to-end.
  s = ensureDisplayMathLineBreaks(s);
  // Isolate display math blocks ($$...$$) from adjacent text paragraphs
  s = isolateDisplayMathBlocks(s);

  // Healing can itself produce empty $$ pairs (e.g. stripping a stray $ from
  // "$$ $") — remove them as a final pass.
  s = stripEmptyDisplayMath(s);

  return restore(s);
}
