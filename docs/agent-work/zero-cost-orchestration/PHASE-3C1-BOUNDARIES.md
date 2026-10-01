# C1 remaining implementation — boundaries, subparts and marks

Accounting cost dependency accepted (`accepted-phase-3c-accounting`): fresh22papers allattempts/tokens0, $0.0000,14runner+7canonical tests. EntirePhase3 still unaccepted. No new inventory-only deliverable: this bundle IMPLEMENTS the known parser repairs from PHASE-3C1 req3/5.

## Required outcomes

1. Fix Jan2025 DecisionMathsD1 source8questions currently split into12cardIDs with missing5/6 and fabricatedparent10/12/13/16/18/19 from graph/table numbers. Use general heading/layout/sequence/continuation evidence and declared-count cross-checks, never filename-specific ranges or forcing a count by dropping real questions. Preserve all source question text, diagrams and subparts. Actual source8 parentIDs must emerge with complete content.
2. Restore missing parentheadings/boundaries on AQA GCSEFM2024 (source23, missing2/6/14/19/21/22), CIE9709/11 M/J22 (source11, missing4/11), CIE9709/11 M/J14 (source12, missing2/3). LastquestionEND and splitrunheading cases need general source-supported fixes. Validate sourcefixture notes/pages while examining; fixture stays tests-only.
3. Recover MadasQ8/Q9 viable carved content from actual source; resolve boundary/heading issue rather than mislabel every failure as matrix. Preserve existing margin allocations/provenance and actualmarks. No-viable candidate is not acceptable if readable source exists.
4. Repair CS2022/23/24 boxed numbered subparts and code/table content, resolving subpart_gap through preserving/normalizing sourcepartlabels. Keep code opaque end-to-end, no mathification/debris stripping insidecode. Test missing realpart remainsflagged and actual encodedparts recovered; do not simply relax gapgate.
5. Fix Nov2024 GCSEMaths marks_checksum_mismatch from actual source allocations/totals. Resolve continuationpages/nonconsecutiveIDs/END bounds using measured regions; preserve each per-partmark once. Repair contextual answerlines only when actual sourcefurniture evidence supportsit, never arbitrary unit/text deletions.
6. Targeted regression tests and fresh affectedimports; actualfrontend renders/sourcecomparisons for CS/code, marks and boundaries. Report correctquestionIDs, marktotals, strict/recovered/quarantine and zeroattempts. Remaining symbol/font/complexmath failures stayflagged forC2. Do not rerunfull22whileiterating except onefinalsummary ifneeded.

## Accounting carry-forward (small integration fixes)

Do not compute accepted strict numerator from extra/wrong parent IDs. Current rawstrict140 includes any invalidJanparents; report rawstrict separately until IDs match, or count verified strict valid IDs from per-card status. Missing/unresolvedsourcefixture must make verifiedcoverage UNKNOWN (not denominator0); human report unknown tokenfields must print? not0. Reject partially unknown --only selectors (currently only rejects zero matches). FinalPhase4 must failquality acceptance on IDmismatch/unknown, even if costcheckPASS. Cost and content verdicts stayseparate.

## Completion

Own deterministic/docmap/layout/parser/harness-related modules and tests/report perPLAN; preserve others' dirty edits. Finish implemented boundary/part/markrepairs and verification before returning. A list of these same unimplemented tasks is not completion. No cloud/deps/prodDB/staging/commit/push/deploy; omit sandbox_permissions. Source-loss invariants remainbinding. Report lateststatus at top and exactevidence.
