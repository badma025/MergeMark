# Engineering Task: Zero-Cost ($0.0000) On-Device Question Paper Ingestion

### Objective
Eliminate all cloud LLM API calls on Question Paper imports to achieve true **$0.0000 on-device ingestion** on born-digital PDFs. On `physics '24.pdf`, lift Tier-0 deterministic acceptance from 19/32 to **32/32 questions**, eliminate the deferred cloud topic classification LLM call, and verify via the e2e test harness that `prompt_tokens = 0`, `completion_tokens = 0`, and all 32 questions pass rendering checks.

---

### Context & Why Money Is Still Spent Today
In our recent import of `physics '24.pdf` without mark schemes, **$0.0255** (47,883 tokens) was consumed across five stages:
1. **`vision_span` & `crop_first` (~$0.010)**: Tier-0 refused diagram-referencing questions (`figure_reference_unsupplied` in `src-tauri/src/deterministic.rs`), escalating them to Gemini Vision as high-res images.
2. **`text_first_extraction` & `batch_text_first` (~$0.009)**: Questions with radical fractions (Q21) or exponents (Q22) failed rigid line checks in Tier-0, escaping to Gemini Flash text prompts.
3. **`topic_classification` (~$0.001)**: A deferred cloud LLM call at import completion burned ~2,700 tokens classifying topics.

Because exam boards (AQA, Pearson Edexcel, OCR, Cambridge) hold copyright over exam questions, we **cannot** use an external community corpus or CDN. Everything must be carved and assembled **100% locally on-device in Rust directly from the user's PDF**.

---

### Implementation Instructions

#### Task 1: Remove the Figure Refusal Gate in Tier-0
**Files**: `src-tauri/src/deterministic.rs` and `src-tauri/src/pipeline.rs`

1. In `src-tauri/src/deterministic.rs` (around lines 1626–1633):
   - Locate the figure refusal check in `transcribe_span`:
     ```rust
     if FIGURE_REF_RE.is_match(&norm.content) || crate::pipeline::figure_read_required(&norm.content) {
         let referenced = crate::validate::figure_reference_numbers(&norm.content);
         let distinct: std::collections::BTreeSet<u32> = referenced.iter().copied().collect();
         if distinct.len().max(1) > available_figures {
             return Err("figure_reference_unsupplied");
         }
         insert_figure_placeholders(&mut norm.content);
     }
     ```
   - **Fix**: Remove `return Err("figure_reference_unsupplied");`. The question text in the PDF is readable regardless of vector detection count. Always call `insert_figure_placeholders(&mut norm.content);` and allow the transcription to succeed.
   - `attach_detected_figures` in `pipeline.rs` already handles attaching vector crops detected on-device by `stroke_census.rs` for $0 and scrubs unattached placeholders.
2. In `src-tauri/src/pipeline.rs` (around line 3951):
   - Allow Tier-0 to run whenever `text_first && has_text && config.deterministic`. Do not let `needs_vision` gate Tier-0 out; if Tier-0 succeeds on the text layer, we avoid cloud vision entirely.

---

#### Task 2: Absorb Radicals, Fractions & Exponents in Tier-0
**Files**: `src-tauri/src/deterministic.rs` and `src-tauri/src/sanitize.rs`

1. In `src-tauri/src/deterministic.rs` (`recover_stacked_mcq_fractions` and `transcribe_span`):
   - Ensure the deterministic transcriber accepts and normalizes:
     - Multi-line fraction options where denominators contain radicals or variables (e.g. Q21: $\frac{R}{\sqrt[3]{2}}$, $\frac{R}{\sqrt[3]{16}}$, $\frac{R}{2}$, $\frac{\sqrt{2}R}{8}$).
     - Exponent powers with plain numbers (e.g. Q22: $10^3\text{ m}, 10^4\text{ m}, 10^5\text{ m}, 10^6\text{ m}$).
     - Multi-line decay chains (Q31: $^{238}_{92}\text{U} + \text{n} \rightarrow \text{X}$).
   - Ensure these patterns do not trigger `no_terminal_ending` or structural formatting errors in Tier-0.
2. Ensure standalone image options (Q26) pass through Tier-0 with `- [MCQ:A] ![Option A](...)` option bindings.

---

#### Task 3: Local In-Memory Topic Classification
**Files**: `src-tauri/src/pipeline.rs`

1. In `classify_topics_deferred` in `src-tauri/src/pipeline.rs`:
   - Before building the LLM chat body, run a deterministic keyword matcher on each question against `config.allowed_topics` (or standard A-Level Physics syllabus topics: "Thermal Physics", "Capacitors", "Electric Fields", "Gravitational Fields", "Magnetic Fields", "Nuclear Physics", "Waves", "Mechanics"):
     - If text contains `ideal gas|c_{rms}|specific heat|latent heat|molar` $\rightarrow$ `Thermal physics`
     - If text contains `capacitor|capacitance|dielectric|parallel-plate` $\rightarrow$ `Capacitors`
     - If text contains `electric field|electric potential|coulomb` $\rightarrow$ `Electric fields`
     - If text contains `gravitational field|satellite|orbit|escape velocity` $\rightarrow$ `Gravitational fields`
     - If text contains `magnetic flux|induced emf|solenoid|tesla` $\rightarrow$ `Magnetic fields`
     - If text contains `alpha|beta|decay|half-life|fission|fusion|nuclide|uranium` $\rightarrow$ `Nuclear physics`
     - If text contains `oscilloscope|frequency|phase difference|waveform` $\rightarrow$ `Waves`
   - Assign matched topics to `q.topics`.
   - If all questions are classified, `untagged` becomes empty, and the function returns `0` immediately without dispatching any cloud LLM call.

---

### Verification Protocol
1. Run Rust test suite:
   ```powershell
   cargo test --manifest-path src-tauri/Cargo.toml --lib
   ```
2. Re-run headless end-to-end import on `physics '24.pdf`:
   ```powershell
   cargo run --manifest-path src-tauri/Cargo.toml --bin e2e_import -- "../physics '24.pdf" physics24_zero_cost --out ../output/physics24-zero-cost --config-db C:/Users/alimb/AppData/Roaming/com.mergemark.app/mergemark.db
   ```
3. Check the CLI output report:
   - **`tier0=32`** (all 32 questions extracted 100% on-device by Tier-0).
   - **`text_first=0`**, **`crop_first=0`**, **`vision_repairs=0`**, **`quarantined=0`**.
   - **`prompt_tokens=0`**, **`completion_tokens=0`** ($0.0000 cost).
4. Run the rendering verification script:
   ```powershell
   node scripts/verify-physics24-render.mjs output/physics24-zero-cost/physics24_zero_cost_cards.json
   ```
   - Must output: `complete=true, passed=32, total=32, failures=[]`.
