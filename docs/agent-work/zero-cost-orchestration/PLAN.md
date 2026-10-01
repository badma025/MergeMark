# Universal zero-cost digital ingestion

Authority: `docs/ZERO_COST_ORCHESTRATION_PROMPT.md`. This plan supersedes the older Tier-0 plan's cloud fallback, table exclusion, and limited corpus target. Astra owns contracts and acceptance; one native Flash writer implements each dependency-ordered phase.

## Baseline and ownership

Workspace: `C:/Users/alimb/Documents/trae_projects/MergeMark`.
HEAD: `52560233abcf779bc8bd2473ef09c8b84de0e3d7`; substantial pre-existing uncommitted work is part of the input. Baseline source copies, tracked/staged patches, and inventory: `output/zero-cost-orchestration-baseline/`. Compare against those copies, not HEAD alone. Preserve existing edits and deleted golden fixtures.

Worker scope: ingestion modules in `src-tauri/src/`, parser modules and their tests in `src/lib/`, relevant offline fixtures in `src-tauri/fixtures/`, verification scripts in `scripts/`, and new run artifacts under `output/zero-cost-orchestration/`. Dependency-manifest changes require a concrete necessity and root resolution. Astra owns this plan and CHECKPOINT.md. Worker reports go to `docs/agent-work/zero-cost-orchestration/phase-N-report.md`.

## Architectural contracts

1. **Digital means local.** Classify usable PDF text independently of whether question mapping passes. A weak map or one failed span must never turn an otherwise digital document into a cloud request. Digital question ingestion permits zero LLM requests at every stage: structure, extraction, batch fallback, crops, repair, and classification. Enforce this at the orchestration boundary and the request boundary as defense in depth. Existing tuning switches must not silently re-enable cloud for digital documents. Keep scanned-only behavior compatible; explicitly test mixed and ambiguous documents and report any unresolved source pages. Local PDF rendering/cropping remains permitted.
2. **Honest recovery.** Run deterministic extraction and conservative local repair. Retain nonempty, isolated best candidates with existing review/anomaly signals when a quality gate fails. Never invent question content, marks, or boundaries; never suppress flags or call all recovered cards strict Tier-0 successes. If no viable candidate exists, return an explicit local failure. Zero cost and extraction completeness are separate metrics.
3. **Existing seams.** Reuse `PageInput`, `QuestionSpan`, `AiQuestionPage`, `BuiltQuestion`, `ImportReport`, the deterministic transcriber, and the existing assembly/sanitizer path. Keep serialized additions backward compatible. Any policy representation must be explicit and checked before network activity; new names are worker discretion within this contract.
4. **Stitch only one question.** At a single-question response boundary, merge compatible subpart items in source order, preserving content, diagrams, topics, and marks without duplicate totals. Validate question identity before stitching. Distinct parent questions and batch responses stay separate; ambiguous identities remain validation failures. Stitching is local and useful for existing scanned/cloud paths too.
5. **Conservative normalization.** Accept terminal mark tags, complete formulas, removed answer blanks, and genuine END OF QUESTIONS boundaries. Repair minor math delimiters before validation while preserving escaped dollars, code, existing display math, and tables. Recognize coherent MCQ runs, isotope contexts, and real tabular rows. Avoid globally deleting standalone variables/units or treating prose A-D as options. Regression tests must include counterexamples and idempotence where meaningful.
6. **Measured certification.** Use an offline refusing/counting LLM client, without API credentials or production DB access, for corpus validation. Any attempted request fails certification even when reported usage is zero. Report real prompt/completion tokens, request/vision counts, strict Tier-0 count, local recovery count, expected/extracted numbers, marks, review/quarantine counts, and rendering failures. Missing files or skipped tests are explicit limits, never passes. Do not infer $0 from rounded nonzero cost or replace usage telemetry with constants.

Phase-1 review clarification: short digital papers, blank/end pages, and mixed or ambiguous documents with meaningful question text must stay local. Only genuinely scanned-only inputs retain cloud compatibility. Provider credential/quota resolution and upload debits are unnecessary for local-only question imports and must be bypassed. Mark schemes remain outside this question-paper scope. Equal subpart marks are not evidence of a duplicated question total; preserve them unless source/expected-total evidence proves duplication. Strict-success siblings in a partial batch retain their strict status.

## Four phases

| Phase | Dependency | Bundle and acceptance |
| --- | --- | --- |
| 1 | Baseline | Enforce the digital cloud circuit breaker across production question ingestion; implement safe single-question subpart stitching; add request-counting regressions and an offline e2e mode. Targeted Rust tests pass, including weak maps, failed local gates, batch paths, and distinct-question stitch rejection. |
| 2 | Astra accepts phase 1 | Repair terminal-ending gates, safe delimiter balancing, and final-question boundaries. Targeted tests demonstrate retained content and correct gate reasons; physics 2021 Q6 and final MCQ boundaries have regression coverage. |
| 3 | Astra accepts phase 2 | Generalize MCQ, isotope, tables, and contextual debris cleanup across Rust and frontend. Positive and negative fixtures pass; preserve code, legitimate units, question identities and source equations. |
| 4 | Astra accepts phase 3 | Run full Cargo suite, all available named corpus PDFs in both corpus directories, physics 2021/2024 offline e2e, and `node scripts/verify-physics24-render.mjs <physics24-cards.json>`. Fix in-scope failures and produce a reproducible per-paper certification report. |

## Final acceptance

- `cargo test --manifest-path src-tauri/Cargo.toml` passes; report ignored/environment-skipped checks separately.
- Every available specified digital corpus PDF attempts zero cloud calls, reports exactly 0 prompt and completion tokens, and therefore incurs $0.0000 ingestion cloud cost.
- At least 95% strict Tier-0 acceptance on the historical corpus, targeting 100%; best-effort local recoveries reported separately.
- Physics 2021 and 2024 have complete question coverage, zero quarantine and zero vision fallback calls.
- Physics 2024 render verifier passes all cards; corpus typesetting has no KaTeX crashes or math-wrapped table delimiters, and contextual debris regressions pass.
- Root reviews actual changes against the dirty baseline for specification compliance and quality/security. Required checks that fail or cannot run prevent full acceptance.

## Execution constraints

One shared-workspace writer; preserve others' edits. No recursive agents, commits, staging, pushes, deployment, paid ingestion tests, credentials, or production DB mutations. Worker owns discovery, implementation, tests and debugging in scope. Report genuine blockers and leave a resumable checkpoint; ordinary implementation decisions need no approval.

Routing doctor: static-ready; root `gpt-6-astra`, worker `openrouter/deepseek-v4.1-flash`, OpenRouter, high effort. Actual upstream inference metadata is pending; no setup smoke test was run. Development-model usage is distinct from the ingestion $0 guarantee.
