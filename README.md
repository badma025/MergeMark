<div align="center">
  <img width="600" alt="MergeMark Logo" src="https://github.com/user-attachments/assets/98b98209-4c32-4677-be8b-57964f845a95" />

  **A privacy-first desktop engine that instantly transforms dense academic past papers into clean, customizable question cards.**

  [![Version](https://img.shields.io/badge/version-0.9.0_Beta-blue.svg)]()
  [![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS-lightgrey.svg)]()
  [![License](https://img.shields.io/badge/license-All%20Rights%20Reserved-red.svg)]()
</div>

---

## ⚡ What is MergeMark?
MergeMark is a local-first desktop application built to replace manual data entry for students and teachers. Instead of spending hours retyping complex typography, matrices, and physics equations, you simply import a PDF past paper. MergeMark's engine parses the document layout and extracts it into beautifully formatted, isolated markdown and LaTeX modules.

Currently optimized for **GCSE and A-Level Mathematics & Further Mathematics**, and tested on A-Level Physics and Computer Science papers.

## ✨ Key Features
* **Zero-Friction Parsing:** Drag and drop full exam papers or question booklets. The engine automatically isolates distinct questions from the layout.
* **Fully Offline Import:** Every paper and mark scheme is read on your machine. MergeMark makes no AI requests of any kind, needs no API key and costs nothing. See [How importing works](#-how-importing-works).
* **100% Local & Private:** Built on a local SQLite database. Your files, extractions, and data live strictly on your machine and are never uploaded anywhere.
* **Markdown & LaTeX Ready:** Export extractions instantly into your preferred knowledge management workflows (like Obsidian) or use them to compile targeted topic tests.

## 🛠️ Tech Stack
This application is engineered for maximum performance and a minimal memory footprint:
* **Core/Backend:** Rust & Tauri
* **Frontend:** React & TypeScript
* **Database:** SQLite (Local)
* **PDF engine:** PDFium (via `pdfium-render`) for glyphs, fonts, vector paths and page rendering
* **Intelligence:** A local geometry-first layout engine for digital papers and a local table reader for mark schemes; the operating system's own OCR engine (Windows, macOS) or Tesseract (Linux) for scanned papers. No AI models.

## 🔍 How importing works
Everything is imported **on this machine**. MergeMark has no AI service to send anything to: it makes no requests, needs no API key and costs nothing.
* **Question papers with a text layer** (almost every board PDF) are rebuilt from the PDF itself, in the four stages below.
* **Scanned papers** (pages printed as pictures or outlines, with no text layer or only a watermark) are read on this computer: on Windows by the operating system's own OCR engine, on macOS by Apple's Vision framework, and elsewhere by Tesseract when it is installed (`tesseract` on the PATH, for example `sudo apt install tesseract-ocr`). OCR reads words, not mathematics, so every question it produces is kept for review. Where no local OCR engine is available, a scanned paper is reported as not transcribed.
* **Mark schemes** are read by a local table reader (`src-tauri/src/mark_scheme_deterministic.rs`), which accepts a scheme only when it can account for every answer page. A scheme it cannot read is reported as not transcribed, with the reason.

The Settings screens for API keys, models, usage, free parses and topic generation are still in the app, but those features are turned off: each one answers that MergeMark works offline.

A local import runs in four stages:
1. **Layout** (`src-tauri/src/layout.rs`). Reads every glyph's position, baseline, font and size from PDFium, together with the page's rules and vector paths. From these it rebuilds reading-order lines and the maths itself: fractions, radicals, scripts, sums and integrals with limits, matrices, piecewise definitions, vector arrows. It also rebuilds tables and code listings, and sets aside page furniture (headers, footers, margin text, barcodes, repeated QR codes). Font codes with no usable Unicode are resolved from the font's own tables; anything it can't identify is kept as `�` and flagged, never guessed.
2. **Question map.** Finds question headings, part labels, mark allocations, section breaks and reference material (for example, an instruction-set table "included so that you can answer…"), so each card holds exactly one question.
3. **Figures.** Diagrams, graphs and formula pictures are found from the drawn strokes and images, cropped together with their labels, and placed where they are printed. Labelled diagram options become multiple-choice options.
4. **Checks** (`src-tauri/src/deterministic.rs`, `validate.rs`). A card counts as a clean import only if its marks add up, it ends where the question ends, every figure is attached and no glyph is unidentified. Anything else is still imported, but flagged for review. Nothing is silently relabelled as a success.

On the 18 historical papers used for testing (Edexcel, AQA, Cambridge and practice papers across maths, physics and computer science), 243 of 244 questions import cleanly, and all 69 test-paper questions do. The one exception is flagged because its source draws a symbol as an outline with no character behind it. All of this runs with zero AI requests and zero cost.

### Verifying the pipeline (developers)
The corpus papers are not in the repository (`papers/` is local: `papers/used/` holds the calibration papers: the 22 originals and the triage fixtures moved there after being fixed against; `papers/unused/` the papers not yet tuned against), and outputs go to the gitignored `output/`.
```bash
cargo test --manifest-path src-tauri/Cargo.toml
node scripts/run-zero-cost-calibration.mjs --stamp <run-id>
node scripts/verify-corpus-render.mjs output/zero-cost-orchestration/c-calibration/run-<run-id>
```
To find problems on papers the engine has not been tuned against, triage everything in `papers/unused/` (offline, no hand-made expected IDs; it checks printed mark totals, ID gaps, unidentified glyphs, missing figures and KaTeX rendering) and read `output/triage/run-<stamp>/summary.md`:
```bash
node scripts/triage-papers.mjs --stamp <run-id>            # resumable
node scripts/triage-papers.mjs --stamp <run-id> --rescore  # re-apply the checks to saved imports
```
A paper that is fixed against moves from `papers/unused/` to `papers/used/` and becomes a regression fixture.

The calibration imports every corpus paper offline. It fails if any artifact is stale or the question IDs differ from the independent list in `scripts/fixtures/source-expected-ids.json` (MergeMark has no model client, so a request cannot be attempted at all). The render check passes every card through the app's Markdown/KaTeX pipeline.

To see what the local reader makes of a mark scheme:
```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin e2e_import -- "<scheme>.pdf" "<name>" --mark-scheme --out <dir>
```

## 🚀 Getting Started (Beta)

### Installation
1. Navigate to the [Releases](../../releases) tab.
2. Download the latest `v0.9.0` installer for your operating system (Windows `.exe` or macOS `.dmg`).
3. Run the installer and launch MergeMark.

### How to Use
1. **Import:** Drag and drop a PDF past paper into the ingestion dropzone.
2. **Review:** Once parsed, review the extracted questions in the interface. You can manually tweak any highly complex typographical edge-cases.
3. **Export:** Copy the isolated question blocks as clean markdown/LaTeX to use in your notes, active recall templates, or custom worksheets.

## 🗺️ Roadmap
I am actively developing MergeMark toward a stable `v1.0.0` release. Upcoming features include:
- [ ] Direct export pipelines to **Anki** and **Quizlet** for automated flashcard generation.
- [ ] Expanded subject schemas (Chemistry, Biology), building on the Physics and Computer Science support.
- [ ] Carry bold/italic emphasis in prose through to the cards.

## 💬 Feedback & Community
Since this is a `v0.9.0` beta, your feedback is critical. If you find a bug, encounter a PDF layout that breaks the parser, or want to request a new feature, please join the community:

👉 **[Join the MergeMark Discord Server](#)** *(Note: Add your Discord invite link here)*

---

## ⚖️ License & Copyright
**Copyright (c) 2026. All Rights Reserved.**

This repository and its contents are proprietary. You may view the code for educational and portfolio evaluation purposes. However, you may not copy, modify, distribute, or use this code (or any of its assets) for commercial or non-commercial purposes without explicit written permission.
