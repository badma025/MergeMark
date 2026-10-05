# Mark-scheme reader: census of every refusal (1.0 plan, W4 step 1)

The local mark-scheme reader (`mark_scheme_deterministic.rs`) accepts a scheme
only when no refusal fires anywhere in it, and it stopped at the first one. The
census of round 2 (cluster 5c) was therefore one reason per scheme, mostly from
page 1. This census reads on past every refusal and records them all, page by
page.

## Method

- `mark_scheme_deterministic::census` runs the reader's own code with a judge
  that notes each refusal and reads on, at the smallest unit the refusal
  concerns (a page, a table, a row). The strict path is the same code with a
  judge that stops at the first refusal and words it as before; every
  mark-scheme test passes unchanged, and
  `a_census_lists_every_refusal_the_reader_stops_at_the_first_of` pins the two
  against each other.
- Each scheme is read twice: as the reader reads it, and with rotated pages
  turned upright (a probe of what lies behind the rotation refusal; the turn
  was checked to read CIE 04's landscape pages left to right, header on top).
- `e2e_import --mark-scheme --census` writes `<name>_ms_census.json`;
  `node scripts/ms-census.mjs <dir>` gives the tables below. Run:
  `output/fix-log/ms-census-w4/`.
- Caveat: a census can only see behind a refusal it reads past. When a page's
  table is not recognised at all, nothing on that page is examined further.
  The last section estimates what lies behind that.

## Per scheme

| scheme | pages | rotated | pages refused | first barrier on most pages |
|---|---|---|---|---|
| 01 Edexcel 9MA0 P1 (Jun 22) | 33 | 0 | 31 | table pages: "no unambiguous column gutter" (14); notes and alternatives pages between tables are prose with no table (12); cover and instructions (3) |
| 02 Edexcel Further CP1 | 21 | 0 | 18 | as 01, plus unmapped glyphs (a maths font PDFium cannot map) on 15 pages |
| 03 AQA Physics P1 | 23 | 0 | 22 | tables **are** recognised (16 pages); there the formula, fraction-bar and spatial-maths rules fire on almost every page, with question keys misread from page numbers; the MCQ key table ("Question / Key / Answer") has no Marks column |
| 04 CIE 9709 P1 | 19 | 18 | 18 | every answer page is landscape (`/Rotate`); turned upright, its header ("Question / Answer / Marks / Guidance") is still refused: "Guidance" without "Additional" is read as a second Answer column |
| 05 AQA CS 7517/1 | 52 | 0 | 51 | header "Question / Marks" with the answer as prose spanning the row: no profile for it; 26 pages are program listings |
| 06 community PMT pack | 25 | 0 | 25 | no text layer at all (a scan) |
| 07 CIE 9709/11 (Jun 14) | 7 | 0 | 7 | no column headers at all: rows are "1 / a = 1, b = 2 / B1B1 / [2]" |

No scheme is accepted, as in round 2.

## Reasons, by pages they fire on

Rotated pages turned upright (as read, the 18 CIE pages are "rotated layout
unsupported" instead). "Sole" counts the pages where the reason is the only one.

| reason | schemes | pages | sole |
|---|---|---|---|
| unrecognized table or continuation | 7 | 131 | 125 |
| no unambiguous column gutter | 2 | 22 | 14 |
| unresolved in-cell rule (possible fraction bar) | 3 | 19 | 0 |
| unmapped glyph | 2 | 17 | 0 |
| question sequence moves backwards | 1 | 16 | 0 |
| text outside recognized tables | 1 | 16 | 0 |
| formula transcription requires source review | 3 | 15 | 0 |
| spatial math or overlapping baselines requires source review | 3 | 12 | 0 |
| invalid question key | 2 | 9 | 0 |
| glyph crosses a physical row separator | 1 | 5 | 0 |
| missing answer or marks | 1 | 1 | 0 |
| table contains unresolved graphical content | 1 | 1 | 0 |
| unreliable glyph geometry or encoding | 1 | 1 | 0 |

"Unrecognized table or continuation" is three different things: cover and
guidance pages the preamble rule misses (all six readable schemes), prose
continuation pages (Edexcel's "Notes" and "Alternative methods" pages after
each table), and tables in layouts the reader has no profile for (CIE's four
columns, CIE 2014's headerless rows, AQA CS's two columns, AQA's MCQ key).

## What lies behind the layout gates

The formula rule refuses any cell holding `= ^ _ ∫ √ ∑ ∏`. On the one scheme
whose tables are recognised (03), it fires on 12 of 16 table pages, and the
fraction-bar and spatial-maths rules on most of the same pages. For the others,
an estimate from each page's text layer (pages holding a question key and
marks, header and footer bands dropped; `pymupdf`, so approximate):

| scheme | marking pages | holding = ^ _ ∫ √ ∑ ∏ |
|---|---|---|
| 01 Edexcel P1 | 22 | 22 (100%) |
| 02 Edexcel CP1 | 13 | 13 (100%) |
| 03 AQA Physics | 16 | 12 (75%) |
| 04 CIE P1 | 14 | 5 (36%; its maths fonts map few of these characters, so likely an undercount) |
| 05 AQA CS | 21 | 7 (33%, mostly assignments in code) |
| 06 PMT pack | 0 | (no text) |
| 07 CIE 2014 | 5 | 4 (80%) |

## Reading

The mass is not the preamble and rotation gates alone. They are the first
layer: tuning them (the preamble predicate, the `Frame`-style turn, the header
vocabulary, gutter inference) would open the pages to the reader's next rules,
and on every maths scheme those are the formula, fraction-bar and spatial-maths
refusals, on most marking pages. Reading those pages means transcribing
maths: fractions, scripts, radicals and symbol fonts, which `layout.rs` does
for question papers and this reader does not. Two schemes need layouts the
reader has no profile for (05's two-column prose, 07's headerless rows), and
one cannot be read without OCR (06).

So by the plan's own test this is a missing capability, not a loose gate. The
options it names are rebuilding the reader on `layout.rs`'s `TableGrid` and
glyph stack, or cutting mark schemes from 1.0. Tuning alone would, on this
evidence, accept none of the seven.

## Traps to remember whichever way it goes

- The reader does no CropBox normalisation (`layout.rs` `Frame` does).
- `a_mark_scheme_the_local_reader_refuses_sends_nothing` asserts the 07
  scheme's refusal reason ("unrecognized table or continuation"); it pins the
  current behaviour, so any rebuild must change that assertion deliberately.
