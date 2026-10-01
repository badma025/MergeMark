# Forensic Audit: Physics 2024 Import Inspection (`mergemark_final.html` vs `physics '24.pdf`)

**Document Version**: 1.0.0  
**Target File**: `mergemark_final.html`  
**Ground Truth**: `physics '24.pdf` (AQA A-Level Physics 7408/2, June 2024)  
**Database**: `C:\Users\alimb\AppData\Roaming\com.mergemark.app\mergemark.db` (Import ID: `66600459-7b8c-4698-bbff-0e2cc7a1069c`)  

---

## 1. Executive Summary

This forensic audit presents a question-by-question cross-check of all **32 questions** between the original examination paper `physics '24.pdf` and the imported HTML artifact `mergemark_final.html`.

- **Total Questions Audited**: 32 (Questions 01–07 in Section A, Questions 08–32 in Section B).
- **Clean / Passing Questions**: 19 / 32 (59.4%).
- **Critical Structural / Parsing Failures**: 9 / 32 (28.1%) — Questions 04, 06, 16, 17, 21, 22, 24, 26, 31.
- **Major Formatting / Layout Defects**: 3 / 32 (9.4%) — Questions 14, 18, 30.
- **Minor LaTeX / KaTeX Font Glitches**: 2 / 32 (6.3%) — Questions 01, 02.

---

## 2. Global Systemic Failure Modes

Inspection of the rendered DOM and underlying markdown reveals four systemic failure patterns:

1. **Frontend MCQ Preprocessor Math Corruption**:
   - In `preprocess-mcq.ts`, options containing inline math (or multiple `$` delimiters) get indiscriminately wrapped in `$$ ... $$`. When an option has text + math (e.g. Q18, Q24, Q27, Q30), this turns normal English words into spaced italic math variables (e.g. `t h e c h a r g e ...`).
   - If an option contains an unclosed or rogue `$`, subsequent options on following lines are swallowed into the math block, causing raw `- [MCQ:B]` markdown tags to leak directly into the rendered text.

2. **Subpart Stripping & Mark Offset in Section A**:
   - In **Q04**, the parser failed to recognize `04.1`, `04.2`, `04.3` as subpart demarcations because they lacked preceding introductory text, stripping all subpart identifiers `(a)`, `(b)`, `(c)` and all mark tags `**[X marks]**`.
   - In **Q06**, subpart `06.1` was erroneously classified as question intro prose. As a result, its `(a)` label and `[3 marks]` were deleted, and subparts `06.2`, `06.3`, `06.4` were shifted to `(a)`, `(b)`, `(c)` with shifted mark values, reducing total card marks from 12 to 9.

3. **Multi-line Math Fraction / Root Collapses**:
   - In Section B, two-dimensional typesetting in the PDF (such as $\frac{R}{\sqrt[3]{16}}$ in Q21 or vertical fractions in Q16) extracted as split lines (`A 3 \n R`). Without OCR fraction reconstruction, the MCQ parser failed to generate valid `- [MCQ:X]` syntax.

4. **Multi-line Nuclear Decay / Equation Scrambling**:
   - Complex horizontal/vertical reaction chains (Q31: $^{238}_{92}\text{U} + \text{n} \rightarrow \text{X}$) became vertically interleaved during text-first extraction (`238Un $X 92 + \rightarrow`).

---

## 3. Comprehensive Question-by-Question Cross-Check

### Section A: Structured & Extended Response (Questions 01 to 07)

#### Question 01 [Total Marks: 9] — MINOR DEFECT
- **PDF Ground Truth**:
  - Intro: Room contains dry air at $20.0\text{ }^\circ\text{C}$ and $105\text{ kPa}$.
  - Subpart 01.1: Show amount of air is about $40\text{ mol}$ [1 mark].
  - Subpart 01.2: Density $1.25\text{ kg m}^{-3}$. Calculate $c_{\text{rms}}$ [3 marks].
  - Subpart 01.3: Calculate $\Delta T$ in K to double $c_{\text{rms}}$ [2 marks].
  - Subpart 01.4: Moist air, dehumidifier, Table 1 with ratio header $\frac{\text{mass of water}}{\text{mass of air}}$ [3 marks].
- **Rendered HTML / Card Issues**:
  - **Table 1 Column Split**: Header $\frac{\text{mass of water}}{\text{mass of air}}$ was split into two separate table columns (`mass of water` | `mass of air`), leaving the `mass of air` column completely empty (`| moist air flowing in | 0.0057 | |`).
  - **Degree Symbol Rendering**: `$20.0 \text{ °C}$` has encoding artifacts depending on the KaTeX font set.

#### Question 02 [Total Marks: 9] — MINOR DEFECT
- **PDF Ground Truth**:
  - Capacitor circuit diagrams (Figures 1, 2, 3, 4) and Table 2.
  - Subparts 02.1 [2 marks], 02.2 [2 marks], 02.3 [3 marks], 02.4 [2 marks].
- **Rendered HTML / Card Issues**:
  - **Invalid LaTeX Commands inside `\text{...}`**:
    - `$31.0 \text{ \mu F}$`: `\mu` is a LaTeX math symbol placed inside a text block.
    - `$2.4 \times 10^5 \text{ \Omega}$`: `\Omega` is a LaTeX math symbol placed inside a text block.
    - Causes KaTeX font fallback warnings and irregular symbol spacing.

#### Question 03 [Total Marks: 7] — PASS
- **PDF Ground Truth**: Conducting rod in Earth's B-field. Subpart 03.1 [3 marks], Subpart 03.2 [4 marks].
- **Rendered HTML / Card Issues**: None. Subparts `(a)` and `(b)` with marks `**[3 marks]**` and `**[4 marks]**` match PDF.

#### Question 04 [Total Marks: 5] — CRITICAL FAILURE
- **PDF Ground Truth**:
  - Subpart 04.1: State other purpose of coolant [1 mark].
  - Subpart 04.2: State two properties engineers consider [2 marks].
  - Subpart 04.3: Explain how power output is decreased [2 marks].
- **Rendered HTML / Card Issues**:
  - **Complete Subpart Stripping**: All subpart labels `(a)`, `(b)`, `(c)` are completely missing.
  - **Missing Mark Allocations**: All mark allocations (`[1 mark]`, `[2 marks]`, `[2 marks]`) were stripped.
  - Text is rendered as 3 plain paragraphs with no subpart structure.

#### Question 05 [Total Marks: 8] — PASS
- **PDF Ground Truth**: Satellite S1 and Moon orbit. Subparts 05.1 [3 marks], 05.2 [2 marks], 05.3 [3 marks].
- **Rendered HTML / Card Issues**: None. Subparts `(a)`, `(b)`, `(c)` and mark allocations match PDF.

#### Question 06 [Total Marks: 12 in PDF; 9 in Card] — CRITICAL FAILURE
- **PDF Ground Truth**:
  - Subpart 06.1: Explain meaning of potential $-4.0\text{ V}$ [3 marks].
  - Figure 9 & text: Electron confinement in GaAs.
  - Subpart 06.2: Determine maximum magnitude of electric field and unit [4 marks].
  - Subpart 06.3: Kinetic energy of electron moving from 300 nm to 800 nm [2 marks].
  - Subpart 06.4: Discuss motion of electron starting at 350 nm [3 marks].
  - Total Question Marks = $3 + 4 + 2 + 3 = 12\text{ marks}$.
- **Rendered HTML / Card Issues**:
  - **Subpart 06.1 Swallowed as Intro**: Subpart 06.1 was parsed as introductory prose. Its subpart label and `[3 marks]` were deleted.
  - **Off-by-One Subpart Shift & Mark Misattribution**:
    - Subpart 06.2 became `(a)` with `[3 marks]` (was [4 marks] in PDF).
    - Subpart 06.3 became `(b)` with `[4 marks]` (was [2 marks] in PDF).
    - Subpart 06.4 became `(c)` with `[2 marks]` (was [3 marks] in PDF).
  - **Lost Marks**: Total question marks dropped from 12 to 9.

#### Question 07 [Total Marks: 10] — PASS
- **PDF Ground Truth**: Dice radioactive decay model. Subparts 07.1 [1 mark], 07.2 [5 marks], 07.3 [2 marks], 07.4 [2 marks].
- **Rendered HTML / Card Issues**: None. Subparts `(a)`, `(b)`, `(c)`, `(d)` and mark allocations match PDF.

---

### Section B: Multiple Choice Questions (Questions 08 to 32)

#### Question 08 [1 mark] — PASS
- **PDF Ground Truth**: Ideal gas work done. Options A–D.
- **Rendered HTML / Card Issues**: Clean. Options `- [MCQ:A]` through `- [MCQ:D]` render properly.

#### Question 09 [1 mark] — PASS
- **PDF Ground Truth**: RMS speed of 3 molecules ($2.00v, 4.00v, 5.00v$).
- **Rendered HTML / Card Issues**: Clean.

#### Question 10 [1 mark] — PASS
- **PDF Ground Truth**: Ideal gas expansion with heater ($300\text{ K}$, $5000\text{ J}$, $1660\text{ J}$).
- **Rendered HTML / Card Issues**: Clean.

#### Question 11 [1 mark] — PASS
- **PDF Ground Truth**: Parallel-plate capacitor plates moved further apart.
- **Rendered HTML / Card Issues**: Clean.

#### Question 12 [1 mark] — PASS
- **PDF Ground Truth**: Transformer efficiency increase.
- **Rendered HTML / Card Issues**: Clean.

#### Question 13 [1 mark] — PASS
- **PDF Ground Truth**: Oscilloscope volts/division setting for $7.0\text{ V}$ RMS.
- **Rendered HTML / Card Issues**: Clean.

#### Question 14 [1 mark] — MAJOR DEFECT
- **PDF Ground Truth**:
  - Two signals oscilloscope display ($5\text{ ms div}^{-1}$).
  - Table: `Frequency of both signals / Hz` | `Phase difference / rad`.
  - A: 50 | $0.30\pi$; B: 50 | $0.15\pi$; C: 25 | $0.30\pi$; D: 25 | $0.15\pi$.
- **Rendered HTML / Card Issues**:
  - **Flattened Table Columns**: Headers stripped; two values are placed on raw separate lines per option (`- [MCQ:A] $50$\n$0.30\pi$`).
  - **Misplaced Diagrams**: Two diagram images dumped after Option D instead of inside the question stem.

#### Question 15 [1 mark] — PASS
- **PDF Ground Truth**: Transmission cable efficiency at $50\text{ Hz}$.
- **Rendered HTML / Card Issues**: Clean.

#### Question 16 [1 mark] — CRITICAL FAILURE
- **PDF Ground Truth**:
  - Electron in uniform B-field. Number of circuits in 1 second.
  - Option A: $\frac{2\pi m_e}{Be}$
  - Option B: $\frac{2\pi r}{v}$
  - Option C: $\frac{v}{\pi r}$
  - Option D: $\frac{Be}{2\pi m_e}$
- **Rendered HTML / Card Issues**:
  - **Inverted & Corrupted Options**:
    - Option A was extracted as $\frac{eB}{2\pi m}$ (inverted from $\frac{2\pi m_e}{Be}$).
    - Option C was extracted as $\frac{v}{2\pi r}$ (added factor of 2 not in PDF $\frac{v}{\pi r}$).
    - Option D was extracted as $\frac{eB}{2\pi m}$ (identical to Option A).
  - **Duplicate Options**: Option A and Option D are identical, making the question unanswerable.

#### Question 17 [1 mark] — CRITICAL FAILURE
- **PDF Ground Truth**:
  - Proton and C-13 fusion: $^{13}_{6}\text{C} + {^{1}_{1}\text{p}} \rightarrow {^{14}_{7}\text{N}}$.
  - Masses: C-13 ($13.00007\text{ u}$), N-14 ($13.99925\text{ u}$), proton ($1.00728\text{ u}$).
  - Options: A: 0.5 MeV, B: 1.1 MeV, C: 7.5 MeV, D: 8.8 MeV.
- **Rendered HTML / Card Issues**:
  - **Rogue `$$` Delimiters & KaTeX Error**:
    - Equation string: `$^{13}_{6}$$\text{C}$$ + {^{1}_{1}$$\text{p}$$} \rightarrow {^{14}_{7}$$\text{N}$$}$`.
    - Throws red `katex-error` in HTML.
  - **Text Forced into Math Mode**: Unbalanced `$` causes following text to render in math mode: `mass of ^{13}_{6}\text{C} n u c l e u s = 13.00007 u$`.
  - **Leaked MCQ Markdown Tags**: Option A breaks and options B, C, D leak into the card as unparsed markdown tags (`\ - [MCQ:B] 1.1 MeV \ - [MCQ:C] ...`).

#### Question 18 [1 mark] — MAJOR DEFECT
- **PDF Ground Truth**:
  - U-235 fission reaction: $^{235}_{92}\text{U} + {^{1}_{0}\text{n}} \rightarrow {^{87}_{35}\text{Br}} + {^{146}_{57}\text{La}} + 3{^{1}_{0}\text{n}}$.
  - Options A–D testing binding energy and mass statements.
- **Rendered HTML / Card Issues**:
  - **Triple Dollars in Option A**: Rendered as `A $$$^{146}_{57}\text{La}$$ has the greatest binding energy per nucleon...$`.
  - **Prose Wrapped in Math Mode in Option B**: Entire English sentence wrapped in math delimiters, rendering prose words as spaced math variables (`B T h e m a s s o f The mass of T h e ma sso f...`).

#### Question 19 [1 mark] — PASS
- **PDF Ground Truth**: Burning $1.0\text{ kg}$ of wood pellets releases $5.6\text{ kW h}$.
- **Rendered HTML / Card Issues**: Clean. (Leading $5.6\text{ kW h}$ correctly preserved).

#### Question 20 [1 mark] — PASS
- **PDF Ground Truth**: Nuclear radius $r$ with nucleon numbers $x$ and $y$. Options $r(x/y)^3$, $r(y/x)^3$, $r(x/y)^{1/3}$, $r(y/x)^{1/3}$.
- **Rendered HTML / Card Issues**: Clean. LaTeX fractions and fractional exponents render correctly.

#### Question 21 [1 mark] — CRITICAL FAILURE (Flagged `Needs Review`)
- **PDF Ground Truth**:
  - Synchronous orbit radius $R$. Planet with $2M_E$ and day = $0.25$ Earth day.
  - Option A: $\frac{R}{\sqrt[3]{2}}$
  - Option B: $\frac{R}{\sqrt[3]{16}}$
  - Option C: $\frac{R}{2}$
  - Option D: $\frac{\sqrt{2}R}{8}$
- **Rendered HTML / Card Issues**:
  - **Complete Fraction Parsing Failure**: Options extracted as fragmented single characters:
    `A 3 \n R \n B 3 \n R \n C \n R \n D 2 \n R`.
  - **Missing MCQ Syntax**: No `- [MCQ:X]` syntax generated; card flagged with red `Needs Review` badge.

#### Question 22 [1 mark] — CRITICAL FAILURE (Flagged `Needs Review`)
- **PDF Ground Truth**:
  - Asteroid mass $2 \times 10^{17}\text{ kg}$, escape velocity $40\text{ m s}^{-1}$.
  - Option A: $10^3\text{ m}$
  - Option B: $10^4\text{ m}$
  - Option C: $10^5\text{ m}$
  - Option D: $10^6\text{ m}$
- **Rendered HTML / Card Issues**:
  - **Missing Option D**: Option D is completely absent from the card.
  - **Flattened Exponents**: Superscripts stripped into raw numbers (`A 103 m`, `B 104 m`, `C 105 m m`).
  - **Missing MCQ Syntax**: Flagged `Needs Review`.

#### Question 23 [1 mark] — PASS
- **PDF Ground Truth**: Binary star gravitational potential equipotentials. Options A–D.
- **Rendered HTML / Card Issues**: Clean.

#### Question 24 [1 mark] — CRITICAL FAILURE
- **PDF Ground Truth**:
  - Definition of $\varepsilon_0$.
  - Option B: charge stored on capacitor of area $1\text{ m}^2$ separated by $1\text{ m}$ at $1\text{ V}$.
  - Options C and D.
- **Rendered HTML / Card Issues**:
  - **Math Preprocessor Corruption in Option B**: Option B contains 3 separate inline math expressions (`$1\ \text{m}^2$`, `$1\ \text{m}$`, `$1\ \text{V}$`). The frontend preprocessor wrapped the entire option in math mode, turning the English sentence into spaced math variables (`B t h e c h a r g e s t o r e d o n a c a p a c i t o r...`).
  - **Swallowed Options C and D**: Options C and D were pulled into the unclosed math block and leak as raw unrendered markdown tags (`\ - [MCQ:C] ... \ - [MCQ:D] ...`).

#### Question 25 [1 mark] — PASS
- **PDF Ground Truth**: Point charge force when charge doubled and distance halved. Options $16F, 8F, 2F, F$.
- **Rendered HTML / Card Issues**: Clean.

#### Question 26 [1 mark] — CRITICAL FAILURE
- **PDF Ground Truth**:
  - Charge distribution where potential and field at P are zero.
  - Four visual charge configurations labeled A, B, C, D.
- **Rendered HTML / Card Issues**:
  - **Zero Option Bindings**: The 4 diagram images were extracted into the card body without any `- [MCQ:A]`, `- [MCQ:B]`, `- [MCQ:C]`, `- [MCQ:D]` option tags. The card has no selectable MCQ options.

#### Question 27 [1 mark] — CRITICAL FAILURE
- **PDF Ground Truth**:
  - Ion specific charge $-7.1 \times 10^7\text{ C kg}^{-1}$.
  - Option A: $1.38 \times 10^{-7}\text{ V m}^{-1}$ upwards
  - Option B: $1.38 \times 10^{-7}\text{ V m}^{-1}$ downwards
  - Option C: $7.24 \times 10^6\text{ V m}^{-1}$ upwards
  - Option D: $7.24 \times 10^6\text{ V m}^{-1}$ downwards
- **Rendered HTML / Card Issues**:
  - **Swallowed Options B and D**: The unit formatting `1.38 $\times 10^{-7}$ V m$^{-1}$ upwards` has broken dollar placement. As a result, Option B was swallowed into Option A, and Option D was swallowed into Option C.
  - Rendered card displays only choices A and C, with raw `- [MCQ:B]` and `- [MCQ:D]` leaked into the text.

#### Question 28 [1 mark] — PASS
- **PDF Ground Truth**: Particle pair with largest ratio of electrostatic to gravitational force.
- **Rendered HTML / Card Issues**: Clean.

#### Question 29 [1 mark] — PASS
- **PDF Ground Truth**: Gold nucleus radius from alpha particle scattering.
- **Rendered HTML / Card Issues**: Clean.

#### Question 30 [1 mark] — MAJOR DEFECT
- **PDF Ground Truth**:
  - Plot of $N$ vs $Z$ for atomic nuclei.
  - Stem contains graph of $N$ against $Z$.
  - Question: $^{115}_{45}\text{Rh}$ is likely to decay by (A: $\alpha$, B: $\beta^+$, C: $\beta^-$, D: electron capture).
- **Rendered HTML / Card Issues**:
  - **Misplaced Diagram**: The diagram of the $N$ vs $Z$ graph is positioned at the very bottom of the card *after* Option D instead of within the question stem.
  - **Triple Dollars**: Options A, B, C wrapped in `$$$\alpha$$ emission.$`.

#### Question 31 [1 mark] — CRITICAL FAILURE
- **PDF Ground Truth**:
  - U-238 absorbs a neutron:
    $^{238}_{92}\text{U} + \text{n} \rightarrow \text{X}$
    $\text{X} \rightarrow \text{Y} + \beta^- + \bar{\nu}_e$
    $\text{Y} \rightarrow \text{Z} + \beta^- + \bar{\nu}_e$
  - Question: How many neutrons does Z have?
  - Options: A: 144, B: 145, C: 149, D: 237.
- **Rendered HTML / Card Issues**:
  - **Catastrophic Nuclear Equation Scrambling**: Text-first extraction vertically scrambled the multi-line equations into nonsense:
    `238Un $X 92 + \rightarrow`
    `XY $v + + e \rightarrow \beta$-`
    `YZ $v + + e \rightarrow \beta$-`

#### Question 32 [1 mark] — PASS
- **PDF Ground Truth**: Rock sample containing U-235 and Pb-207. Options A–D.
- **Rendered HTML / Card Issues**: Clean.

---

## 4. Remediation Recommendations

1. **Fix `src/lib/preprocess-mcq.ts`**:
   - Stop regex auto-wrapping options with `$$ ... $$` when the option contains multiple `$` signs or plain text mixed with math.
   - Parse each `- [MCQ:X]` option line independently so an unclosed `$` on one line cannot swallow subsequent option lines.

2. **Fix `src-tauri/src/sanitize.rs`**:
   - Repair rogue double/triple dollars (`$$` and `$$$`) in nuclear reactions (Q17, Q18, Q30).
   - Reconstruct multi-line isotope decay chains (Q31).
   - Move LaTeX math commands out of `\text{...}` (e.g. `\text{ \mu F}` $\rightarrow$ `\mu\text{F}`).

3. **Fix `src-tauri/src/deterministic.rs` & `validate.rs`**:
   - Prevent subpart stripping when Question X starts directly with `0X.1` without preceding intro prose (Q04).
   - Prevent `0X.1` from being classified as intro prose when it is followed by a diagram on the next page (Q06).
   - Fuse multi-line fraction options into proper LaTeX `\frac{...}{...}` (Q16, Q21).
   - Bind image-only options to `- [MCQ:A] ![Diagram]...` (Q26).
   - Ensure stem diagrams remain before the option block (Q14, Q30).
