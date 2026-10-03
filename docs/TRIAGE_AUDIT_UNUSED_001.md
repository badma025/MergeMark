# Triage and audit of the unused papers (run unused-001)

168 question papers from `papers/unused/` (20 already in the repo, 148 downloaded) were imported with the offline pipeline, then 48 cards were audited by eye against the source page images. No pipeline code was changed. All imports: 0 cloud attempts, 0 tokens, $0, except one scanned paper whose 19 refused image requests were never sent.

Raw outputs (gitignored): `output/triage/run-unused-001/` (`summary.md`, `results.tsv`, `results.json`, `audit-pack/merged.json`).

## 1. Automatic triage (168 papers)

| PASS | REVIEW | FLAGGED | FAIL |
|---|---|---|---|
| 16 | 2 | 55 | 95 |

PASS = every source-independent check holds (printed paper total, per-question total, unique IDs, no U+FFFD, figures exist, KaTeX renders). FLAGGED = the pipeline marked cards for review itself.

Largest failure classes: unidentified glyphs (45 papers), card marks below the printed paper total (43), papers that import as 0 cards because they are scanned images with a watermark text layer (3), OCR and CIE mark conventions read as 0 marks, and OCR plain "(a), (b)" numbering collapsing a paper into one card.

## 2. Visual audit (48 cards, blinded, seeded sample)

Four independent agents compared each card to its source pages without seeing the automatic verdicts. Two claims were re-verified by hand against the source text.

| sample group | cards | CLEAN | MINOR | MAJOR |
|---|---|---|---|---|
| cards from papers the checks PASSED | 14 | 9 | 3 | 2 |
| unflagged cards inside failing papers | 10 | 4 | 4 | 2 |
| cards the pipeline flagged for review | 8 | 1 | 3 | 4 |
| cards with an unidentified glyph | 5 | 0 | 0 | 5 |
| cards with marks 0 or short | 6 | 0 | 0 | 6 |
| collapsed or ID-gap papers | 5 | 2 | 1 | 2 |
| **all** | **48** | **16** | **11** | **21** |

The sample is stratified toward suspected failures, so the overall MAJOR share (44%) is not a population rate. The one fair estimate is the first row: 2 of 14 cards from papers that passed every automatic check were MAJOR (a wide 95% interval: roughly 4% to 40%).

MAJOR defect families (cards, overlapping): marks wrong or missing 11, symbol or glyph 9, figures (bad crop, extra, missing) 9, extra or merged text 8, missing words or maths structure 7.

## 3. What the automatic checks cannot see

The pipeline's own flags and my checks caught 15 of the 21 MAJOR cards. Six had no signal at all:

- A source PDF whose text layer encodes "<" as the character "1" (OCR-MEI June 2025 Q6, "0° 1 x 1 360°"). Passes every check because marks and IDs are right. Verified against `pdftotext`.
- A missing word from the card text ("from the ." for "from the box", Edexcel Further June 2021) and a garbled summation.
- Question merged with the next (AQA Nov 2017 Q25 carries Q26).
- Figure cropped without its label, or the question's figure absent.
- Printed marks not carried onto the card (OCR GCSE).

## 4. Limits

- Auditors saw 1 to 3 source pages per card. Three cards were only partly verifiable and were marked low confidence.
- Page images were rendered with poppler, which substitutes fonts that a PDF does not embed. Symbol differences that could be substitution were recorded as unverifiable, not errors.
- Subject mix of the sample: Mathematics 25, Physics 16, Biology 4, Chemistry 2, Economics 1. Chemistry, Biology and Economics are under-sampled relative to how badly they triaged.
- One reviewer model with a fixed rubric; the verdicts are judgments, not ground truth.
