/**
 * Phase-3 regression: code preservation across the actual frontend path.
 *
 * Bundles the real `src/lib/preprocess-math.ts` (the assembler + frontend
 * preprocessing stack) and asserts that fenced code blocks and inline code
 * spans are never rewritten by the delimiter machinery:
 *   - a literal `$$$` inside a fence stays `$$$`
 *   - a double-backtick span containing a single `$` (and a backtick) is opaque
 *   - a four-backtick fence is not closed by an inner three-backtick line
 *   - a tilde fence is not closed by a backtick line (and vice versa)
 *   - escaped dollars stay literal
 *   - a closing fence may not carry a trailing info string
 *   - real multi-line display math still receives an explicit `\\` separator
 *
 * Usage: node scripts/verify-phase3-code-opacity.mjs
 * Exit code is non-zero when any assertion fails.
 */
import { build } from 'esbuild';

const bundleModule = async (entry) => {
  const built = await build({
    entryPoints: [entry],
    bundle: true,
    write: false,
    format: 'esm',
    platform: 'node',
    absWorkingDir: process.cwd(),
  });
  return import(`data:text/javascript;base64,${Buffer.from(built.outputFiles[0].text).toString('base64')}`);
};
const mathMod = await bundleModule('src/lib/preprocess-math.ts');
const examMod = await bundleModule('src/lib/preprocess-exam-markdown.ts');
const { preprocessExamMarkdown } = mathMod;
const { ensureDisplayMathLineBreaks, healLatexDelimiters } = examMod;

const failures = [];
const check = (name, actual, expected) => {
  if (actual !== expected) failures.push({ name, expected, actual });
};

// 1. Literal `$$$` inside a fenced code block survives the whole stack.
const fencedLiteral = 'Intro\n```text\na $$$ b\n```\nDone.';
check('fenced_literal_dollars', preprocessExamMarkdown(fencedLiteral), fencedLiteral);

// 2. Double-backtick span containing a single `$` and a backtick is opaque.
const doubleTicks = 'Use ``$`` and ``a ` b`` in code; keep $x$ in math.';
check('double_backtick_span', preprocessExamMarkdown(doubleTicks), doubleTicks);

// 3. A four-backtick fence is NOT closed by an inner three-backtick line.
const fourTicks = '````\n```\n$$\n````';
check('four_backtick_fence', preprocessExamMarkdown(fourTicks), fourTicks);

// 4. Marker characters must match: a tilde fence is not closed by backticks.
const tilde = '~~~\n```\n$$\n~~~';
check('tilde_vs_backtick_fence', preprocessExamMarkdown(tilde), tilde);

// 5. Escaped dollars stay literal.
const escaped = 'Costs \\$5 for $n$ items.';
check('escaped_dollar', preprocessExamMarkdown(escaped), escaped);

// 6. A closing fence may not carry an info string: the block stays open.
const infoClose = '```text\na $$$ b\n```not-a-close\n$x$\n```';
check('fence_closer_requires_whitespace', preprocessExamMarkdown(infoClose), infoClose);

// 7. `ensureDisplayMathLineBreaks` treats a fenced `$$` block as opaque.
const fencedDisplay = 'before\n```\n$$\npacket\n```\nafter';
check('display_breaks_respect_fence', ensureDisplayMathLineBreaks(fencedDisplay), fencedDisplay);

// 8. Real multi-line display math still receives its explicit `\\` separator.
const displayOut = ensureDisplayMathLineBreaks('$$x = 1\ny = 2$$');
if (!displayOut.includes('\\\\ y = 2')) {
  failures.push({ name: 'display_break_added', expected: 'contains "\\\\ y = 2"', actual: displayOut });
}

// 9. `healLatexDelimiters` leaves a fenced `$$$` literal (idempotent stack).
check('heal_respects_fence', healLatexDelimiters('Intro\n```\n$$$\n```\nDone.'), 'Intro\n```\n$$$\n```\nDone.');

// 10. A literal empty-dollar pair inside code is not stripped as phantom math.
const emptyPair = 'prose $x = 1$\n```\n$$ $$\n```\nend';
check('code_empty_dollar_pair', preprocessExamMarkdown(emptyPair), emptyPair);

// 11. A placeholder-looking string inside code is not stripped.
const placeholder = 'text\n```\nlet t = "[DIAGRAM_PLACEHOLDER]";\n```\nend';
check('code_placeholder_preserved', preprocessExamMarkdown(placeholder), placeholder);

// 12. Mixed real math outside code + code with dollars keeps both.
const mixed = 'Value $y = 2$\n```js\nconst p = "$5";\n```\nEnd $z = 3$.';
check('mixed_math_and_code', preprocessExamMarkdown(mixed), mixed);

// 13. An existing U+0001 marker in content must not restore as a code span.
const markerCollision = 'literal \u00010\u0001 here\n```\n$code$\n```\nafter';
check('sentinel_collision_safe', preprocessExamMarkdown(markerCollision), markerCollision);

// 14. Idempotence: healing the healed content is a no-op.
const once = preprocessExamMarkdown(fencedLiteral);
check('idempotent', preprocessExamMarkdown(once), once);

const summary = { total: 14, passed: 14 - failures.length, failures };
console.log(JSON.stringify(summary, null, 2));
console.log(failures.length ? 'CODE OPACITY REGRESSION: FAIL' : 'CODE OPACITY REGRESSION: PASS');
if (failures.length) process.exitCode = 1;
