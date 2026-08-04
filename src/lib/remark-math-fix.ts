/**
 * Phase B: AST Plugin - Runs AFTER remark-math parses math nodes
 *
 * This plugin mutates already-parsed math AST nodes to fix issues that
 * can only be safely fixed AFTER the Markdown parser has processed the text:
 * 1. Fix single backslash row separators → double backslash inside math nodes
 * 2. Strip nested $ delimiters inside display math nodes
 * 3. Repair corrupted LaTeX commands (hline → \hline, etc.)
 * 4. Normalize matrix spacing
 */

import type { Root, Node } from 'mdast';
import type { Math } from 'mdast-util-math';
import { visit } from 'unist-util-visit';

// Commands that are frequently corrupted (missing backslash)
const CORRUPTED_COMMANDS = new Set([
  'hline', 'hlinex', 'cline', 'multicolumn', 'multirow',
  'begin', 'end', 'frac', 'dfrac', 'cfrac', 'sqrt',
  'text', 'textbf', 'textit', 'mathbf', 'mathit', 'communityMath',
  'mathbb', 'mathcal', 'operatorname',
  'pmatrix', 'bmatrix', 'vmatrix', 'matrix', 'array', 'cases', 'aligned',
  'theta', 'lambda', 'alpha', 'beta', 'gamma', 'delta', 'Delta',
  'pi', 'mu', 'sigma', 'omega', 'Omega', 'phi', 'Phi',
  'psi', 'Psi', 'times', 'div', 'pm', 'mp',
  'leq', 'geq', 'neq', 'approx', 'sim', 'equiv',
  'subset', 'supset', 'subseteq', 'supseteq', 'in', 'notin',
  'forall', 'exists', 'infty', 'partial', 'nabla',
  'cos', 'sin', 'tan', 'sec', 'csc', 'cot', 'cosh', 'sinh', 'tanh',
  'ln', 'log', 'exp', 'int', 'iint', 'iiint', 'oint',
  'sum', 'prod', 'lim', 'vec', 'hat', 'bar', 'dot', 'ddot', 'tilde',
  'binom', 'quad', 'qquad', 'overrightarrow', 'overleftarrow',
  'overbrace', 'underbrace', 'widehat', 'widetilde',
  'overline', 'underline', 'widehat', 'widetilde'
]);

// Math environments that use row separators
const MATRIX_ENVS = new Set([
  'pmatrix', 'bmatrix', 'vmatrix', 'Vmatrix', 'matrix',
  'array', 'cases', 'aligned', 'align', 'align*',
  'alignat', 'alignat*', 'flalign', 'flalign*',
  'eqnarray', 'eqnarray*', 'multline', 'multline*',
  'split', 'subequations', 'gathered'
]);

/**
 * Check if a math node value contains a matrix/array environment
 */
function containsMatrixEnv(value: string): boolean {
  for (const env of MATRIX_ENVS) {
    if (value.includes(`\\begin{${env}}`)) {
      return true;
    }
  }
  return false;
}

/**
 * Fix single backslash row separators in matrix environments
 * Only touches lines that are clearly matrix row separators
 */
function fixMatrixRowSeparators(value: string): string {
  // First, find all \begin{env}...\end{env} blocks
  let result = value;

  for (const env of MATRIX_ENVS) {
    const beginRegex = new RegExp(`\\\\begin\\{${env}\\}`, 'g');
    const matches = [];
    let match;

    while ((match = beginRegex.exec(value)) !== null) {
      matches.push({ env, start: match.index, endTag: `\\end{${env}}` });
    }

    // Process in reverse to maintain indices
    for (let i = matches.length - 1; i >= 0; i--) {
      const m = matches[i];
      const endPos = value.indexOf(m.endTag, m.start);
      if (endPos === -1) continue;

      const block = value.slice(m.start, endPos + m.endTag.length);
      const fixedBlock = fixMatrixBlock(block, env);

      result = result.slice(0, m.start) + fixedBlock + result.slice(m.start + block.length);
    }
  }

  return result;
}

/**
 * Fix a single matrix/array block's row separators
 */
function fixMatrixBlock(block: string, _env: string): string {
  // Split by lines, fix row separators
  const lines = block.split('\n');
  const result: string[] = [];

  for (let i = 0; i < lines.length; i++) {
    let line = lines[i];

    // Check if this is a row separator line (ends with single \ not followed by known command)
    if (line.endsWith('\\') && !line.endsWith('\\\\')) {
      // Look ahead: is the next non-empty content a matrix cell?
      let nextContent = '';
      for (let j = i + 1; j < lines.length; j++) {
        const next = lines[j].trim();
        if (next) {
          nextContent = next;
          break;
        }
      }

      // If next content starts with matrix cell content (not a command), fix to \\
      if (nextContent &&
          !/^\\(?:begin|end|frac|sqrt|text|mathbf|mathit|mathrm|mathbb|mathcal|operatorname|theta|lambda|alpha|beta|gamma|delta|pi|sigma|omega|phi|psi|cos|sin|tan|ln|log|exp|int|sum|prod|lim|vec|hat|bar|dot|ddot|tilde|binom|quad|quad|times|div|pm|mp|leq|geq|approx|sim|equiv|subset|supset|in|forall|exists|infty|partial|nabla|overline|underline|overrightarrow|overleftarrow|overbrace|underbrace|widehat|widetilde|co?hline|cline|multicolumn|multirow)/i.test(nextContent)) {
        line = line.replace(/\\$/, '\\\\');
      }
    }

    result.push(line);
  }

  return result.join('\n');
}

/**
 * Strip nested $ delimiters inside display math
 * e.g. $$\int_0^1 $x$ dx$$ -> $$\int_0^1 x dx$$
 */
function stripNestedDollars(value: string): string {
  // Only process display math (starts/ends with $$ or \[)
  const isDisplay = value.startsWith('$$') && value.endsWith('$$') ||
                    value.startsWith('\\[') && value.endsWith('\\]');

  if (!isDisplay) return value;

  // Remove inner $...$ but keep content
  return value.replace(/\$([^$\n]+)\$/g, '$1');
}

/**
 * Repair corrupted commands (missing backslash)
 * e.g. "hline" -> "\hline", "begin" -> "\begin"
 * Only applies when clearly a LaTeX command in math context
 */
function repairCorruptedCommands(value: string): string {
  let result = value;

  // Pattern: word boundary, then known command, then word boundary or { or [
  // But only when NOT already preceded by \
  for (const cmd of CORRUPTED_COMMANDS) {
    // Match command not preceded by \ (and not part of a longer word)
    const regex = new RegExp(`(?<!\\\\)\\b${cmd}\\b(?=[\\s{[]|$)`, 'g');
    result = result.replace(regex, `\\${cmd}`);
  }

  return result;
}

/**
 * Normalize spacing around & and \\ in matrix environments
 */
function normalizeMatrixSpacing(value: string): string {
  let result = value;

  // Normalize & spacing: " & " or "&" -> " & " (consistent)
  result = result.replace(/\\s*&\\s*/g, ' & ');

  // Normalize \\ spacing: ensure space before \\
  result = result.replace(/([^\\s])\\\\/g, '$1 \\\\');

  // Fix double/triple backslashes
  result = result.replace(/\\\\\\\\/g, '\\\\');

  return result;
}

/**
 * Main visitor function for math nodes
 */
function mathVisitor(tree: Root): void {
  visit(tree, 'math', (node: Math, index: number | undefined, parent: Node | undefined) => {
    if (!parent || index === null) return;

    const value = node.value;
    if (!value || typeof value !== 'string') return;

    let fixed = value;

    // Strip nested $ in display math
    // remark-math adds metadata about whether it's display math
    const meta = node.meta as Record<string, unknown> | undefined;
    const isDisplay = meta?.display === true ||
                      (fixed.trim().startsWith('$$') && fixed.trim().endsWith('$$')) ||
                      (fixed.trim().startsWith('\\[') && fixed.trim().endsWith('\\]'));

    if (isDisplay) {
      fixed = stripNestedDollars(fixed);
    }

    // Fix matrix row separators
    if (containsMatrixEnv(fixed)) {
      fixed = fixMatrixRowSeparators(fixed);
      fixed = normalizeMatrixSpacing(fixed);
    }

    // Repair corrupted commands
    fixed = repairCorruptedCommands(fixed);

    // Fix common typos: \ begin -> \begin etc. (space after backslash)
    fixed = fixed.replace(/\\ +(begin|end|frac|dfrac|cfrac|sqrt|text|textbf|textit|mathbf|mathit|mathrm|mathbb|mathcal|operatorname|pmatrix|bmatrix|vmatrix|matrix|array|cases|aligned|theta|lambda|alpha|beta|gamma|delta|Delta|pi|mu|sigma|omega|Omega|phi|Phi|psi|Psi|times|div|pm|mp|leq|geq|neq|approx|sim|equiv|subset|supset|subseteq|supseteq|in|notin|forall|exists|infty|partial|nabla|cos|sin|tan|sec|csc|cot|cosh|sinh|tanh|ln|log|exp|int|iint|iiint|oint|sum|prod|lim|vec|hat|bar|dot|ddot|tilde|binom|quad|qquad|hline|hlinex|cline|multicolumn|multirow)\\b/g, '\\$1');

    if (fixed !== value) {
      node.value = fixed;
    }
  });
}

/**
 * Remark plugin: run after remark-math to fix math AST nodes
 */
export function remarkMathFix() {
  return function transformer(tree: Root) {
    mathVisitor(tree);
  };
}

export default remarkMathFix;