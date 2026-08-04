import { clsx, type ClassValue } from "clsx"
import { twMerge } from "tailwind-merge"

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

export function sanitizeMarkdownMath(text: string): string {
  if (!text) return "";

  let s = text;

  // 1. Fix space after backslash for LaTeX keywords: "\ begin" -> "\begin", "\ sqrt" -> "\sqrt", etc.
  s = s.replace(/\\ +(begin|end|frac|dfrac|cfrac|sqrt|text|textbf|textit|mathbf|mathit|mathrm|mathbb|mathcal|operatorname|pmatrix|bmatrix|vmatrix|matrix|array|cases|aligned|theta|lambda|alpha|beta|gamma|delta|Delta|pi|mu|sigma|omega|Omega|phi|Phi|psi|Psi|times|div|pm|mp|leq|geq|neq|approx|sim|equiv|subset|supset|subseteq|supseteq|in|notin|forall|exists|infty|partial|nabla|cos|sin|tan|sec|csc|cot|cosh|sinh|tanh|ln|log|exp|int|iint|iiint|oint|sum|prod|lim|vec|hat|bar|dot|ddot|tilde|binom|quad|qquad)(?![a-zA-Z])/gi, "\\$1");

  // 2. Fix multiple backslashes before LaTeX commands: "\\begin" -> "\begin", "\\pmatrix" -> "\pmatrix"
  s = s.replace(/\\{2,}(begin|end|frac|dfrac|cfrac|sqrt|text|textbf|textit|mathbf|mathit|mathrm|mathbb|mathcal|operatorname|pmatrix|bmatrix|vmatrix|matrix|array|cases|aligned|theta|lambda|alpha|beta|gamma|delta|Delta|pi|mu|sigma|omega|Omega|phi|Phi|psi|Psi|times|div|pm|mp|leq|geq|neq|approx|sim|equiv|in|forall|exists|infty|partial|nabla|cos|sin|tan|cosh|sinh|tanh|ln|log|exp|int|sum|prod|lim|vec|hat|bar|dot|ddot|tilde|binom)(?![a-zA-Z])/gi, "\\$1");

  // 3. Repair escaped equals (\= -> =)
  s = s.replace(/\\=/g, "=");

  // 4. Repair corrupted LaTeX braces (e.g. \mathbf\{ -> \mathbf{, \begin\{ -> \begin{)
  s = s.replace(/\\(mathbf|begin|end|frac|sqrt|text|textbf|textit|mathrm|mathit|pmatrix|bmatrix|vmatrix|matrix|array|cases|aligned|operatorname|vec|hat|bar|dot|ddot|tilde|mathcal|mathbb|binom|overline|underline)\s*\\\{/gi, "\\$1{");
  s = s.replace(/(\\[a-zA-Z]+{[^{}]+)\\\}/g, "$1}");
  s = s.replace(/\\}/g, "}");
  s = s.replace(/\\\{/g, "{");

  // 5. Clean up matrix internals: row breaks, stray backslashes before variables
  s = s.replace(/(\\begin\{(?:pmatrix|bmatrix|vmatrix|matrix|array|cases|aligned)\}[\s\S]*?\\end\{(?:pmatrix|bmatrix|vmatrix|matrix|array|cases|aligned)\})/g, (env) => {
    let cleanEnv = env;
    // Replace "\\ \ " or "\\ \" before a cell entry with "\\ "
    cleanEnv = cleanEnv.replace(/\\\\\s*\\(?=\s*[\-0-9a-zA-Z])/g, "\\\\ ");
    // Replace single backslash row breaks (not followed by a known command) with double backslashes
    cleanEnv = cleanEnv.replace(/(?<!\\)\\\s*(?=\r?\n|[\-0-9a-zA-Z&])/g, (m, offset, full) => {
      const rest = full.slice(offset + m.length);
      if (/^(?:begin|end|frac|sqrt|text|mathbf|mathit|mathrm|theta|lambda|alpha|beta|gamma|pi|sigma|pm|mp|times)/i.test(rest)) {
        return m;
      }
      return " \\\\ ";
    });
    // Remove stray backslash before plain variables inside matrix (e.g. "\ a" -> "a")
    cleanEnv = cleanEnv.replace(/(?<=[&,\s]|^)\\\s+([a-zA-Z0-9])/g, "$1");
    return cleanEnv;
  });

  // 6. Wrap unwrapped matrix / environment blocks cleanly in $$...$$
  s = s.replace(/\${0,2}\s*(\\begin\{(?:pmatrix|bmatrix|vmatrix|matrix|array|cases|aligned)\}[\s\S]*?\\end\{(?:pmatrix|bmatrix|vmatrix|matrix|array|cases|aligned)\})\s*\${0,2}/g, "\n\n$$\n$1\n$$\n\n");

  // 7. Collapse 3+ dollar signs to $$
  s = s.replace(/\${3,}/g, "$$");

  return s;
}
