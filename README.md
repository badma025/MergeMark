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
* **Zero-Cost Import for Digital PDFs:** Papers with a text layer (almost every board PDF) are transcribed entirely on your machine, with no AI requests, no tokens and no cost. See [How importing works](#-how-importing-works).
* **100% Local & Private:** Built on a local SQLite database. Your files, extractions, and data live strictly on your machine and are never uploaded to a central server.
* **BYOK (Bring Your Own Key):** AI is only needed for scanned papers with no text layer. The app includes 3 free parses out of the box. After that, plug in your own AI provider key in the Settings tab to process unlimited scanned papers at raw wholesale API cost—bypassing expensive SaaS subscriptions.
* **Markdown & LaTeX Ready:** Export extractions instantly into your preferred knowledge management workflows (like Obsidian) or use them to compile targeted topic tests.

## 🛠️ Tech Stack
This application is engineered for maximum performance and a minimal memory footprint:
* **Core/Backend:** Rust & Tauri
* **Frontend:** React & TypeScript
* **Database:** SQLite (Local)
* **PDF engine:** PDFium (via `pdfium-render`) for glyphs, fonts, vector paths and page rendering
* **Intelligence:** A local geometry-first layout engine for digital papers; multimodal vision models only for scanned ones

## 🔍 How importing works
Any PDF with extractable text is imported **locally only**: the app never sends it to an AI provider, needs no API key, and does not use a free parse. Only a fully scanned PDF (no text at all) falls back to the vision models.

A local import runs in four stages:
1. **Layout** (`src-tauri/src/layout.rs`). Reads every glyph's position, baseline, font and size from PDFium, together with the page's rules and vector paths. From these it rebuilds reading-order lines and the maths itself: fractions, radicals, scripts, sums and integrals with limits, matrices, piecewise definitions, vector arrows. It also rebuilds tables and code listings, and sets aside page furniture (headers, footers, margin text, barcodes, repeated QR codes). Font codes with no usable Unicode are resolved from the font's own tables; anything it can't identify is kept as `�` and flagged, never guessed.
2. **Question map.** Finds question headings, part labels, mark allocations, section breaks and reference material (for example, an instruction-set table "included so that you can answer…"), so each card holds exactly one question.
3. **Figures.** Diagrams, graphs and formula pictures are found from the drawn strokes and images, cropped together with their labels, and placed where they are printed. Labelled diagram options become multiple-choice options.
4. **Checks** (`src-tauri/src/deterministic.rs`, `validate.rs`). A card counts as a clean import only if its marks add up, it ends where the question ends, every figure is attached and no glyph is unidentified. Anything else is still imported, but flagged for review. Nothing is silently relabelled as a success.

On the 18 historical papers used for testing (Edexcel, AQA, Cambridge and practice papers across maths, physics and computer science), 243 of 244 questions import cleanly, and all 69 test-paper questions do. The one exception is flagged because its source draws a symbol as an outline with no character behind it. All of this runs with zero AI requests and zero cost.

### Verifying the pipeline (developers)
The corpus papers are not in the repository (`past papers for mergemark/` and `test_papers/` are local), and outputs go to the gitignored `output/`.
```bash
cargo test --manifest-path src-tauri/Cargo.toml
node scripts/run-zero-cost-calibration.mjs --stamp <run-id>
node scripts/verify-corpus-render.mjs output/zero-cost-orchestration/c-calibration/run-<run-id>
```
The calibration imports every corpus paper offline. It fails if any request is attempted, any artifact is stale, or the question IDs differ from the independent list in `scripts/fixtures/source-expected-ids.json`. The render check passes every card through the app's Markdown/KaTeX pipeline.

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
