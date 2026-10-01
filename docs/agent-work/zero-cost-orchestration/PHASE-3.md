# Phase 3 implementation brief

Prerequisite: Astra accepts phase 2. Use the same single Flash writer and ownership boundaries in PLAN.md. Preserve the accepted snapshots and pre-existing work.

## Objective and interfaces

Generalize local transcription across boards and subjects, with reliable MCQ, nuclide, table and contextual debris handling. Resolve observed symbol-font and margin-layout failures without guessing scientific content. Keep the existing BuiltQuestion/ImportReport seams and strict-vs-recovered distinction; preserve the phase-1 prohibition on cloud calls.

Text and layout belong to the local extraction layer. Where flattened text loses column or symbol ordering, use local PDF glyph/position/font evidence through existing PDF modules; any new evidence type should stay internal and optional for synthetic/text-only inputs. Normalize before strict validation, preserve source provenance for review, and keep unresolved content flagged. Do not substitute arbitrary glyphs, drop unknown symbols, invent matrix entries, infer answers, or hardcode fixes keyed to paper name/question number. Extra source-evidence helpers in `pdf_render.rs` or related ingestion modules are in scope; provider and billing architecture are not.

## Required behavior

1. Recognize coherent A-D (optionally E) MCQ runs with plain letters, including AQA/Edexcel two-column options. Emit exactly one `- [MCQ:X]` per option in source order with intact multiline bodies and associated figures. Preserve table headers and prevent ordinary prose/variables A-D from becoming options.
2. Reconstruct stacked/glued nuclides from local evidence: `10 D 2`, `238 92 U`, superscript/subscript runs, and decay expressions. Preserve atomic/mass ordering and avoid converting unrelated triples, dates, matrix entries, and code. Use targeted physics 2021 Q27 plus source-derived negative cases.
3. Produce valid Markdown pipe tables from actual tabular data. Headers, unit columns, rows and values must survive. Table separators remain outside math; math is confined to cells. Address physics 2021 Q5/Q6/Q25 examples from phase-2-report.md.
4. Recover source-supported symbol-font math, especially Edexcel matrix brackets (U+F8EB/F8EC/F8ED/F8F6/F8F7/F8F8) in core pure 2021 and madas Q8/Q9. Known bracket glyphs alone do not establish matrix shape: preserve row/column order using available source evidence; otherwise retain a local review flag.
5. Associate detached margin marks with their own questions using source layout. Resolve the observed madas `(5)`/`(6)` and figure-label contamination; preserve source allocations and keep ambiguous cases recovered. Never clear the phase-2 ambiguity guard without evidence.
6. Remove margin barcodes (`*05*`), answer-area labels (`left right`, `= J 9`) and orphan unit debris in their actual furniture/answer context. Retain legitimate variables, units, code, equations, table cells and question prose. Check cleanup idempotence.
7. Keep Rust output and frontend preprocessing consistent. Render generated cards through the actual preprocessing + Markdown + KaTeX stack. Fix parser defects, not merely the verifier; keep physics24 verifier's existing scientific/structural assertions intact.

## Calibration and evidence

Use named files in the specification as the corpus manifest. Inventory absent files explicitly. Inspect representative source content from AQA, Edexcel, CIE and the available subject families; tests must include CS code and maths/matrices, not only physics. Use the offline refusing client for every import. Calibrate the rules with targeted imports and relevant Rust/Node tests; reserve final full suite and final corpus certification for phase 4.

Completion means all behaviors above have positive/negative regression coverage and the known phase-2 examples are repaired or documented as concrete source-evidence blockers. Target >=95% strict local extraction across the specified historical corpus, with zero cloud requests, no lost questions, honest recoveries and no renderer crashes. Do not relabel uncertain results to meet the percentage. Report question numbers/marks/content evidence for resolved cases, not only aggregate counts.

Write `phase-3-report.md` with exact changed paths, verification commands/exits, representative render results, per-paper calibration evidence and remaining failures. Root reviews before phase 4. If progress is blocked by unavailable local PDF evidence, report the precise missing capability and a resumable checkpoint rather than enabling cloud.
