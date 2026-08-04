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

    // Track display math ($$ ... $$)
    if (trimmed === '$$') {
      inDisplayMath = !inDisplayMath;
      // If closing display math and we have a buffered env, flush it
      if (!inDisplayMath && inMathEnv) {
        // Find the start of the env in result and wrap it
        for (let j = envStartIdx; j < result.length; j++) {
          if (result[j].trim().startsWith(`\\begin{${mathEnvName}}`)) {
            result[j] = `$$\n${result[j]}`;
            break;
          }
        }
        result[result.length - 1] = `${result[result.length - 1]}\n$$`;
        inMathEnv = false;
        mathEnvName = null;
        envStartIdx = -1;
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

  // 3. Wrap bare math environments in $$...$$
  s = wrapBareMathEnvs(s);

  // 4. Balance any unclosed delimiters
  s = balanceDelimiters(s);

  // 5. Collapse 3+ newlines to 2 (clean up)
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