# Phase 4: integration and certification

Prerequisite: Astra accepts phase 3. Same worker, workspace and ownership boundaries in PLAN.md. Compare changes against the accepted phase-3 snapshot; preserve all earlier dirty work.

## Verification contract

1. Run `cargo test --manifest-path src-tauri/Cargo.toml` in full, saving exit status and complete log. Report ignored tests and environment limitations separately. Targeted suites alone do not satisfy this gate. Fix relevant regressions, then rerun affected tests and the final integration gate as needed.
2. Inventory every named PDF in `docs/ZERO_COST_ORCHESTRATION_PROMPT.md`, resolving abbreviated names explicitly. Do not silently replace an absent file with another paper. Run the offline refusing/counting client across every available named paper without credentials or production DB. Use fresh outputs and enforce child exit codes/artifact freshness.
3. Produce a machine-readable and human-readable certification table: file identity; digital classification; expected/source question identifiers and extracted identifiers; strict, recovered, placeholder/quarantined counts; review flags; marks anomalies; attempted cloud and image requests; prompt/completion tokens; exact cloud cost; rendering failures. Report denominators and weighted strict acceptance across the historical corpus. Missing files and unmapped source questions must be visible, not excluded as successes.
4. Physics 2021 and 2024 must have complete question coverage, zero quarantine, zero cloud attempts, zero vision fallback, zero prompt/completion tokens and $0.0000 cost. Run `node scripts/verify-physics24-render.mjs <fresh-physics24-cards.json>` and retain its existing scientific/structural assertions.
5. Render all corpus cards through the actual frontend preprocessing, Markdown and KaTeX stack. Record syntax errors, malformed math-wrapped table delimiters and contextual debris failures. Include representative CS code, matrices, nuclides and tables. Any additional checker should inspect real output and fail on errors rather than silently tolerate/strip content.
6. The historical strict Tier-0 rate must be >=95%, targeting 100%. Recoveries remain separate. Correct observed in-scope defects using source evidence and regression tests; do not remove quality gates, invent marks/content or reclassify uncertainty to meet the threshold. If a concrete local extraction capability is unavailable, report exact source evidence and the missing capability for root decision.

## Deliverable

Update `phase-4-report.md` in place with exact commands/exits, artifacts and corpus rows, changes during integration, ignored/missing checks, remaining limitations, and reproducible verification instructions. Return ready_for_review. Root owns final acceptance; do not claim it yourself. Never delete a report before rewriting it. No cloud ingestion, paid probes, provider changes, dependency changes without necessity/root resolution, staging, commits, pushes, deployment or production data changes.
