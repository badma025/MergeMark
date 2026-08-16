/**
 * MCQ Option Normalizer
 *
 * Detects unstructured multiple-choice options (uppercase A, B, C, D) produced by LLMs
 * and converts them into structured Markdown list items tagged with `[MCQ:X]`.
 *
 * CRITICAL GUARDRAIL:
 * Exam sub-questions (e.g. "(a) Find the value of...", "(b) Show that...") MUST NEVER
 * be treated as multiple-choice options. Only strict uppercase choice patterns (A, B, C, D)
 * that represent answer alternatives are converted.
 */

const SUBQUESTION_VERB_REGEX = /^(?:find|show|prove|calculate|determine|evaluate|state|hence|given|verify|describe|explain|sketch|differentiate|integrate|solve|write|express|suggest|estimate|deduce)\b/i;

export function normalizeMCQOptions(markdown: string): string {
  if (!markdown) return '';
  let text = markdown;

  // ── Pattern 1: Single-line crammed MCQs (e.g. "A: 10 m   B: 20 m   C: 30 m   D: 40 m") ──
  // STRICT UPPERCASE only
  const singleLineMcqRegex = /(?:^|\n)(?:\*\*|\[)?\s*([A-D])[\s.:)\]]+(.+?)\s+(?:\*\*|\[)?\s*B[\s.:)\]]+(.+?)\s+(?:\*\*|\[)?\s*C[\s.:)\]]+(.+?)\s+(?:\*\*|\[)?\s*D[\s.:)\]]+(.+?)(?=\n|$)/;

  text = text.replace(singleLineMcqRegex, (_match, _lead, a, b, c, d) => {
    // If any option starts with an exam prompt command verb, this is subquestions not MCQ
    if (SUBQUESTION_VERB_REGEX.test(a.trim()) || SUBQUESTION_VERB_REGEX.test(b.trim())) {
      return _match;
    }
    return `\n\n- [MCQ:A] ${a.trim()}\n- [MCQ:B] ${b.trim()}\n- [MCQ:C] ${c.trim()}\n- [MCQ:D] ${d.trim()}\n\n`;
  });

  // ── Pattern 2: Multi-line MCQs (Strictly UPPERCASE A, B, C, D each on its own line) ──
  // Do NOT match lowercase (a), (b), (c)
  const multiLineMcqBlockRegex = /(?:^|\n)(?:[ \t]*(?:\*\*|\[)?\s*A[\s.:\]]+([^\n]+)\n)[ \t]*(?:\*\*|\[)?\s*B[\s.:\]]+([^\n]+)\n[ \t]*(?:\*\*|\[)?\s*C[\s.:\]]+([^\n]+)(?:\n[ \t]*(?:\*\*|\[)?\s*D[\s.:\]]+([^\n]+))?/g;

  text = text.replace(multiLineMcqBlockRegex, (match, a, b, c, d) => {
    // Protect sub-questions from being mangled into MCQs
    if (
      SUBQUESTION_VERB_REGEX.test(a.trim()) ||
      SUBQUESTION_VERB_REGEX.test(b.trim()) ||
      SUBQUESTION_VERB_REGEX.test(c.trim()) ||
      (d && SUBQUESTION_VERB_REGEX.test(d.trim()))
    ) {
      return match;
    }
    let result = `\n\n- [MCQ:A] ${a.trim()}\n- [MCQ:B] ${b.trim()}\n- [MCQ:C] ${c.trim()}`;
    if (d) {
      result += `\n- [MCQ:D] ${d.trim()}`;
    }
    return result + '\n\n';
  });

  // ── Pattern 3: Standard markdown list items that start explicitly with UPPERCASE A, B, C, D ──
  text = text.replace(/^[ \t]*[-*+][ \t]+(?:\*\*|\[)?\s*([A-D])[\s.:\]]+(.+)$/gm, (match, letter, content) => {
    if (SUBQUESTION_VERB_REGEX.test(content.trim())) {
      return match;
    }
    return `- [MCQ:${letter}] ${content.trim()}`;
  });

  return text;
}
