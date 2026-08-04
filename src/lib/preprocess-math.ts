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
 * Check if a line looks like a subquestion marker or new paragraph starter
 * that should force-close any open $$ block.
 * Matches: (a), (b), (i), (ii), 1., 2., **[, **], **, etc.
 */
function isSubquestionMarker(trimmed: string): boolean {
  // Matches: (a), (b), (1), (i), (A), etc. - parenthesized letter/number
  if (/^\([a-zA-Z0-9]+\)/.test(trimmed)) return true;
  // Matches: 1., 2., 1), 2) - numbered lists
  if (/^\d+[.)]/.test(trimmed)) return true;
  // Matches: **[text] - bold brackets (common in VLM output for subquestions)
  if (/^\*\*\[/.test(trimmed)) return true;
  // Matches: **text** at start of line (bold heading)
  if (/^\*\*.+\*\*$/.test(trimmed)) return true;
  // Matches: "Show that", "Prove that", "Find the" - common math problem starters
  if (/^(show that|prove that|find the|determine the|calculate the|evaluate the)\b/i.test(trimmed)) return true;
  return false;
}

/**
 * Wrap bare matrix/math environments in $$...$$
 * Uses a state machine to track if we're already inside math delimiters
 * Also handles Issue 1: auto-close unclosed $$ blocks at paragraph breaks
 * AND at subquestion markers (tightly packed text without blank lines)
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

    // Issue 1 Fix: If we're in an open $$ block and encounter an empty line (paragraph break)
    // OR a subquestion marker (tightly packed text without blank lines),
    // auto-close the $$ block at the end of the previous line.
    if (inDisplayMath && (trimmed === '' || isSubquestionMarker(trimmed))) {
      inDisplayMath = false;
      // Add closing $$ to the previous non-empty line in result
      for (let j = result.length - 1; j >= 0; j--) {
        if (result[j].trim() !== '') {
          result[j] = `${result[j]}\n$$`;
          break;
        }
      }
      // Don't add the empty line to result if it's a paragraph break
      if (trimmed === '') {
        continue;
      }
      // For subquestion markers, we DO add the line (it's the start of next content)
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
 * Check if a line is likely a display equation (not inline math in text).
 * Equations are typically math-dominant: they start with math symbols/commands,
 * contain multiple math constructs, and don't look like natural language sentences.
 */
function isLikelyDisplayEquation(trimmed: string): boolean {
  // Must have substantial math content
  const mathSyntaxRegex = /\\(?:cos|sin|tan|sec|csc|cot|cosh|sinh|tanh|frac|dfrac|cfrac|sqrt|left|right|le|ge|leq|geq|int|iint|iiint|oint|sum|prod|lim|vec|hat|bar|dot|ddot|tilde|binom|quad|qquad|overrightarrow|overleftarrow|overbrace|underbrace|widehat|widetilde|overline|underline)\b/;
  const mathSymbolRegex = /[=<>≤≥≠≈±×÷∂∇∞∫∑∏√]/;
  const hasMathSyntax = mathSyntaxRegex.test(trimmed) || mathSymbolRegex.test(trimmed);
  if (!hasMathSyntax) return false;

  // Quick exclusion: if line looks like a sentence (starts with common sentence starters)
  // These are natural language, not display equations
  const sentenceStarters = /^(the|this|that|these|those|it|we|you|i|a|an|in|on|at|by|for|with|as|is|are|was|were|has|have|had|be|been|being|do|does|did|can|could|will|would|should|may|might|must|here|there|where|when|why|how|what|which|who)\b/i;
  if (sentenceStarters.test(trimmed)) return false;

  // Quick exclusion: if line is a subquestion marker
  if (/^\([a-zA-Z0-9]+\)/.test(trimmed)) return false;
  if (/^\d+[.)]/.test(trimmed)) return false;
  if (/^\*\*\[/.test(trimmed)) return false;
  if (/^\*\*.+\*\*$/.test(trimmed)) return false;
  if (/^(show that|prove that|find the|determine the|calculate the|evaluate the)\b/i.test(trimmed)) return false;

  // Check if line starts with math-like content (not natural language)
  // Equations often start with: \, {, (, [, math symbol, digit, or single letter variable
  // But NOT with a word that looks like sentence start
  const startsWithMath = /^[\\{[(<]|\d|[a-zA-Z](?=\s*[=+\-*/^_|])/.test(trimmed);
  // Also allow single letter variables at start (like "H\cos" or "x = ")
  const startsWithSingleVar = /^[a-zA-Z](?=\s*[\\{[(<]|\s*[=+\-*/^_|]|\s*\\?[a-zA-Z])/.test(trimmed);
  if (!startsWithMath && !startsWithSingleVar) return false;

  // Check for multiple math constructs (indicates a full equation, not inline)
  const mathCommands = trimmed.match(/\\[a-zA-Z]+/g) || [];
  const mathSymbols = trimmed.match(/[=<>≤≥≠≈±×÷∂∇∞∫∑∏√]/g) || [];
  const totalMathTokens = mathCommands.length + mathSymbols.length;

  // Count natural language words (sequences of letters not preceded by \)
  // Exclude single letters (variables) and common math functions
  const textWords = (trimmed.match(/(?<!\\)[a-zA-Z]{3,}/g) || []).filter(w =>
    !/^(cos|sin|tan|sec|csc|cot|cosh|sinh|tanh|frac|sqrt|int|sum|prod|lim|vec|hat|bar|dot|ddot|tilde|binom|quad|qquad|overrightarrow|overleftarrow|overbrace|underbrace|widehat|widetilde|overline|underline|left|right|le|ge|leq|geq)$/i.test(w)
  );

  // If there are many math tokens relative to text words, it's likely an equation
  // Also require at least 2 math constructs or 1 complex command (like \frac, \int)
  const hasComplexMath = mathCommands.some(cmd =>
    /^(frac|dfrac|cfrac|sqrt|int|sum|prod|lim|binom|overrightarrow|overleftarrow|overbrace|underbrace)$/.test(cmd.slice(1))
  );

  // Must be math-dominant: math tokens >= text words, AND at least some math complexity
  const isMathDominant = totalMathTokens >= textWords.length;

  return isMathDominant && (totalMathTokens >= 2 || hasComplexMath);
}

/**
 * Issue 2 Fix: Wrap orphaned equations (standalone lines with heavy LaTeX syntax
 * but no $ delimiters) in $$...$$
 *
 * Detects lines that:
 * - Are likely display equations (math-dominant, not natural language)
 * - Contain NO $ delimiters
 * - Contain heavy LaTeX syntax (\cos, \sin, \frac, \left, \le, \ge, \int, etc.)
 * Works with tightly packed text (zero blank lines around equations).
 */
function wrapOrphanedMath(text: string): string {
  // Regex for heavy LaTeX math syntax indicators
  const mathSyntaxRegex = /\\(?:cos|sin|tan|sec|csc|cot|cosh|sinh|tanh|frac|dfrac|cfrac|sqrt|left|right|le|ge|leq|geq|int|iint|iiint|oint|sum|prod|lim|vec|hat|bar|dot|ddot|tilde|binom|quad|qquad|overrightarrow|overleftarrow|overbrace|underbrace|widehat|widetilde|overline|underline)\b/;

  // Also check for common math symbols that indicate an equation
  const mathSymbolRegex = /[=<>≤≥≠≈±×÷∂∇∞∫∑∏√]/;

  const lines = text.split('\n');
  const result: string[] = [];

  // Only consider "standalone" if there are multiple lines (multi-paragraph context)
  const isMultiLine = lines.length > 1;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();
    const prevLine = i > 0 ? lines[i - 1].trim() : '';
    const nextLine = i < lines.length - 1 ? lines[i + 1].trim() : '';

    // Check if this line is a standalone line (surrounded by ACTUAL empty lines in multi-paragraph text)
    // NOT just at boundaries of a single-line input
    const hasEmptyPrev = prevLine === '';
    const hasEmptyNext = nextLine === '';
    const isStandalone = isMultiLine && hasEmptyPrev && hasEmptyNext;

    const isLikelyEquation = isLikelyDisplayEquation(trimmed);

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

    if ((isStandalone || isLikelyEquation) && hasNoDelimiters && hasMathSyntax && !isMathEnvMarker && !isDelimiterOnly && !isEmpty) {
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