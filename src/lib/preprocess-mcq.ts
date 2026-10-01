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

const MCQ_OPTION_LINE_RE = /^[ \t]*-[ \t]+(?:\[MCQ:[A-E]\]|\*\*(?:\([A-E]\)|[A-E][).:])\*\*)/m;
const IMAGE_LINE_RE = /^[ \t]*!\[[^\]]*\]\([^)]+\)[ \t]*$/gm;

/**
 * Checks if a string or link target is an image URL/file
 */
function isImageTarget(url: string): boolean {
  const trimmed = url.trim();
  return (
    /\.(?:png|jpe?g|webp|svg|gif|bmp|tiff?)(?:\?.*)?$/i.test(trimmed) ||
    trimmed.startsWith('data:image/') ||
    trimmed.startsWith('appdata://') ||
    trimmed.startsWith('asset://') ||
    trimmed.startsWith('/') ||
    /^[a-zA-Z]:[\\/]/.test(trimmed)
  );
}

/**
 * Enforce the structural invariant: a referenced diagram/image ALWAYS renders
 * ABOVE the MCQ options grid, never below it. Mirrors the backend
 * `ensure_diagrams_above_mcq` — this is the rendering-side safety net for
 * legacy rows saved before the backend reorder existed.
 */
export function promoteDiagramsAboveMcq(markdown: string): string {
  if (!markdown) return markdown;
  const mcqMatch = MCQ_OPTION_LINE_RE.exec(markdown);
  if (!mcqMatch) return markdown;
  const mcqIndex = mcqMatch.index;

  const moved: string[] = [];
  const ranges: Array<{ start: number; end: number }> = [];
  IMAGE_LINE_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = IMAGE_LINE_RE.exec(markdown)) !== null) {
    if (m.index <= mcqIndex) continue;
    // An image directly below an MCQ option line belongs to THAT option
    // (image-based option payload) — look back across any blank lines.
    const beforeText = markdown.slice(0, m.index);
    const beforeLines = beforeText.split('\n');
    let isOptionImage = false;
    for (let i = beforeLines.length - 1; i >= 0; i--) {
      const l = beforeLines[i].trim();
      if (l === '') continue;
      if (
        /^-\s+\[MCQ:[A-E]\]\s*$/.test(l) ||
        /^(?:[-*+][ \t]+)?(?:\*\*)?[A-E][.:)]?\s*$/.test(l)
      ) {
        isOptionImage = true;
      }
      break;
    }
    if (isOptionImage || /^\s*!\[(?:Option|Choice)\s+[A-E]\]/i.test(m[0])) continue;

    moved.push(m[0].trim());
    ranges.push({ start: m.index, end: m.index + m[0].length });
  }
  if (moved.length === 0) return markdown;

  // Remove the offending image lines (swallow one adjacent newline so no
  // blank husk is left behind).
  let cleaned = '';
  let last = 0;
  for (const r of ranges) {
    let start = r.start;
    let end = r.end;
    if (markdown[end] === '\n') {
      end += 1;
    } else if (start > 0 && markdown[start - 1] === '\n') {
      start -= 1;
    }
    cleaned += markdown.slice(last, start);
    last = end;
  }
  cleaned += markdown.slice(last);

  // Re-locate the MCQ block in the cleaned string and insert the images
  // directly above it.
  const reMatch = MCQ_OPTION_LINE_RE.exec(cleaned);
  const insertAt = reMatch ? reMatch.index : 0;
  const block = `\n\n${moved.join('\n')}\n\n`;
  return (
    cleaned.slice(0, insertAt).replace(/\s+$/, '') +
    block +
    cleaned.slice(insertAt).replace(/^\n+/, '')
  );
}

/**
 * Detects questions ending with 3 to 5 standalone diagrams (representing MCQ visual options)
 * and automatically structures them into standardized MCQ options, respecting any explicit
 * letter labels (e.g. "B", "Option B", alt="Option B", or filename "_b.png").
 */
export function convertStandaloneImagesToMcq(text: string): string {
  if (!text) return '';
  // If already has explicit MCQ tags, leave untouched
  if (text.includes('[MCQ:')) return text;

  const lines = text.split('\n');
  const LETTERS = ['A', 'B', 'C', 'D', 'E'];

  // Scan trailing lines for standalone images and optional preceding letter labels
  const detectedOptions: Array<{
    letter: string | null;
    imageLine: string;
    lineIndices: number[];
  }> = [];

  let idx = lines.length - 1;
  while (idx >= 0) {
    const line = lines[idx].trim();
    if (line === '') {
      idx--;
      continue;
    }

    const imgMatch = line.match(/^!\[([^\]]*)\]\(([^)]+)\)$/);
    if (!imgMatch) {
      break;
    }

    const altText = imgMatch[1];
    const url = imgMatch[2];
    const lineIndices = [idx];

    // Check if the preceding line is an explicit letter label (e.g. "A", "Option A", "**A**", "(A)")
    let explicitLetter: string | null = null;
    let prevIdx = idx - 1;
    while (prevIdx >= 0 && lines[prevIdx].trim() === '') {
      prevIdx--;
    }

    if (prevIdx >= 0) {
      const prevLine = lines[prevIdx].trim();
      const labelMatch = prevLine.match(/^(?:Option\s+|Choice\s+)?(?:\*\*|\[|\()?\s*([A-E])\s*(?:\*\*|\]|\)|\.|:)?$/i);
      if (labelMatch) {
        explicitLetter = labelMatch[1].toUpperCase();
        lineIndices.push(prevIdx);
        idx = prevIdx; // consume label line
      }
    }

    // Check alt text if no preceding label
    if (!explicitLetter && altText) {
      const altLetterMatch = altText.match(/\b(?:Option|Choice|Diagram)?\s*([A-E])\b/i);
      if (altLetterMatch) {
        explicitLetter = altLetterMatch[1].toUpperCase();
      }
    }

    // Check URL / filename if still no letter
    if (!explicitLetter && url) {
      const urlMatch = url.match(/[_\-/]([a-eA-E])\.(?:png|jpe?g|webp|svg|gif)\b/i);
      if (urlMatch) {
        explicitLetter = urlMatch[1].toUpperCase();
      }
    }

    detectedOptions.unshift({
      letter: explicitLetter,
      imageLine: line,
      lineIndices,
    });

    idx--;
  }

  // If there are 3 to 5 trailing standalone images, convert them to MCQ options
  if (detectedOptions.length >= 3 && detectedOptions.length <= 5) {
    // Find the earliest line index consumed
    const allIndices = detectedOptions.flatMap(o => o.lineIndices);
    const minIndex = Math.min(...allIndices);
    const stem = lines.slice(0, minIndex).join('\n').trimEnd();

    // Assign letters: if any option didn't have an explicit letter, fill from unused LETTERS
    const usedLetters = new Set(detectedOptions.map(o => o.letter).filter((l): l is string => l !== null));
    const availableLetters = LETTERS.filter(l => !usedLetters.has(l));

    const resolved = detectedOptions.map(opt => {
      let letter = opt.letter;
      if (!letter) {
        letter = availableLetters.shift() || 'A';
      }
      const match = opt.imageLine.match(/^!\[([^\]]*)\]\(([^)]+)\)$/);
      if (match) {
        const url = match[2];
        return {
          letter,
          formatted: `- [MCQ:${letter}] ![Option ${letter}](${url})`
        };
      }
      return {
        letter,
        formatted: `- [MCQ:${letter}] ${opt.imageLine}`
      };
    });

    // Sort alphabetically by letter so A is first, B is second, C is third, D is fourth
    resolved.sort((a, b) => a.letter.localeCompare(b.letter));

    return `${stem}\n\n${resolved.map(r => r.formatted).join('\n')}\n`;
  }

  return text;
}

/**
 * Formats bare LaTeX math or units inside an MCQ option body into clean $ ... $ math
 */
export function formatMcqOptionBody(rawBody: string): string {
  let body = rawBody.trim();
  body = body.replace(/((?:\*\*|\*|\[)?\s*\[?\s*\d+\s*marks?\s*\]?\s*(?:\*\*|\*|\])?)\s*$/i, '').trim();

  if (body.includes('$')) {
    return body.replace(/\${3,}/g, '$');
  }
  // Markdown payloads and prose are never standalone equations.
  if (body.includes('](') || body.split(/\s+/).length > 3) return body;

  const hasLatex = /(?:\\(?:times|frac|dfrac|cfrac|sqrt|pm|mp|approx|leq|geq|cdot|text|mu|pi|alpha|beta|gamma|delta|Delta|Omega|theta|lambda|sigma|varepsilon|mathrm|mathbf|vmatrix|pmatrix)|10\^\{?[-−–\d]+\}?|\^\{?[-−–\d]+\}?|_[0-9a-zA-Z]+|\b\d+(?:\.\d+)?\s*[×x]\s*10)/i.test(body);

  if (hasLatex) {
    const unitMatch = body.match(/^(.*?)[ \t]+([A-Za-z]+|\\[a-zA-Z]+(?:\{[^\}]*\})?)$/);
    if (unitMatch && !body.endsWith('}')) {
      const mathPart = unitMatch[1].trim();
      const unitPart = unitMatch[2].trim();
      if (unitPart.startsWith('\\')) {
        body = `$${mathPart}\\ ${unitPart}$`;
      } else {
        body = `$${mathPart}\\text{ ${unitPart}}$`;
      }
    } else {
      body = `$${body}$`;
    }
  }

  return body;
}

/**
 * Fuses multi-line MCQ options where unit lines or continuation phrases
 * have been broken across newlines beneath option letters:
 * e.g.:
 * A 4.0\times10^{7}
 * W
 * B 1.0\times10^{8}
 * W
 * C 6.0\times10^{8}
 * W
 * D 3.6\times10^{10}\text{W} **[1 mark]**
 */
export function fuseSplitMcqLines(text: string): string {
  if (!text) return '';
  text = text.replace(/\r/g, '');
  // NOTE: a generic "number line then short token line" rule used to turn any
  // `9\nF` into `\frac{9}{F}`. That is an unsafe guess (product? value+unit?
  // flattened fraction?), so it is intentionally gone. Reconstructing a real
  // stacked fraction requires fraction-bar glyph evidence from the backend; the
  // backend leaves such runs un-tagged and flags them as local recoveries.
  if (text.includes('[MCQ:')) return text;

  const lines = text.split('\n');
  let mcqStart = -1;
  // Search from the end backwards to find the start of the valid contiguous MCQ options block
  for (let i = lines.length - 1; i >= 0; i--) {
    const trimmed = lines[i].trim();
    const m = trimmed.match(/^(?:\*\*|\[)?\s*A(?:[\s.:)\]]+|$)(.*)$/i);
    if (m) {
      if (SUBQUESTION_VERB_REGEX.test(m[1].trim())) continue;
      const remaining = lines.slice(i + 1);
      let foundB = false;
      let foundC = false;
      for (const rl of remaining) {
        const rt = rl.trim();
        if (/^(?:\*\*|\[)?\s*B(?:[\s.:)\]]+|$)/.test(rt)) foundB = true;
        if (foundB && /^(?:\*\*|\[)?\s*C(?:[\s.:)\]]+|$)/.test(rt)) foundC = true;
      }
      if (foundB && foundC) {
        mcqStart = i;
        break;
      }
    }
  }

  if (mcqStart === -1) return text;

  const stemLines = lines.slice(0, mcqStart);
  const optLines = lines.slice(mcqStart);

  let currentLetter: string | null = null;
  let currentContent: string[] = [];
  const parsedOpts: Array<{ letter: string; body: string }> = [];

  for (const l of optLines) {
    const trimmed = l.trim();
    if (trimmed === '') continue;
    if (/^!\[/.test(trimmed) && currentContent.length > 0 && !/^!\[(?:Option|Choice)\s+[A-E]\]/i.test(trimmed)) {
      stemLines.push('', l, '');
      continue;
    }
    const optM = trimmed.match(/^(?:\*\*|\[)?\s*([A-E])(?:[\s.:)\]]+|$)(.*)$/);
    if (optM && ['A', 'B', 'C', 'D', 'E'].includes(optM[1].toUpperCase())) {
      if (currentLetter) {
        parsedOpts.push({ letter: currentLetter, body: currentContent.join(' ').trim() });
      }
      currentLetter = optM[1].toUpperCase();
      currentContent = optM[2].trim() ? [optM[2].trim()] : [];
    } else {
      if (currentLetter) {
        currentContent.push(trimmed);
      } else {
        stemLines.push(l);
      }
    }
  }
  if (currentLetter) {
    parsedOpts.push({ letter: currentLetter, body: currentContent.join(' ').trim() });
  }

  if (parsedOpts.length < 3) return text;

  // Check if any option contains a subquestion command verb
  if (parsedOpts.some(o => SUBQUESTION_VERB_REGEX.test(o.body))) {
    return text;
  }

  // Strip mark allocation from last option if present
  let trailingMark: string | null = null;
  const lastOpt = parsedOpts[parsedOpts.length - 1];
  const markMatch = lastOpt.body.match(/((?:\*\*|\*|\[)?\s*\[?\s*\d+\s*marks?\s*\]?\s*(?:\*\*|\*|\])?)\s*$/i);
  if (markMatch) {
    trailingMark = markMatch[1].trim();
    lastOpt.body = lastOpt.body.slice(0, markMatch.index).trim();
  }

  let stem = stemLines.join('\n').trim();
  // Strip leading stray subquestion letters `(a)`, `(a) `, `(b)` from MCQ stem
  stem = stem.replace(/^\s*\([a-z0-9]+\)\s*/i, '');
  // Strip stray margin `box ` at start of stem
  stem = stem.replace(/^(?:box[ \t]+(?:0\s*)?\d+(?:[ \t]*\d+)?(?:[ \t]*\.[ \t]*\d+)?|\([a-z]\)[ \t]*box[ \t]*|box[ \t]+)/i, '');

  if (trailingMark && !/\[\s*\d+\s*marks?\s*\]/i.test(stem)) {
    const numMatch = trailingMark.match(/\d+/);
    if (numMatch) {
      const n = numMatch[0];
      stem += ` **[${n} mark${parseInt(n, 10) > 1 ? 's' : ''}]**`;
    }
  }

  const formatted = parsedOpts.map(opt => {
    const body = formatMcqOptionBody(opt.body);
    return `- [MCQ:${opt.letter}] ${body}`;
  });

  return `${stem}\n\n${formatted.join('\n')}\n`;
}

export function normalizeMCQOptions(markdown: string): string {
  if (!markdown) return '';
  let text = markdown;

  // ── Step -2: Fuse multi-line split MCQ options with continuation lines ──
  text = fuseSplitMcqLines(text);

  // ── Step -1: Convert standalone visual options (3-5 trailing images) into MCQ options ──
  text = convertStandaloneImagesToMcq(text);

  // ── Step 0: Convert image links inside MCQ options to markdown images ──
  // e.g. "- [MCQ:A] [Option A](img.png)" -> "- [MCQ:A] ![Option A](img.png)"
  // e.g. "A) [Option A](img.png)" -> "A) ![Option A](img.png)"
  text = text.replace(
    /((?:^|\n)[ \t]*(?:[-*+][ \t]+)?(?:\[MCQ:[A-E]\]|\(?\b[A-E]\)?[.:]?)[ \t]+)(?:!\[([^\]]*)\]|\[([^\]]*)\])\(([^)]+)\)/gi,
    (_match, prefix, alt1, alt2, url) => {
      const alt = alt1 ?? alt2 ?? '';
      if (isImageTarget(url) || alt.toLowerCase().includes('option') || alt.toLowerCase().includes('diagram') || alt.toLowerCase().includes('figure')) {
        return `${prefix}![${alt}](${url})`;
      }
      return _match;
    }
  );

  // ── Step 0b: Join stranded image lines onto their parent MCQ option ──
  // e.g. "- [MCQ:A]\n![Option A](...)" or "- [MCQ:A]\n\n![Option A](...)" -> "- [MCQ:A] ![Option A](...)"
  text = text.replace(
    /(^[ \t]*-[ \t]+\[MCQ:[A-E]\])[ \t]*\n+[ \t]*(!\[[^\]]*\]\([^)]+\))/gm,
    '$1 $2'
  );

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

  // ── Pattern 2: Multi-line MCQs (Strictly UPPERCASE A, B, C, D each on its own line, permitting blank lines) ──
  // Do NOT match lowercase (a), (b), (c)
  const multiLineMcqBlockRegex = /(?:^|\n)(?:[ \t]*(?:\*\*|\[)?\s*A[\s.:\]]+([^\n]+))\n+[ \t]*(?:\*\*|\[)?\s*B[\s.:\]]+([^\n]+)\n+[ \t]*(?:\*\*|\[)?\s*C[\s.:\]]+([^\n]+)(?:\n+[ \t]*(?:\*\*|\[)?\s*D[\s.:\]]+([^\n]+))?/g;

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

  // ── Step 3c: Wrap bare LaTeX math in existing - [MCQ:X] options & strip misplaced marks ──
  text = text.replace(/^[ \t]*-[ \t]+\[MCQ:([A-E])\][ \t]+(.+)$/gm, (_match, letter, rawBody) => {
    let body = formatMcqOptionBody(rawBody);
    // Delimiters are local to an option, including malformed display markers.
    body = body.replace(/(?<!\\)\${2,}/g, '$');
    if ((body.match(/(?<!\\)\$/g) || []).length % 2) body += '$';
    return `- [MCQ:${letter}] ${body}`;
  });

  // ── Step 3d: Strip leading stray subquestion letters `(a)`, `(b)` from MCQ question stem ──
  if (text.includes('- [MCQ:')) {
    text = text.replace(/^\s*\([a-z0-9]+\)\s*/i, '');
  }

  // ── Pattern 3b: Tighten loose MCQ lists by removing blank lines between - [MCQ:X] items ──
  text = text.replace(/(^[ \t]*-[ \t]+\[MCQ:[A-E]\][^\n]+)\n+(?=[ \t]*-[ \t]+\[MCQ:[A-E]\])/gm, '$1\n');

  // ── Pattern 4: Drop option bullets left empty (e.g. by placeholder-token
  // stripping) so the grid never renders blank cards.
  text = text.replace(/^[ \t]*[-*+][ \t]+\[MCQ:[A-E]\][ \t]*$/gim, '');

  // ── Pattern 5: Referenced diagrams always render ABOVE the options grid.
  text = promoteDiagramsAboveMcq(text);

  return text;
}
