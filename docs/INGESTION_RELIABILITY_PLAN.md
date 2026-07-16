# MergeMark Ingestion Reliability Plan
*Assessment of `master` (commit 745b618) + a systematic fix so pipeline correctness no longer depends on AI behaviour.*

> **STATUS: IMPLEMENTED (2026-07-16)** — all phases below now live in the codebase:
> `geometry.rs` (§3.1 bbox sanitizer), `json_salvage.rs` (§3.2 boundary discipline),
> `validate.rs` (deterministic validators), `doc_map.rs` (Phase 1 Document Map),
> `llm.rs` (Phase 6 mockable client), `pipeline.rs` (Phases 2–5 + golden-test suite),
> `db.rs` (UNIQUE index + dedupe migration), `commands.rs` (thin Tauri glue,
> idempotent upserts, `import-report` event), `IngestionDropzone.tsx` (report toasts).
> Algorithm logic verified in `scratch/bug_verify/logic_mirror.py` (32 checks green).
> The sandbox has no Rust toolchain — run `cargo test` on your machine to compile-verify.

---

---

## 0. Executive summary

**Root cause of the instability:** the pipeline currently lets the AI be the *authority* on structure. The AI decides question numbers, `is_continuation`, merging/splitting, marks, topic tags, and bounding-box coordinates — and Rust mostly *decorates* those decisions (sanitise strings, insert into DB). When the AI is wrong, the result is **silent data corruption**, not a visible error.

**The systematic fix is an inversion of control:**

> **The AI proposes. Rust disposes.**
> The AI only ever returns *proposals*. Every proposal passes through a deterministic Rust **validation gate**; failures go through a bounded **repair loop** (re-prompt with the exact validator errors); anything still failing is **quarantined and reported** — never silently dropped. Nothing is trusted: not JSON, not coordinates, not question numbers, not marks, not the continuation flag.

Rust has ground truth available that the AI doesn't: page count, expected question-number sequences, `(Total for Question N is M marks)` footers, image dimensions, and the DB's existing state. Today almost none of it is used to check the AI.

---

## 1. Verified critical bugs on `master` (reproduced, not hypothetical)

### 1.1 The crop-overflow panic is STILL live — `commands.rs:1230-1236` (and duplicate at 2087-2093)

The code clamps with `saturating_sub` on the *origin*, but the clamp on the *size* underflows:

```rust
let safe_width  = (w + (x - safe_x) + padding).min(width - safe_x);   // width - safe_x underflows when safe_x > width
let safe_height = (h + (y - safe_y) + padding).min(height - safe_y);
```

If the AI returns pixel-integer bboxes (`[100, 150, 600, 400]`) instead of relative `[0.06, 0.06, 0.36, 0.17]` for a 1654×2339 render:

```
x = 100 * 1654 = 165_400   safe_x = 165_360
width - safe_x  →  u32 subtraction overflow → PANIC in debug builds.
In release: wraps to ~4.29e9 → imageops::crop receives an out-of-bounds rect → panic/garbage.
```

Your earlier brief flagged this as "the most likely source of future runtime errors" — confirmed: the intent to clamp is there, the clamp itself overflows. **There is also zero format validation:** `[x, y, w, h]` vs `[x1, y1, x2, y2]` ambiguity, out-of-range values, degenerate boxes (w/h = 0), and full-page boxes (~90%+ of the page = almost certainly a misdetection) are all accepted verbatim.

### 1.2 `fix_json_escapes` silently mangles LaTeX — `commands.rs:658`

The question-paper path uses a fixer that keeps "valid" JSON escapes (`\n \t \f \r \b \u`). But the most common LaTeX commands *start with exactly those letters*. Wire output `{"content":"$\nabla$"}` (single backslash — the exact thing the fixer exists to repair) parses as:

```
$\nabla f$        →  $<newline>abla f$
$\tan \theta$     →  $<tab>an <tab>heta$
$\frac{1}{2}$     →  $<form-feed>rac{1}{2}$
$\times$          →  $<tab>imes$
x \to y           →  x <tab>o y
```

Verified with an exact port of the function (`scratch/bug_verify`). It fixes nothing in this class and *corrupts silently* — hits `\nabla \nu \notin \ne \tan \theta \times \to \frac \rho \right \beta \binom …`. The mark-scheme path (`commands.rs:1858-1891`) has a partial special-case list; **the question-paper path has none**. Even the MS list can't solve this in general — once broken JSON is on the wire, `"\n"` is *ambiguous* (newline vs `\nabla`). The correct fix is at the boundary: refuse/round-trip broken JSON instead of guessing (§3.3).

### 1.3 Silent page drops — `commands.rs:1038, 1077, 1081, 1853, 1941, 2136`

* Every API/JSON error path does `continue;` — a page that errors, or returns unparseable JSON after the salvage loop, is **silently skipped**. The questions on it vanish with no UI signal.
* Let alone the 10-second single retry on 429 in the question path (MS path does 20s×3 — inconsistent), one transient error = a permanently missing page.

### 1.4 Heuristics second-guessing a hallucinating model

* **Merge logic** (`commands.rs:1267-1280`): two objects the AI labels with the *same* number are merged — so a single hallucinated duplicate number welds two unrelated questions into one card.
* **AQA number strings**: `question_number: "03.1"` → digit filter → `"031"` → **question 31**. No range/monotonicity check exists to catch it.
* **Question 0**: a missing/garbled number becomes `question_number = 0` and is inserted into the DB as-is (`commands.rs:1180-1192`). Mark-scheme matching (`q_by_number`) then maps answer 0 onto the *first* such card.
* **Duplicates on re-import**: nothing enforces the composite-key idea from your brief. `INSERT INTO questions …` with a fresh UUID every run → re-importing the same paper inserts every question again; and the continuation `UPDATE` path (append with `\n\n`) can append already-stitched content *again*.
* **Mark-scheme window dedup** (`commands.rs:2120-2135`): the "fingerprint" is the first 20 whitespace tokens, lowercased. Two overlapping batches transcribing the same question with *any* wording difference → the answer is stitched **twice** into one card. Identical intros on different questions → a real answer silently discarded.

### 1.5 The mega-prompt problem

The per-page system prompt (`commands.rs:876-993`, ~120 lines) asks the model to simultaneously: transcribe, segment, number, merge/split, track continuations, tag topics/module, count marks, format LaTeX/Markdown **and** emit relative bboxes — in one shot, per page, with cross-page state injected as one sentence of context. Multi-task single-shot prompting is the classic setup for exactly your symptoms: inconsistent IDS, dropped rules, mode collapse on long pages. The fix is not more rules — it's **fewer responsibilities per call** (§3.1).

---

## 2. Design principle

**PVRV: Propose → Validate → Repair → Verify.** Every AI call is wrapped in the same deterministic harness:

```
┌────────┐   proposal    ┌──────────────┐  pass   ┌────────────┐
│ AI call│ ────────────▶ │ Rust validator│ ─────▶ │ accept      │
└────────┘               └──────────────┘         └────────────┘
                                │ fail
                                ▼
                       repair prompt (validator
                       errors quoted verbatim),
                       max 2–3 attempts
                                │ still failing
                                ▼
                       QUARANTINE: page/question flagged
                       in the import report. NEVER silent.
```

The AI can hallucinate all it wants — nothing unvalidated reaches the DB, and every failure is visible to you.

---

## 3. The five-phase plan

### Phase 1 — Deterministic Document Map (structure pass)

**Stop asking the AI to segment while it transcribes.** Build the document's skeleton *first*:

1. **Rust scan of the text layer** (where it exists): Edexcel prints `(Total for Question N is M marks)` once per question, deterministically, plus `Question N continued` and `TOTAL FOR PAPER IS X MARKS`. Regex what you can: expected question numbers, per-question expected marks, end-of-paper index.
2. **Where the text layer is corrupt, a cheap AI structure pass**: one call per page, `max_tokens ≈ 150`, schema = `{ "question_numbers_visible": [int], "starts_continuation_of": int|null, "total_marks_footer": int|null, "page_role": "QUESTION|BLANK|COVER|ANSWER_BOOKLET|REFERENCE" }`. Small output = far less hallucination surface, and trivially verifiable:
   - numbers must be in the page's expected range;
   - the sequence across pages must be non-decreasing;
   - marks footers must agree with the text-layer regex where both exist.
3. Rust builds `QuestionSpan { number, start_page, end_page, expected_marks }[]` — **the map**. Topic-list, module list, everything else stays as is.

This reuses your existing two-stage firewall; it just also *remembers what the firewall learned*.

### Phase 2 — Extraction against the map, with a repair loop

Per valid page, the AI transcribes (your existing prompt body, minus all numbering/merging responsibility — it no longer invents `question_number`; it gets the **expected span(s) for this page** from the map and must assign content to *those*):

- Response must echo `page_index` and only use span numbers from the map; anything else → validation failure.
- Content validators (all deterministic, all cheap):
  - ends with terminal punctuation or a marks tag (`**[N marks]**`) — the anti-truncation rule, enforced in Rust;
  - marks consistency: Σ marks tags in the span == `expected_marks` from the map (when known);
  - no excluded boilerplate (your existing post-hoc regexes run *before* validation, not after);
  - **JSON is never guessed at** (§3.3).
- Failure → repair prompt: *"Your previous response failed validation: [validator errors quoted]. Regenerate the full corrected JSON."* Max 2–3 attempts, then the page goes to the quarantine report with the raw output attached for debugging. **The word `continue` disappears from error paths.**

Structural bonus: with spans known, you can feed the AI *one question's pages at a time* (span-batched) instead of blindly per-page — smaller context per call, no cross-question bleed, and `is_continuation` ceases to exist as a concept. (Keep 1 request per *span*, honoring your batching concern.)

### Phase 3 — Bulletproof geometry & JSON boundaries

**3.1 BBox sanitizer** — a single pure function, unit-testable, used by *all three* crop sites:

```rust
/// Accept any bbox format, normalize to pixel rect, or reject. Never panics.
fn sanitize_bbox(b: &[f32], img_w: u32, img_h: u32) -> Option<(u32, u32, u32, u32)> {
    if b.len() != 4 || b.iter().any(|v| !v.is_finite()) { return None; }
    let vals = [b[0].abs(), b[1].abs(), b[2].abs(), b[3].abs()];
    let max = vals.iter().cloned().fold(0.0f32, f32::max);

    // 1) Detect scale: 0–1.5 relative; ≤100 percent; ≤ image dims → pixels; else garbage.
    let scale = if max <= 1.5 { 1.0 }
        else if max <= 100.0 { img_w.max(img_h) as f32 / 100.0 }
        else if max <= img_w.max(img_h) as f32 { img_w as f32 / img_w as f32 } // pixels: pass through
        else { return None; };

    // 2) Detect [x,y,w,h] vs [x1,y1,x2,y2]: prefer the reading that gives positive extents.
    let (mut x0, mut y0, mut x1, mut y1) = (b[0], b[1], b[0] + b[2].abs(), b[1] + b[3].abs());
    if b[2] < b[0] || b[3] < b[1] { x1 = b[2].max(b[0]); y1 = b[3].max(b[1]); x0 = b[0].min(b[2]); y0 = b[1].min(b[3]); }

    let px = ((x0 * scale).clamp(0.0, img_w as f32) as u32).min(img_w.saturating_sub(1));
    let py = ((y0 * scale).clamp(0.0, img_h as f32) as u32).min(img_h.saturating_sub(1));
    let pw = (((x1 - x0) * scale) as u32).min(img_w - px);
    let ph = (((y1 - y0) * scale) as u32).min(img_h - py);

    // 3) Plausibility gates
    let area_frac = (pw as f32 * ph as f32) / (img_w as f32 * img_h as f32);
    if pw < 12 || ph < 12 { return None; }                 // degenerate speck
    if area_frac > 0.92 { return None; }                   // "the whole page" = misdetection
    if area_frac < 0.0005 { return None; }                 // noise
    Some((px, py, pw, ph)) // + your existing is_blank_or_grid() check on the crop
}
```

This one function permanently retires the crash class *and* the wrong-crop class, regardless of what scale or ordering the model invents.

**3.2 JSON boundary discipline**, in order of preference:
1. **Delete `fix_json_escapes` from the question path.** First parse verbatim: strict providers (`response_format: json_object`, OpenAI Structured Outputs) produce valid JSON ~always; the current fixer only *maul*s the rare broken case into silent corruption.
2. On parse failure: **round-trip the error to the model** — "Your output was not valid JSON: {serde error}. Return the same content as valid JSON. Escape backslashes as `\\`." The model that wrote the content resolves ambiguities a regex never can (is `\n` a newline or `\nabla`? Ask the author, don't guess).
3. Keep `auto_close_json` as last-ditch salvage for *truncation only*, but replace the pop-one-char × 2000 loop (O(n²), can silently truncate real content) with: find the last `}` that closes a *complete element* of `extracted_questions`, truncate there, close the array+object. Recover complete items only — and mark the page `truncated: true` in the report so you know a repair happened.

### Phase 4 — Deterministic assembly, checksums, and a visible report

Assembly becomes pure Rust arithmetic:
- Stitching = "append spans in map order". The `is_cont`/`should_merge` heuristic block (1267-1280) is deleted.
- Marks: span-level check — extracted Σ vs `expected_marks`; mismatch → one repair attempt → flagged `needs_review` on the card. Paper-level checksum: Σ question marks vs `TOTAL FOR PAPER` when available.
- **End-of-import report** (new Tauri event / returned struct): pages processed, pages quarantined (with page numbers + reasons), questions extracted vs expected, per-question mark checks, #crop rejections, #repairs. This converts today's silent failures into a checklist you actually see in the UI.

### Phase 5 — Idempotent DB & regression harness

- **Schema**: `CREATE UNIQUE INDEX IF NOT EXISTS ux_paper_q ON questions(paper_name, question_number)` (+ a one-off dedupe migration keeping the latest rowid). Writes become upserts (`INSERT .. ON CONFLICT(paper_name, question_number) DO UPDATE`) — re-imports become *idempotent refreshes* instead of duplicate generators.
- **Tests without the API**: the reqwest layer goes behind a tiny trait (`trait LlmClient { async fn chat(&self, body) -> Result<String> }`); a `MockLlm` replays recorded responses from golden files (`tests/fixtures/*.json`). Then `cargo test` covers: pixel bboxes, `[x1,y1,x2,y2]` bboxes, truncated JSON, `\nabla` JSON, wrong question numbers, duplicate numbering, missing pages — *once written, these regressions can never come back, on any model, forever.* This is the "regardless of how the AI behaves" guarantee embodied in code.

---

## 4. Sequencing (highest value per hour first)

| # | Work item | Effort | Kills |
|---|-----------|--------|-------|
| 1 | BBox sanitizer (§3.1) wired into all 3 crop sites | ~½ day | crop panics, wrong crops |
| 2 | Kill `fix_json_escapes`; parse-verbatim + JSON repair-reprompt; salvage truncation-at-item-boundary | ~½ day | silent LaTeX corruption |
| 3 | Remove all error-path `continue`s → quarantine + end-of-import report event | ~½ day | invisible page loss |
| 4 | UNIQUE(paper_name, question_number) + upserts + dedupe migration | ~½ day | duplicates, re-import corruption |
| 5 | Structure pass → Document Map (§1/Phase 2); AI stops assigning numbers | 2–3 days | wrong segmentation, merge bugs, Q0/Q31, continuation misfires |
| 6 | `LlmClient` trait + golden-file regression suite | 1–2 days | every past bug, permanently |
| 7 | Span-batched extraction; delete `should_merge` heuristics; mark-scheme windowing replaced by span stitching (retire the 20-word fingerprint) | 2–3 days | cross-question bleed, double-stitched answers |

Items 1–4 are surgical and can ship independently. Items 5–7 are the structural change that makes correctness model-independent.

---

## 5. What stays (it's good)

- One-page-per-call discipline and your rate-limit posture (make it consistent: same backoff everywhere).
- The two-stage firewall concept — extended into the Document Map.
- Topic allow-lists + post-hoc exact-match filtering (`get_allowed_topics` containment check) — already the right instinct: *deterministic containment, not trust*.
- `is_blank_or_grid` crop guard — keep, run *after* the sanitizer.
- Hybrid text layer as a *hint* to the model — keep, but let the map be the authority.

---

## 6. Addendum — the diagram audit (2026-07-16)

**Regression observed:** AQA CS June 2024 Paper 1 Q30 (an empty student trace
table) produced ~10 near-identical PNG crops of the *same empty ruled grid*.
The soft prompt rule ("do not box empty grids") was ignored by the model, and
`is_blank_or_grid` cannot see ruled grids — the rules and header text are real
ink, so variance stays high. Nothing deduplicated the saves, and the model was
never told its boxes were wrong.

**Fix (PVRV applied to diagrams — the AI proposes boxes, Rust decides):**

- `geometry::crop_diagram` now returns `Result<_, CropReject>`:
  `BadBox` (sanitizer), `Blank` (luma guard), or `AnswerGrid`
  (`looks_like_answer_grid`: ≥4 long horizontal rules + ≥2 long vertical
  rules + ≥80% empty inter-rule bands — a structural test that sees through
  header text). Filled Gantt figures, charts, and populated tables pass.
- `pipeline::audit_diagram_boxes` runs *inside* the repair loop of both
  question-extraction paths. Every proposed box is cropped and checked
  (sanitizer → blank → answer-grid → duplicate-signature) **before** the
  response is accepted; violations are quoted back to the model as repair
  feedback ("the box covers an empty ruled answer grid — transcribe it as a
  Markdown table, keep any pre-filled cells"). When the repair budget is
  spent, the offending boxes are pruned deterministically, counted in
  `crop_rejections`, and recorded in `anomalies` — nothing dangles and
  nothing is silent.
- `tile_signature` (8×8 block-mean luma) + `signature_distance`: identical
  crops are detected both at audit time ("identical image to box #N") and at
  save time (`save_diagram` reuses the existing file link instead of writing
  yet another PNG; counted in the new `diagrams_deduped` report field).
  Point-sample hashes were considered and rejected — sparse line art
  collapses them to all-zeros.
- Prompts (span extraction + fallback) now state the rule the parser
  enforces: structured tables with headers (trace tables, function tables,
  working grids) are question content **even when empty** — always Markdown
  tables, never diagram boxes; one box per figure; blank/empty-grid/duplicate
  boxes are rejected and cost a repair round.

Golden tests pin the regression: audit verdicts on synthetic trace-table /
chart / duplicate fixtures, repair-loop recovery (bad boxes → quoted feedback
→ model returns a Markdown table), deterministic pruning after budget spent,
and save-time dedupe (one PNG, one reused link, grid rejected).
