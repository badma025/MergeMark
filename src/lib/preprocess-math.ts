/**
 * Pre-AST String Sanitizer
 *
 * Runs on RAW MARKDOWN STRING before it enters ReactMarkdown.
 * This phase protects structure BEFORE the Markdown parser sees it:
 * 1. Wraps bare matrix environments in $$...$$
 * 2. Strips blank lines inside math environments (prevents <p> injection)
 * 3. Balances unclosed $ and $$ delimiters
 * 4. Fixes obvious LaTeX typos (\ hline -> \hline, etc.)
 */

// Known LaTeX math environments that should be wrapped in display math
const MATH_ENVS = [
  'pmatrix', 'bmatrix', 'vmatrix', 'Vmatrix', 'matrix',
  'array', 'cases', 'aligned', 'gathered', 'align', 'align*',
  'alignat', 'alignat*', 'flalign', 'flalign*',
  'eqnarray', 'eqnarray*', 'multline', 'multline*',
  'split', 'subequations'
];

/**
 * Fix space after backslash for known LaTeX commands: "\ hline" -> "\hline"
 */
function fixSpacedCommands(text: string): string {
  // Use regex for this - it's a simple pattern
  return text.replace(
    /\\ +(begin|end|frac|dfrac|cfrac|sqrt|text|textbf|textit|mathbf|mathit|mathrm|mathbb|mathcal|operatorname|pmatrix|bmatrix|vmatrix|matrix|array|cases|aligned|hline|hlinex|cline|multicolumn|multirow|theta|lambda|alpha|beta|gamma|delta|Delta|pi|mu|sigma|omega|Omega|phi|Phi|psi|Psi|times|div|pm|mp|leq|geq|neq|approx|sim|equiv|subset|supset|subseteq|supseteq|in|notin|forall|exists|infty|partial|nabla|cos|sin|tan|sec|csc|cot|cosh|sinh|tanh|ln|log|exp|int|iint|iiint|oint|sum|prod|lim|vec|hat|bar|dot|ddot|tilde|binom|quad|qquad|overrightarrow|overleftarrow|overbrace|underbrace|widehat|widetilde|overline|underline)\b/g,
    '\\$1'
  );
}

/**
 * State machine to track if we're inside a math environment
 * and collapse double newlines to single newlines inside them
 * Only tracks \begin{env}...\end{env} environments, NOT $$...$$ blocks
 * (those are handled by wrapBareMathEnvs)
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
      // Inside math env: collapse consecutive blank lines
      if (trimmed === '') {
        blankLineCount++;
        if (blankLineCount === 1) {
          result.push(''); // Keep one blank line
        }
        // Skip additional blank lines
      } else {
        blankLineCount = 0;
        result.push(line);
      }
    } else {
      // Outside math env: keep all lines as-is
      blankLineCount = 0;
      result.push(line);
    }
  }

  return result.join('\n');
}

/**
 * Wrap bare matrix/math environments in $$...$$
 * Uses a state machine to track if we're already inside math delimiters
 * Also handles Issue 1: auto-close unclosed $$ blocks at paragraph breaks
 */
function wrapBareMathEnvs(text: string): string {
  const lines = text.split('\n');
  const result: string[] = [];

  let inDisplayMath = false; // $$ ... $$
  let inInlineMath = false;  // $ ... $
  let inMathEnv = false;
  let mathEnvName: string | null = null;
  let envStartIdx = -1; // Line index where env started

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();

    // Track display math ($$ ... $$) - detect delimiters anywhere in line
    // We need to count unescaped $$ pairs to toggle state correctly
    if (!inDisplayMath) {
      // Count unescaped $$ in the entire line (not just at start)
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
      // If odd number of $$, we enter display math
      if (dollarCount % 2 === 1) {
        inDisplayMath = true;
      }
    } else {
      // We're already in display math - check for closing $$
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
      // If odd number of $$, we close display math
      if (dollarCount % 2 === 1) {
        inDisplayMath = false;
      }
    }

    // Track inline math ($ ... $) - simplified, just toggle on single $ not part of $$
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

    // Issue 1 Fix: If we're in an open $$ block and encounter an empty line (paragraph break),
    // auto-close the $$ block at the end of the previous line.
    // Since blank lines inside math environments are already stripped by collapseBlankLinesInMathEnvs,
    // any empty line encountered while inDisplayMath is a guaranteed paragraph break where VLM forgot to close.
    if (inDisplayMath && trimmed === '') {
      inDisplayMath = false;
      // Add closing $$ to the previous non-empty line in result
      for (let j = result.length - 1; j >= 0; j--) {
        if (result[j].trim() !== '') {
          result[j] = `${result[j]}\n$$`;
          break;
        }
      }
      // Don't add the empty line to result (it's the paragraph break)
      continue;
    }

    // Check for math environment start (only if not already in math mode)
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
      // Check if this env is already wrapped (preceding line in result is $$)
      const isWrapped = envStartIdx > 0 && result[envStartIdx - 1].trim() === '$$';

      if (!isWrapped) {
        // Wrap the entire environment
        // Find the begin line in result and prepend $$
        for (let j = envStartIdx; j < result.length; j++) {
          if (result[j].trim().startsWith(`\\begin{${mathEnvName}}`)) {
            result[j] = `\n$$\n${result[j]}`;
            break;
          }
        }
        // Append $$ after this end line
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
 * Issue 2 Fix: Wrap orphaned equations (standalone lines with heavy LaTeX syntax
 * but no $ delimiters) in $$...$$
 *
 * Detects lines that:
 * - Sit on their own line (surrounded by \n\n or start/end of text)
 * - Contain NO $ delimiters
 * - Contain heavy LaTeX syntax (\cos, \sin, \frac, \left, \le, \ge, \int, etc.)
 */
function wrapOrphanedMath(text: string): string {
  // Regex for heavy LaTeX math syntax indicators
  const mathSyntaxRegex = /\\(?:cos|sin|tan|sec|csc|cot|cosh|sinh|tanh|frac|dfrac|cfrac|sqrt|left|right|le|ge|leq|geq|int|iint|iiint|oint|sum|prod|lim|vec|hat|bar|dot|ddot|tilde|binom|quad|qquad|overrightarrow|overleftarrow|overbrace|underbrace|widehat|widetilde|overline|underline)\b/;

  // Also check for common math symbols that indicate an equation
  const mathSymbolRegex = /[=<>≤≥≠≈±×÷∂∇∞∫∑∏√]/;

  const lines = text.split('\n');
  const result: string[] = [];

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();
    const prevLine = i > 0 ? lines[i - 1].trim() : '';
    const nextLine = i < lines.length - 1 ? lines[i + 1].trim() : '';

    // Check if this line is a standalone line (surrounded by empty lines or boundaries)
    const isStandalone = (prevLine === '' || i === 0) && (nextLine === '' || i === lines.length - 1);

    // Check if line contains NO $ delimiters (unescaped)
    const hasNoDelimiters = !/[^\\]\$/.test(line) && !/^\$/.test(line);

    // Check if line contains heavy LaTeX math syntax
    const hasMathSyntax = mathSyntaxRegex.test(trimmed) || mathSymbolRegex.test(trimmed);

    // Skip if line is already inside a math environment (starts with \begin or \end)
    const isMathEnvMarker = trimmed.startsWith('\\begin{') || trimmed.startsWith('\\end{');

    // Skip if line is just a bare $$ delimiter
    const isDelimiterOnly = trimmed === '$$' || trimmed === '$';

    // Skip if line is empty
    const isEmpty = trimmed === '';

    if (isStandalone && hasNoDelimiters && hasMathSyntax && !isMathEnvMarker && !isDelimiterOnly && !isEmpty) {
      // Wrap this orphaned equation in $$...$$
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
 * Balance unclosed $ and $$ delimiters at end of document
 */
function balanceDelimiters(text: string): string {
  let inlineCount = 0;
  let inDisplayMath = false;
  let inInlineMath = false;

  for (let i = 0; i < text.length; i++) {
    if (text[i] === '$') {
      const isEscaped = i > 0 && text[i - 1] === '\\';
      const isDouble = i + 1 < text.length && text[i + 1] === '$';

      if (isEscaped) continue;

      if (isDouble) {
        inDisplayMath = !inDisplayMath;
        i++; // Skip next $
      } else {
        inInlineMath = !inInlineMath;
        if (inInlineMath) inlineCount++;
        else inlineCount--;
      }
    }
  }

  let result = text;

  // Close unclosed display math
  if (inDisplayMath) {
    result += '\n$$';
  }

  // Close unclosed inline math (add $ for each unclosed)
  while (inlineCount > 0) {
    result += '$';
    inlineCount--;
  }

  return result;
}

/**
 * Main entry point: preprocess raw markdown string before ReactMarkdown
 */
export function preprocessMathString(raw: string): string {
  if (!raw || !raw.trim()) return '';

  let s = raw;

  // 1. Fix obvious LaTeX typos (space after backslash)
  s = fixSpacedCommands(s);

  // 2. Collapse blank lines inside math environments (prevents <p> injection)
  s = collapseBlankLinesInMathEnvs(s);

  // 3. Wrap bare math environments in $$...$$ (includes Issue 1: auto-close at double newlines)
  s = wrapBareMathEnvs(s);

  // 4. Wrap orphaned equations (Issue 2: standalone lines with heavy LaTeX syntax)
  s = wrapOrphanedMath(s);

  // 5. Balance any unclosed delimiters
  s = balanceDelimiters(s);

  // 6. Collapse 3+ newlines to 2 (clean up)
  s = s.replace(/\n{3,}/g, '\n\n');

  return s.trim();
}

/**
 * Also export a simpler version for use in places that just need string sanitization
 * without the full ReactMarkdown pipeline (e.g., LaTeX export)
 */
export function sanitizeForLatex(raw: string): string {
  if (!raw || !raw.trim()) return '';

  let s = raw;
  s = fixSpacedCommands(s);
  s = collapseBlankLinesInMathEnvs(s);
  s = balanceDelimiters(s);
  s = s.replace(/\n{3,}/g, '\n\n');

  return s.trim();
}