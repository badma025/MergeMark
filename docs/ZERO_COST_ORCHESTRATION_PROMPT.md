# Astra-Flash Orchestration: Universal Zero-Cost ($0.0000) Ingestion Across All Exam Papers

### Role & Workflow Mandate
- **Astra (Root Architect & Reviewer)**: Establishes the contracts, manages the phased plan, enforces zero-cost architectural boundaries, and conducts batched acceptance reviews.
- **Flash (`astra_flash_builder`)**: Executes repository discovery, implementation, testing, debugging, and verification across the Rust backend (`src-tauri`) and frontend parser.
- **Strict Rule**: No cloud AI dependencies on digital PDFs. Total wean-off to achieve true **$0.0000 cost (0 prompt tokens, 0 completion tokens)** on all born-digital papers.

---

### Mission & Context
In recent runs on historical papers (e.g. `physics '21`), the ingestion engine burned **$0.0312** (47k+ tokens) because a single question (Q6) failed Tier-0 with `no_terminal_ending`, failed text-first schema validation because the model returned 3 separate items for subparts `(a)`, `(b)`, `(c)`, and escalated into a catastrophic **Full-Page Vision repair loop** across 3 high-resolution multi-page turns.

We must eliminate this failure mode permanently and lift deterministic (Tier-0) extraction to **100% on-device acceptance** across all exam boards (AQA, Pearson Edexcel, OCR, Cambridge CIE) and subjects (Physics, Mathematics, Further Maths, Computer Science).

---

### Test Corpus Scope
The implementation must be validated across the repository's test corpora:
1. **`past papers for mergemark/`**:
   - `physics '21.pdf`, `physics '24.pdf`
   - `core pure 1 '21.pdf`, `'22.pdf`, `'23.pdf` (Edexcel Further Maths)
   - `fp2 '24.pdf`, `further mechanics 1 '21, '22, '23.pdf`
   - `aqa gcse further maths '24.pdf`, `aea2024.pdf`
   - `computer science 2 '22.pdf`, `'23.pdf`, `'24.pdf`
   - `January 2025 QP.pdf`, `Nov 2024 QP.pdf`, `naikermaths_paper_m16_pure.pdf`, `madas_paper_2_t.pdf`
2. **`test_papers/`**:
   - `01_edexcel_maths_p1_qp.pdf`
   - `03_aqa_physics_p1_qp.pdf`
   - `04_cie_maths_9709_p1_qp.pdf`
   - `07_legacy_c3_or_c4_qp.pdf`

---

### Phased Architectural Contracts

#### Phase 1: Hard Vision Circuit Breaker & In-Memory Auto-Stitcher
- **Hard Vision Circuit Breaker**: Born-digital PDFs with accessible text layers must **never** escalate to `FullPageVision` or trigger multi-page image repair loops. If text-first fails validation, repair must occur text-only (0 image tokens) or accept the best candidate flagged locally.
- **In-Memory Subpart Auto-Stitcher**: When an extraction call for a single question returns multiple array items (one per subpart `(a)`, `(b)`, `(c)`), the pipeline must automatically concatenate them in memory into a single structured question card rather than throwing a schema validation error.

#### Phase 2: Universal Tier-0 Gate Healing & Terminal Ending Resilience
- **`no_terminal_ending` Resilience**: Relax rigid punctuation checks in Tier-0. Questions ending in mark tags (e.g. `**[3 marks]**`, `[2]`), formulas, or lines following stripped answer blanks must be accepted cleanly.
- **Auto-Balancing Math Delimiters**: Run local delimiter balancing before evaluating `unbalanced_math` so minor single-dollar discrepancies in the text layer are repaired without falling back to the LLM.
- **Boundary Detection**: Ensure the final question on a paper (e.g. Q31) reliably captures its boundary before "END OF QUESTIONS" without triggering `boundary_not_found`.

#### Phase 3: Multi-Board Syntax, Isotope & MCQ Generalization
- **MCQ Option Tagging**: Generalize the deterministic option separator to automatically identify plain letter choices (`A ...`, `B ...`, `C ...`, `D ...`) across AQA and Edexcel Section B and format them into `- [MCQ:A]` through `- [MCQ:D]`.
- **Nuclide & Isotope Reconstruction**: Reconstruct stacked plaintext nuclide notation (e.g. `10 D 2` $\to$ $^{10}_{2}\text{D}$, `238 92 U` $\to$ $^{238}_{92}\text{U}$) locally before validation.
- **Markdown Tables**: Ensure tabular data extracts into clean Markdown pipe tables (`| --- | --- |`) and never wraps table delimiters in math mode (`$| --- |$`).
- **Zero Debris**: Ensure complete removal of margin barcodes (`*05*`), answer prompts (`left right`, `$= \text{J} 9`), and orphan unit characters (lone `m`).

#### Phase 4: Full Corpus Verification & Zero-Cost Certification
- Validate all unit tests pass: `cargo test --manifest-path src-tauri/Cargo.toml`.
- Execute e2e test imports on `physics '21.pdf` and `physics '24.pdf`:
  - Verify `prompt_tokens = 0`, `completion_tokens = 0`, `cost = $0.0000`.
  - Verify 0 quarantined cards and 0 vision fallback calls.
- Run render verification (`verify-physics24-render.mjs`) to confirm 100% pass on KaTeX typesetting.

---

### Acceptance Criteria & Definition of Done
1. **$0.0000 Cost Guarantee**: Exactly 0 prompt tokens and 0 completion tokens on digital PDFs. Total elimination of cloud vision calls on born-digital papers.
2. **Deterministic Extraction**: $\ge 95\%$ to 100% Tier-0 acceptance across historical test papers in `past papers for mergemark/`.
3. **Flawless Typesetting**: 0 KaTeX syntax crashes, 0 math-wrapped markdown tables, and 0 prompt margin debris.
4. **Test Suite Integrity**: Full Cargo test suite passes 100% green with zero regressions.
