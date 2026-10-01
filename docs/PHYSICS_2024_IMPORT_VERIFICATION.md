# Physics 2024 import verification

Verified on 12 September 2026 against `physics '24.pdf` (40 pages, 32 questions).

The fresh production-pipeline import passes **32/32 automated rendering checks**. It produced no quarantined questions, no structural flags and no `needs_review` cards. This supersedes the failing output described in `PHYSICS_2024_IMPORT_AUDIT.md` for the checks below.

## Repairs and evidence

| Area | Verification |
| --- | --- |
| Mixed prose and inline math, Q18/24/27/30 | Option-local delimiters; four separately rendered choices; no leaked tags or KaTeX errors. |
| AQA margin debris, Q4/6 | All subpart labels retained; allocations 1/2/2 and 3/4/2/3 respectively. All Q1–7 mark allocations checked. |
| Nuclear notation, Q17/31 | Fusion and three decay equations survive the production validation step and KaTeX rendering. |
| Units, Q2 | Math commands removed from text-mode unit wrappers. Four distinct capacitor figures retained. |
| Fractions and exponents, Q16/21/22 | Complete source-row fixtures recover the audited formulas, including Q16 A and D and all four Q22 powers. Missing operands are not guessed. |
| Diagram choices, Q26 | Four image choices in printed A–D order. Crops visually compared with source page 32; final import hashes match the inspected crops. |
| Stem diagrams, Q14/30 | Images precede choices. Q14 has one oscilloscope image; its numerical option table is represented as choices, not duplicate images. |
| Additional extraction checks | Q1 ratio header preserved as a fraction; Q20 unmapped symbol-font fragments trigger fallback and produce four complete fraction/power choices. |

## Commands and results

- `cargo test --manifest-path src-tauri/Cargo.toml --lib`: **308 passed, 0 failed, 2 ignored** (diagnostic helpers).
- After the final Q20 fallback guard and margin cleanup, `cargo test --manifest-path src-tauri/Cargo.toml --lib physics24_`: **8 passed, 0 failed**.
- `node --experimental-strip-types --test src/lib/preprocess-mcq.test.mjs`: **6 passed**.
- `npm.cmd run build`: **passed**; Vite reports the existing large-chunk advisory.
- `git diff --check`: **passed**.
- From `src-tauri`: `./target/debug/e2e_import.exe "../physics '24.pdf" physics24 --out ../output/physics24-verification --config-db C:/Users/alimb/AppData/Roaming/com.mergemark.app/mergemark.db`.
- `node scripts/verify-physics24-render.mjs output/physics24-verification/physics24_cards.json`: **complete=true, passed=32, total=32, failures=[]**.

The final import used 19 deterministic questions and 8 text-first questions, with zero vision repairs or quarantines. The harness reads provider configuration without modifying the application database.

## Artifacts and limits

- [Rendered verification report](../output/physics24-verification/render-verification.html)
- [Machine-readable checks and processed content](../output/physics24-verification/render-verification.json)
- [Fresh imported cards](../output/physics24-verification/physics24_cards.json)

The renderer audit uses the frontend preprocessor, React Markdown and KaTeX through server-side rendering. It checks question coverage, exact choices, audited source expressions, marks, delimiter errors and image availability. Source pages and the affected image crops were also inspected. No browser was available for an in-app visual pass, so **32/32 is an automated verification result, not a claim of exhaustive visual or scientific fidelity**. Existing imported cards in the application database have not been replaced.
