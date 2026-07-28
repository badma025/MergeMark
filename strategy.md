# Diagnosis & High-Level Strategy — Before Any Code Changes

## What I Read
- `pipeline.rs`: PVRV loop, `contents.join("\n\n")`, `items.retain()` for collateral, per-item validation
- `doc_map.rs`: `question_heading_regex`, `scan_text_layer`, `build_spans_from_vision`, `expected_max_q` monotonic guard, `append_text_only_short_answer_spans`
- `validate.rs`: `value_to_question_number`, `normalize_decimal_parts`, diagram consistency, terminal endings

---

## The Three Cascading Failures (Root → Symptom)

### Root: Regex Noise (Doc Map)
`question_heading_regex` is too loose. It matches:
- Quantity strings (`1 kg`, `3.5 V`)
- Sub-part labels (`03.1`, `01 5` as question 15)
- Page numbers (`2 5 IB/M/...` when `chars_remaining` guard is weak)
- Marks tags (`[30 marks]` → number 30)

These false headings corrupt the heading array, which corrupts `append_text_only_short_answer_spans` (heading-only carving for MCQs) and feeds garbage into `build_spans_from_vision`.

### Symptom 1: Missing MCQ Bug (22 / 32 found)
`build_spans_from_vision` and `build_map_from_structure` apply a strict global `expected_max_q` sequence filter (`q >= expected_max_q`, jump > 15 kills, `q == page + 1` kills). When false headings inject outliers (e.g. 30 from marks, 15 from `01 5`), the filter either:
- Drops the outlier and everything after it (sequence breaks)
- Keeps the outlier but kills the real dense questions because `page == 0` and `q` doesn't match the expected progression
- `vision_bounds` math (`start_page.max(b.last_page)`) collapses spans into empty ranges when overlapping headings confuse the vertical order

### Symptom 2: Frankenstein Bug (Concatenation)
In `extract_span`, the loop builds `contents: Vec<String>` from `items` and assembles with `contents.join("\n\n")`. There is NO guard enforcing `items.len() == 1`. If the LLM returns 3 questions in one array, all 3 pass `retain()` (if their numbers match or are close) and are concatenated into one massive `BuiltQuestion`. `validate_span_items` validates each item individually but never checks that the aggregate assembled content represents only ONE question.

### Symptom 3: Collateral Questions Fail Silently
`items.retain(|item| item.question_number == Some(span.number))` silently deletes collateral questions (e.g., Q9 when extracting Q8). The repair loop (`continue`) only fires if `items.is_empty()` after retention. So:
- If the LLM returns Q8 + Q9, Q9 is dropped silently, Q8 is accepted → no repair triggered, user never knows Q9 was extracted
- If the LLM returns ONLY Q9 (wrong question), retention empties the array, repair fires — but the repair prompt says only "You extracted data, but none of it was Question N". It never quotes WHAT collateral was found, so the model doesn't learn the boundary error.
---

## Proposed Logical Strategy (3 Points)

### 1. Map All 32 Questions Accurately (Kill Regex Noise)
**Strategy: Tighten the heading filter; separate headings from marks; make span building resilient to outliers.**

- **Heading filter (`doc_map.rs`)**: Before accepting a regex match as a `QuestionHeading`, apply a multi-layer filter:
  1. The matched number must be within a plausible exam range (1–50, not just 1–200).
  2. The number must NOT match common quantity/unit patterns (reject if the surrounding text contains `kg`, `m`, `V`, `cm`, `N`, `J`, etc. within a window).
  3. The match must be followed by substantial text (`chars_remaining > 30`) — this already exists but must be strictly enforced, not just for `chars_remaining` but also requiring that the text does not look like a footnote/footer (`marks` pattern nearby without `Total for Question` is a red flag).
  4. Reject sub-part patterns explicitly: `03.1` (compact decimal with leading zero + dot + digit) and `01 5` (two tokens, first is `01`/`02` etc., second is single digit) must be parsed as SUB-PARTS, not whole questions. Only the integer part (`3`, `1`) should ever become a heading, and only if the text layer also shows a clear parent number (`3` for `03.1`).
  5. Reject heading candidates that are exactly page numbers (`q == page + 1`) unless there is independent structural evidence (e.g., a footer for that question nearby).

- **Separate headings from marks**: `marks_re` (`[3 marks]`) currently feeds false headings when `heading_re` captures digits. The heading regex must not match strings where the primary context is a marks tag. A simple guard: if the matched text is within 20 chars of a marks tag (`[\d+ marks]`), treat it as a marks allocation, not a question heading.

- **Monotonic sequence filter (`build_spans_from_vision` / `build_map_from_structure`)**: Replace the brittle global `expected_max_q` guard with a **per-page plausible-number filter** followed by a **loose sequence check**:
  1. On each page, collect all candidate numbers from vision structure.
  2. Filter out obvious outliers per page: reject any number that has no adjacent number within ±3 on the same page (a lone `30` on a page with `3,4,5` is a false positive from marks or page numbers).
  3. Only after per-page filtering apply a loose sequence: accept ascending sequences even with gaps up to 30 (dense MCQ pages jump from 8 to 9 to 10 without issue; a gap > 30 indicates hallucination, not a missing page).
  4. For `vision_bounds`: instead of the strict `start_page.max(b.last_page)` clamp that creates empty spans, compute span ranges by taking the MIN start and MAX end across all valid headings/footers for the same question, allowing overlap only when vertical y-clips (`start_y_frac`, `end_y_frac`) clearly separate them on the same page.

- **Heading-only carving (`append_text_only_short_answer_spans`)**: Only carve headings that passed the tightened filter (not raw regex hits). When carving same-page spans for dense MCQs, compute vertical bands by sorting validated headings by `y_frac` and splitting at the midpoint between adjacent headings (`end_y = next_start - 0.005`). Never skip a heading simply because another span "covers" the page — check vertical overlap (`y_frac` ranges), not just page index overlap.

### 2. Prevent Extraction Concatenation (Kill Frankenstein)
**Strategy: Enforce single-item output per span; assemble only the first valid item; treat multi-item responses as repairable errors.**

- **In `extract_span` (`pipeline.rs`)**: Before processing any chunk response:
  1. After `parse_llm_json::<AiQuestionPage>`, check `items.len()`.
  2. If `items.len() == 0`: empty response (continue repair if budget remains, else quarantine).
  3. If `items.len() > 1`: this is a **collateral / multi-question response**. Do NOT concatenate. Instead:
     - Identify which item(s) match the target `span.number`.
     - If exactly ONE item matches: accept ONLY that item (ignore the rest).
     - If ZERO items match: trigger repair with a message quoting the actual numbers found ("You returned questions 9, 10 — I asked for 8").
     - If MULTIPLE items match the target (e.g., LLM split Q8 into two items): this is a split/continuation case. Check `looks_like_new_question()` on the second item's content relative to the first. If it looks like a new question (part reset, new bold heading, new number), treat it as collateral and drop it. If it's a genuine continuation (same number, advancing sub-parts), accept ONLY the FIRST item and discard the continuation as redundant (the span covers the full question; if a continuation is needed, the page range should extend, not split the item).

- **Assemble logic**: Replace `contents.join("\n\n")` with a single assignment:
  - After selecting the single valid item (`items` array reduced to at most one entry), take `item.content.unwrap_or_default()` directly.
  - Never accumulate `contents` across chunks for multi-item arrays. Each chunk produces at most one accepted item; if a chunk produces zero or more than one valid item, handle as above (repair or discard extras).

- **Content assembly**: The final `BuiltQuestion.content` should come from the single accepted item's `content` (after `clean_question_content`, `normalize_decimal_parts`, etc.), not from joining multiple strings.

### 3. Handle Collateral Questions Without Failing Validation (Repair Loop Intelligence)
**Strategy: Use collateral data as repair feedback; don't silently drop it; don't fail validation on collateral alone.**

- **Retention + Quoting (`pipeline.rs`)**: Change `items.retain()` from a silent filter to an **auditable filter**:
  1. Before retention, collect all item numbers present in the raw response.
  2. After retention, if any items were removed, quote the removed numbers in the repair prompt: `"PREVIOUS ATTEMPT: You also returned questions [9, 10] when asked for Question 8. Drop all items except the one for Question 8."`
  3. If retention leaves exactly one item matching the span: proceed (no extra repair needed, but the anomaly is logged).
  4. If retention leaves zero items: proceed with existing repair logic (`"none of it was Question N"`), but ENHANCE it with the quoted collateral numbers so the model learns the boundary.

- **Validation loop (`validate_span_items`)**: The per-item validation should remain strict (number must match, content must not be empty, bbox count must match placeholders). However, add a **pre-validation aggregate check**:
  - Before calling `validate_span_items`, check `page_items.items.len()`.
  - If `len > 1`: this is an automatic repair trigger (not a validation failure of individual items, but an **aggregate violation** — "more than one item returned for a single-question span"). The repair prompt should explicitly say: `"Return ONLY ONE item. You returned 3 items — combine only if they are sub-parts of Question N; otherwise delete the extra items."`
  - Only after reducing to `len <= 1` does `validate_span_items` run.

- **Repair budget**: The repair loop already runs `max_attempts = 1 + max_repairs`. When collateral is detected, count it as a repair attempt (`report.repairs += 1`) so that repeated collateral errors don't silently burn the budget without feedback.

- **Verification of boundary**: After accepting a single-item response, verify that the item's content does NOT contain headings/sub-parts that clearly belong to a different main question (e.g., a bold `**9.**` heading, a `(a)` label that resets after reaching `(b)` on the previous chunk). If a boundary violation is detected in the accepted content, trigger a final repair with the exact offending line quoted.

---

## Implementation Order (When You Say Go)

1. **Doc map fixes first**: Tighten `question_heading_regex` / heading filter in `scan_text_layer`; fix `expected_max_q` logic in `build_spans_from_vision` and `build_map_from_structure`; fix heading-only carving overlap logic.
2. **Pipeline fix second**: Change `extract_span` to enforce single-item output (`len > 1` = repair trigger), eliminate `contents.join()`, and quote collateral in repair prompts.
3. **Validation enhancement third**: Add aggregate `len > 1` check before `validate_span_items`; enhance repair messages with quoted collateral.

---

## What I Am NOT Changing (Without Your Approval)
- The vision structure pass prompt (it already instructs non-overlapping bands; the problem is filtering its output, not the prompt).
- The diagram audit / crop pipeline (it works; the bugs don't involve diagram logic directly).
- The mark-scheme pipeline (`run_markscheme_pipeline`) — it has similar patterns but the user only described question-pipeline symptoms.

Confirm the logic above — especially the single-item enforcement and the per-page outlier filter for headings — and I'll write the Rust implementation.
