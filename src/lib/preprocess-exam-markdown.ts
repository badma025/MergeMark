/**
 * KaTeX Delimiter Healing & LaTeX Syntax Sanitization
 *
 * Runs on raw markdown string before it enters ReactMarkdown:
 * 1. Normalizes Markdown tables (injects missing GFM delimiters & blank lines)
 * 2. Strips orphaned trailing $ symbols (e.g. "[2 marks]$", "answer: $x$.$", "$$ $")
 * 3. Balances unclosed LaTeX math environments (\begin{pmatrix} -> \end{pmatrix})
 * 4. Balances inline ($) and block ($$) math delimiters
 * 5. Fixes spaced LaTeX commands ("\ frac" -> "\frac")
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
 * Normalizes Markdown tables produced by OCR / LLMs:
 * 1. Ensures blank lines before and after table blocks (required by CommonMark/GFM table parser).
 * 2. Injects missing delimiter rows (|---|---|...) if the LLM omitted the header separator.
 * 3. Trims pipe lines and cleans table boundaries.
 */
export function normalizeMarkdownTables(text: string): string {
  if (!text || !text.includes('|')) return text;

  const lines = text.split('\n');
  const result: string[] = [];
  let inTable = false;
  let tableLines: string[] = [];

  const isTableLine = (l: string) => {
    const trimmed = l.trim();
    return trimmed.startsWith('|') && trimmed.endsWith('|') && trimmed.length > 1;
  };

  const isDelimiterRow = (l: string) => /^\|(?:\s*:?-+:?\s*\|)+$/.test(l.trim());

  const flushTable = (tbl: string[]) => {
    if (tbl.length === 0) return;

    let repaired = tbl;
    // If only 1 line, or second line is not a delimiter row, create and insert one
    if (tbl.length >= 1 && (tbl.length === 1 || !isDelimiterRow(tbl[1]))) {
      const headerCols = tbl[0].split('|').filter((_, idx, arr) => idx > 0 && idx < arr.length - 1);
      const colCount = Math.max(headerCols.length, 1);
      const delimiterRow = '|' + ' --- |'.repeat(colCount);
      repaired = [tbl[0], delimiterRow, ...tbl.slice(1)];
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
    if (isTableLine(line)) {
      if (!inTable) {
        inTable = true;
        tableLines = [line.trim()];
      } else {
        tableLines.push(line.trim());
      }
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
  s = s.replace(/\$\$\s*\$(?!\$)/g, '$$$$');
  s = s.replace(/(?<!\$)\$\s*\$\$/g, '$$$$');

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
      const singleDollars = (part.match(/(?<!\\)\$/g) || []).length;
      if (singleDollars % 2 !== 0) {
        return part + '$';
      }
      return part;
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
 * Main delimiter and table healing function
 */
export function healLatexDelimiters(raw: string): string {
  if (!raw || !raw.trim()) return '';

  let s = raw;
  s = deduplicateRepeatedParagraphs(s);
  s = normalizeMarkdownTables(s);
  s = fixSpacedCommands(s);
  s = healMatrixEnvironments(s);
  s = healPolarAndDroppedEquations(s);
  s = stripOrphanedDollars(s);
  s = balanceMathEnvironments(s);
  s = validateAndEnforceDelimiters(s);

  return s;
}
