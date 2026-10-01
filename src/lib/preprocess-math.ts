/**
 * Pre-AST String Sanitizer
 *
 * Runs on RAW MARKDOWN STRING before it enters ReactMarkdown.
 * This phase protects structure BEFORE the Markdown parser sees it:
 * 1. Wraps bare matrix environments in $$...$$
 * 2. Strips blank lines inside math environments (prevents <p> injection)
 * 3. Strips orphaned trailing $ symbols (e.g. "[2 marks]$")
 * 4. Balances unclosed LaTeX math environments and unclosed $ / $$ delimiters
 * 5. Fixes obvious LaTeX typos (\ hline -> \hline, etc.)
 * 6. Detects and normalizes sequential MCQ options (A, B, C, D)
 */

import { MATH_ENVS, healLatexDelimiters, fixSpacedCommands, stripPlaceholderTokens, stripEmptyDisplayMath, fixTabMangledLatex, protectCode } from './preprocess-exam-markdown';
import { normalizeMCQOptions } from './preprocess-mcq';

/**
 * State machine to track if we're inside a math environment
 * and collapse double newlines to single newlines inside them.
 * Only tracks \begin{env}...\end{env} environments.
 */
function collapseBlankLinesInMathEnvs(text: string): string {
  const lines = text.split('\n');
  const result: string[] = [];

  let inMathEnv = false;
  let mathEnvName: string | null = null;
  let blankLineCount = 0;

  for (const line of lines) {
    const trimmed = line.trim();

    // Check for environment start
    if (!inMathEnv) {
      for (const env of MATH_ENVS) {
        if (trimmed.startsWith(`\\begin{${env}}`)) {
          inMathEnv = true;
          mathEnvName = env;
          break;
        }
      }
    }

    // Check for environment end
    if (inMathEnv && mathEnvName && trimmed === `\\end{${mathEnvName}}`) {
      inMathEnv = false;
      mathEnvName = null;
    }

    if (inMathEnv) {
      if (trimmed === '') {
        blankLineCount++;
        if (blankLineCount === 1) {
          result.push(''); // Keep one blank line
        }
      } else {
        blankLineCount = 0;
        result.push(line);
      }
    } else {
      blankLineCount = 0;
      result.push(line);
    }
  }

  return result.join('\n');
}

/**
 * Check if a line looks like a subquestion marker or new paragraph starter
 * that should force-close any open $$ block.
 * Matches: (a), (b), (i), (ii), 1., 2., **[, **], **, etc.
 */
function isSubquestionMarker(trimmed: string): boolean {
  if (/^\([a-zA-Z0-9]+\)/.test(trimmed)) return true;
  if (/^\d+[.)]/.test(trimmed)) return true;
  if (/^\*\*\[/.test(trimmed)) return true;
  if (/^\*\*.+\*\*$/.test(trimmed)) return true;
  if (/^(show that|prove that|find the|determine the|calculate the|evaluate the)\b/i.test(trimmed)) return true;
  return false;
}

/**
 * Wrap bare matrix/math environments in $$...$$
 */
function wrapBareMathEnvs(text: string): string {
  const lines = text.split('\n');
  const result: string[] = [];

  let inDisplayMath = false;
  let inInlineMath = false;
  let inMathEnv = false;
  let mathEnvName: string | null = null;
  let envStartIdx = -1;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();

    // Track display math ($$ ... $$)
    if (!inDisplayMath) {
      let dollarCount = 0;
      for (let charIdx = 0; charIdx < line.length;) {
        if (line[charIdx] === '$') {
          const isEscaped = charIdx > 0 && line[charIdx - 1] === '\\';
          const isDouble = charIdx + 1 < line.length && line[charIdx + 1] === '$';
          if (!isEscaped && isDouble) {
            dollarCount++;
            charIdx += 2;
          } else {
            charIdx++;
          }
        } else {
          charIdx++;
        }
      }
      if (dollarCount % 2 === 1) {
        inDisplayMath = true;
      }
    } else {
      let dollarCount = 0;
      for (let charIdx = 0; charIdx < line.length;) {
        if (line[charIdx] === '$') {
          const isEscaped = charIdx > 0 && line[charIdx - 1] === '\\';
          const isDouble = charIdx + 1 < line.length && line[charIdx + 1] === '$';
          if (!isEscaped && isDouble) {
            dollarCount++;
            charIdx += 2;
          } else {
            charIdx++;
          }
        } else {
          charIdx++;
        }
      }
      if (dollarCount % 2 === 1) {
        inDisplayMath = false;
      }
    }

    // Track inline math ($ ... $)
    if (!inDisplayMath) {
      let charIdx = 0;
      while (charIdx < line.length) {
        if (line[charIdx] === '$') {
          const isEscaped = charIdx > 0 && line[charIdx - 1] === '\\';
          const isDouble = charIdx + 1 < line.length && line[charIdx + 1] === '$';
          if (!isEscaped && !isDouble) {
            inInlineMath = !inInlineMath;
          }
          charIdx++;
        } else {
          charIdx++;
        }
      }
    }

    // Auto-close open $$ block on paragraph break or subquestion marker
    if (inDisplayMath && (trimmed === '' || isSubquestionMarker(trimmed))) {
      inDisplayMath = false;
      for (let j = result.length - 1; j >= 0; j--) {
        if (result[j].trim() !== '') {
          result[j] = `${result[j]}\n$$`;
          break;
        }
      }
      if (trimmed === '') {
        continue;
      }
    }

    // Check for math environment start
    if (!inDisplayMath && !inInlineMath && !inMathEnv) {
      for (const env of MATH_ENVS) {
        if (trimmed.startsWith(`\\begin{${env}}`)) {
          inMathEnv = true;
          mathEnvName = env;
          envStartIdx = result.length;
          break;
        }
      }
    }

    // Check for math environment end
    if (inMathEnv && mathEnvName && trimmed === `\\end{${mathEnvName}}`) {
      const isWrapped = envStartIdx > 0 && result[envStartIdx - 1].trim() === '$$';

      if (!isWrapped) {
        for (let j = envStartIdx; j < result.length; j++) {
          if (result[j].trim().startsWith(`\\begin{${mathEnvName}}`)) {
            result[j] = `\n$$\n${result[j]}`;
            break;
          }
        }
        result.push(`${line}\n$$`);
      } else {
        result.push(line);
      }

      inMathEnv = false;
      mathEnvName = null;
      envStartIdx = -1;
      continue;
    }

    result.push(line);
  }

  return result.join('\n');
}

/**
 * Check if a line is likely a display equation (not inline math in text).
 */
function isLikelyDisplayEquation(trimmed: string): boolean {
  const mathSyntaxRegex = /\\(?:cos|sin|tan|sec|csc|cot|cosh|sinh|tanh|frac|dfrac|cfrac|sqrt|left|right|le|ge|leq|geq|int|iint|iiint|oint|sum|prod|lim|vec|hat|bar|dot|ddot|tilde|binom|quad|qquad|overrightarrow|overleftarrow|overbrace|underbrace|widehat|widetilde|overline|underline)\b/;
  const mathSymbolRegex = /[=<>≤≥≠≈±×÷∂∇∞∫∑∏√]/;
  const hasMathSyntax = mathSyntaxRegex.test(trimmed) || mathSymbolRegex.test(trimmed);
  if (!hasMathSyntax) return false;

  const sentenceStarters = /^(the|this|that|these|those|it|we|you|i|a|an|in|on|at|by|for|with|as|is|are|was|were|has|have|had|be|been|being|do|does|did|can|could|will|would|should|may|might|must|here|there|where|when|why|how|what|which|who)\b/i;
  if (sentenceStarters.test(trimmed)) return false;

  if (/^\([a-zA-Z0-9]+\)/.test(trimmed)) return false;
  if (/^\d+[.)]/.test(trimmed)) return false;
  if (/^\*\*\[/.test(trimmed)) return false;
  if (/^\*\*.+\*\*$/.test(trimmed)) return false;
  if (/^(show that|prove that|find the|determine the|calculate the|evaluate the)\b/i.test(trimmed)) return false;

  const startsWithMath = /^[\\{[(<]|\d|[a-zA-Z](?=\s*[=+\-*/^_|])/.test(trimmed);
  const startsWithSingleVar = /^[a-zA-Z](?=\s*[\\{[(<]|\s*[=+\-*/^_|]|\s*\\?[a-zA-Z])/.test(trimmed);
  if (!startsWithMath && !startsWithSingleVar) return false;

  const mathCommands = trimmed.match(/\\[a-zA-Z]+/g) || [];
  const mathSymbols = trimmed.match(/[=<>≤≥≠≈±×÷∂∇∞∫∑∏√]/g) || [];
  const totalMathTokens = mathCommands.length + mathSymbols.length;

  const textWords = (trimmed.match(/(?<!\\)[a-zA-Z]{3,}/g) || []).filter(w =>
    !/^(cos|sin|tan|sec|csc|cot|cosh|sinh|tanh|frac|sqrt|int|sum|prod|lim|vec|hat|bar|dot|ddot|tilde|binom|quad|qquad|overrightarrow|overleftarrow|overbrace|underbrace|widehat|widetilde|overline|underline|left|right|le|ge|leq|geq)$/i.test(w)
  );

  const hasComplexMath = mathCommands.some(cmd =>
    /^(frac|dfrac|cfrac|sqrt|int|sum|prod|lim|binom|overrightarrow|overleftarrow|overbrace|underbrace)$/.test(cmd.slice(1))
  );

  const isMathDominant = totalMathTokens >= textWords.length;

  return isMathDominant && (totalMathTokens >= 2 || hasComplexMath);
}

/**
 * Wrap orphaned equations (standalone lines with heavy LaTeX syntax but no $ delimiters)
 */
function wrapOrphanedMath(text: string): string {
  const mathSyntaxRegex = /\\(?:cos|sin|tan|sec|csc|cot|cosh|sinh|tanh|frac|dfrac|cfrac|sqrt|left|right|le|ge|leq|geq|int|iint|iiint|oint|sum|prod|lim|vec|hat|bar|dot|ddot|tilde|binom|quad|qquad|overrightarrow|overleftarrow|overbrace|underbrace|widehat|widetilde|overline|underline)\b/;
  const mathSymbolRegex = /[=<>≤≥≠≈±×÷∂∇∞∫∑∏√]/;

  const lines = text.split('\n');
  const result: string[] = [];
  const isMultiLine = lines.length > 1;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();
    const prevLine = i > 0 ? lines[i - 1].trim() : '';
    const nextLine = i < lines.length - 1 ? lines[i + 1].trim() : '';

    const hasEmptyPrev = prevLine === '';
    const hasEmptyNext = nextLine === '';
    const isStandalone = isMultiLine && hasEmptyPrev && hasEmptyNext;
    const isLikelyEquation = isLikelyDisplayEquation(trimmed);

    const hasNoDelimiters = !/[^\\]\$/.test(line) && !/^\$/.test(line);
    const hasMathSyntax = mathSyntaxRegex.test(trimmed) || mathSymbolRegex.test(trimmed);
    const isMathEnvMarker = trimmed.startsWith('\\begin{') || trimmed.startsWith('\\end{');
    const isDelimiterOnly = trimmed === '$$' || trimmed === '$';
    const isEmpty = trimmed === '';

    if ((isStandalone || isLikelyEquation) && hasNoDelimiters && hasMathSyntax && !isMathEnvMarker && !isDelimiterOnly && !isEmpty) {
      result.push('$$');
      result.push(line);
      result.push('$$');
    } else {
      result.push(line);
    }
  }

  return result.join('\n');
}

/**
 * Normalizes unicode math debris that survives legacy imports (trap class T4):
 * - U+2212 MINUS SIGN / en dash -> ASCII hyphen-minus
 * - "\text{kg} \text{m}-3"      -> "\text{kg} \text{m}^{-3}"
 * - "x × 10-15" / "10-15 m"     -> "x \times 10^{-15}" / "$10^{-15}$ m"
 * Runs before any structural pass so later $-balancing sees clean input.
 */
export function normalizeUnicodeMath(text: string): string {
  if (!text) return text;
  let s = text.replace(/\u2212/g, '-').replace(/\u2013/g, '-');
  s = s.replace(/\\text\{([A-Za-z]+)\}[ ]?-(\d)/g, '\\text{$1}^{-$2}');
  s = s.replace(/(?:×|\\times)[ \t]*10-(\d+)/g, (_m, d: string) => `\\times 10^{-${d}}`);
  // Bare "10-15" outside math: preceding char consumed & re-emitted (no lookbehind in JS).
  s = s.replace(/(^|[^$\w^{])10-(\d+)(?![\w}])/g, (_m, pre: string, d: string) => `${pre}$10^{-${d}}$`);
  return s;
}

/**
 * Master Pre-AST Sanitizer:
 * Executes on raw string content before entering ReactMarkdown.
 */
export function preprocessExamMarkdown(raw: string): string {
  if (!raw || !raw.trim()) return '';

  let s = raw;

  // Fenced code blocks and inline code spans are opaque to EVERY pass in this
  // chain, including tab-mangled repair, MCQ normalization, unicode math
  // normalization, placeholder stripping and the empty-display strip. Mask
  // first, run the whole transform stack, then restore the exact source text.
  const { masked, restore } = protectCode(s);
  s = masked;

  // 0. Escape-mangled LaTeX (unescaped "\text" stored as TAB + "ext") is
  //    restored before any structural pass runs.
  s = fixTabMangledLatex(s);

  // Establish option boundaries before global math healing can interpret a
  // malformed dollar run as a display block spanning several choices.
  s = normalizeMCQOptions(s);

  // 0.5 Unicode math debris (U+2212 minus, plain negative exponents) is
  //     normalized before delimiter healing so `$` balancing operates on
  //     the intended math spans, not on minus-sign noise.
  s = normalizeUnicodeMath(s);

  // 0. Backend placeholder tokens ([DIAGRAM_PLACEHOLDER] /
  //    [VISUAL_MCQ_PLACEHOLDER]) and phantom/empty display math blocks are
  //    stripped BEFORE any structural pass so they can never reach the
  //    markdown parser or the DOM.
  s = stripPlaceholderTokens(s);
  s = stripEmptyDisplayMath(s);

  // 1. Fix obvious LaTeX typos (space after backslash)
  s = fixSpacedCommands(s);

  // 2. Collapse blank lines inside math environments
  s = collapseBlankLinesInMathEnvs(s);

  // 3. Wrap bare math environments in $$...$$
  s = wrapBareMathEnvs(s);

  // 4. Wrap orphaned equations
  s = wrapOrphanedMath(s);

  // 5. Heal LaTeX delimiters (strip orphaned trailing $, balance environments and delimiters)
  s = healLatexDelimiters(s);

  // 6. Detect and restructure MCQs into standardized components
  s = normalizeMCQOptions(s);

  // 7. Clean excessive blank lines (3+ to 2)
  s = s.replace(/\n{3,}/g, '\n\n');

  return restore(s).trim();
}

/**
 * Alias for backward compatibility with existing components
 */
export const preprocessMathString = preprocessExamMarkdown;

/**
 * Sanitizer for LaTeX export
 */
export function sanitizeForLatex(raw: string): string {
  if (!raw || !raw.trim()) return '';

  let s = raw;
  s = fixSpacedCommands(s);
  s = collapseBlankLinesInMathEnvs(s);
  s = healLatexDelimiters(s);
  s = s.replace(/\n{3,}/g, '\n\n');

  return s.trim();
}
