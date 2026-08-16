import { clsx, type ClassValue } from "clsx"
import { twMerge } from "tailwind-merge"

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

// Re-export markdown and math preprocessing functions
export { preprocessMathString, preprocessExamMarkdown, sanitizeForLatex } from "./preprocess-math"
export { healLatexDelimiters } from "./preprocess-exam-markdown"
export { normalizeMCQOptions } from "./preprocess-mcq"