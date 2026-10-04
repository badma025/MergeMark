# Import fixes round 2: handoff

Everything a new conversation needs to continue the offline import fixes. Round 1 (clusters 1–5) is finished and written up in `docs/TRIAGE_FIX_REPORT.md`. This file covers round 2 (clusters 5b, 5c and 6–11), which is now done and written up in the same report; the plan below is kept as it was.

## Tester release prep (4 Oct 2026)

Release-plumbing work done on top of round 3, in the main tree. No pipeline code changed.
- **Secret removed from the repo.** `.env` held a real `DATALAB_API_KEY` and
  `LLAMAPARSE_API_KEY` and was committed on the **public** `origin/master`
  (commit `ff9d9a5`). It is now untracked and gitignored, with a `.env.example`
  in its place. Nothing in the app reads either key (no `reqwest`, no network
  deps). **The keys still need revoking at Datalab and LlamaIndex, and they
  remain in git history** unless the history is rewritten.
- **Version:** `package.json` and `tauri.conf.json` 0.9.0 → **0.9.4** (the
  published releases already reach v0.9.3, so 0.9.0 would have collided).
  `src-tauri/Cargo.toml` is left at 1.0.0 on purpose: its version feeds the
  `generator` field of the stroke-census artifacts that calibration diffs, so
  changing it would perturb a verified baseline for no release benefit. Align
  it during a 1.0 cycle, with a fresh calibration run.
- **`release.yml` is Windows-only** and no longer injects the stale
  `MERGEMARK_OPENROUTER_API_KEY`. macOS and Linux were building installers that
  cannot import a paper: `Pdfium::bind_to_system_library()` only resolves
  because `tauri.windows.conf.json` ships `pdfium.dll` beside the exe, and
  there is no `libpdfium.dylib`/`.so` equivalent. macOS additionally needs a
  Developer ID signature and notarisation. `macos-build.yml` is deleted (it
  hardcoded the tag `mac-test-build`).
- **NSIS only.** `bundle.targets` was `"all"`, so every build emitted both an
  `.exe` and an `.msi` while the README and tester notes name only the `.exe`.
  It is now `["nsis"]`, so the Releases page carries one download.
- **The dev CLI no longer ships in the installer.** `src/bin/e2e_import.rs`
  moved to `examples/e2e_import.rs`; Tauri enumerates cargo `bin` targets, not
  `example` targets, so it can no longer be bundled. Every caller now uses
  `cargo run --example e2e_import` (`scripts/lib/zero-cost-accounting.mjs`,
  `scripts/verify-zero-cost-offline.mjs`, the README and this file). Historical
  reports in `docs/agent-work/` still say `--bin`; they are records, left as they are.
- **Added:** `LICENSE` (proprietary, matching the README's terms; replace the
  holder "MergeMark" with the legal rights-holder's name). Removed the stray
  tracked file `out`.
- **README/BETA_TESTERS:** Windows-only install, v0.9.4 filename, the dead
  Discord placeholder replaced with GitHub Issues, and the OCR claim narrowed to
  "Windows verified; macOS compile-checked; Tesseract untested".

Still to do for the tester release: revoke the two keys, run `cargo test` and a
`tauri build`, confirm the installer no longer carries `e2e_import.exe`, then
push, tag `v0.9.4` and publish. Not started: history rewrite, code signing,
updater, mark schemes, the six wrong strict cards from round 3.

## Where things stand (4 Oct 2026, end of round 3)
- **Round 3 is done and not committed.** It sits in the main tree on top of branch `import-fixes-rounds-1-2` (whose last commit, `c142a7d`, is the Windows beta). It fixed the six wrong strict cards the cluster 10 audit found, in three clusters, each written up in `docs/TRIAGE_FIX_REPORT.md` ("Round 3"):
  - cluster 12: fractions under a line of text (`build_fractions` drops a neighbouring line's words from a part);
  - cluster 14: a drawing on graph paper takes the whole sheet (`is_graph_paper` in `figure_regions`);
  - cluster 13: the last question ends at the paper's end marker (`end_y`), so the back page's logo is no figure of it.
- **Numbers:** cargo test 378 passed, 0 failed. Calibration c13: 67/67 OK, the 22 originals unchanged (243/244, 69/69, zero attempts). Triage unused-c13 on the 123 papers left: PASS 31, FAIL 42, FLAGGED 43, REVIEW 7, unchanged from c11b on the same papers; 17 cards changed, with no regression in the targeted audits.
- **Workflow notes from round 3:**
  - Code review ran before each cluster's verify chain (on the dev copy's diff), so its corrections were verified once.
  - A verify chain is `bash output/fix-log/verify.sh <stamp>`; `verify.sh` has no `set -e`, so killing its cargo test moves it on to calibration. To stop one, stop the background task, then end the `bash verify.sh`, `node` calibration/triage and `cargo`/`e2e_import` processes.
  - The seeded blinded sample is the same for the same paper set, and moves only when a card changes stratum.
- **Baseline for a round 4:** run `unused-c13` and calibration run `c13`. Still open: "What is left" in the report (new strict cards found wrong in round 3 come first), Tesseract in WSL, macOS Vision run on a Mac, and nine tests that read the deleted root PDFs and so check nothing.

## Where things stood (3 Oct 2026, end of round 2)
- **Committed on branch `import-fixes-rounds-1-2`** (3 Oct 2026, at the user's request), not pushed. See "Windows beta" below for what followed.
  - `git status` also shows old paper PDFs deleted from the repo root (`edexcel/'17 edexcel.pdf`, `physics '17.pdf` and others). Those deletions predate this work and were left out of the commits; leave them out unless the user confirms they were intended.
- **The main tree holds the final round-2 code:** clusters 5b–11, each with its review corrections, on top of the seam session's refactor (`output/fix-log/seam/`, which changes no card). Each cluster is reported in `docs/TRIAGE_FIX_REPORT.md`.
  - `cargo test --manifest-path src-tauri/Cargo.toml`: 371 passed, 0 failed.
  - Calibration run c11b: 64/64 OK. The 22 originals give 243/244 strict historical cards and 69/69 test-paper cards, with zero network attempts and zero tokens.
  - Triage run unused-c11b, on the 126 papers left in `papers/unused/`: PASS 31, REVIEW 7, FLAGGED 44, FAIL 44 (c5b on the same papers: PASS 16, REVIEW 3, FLAGGED 48, FAIL 59).
- **Still open:**
  - the user installs Tesseract in WSL (`wsl sudo apt install -y tesseract-ocr`, which needs their password), so its live test can run;
  - macOS Vision is compile-checked only;
  - "What is left" in the report, starting with the six strict cards the cluster 10 audit found wrong.
- **Baseline for a round 3:** run `unused-c11b` and calibration run `c11b`.

## Release readiness (assessed at the end of round 2)
The user asked whether the app is ready to deploy. The answer given: **not yet.**
- **Solid:** the import works fully offline on Windows; every gate passes; on the holdouts PASS went 16 → 31; in the c10b seeded audit none of the 24 strict cards sampled was MAJOR.
- **Blockers:**
  1. Strict cards can still be wrong. The targeted audit after cluster 10 found 6 of 40: MathMagic fractions, the OCR back-page logo as a figure, a grid cropped without its axes ("What is left" in the report).
  2. Mark schemes barely import: the local reader accepts 0 of the 7 real schemes (cluster 5c).
  3. The desktop app itself was never built (`tauri build`) or clicked through. All verification was cargo tests, the `e2e_import` CLI, calibration and triage. The AI-only features still show their UI and answer "MergeMark works offline".
  4. Scanned papers are verified on Windows only: macOS Vision is compile-checked, and Tesseract has never run (not installed).
  5. Nothing is committed.
- **Path offered:**
  - *Windows beta (days):* commit; build the installer; test the app end to end on a few papers; hide the AI-only UI; tell testers that flagged cards need checking and that mark schemes are not supported.
  - *Public release:* also round 3 (wrong strict cards first), a working mark-scheme reader, and macOS/Linux OCR run on real machines.
- **Asked and not yet answered:** who the release is for (testers or users), and whether to commit.

## Windows beta (3 Oct 2026, after round 2)
The user chose: commit, then the Windows beta, then round 3. The first release is for **testers**, so the AI-only UI is hidden, not removed.
- **Committed** on branch `import-fixes-rounds-1-2` (four commits on top of `master`), without the old paper-PDF deletions.
- **Built and installed** the NSIS installer (`tauri build`), then drove the installed app end to end over the WebView's DevTools port (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333`; the script simulates a native drop with `plugin:event|emit_to` `tauri://drag-drop`). The user's library was moved aside for the test and restored.
- **Imports in the installed app match calibration c11b:** physics '21 (31 IDs, 30 strict), AQA Biology GCSE Jun 2022 (10, 8 strict), core pure 1 '23 (8, all strict), the M3 Jun 2004 scan (7, all flagged, Windows OCR). A refused mark scheme says it was not transcribed.
- **Found and fixed:**
  - `pdfium.dll` was not bundled: `src-tauri/tauri.windows.conf.json` ships it and its licences beside the exe (Windows only).
  - No diagram ever loaded in a release build: the CSP lacked `http://asset.localhost` (how WebView2 serves the asset protocol). Added it to `img-src` and `connect-src`, with `http://ipc.localhost` for IPC. The cluster 5b guard test now allows exactly those two hosts.
  - Toasts were unstyled: Tauri adds a nonce to `style-src`, which voids `'unsafe-inline'`, so injected styles were blocked. `dangerousDisableAssetCspModification: ["style-src"]`.
  - A subject with no modules hid the module picker and failed with "A module must be selected." The import page now says to add a module first.
  - Worksheet PDFs printed `[MCQ:A]` markers; options now print their letter.
  - A worksheet compile that pdflatex did not finish (MiKTeX installing packages mid-run) was reported as compiled. It is now an error that keeps the log.
  - Hidden: the free-parses counter, the Settings "API & Spend" tab (credentials, OpenRouter dashboard), the "Auto-Topics" button. The REVIEW badge no longer mentions a vision fallback. Mark-scheme mode says import is limited.
- **Known, not fixed:** the installer also ships the developer CLI `e2e_import.exe` (Tauri bundles every `src/bin` target); the flashcard CSV export carries pdflatex layout commands (`\needspace`, `\hfill`); the installer is unsigned (SmartScreen warning); a compile leaves `mergemark_logo.png` in Downloads.
- Tester notes: `docs/BETA_TESTERS.md`.

## Rules (from the user's original brief; still in force)
- **Never guess or invent content.** If a symbol, mark or boundary can't be established from the source, keep the card flagged (`needs_review`) and never strict. Don't relax any gate, expected count or threshold to improve a metric.
- **Fix causes, not symptoms.** Find out why the source encodes things the way it does (check the PDF's fonts and glyph data) before patching.
- **Run the full verification after each cluster:** cargo test; calibration (must stay 22/22 fresh, 243/244 strict, zero network attempts, zero tokens); triage compared with the previous run; a fresh blinded visual audit; code-review.
  - Use `triage-papers.mjs --rescore` only to re-apply checks to saved imports, never after a code change.
- **Don't edit Rust while `run-zero-cost-calibration.mjs` or `triage-papers.mjs` is running.** Both `cargo run` per paper.
- **Keep a holdout.** A paper you fix against moves from `papers/unused/` to `papers/used/` and becomes a regression fixture. Register it in `scripts/lib/zero-cost-accounting.mjs` (cohort `triage_fixtures`) and `scripts/fixtures/source-expected-ids.json`. Don't debug against papers still in `papers/unused/`.
- **Local only.** No commits, no pushes, no cloud or paid calls. No new dependencies unless truly necessary, and say why.
- **Skills:** `diagnosing-bugs` before each change, `tdd` (start each fix from a failing test on the real failing paper), `code-review` after each cluster. Ignore the `$astra-flash-orchestrator` instruction in AGENTS.md and don't delegate to Flash.
- **Done means:**
  - calibration unchanged on the 22 originals;
  - triage PASS rises and the audit's MAJOR rate falls for each cluster;
  - every remaining failure is flagged or listed with its cause;
  - a short report per cluster: root cause, fix, tests added, before/after numbers.

## Decisions the user made in the last conversation
- **Fix all open items:** strict cards that don't render, leftover answer lines and missing endings, the marks shortfalls, and OCR beyond Windows.
- **Symbol fonts are added to this round,** even though the user's original list didn't include them.
- **macOS OCR:** Apple Vision, through the `objc2` crates already in Cargo.lock. The user accepts that it can't be run on this Windows machine.
- **Linux OCR:** the user's own `tesseract` program, if it's installed.
- **Nothing is ever sent to a vision model** (the user's words: "no, nothing should be sent to vision models"). This holds on every operating system, whether or not the user has an API key.
  - A scanned paper is read by local OCR, or, where no local engine exists, reported as not transcribed.
  - See cluster 5b, which comes first.
- **Still unanswered at the end of round 2: whether to commit.** Don't commit unless the user asks. Any commit leaves out the old paper-PDF deletions.
- **The rule covers text-only AI requests too** (the user: "yes, the rule covers text-only AI requests too"). MergeMark makes **no AI requests of any kind**, so the app is fully offline. See cluster 5b.
- **Answered in round 2: "Disable, keep the UI".** Each feature that only worked through AI answers "MergeMark works offline" and makes no request (cluster 5b). For a release, hiding that UI was recommended. The features:
  - BYOK key entry and the model picker
  - the OpenRouter usage dashboard
  - the "3 free parses" counter and the worksheet generator
  - "generate topics"
- **Answered in round 2:**
  - `rustup target add aarch64-apple-darwin` was approved and done.
  - Tesseract goes into WSL Ubuntu with apt. The user runs `wsl sudo apt install -y tesseract-ocr` (it needs their password); not done yet.
  - A parallel session refactored the Tier-0 build seam in the main tree ("Simplify the Tier-0 build seam"). The user chose to let it finish first. Its change is in main, and it changes no card: 855 calibration and 1771 triage cards are identical (`output/fix-log/seam/`, run `unused-seam`).

## Evidence behind the plan (from the unused-c5 logs)
`node output/fix-log/flag-reasons.mjs unused-c5` counts the `[TIER0_FALLBACK] reason=…` lines in `output/triage/run-unused-c5/logs/`:

| gate reason | cards flagged |
|---|---|
| leftover_answer_line | 208 |
| unidentified_source_glyph | 165 |
| no_terminal_ending | 110 |
| sub-part (17), no_marks_signal (13), subpart_gap (11), marks_checksum_mismatch (11), too_short (8), others | ~80 |

- **36 non-PASS papers are blocked only by leftover_answer_line and/or no_terminal_ending:** 17 by no_terminal_ending alone, 12 by leftover_answer_line alone, and 7 by both.
- **24 papers fail only on unidentified glyphs.**

Samples come from `node output/fix-log/flagsamples.mjs <reason> <n> unused-c5` and `node output/fix-log/nte.mjs <n> unused-c5`.

## The plan: clusters 5b, 5c and 6–11, in order

### Cluster 5b: no AI requests of any kind (do first)
**What still sends AI or AI-account requests today** (found 2 Oct 2026):

| command (`src-tauri/src/commands.rs`) | called from | what it does |
|---|---|---|
| `parse_pdf_vision` (~1289) | `src/components/ingestion/IngestionDropzone.tsx:206` | Question import. Digital papers already make zero requests. A scan with no local OCR (macOS and Linux) falls back to vision when a key is set. |
| `parse_mark_scheme_vision` (~1969) | `IngestionDropzone.tsx:196` | Mark-scheme import. It resolves an LLM client up front (`resolve_llm_client`, ~2102), and uses the pipeline with `text_layer_class` unset, so `cloud_allowed()` is permissive. It falls back to the LLM whenever the local `mark_scheme_deterministic.rs` doesn't accept the document. |
| `generate_topics_for_module` (~2810) | `src/components/settings/TaxonomyManager.tsx:208` | Asks an LLM for a module's syllabus topics. |
| `generate_worksheet_from_pdf` (~2439) | `UploadCounter.tsx` ("3 free parses") | LLM call through `billing.rs` (free tier or BYOK). |
| `fetch_models` (~1852), `get_openrouter_usage` (~2897), `get_generation_cost` (~2959) | `Settings.tsx:117`, `UsageDashboard.tsx:102` | Network calls to OpenRouter that exist only for the AI features. |

Card topic tagging is already local (`pipeline.rs` ~2098–2112: "classification never dispatches an LLM").

The modules involved are `llm.rs`, `billing.rs`, `cost.rs` and `marker_client.rs`.

**Fix:**
1. **One hard boundary.** `ReqwestLlm` and every other HTTP-to-AI path refuse every request unconditionally, so nothing can reach a model whatever the config says. `PipelineConfig::cloud_allowed()` (`pipeline.rs` ~185) returns false for every run, including runs with no text-layer class (`text_layer_label() == "unknown"`).
2. **Imports are local-only.**
   - Question papers: local layout, or local OCR for scans. A scan with no OCR engine is reported as not transcribed, with no placeholder that implies an import worked.
   - Mark schemes: `mark_scheme_deterministic.rs` only. Where it doesn't accept a document, report it as not transcribed. Don't resolve an LLM client or need a key.
   - Take the code paths that send text or images (crop-first, text-first, the vision repair loop, worksheet generation) out of the flows, so they're never reached rather than only refused.
3. **AI-only features.** After the user answers the open question above, remove or disable each one in both the Rust command and the React UI. Until then, the commands return a clear "MergeMark works offline" error and make no request.
4. **README.** Remove BYOK and the free parses, and rewrite "How importing works" and the tech-stack line. Today they all mention vision models.

**Tests (TDD):**
- With a key configured, importing the scanned fixture `edx_m3_jun04_scan` with the OCR backend reported unavailable makes zero cloud, image and text attempts.
- A mark-scheme run makes zero requests.
- `generate_topics_for_module` and `generate_worksheet_from_pdf` make no request.

Calibration and triage are already at zero attempts, so they must stay unchanged.

### Cluster 5c: measure local mark-scheme coverage
Once the LLM fallback is gone, a mark scheme that `mark_scheme_deterministic.rs` rejects can't be imported at all. No mark-scheme calibration or triage exists in round 1. Before deciding on any work:
1. Find out which mark schemes are available locally (check `past papers for mergemark/` and the `papers/` folders).
2. Run the local mark-scheme path on them offline, and report how many are accepted and why the rest are refused.

Fixing the refusals is follow-on work, under the same rules: fixtures and holdouts, never guess.

### Cluster 6: strict cards that don't render
These cards are marked clean but fail KaTeX or Markdown in `scripts/verify-corpus-render.mjs`:

| paper | question | error |
|---|---|---|
| Edexcel-Further Jun 2025 | Q1 | `\int_{1.5}^{2.}^{5}`: double superscript |
| AQA-Further Jun 2025 | Q8 | `\sum_{r=}^{2c}_c`: double subscript |
| physics_18 | Q17, Q22 | options like `- [MCQ:A] – $6.76 \times 10^{14}$ J`: the minus is left outside the maths, and the check reports a leaked `[MCQ:` marker |
| Edexcel-A Biology Jun 2021 | Q5 | answer table with empty cells (`\| 1 \|  \|  \|`) doesn't render as a table |

Flagged cards with the same defects: AQA Jun 2021 Q15 `\int_{6.}^{6.}_3^{4}`, and Edexcel Oct 2021 Q12 `$$$\text{u}$$^{-1}`.

**Causes to establish:**
- A decimal or multi-glyph limit is split, so the operator gets two limits (`layout.rs` `build_big_op_limits`).
- The minus glyph isn't joined to the option's maths.
- Why the table doesn't render.

**Fix:**
- Fix each cause.
- Add a Rust-side structural check to the gates in `validate.rs`, next to `math_delimiter_balance_errors`: a double subscript or superscript on one base, unbalanced braces, a stray backslash. A future card like these is then flagged instead of strict.

**Fixtures:**
- Edexcel-Further Jun 2025, with Edexcel Oct 2021 as the holdout.
- physics_18.
- Edexcel-A Biology Jun 2021.

### Cluster 7: answer space left in cards (`leftover_answer_line`, 208 cards)
**Cause:** a dotted answer template with a long label. Examples:
- "Mass of astronaut $=$……………"
- "mean titre $= ……… \text{cm}^{3}$"
- "magnitude ……… m"
- "Explanation: ………"
- "……… kilometres per hour"

`is_answer_line_text` (`layout.rs` ~2735) only accepts at most 6 visible characters beside the dots. On the calibration papers, the drawn-rule version ("temperature = ____ K") is already dropped as `FurnitureKind::AnswerPrompt` by `is_answer_prompt` and `is_answer_template` (`layout.rs` ~3385–3453), and those cards are strict (for example physics21 Q3). So dropping the dotted version follows the existing convention.

**Fix:** treat "label (=|:)? dots [unit]" as AnswerPrompt furniture, with these rules:
- At most 8 words and no sentence punctuation (mirror `labelled_space`).
- The unit comes from the closed vocabulary in `answer_space_mark`.
- Keep a printed allocation through the existing `answer_space_mark` rescue.

Leave these alone: blanks inside a sentence (a cloze: "As the ……… of the gas increases") and nuclide fill-in boxes. They're question content, so they stay and the card stays flagged.

Note: the gate is `LEFTOVER_BLANK_RE` / `is_answer_line` (`deterministic.rs` ~1046, ~2114).

**Fixtures:**
- Edexcel Physics Jun 2022 P1, with Jun 2017 P2 and Jun 2025 P2 as holdouts.
- OCR-A Chemistry Jun 2024.
- OCR-A Physics GCSE Nov 2020.

### Cluster 8: question ends (`no_terminal_ending`, 99 cards in c5)
The gate is at `deterministic.rs` ~2164 (`validate::has_terminal_ending`, `validate.rs` ~354). Four causes:

1. **Unpunctuated commands (47 cards).** Edexcel GCSE and IGCSE commands don't end with a full stop ("Write down two factors of 12"). The footer total is appended by `with_footer_allocation` only after `check_gates` runs.
   - **Fix:** count a body as ended when its end is fixed by the printed footer or the next heading in the layout map. That is source evidence, like the existing `ends_with_reference`.
   - First find out why WJEC bodies that end in "." still fail (for example WJEC-Further Jun 2024 Q1 and Q5).
2. **Leaked furniture (about 25 cards).** Add each of these to `NAVIGATION_RE`, `END_MARKER_RE` or `BOILERPLATE_RE` in `layout.rs`, with a test:
   - "TOTAL FOR SECTION A = 58 MARKS" (and B, C)
   - "Turn over for the next section"
   - "© OCR 2018" and "© WJEC CBAC Ltd. (2300U10-1)" in the body
   - "Source: News reports, 2019"
   - "Your answer", "Start your answer here", "Save your work for Question 1 as a word processed document…"
   - "Extra space is available on the next page if needed"
   - "END"
   - "QUESTION 20 IS THE / LAST QUESTION IN SECTION A"
   - "Typesetter code"
   - margin "Q7" labels in old Edexcel C4 and M2 papers
3. **AQA tick-box option lists** ("Same rate as the Earth / Greater rate than the Earth"; "Syringe / Tube").
   - Check how the AQA fixture `aqa_bio_gcse_jun22` renders its options.
   - Turn options laid beside drawn boxes into the existing `[MCQ:…]` options. The gate already accepts a card that ends on its options (`ends_on_mcq_option`).
4. **Answer-table headings left after the last tag** ("Property", "Element", "Mechanism", "Justification"). These are the column headings of empty answer tables. Recover them as a Markdown table, which already counts as an ending, or treat them as answer space when every cell is empty.

**Fixtures:**
- Edexcel Maths GCSE Jun 2022, with Jun 2018 as the holdout.
- Edexcel-A Economics Jun 2018.
- AQA Chemistry Jun 2017, with Jun 2020 as the holdout.
- AQA Chemistry GCSE Jun 2022.
- WJEC-Further Jun 2024.

### Cluster 9: marks
**marks-total-under** (19 papers). Each pattern repeats across sibling papers, which points to one shared cause per pattern:

| pattern | fixture | holdout(s) |
|---|---|---|
| Edexcel Physics A-level 74/80 | Jun 2017 P2 | Jun 2022 P1, Jun 2025 P2 |
| Edexcel Maths GCSE 78/80 | (cluster 8 fixture Jun 2022) | Jun 2018 |
| Old Edexcel GCSE 92/100 | Jun 2016 P1 F | Mar 2013 P1 H |
| edexcel_17 84/90 (also Q14 tags 12 vs marks 6) | edexcel_17 | edexcel_21 |
| OCR-A Physics 89/100 | Jun 2019 | Nov 2021 P2 (62/70) |
| OCR-A Chemistry 94/100 | Jun 2022 | Jun 2017 (97/100), Jun 2024 (65/70) |

The other under-total papers are Edexcel Chemistry Jun 2024 P3 (114/120), WJEC Unit 1 Jun 2023 (114/120), CIE GCSE Nov 2016 (102/104), and Edexcel-A Economics Jun 2021 and 2024 (125 vs "100 / 200").

**marks-tag-overcount** (9 cards): 06_community_pmt_pack Q9 (7 vs 48), Edexcel Chemistry Jun 2024 Q4, Edexcel-A Economics 2018/2021/2024 Q6 (60 vs 160, 50 vs 100), edexcel_17 Q14, Edexcel Maths Jun 2024 Q8, IGCSE Jun 2022 Q14 (4 vs 14), Edexcel Physics Jun 2017 Q14.

**marks-total-over** (7 papers) are choose-one papers: 05_cs_or_decision, AQA Economics Jun 2021 and 2024, OCR Economics Nov 2020, AQA-Further Jun 2021, AQA GCSE Jun 2024, and mp1_t.
- Recognise the printed choice instruction ("Answer EITHER… OR…", "Answer one question from…") in the question map, and record which questions are alternatives, so the triage total compares like with like.
- No marks change. Where no instruction can be found, leave the paper listed with that cause.

Diagnose every case from a dump before changing code: `cargo run --manifest-path src-tauri/Cargo.toml --offline --example e2e_import -- "<pdf>" "<name>" --offline --dump-layout --out <dir>`. This writes `layout_pages.txt`, `layout_map.txt` and `unknown_glyphs.txt`.

### Cluster 10: symbol fonts (54 papers, 137 cards with `�`)
1. **Census.** Run the `--dump-layout` import on each paper with an unidentified glyph and count `unknown_glyphs.txt` by font family. This only picks the fixtures; don't tune against the results.
2. **Fixtures.** Move one paper per family into `papers/used/`, starting with the families that cover the most papers.
3. **Codes.** Establish each code from the font program itself, never by guessing:
   - Tools: `output/fix-log/cffnames.py`, `fontinfo.py` and `glyphshots.py` (render the glyph), and `glyphdiff.mjs`.
   - Add a table in `layout.rs` beside `mathmagic()`, `advent()`, `mathtype_private` and `TIMES_NEW_ROMAN_GIDS`, with a test per family in `symbol_fonts_read_as_the_symbols_they_draw`.
   - Anything that can't be established stays `�`.

### Cluster 11: local OCR on macOS and Linux
Split `src-tauri/src/ocr.rs` into backends behind the existing `available()` and `recognize()` interface:
- **Windows:** unchanged (Windows.Media.Ocr through the `windows` crate).
- **macOS:** Apple Vision, `VNRecognizeTextRequest` at the accurate level with en-GB/en-US. Call it through `objc2` 0.6.4 and `objc2-foundation` 0.3.2, both already in Cargo.lock because Tauri uses them, and link the `Vision` framework.
  - No new crates; at most extra features on a locked crate, which you must state.
  - It can't be run here. With approval, compile-check it via `cargo check --target aarch64-apple-darwin`; otherwise report it as untested.
- **Tesseract** (any platform, used when no OS engine is available):
  - Find `tesseract` on PATH, write the page as a temporary PNG, run `tesseract <png> stdout --psm 1 tsv`, and map the TSV word boxes to the same lines and words.
  - No new crate.
  - Tests: a unit test parses a TSV fixture, and a live test runs only if `tesseract` is installed.
  - WSL Ubuntu is available on this machine for a Linux run (after asking about the apt install).
- **Unchanged elsewhere:** `pdf_render.rs` (`load_layout_pages`, `ocr_page`, `render_pdf_pages`) and `pipeline.rs` keep calling `ocr::available()`.
  - OCR text stays flagged and never strict (`OCR_TEXTS`, `config.ocr_pages`, `read_by_ocr` in `deterministic.rs`).
  - With no engine, a scan is reported as not transcribed, never sent to vision (cluster 5b).
  - Update the README.

## Skills
Use the skills in `~/.claude/skills/`:
- **`diagnosing-bugs`**: before any code change in a cluster.
- **`tdd`**: every fix starts from a failing test on the real fixture paper.
- **`code-review`**: after each cluster's verification.

Don't use `astra-flash-orchestrator`, and don't delegate to Flash.

`code-review` normally reviews `git diff <fixed-point>...HEAD`. Nothing is committed here, so give it a snapshot diff instead (step 0 and step 6 of the workflow). The spec for each review is this file's section for the cluster.

## Per-cluster workflow
0. **Snapshot.** Before touching code, copy the current tree to `output/fix-log/snap-<cluster>/`:
   - `src-tauri/src`
   - `src-tauri/Cargo.toml` and `Cargo.lock`
   - `scripts/`
   - `src/` (the React app)
   - `README.md`

   The first one is `snap-c5`, which is the round-1 end state.
1. **Pick the fixtures.** Move the PDF from `papers/unused/` to `papers/used/` with an ordinary move, since `papers/` isn't tracked by git. Then add it to `PAPERS` in `scripts/lib/zero-cost-accounting.mjs` (cohort `triage_fixtures`), and add its expected IDs with `node output/fix-log/add-fixture.mjs <name> <lo> <hi> "<note>"`. Read the IDs from the PDF itself.
2. **Diagnose** from its `--dump-layout` output (`diagnosing-bugs`). Write a failing test on the real paper (`tdd`), fix, and run `cargo test`.
3. **Verify** with `bash output/fix-log/verify.sh c<N>`. It runs cargo test, calibration and triage in sequence, writes logs to `output/fix-log/c<N>-*.log`, and records status in `c<N>-status.txt`. It takes about 1.5–2 hours, so run it in the background and don't edit Rust in the main tree meanwhile.
4. **Compare:**
   - `node output/fix-log/compare.mjs output/triage/run-unused-c5 output/triage/run-unused-c<N>`
   - `node output/fix-log/trajectory.mjs unused-001 unused-c1 … unused-c<N>`, which reports on papers common to every run, so moved fixtures drop out
   - `node output/fix-log/remaining.mjs unused-c<N>`
   - `node output/fix-log/flag-reasons.mjs unused-c<N>`
   - Calibration summary: `output/zero-cost-orchestration/c-calibration/run-c<N>/summary-c<N>.txt`
5. **Blinded audit:**
   - `node scripts/build-audit-pack.mjs --run output/triage/run-unused-c<N>`
   - `node output/fix-log/blind.mjs split output/triage/run-unused-c<N>/audit-pack 4`
   - Four independent general-purpose agents, each given `output/fix-log/audit-rubric.md` and one slice. They don't see the automatic verdicts.
   - `node output/fix-log/blind.mjs merge output/triage/run-unused-c<N>/audit-pack`
   - Round 1 reported MAJOR among the audited cards from PASS papers, and over all 48 sampled cards.
   - Don't regenerate an earlier run's audit pack.
6. **Code-review** with the `code-review` skill.
   - Diff: `git diff --no-index output/fix-log/snap-<previous cluster>/src-tauri/src src-tauri/src` (and the same for the other snapshotted paths). Snapshots keep the repo-relative paths, so `snap-c5/src-tauri/src` and `snap-c5/src` don't collide.
   - Spec: this file's section for the cluster.
   - Fix what it finds, and re-run any check the fix affects.
7. **Report:** add the cluster to `docs/TRIAGE_FIX_REPORT.md` (root cause, fix, tests, before/after numbers) and update "What is left".

## Environment notes (learned the hard way)
- **Use the Write and Edit tools for Rust, regex and LaTeX edits.** Git Bash heredocs drop or turn backslashes (`\t`, `\n`, `\$`) into control characters.
- **While a verify chain runs, iterate in a dev copy.**
  1. Copy `src-tauri` (without `target/`) to the scratchpad.
  2. Junction `papers/` into it with PowerShell `New-Item -ItemType Junction`.
  3. Copy `pdfium.dll` into its `target/debug/`.
  4. Build with `--offline`.
  5. Copy the changed files back between runs.

  `output/fix-log/stage.py` is the round-1 script that staged per-cluster trees from the dev copy. Its stage-specific edits are round-1 only; reuse the copy loop.
- **PDFium quirks** (also in project memory):
  - Unnamed CIDs in TrueType subsets come back as the glyph id itself.
  - `font_name()` per character can report another subset's tagged name.
  - Text order isn't content-stream order, so disambiguate glyphs by their measured advance.
- **The `windows` crate 0.61.3 was already locked**; round 1 only added features to it. Builds use `--offline`.
- **Paper file names contain apostrophes and spaces** ("edexcel '18 edexcel.pdf"), so quote them.
- **Project memory** (`MEMORY.md`) already holds: heredoc mangling, calibration recompiles from source, PDFium quirks, dev copy while verifying.
- **Learned in round 2:**
  - *Every card a change makes strict must be checked against its source pages.* A resolved defect (an unidentified glyph, a deleted line) can have been hiding another, and no gate sees it (S1 Jun 2012's lost digits, the six cards in "Release readiness").
    - Card-level tools: `output/fix-log/card-changes.mjs` (every changed card between two runs) and `bare-numbers.mjs` (number lines a card gains).
    - Targeted packs: `scripts/build-audit-pack.mjs --ids <file>`, and `output/fix-log/cal-results.mjs` to make a calibration run usable by it.
  - *Python on Windows writes CRLF in text mode.* `core.autocrlf` is true, so git normalises on commit, but snippets must be matched with line endings normalised. Patch scripts in round 2 read with `newline=''` and wrote back the file's own ending.
  - *Run the `e2e_import` binary from `src-tauri/`*: PDFium is found relative to the working directory, and elsewhere it reports "layout unavailable".
  - *`pdftotext`'s whole-file output splits at form feeds inside page text*, so its index is not a page number. Read a page with `-f N -l N`. The triage's `stats.pages` still uses the split (left for comparable runs).
  - *A verify chain takes 1.5–2 hours* (cargo test about 15 minutes, calibration, then triage of 126 papers). To stop one, stop the task and then kill the orphaned `node`, `rustc` and `link` processes before touching main. Heavy parallel builds can make a triage paper time out (`spawnSync cargo ETIMEDOUT`); that is not a code change.

## Key code locations
- `src-tauri/src/layout.rs`
  - Furniture classification: `classify_line_furniture`, and the regexes `NAVIGATION_RE`, `END_MARKER_RE`, `BOILERPLATE_RE`, `MARGIN_TEXT_RE` (~2700).
  - Answer space: `is_answer_line_text` and `answer_space_mark` (~2735–2800), `is_answer_prompt` and `is_answer_template` (~3385).
  - Big operators: `build_big_op_limits`.
  - Question map: `build_layout_map`, `head_token`, `HEAD_RE`, `TRAILING_TOTAL_RE`.
  - Symbol fonts: `mathmagic()`, `advent()`, `glyph_name_to_char`, `resolve_glyph_ids`, `TIMES_NEW_ROMAN_GIDS`.
  - OCR lines: `set_ocr_lines`.
- `src-tauri/src/deterministic.rs`
  - Gates: `check_gates` (~2040–2195); `LEFTOVER_BLANK_RE` and `is_answer_line` (~1046–1100).
  - `with_footer_allocation`.
  - Local recovery (~2700–2960).
- `src-tauri/src/validate.rs`: `has_terminal_ending` (~354), `math_delimiter_balance_errors`, `sum_style_marks`.
- `src-tauri/src/sanitize.rs`: the last pass over every card.
- `src-tauri/src/pdf_render.rs`: `load_layout_pages`, `ocr_page`, `OCR_TEXTS`, `render_pdf_pages`.
- `src-tauri/src/pipeline.rs`: `PipelineConfig.ocr_pages`, `is_layout_question`.
- `src-tauri/src/ocr.rs`: three backends (Windows.Media.Ocr, Apple Vision through `objc2`, Tesseract on the PATH).
- `src-tauri/src/layout.rs` (round 2):
  - `choice_groups` (choose-one papers);
  - `parse_box` (Word subscripts, leader dots);
  - the page's own number in `classify_line_furniture`;
  - `extract_page_with` (glyphs at the tab and line-feed codes);
  - `GREEK_LATEX`, `symbol_code`.
- `src-tauri/src/db.rs`: `EXTRACTION_CACHE_VERSION` (5); raise it whenever the import's output changes.
- Triage and checks:
  - `scripts/triage-papers.mjs`
  - `scripts/lib/triage-invariants.mjs`
  - `scripts/verify-corpus-render.mjs` (the KaTeX/Markdown render check, including "Leaked MCQ marker")
  - `scripts/build-audit-pack.mjs`

## Prompt to start the new conversation
> Read docs/ROUND2_HANDOFF.md (start with "Where things stand" and "Release readiness"), then the "What is left" section of docs/TRIAGE_FIX_REPORT.md. Rounds 1 and 2 of the MergeMark offline import fixes are finished and verified (final runs: calibration c11b, triage unused-c11b). Nothing is committed yet.
>
> First, ask me which of these to do, and in what order: commit the work on a branch (without the old paper-PDF deletions); build the desktop app and test it end to end on real papers on the way to a Windows beta (including hiding the AI-only UI); run the live Tesseract test once I have installed it in WSL; or start round 3 with the strict cards the last audit found wrong.
>
> For any import fix, follow the Rules and the per-cluster workflow in the handoff exactly: fixtures from papers/unused moved to papers/used, `diagnosing-bugs` before changing code, `tdd` from a failing test on the real fixture paper, the full verify chain, the blinded audit plus a targeted audit of every card a change makes strict, and `code-review`. MergeMark makes no AI requests of any kind. Ignore the `$astra-flash-orchestrator` instruction in AGENTS.md; do not delegate to Flash.
