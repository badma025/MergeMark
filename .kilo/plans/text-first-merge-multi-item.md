# Plan: Merge multi-item text-first responses (fix the $0.06 cost spike)

## Goal
Stop large multi-page questions (e.g. Q2 on physics '24) from falling back to the
expensive vision repair loop when the text-first model returns several items for the
same question. Expected on physics '24: ~31 text-first / ~3 full-page vision,
~$0.015-0.025 per import, and no run-to-run cost spikes.

## Root cause
`try_text_first_extraction` (src-tauri/src/pipeline.rs) rejects any text-first response
whose target items are not EXACTLY one:

    if target_items.len() != 1 {
        eprintln!("[TEXT_FIRST_FALLBACK] question={} reason=item_count items={}", ...);
        return None;
    }

Large questions (Q2 spans 4 pages, references Figures 1-4) are frequently returned SPLIT
across sub-parts - 4 items for one question number. The gate rejects them, so the question
falls back to the full-page vision path, which for Q2 means 4 images per call plus repair
rounds (observed `[DIAGNOSTIC][DIAGRAM_AUDIT_ERROR] ... diagram 4: degenerate` printed
twice). That single fallback is what pushed the import from ~$0.03 to $0.062.

The vision path already handles this case: `merge_split_questions` (pipeline.rs:540)
merges multiple items for the same question number. Text-first must do the same.

## Change (one function: src-tauri/src/pipeline.rs)
In `try_text_first_extraction`, replace the acceptance-gates block (from the
`// Text-first acceptance gates` comment through the `validate_span_items` check) with a
merge-tolerant version:

1. Reject only when `target_items.is_empty()` (drop the `!= 1` requirement).
2. Reject when ALL items have empty content (instead of checking only item[0]).
3. Count `[DIAGRAM_PLACEHOLDER]` across ALL items (sum), compare to `available_figures`.
4. Reject when ANY item has non-empty `diagram_bboxes` (instead of only item[0]).
5. Keep `validate_span_items` on the wrapped page (already covers all items).

The accumulation loop below (contents/topics/marks) already iterates `target_items` and
merges them into one `BuiltQuestion` - no change needed there.

Exact replacement block:

    // All items for the target question. Large multi-page questions often come
    // back SPLIT across sub-parts; the vision path merges those, so text-first
    // does too - a fragile "exactly one item" gate is what dumped Q2 into the
    // expensive vision repair loop on this run.
    let target_items: Vec<AiQuestion> = page
        .items
        .into_iter()
        .filter(|i| {
            i.question_number
                .as_ref()
                .and_then(validate::value_to_question_number)
                == Some(span.number)
        })
        .collect();
    if target_items.is_empty() {
        eprintln!("[TEXT_FIRST_FALLBACK] question={} reason=no_target_items", span.number);
        return None;
    }
    if target_items
        .iter()
        .all(|i| i.content.as_deref().unwrap_or("").trim().is_empty())
    {
        eprintln!("[TEXT_FIRST_FALLBACK] question={} reason=empty_content", span.number);
        return None;
    }
    let placeholder_count: usize = target_items
        .iter()
        .map(|i| i.content.as_deref().unwrap_or("").matches("[DIAGRAM_PLACEHOLDER]").count())
        .sum();
    if placeholder_count > available_figures {
        eprintln!(
            "[TEXT_FIRST_FALLBACK] question={} reason=figure_needed placeholders={} figures={}",
            span.number, placeholder_count, available_figures
        );
        return None;
    }
    if target_items
        .iter()
        .any(|i| i.diagram_bboxes.as_ref().is_some_and(|b| !b.is_empty()))
    {
        eprintln!("[TEXT_FIRST_FALLBACK] question={} reason=unexpected_bboxes", span.number);
        return None;
    }
    let wrapped = AiQuestionPage {
        items: target_items.clone(),
    };
    if !validate_span_items(&wrapped, span).is_empty() {
        eprintln!("[TEXT_FIRST_FALLBACK] question={} reason=validation", span.number);
        return None;
    }

Also bump the build marker in commands.rs (parse_pdf_vision) to figure-fix-v5 so the new
binary is verifiable: `[BUILD] figure-fix-v5: text-first merges multi-item responses`.

## Tests
Add `text_first_merges_multi_item_response` (pipeline.rs tests): mock returns TWO items
with the same question_number; assert:
- `report.text_first == 1` (one success, no vision fallback),
- built content contains both parts,
- marks summed.

Existing text-first tests stay green (single-item responses still pass the same gates).

## Validation
1. `cd src-tauri && cargo test --lib -- --test-threads=1`
2. `cargo check --all-targets` (no warnings), `npx tsc --noEmit`
3. Manual: import physics '24 with MERGEMARK_LOG_USAGE=1. Expect the `[BUILD] figure-fix-v5`
   marker, `[PATH_SUMMARY]` ~31 text-first / ~3 full-page vision, cost ~$0.02.

## Out of scope (separate follow-up)
Q8 / Q24 / +1 reference figures by word only ("the diagram", "the graph") with no
"Figure N" caption, so `fig_count = 0` and they stay on vision. Relaxing the vertical band
for unnumbered figures on single-page spans would capture them; minor residual (~$0.003 each).
