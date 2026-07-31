import { clsx, type ClassValue } from "clsx"
import { twMerge } from "tailwind-merge"

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

export function sanitizeMarkdownMath(text: string): string {
  if (!text) return text;
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

  return normalizeInlineMathSpacing(outputLines.join("\n").replace(/\n{3,}/g, "\n\n"));
}

function normalizeInlineMathSpacing(text: string): string {
  const token = /\$\$[\s\S]*?\$\$|\$[^$\n]+\$/g;
  return text.replace(token, (match, offset: number, source: string) => {
    if (match.startsWith("$$")) return match;
    const before = source.slice(0, offset).slice(-1);
    const after = source.slice(offset + match.length, offset + match.length + 1);
    const leading = /[\p{L}\p{N}]/u.test(before) ? " " : "";
    const trailing = /[\p{L}\p{N}]/u.test(after) ? " " : "";
    return `${leading}${match}${trailing}`;
  });
}
