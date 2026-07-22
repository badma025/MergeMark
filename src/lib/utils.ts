import { clsx, type ClassValue } from "clsx"
import { twMerge } from "tailwind-merge"

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

export function sanitizeMarkdownMath(text: string): string {
  if (!text) return text;
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

  return outputLines.join("\n");
}
