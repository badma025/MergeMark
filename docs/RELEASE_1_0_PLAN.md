# MergeMark 1.0 — plan and handoff

> **Start here.** This is the active plan for MergeMark 1.0, approved 5 Oct 2026. It assumes no memory of
> the session that produced it. Background on how the import engine got here is in
> [`TRIAGE_FIX_REPORT.md`](TRIAGE_FIX_REPORT.md) (clusters 1–14, round by round) and
> [`ROUND2_HANDOFF.md`](ROUND2_HANDOFF.md) (working rules, environment notes, key code locations).
> Where those two disagree with this file, this file is newer — see the corrections noted in W0 and W5.

## Context

MergeMark imports past-paper PDFs into question cards, entirely offline — no AI requests of any kind, no network dependencies in the crate graph. On the 22 calibration papers it has been tuned against it produces 243 of 244 strict cards. It is still not releasable to real users:

1. **A card can pass every automated check and be wrong.** Round 3 fixed the six wrong strict cards the cluster 10 audit found; its own audits immediately found six more. A card with no REVIEW badge reads to a student as "this is correct".
2. **Nothing lets a user notice.** The `questions` table stores `paper_name` and `question_number` and nothing else — no PDF path, no page. A wrong card is unfalsifiable from inside the app.
3. **It cannot be shipped or maintained.** No updater, macOS never run, dead AI code still on the IPC surface, and a test suite that is silent on any machine but the author's.

The v0.9.4 tester release is shelved. Everything targets a single 1.0.

## Decisions already taken

| | |
|---|---|
| Definition of done | **Convergence** — a round whose blinded audit finds no new wrong strict cards |
| Release shape | One 1.0. No interim releases |
| Source verification | **In scope** — new feature, deliberately |
| Platforms | Windows + macOS, **both unsigned**. A Mac is available for testing |
| Mark schemes | Measure the real failure distribution first, then decide |
| Coverage | Correctness + bounded wins. **VectorOutline out of scope** |
| Updater | Yes, in 1.0 |
| Dead AI code | Remove properly |
| Git history | Keep. Rotate/accept rather than rewrite |
| Working rhythm | Run the plan end to end. Stop only at the points listed under Verification |
| Audit | I judge blinded slices; **the user independently re-judges ~5 CLEAN cards per round** |

Baseline to beat (audit c13): **16 MAJOR of 46** sampled; **1 MAJOR of 14** among strict cards from passing papers.

---

## W0 — Make the test suite mean something

Do first. Every later claim is measured with this.

**The problem is larger than the docs say.** `.gitignore` excludes the whole `papers/` tree, so on a clean checkout roughly **110 fixture guards silently `return`** — 82 in `layout.rs`, 16 in `pdf_render.rs`, 9 in `stroke_census.rs`, 7 in `commands.rs`, 5 in `ocr.rs`, 4 each in `doc_map.rs` and `pipeline.rs`. `release.yml` runs no test step at all. "378 passing" is true on one machine.

- Commit a **small committed fixture set** (or Git LFS) sufficient for the core guards.
- Add `MERGEMARK_REQUIRE_FIXTURES=1`, under which a missing fixture **fails** instead of returning, and set it in CI.
- Add a test step to `release.yml`.
- **Correction to the handoff docs:** the "nine dead tests" were already repointed at `papers/used/` by round 3 (commit `4414b74`). Only one genuine orphan remains — `doc_map.rs:1666` reads `'23 edexcel.pdf`, which exists nowhere in the repo; delete it or supply the paper. `stroke_census.rs:1730` is `#[ignore]`d, a manual helper.
- `stroke_census.rs:1505` has also never run — there is no `src-tauri/fixtures/figure_golden/`, so the IoU ≥ 0.85 figure gate is untested.
- Commit the **17 root PDF deletions** (exam-board copyright material, currently live files on a public repo).
- Move the audit toolchain out of gitignored `output/fix-log/` into `scripts/`: `blind.mjs`, `compare.mjs`, `add-fixture.mjs`, `card-changes.mjs`, `trajectory.mjs`, `flag-reasons.mjs`, `verify.sh`. The apparatus that defines "done" is one deleted folder from gone.

## W1 — Wrong strict cards

The actual release gate. Per cluster, the rhythm rounds 1–3 established:

> promote the paper to a fixture (`add-fixture.mjs`) → diagnose from `--dump-layout` → failing test on the real paper → fix → verify chain → audit **every** card that newly turns strict

The six known, from `TRIAGE_FIX_REPORT.md` "What is left":

| paper | symptom | likely area |
|---|---|---|
| Edexcel M2 Jan 2009 Q2 | `\text{is}^{\frac{11}{25}}` — fraction hung on a word as an exponent | `build_fractions` |
| WJEC Physics Jun 2022 U3 Q2 | `\frac{1}{3}Nm^{\bar{2}}c` — bar lands on the square | overline/script attachment |
| Eduqas CS Jun 2023 Q7 | underline marking a primary key is lost | underline detection |
| Edexcel-A Economics Jun 2021 Q8 | answer-booklet furniture follows the total — **on a PASSING paper** | page-furniture rejection |
| CIE S1 Jun 2019 Q2 | "from Rosa's box." loses "box" | word assembly |
| Edexcel C4 Jan 2006 Q2 (fixture) | x/y table with blank cells flattened to lines | table reconstruction |

Then re-audit and repeat. **Expect new ones** — resolved defects unmask others, which is why the bar is convergence, not a checklist.

## W2 — Source-page verification

What makes residual error survivable instead of silent.

- **Schema:** `questions` gains the source PDF path and a page span, following the existing additive `ALTER TABLE … ADD COLUMN` pattern in `db.rs` so existing libraries survive.
- **Pipeline:** carry the page span the layout map already computes through to the saved card.
- **UI:** a source control on `QuestionCard.tsx` (which already owns the REVIEW badge and inline editing), rendering through the 300-DPI rasteriser in `pdf_render.rs` — the machinery that already crops figures.
- Handle a moved or deleted source PDF explicitly.

## W3 — Coverage: the bounded wins

Flagged cards are honest, not wrong — this is usefulness, not correctness.

- **Symbol fonts:** a fixture per remaining family (Advent variants, MathExtn, NewMathExtn, CIDFont). Cluster 10's method only — codes established from the font program, the rendered outline, or the installed font's cmap, with the declared advance. Never guessed. **VectorOutline (8 papers, symbols drawn as curves) stays out**; it needs shape matching.
- **Marks:** 10 papers — 3 under printed total, 4 choose-one over total, 3 with tags exceeding card marks. Diagnosed as cluster 9 did.
- **3 render errors**, unchanged since cluster 6.
- **WJEC Physics Unit 5 Jun 2024** prints no question numbers and yields no cards.

## W4 — Mark schemes: measure, then decide

`mark_scheme_deterministic.rs` is a separate 1,235-line parser sharing no code with the 9,336-line `layout.rs`. It accepts a document only if none of **24** refusal conditions fires, and it **returns on the first one** — so the recorded census is seven single data points and nobody has seen past page 1 of most schemes.

**Step 1 (cheap — do before committing to anything):** instrument `extract` to collect all refusal reasons per document, re-run the 7-scheme census, get a real distribution.

**Step 2, the decision gate:**
- If the mass is the preamble and rotation gates → **tune**. Rotation is a drop-in: adopt the `Frame` type `layout.rs` already uses to normalise `/Rotate`. Loosen the preamble predicate. Port the glyph-identity recovery stack for the two unmapped-glyph schemes.
- If the mass is the formula rule — **any cell containing `=` `^` `_` `∫` `√` `∑` `∏` refuses the whole document**, and maths schemes are wall-to-wall equals signs — this is a missing capability, not a loose gate. The honest options are rebuilding on `layout.rs`'s `TableGrid` and glyph stack, or cutting the feature from 1.0.

**Decided 5 Oct 2026: cut from 1.0** (after step 1, [`MARK_SCHEME_CENSUS.md`](MARK_SCHEME_CENSUS.md): tuning alone would accept none of the 7 schemes). The import UI and its commands are removed; the reader stays outside the app for a rebuild.

Two traps: the reader does **no CropBox normalisation**, which will bite on the Edexcel schemes the moment the cover gate opens; and the single real-PDF test **asserts the refusal**, pinning the bug in place.

## W5 — Ship-ability

**Dead AI surface.** The orphan React components (`UsageDashboard.tsx`, `UploadCounter.tsx`) are tree-shaken and *not* in the bundle — but **11 Tauri commands are compiled in and live on the IPC surface**: 7 stubs returning `WORKS_OFFLINE` (`fetch_models`, `generate_worksheet_from_pdf`, `get_usage_status`, `set_byok_key`, `generate_topics_for_module`, `get_openrouter_usage`, `get_generation_cost`) and 4 implemented but unreachable (`get_import_cost_history`, `clear_import_cost_history`, `delete_import_cost_log`, `prune_orphaned_import_logs`). Delete these with their `lib.rs` registrations; the test `ai_only_features_answer_that_mergemark_works_offline` must go in the same change. Fix the stale `TaxonomyManager.tsx` string still mentioning an "Auto-Topics" button that no longer exists. Keep `parse_mark_scheme_vision` / `parse_pdf_vision` — misleading names, live local commands; rename rather than delete.

**Legacy key on disk.** A `usage_config` table with `byok_api_key` in plaintext survives in databases written by pre-offline builds. Nothing reads it, nothing drops it. Add a migration that drops the table.

**Version reconciliation, before the updater.** `Cargo.toml` says `1.0.0`, `tauri.conf.json` and `package.json` say `0.9.4`. Tauri takes the bundle version from the config but `env!("CARGO_PKG_VERSION")` yields the other. The updater compares versions, so fix this first. Note `pdf_render.rs` embeds `CARGO_PKG_VERSION` in the stroke-census `generator` field, so changing it perturbs artifacts calibration diffs — do it deliberately with a fresh calibration run.

**Updater.** `tauri-plugin-updater` + `@tauri-apps/plugin-updater`; `updater:default` in `capabilities/default.json`; `npx tauri signer generate`, pubkey into config, private key as a CI secret; `createUpdaterArtifacts: true`; an endpoint. Note `/releases/latest/download/latest.json` requires a **published, non-prerelease** release — `release.yml` currently sets both `releaseDraft: true` and `prerelease: true`. A frontend `check()` / `downloadAndInstall()` call does not exist yet.

**macOS.** Bundle `libpdfium.dylib` via a `tauri.macos.conf.json` — today only `tauri.windows.conf.json` ships the library and binding is `Pdfium::bind_to_system_library()`. Run the Vision OCR backend for the first time. Test the `.dmg`. Unsigned, so document the Privacy & Security → Open Anyway step prominently; it is not a dismissible warning like SmartScreen. Re-add the macOS job to `release.yml` once it can actually import.

**Worksheet and flashcards.**
- CSV export reuses the *print* formatter (`format_markdown_for_latex`). The leak is worse than documented — not just `\needspace` and `\hfill` but the whole macro set, including **absolute local filesystem paths** inside `\includegraphics` for any card with a diagram. The seam already exists: split `sanitize_for_latex` at the step-7/step-8 boundary into content repair (wanted everywhere) and LaTeX-ification (print only), and give cards their own formatter. The PDF path stays byte-identical so the worksheet golden tests are untouched.
- The MiKTeX-install dialog in `WorksheetBuilder.tsx` waits for a `PDFLATEX_NOT_FOUND` sentinel **the Rust side never emits**, so the most likely first-run failure shows a raw toast instead. One line.
- Compile into a temp directory and copy out only the two PDFs. That fixes `mergemark_logo.png` orphaned in Downloads *and* the `.tex`/`.aux`/`.log` litter every error path leaves behind.
- Pass `CREATE_NO_WINDOW` to the four `pdflatex` spawns; they currently flash console windows in release builds.

**Data safety.** `EXTRACTION_CACHE_VERSION` is safe to bump — it only changes a cache key, the `questions` table is untouched (old cache rows are orphaned forever, worth a sweep). The real risks are elsewhere: `init_db` has **no backup before migrating**, `lib.rs` `.expect()`s on failure so a bad migration panics with no message, and the dedupe at `db.rs:126-152` **silently deletes rows a user may have hand-edited**, keeping only `MAX(rowid)` per `(paper_name, question_number)`. Copy `mergemark.db` to `mergemark.db.bak-<version>` at the top of `init_db`, and replace the panic with a dialog naming the backup.

---

## Verification

Per cluster, unchanged from rounds 1–3:

```bash
bash scripts/verify.sh <stamp>    # cargo test, calibration, triage
```

- **cargo test** — 378 passing today; every fix adds a test that was red first
- **Calibration** must stay 22/22 fresh, **243/244 strict**, 69/69 test-paper, **zero network attempts**
- **Triage** compared against the previous run; PASS rises, MAJOR falls
- **Targeted audit** of every card that changed or newly turned strict
- **Blinded audit** via `blind.mjs` slices; the user re-judges ~5 CLEAN cards. Disagreement means the bar is not met
- `code-review` before each verify chain

### Where to stop

Work continuously through W0 → W4 step 1 → W1 rounds, interleaving W2, W3 and W5, and
report at each workstream boundary without waiting for a reply. Stop and ask only here:

1. **The W4 step-2 decision** — tune, rebuild on `layout.rs`, or cut mark schemes from 1.0.
   It changes scope materially and is not yours to take alone.
2. **Every blinded audit.** Do not audit your own fixes: a session that wrote a change
   cannot judge it blind. Build the audit pack, then stop and say it is ready — the user
   opens a separate session to judge it, and re-judges ~5 CLEAN cards.
3. **macOS** — the Vision OCR backend and the `.dmg` need the user's Mac. Prepare the
   bundling and then hand over.
4. **Declaring convergence**, and therefore 1.0.
5. **Anything outward-facing** — pushing, tagging, publishing, or touching the public repo.

Also stop if a fix would require relaxing a gate, or if a diagnosis turns out to need a
paper that is not on disk. Both mean the plan met something it did not anticipate.

**Non-negotiables.** Never guess content — unresolvable stays `�` and flagged. Never relax a gate, threshold or expected count to move a metric. Never add a gate on holdout evidence. A paper fixed against moves to `papers/used/` and becomes a fixture, registered in `zero-cost-accounting.mjs` and `source-expected-ids.json`. Never edit Rust while calibration or triage is running. Do not delegate implementation to another agent runner.

## Risks

- **Convergence may be slow.** Round 3 fixed six and found six; the discovery rate has not yet fallen. The one item with no predictable end.
- **Mark schemes may be a rewrite.** W4 step 1 exists to find that out cheaply.
- **macOS unsigned** loses a share of users at Gatekeeper. Accepted deliberately.
- **The audit is human reading**, and it paces the project.
- **16 exam-board PDFs remain in git history** on a public repo (`.git` is 221 MB). A decision was taken not to rewrite history; revisit if that becomes untenable.

## Sequencing

W0 → W4 step 1 (so the mark-scheme decision lands early, while it can still change scope) → W1 rounds, with W2, W3 and W5 interleaved between verify chains, since those are long and the dev tree is free while they run.

---

## Where things stand right now

*Updated 11 Oct 2026.*

- Every workstream is done: W0, W2, W4 (mark schemes cut from 1.0), W5, and W1 rounds 1–4 with W3 batches 1–2, each recorded in `TRIAGE_FIX_REPORT.md`. They are on `import-fixes-rounds-1-2` and `master`, pushed on 10 Oct 2026.
- The blinded audits w1r3 and w1r4 found no MAJOR card, and the user declared convergence on 10 Oct 2026 (Where to stop, 4). The release starts with dry runs, tags `v1.0.0-rc.N`, each failed one's tag removed; then `v1.0.0`. **A candidate's draft is ready when:**
  - both jobs succeeded, and its `latest.json` lists `windows-x86_64` and both `darwin-aarch64` and `darwin-x86_64` (with `fail-fast: false`, a failed job leaves the other's alone);
  - every signature carries key ID `51088306F548AC07`;
  - the user has installed both downloads: Windows; and on the Mac the universal disk image, the README's "Open Anyway" steps, a digital paper, and the scanned M3 Jun 2004 paper giving seven questions (`docs/MACOS_TEST.md`, the steps left).
- **The dry runs so far:**
  - **rc.1:** Windows built, but signing failed: the key 0.9.1 and 0.9.2 trust (A59B…) can no longer sign, its password lost. macOS failed `scanned_pages_are_read_by_local_ocr_and_never_clean`, with questions 3–7 for 1–7.
  - **rc.2:** a new key, but the password secret was wrong (typed in `gh`'s hidden prompt). The test now names what the OCR read: Vision on GitHub's runner, a virtual Mac with no neural engine, dropped the margin labels "1." and "2.", which the user's Mac (macOS 26, like the runner) and Windows read.
  - **Fixes since:**
    - `scripts/check-signing.mjs` (`gh workflow run signing-check.yml`, and the first step of every release build) tells in two minutes whether the secrets sign with the app's key.
    - On macOS, the margin is read again by itself, for question numbers the whole page's reading dropped. A census on the runner found no Vision setting that reads them. The test passes on the runner with the fix (temporary branch `ci/vision-census`).
    - Code review of all of it: `output/fix-log/review-release-dryrun-findings.md`.
  - **rc.3:** the secrets set by `scripts/set-signing-secrets.mjs` signed (the signing check passed on GitHub). Windows built and signed with key `51088306F548AC07`. On macOS every Rust test passed (456: the Windows-only one does not run there), but three of the scripts' tests failed: they need pdftotext, which Git for Windows ships. They now run in the Windows job only.
  - **rc.4 (`5248cdc`):** both jobs succeeded. The draft holds the Windows installer, the universal disk image and the macOS update bundle. Its `latest.json` lists `windows-x86_64` and both `darwin-aarch64` and `darwin-x86_64`, and every signature carries key `51088306F548AC07`. Superseded by rc.5 before the installs finished.
  - **rc.5:** the user's testing of rc.4 and an end-to-end run of the app (11 Oct 2026, the real desktop build driven over WebView2's DevTools port, on its own app identifier) found faults outside the import engine. Fixed and committed:
    - adding a question by hand always failed (`add_question` required `needsReview`), and the dialog started on a subject that does not exist;
    - a deleted question stayed on the worksheet;
    - the editor's preview showed no diagrams;
    - the card's action icons did not fit, so they moved to a ⋯ menu, and Delete now takes a second click;
    - on the worksheet PDF, a figure-first question's number printed under its figure, printed bullets ran into one line, and a part's mark followed the next part's lead-in;
    - smaller things: the topic filter appears once a subject is chosen, long equations shrink to their card, and worksheet lines name their paper.

    The stray "⎡" it found on a strict card is under "What is left" in `TRIAGE_FIX_REPORT.md`, for 1.0.1. Both jobs succeeded, every signature carries key `51088306F548AC07`, and the user installed it on Windows and the Mac (11 Oct 2026).
  - **rc.6:** the user asked for batch import before 1.0, having seen imports finish in about a second (0.6–2.5 s a digital paper, uncached). The import screen takes several files from the picker or a drop. They import one after another (the backend runs one extraction at a time) under the subject and module on screen, and end with a single summary; Cancel stops the batch. The checklist above applies to rc.6 in full, including the user's installs. Then: remove the rc.4, rc.5 and rc.6 drafts and tags, tag `v1.0.0` at rc.6's commit (its `latest.json` names the tag in every URL, so a draft cannot be retagged), check its draft the same way, and the user publishes it.
- **1.0.1, proposed (11 Oct 2026, not yet designed):** topics for imported questions, and the "Auto-Topics" button back. The user is weighing a cloud model (an exception to the no-AI rule) against offline options: syllabus keyword signs, a model trained on papers labelled at development time (it cannot place a subject it was not trained on, so it does not cover user-made subjects), and a small embedding model on the computer that compares a question with any topic list, learning from the user's own tags. Measure on a labelled set before choosing.
- The macOS test passed on the user's Mac on 10 Oct 2026 (`docs/MACOS_TEST.md`), and the macOS job is back in `release.yml`. The margin re-read came after it: on a real Mac it is checked with the release candidate, as above.
- **Updater signing key: new in 1.0**, key ID `51088306F548AC07` (`~/.tauri/mergemark-1.key` on the user's PC, its password in their password manager). The key secret holds it (the signing check matched its fingerprint). The password secret, set twice by hand (`gh`'s hidden prompt, then GitHub's web page), did not open it; `scripts/set-signing-secrets.mjs` sets both from the key file and the password typed once, after checking locally that they sign with the app's key. The key 0.9.1 and 0.9.2 trust (A59B…) can no longer sign. Those betas show the release text in their update prompt, and it tells their users to answer No and install 1.0 by hand. 0.9.3 has no update address. Before a release is published, check that its signatures carry the new key ID.
- `cargo test` with `MERGEMARK_REQUIRE_FIXTURES=1`: 455 passing at round 4; the dry run's fixes add two OCR unit tests. 99 papers are in the encrypted test set, and 96 question papers are left in the holdout. The faults still known are under "What is left" in `TRIAGE_FIX_REPORT.md`.
- Local tags `v1.0.0-byok` and `v1.0.0-free` are old and not on the remote: push a release tag by name, never with `--tags`.
