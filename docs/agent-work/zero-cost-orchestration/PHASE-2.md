# Phase 2 implementation brief

Prerequisite: root acceptance of phase 1, recorded in CHECKPOINT.md. Same worker and workspace ownership as PLAN.md. Authoritative specification remains `docs/ZERO_COST_ORCHESTRATION_PROMPT.md`.

## Outcome

Heal local terminal-ending, delimiter, marks-preservation and question-boundary failures while preserving the accepted zero-cloud policy. A legitimate question should pass strict Tier-0 after a justified local repair; uncertain or incomplete extraction retains review/failure status.

## Work

1. Accept terminal mark tags (`**[3 marks]**`, `[2]`), complete formula endings, and endings exposed when answer blanks are removed. Preserve rejection of genuinely cut-off prose. Apply consistent terminal semantics at the deterministic gate and assembly/validation seams so a repaired card is not re-flagged downstream.
2. Balance minor unescaped single-dollar defects before `unbalanced_math` evaluation. Preserve code spans/blocks, escaped currency, existing paired math/display math, Markdown pipe tables, and equation content. Test idempotence and malformed cases that remain uncertain.
3. Fix local span carving for final questions before END OF QUESTIONS, footerless question endings, and the Edexcel/madas heading layouts already detected by the document map but rejected by the carver. Share structural evidence where possible; do not suppress boundary checks or derive expected counts from the output being certified. Include first-page questions and sparse/short papers in regression coverage.
4. Fix the reported `marker_client::fix_mark_allocation` single-part/single-tag deletion (leaves `****`). Keep legitimate equal-valued subpart tags and printed totals intact. Add direct and pipeline regression evidence so the same content is not damaged by later sanitization.

## Verification and completion

- Targeted Rust tests for each repair and its negative case pass, including existing affected suites.
- Rebuild the offline e2e harness and inspect physics 2021 Q6, its final question, core pure 1 2021 and madas paper 2 boundaries. Report strict/recovered/quarantine changes, actual extracted question identities and retained source content, not just counts.
- Cloud attempts and both token counters remain exactly zero. Preserve physics 2024's 32 strict cards.
- Leave remaining isotope/MCQ/table/font-glyph failures for phase 3 with exact failing source examples; do not hardcode corrections keyed to paper name or question number.
- Full Cargo suite and full-corpus certification are phase 4; avoid repeated broad checks unless a concrete failure justifies them.
- Write `phase-2-report.md` with changed paths, commands/exits, targeted evidence and remaining gate reasons. Return ready for root review before phase 3.

Keep dependencies, mark-scheme behavior, provider settings and production data unchanged. No paid ingestion, commits, deployment, recursive agents, or test weakening.
