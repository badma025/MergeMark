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

Currently optimized for **GCSE and A-Level Mathematics & Further Mathematics**.

## ✨ Key Features
* **Zero-Friction Parsing:** Drag and drop full exam papers or question booklets. The engine automatically isolates distinct questions from the layout.
* **100% Local & Private:** Built on a local SQLite database. Your files, extractions, and data live strictly on your machine and are never uploaded to a central server.
* **BYOK (Bring Your Own Key):** The app includes 3 free parses out of the box. After that, plug in your own AI provider key in the Settings tab to process unlimited papers at raw wholesale API cost—bypassing expensive SaaS subscriptions.
* **Markdown & LaTeX Ready:** Export extractions instantly into your preferred knowledge management workflows (like Obsidian) or use them to compile targeted topic tests.

## 🛠️ Tech Stack
This application is engineered for maximum performance and a minimal memory footprint:
* **Core/Backend:** Rust & Tauri
* **Frontend:** React & TypeScript
* **Database:** SQLite (Local)
* **Intelligence:** Integrates with multimodal vision models for layout parsing

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
- [ ] Expanded subject schemas (Physics, Chemistry, Computer Science).
- [ ] Enhanced parsing for complex diagrams and graphs.

## 💬 Feedback & Community
Since this is a `v0.9.0` beta, your feedback is critical. If you find a bug, encounter a PDF layout that breaks the parser, or want to request a new feature, please join the community:

👉 **[Join the MergeMark Discord Server](#)** *(Note: Add your Discord invite link here)*

---

## ⚖️ License & Copyright
**Copyright (c) 2026. All Rights Reserved.**

This repository and its contents are proprietary. You may view the code for educational and portfolio evaluation purposes. However, you may not copy, modify, distribute, or use this code (or any of its assets) for commercial or non-commercial purposes without explicit written permission.
