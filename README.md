<div align="center">
  <img width="600" alt="MergeMark Logo" src="https://github.com/user-attachments/assets/98b98209-4c32-4677-be8b-57964f845a95" />

  **A privacy-first desktop engine that instantly transforms dense academic past papers into clean, customizable question cards.**

  [![Version](https://img.shields.io/badge/version-1.0.0-blue.svg)]()
  [![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS-lightgrey.svg)]()
  [![License](https://img.shields.io/badge/license-All%20Rights%20Reserved-red.svg)](LICENSE)
</div>

---

## ⚡ What is MergeMark?
MergeMark is a local-first desktop application built to replace manual data entry for students and teachers. Instead of spending hours retyping complex typography, matrices, and physics equations, you simply import a PDF past paper. MergeMark's engine parses the document layout and extracts it into beautifully formatted, isolated markdown and LaTeX modules.

Currently optimized for **GCSE and A-Level Mathematics & Further Mathematics**, and tested on A-Level Physics and Computer Science papers.

## ✨ Key Features
* **Zero-Friction Parsing:** Drag and drop full exam papers or question booklets. The engine automatically isolates distinct questions from the layout.
* **Fully Offline Import:** Every paper is read on your machine. MergeMark makes no AI requests of any kind, needs no API key and costs nothing. See [How importing works](#-how-importing-works).
* **Check any card against its paper:** each card records the PDF and the pages it came from. Its source button shows those pages, so a card can always be compared with what the paper prints.
* **100% Local & Private:** Built on a local SQLite database. Your files, extractions, and data live strictly on your machine and are never uploaded anywhere. The one request MergeMark makes is the [update check](#updates).
* **Markdown & LaTeX Ready:** Export extractions instantly into your preferred knowledge management workflows (like Obsidian) or use them to compile targeted topic tests.

## 🛠️ Tech Stack
This application is engineered for maximum performance and a minimal memory footprint:
* **Core/Backend:** Rust & Tauri
* **Frontend:** React & TypeScript
* **Database:** SQLite (Local)
* **PDF engine:** PDFium (via `pdfium-render`) for glyphs, fonts, vector paths and page rendering
* **Intelligence:** A local geometry-first layout engine for digital papers; the operating system's own OCR engine for scanned papers (Windows and macOS, verified; the Linux backend exists but is not yet run-tested). No AI models.

## 🔍 How importing works
Everything is imported **on this machine**. MergeMark has no AI service to send anything to: imports make no requests, need no API key and cost nothing.
* **Question papers with a text layer** (almost every board PDF) are rebuilt from the PDF itself, in the four stages below.
* **Scanned papers** (pages printed as pictures or outlines, with no text layer or only a watermark) are read on this computer: on Windows by the operating system's own OCR engine, on macOS by Apple's Vision framework, and elsewhere by Tesseract when it is installed (`tesseract` on the PATH, for example `sudo apt install tesseract-ocr`). The Windows and macOS paths have been run against real papers; Tesseract is untested. OCR reads words, not mathematics, so every question it produces is kept for review. Where no local OCR engine is available, a scanned paper is reported as not transcribed.
* **Mark schemes are not imported in 1.0.** The local reader could not read the schemes it was measured on ([`docs/MARK_SCHEME_CENSUS.md`](docs/MARK_SCHEME_CENSUS.md)), so the feature is left out rather than shipped half-working. A question's answer can be typed into its card.

A local import runs in four stages:
1. **Layout** (`src-tauri/src/layout.rs`). Reads every glyph's position, baseline, font and size from PDFium, together with the page's rules and vector paths. From these it rebuilds reading-order lines and the maths itself: fractions, radicals, scripts, sums and integrals with limits, matrices, piecewise definitions, vector arrows. It also rebuilds tables and code listings, and sets aside page furniture (headers, footers, margin text, barcodes, repeated QR codes). Font codes with no usable Unicode are resolved from the font's own tables; anything it can't identify is kept as `�` and flagged, never guessed.
2. **Question map.** Finds question headings, part labels, mark allocations, section breaks and reference material (for example, an instruction-set table "included so that you can answer…"), so each card holds exactly one question.
3. **Figures.** Diagrams, graphs and formula pictures are found from the drawn strokes and images, cropped together with their labels, and placed where they are printed. Labelled diagram options become multiple-choice options.
4. **Checks** (`src-tauri/src/deterministic.rs`, `validate.rs`). A card counts as a clean import only if its marks add up, it ends where the question ends, every figure is attached and no glyph is unidentified. Anything else is still imported, but flagged for review. Nothing is silently relabelled as a success.

On the 18 historical papers used for testing (Edexcel, AQA, Cambridge and practice papers across maths, physics and computer science), 243 of 244 questions import cleanly, and all 69 test-paper questions do. The one exception is flagged because its source draws a symbol as an outline with no character behind it. All of this runs with zero AI requests and zero cost.

### Verifying the pipeline (developers)
The exam papers are the boards' copyright, so `papers/` is not in the repository (`papers/used/` holds the calibration papers: the 22 originals and the triage fixtures moved there after being fixed against; `papers/unused/` the papers not yet tuned against). The papers the tests read are committed encrypted in [`src-tauri/fixtures/corpus/`](src-tauri/fixtures/corpus/README.md); with the key, `node scripts/fixtures.mjs unpack` restores them. Outputs go to the gitignored `output/`.
```bash
MERGEMARK_REQUIRE_FIXTURES=1 cargo test --manifest-path src-tauri/Cargo.toml
node scripts/run-zero-cost-calibration.mjs --stamp <run-id>
node scripts/verify-corpus-render.mjs output/zero-cost-orchestration/c-calibration/run-<run-id>
```
With `MERGEMARK_REQUIRE_FIXTURES=1`, a test whose paper is missing fails instead of passing unchecked. `bash scripts/verify.sh <stamp>` runs the tests, the calibration and the triage below in one go (1.5–2 hours).

To find problems on papers the engine has not been tuned against, triage everything in `papers/unused/` (offline, no hand-made expected IDs; it checks printed mark totals, ID gaps, unidentified glyphs, missing figures and KaTeX rendering) and read `output/triage/run-<stamp>/summary.md`:
```bash
node scripts/triage-papers.mjs --stamp <run-id>            # resumable
node scripts/triage-papers.mjs --stamp <run-id> --rescore  # re-apply the checks to saved imports
```
A paper that is fixed against moves from `papers/unused/` to `papers/used/` and becomes a regression fixture.

The calibration imports every corpus paper offline. It fails if any artifact is stale or the question IDs differ from the independent list in `scripts/fixtures/source-expected-ids.json` (MergeMark has no model client, so a request cannot be attempted at all). The render check passes every card through the app's Markdown/KaTeX pipeline.

The mark-scheme reader is kept outside the app, for measuring a rebuild. To see what it makes of a scheme, and every reason it refuses one:
```bash
cargo run --manifest-path src-tauri/Cargo.toml --example e2e_import -- "<scheme>.pdf" "<name>" --mark-scheme --census --out <dir>
```

## 🚀 Getting Started

### Installation
Download the installer for your computer from the [Releases](../../releases) tab. Neither build is code-signed yet, so the first launch needs one extra step.

**Windows** (`MergeMark_1.0.0_x64-setup.exe`): run the installer. It installs for your own Windows account and needs no administrator rights. SmartScreen may say "Windows protected your PC": choose **More info**, then **Run anyway**.

**macOS** (`MergeMark_1.0.0_universal.dmg`, Apple Silicon and Intel, macOS 11 or later): open the disk image and drag MergeMark into Applications. The first time you open it, macOS refuses, because the app is not notarised by Apple. This is not a warning you can click through:
1. Open MergeMark once, and choose **Done** (or **OK**) when macOS says it cannot be opened.
2. Open **System Settings → Privacy & Security** and scroll down to **Security**.
3. Next to "MergeMark was blocked to protect your Mac", choose **Open Anyway**, then confirm with your password.

MergeMark then opens normally from then on.

Read [notes for users](docs/BETA_TESTERS.md) before your first import — in particular, what the **REVIEW** badge means and how to check a card against its paper.

### Updates
MergeMark checks for a newer version when it starts, and from **Settings → Backup & Maintenance → Updates**, where the check at start-up can be turned off. To check, it asks this repository's GitHub releases for the newest version's details: an ordinary web request that carries nothing about your library or your papers. An update installs only if it is signed with MergeMark's key, and only when you choose **Install and restart**. Your library is backed up when a new version first opens it. A 0.9 beta cannot update itself to 1.0, which is signed with a new key: install 1.0 over it from the Releases tab, and your library is kept.

### How to Use
1. **Import:** Drag and drop a PDF past paper into the ingestion dropzone.
2. **Review:** Check the extracted questions, against their pages with each card's source button. You can edit any card to correct it.
3. **Export:** Copy the isolated question blocks as clean markdown/LaTeX to use in your notes, active recall templates, or custom worksheets.

## 🗺️ Roadmap
- [ ] Mark schemes, rebuilt on the layout engine's tables and glyph reading.
- [ ] Expanded subject schemas (Chemistry, Biology), building on the Physics and Computer Science support.
- [ ] Carry bold/italic emphasis in prose through to the cards.

## 💬 Feedback
Your feedback is critical — especially a card that imports **without** a REVIEW badge and is still wrong, because no automated check catches those.

👉 **[Open an issue](../../issues/new)** with the paper's file name, the question number, what is wrong, and a screenshot of the card and the page if you can.

---

## ⚖️ License & Copyright
**Copyright (c) 2026. All Rights Reserved.**

This repository and its contents are proprietary. You may view the code for educational and portfolio evaluation purposes. However, you may not copy, modify, distribute, or use this code (or any of its assets) for commercial or non-commercial purposes without explicit written permission.
