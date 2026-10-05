# Card-versus-source audit rubric

You compare one imported exam-question card (Markdown with LaTeX maths) against images of the source exam pages it came from. You judge only what you can see; you do not know how the card was produced.

For each item in your slice:

1. Read `cardFile` (the card). `cardMarksField` is the mark value stored for the card.
2. View every image in `pagePngs` (the source pages; the question is on one or more of them) and every image in `figurePngs` (figure crops embedded in the card).
3. Find the question on the pages (its number is `question`). Compare the card with the printed question, from its heading to the end of its last part.

Check:
- **Text**: every word, number and symbol of the question is on the card, nothing changed. Maths must say what the page says (signs, symbols, fractions, powers, subscripts, sums with limits, matrices).
- **Boundaries**: the card starts at this question and ends where it ends: no part of another question, no page furniture (headers, footers, "Turn over", "END OF QUESTION PAPER", "TOTAL FOR PAPER", copyright notices, answer lines/dots, "Do not write outside the box").
- **Marks**: the printed allocations ("[3]", "(3)", "[3 marks]") of every part appear on the card as **[N marks]** tags after the part they belong to, and `cardMarksField` equals the question's printed total.
- **Figures**: every diagram, graph or table picture the question prints is on the card, cropped whole with its labels; no unrelated image (logos, other questions' figures).
- **MCQ**: every option is present and answerable.

Pages were rendered with poppler, which substitutes fonts a PDF does not embed. If a symbol difference could be font substitution in the page image rather than a card error, record it as `unverifiable`, not as an error.

Severity:
- **MAJOR**: a reader would get the question wrong or could not answer it: missing/changed words, numbers or symbols that change meaning, garbled maths, wrong or missing marks, another question merged in, a needed figure missing or cut so labels are lost, unrelated content or furniture of substance attached.
- **MINOR**: cosmetic only, meaning intact: layout, emphasis, a stray "Your answer" label, a cosmetic table header.
- **CLEAN**: no issue.

Issue `type` is one of: wrong-symbol, wrong-number, missing-text, extra-text, math-structure, merged-questions, boundary-start, boundary-end, wrong-marks, missing-mark-tag, figure-missing, figure-bad-crop, figure-extra, mcq-options, table-broken, page-furniture, formatting-only, unverifiable.

Write your verdicts as a JSON array to the output path you are given, one object per item:

```json
{"id": "<item id>", "verdict": "CLEAN|MINOR|MAJOR", "issues": [{"type": "...", "severity": "MAJOR|MINOR", "detail": "what the page shows vs what the card has"}], "printedMarks": <number or null>, "confidence": "high|medium|low"}
```

`verdict` is the most severe issue's severity (CLEAN when there are none). Use `confidence: "low"` when the provided pages do not show the whole question.
