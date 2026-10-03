# Fixing unseen-paper imports: clusters 1 to 5, and round 2

Follow-up to [TRIAGE_AUDIT_UNUSED_001.md](TRIAGE_AUDIT_UNUSED_001.md). Every run here was local: zero network attempts, zero tokens, $0. Nothing was committed.

## How it was measured

- **Calibration** after each cluster: the 22 original papers stayed 22/22 fresh, 243/244 strict (historical) and 69/69 (test papers), zero attempts, in every run. The extra rows are papers moved from `papers/unused/` to `papers/used/` as fixtures (cohort `triage_fixtures`, registered in `scripts/lib/zero-cost-accounting.mjs` and `scripts/fixtures/source-expected-ids.json`).
- **Triage** of what is left in `papers/unused/`, compared with run unused-001 **on the same papers** (fixtures excluded from both sides).
- **Blinded visual audit** after each cluster: a fresh seeded sample (`build-audit-pack.mjs --seed 7`, 48 cards, same strata) judged by four agents that saw only the card, its pages and its figure crops, against a written rubric (`output/fix-log/audit-rubric.md`).

Caveats that matter for reading the numbers:
- Triage PASS needs every card of a paper to be clean, so a paper moves to PASS only when its last problem goes. Most fixes lift papers from FAIL to FLAGGED first.
- Cluster 2 made the checks *see* more errors: symbols that used to pass as wrong letters are now `�` and flagged. That lowers PASS and raises "unidentified glyph" while the cards get more honest. Every such change I checked was a wrong character before (examples below).
- The audit sample is redrawn each time and is stratified toward failures, so its overall MAJOR share moves with the sample. The one fair series is the "cards from papers the checks passed" row (14 cards). Auditors also disagreed on whether a correct marks field without a visible "[N marks]" tag is MAJOR; that alone is 3-7 MAJOR cards per audit.
- The page picker in `build-audit-pack.mjs` sometimes attaches the wrong page (7-11 low-confidence verdicts per audit).

## Results on the 152 papers in every run

| run | PASS | REVIEW | FLAGGED | FAIL | totals match | marks under | 0-mark cards | glyph papers | glyph findings | cards | flagged cards |
|---|---|---|---|---|---|---|---|---|---|---|---|
| unused-001 (before) | 15 | 2 | 53 | 82 | 95 | 37 | 45 | 39 | 106 | 2125 | 593 |
| after cluster 1 | 15 | 3 | 67 | 67 | 114 | 18 | 12 | 39 | 106 | 2125 | 534 |
| after cluster 2 | 13 | 3 | 55 | 81 | 112 | 20 | 12 | 56 | 125 | 2124 | 561 |
| after cluster 3 | 16 | 3 | 52 | 81 | 113 | 19 | 12 | 55 | 152 | 2125 | 564 |
| after cluster 4 | 16 | 3 | 53 | 80 | 112 | 19 | 12 | 54 | 137 | 2132 | 551 |
| after cluster 5 | 16 | 3 | 53 | 80 | 112 | 19 | 12 | 54 | 137 | 2132 | 551 |
| after cluster 5b (no AI requests) | 16 | 3 | 53 | 80 | 112 | 19 | 12 | 54 | 137 | 2132 | 551 |
| cluster 5b, on the 147 papers left in round 2 | 16 | 3 | 53 | 75 | 108 | 18 | 12 | 53 | 135 | 2057 | 540 |
| after cluster 6 (147 papers) | 16 | 3 | 54 | 74 | 108 | 18 | 12 | 53 | 135 | 2057 | 540 |
| cluster 6, on the 137 papers left after cluster 7 | 16 | 3 | 50 | 68 | 102 | 15 | 11 | 50 | 129 | 1936 | 482 |
| after cluster 7 (137 papers) | 18 | 3 | 48 | 68 | 102 | 15 | 11 | 50 | 129 | 1936 | 374 |
| cluster 7, on the 131 papers left after cluster 8 | 18 | 3 | 48 | 62 | 102 | 10 | 10 | 48 | 126 | 1819 | 357 |
| after cluster 8 (131 papers) | 24 | 5 | 41 | 61 | 106 | 6 | 10 | 48 | 126 | 1815 | 292 |
| cluster 8, on the 126 papers left after cluster 9 (run unused-seam: c8b's cards) | 23 | 5 | 42 | 56 | 101 | 6 | 10 | 44 | 113 | 1771 | 278 |
| after cluster 9 (126 papers) | 26 | 6 | 43 | 51 | 106 | 3 | 10 | 44 | 113 | 1771 | 268 |
| after cluster 10 (126 papers) | 31 | 7 | 44 | 44 | 106 | 3 | 10 | 37 | 87 | 1771 | 243 |
| after cluster 11 (126 papers; no card changed) | 31 | 7 | 44 | 44 | 106 | 3 | 10 | 37 | 87 | 1771 | 243 |
| round 2's start (cluster 5b) on the same 126 papers | 16 | 3 | 48 | 59 | 97 | 10 | 10 | 44 | 113 | 1775 | 434 |

**Round 2 in sum**, on the 126 papers still unused after it (cluster 5b → cluster 11):
- PASS 16 → 31 and FAIL 59 → 44;
- printed total matched on 97 → 106 papers; papers with marks under it 10 → 3;
- papers with an unidentified glyph 44 → 37;
- flagged cards 434 → 243, strict cards 1341 → 1528.

The audits are the check that strict cards are right. After cluster 10, a targeted audit found 6 wrong among the 40 cards that cluster made strict ("What is left").

Round 2 moves fixtures out of `papers/unused/`, so from cluster 6 on the rows are on the papers still there (147 after cluster 6, 137 after cluster 7, 131 after cluster 8, 126 after cluster 9), with the previous run recomputed on the same papers.

Visual audit (48 cards each):

| run | passed-paper cards MAJOR (of 14) | all MAJOR (of 48) | MAJOR wrong-symbol cards unflagged |
|---|---|---|---|
| unused-001 | 2 | 21 | several (e.g. OCR-MEI "<" as "1", passed every check) |
| cluster 1 | 1 | 18 | |
| cluster 2 | 1 | 19 | 1 of 7 |
| cluster 3 | 1 | 22 | |
| cluster 4 | 1 | 18 | |
| cluster 5 | n/a | 8 of 9 scanned-paper cards (all flagged; marks right on all 9) | 0 |
| cluster 5b | 1 | 17 (no card changed since c5: auditor variation; round 2 baseline) | |
| cluster 6 | 1 | 19 (the 16 cards shared with the c5b sample got identical verdicts; the one changed MAJOR card improved) | |
| cluster 7 | 1 | 19 (one MAJOR card changed: an answer line removed; its MAJOR issues are elsewhere) | |
| cluster 8 | 2 (both AQA Chemistry GCSE Jun 2024, passing for the first time; fixed with cluster 9) | 17 of 46 | |
| cluster 9 | 1 (Edexcel A Economics Jun 2024 Q1, a chart cropped without its axis labels; unchanged since c8b) | 17 of 46 (sample shares 3 cards with c8b's) | |
| cluster 10 | 0 | 13 of 46; of the 40 cards it made strict, 6 MAJOR, none for a symbol it reads | |

## Cluster 1: OCR, CIE and WJEC mark conventions

**Root cause.** OCR and CIE print a part's allocation at the end of its dotted answer line ("(a) ………… [1]", "………… cm [1]", "\$ ………… [2]"). `is_answer_line_text` treated the whole line as answer space, so the allocation went to furniture with the dots and the card got 0 or short marks. WJEC: bracket "tallness" was measured against the page-wide median glyph size, which the 8pt "Examiner only" column pulled down, so a 12pt "[3]" beside a formula became `\left[3\right]` and "only" attached as a superscript.

**Fix.** `answer_space_mark` (layout.rs) recognises answer space that closes with an allocation: optional part label, dots, and only units and prompt tokens (a variable, "=", "or", "cm", "m/s", "days", "AB ="); a number or any other word, or a radical or fraction, makes the line question text. The allocation stays a body line as printed. Brackets are judged against their own line's type; "Examiner only" is margin furniture. Follow-ups: old Edexcel "(7)(Total 7 marks)" is the question footer (only on the question's last line, so a merged question's total stays visible); a single-part question's footer allocation is shown on the card as its tag.

**Tests.** `allocations_printed_at_the_end_of_answer_lines_stay_with_the_question` (OCR J560 Jun 24, OCR A Physics Jun 17, CIE 0580 Nov 22, WJEC Unit 1 Jun 25), `answer_space_gives_its_allocation_only_beside_units_and_prompts`, `a_question_total_printed_after_its_last_allocation_is_the_footer` (Edexcel FP2 Jun 05), `single_part_question_with_footer_only_allocation_is_strict` (tag).

**Numbers.** Papers whose card marks fall short of the printed total: 37 → 18. Cards with 0 marks: 45 → 12. Printed total matched: 95 → 114 papers. FAIL 82 → 67 (to FLAGGED). PASS unchanged (15). Audit MAJOR 21 → 18; marks-family MAJOR cards 11 → 7.

## Cluster 2: symbol fonts and text layers

**Root causes** (each from the fonts' own data):
- **MathMagic** (OCR, CIE): fonts without a ToUnicode map draw symbols at ASCII codes (`MMBinary` "'" is ÷, "#" ×; `MMExtra` "1" is <; `MMRelation` "G" is ⩽; `MMVariableA` letters are parentheses in steps of size; `MMVariable` "J K L" are bracket pieces; `MMGreek` "r" is π).
- **Advent 3B2** (AQA): ToUnicode maps name the slot, not the symbol ("¼" is =, "þ" +, ":" the decimal point); control-range glyphs "C0/C2/C20/C21" are − × ≤ ≥; its Σ hangs from its origin, so it was laid out on a line of its own.
- **Times New Roman CID subsets** (Edexcel): for glyphs neither ToUnicode nor the subset's (absent) `cmap` names, PDFium reports the **glyph id** as the character: glyph 237 (minus) arrives as "í", digits as control codes.
- **Euclid Extra**: ⇌ and the standard-state sign ⦵ (arriving as "d"); **Euclid Symbol** CIDs; **Wingdings** 0xFC is the tick ✓; the sun sign ☉ is drawn as a ring and a dot with no character; CIE `NewMathSymb` names its glyphs (`thetaslant`, `degrees`) under control codes.
- The sanitizer deleted "box" before a full stop ("Tick (✓) one ." , "from the .").

**Fix.** Per-font identity tables, each entry established from the rendered glyph in a fixture paper (`mathmagic`, `advent`, `mathtype_private`, Wingdings tick); unlisted codes of these symbol fonts are `�` (flagged), never a guessed letter. Glyph-id resolution for CID fonts through Times New Roman's glyph order (generated from the four Windows faces, ids agreeing in all four), accepted only when the PDF's stated width is that glyph's advance, the font's named glyphs agree with the order, and no other font of the family on the page could give the value (otherwise the glyph's measured advance decides, else `�`). Hanging big operators take their line's baseline from the maths axis. A ring-and-dot after a letter is ☉. "box" after a determiner is prose.

**Tests.** `symbol_fonts_read_as_the_symbols_they_draw` (OCR J560 Jun 24, OCR MEI Jun 25, CIE 0580 Nov 22, AQA 7357/1 Jun 19, Edexcel chemistry Jun 22, Edexcel physics 2018, AQA Biology GCSE Jun 22), `to_unicode_maps_parse_chars_ranges_and_packed_strings`, `glyph_ids_resolve_only_on_the_source_evidence`, `symbol_font_codes_not_established_stay_unknown`, `box_as_the_noun_of_a_sentence_is_question_text`.

**Numbers.** PASS 15 → 13 and glyph papers 39 → 56 after this cluster alone: papers that had been passing with wrong symbols now fail honestly (M2 Jan 2009 "0 - t - 6" for 0 ≤ t ≤ 6; OCR MEI Nov 21 "y H 2x²" for y ⩾ 2x²; OCR Biology "6Rd²" for 6Σd²). On the fixtures the fixed papers read correctly (OCR MEI Q6 "0° < x < 360°", AQA Q8 "P(n) = Σ…", "3.25 × 10⁻¹¹", "L☉", "⇌", "E⦵"). In the audit, wrong-symbol MAJOR cards went from mostly unflagged to 6 of 7 flagged. The MathMagic identities found on the CIE fixture then carried to holdout OCR-MEI papers: three of them reached PASS after cluster 3's run.

## Cluster 3: merged questions and page furniture

**Root causes.** AQA Nov 17 Q26 opens with maths ("26  a² − b² ≡ …"), which the "number followed by lowercase prose is a quantity" guard rejected, so Q25 carried Q26. `END_MARKER_RE` knew "total for paper is N marks" but not "TOTAL FOR PAPER = 90 MARKS", and "end of question(s)" but not "END OF QUESTION PAPER", so the last question took the extra-answer pages, the copyright block and the OCR logo page. WJEC's blank-page notice sits mid-page under a page number. OCR's "© OCR 2024" footer touches the last dotted line, so it failed the "isolated" test. Upright axis titles are rotated text, which the figure crop never grew into.

**Fix.** Maths-led headings (number, then a maths span) open questions; both end-of-paper wordings end the map; the blank-page notice is navigation on the page that carries "BLANK PAGE"; answer space is not a neighbour for the footer test; rotated text other than the margin strips grows a figure crop like other labels.

**Tests.** `questions_end_at_the_paper_end_and_maths_led_headings_open_questions` (Edexcel physics 2018, Edexcel chemistry Jun 22, OCR J560 Jun 24, AQA 8300 Nov 17, OCR Economics Jun 24, WJEC Unit 1 Jun 25), `an_axis_title_printed_upright_along_the_axis_is_in_the_figure_crop`.

**Numbers.** PASS 13 → 16 (OCR MEI Jun 19, Jun 23, MEI Further Nov 20; the cluster-2 MathMagic additions made in this run carried to them). AQA Nov 17 fixture now maps 30 of 30 questions.

## Cluster 4: the OCR Economics collapse

**Root cause.** Not missing question numbers: OCR prints "2*", "3*" (quality of extended response assessed) and `HEAD_RE` did not accept the star, so questions 2-5 folded into Q1 (Q1 itself is "1" with parts (a)-(e)).

**Fix.** The star is part of a heading (also after a part label, "1 (a)*"); the card keeps it as "\*"; the "OR" between alternative questions is navigation only when the next line opens a question; "Question No. ……" is answer space; "Section B starts on the next page" is navigation.

**Tests.** `headings_marked_for_extended_response_open_questions` (OCR H460/01 Jun 24), `extended_response_heading_is_stripped_whole`.

**Numbers.** Fixture: 1 card → 5, all strict, IDs match. Holdout OCR Economics Nov 20: 1 card → 5 (30 + 2×25 + 2×25; triage reports "marks over" because a candidate answers one of each pair). Cards 2125 → 2132, ID gaps 19 → 17. PASS unchanged (16). Papers with no question numbers at all remain unhandled (none in the corpus).

## Cluster 5: scanned papers and local OCR

**Root cause.** Any extracted text makes a paper "digital" (the cost policy: zero cloud requests), so a paper whose only text is a watermark ("PMT") went through the layout path with nothing to read: 0 cards and no explanation. edexcel '19 is not a picture scan at all: its text is drawn as outlines and picture strips. A paper with no text at all went to the vision models; offline that is 19 refused requests.

**Fix** (your choice of engine): `src/ocr.rs` uses Windows' own OCR (`Windows.Media.Ocr`, via the `windows` crate that was already in Cargo.lock: a direct dependency edge, no new package, built `--offline`). A page is read when it has print but no text except a watermark, and only when such pages are half the paper or more (a digital paper's picture page is never OCR'd). OCR lines get the same furniture and answer-line rules; every question on an OCR page fails strict with `ocr_transcription` and is kept for review, on the layout and the text-layer paths (provenance by the exact OCR text). Pages the engine cannot read are reported. macOS (Vision) and Linux (tesseract) have no backend yet: there a scan is detected and reported, never guessed.

**Behaviour change to note.** On Windows a fully scanned paper is now imported locally (flagged for review) instead of being sent to the vision models with the user's key. README updated.

**Tests.** `scanned_pages_are_read_by_local_ocr_and_never_clean` (Edexcel M3 Jun 04), `only_a_mostly_text_free_paper_is_read_as_a_scan`.

**Numbers.** Fixtures: Edexcel M3 Jun 04 (scanned) 0 cards / 19 refused requests → 7 cards, marks 75 of 75, all flagged, 0 requests; Delphis S1 paper (scanned) 0 → 7 cards, marks match the page; edexcel '19 (outlined) 0 → 17 of 18 questions (Q13's heading not read). None strict. Triage of the three: FAIL (no cards) → FLAGGED. Blinded audit of 9 of their cards: 8 MAJOR (OCR loses powers, fractions, Greek letters), 1 MINOR, all flagged, marks correct on all 9. No scanned papers remain in `papers/unused/`, so the holdout only shows that nothing else changed: every count identical to cluster 4.

## Cluster 5b: no AI requests of any kind

**Root cause.** The local import was one branch of a pipeline built around AI models. A paper took it only when its text layer made it "digital". Anything else fell through to text-first, crop-first, vision and repair requests: a scan with no local OCR, a picture, or a mark scheme the local reader declined. Those requests went to a provider resolved from the user's saved key, or the built-in free tier. I measured this with a local listener standing in for the network, set as the proxy for every HTTP(S) request the process makes, with a key saved:
- the scanned fixture Edexcel M3 Jun 04, with OCR switched off: requests left the machine;
- the mark scheme 07 (C3/C4 legacy): 25 connections;
- fetch models, generate topics, the worksheet generator, and the OpenRouter usage and generation-cost lookups: 5 connections;
- every question import with a key saved also called OpenRouter's usage API afterwards, whatever the paper.

Found in passing:
- **Mark-scheme hang.** The mark-scheme import locked the database twice (a shadowed guard), so any scheme the reader accepted hung the import.
- **Invented history.** Local imports wrote invented token counts and costs to the import history (an estimate of 1400 tokens a page).
- **Misleading report.** The report could say "0 questions extracted" for a paper nobody read.

**Fix.**
- **No HTTP client.** `reqwest` is gone from MergeMark's dependencies, along with `llm.rs`, `billing.rs`, `cost.rs` and `json_salvage.rs`. (The lockfile still lists `reqwest` because Tauri names it for other targets; nothing builds it here.) The window's content-security policy no longer allows OpenAI, Anthropic, Google or a local Ollama port. A test fails if either comes back.
- **Every import is local, and the pipeline has no model client.**
  - **Question papers:** the layout map and the local transcriber only, for every document. The structure pass, the hybrid map, the per-page fallback, and the text-first, crop-first, vision and repair stages are deleted, not just refused.
  - **Unread pages:** pages no local reader could read are listed in the report (`pagesNotTranscribed`) and quarantined.
  - **Nothing read:** an import that read no questions fails with the reason. For example: "This paper was not transcribed: pages … are scanned, and this computer has no local OCR engine…". Pictures and "no question headings" get their own messages.
  - **Mark schemes:** the local reader only. A refusal fails the import, with the reader's reason.
- **AI-only features** (your choice: disabled, UI kept). Each of these answers "MergeMark works offline: it sends nothing to AI services, so this feature is turned off." and reads no stored key:
  - `fetch_models`, `generate_topics_for_module`, `generate_worksheet_from_pdf`;
  - `get_usage_status`, `set_byok_key`;
  - `get_openrouter_usage`, `get_generation_cost`.
- **The screens:**
  - **Settings:** the key, base-URL and model fields are shown but disabled, and nothing typed there is stored or sent.
  - **Import screen:** it no longer reads or sends a key or model, and a failed import no longer also shows "finished with review items".
  - **Import history:** local imports are recorded at 0 tokens and $0.
- **Extraction cache:** it no longer depends on the model. The cache version is now 4, so entries cached from model runs are dropped.
- **`e2e_import`:**
  - It is offline-only. `--offline` is still accepted.
  - A new `--mark-scheme` mode runs the local mark-scheme reader.
  - `--dump-layout` now also writes `glyphs.txt`, every glyph with its position and font.
  - The certification keeps the request fields the calibration checks; they are now zero by construction, with `"modelClient": "none"`, which the calibration requires.
- **README:** BYOK and the free parses are removed, and "How importing works", the feature list and the tech-stack line are rewritten.
- **Size:** 12,315 lines of Rust are gone (`pipeline.rs` 12,602 → 4,272).

Left as they were:
- A key saved by an earlier version stays in the old `usage_config` table of an existing database. Nothing reads it.
- The command names `parse_pdf_vision` and `parse_mark_scheme_vision` are unchanged.
- The local seam still runs carved questions through scaffolding written for model output. This is offered as a separate task, "Simplify the Tier-0 build seam".

**Tests.**
- `a_scan_with_no_local_ocr_is_not_transcribed_and_sends_nothing` (Edexcel M3 Jun 04, with OCR switched off for the test).
- `a_mark_scheme_the_local_reader_refuses_sends_nothing` (the 07 C3/C4 mark scheme, now a fixture in `papers/used/`).
- `ai_only_features_answer_that_mergemark_works_offline`.
- `mergemark_has_no_way_to_reach_an_ai_service` (Cargo.toml and the CSP).
- `a_mark_scheme_the_local_reader_accepts_maps_onto_its_questions`. I checked it times out with the old double lock.
- `pictures_with_no_text_are_reported_not_transcribed`.
- `mark_scheme_tier0_refusal_is_reported_without_partial_answers`.
- Frontend: script tests for the new `modelClient` gate.

The first three went red on the old code. The tests of the deleted cloud stages were deleted with them, 98 in all, mostly pipeline mock-model tests. Tests of local behaviour that used a mock client keep their assertions without it.

**Numbers.**
- **cargo test:** 340 passed, 0 failed (434 tests before, 344 now).
- **Calibration c5b:** 38/38 OK. Historical 243/244 strict, test papers 69/69.
- **Triage unused-c5b:** PASS 16, REVIEW 3, FLAGGED 53, FAIL 80.
- **Cards:** identical to c5 on all 38 calibration papers and all 152 triage papers, apart from diagram file names.
- **Requests:** now impossible.
- **Blinded audit c5b** (fresh, same seeded 48 cards): 17 MAJOR (c4: 18); cards from passing papers, 1 MAJOR of 14 (c4: 1). No card changed, so the difference is auditor variation. This audit is round 2's baseline.
- **After the review fixes:** they change no card on any of the 190 papers, none of which has an unread page. Only messages, comments and names changed.

## Cluster 5c: local mark-scheme coverage

Seven mark schemes exist locally: the `0*_ms.pdf` set, now six in `papers/unused/` and one (07) in `papers/used/`. Run through the local reader (`e2e_import --mark-scheme`): **0 of 7 accepted.** Before 5b, each of them fell back to 9–78 image requests.

| scheme | pages | refused at | reason |
|---|---|---|---|
| 01 Edexcel 9MA0 P1 (Jun 22) | 33 | page 1 | the cover page ("Mark Scheme (Results)…") is not taken as preamble: `unrecognized table or continuation` |
| 02 Edexcel Further CP1 | 21 | page 6 | `unmapped glyph 38`: a character with no Unicode |
| 03 AQA Physics P1 | 23 | page 14 | `unmapped glyph 119` |
| 04 CIE 9709 P1 | 19 | page 2 | `rotated layout unsupported` (a landscape page) |
| 05 AQA CS 7517/1 (Jun 22) | 52 | page 2 | general marking guidance prose, not taken as preamble |
| 06 community PMT pack | 25 | page 1 | not a board scheme: worked answers laid out as a worksheet |
| 07 CIE 9709/11 (Jun 14) | 7 | page 1 | the cover page ("MARK SCHEME for the May/June 2014 series…") is not taken as preamble |

With the AI fallback gone, a mark scheme can be imported today only if it uses an AQA or Pearson table layout and every page is a recognised table or a recognised preamble. The refusals fall into four kinds:
- cover and guidance pages the preamble rule misses (4 schemes);
- unmapped glyphs (2);
- rotated pages (1);
- a non-board layout (1).

Fixing them is follow-on work under the same rules: fixtures and holdouts, never guess. The census output is in `output/fix-log/ms-census-c5b/`.

## Cluster 6: strict cards that don't render

**Root causes.**

- **Limits split in two.** Each from the glyph data (`e2e_import --dump-layout`, `glyphs.txt`):
  - *Edexcel Further Jun 2025 Q1*, `\int_{1.5}^{2.}^{5}`. An integral's limits are found in a window beside the sign; for this sign the window ends at x = 279.7. The upper limit "2.5" is three glyphs, and its "5" starts at x0 = 279.8, just outside. So "2." became the limit and "5" a second superscript.
  - *AQA Further Jun 2025 Q8(c)*, `\sum_{r=}^{2c}_c`. A different cause: the window does reach the "c" of the lower limit "r = c", but a limit must be script-size, at most 0.85 of the page's text size (11 pt, so 9.35 pt). This paper sets its italic maths letters at 12 pt beside 11 pt Arial (12 of the 15 Times-Italic letters on the page), and the limit's "c" is 9.6 pt, 0.8 of its own font's size. So "c" was not taken and became a second subscript. (My first draft gave this the integral's cause; the fixture showed otherwise.)
- **Minus outside the maths** (physics '18 Q17 and Q22, "A – $6.76 \times 10^{14}$ J").
  - The minus is drawn in Cambria Math; the text layer calls it an en dash.
  - The rule "a standalone dash between maths is a minus sign" needs maths *before* the dash. In an option, the dash follows the option letter, which is prose. So the minus stayed text, outside the `$…$`.
  - The app's preprocessor then made it worse. `healFracturedMath` means to move such a minus inside the maths, but its replacement `'$1-$2$'` printed the dash *and* a hyphen, dropped the space before it and misplaced the closing `$`. "- [MCQ:A] – $6.76…$ J" became "- [MCQ:A]--6.76…$ J$", and the render check saw the option marker as leaked.
- **The Biology table** (Edexcel A Biology Jun 2021 Q5). The table is valid Markdown, and the app renders it. The render check rendered only the text before the card's first option and the option lines, so a table printed between two option groups was never rendered.
  - The same card has a real display defect in the app. `fuseSplitMcqLines` assumes the options end the card, so it folded parts (iv) and (c) into option D.
- **No gate saw it.** The Tier-0 gates checked `$` pairing but not what KaTeX can typeset, so all of these were strict.

**Fix.**
- **Limits.**
  - A limit found in the window is extended along its own baseline through script-size items no more than a script space apart (`extend_limit`).
  - Script size is measured against the larger of the page's text size and the size the item's own font is set at in the text: the median size of that font's letters and digits on the page (`font_text_sizes`). A font set no larger than the text is judged as before.
- **Minus signs.** A lone dash drawn in a maths font, before maths, is a minus. A minus opening a math run is joined to what follows (`$-6.76…$`).
- **New gate.** `math_structure_errors` (validate.rs) finds three faults:
  - a second superscript or subscript on one base;
  - unbalanced braces;
  - a backslash that starts no command, or ends the maths.

  It reads math spans the way the app's renderer does: a dollar after an odd run of backslashes is a literal in prose ("\$5"), but nothing is escaped inside maths. So CIE Nov 2016 Q5's answer prompt "Bills $= \$ ……" is the maths "= \", which KaTeX rejects. It is a Tier-0 gate (`math_structure`) and part of `card_structure_errors`.
- **App preprocessor.**
  - `healFracturedMath` now moves only a minus that touches an opening `$`. A spaced dash stays as printed.
  - `fuseSplitMcqLines` ends an option list at a part label after a blank line, and keeps the rest of the card below the list.
- **Render check** (`verify-corpus-render.mjs`): it now renders everything that is not an option line. It checks more text than before, not less: the KaTeX, leaked-marker and table checks are unchanged.

**Tests.** Each went red on the old code. The fixtures are Edexcel Further Jun 2025, AQA Further Jun 2025, physics '18, Edexcel A Biology Jun 2021 and CIE Nov 2016 (v1), all now in `papers/used/`. The holdouts are Edexcel Oct 2021, AQA Jun 2021 and WJEC Unit 1 Jun 2023, still in `papers/unused/`.
- `a_limit_printed_as_several_glyphs_stays_one_limit` (Edexcel Further Q1).
- `a_limit_letter_in_a_font_set_larger_than_the_text_stays_in_the_limit` (AQA Further Q8).
- `a_minus_drawn_in_a_maths_font_after_an_option_letter_is_maths` (physics '18 Q17, Q22).
- `math_the_renderer_cannot_typeset_is_a_structure_error`. It uses the two fixtures' split limits, plus synthetic strings for the other faults and for valid LaTeX that must pass (`pmatrix`, `{}^{14}_{6}`, `\$5`, a `\\` row break before `$$`). The valid forms come from KaTeX's grammar, not from a paper.
- `a_dollar_inside_maths_closes_it_as_the_renderer_reads_it` (CIE Nov 2016 Q5). The renderer's reading was checked with `output/fix-log/mathspans.mjs`.
- Frontend:
  - "the parts printed after an option list stay outside its last option" (Edexcel A Biology Q5);
  - "a minus touching an opening dollar moves inside the maths", including physics '18 Q17's option line.

**How the gate was checked.** I ran it over every card of a run: 2,670 cards, calibration plus triage, all of whose content is unchanged by the gate. This is a measurement, not a tuning set. It flags 4 cards, all of which KaTeX rejects: the AQA Further Q8 sum, and three holdout cards (Edexcel Oct 2021 Q12, WJEC Jun 2023 Q6, and the CIE Q5 that then became a fixture). It flags no calibration card. Before reading spans as the renderer does, it flagged 2 of these 4.

**Numbers.**
- **cargo test:** 345 passed, 0 failed.
- **Calibration c6b:** 43/43 OK. Historical 243/244 strict, test papers 69/69, zero attempts. Cards are identical to c6 on all 41 earlier rows. The five fixtures:
  - Edexcel Further Jun 2025, physics '18, Edexcel A Biology Jun 2021 and AQA Further Jun 2025 render cleanly (AQA Further Q8 is now `\sum_{r=c}^{2c}`, strict).
  - CIE Nov 2016 Q5 is flagged and still fails to render. Its "Bills = $ ……" lines are dotted answer prompts, cluster 7's subject.
- **Triage unused-c6b**, on the 147 papers still in `papers/unused/` (compared with c5b on the same papers):
  - render-error findings 8 → 4 (papers 5 → 3), and none of the 4 is on a strict card;
  - FAIL 75 → 74 (Edexcel Chemistry Jun 2020, whose table now passes the render check, to FLAGGED); nothing else moved.
- **Holdouts:**
  - AQA Jun 2021 Q15 is fixed: `\int_{6.3}^{6.4}`, three times.
  - Three cards still fail to render, all flagged, each with a different defect: Edexcel Oct 2021 Q12 (`($$\text{u}$$^{-1}$)`), WJEC Unit 1 Jun 2023 Q6 (`A^{\$}BC`) and AQA Physics Jun 2022 Q5 (a fraction of text outside the maths).
  - One holdout card changed between c6 and c6b: AQA Jun 2023 Q8, `\int_0` → `\int_{0}`, which renders the same. The card is strict, but its upper limit π/2 sits on a line of its own: a defect no gate catches, left as a holdout.
- **Blinded audit c6** (drawn from run c6; no holdout card in it changed in c6b): 19 MAJOR of 48 (c5b: 17). Cards from passing papers: 1 MAJOR of 14 (c5b: 1). The two samples share 16 cards, all with identical verdicts. Of the MAJOR cards, only Edexcel M2 Jan 2009 Q4 changed, and it improved: `\frac{- 432}` → `\frac{-432}`. Its MAJOR is the `�` glyphs (cluster 10). So the difference is the sample, not the change.

## Cluster 7: answer space left in cards

**Root cause.** A dotted answer line under a label ("Total resistance = ………", "Light intensity = ……… lux", "Explanation ………", "……… kilometres per hour") was question text to the layout. `is_answer_line_text` accepts at most 6 visible characters beside the dots, and `answer_space_mark` only lines that end in a printed allocation. The drawn-rule form of the same prompt ("temperature = ____ K") was already answer space (`is_answer_prompt`, `FurnitureKind::AnswerPrompt`). So the dotted lines stayed in the card, and the `leftover_answer_line` gate flagged it.

**Fix.** `dotted_answer_prompt` (layout.rs) treats "label (= or :) dots [unit]" as answer space (`AnswerPrompt`), and an allocation printed at its end ("Energy = ……… J [2]") stays a body line through the existing rescue (`answer_space_tag`).
- **The label:**
  - either up to eight words ending in "=" or ":" (outside the maths, as `is_answer_prompt` reads "=");
  - or up to four words, each with a letter, that read as a name, not a sentence.
- **Question text, never answer space:**
  - a command ("Name the gas ………", "Calculate 15% of 80 = ………"; "Name of compound" is still a label);
  - a sentence that runs on into the blank: the cloze "The nucleus contains protons and ………", "Energy is transferred ………", or the last line of a wrapped description, "for communication ………";
  - printed data ("1 3 6 10 ………");
  - maths other than units beside the blank ("√x = ………", as cluster 1 decided);
  - a blank inside a script (a nuclide to complete, "${}^{……}_{……}\mathrm{K}$").
- **Units:** one closed vocabulary, `ANSWER_UNITS`, now shared with `answer_space_mark`. It is the cluster 1 list plus the units the fixtures print: lux, Bq, eV, kWh, rad, dB, words such as "kilometres per hour", and "/mol".
- **Currency:** a currency sign before the blank is its unit ("Bills = $ ………", CIE Nov 2016 Q5).

**Tests.** On the fixtures Edexcel Physics AS Jun 2022 P1, OCR A Chemistry Jun 2024 and OCR A Physics GCSE Nov 2020 (now in `papers/used/`), with lines from the calibration fixtures (CIE Nov 2016, Edexcel Chemistry Jun 2022, OCR A Physics Jun 2017):
- `a_dotted_answer_line_under_a_short_label_is_answer_space`: 13 prompts that go and 15 lines of question text that stay;
- `dotted_answer_lines_leave_the_card_and_cloze_blanks_stay`.

Each went red on the old code.

**Review.**
- The full test suite first caught that the rule would drop OCR J560 Jun 2024 Q13's printed sequence "1 3 6 10 ……… [1]" (cluster 1's test). Labels now need letters.
- The code review then found:
  - a second unit list that had already drifted from `answer_space_mark`'s;
  - no maths, command or nuclide guard on labelled lines;
  - a run-on check that looked only at the last word.

  All are fixed, with test cases. On the holdouts, run c7 had dropped one line of question text: Edexcel Physics GCSE Jun 2024's "produced in an aerial and can be used / for communication ………". The corrected rule keeps it: a label never opens with a function word.

**Numbers.**
- **cargo test:** 347 passed, 0 failed.
- **Calibration c7b:** 53/53 OK. Historical 243/244 strict, test papers 69/69, zero attempts. Run c7 changed cards on fixtures only, and every line it removed was a labelled dotted prompt ("energy = ……… J", "Colour in region E ………", "Answer ……… %"). No marks changed. Strict cards rose on OCR A Physics Jun 2017 (19 → 22), Edexcel '18 (15 → 18), Edexcel Chemistry Jun 2022 (5 → 7) and CIE Nov 2016 (7 → 8). The review corrections changed no calibration card.
- **Triage unused-c7b**, on the 137 papers still in `papers/unused/` (compared with c6b on the same papers):
  - flagged cards 482 → 374;
  - cards flagged `leftover_answer_line` 176 → 51;
  - PASS 16 → 18 (Edexcel A Biology Jun 2018 and Edexcel Physics Jun 2020, from FLAGGED);
  - `no_terminal_ending` 79 → 83: four cards whose prompt was removed now show the ending problem underneath (cluster 8).
- **Holdouts:**
  - Edexcel Physics AS Jun 2017 P2: answer-line cards 5 → 0.
  - Edexcel Physics AS Jun 2025 P2: 4 → 1.
- **The correction cycle (c7 → c7b) on the holdouts.** It changed five cards:
  - it restored the question text "for communication ………";
  - it kept four genuine prompts, so those cards stay flagged with nothing lost: `KE_{\max} = ………`, "Number of α particles = ………" and its β line, and a 62-character label.
  - It also undid run c7's one gain on OCR A Physics Nov 2021 Q2: the "σ = ……… Pa [2]" and "ε = ……… [2]" allocations, back to the c6b state (10 of 14 marks). Symbols that name a quantity are handled in cluster 9.
- **Blinded audit c7b:** 19 MAJOR of 48 (c6: 19); cards from passing papers 1 MAJOR of 14 (c6: 1). Only one MAJOR card changed in cluster 7: Edexcel IGCSE Jan 2014 Q10 lost a "……… minutes" answer line, and its MAJOR issues are a wrong symbol and a figure crop. One MAJOR, OCR A Physics Jun 2019 Q16 (an empty card), is cluster 9's first finding.

## Cluster 8: question ends

Each cause below was found from a fixture's layout dump and from what the gate actually saw (a temporary `[DEBUG-c8]` print of the body at the `no_terminal_ending` gate, since removed).

**Root causes and fixes.**
- **WJEC bodies ending in "." still failed** (WJEC Further Unit 2 Jun 2024 Q1, Q3–Q6).
  - *Page numbers.* At the gate those bodies ended in a page number ("…part (c).\n[2]\n5\n6"); a later step strips it, so the cards looked ended. WJEC prints each page's own number centred 72 pt from the top, just below the running-header zone (0.068 of the page). It is now page furniture when it is the page's own number, centred, isolated and above the body.
  - *Decimal points.* The same dump showed WJEC's decimal point printed as a full stop raised 3 pt ("2·5"). It started a line of its own and the number read "2 5" ("mean 2 5", "0 3"): a changed number, found in passing. A full stop between two digits on one baseline, raised by under half its size, now takes their baseline (`seat_raised_decimal_points`).
- **Unpunctuated commands** (Edexcel GCSE "Write down two factors of 12", "Work out the lowest common multiple (LCM) of 24 and 56"). The question's printed total "(Total for Question 21 is 2 marks)" follows them, but the card's tag is added only after the gates run. A layout body that closes on its own printed total, with nothing kept after it, is now ended (`QuestionBody::ends_at_footer`, used by both ending checks). That is source evidence, like `ends_with_reference`.
  - Commands under 30 characters ("Write down two factors of 12") now fail `too_short` instead. That threshold is left as it is.
- **Furniture in the body:**
  - *Section totals and candidates' instructions:* "TOTAL FOR SECTION A = 20 MARKS" (also "TOTAL FOR STATISTICS IS 50 MARKS"), "Turn over for the next section", "Your answer", "Start your answer here", "Save your work for Question 1 as a word processed document named Answers.", "Extra space is available on the next page if needed", "QUESTION 20 IS THE / LAST QUESTION IN SECTION A", the board's "© OCR 2018" and "© WJEC CBAC Ltd. (…)" lines, and an extract's "Source: News reports, 2019". These are now navigation furniture, matched on the line with its maths unwrapped, since section totals are typeset in bold maths.
  - *AQA's "Typesetter code"* is a bottom-zone footer.
  - *A bare "END"* (Edexcel S1 Nov 2003) ends the paper only when it is the paper's last text and not a line of code, because pseudocode ends blocks with "END".
  - *A question's own "Q7" label* at its foot (Edexcel C4 Jan 2011) leaves the body.
- **AQA tick boxes** ("Tick (✓) one box." over "Burette / Pipette / Syringe / Tube"). Each option has an empty square box drawn to its right, as four strokes about 28 pt across. A column of 2–5 such lines straight after the instruction is the question's list of options. A body that closes on that list is ended (`QuestionBody::ends_on_tick_options`), and the options stay exactly as printed. A row of choices across the page ("25%  50%  75%  100%") is not such a column, and neither is a list with an unboxed line in it.
- **Labels over answer space** (AQA Chemistry Jun 2017: "Mechanism" over an empty space to draw in, "Justification" over ruled lines). The convention that drops a label beside its rule (`labelled_space`) now also covers a one- or two-word label above ruled lines or empty space. A command ("Explain why…") or a tick-box option never counts as one.
- **Found in passing, merged questions:** Edexcel Physics AS Jun 2022 P1 printed Q14 as "*14"; the star marks an extended-response question.
  - The heading was not read, so Q14 ran into Q13. Q13's own parts still matched its total, so the merged card was **strict**, with 11 marks.
  - Fix: a star before the number is part of the heading, and it no longer moves the heading column.
  - New gate `another_questions_total`: a body that holds another question's printed total ("(Total for Question 14 = 6 marks)" in Q13) is never strict.

**Tests.** Each went red on the old code. The fixtures, now in `papers/used/`, are Edexcel Maths GCSE Jun 2022, Edexcel A Economics Jun 2018, AQA Chemistry Jun 2017, AQA Chemistry GCSE Jun 2022, WJEC Further Jun 2024, Edexcel S1 Nov 2003 and Edexcel C4 Jan 2011, plus the AQA Biology GCSE Jun 2022 and Edexcel Physics Jun 2022 fixtures. The holdouts are Edexcel Maths GCSE Jun 2018 and AQA Chemistry Jun 2020.
- `a_page_number_printed_below_the_header_zone_is_page_furniture`
- `a_decimal_point_printed_raised_stays_in_its_number`
- `a_body_closed_by_its_printed_total_ends_there`
- `a_command_closed_by_its_printed_total_is_ended`
- `printed_instructions_and_section_totals_are_page_furniture`: the handoff's lines, plus five lookalikes that stay question text.
- `section_totals_and_typesetter_codes_leave_the_question`
- `a_closing_end_and_a_question_label_leave_the_question`
- `options_beside_tick_boxes_fix_the_end_of_their_question`
- `a_tick_list_with_an_unboxed_line_after_it_stays_as_printed`
- `an_answer_label_needs_answer_space_under_it_and_no_list_around_it`
- `a_label_over_answer_space_is_part_of_that_space`
- `a_heading_starred_for_extended_response_opens_its_question`
- `a_body_holding_another_questions_total_is_never_strict`

**The correction cycle (c8 → c8b).** The code review found:
- Run c8 rewrote tick-box options as `[MCQ:A]`–`[MCQ:D]`. The MCQ structure gate then refused every list that was not exactly four options of a single-part question, so those cards were flagged. The options now stay as printed and only fix the body's end.
- A label over answer space could be one word of a column of options or of a table, so the rule now needs ruled lines or empty space under the label, and nothing around it.
- The command verbs were kept in two lists, and the navigation patterns were matched twice: each is now one list.

**Numbers.**
- **cargo test:** c8 358 passed; c8b 360 passed, 0 failed.
- **Calibration c8b:** 59/59 OK. Historical 243/244 strict, test papers 69/69, zero attempts and tokens. Triage-fixture strict cards 424 (c8) → 434 (c8b). c7b's two ID mismatches (edexcel '19 outlined, Edexcel Physics Jun 2022 P1, whose Q14 was merged into Q13) are gone. The one row still not matched, old Edexcel GCSE Jun 2016 P1F (Q9 and Q24 missing), is cluster 9's fixture, registered before this run.
- **Triage unused-c8b**, on the 131 papers in both runs (compared with c7b):
  - `no_terminal_ending` cards 79 → 20; flagged cards 357 → 292;
  - PASS 18 → 24 (AQA Chemistry GCSE Jun 2024, Eduqas Computer Science Jun 2023, Edexcel A-level Jun 2018 and Jun 2022 Stats, WJEC Further Unit 2 Jun 2019, Edexcel GCSE Jun 2020);
  - printed total matched 102 → 106; papers with marks under the printed total 10 → 6;
  - FLAGGED → FAIL on two papers, neither a lost card:
    - *WJEC Physics Unit 5 Jun 2024* prints no question numbers: it is one practical task, parts (a)–(e). c7b had read its page numbers 2–6 as question numbers and split the task into five cards, two of them strict. Since c8 they are page numbers, so the paper has no question headings and is reported as giving no cards. Papers without question numbers are not handled (see "What is left").
    - *AQA Biology GCSE Jun 2019*: its run hit the harness's time limit (`spawnSync cargo ETIMEDOUT` after 40 minutes, while another session was compiling). Its 9 cards and their flags are the same as c7b's.
- **Blinded audit c8b:** 17 MAJOR of 46 (c7b: 19 of 48). Cards from passing papers: 2 MAJOR of 14 (c7b: 1). Both are from AQA Chemistry GCSE Jun 2024, which passes for the first time:
  - Q1 and Q4 lost the tick-box options printed as bare numbers ("2 / 6 / 14 / 20", "1 / 2 / 3 / 4"): the sanitizer deleted every line holding only a number as page-number debris.
  - Q1 printed C₆H₁₄ as "C₆H14", C₁₂H₂₆ as "C₁2H₂6" and CₙH₂ₙ₋₂ as "CₙH₂n−2": only the first character of a Word subscript was read as one.
  
  Neither defect is new in cluster 8; both are fixed with cluster 9, where the paper is a fixture.

## Cluster 9: marks

Each under-total pattern was diagnosed on its fixture by comparing, question by question, the allocations the PDF prints with those in the layout body. The fixtures are now in `papers/used/`: Edexcel Physics AS Jun 2017 P2, old Edexcel GCSE Jun 2016 1F, edexcel '17, OCR A Physics Jun 2019, OCR A Chemistry Jun 2022 and AQA Economics Jun 2021, plus cluster 8's Edexcel GCSE Jun 2022 and Edexcel A Economics Jun 2018 and cluster 6's CIE Nov 2016. AQA Chemistry GCSE Jun 2024 is the fixture for the two defects cluster 8's audit found.

**Root causes and fixes.**
- **Edexcel Physics 74/80 and edexcel '17 84/90.** A starred extended-response question ran into the one before it, already fixed by cluster 8's "*14" rule: both papers now match their printed totals, pinned by a test on each.
- **Old Edexcel GCSE 92/100.** The star of "*9" and "*24" is set in the text weight before a bold number, so the heading did not read as bold and the map skipped it: two questions lost. A line opening with "*" and a digit now has the number's weight.
- **OCR A Physics 89/100.** Page 9 carries only the notice "Question 16 begins on page 10". It was read as Q16's heading, so the real "16 (a) …" on page 10 looked like a part of the running question, and **all of Q16 (11 marks) was lost**. The audit sampled that card as an empty placeholder. The notice is now navigation furniture.
- **OCR A Chemistry 98/100.** Page 23 opens "Additional answer space if required." above Q20(b). The map skips a page starting with that notice as a spare page, so Q20(b)(i)–(ii) [1]+[1] were lost. A page whose lines after the notice open a part ("(b) …") goes on with its question; OCR's real spare pages at the end have no part labels and are still dropped. The notice line itself leaves the body.
- **Edexcel GCSE 78/80.** Q16's "(Total for Question 16 is 2 marks)" is printed just under a centimetre grid, and lines beside graph paper are taken as its scales and titles. An allocation, a part label or a question's total printed beside a grid is now question text.
- **CIE GCSE 102/104.** Q1(b)(ii)'s answer line "……… ⩽ x < ……… [2]" lost its allocation: the rescue rejected any LaTeX command, and `\leqslant` is one. Cluster 1's prompt vocabulary already lists the relation symbols, so their LaTeX spellings now count the same.
- **A Greek letter naming a quantity** ("σ = ……… Pa [2]", OCR A Physics Jun 2019 Q16(c)). It is a label, and the rescue now reads a Greek letter, plain or subscripted, as a word. It is read back through the same table the engine writes Greek letters with, so every letter the engine writes reads.
- **marks-tag-overcount, Edexcel A Economics Q6 (60 marks, tags 160).** The paper prints Q6(a)–(g) twice: as a list after its extracts, then each part above its answer space, word for word. From where the first part's line recurs, when the reprint adds no part of its own, a body keeps no line twice (figures excepted). The answer form's "Chosen question number:" box is furniture.
  - Q6 also offers "EITHER (f) OR (g)". Tags inside such a choice count once (the largest alternative), both in the card's own mark check (`sum_inline_marks`) and in the triage's tag check. The choice ends at the next part of the alternatives' own kind.
- **marks-total-over, choose-one papers** (AQA Economics: "In Section A, answer EITHER Context 1 OR Context 2. In Section B, answer ONE essay."). The question map now records the alternatives (`choice_groups`) when the paper prints a choice instruction. A bold "EITHER" opens a choice, each bold "OR" its next alternative, and a section heading closes it. The import report and certification carry them (`choiceGroups`). The triage compares the printed total with one alternative of each choice, but only when the paper's own text prints a choice instruction; alternatives carrying different marks fail the paper. AQA Economics' 200 card marks are 80 answered, its printed total. No card's marks change. Where no instruction is printed, nothing is recorded. The last alternative runs to the next section heading or the paper's end, so a compulsory question after it with no heading between would join it; the triage would then fail the paper for unequal alternatives.

**Found by cluster 8's audit** (AQA Chemistry GCSE Jun 2024, now a fixture):
- **Tick-box options printed as numbers were deleted** (Q1 "Tick (✓) one box." over "2 / 6 / 14 / 20", Q4 over "1 / 2 / 3 / 4"). The cards were strict and their layout bodies held the options. The sanitizer, which runs on every card before it is saved, then dropped every line holding only a one- or two-digit number as "page-number debris". Page numbers never reach a card now: the layout engine drops them by position, the text-layer carve by its furniture rules, and the extraction cache stores cards already sanitized. So the rule is removed, not narrowed.
- **Word subscripts of several characters** ("C₆H₁₄" read as "C₆H14", "C₁₂H₂₆" as "C₁2H₂6", "CₙH₂ₙ₋₂" as "CₙH₂n−2"). Word sets them at 7 pt against 11 pt, only 0.6 pt below the line. Such a character counted as a subscript only if its ink started within 0.12 em of a full-size letter, and the rule looked for a letter, never for the subscript before it. A "1" inks well inside its advance, so "₁₄" after H failed at the "1", and every later character failed for want of a letter. Now:
  - the gap may be measured on the ink or on the advance;
  - a reduced character set tight after a subscript character, at the same size and depth, continues that subscript.

**Tests.** Each went red on the old code:
- `a_notice_where_a_question_begins_opens_no_question` (OCR A Physics Jun 2019)
- `a_star_set_in_text_weight_leaves_its_heading_bold` (Edexcel GCSE Jun 2016)
- `a_page_opening_with_extra_answer_space_can_continue_its_question` (OCR A Chemistry Jun 2022)
- `a_total_printed_under_graph_paper_is_the_questions` (Edexcel GCSE Jun 2022)
- `an_inequality_answer_line_keeps_its_allocation` (CIE Nov 2016)
- `a_question_printed_twice_is_kept_once` (Edexcel A Economics Jun 2018)
- `a_choose_one_paper_records_its_alternatives` (AQA Economics Jun 2021)
- `parts_printed_as_alternatives_count_once`
- `tick_box_options_printed_as_numbers_survive_the_import` (AQA Chemistry GCSE Jun 2024, through the whole import, as the card is saved)
- `word_subscripts_of_several_characters_stay_one_subscript` (the same paper)

Added with the review's corrections (the first three went red on the uncorrected code; the last two pin the fixes on their fixtures):
- `parts_printed_as_alternatives_count_once`: a part after the alternatives, and sub-parts inside one;
- `a_part_line_printed_again_drops_lines_only_from_a_reprint`;
- `every_greek_letter_the_engine_writes_names_a_quantity_on_an_answer_line`;
- `starred_questions_keep_the_papers_printed_total` (Edexcel Physics AS Jun 2017 P2: 17 questions, 80 marks; edexcel '17: 20 questions, 90 marks);
- `a_question_with_alternative_parts_keeps_its_printed_total` (Edexcel A Economics Jun 2018 Q6 through the whole import: 60 marks, tags 60);
- triage script tests for a part after the alternatives, unequal alternatives (a FAIL), and alternatives the paper does not print.

**Review corrections.** The code review found:
- a choice ran to the card's end, so a part printed after it was counted inside the last alternative;
- the reprint rule dropped any later line equal to an earlier one, even where the second printing was not a reprint;
- subscripted Greek names ("\lambda_1") and six Greek letters the engine writes were not read back;
- two answer-line test cases (`KE_{\max}`, `\lambda`) came from holdout cards, and are removed with the `\max` / `\min` reading they motivated;
- the triage trusted the import's choice record without the paper's own text, and compared unequal alternatives with the first;
- the extraction cache key (`EXTRACTION_CACHE_VERSION`) was never raised in round 2, so re-importing a cached paper returned its old cards. It is now 5.

Each is corrected. Smaller changes: one part-label pattern, one notice for extra answer space, a `ChoiceGroup` type, and a test helper.

**Found in verification** (run c9, comparing every calibration card with c8b). Run c9 was stopped before its triage finished and run again as c9c with these two fixes.
- **Page numbers in OCR A cards.** Four OCR A cards gained a number after their last allocation, three of them strict: Chemistry Jun 2022 Q21 "25" and Q4 "4", Chemistry Jun 2024 Q3 "10", Physics Jun 2019 Q18 "18". These are page numbers. OCR A sets each page's number centred in the header zone, 9 pt above the first line, and that line's exponent ("cm³") rises to 0.8 em of it. The header rule wants 0.9 em clear, so these numbers had stayed in the body; the sanitizer's bare-number rule had been hiding them. So "page numbers never reach a card" was not yet true. The page's own number (cluster 8's rule) now needs only half a line clear of the text. An exponent or a limit set to a line still touches it. Tests:
  - `a_page_number_set_close_above_the_first_line_is_page_furniture` (red, then green);
  - `a_page_number_needs_half_a_line_clear_of_the_text`: 9 pt clear it is the page's number, touching the line it is text.
  - The rule still needs the number to be the page's own, centred, in the top eighth and with nothing above it. A table value equal to its page's number, alone and centred at the head of that page, would now be taken for it.
- **Leader dots read as a subscript** (Edexcel A Biology Jun 2021 Q7 and Q8, `\text{Answer}_{……}`). The answer line's dots are 6 pt full stops on the baseline, set tight after the word, so the first dot was already a subscript in c8b. The continuation rule then carried the subscript through every dot. A subscript now never opens with a full stop, and a run of stops (or an ellipsis) never continues one. A lone stop inside a subscript stays its decimal point. Tests (each red, then green):
  - `leader_dots_set_small_after_a_word_are_no_subscript`;
  - `a_decimal_point_inside_a_word_subscript_stays_in_it`: "t₀.₅" as `\mathrm{t}_{0.5}`, where the first form of this fix gave `\mathrm{t}_0.5`.
- **Every other number line a calibration card gained is printed on the paper.** For example: the pH values 1 and 7 that AQA Chemistry GCSE Jun 2022 Q3(d) asks to be matched to colours; a table of item counts and the fraction 12/25 split by OCR in the scanned Delphis S1 paper; chart and axis labels; and a column vector's entries. All of these had been deleted. Axis scales of drawn grids now show as text lines (old Edexcel GCSE Q22, flagged), as chart labels already did.
- the symbol cases in `answer_space_gives_its_allocation_only_beside_units_and_prompts` and `a_dotted_answer_line_under_a_short_label_is_answer_space`
- script tests in `scripts/tests/triage-invariants.test.mjs`: like-with-like totals on the AQA fixture, uneven alternatives, and EITHER/OR tags.

**Numbers.** Run c9c holds this cluster with every correction above, on top of the seam session's refactor. That refactor changes no card: 855 calibration cards and 1771 triage cards are identical to c8b (run `unused-seam`).
- **cargo test:** 366 passed, 0 failed.
- **Calibration c9c:** 64/64 OK. Historical 243/244 strict, test papers 69/69, zero attempts and tokens, and every row matches its expected IDs (old Edexcel GCSE Jun 2016 1F gains Q9 and Q24).
  - On the 37 fixture rows in both runs, strict cards 434 → 443: OCR A Physics Jun 2019 20 → 22, old Edexcel GCSE 17 → 21, Edexcel GCSE Jun 2022, OCR A Chemistry Jun 2022 and Edexcel A Biology Jun 2021 one each.
  - 23 calibration cards changed, each checked against its page: the fixes above, plus number lines that had been deleted.
  - The five rows added for clusters 9 and 10 all match their IDs; AQA Chemistry GCSE Jun 2024 has 10 of 10 strict.
- **Triage unused-c9c**, on the 126 papers still in `papers/unused/` (compared with `unused-seam`, whose cards are c8b's):
  - PASS 23 → 26: Edexcel A Economics Jun 2021 and Jun 2024 (Q6 printed twice and its EITHER/OR), Edexcel A Biology Jun 2024, OCR A Physics Jun 2025;
  - printed total matched 101 → 106;
  - papers with marks under the total 6 → 3; tags over the card's marks 5 → 3; marks over the total 6 → 4 (AQA Economics Jun 2024 and OCR Economics Nov 2020 now compare one alternative);
  - OCR A Physics Nov 2021 reaches 70 of 70 (its "σ = …" and "ε = …" lines keep their allocations);
  - flagged cards 278 → 268; `no_terminal_ending` 23 → 18, `leftover_answer_line` 49 → 46, `marks_checksum_mismatch` 5 → 2.
  - 47 cards changed. One paper went the other way: **physics '20 PASS → FLAGGED.** Its Q10 options print "c²ᵣₘₛ". The superscript 2 is laid out on a line of its own, which the sanitizer used to delete. The card was strict with "c_r ms" and no square, and it now shows the stray "2" and is flagged. The subscript itself now reads "c_{rms}". mp1_t Q10 shows the same defect, a displaced superscript ("x²" read as "x" with a lone "2"), and is still strict. These are holdouts and were not debugged.
  - Number lines the cards gained, on the holdouts: almost all are printed content (axis scales, chart values, table entries, atom labels, an extract's line numbers). Two flagged cards gained what may be a page number: OCR B Physics Jun 2024 Q2's last line "6", and a "3" by WJEC Physics Jun 2017's paper code. Not debugged: holdouts.
- **Blinded audit c9c:** 17 MAJOR of 46 (c8b: 17 of 46). Cards from passing papers: 1 MAJOR of 14 (c8b: 2, both fixed here). The statuses changed, so the seeded sample shares only 3 cards with c8b's.
  - The passing-paper MAJOR, Edexcel A Economics Jun 2024 Q1 (a chart cropped without its axis labels), is the same card as in c8b. The paper passes now because its Q6 is counted right.
  - Two sampled cards changed in this cluster, both flagged, and their MAJOR issues predate it. Edexcel Physics GCSE Jun 2024 Q7 has missing figures and gained its graph's axis numbers. WJEC Physics Jun 2017 Q2 is a raw page dump and gained a "3" by the paper code.
  - The other MAJORs are unidentified glyphs (cluster 10), a scanned solutions pack, missing figures and one merged question.

## Cluster 10: symbol fonts

**Census** (`e2e_import --dump-layout` on the 48 holdout papers with `�`, counted by font family; used only to pick fixtures). The families covering the most papers:

| family | papers | glyphs |
|---|---|---|
| Euclid Math Two | 9 | 47 |
| MathMagic MMRelation | 9 | 26 |
| VectorOutline (symbols drawn as curves) | 8 | 31 |
| Monotype SymbolMT | 6 | 10 |
| Times New Roman PS | 6 | 64 |
| MathMagic MMGreekItalic | 5 | 63 |

**Codes established**, each from the font program itself on a fixture: glyph names, the glyph's rendered outline (`glyphshots.py`), or the installed font's own cmap, with the advance the PDF declares.
- **Euclid Math Two as a TrueType subset** (Edexcel C4 Jan 2006; Identity-H, no names, no cmap; PDFium reports the glyph id). Glyph 45 renders "⩽" ("0 ⩽ t ⩽ 2π") and glyph 46 "⩾" ("t ⩾ 0"), both with the 778 advance in the glyph program. They are accepted only at that width, as for Times New Roman (`original_entry`).
- **Euclid Math Two as CFF** (Edexcel FP2 Jun 2014). The subset names the glyph at code 2 `lessthanorequalangled` (its built-in code 0x84, which round 1 had as the private code 0xF084, ⩽). The name now reads as ⩽ wherever it occurs. Its "greater than" twin was not seen on a fixture and stays `�`.
- **Monotype SymbolMT as a TrueType subset** (FP2 Jun 2014). The installed `symbol.ttf` (5.01, 192 glyphs) orders its glyphs by the Symbol encoding in its own cmap: ids 3–97 are codes 0x20–0x7E and ids 98–191 codes 0xA1–0xFE. Glyphs 122, 123 and 151 are 0xB9 "≠", 0xBA "≡" and 0xD6 "√", which the renders show ("z ≠ i", "sin 5θ ≡ 16 sin⁵θ", "a√2 + b"; the last is the card the cluster 7 audit marked MAJOR for "a�2 + b"). They are accepted only at the Symbol advance.
  - *That embedded subsets keep this order:* the fixtures embed 34 SymbolMT TrueType programs. 33 keep all 192 glyphs, and 161 of the 163 glyphs they draw have exactly the installed font's outline at the same id. The other two are glyph 0 (`.notdef`, never read) and a 2-glyph subset that renumbered its glyphs (naikermaths M16), whose glyph 1 lies outside the ids read this way.
  - *Limitation:* a renumbered subset with more glyphs would not keep the order, and the advance check cannot tell symbols of equal width apart (≠ ≡ ≈ ± × ÷ ≤ ≥ √ are all 549). No fixture has one.
  - The glyph-name route for Symbol (CFF fonts naming glyphs "gNNN") now uses the same order. It had also tried the code one below (a hedge with no evidence), which is gone.
- **Times New Roman converted to CFF** (Edexcel S1 Jun 2012) names its glyphs by the original TrueType ids ("g153", "g32"). These now resolve through round 1's Times New Roman glyph order, with the width check: g153 is "∑" (713), g32 "=" (564), g24 "5". Only Times New Roman is read this way, the one family a fixture showed.
- **MathMagic** (OCR Further Nov 2021), from the rendered glyphs:
  - MMRelation "H" is ⩾ and "/" is ≡;
  - MMGreekItalic "a b c i m r" are α β γ θ λ π (its own order, not the Symbol font's).
- **Not established:**
  - *VectorOutline* glyphs are symbols converted to curves, with no font program to read. Telling them apart would mean matching shapes, which this cluster does not do, so they stay `�` and flagged.
  - *Smaller families* (Advent variants, MathExtn, NewMathExtn, CIDFont, …) each cover one or two papers, and no fixture has been taken for them.

**Tests.**
- `symbol_fonts_read_as_the_symbols_they_draw`, round 1's test per family, now also runs on the fixtures OCR Further Nov 2021, Edexcel C4 Jan 2006, FP2 Jun 2014 and S1 Jun 2012 (now in `papers/used/`). Every glyph of each paper is identified, and each symbol reads as printed in its question. It went red on the old code.
- Round 1's negative tests now cover the new tables: an MMRelation and an MMGreekItalic code not established, Euclid Math Two's glyph 47 and a wrong advance, SymbolMT at a wrong advance and outside ids 3–191, a glyph name for a family no fixture showed named that way, and Euclid Math Two's "greaterthanorequalangled". All stay `�`.

The holdouts are Edexcel IGCSE Jan 2014 and Jun 2018, C4 Jun 2014, M2 Jan 2009, old Edexcel GCSE Mar 2013, Further Jun 2019, OCR Further Jun 2023 and Jun 2025, and OCR GCSE Nov 2021.

**Review corrections.** The code review found that:
- SymbolMT's arithmetic also read ids 0–2 (glyph 1 as "≅");
- the glyph-name route ran for every family with a known glyph order, though only Times New Roman's names were seen;
- the two Symbol routes used different arithmetic;
- the tests sat in a separate function that checked the paper's text as a whole.

Each is corrected, starting from a failing test.

**Found in verification** (run c10, reading each calibration card it changed against its page). Run c10 was followed by c10b with this fix.
- **Edexcel S1 Jun 2012 Q2 turned strict with numbers missing.** The page prints "Sxx = 5606.25 … Syy = 4244"; the card read "S_xx = 5606. 5 … S_yy =".
  - The paper's Times New Roman subset numbers its glyphs in order of use, so "2" and "4" are codes 9 and 10. The text layer reports them as a tab and a line feed.
  - The layout engine dropped every line feed and turned every tab into a space, so the 4s vanished and the 2 became a space.
  - In c9c the card's other unidentified glyphs had kept it flagged. Once cluster 10 identified those, nothing showed the loss.
  - A tab, line feed or carriage return with a glyph's height is now kept as a glyph with its code. The font's own glyph names then give "2" and "4" with their widths, as for its other codes.
  - Whitespace has no height: in the fixtures a tab's box is about 1% of the font size, and a full stop's about 10%. So the line is drawn at 3% of the size. The first form of this fix used 30%, which would still have dropped a stop, a hyphen or an equals sign at these codes.
  - Test: `glyphs_set_at_the_tab_and_line_feed_codes_are_read`, red then green.
  - Census of whitespace codes that draw ink (used only to pick fixtures):
    - fixtures: S1 Jun 2012 (5 glyphs at the tab and line-feed codes) and the legacy C3/C4 mark scheme (1);
    - edexcel '18 and January 2025 QP report 10 more at the text layer as spaces, none of which the layout engine shows as a space;
    - holdouts: six papers have some.

**Numbers** (run c10b, with every correction above).
- **cargo test:** 367 passed, 0 failed.
- **Calibration c10b:** 64/64 OK. Historical 243/244 strict, test papers 69/69, zero attempts and tokens, and every row matches its expected IDs.
  - Strict cards on the fixture rows 473 → 488.
  - The four fixtures: OCR Further Nov 2021 5 → 11 of 11, C4 Jan 2006 6 → 8 of 8, FP2 Jun 2014 5 → 8 of 8, S1 Jun 2012 4 → 6 of 7.
  - Each of the 15 changed cards replaces `�` with the printed symbol, plus S1 Jun 2012's digits.
- **Triage unused-c10b**, on the 126 papers (compared with c9c):
  - PASS 26 → 31: M2 Jan 2009, OCR Further Jun 2025, OCR MEI Further Jun 2024, OCR MEI Nov 2021 and Edexcel IGCSE Jan 2014;
  - FAIL 51 → 44;
  - papers with an unidentified glyph 44 → 37, glyph findings 113 → 87;
  - cards flagged for an unidentified glyph 139 → 106, flagged cards 268 → 243.
  - 30 cards changed. Each replaced `�` with a symbol that reads as the question requires: ⩽ and ⩾ in ranges, ≡ in congruences, θ, π, α and β, ∠ABC, ∩ and ∪, and modulus bars.
- **Blinded audit c10b (seeded sample):** 13 MAJOR of 46 (c9c: 17). Cards from passing papers: 0 MAJOR of 14 (c9c: 1). No MAJOR concerns a symbol this cluster reads. The wrong-symbol MAJORs are glyphs still unidentified, all flagged.
- **Targeted audit:** every card this cluster changed that is strict afterwards, against its pages: 25 holdout cards and 15 fixture cards. Every symbol the cluster reads was correct, but six of these cards are MAJOR for other reasons. Their unidentified glyphs had kept them flagged; once those were read, nothing else stopped them.
  - *Fractions from MathMagic papers garbled* (holdouts, 3 cards): OCR Further Jun 2023 Q7 "cos θ = 4/5" as `=_5^{4}`; OCR Further Jun 2025 Q7 "(1/2)π" with the letters of "with" inside it; OCR MEI Further Jun 2022 Q7 "1/z" split over three lines.
  - *The OCR logo and "Copyright Information" heading* from the back page, embedded as a figure of the paper's last question (holdouts, 2 cards: OCR MEI Further Jun 2019 Q5 and Jun 2022 Q8).
  - *A coordinate grid cropped to the shape*, without its axes (fixture, old Edexcel GCSE Jun 2016 Q22).
  - 1 MAJOR in 15 fixture cards and 5 in 25 holdout cards. These are fraction reconstruction and figure detection, outside this cluster. They were not debugged, since five are holdouts and no gate is added on holdout evidence, and they are listed in "What is left".

## Cluster 11: local OCR on macOS and Linux

`ocr.rs` now has three backends behind the same `available()` and `recognize()`:
- **Windows:** unchanged (`Windows.Media.Ocr`).
- **macOS:** Apple's Vision framework, `VNRecognizeTextRequest` at the accurate level. It is called through `objc2` 0.6.4 and `objc2-foundation` 0.3.2 and links the `Vision` framework.
  - Both crates, and `objc2-core-foundation` behind `NSRect`, were already in `Cargo.lock` through Tauri. MergeMark now lists the first two as macOS-only dependencies. `objc2-foundation`'s default features (every Foundation class) are off; MergeMark asks only for std, NSArray, NSData, NSDictionary, NSEnumerator, NSGeometry, NSObject, NSRange, NSString and objc2-core-foundation. No crate was added; the lock file only lists the two under MergeMark.
  - English is requested as en-GB and en-US, filtered through the languages Vision itself reports, since Vision lists only en-US and an unsupported language would fail the request.
  - Each page runs in its own autorelease pool.
  - Each line is a Vision observation, and each word's box is the one Vision gives for that word's range, converted from Vision's normalised, bottom-left coordinates.
- **Tesseract** wherever the system has no engine (Linux): `tesseract <png> stdout --psm 1 tsv` on a temporary copy of the page, a new file the import creates (never one already there) and removes. Words (TSV level 5) are grouped by block, paragraph and line. A machine without `tesseract` on the PATH has no engine, so a scan is reported as not transcribed.

Unchanged: `pdf_render.rs` and `pipeline.rs` still call `ocr::available()`, OCR text is always flagged and never strict, and a scan with no engine is never sent anywhere.

**Tests and checks.**
- `tesseract_tsv_words_group_into_their_lines`: TSV parsing.
- `tesseract_reads_a_scanned_page_when_installed`: the scanned fixture Edexcel M3 Jun 2004 must read "flywheel". It runs only when `tesseract` is installed. Tesseract is not installed on this machine yet (the WSL install needs your password), so the test skipped in every run: **the Tesseract path is untested at run time.**
- The Vision backend compiles for `aarch64-apple-darwin` (target added with your approval), checked in a standalone crate with the same locked `objc2` crates and features. The whole app cannot be checked for macOS here: the build script of `objc2-exception-helper` (a Tauri dependency) compiles Objective-C and needs Apple's toolchain. A compile check does not check Objective-C message types, which objc2 checks at run time, so the macOS backend is compile-checked but untested at run time.
- The code review's corrections:
  - the feature list (the earlier comment claimed default features were five classes; they are all of Foundation);
  - the language filter and the autorelease pool;
  - the temporary file;
  - the live test, which now fails if the page cannot be rendered instead of passing silently.
- `README.md` describes the three engines.

**Numbers** (run c11, the final state of round 2).
- **cargo test:** 369 passed, 0 failed (the two OCR tests added; the live one skipped, see above).
- **Calibration c11:** 64/64 OK. Historical 243/244 strict, test papers 69/69, zero attempts and tokens, every row matching its IDs. No card differs from c10b, which is expected: Windows keeps its engine.
- **Triage unused-c11:** every card identical to c10b on the 126 papers.
- **The late fixes' review.** The three fixes found in verification (OCR A page numbers, leader dots, glyphs at the tab and line-feed codes) were reviewed after the clusters' reviews. That review's corrections are described in clusters 9 and 10, together with the audit pack's page reading. Run c11b holds them:
  - cargo test 371 passed, 0 failed;
  - calibration 64/64 OK, historical 243/244, test papers 69/69, zero attempts;
  - no card differs from c11, in calibration or in triage. The corrections concern cases the corpus does not print, which their tests cover.

## What is left

On the 126 holdout papers after round 2 (run unused-c11):

| remaining hard finding | papers | cards | cause |
|---|---|---|---|
| unidentified glyph | 37 | 106 cards, all flagged (87 findings) | symbol-font glyphs no fixture established (each is `�`, never a guess): symbols drawn as curves (VectorOutline) and font families of one or two papers each (Advent variants, MathExtn, NewMathExtn, CIDFont) |
| card marks below printed total | 3 | flagged | holdouts, not debugged: OCR A Chemistry Jun 2017 (97 of 100), WJEC Unit 1 Jun 2023 (114 of 120), Edexcel GCSE Jun 2018 (78 of 80) |
| card marks above printed total | 4 | | choose-one papers whose instruction the question map does not recognise, so no alternatives are recorded: the CS/decision paper (125 vs 100), AQA Further Jun 2021 (58 vs 40), AQA GCSE Jun 2024 (85 vs 80), mp1_t (129 vs 125). Not debugged |
| marks tags exceed the card's marks | 3 | 3 of 3 flagged | 06_community_pmt_pack Q9 (48 vs 7), Edexcel Jun 2024 (13 vs 11), Edexcel IGCSE Jun 2022 Q14 (14 vs 4). Holdouts, not investigated |
| render error | 3 | 4, all flagged | unchanged since cluster 6: Edexcel Oct 2021 Q12, WJEC Unit 1 Jun 2023 Q6, AQA Physics Jun 2022 Q5 |
| no cards | 1 | | WJEC Physics Unit 5 Jun 2024 prints no question numbers (see below) |

Known gaps outside these numbers:
- Scanned papers are imported but their text needs a person (cluster 5's audit). The macOS (Vision) and Linux (Tesseract) backends exist but are untested at run time: the macOS one is compile-checked only, and Tesseract is not installed on this machine. With no engine, a scan is reported as not transcribed, never sent anywhere.
- Mark schemes: the local reader accepts 0 of the 7 schemes available (cluster 5c); a refused scheme is reported as not transcribed.
- The CIE M2 rotation arrow and its "5 rad s⁻¹" label lie 25pt beside the drawing and are not joined to its crop (the label's text is still on the card).
- Single-part Edexcel questions now carry their footer allocation as a tag; cards imported before this run do not.
- The audit-pack page picker sometimes attaches the wrong page. One cause, found and fixed in round 2: `pdftotext` splits its whole-file output at form feeds, and some pages carry a form feed in their text, so its index ran ahead of the PDF's pages. The picker now reads each page by itself.
  - The triage's page count (`stats.pages`, characters per page) still comes from that split, which also counts the empty text after the last form feed. It is left as it is so that runs stay comparable.
- Maths no gate catches: AQA Jun 2023 Q8 is strict, but its integral's upper limit π/2 sits on a line of its own (holdout, not debugged).
- A paper that prints no question numbers gives no cards (WJEC Physics Unit 5 Jun 2024, one practical task in parts (a)–(e)). The import says that no questions were found in the paper.
- A superscript set over a subscript of several letters, or inside an exponent, lands on a line of its own:
  - physics '20 Q10 "c²ᵣₘₛ" (flagged);
  - mp1_t Q10 "x² − 4x + 5" read as "x − 4x + 5" with a lone "2" (still strict).

  Holdouts, not debugged. Until cluster 9, the sanitizer deleted the lone "2" and the cards looked clean.
- The text-layer carve, used only when a question has no layout body, still drops every line that holds only a number (its page-number rule cannot tell a page number from an option without geometry).
- Two flagged holdout cards end on what may be a page number (OCR B Physics Jun 2024 Q2 "6", WJEC Physics Jun 2017 Q2 "3"); OCR A's is handled (cluster 9).
- Axis scales of drawn graphs and grids appear as lines of numbers in the card text, as their titles already did.
- **Strict cards wrong for reasons no gate sees.** The targeted audit after cluster 10 found six among the 40 cards that cluster made strict. These come first for the next round:
  - fractions from MathMagic papers garbled (OCR Further Jun 2023 Q7, Jun 2025 Q7, OCR MEI Further Jun 2022 Q7);
  - the OCR logo from the back page embedded as the last question's figure (OCR MEI Further Jun 2019 Q5, Jun 2022 Q8);
  - a coordinate grid cropped to the shape without its axes (old Edexcel GCSE Jun 2016 Q22, a fixture).
- Monotype Symbol glyph ids are read in the installed font's order, checked by advance. A subset that renumbered its glyphs would not keep that order, and the advance cannot tell apart symbols of equal width. Of the 34 SymbolMT programs in the fixtures, only one did (2 glyphs), and its ids fall outside those read this way.

## Files

Round 1 pipeline: `src-tauri/src/layout.rs`, `deterministic.rs`, `sanitize.rs`, `pdf_render.rs`, `pipeline.rs`, `doc_map.rs` (fixture path), `ocr.rs` (new), `lib.rs`, `bin/e2e_import.rs` (`--dump-layout` developer view), `Cargo.toml` (Windows `windows` features). Fixtures: 16 papers moved from `papers/unused/` to `papers/used/`. README updated for local OCR.

Round 2:
- **Pipeline:**
  - `layout.rs` (clusters 6–10);
  - `deterministic.rs`, `validate.rs`, `sanitize.rs`, `pipeline.rs`;
  - `commands.rs` (offline commands and import tests);
  - `db.rs` (`EXTRACTION_CACHE_VERSION` 5);
  - `ocr.rs` (three backends);
  - `bin/e2e_import.rs` (`choiceGroups` in the certification);
  - `Cargo.toml` and `Cargo.lock` (macOS-only `objc2` and `objc2-foundation`, both already locked), `tauri.conf.json`.
  - The seam session's refactor of `pipeline.rs`, `deterministic.rs` and `validate.rs` is in the same tree (`output/fix-log/seam/`).
- **Scripts:**
  - `scripts/lib/triage-invariants.mjs` (choices, EITHER/OR) and its new test file;
  - `scripts/build-audit-pack.mjs` (fixtures read from `papers/used/`, `--ids` for targeted packs, the page-count check).
- **Fixtures:** 26 more papers moved to `papers/used/` (65 in all, 126 question papers left in `papers/unused/`), registered in `scripts/lib/zero-cost-accounting.mjs` and `scripts/fixtures/source-expected-ids.json`. README updated for the three OCR engines.
- **Run outputs** (gitignored):
  - `output/triage/run-unused-c5b` … `run-unused-c11` and `run-unused-seam`;
  - calibration `output/zero-cost-orchestration/c-calibration/run-c5` … `run-c11`;
  - snapshots `output/fix-log/snap-c5` … `snap-c11` and `snap-seam`;
  - tools in `output/fix-log/`: `compare.mjs`, `trajectory.mjs`, `card-changes.mjs`, `bare-numbers.mjs`, `cal-results.mjs`, `inked-spaces.py`, `blind.mjs` and the glyph tools.
