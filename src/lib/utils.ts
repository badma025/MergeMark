import { clsx, type ClassValue } from "clsx"
import { twMerge } from "tailwind-merge"

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

const OP_MAP: [string, string][] = [
  ["\\text{arcosh}", "\\operatorname{arcosh}"],
  ["\\text{arsinh}", "\\operatorname{arsinh}"],
  ["\\text{artanh}", "\\operatorname{artanh}"],
  ["\\text{arcsec}", "\\operatorname{arcsec}"],
  ["\\text{arccsc}", "\\operatorname{arccsc}"],
  ["\\text{arccot}", "\\operatorname{arccot}"],
  ["\\text{ln}", "\\ln"],
  ["\\text{sin}", "\\sin"],
  ["\\text{cos}", "\\cos"],
  ["\\text{tan}", "\\tan"],
  ["\\text{sec}", "\\sec"],
  ["\\text{csc}", "\\csc"],
  ["\\text{cot}", "\\cot"],
  ["\\text{log}", "\\log"],
  ["\\text{exp}", "\\exp"],
  ["\\text{lim}", "\\lim"],
  ["\\text{max}", "\\max"],
  ["\\text{min}", "\\min"],
  ["\\text{sup}", "\\sup"],
  ["\\text{inf}", "\\inf"],
];

export function normalizeLatexOperators(text: string): string {
  if (!text) return text;
  let s = text;
  for (const [oldOp, newOp] of OP_MAP) {
    s = s.split(oldOp).join(newOp);
  }
  return s;
}

export function sanitizeMarkdownMath(text: string): string {
  if (!text) return text;
  text = normalizeLatexOperators(text);
  // Match the backend sanitizer so existing cards with older malformed
  // content are repaired before remark-math sees them.
  text = text.replace(/\r\n?/g, "\n");
  text = text.replace(/\s*(!\[.*?\]\(.*?\))\s*/gs, "\n\n$1\n\n");
  text = text.replace(/^((?:(?!\$\$).)*)\$\$\s*$/gm, (whole, body: string) => {
    if (!body.trim()) return whole;
    // A whole-line display cannot contain nested inline delimiters.
    const displayBody = body.replace(/(^|[^\\])\$/g, "$1");
    return `$$${displayBody}$$`;
  });
  text = text.replace(/,\s*\$\$/g, () => "$$");
  text = text.replace(/\.\s*\$\$/g, () => "$$.");

  let inBlock = false;
  let lines = text.split('\n');
  let outputLines = [];

  for (let line of lines) {
      let trimmed = line.trim();
      if (trimmed === "$$") {
          inBlock = !inBlock;
          outputLines.push(line);
          continue;
      }

      if (inBlock) {
          outputLines.push(line);
          continue;
      }

      let processedLine = "";
      let inlineCount = 0;
      let i = 0;
      let inInline = false;
      while (i < line.length) {
          if (line[i] === '$') {
              let escaped = i > 0 && line[i - 1] === '\\';
              let double = i + 1 < line.length && line[i + 1] === '$';
              if (!escaped) {
                  if (double) {
                      processedLine += "$$";
                      i += 2;
                      continue;
                  } else {
                      inlineCount += 1;
                      inInline = !inInline;
                  }
              }
              processedLine += "$";
          } else if (!inInline) {
              if (line[i] === '<') {
                  processedLine += "&lt;";
              } else if (line[i] === '{') {
                  processedLine += "\\{";
              } else if (line[i] === '}') {
                  processedLine += "\\}";
              } else {
                  processedLine += line[i];
              }
          } else {
              processedLine += line[i];
          }
          i += 1;
      }

      if (inlineCount % 2 !== 0) {
          outputLines.push(processedLine + "$");
      } else {
          outputLines.push(processedLine);
      }
  }

  if (inBlock) {
      outputLines.push("$$");
  }

  const balanced = outputLines.join("\n");
  const promoted = stripInlineMathPaddingAndPromoteMatrices(balanced);
  return isolateDisplayMath(promoted);
}

const MATRIX_ENVS = [
  "pmatrix", "bmatrix", "vmatrix", "Vmatrix", "Bmatrix",
  "matrix", "smallmatrix", "array", "cases", "aligned",
  "align", "align*", "gather", "gather*", "gathered",
  "tabular", "tabular*",
];

function isMatrixBeginEnv(s: string): boolean {
  return MATRIX_ENVS.some((env) => s.includes(`\\begin{${env}}`));
}

function isEscapedDollar(s: string, idx: number): boolean {
  let cnt = 0;
  let j = idx;
  while (j > 0 && s[j - 1] === '\\') {
    cnt++;
    j--;
  }
  return cnt % 2 !== 0;
}

export function stripInlineMathPaddingAndPromoteMatrices(text: string): string {
  if (!text) return text;
  let out = "";
  let i = 0;
  const n = text.length;

  while (i < n) {
    if (text[i] === '$' && i + 1 < n && text[i + 1] === '$' && !isEscapedDollar(text, i)) {
      const start = i + 2;
      let j = start;
      while (j < n) {
        if (text[j] === '$' && j + 1 < n && text[j + 1] === '$' && !isEscapedDollar(text, j)) {
          break;
        }
        j++;
      }
      if (j < n) {
        out += "$$" + text.slice(start, j) + "$$";
        i = j + 2;
        continue;
      } else {
        out += "$$";
        i += 2;
        continue;
      }
    }

    if (text[i] === '$' && !isEscapedDollar(text, i)) {
      const start = i + 1;
      let j = start;
      let foundClose = false;
      while (j < n) {
        if (text[j] === '$' && !isEscapedDollar(text, j) && (j + 1 >= n || text[j + 1] !== '$')) {
          foundClose = true;
          break;
        }
        j++;
      }
      if (foundClose) {
        const inner = text.slice(start, j);
        const trimmed = inner.trim();
        if (isMatrixBeginEnv(inner)) {
          out += `\n\n$$\n${trimmed}\n$$\n\n`;
        } else if (trimmed.length > 0) {
          out += `$${trimmed}$`;
        } else {
          out += `$${inner}$`;
        }
        i = j + 1;
        continue;
      }
    }

    out += text[i];
    i++;
  }

  return out;
}

export function isolateDisplayMath(text: string): string {
  if (!text) return text;
  let out = "";
  let i = 0;
  const n = text.length;

  while (i < n) {
    if (text[i] === '$' && i + 1 < n && text[i + 1] === '$' && !isEscapedDollar(text, i)) {
      const start = i + 2;
      let j = start;
      while (j < n) {
        if (text[j] === '$' && j + 1 < n && text[j + 1] === '$' && !isEscapedDollar(text, j)) {
          break;
        }
        j++;
      }
      if (j < n) {
        const inner = text.slice(start, j).trim();
        out += `\n\n$$\n${inner}\n$$\n\n`;
        i = j + 2;
        continue;
      } else {
        out += "$$";
        i += 2;
        continue;
      }
    }

    out += text[i];
    i++;
  }

  return out.replace(/\n{3,}/g, "\n\n").trim();
}
