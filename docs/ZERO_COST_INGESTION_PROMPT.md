# Engineering Brief: Driving Exam Paper Ingestion to Zero Cost ($0.00) in MergeMark

### Context & Project Background
**MergeMark** is a desktop flashcard and worksheet authoring application built with **Tauri v2 (Rust backend) + React 19 / TypeScript + SQLite**. It ingests past UK and international exam papers (AQA, Edexcel, OCR, CIE — typically 30–45 page PDFs containing question papers and mark schemes), extracts each question into individual cards with KaTeX math formatting, crops embedded diagrams/charts, pairs them with official mark schemes, and tags syllabus topics.

Over recent optimization sprints, we drove per-paper ingestion costs down from ~$0.50 to **~3.62¢ per paper** (using Gemini 2.5 Flash via OpenRouter) through:
1. **Document Mapping (`src-tauri/src/doc_map.rs`)**: Deterministically scans the PDF text layer for printed whole-number headings and `(Total for Question N is M marks)` footers to construct monotonic question spans before any transcription.
2. **On-Device Figure Detection (`src-tauri/src/stroke_census.rs`)**: Scans `pdfium` vector stroke paths, Bézier curves, and raster images on-device to isolate and crop diagrams locally for **$0.00**.
3. **PVRV Ingestion Seam (`src-tauri/src/pipeline.rs`)**: *Propose → Validate → Repair → Verify*. Bounding boxes, mark allocations, math delimiters (`$...$`), and question IDs pass through strict Rust validation gates; repair loops re-prompt on failure.
4. **Deterministic Content Sanitizer (`src-tauri/src/sanitize.rs`)**: Runs as a last line of defense on all cards, repairing unescaped LaTeX, balancing rogue `$` signs, fixing scrambled isotope notation, and cleaning OCR boilerplate.
5. **Tier-0 Deterministic Extraction (`src-tauri/src/deterministic.rs`)**: Uses a local regex/line-based Rust transcriber to carve text-reliable, figure-free question spans directly from the text layer for **$0.00**. If any confidence gate fails, it falls back to the LLM.

---

### The Objective
**Drive end-to-end import and processing costs to literal zero ($0.00)** so users need no paid API keys and maintainers pay zero server/API bills, **while keeping paper ingestion time under 15–20 seconds**.

---

### Empirical Baseline: Where the Remaining 3.62¢ Goes Today
Based on production telemetry and test diagnostics across fixture papers (AQA Physics 2017–2024, Edexcel Maths, CIE):

| Stage | Current Behavior | Frequency | Share of Remaining Cost |
| :--- | :--- | :--- | :--- |
| **Mark-Scheme Pipeline** | **0% deterministic today.** Every 3-page window is sent to the LLM (`MsTextFirst` or full-page vision `MsWindow`). | 7–13 calls / paper | **~65–70% of total spend** |
| **Figure-Referencing Questions** | Tier-0 currently rejects spans that reference figures (`figure_reference_unsupplied`), escalating them to cloud vision/crop calls. | 4–6 questions / paper | **~15–20% of total spend** |
| **Board Syntax Drift** | Non-AQA boards (Edexcel, CIE) currently score **0%** in Tier-0 due to syntax mismatches (e.g. Edexcel `(3)` vs AQA `[3 marks]`, CIE dotted answer lines). | 100% of non-AQA questions | **100% of cost on non-AQA** |
| **Deferred Topic Tagging** | Batched cloud LLM call classifies topics for Tier-0 cards at import completion. | 1 call / paper | **~5% of total spend** |

---

### Approaches Evaluated & Why They Were Rejected

#### 1. Cloud Free-Tier API Routing (Google AI Studio, Groq, OpenRouter `:free`) — ❌ REJECTED
- **Empirical Failure**: We tested this extensively in practice. Cloud free tiers enforce strict concurrency limits (typically 1–2 concurrent requests, 15 RPM).
- When a 35-page paper dispatches parallel batch requests across spans, free endpoints immediately throw HTTP 429 (Rate Limited).
- Exponential backoff loops cause requests to queue up, triggering MergeMark’s 45-second HTTP request timeouts or dragging single-paper ingestion times out to **5–10 minutes**.
- Free tiers also suffer from severe throughput degradation during peak hours.

#### 2. Local Small Vision-Language Models (SLMs/VLMs on Desktop) — ❌ REJECTED
- Models evaluated: `Qwen2.5-VL-3B`, `SmolVLM-2B`, `Florence-2`.
- **Empirical Failure**: Testing on consumer hardware (e.g. Intel Core Ultra / Arc / modern laptop CPUs/iGPUs) demonstrated that **Vision Transformer (ViT) image patch encoding takes 10–20 seconds per high-resolution PDF page render**.
- Processing a 35-page paper across 20+ question and mark-scheme spans through a local VLM takes **15 to 30 minutes** with fans blaring, completely unacceptable compared to the current 15–20 second baseline.

---

### The Two Viable Zero-Cost, High-Speed Paradigms

#### Paradigm A: 100% Deterministic On-Device Rust Ingestion (1–3 seconds, $0.00)
Born-digital PDFs represent >95% of UK exam papers from 2010–2026. Vector paths, text strings, and font metrics are already present in the PDF:
1. **Deterministic Mark-Scheme Parser (The 70% cost block)**:
   - UK mark schemes are rigid grid tables with standard headers (`Question | Marking guidance | Additional comments | Mark`).
   - `pdfium` provides exact glyph bounding boxes. A coordinate-aligned column gutter and row parser in pure Rust could extract answers, question IDs (`01.1`), and mark points (`M1`, `A1`) in **< 200 milliseconds** on CPU without touching an LLM.
2. **Local Figure Splicing in Tier-0**:
   - In fixture diagnostics (`physics '24.pdf`), **100% of rejected Tier-0 questions failed solely because of `figure_reference_unsupplied`**.
   - `stroke_census.rs` already extracts and crops diagrams on-device for $0.
   - If Tier-0 carves the text, inserts a `[DIAGRAM_PLACEHOLDER]`, and splices the local crop via the existing `attach_detected_figures` seam, Tier-0 question paper acceptance rises from **78% to >95%**.
3. **Multi-Board Syntax Normalization**:
   - Calibrating regexes for Edexcel `(n)` marks and CIE trailing dotted answer lines `............ [3]` brings non-AQA papers from 0% to >85% Tier-0 acceptance.
4. **Local Topic Matching**:
   - Matching question text against a finite syllabus taxonomy via BM25/keyword glossary or a tiny 22MB ONNX embedding model (`all-MiniLM-L6-v2`) in pure Rust takes **< 5 milliseconds**.

#### Paradigm B: The Content-Addressed Global Community Corpus (< 0.5 seconds, $0.00)
- **The Core Insight**: Past exam papers are static, public, immutable commodities. Thousands of users import the exact same file (`AQA-74081-QP-JUN22.PDF`).
- **Mechanism**:
  1. Hash PDF bytes (`SHA-256`).
  2. Check local SQLite cache ($0).
  3. Query a free static CDN (GitHub Releases or Cloudflare R2 with $0 egress fees) for pre-extracted, human-verified JSON cards + diagram crops.
  4. If hit: **Instant < 0.5s import at $0.0000**.
  5. Pre-seeding a 10-year archive (2015–2025) for major boards immediately solves >90% of all real-world imports.

---

### What We Need From You

1. **Deterministic Mark-Scheme Parsing**: How would you architect a robust, board-agnostic tabular parser in Rust using `pdfium` text boxes (handling wrapped lines, subpart keys like `01.1`, and multi-line marking guidance without column bleed)? What edge cases typically break deterministic table extraction in exam mark schemes?
2. **Pushing Tier-0 Carving to the Limit**: What deterministic techniques can we use to handle complex mathematical formulas, stacked fractions, and ambiguous boundary lines in born-digital exam PDFs without hallucination or silent corruption?
3. **Beyond Paradigms A & B**: Are there other non-cloud, non-heavy-ViT architectural strategies to achieve sub-second, zero-cost exam paper ingestion that we haven't considered?
4. **Edge-Case Escalation Policy**: For the remaining <5% of papers that genuinely cannot be parsed deterministically (e.g. 1990s photocopies, handwritten mark sheets), how should the pipeline fail or escalate gracefully without introducing free-tier throttling traps?
