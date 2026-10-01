# R1 root acceptance review

Timebox: 2026-09-28 16:01:06–17:31:06 UTC. Initial submission returned around minute 61. Root interrupted the completed child and reviewed actual code, logs, and card artifacts. No next bundle started.

## Initial disposition: NOT ACCEPTED

Useful implementation: source-heading filters, case-sensitive isotope matching, declared-count restriction narrowed to ambiguous candidates, and final lone-heading handling. Saved doc_map suite:24 passed. Only doc_map changed relative to the paused source snapshot. These facts do not satisfy retained-body/no-new-quarantine requirements.

Concrete findings sent as one correction within the original timebox:

1. `r1f-aqafm24` Q5 and Q22 are zero-mark Needs Review placeholders after boundary_not_found, not recovered question bodies. Q5 fallback includes Q6. Report claiming all named checks pass is inaccurate.
2. `r1e-tp04_cie_maths` Q5 now fails boundary recovery; actual counts are6strict/2recovered/2quarantined, not report's5/3/1. AQA actual counts are8/11/4.
3. CIE Q4 is marked strict with control characters and garbled maths. Mapping improvement must not be counted as verified source-quality success. Complex maths reconstruction is outside R1, but known corruption must remain flagged.
4. `q_cross_reference` rejects any Q-prefixed heading whose continuation starts lowercase, including legitimate `Q1 x=...` or `Q1\nx=...`. Lowercase text alone is insufficient evidence of a margin reference. Require source context and positive/negative tests.
5. New tests primarily assert heading IDs; isolated body/part evidence for the named recovered cards is missing. Require actual carving-seam regressions and fresh affected artifacts.

Correction scope remains R1: necessary doc_map/carver seam and focused tests, plus a narrow source-quality flag if required for known corruption. No complex maths, CS validator refactor, or new bundle. Deadline unchanged. PARTIAL is required if any named check remains unresolved.
