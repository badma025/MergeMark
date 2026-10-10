# Testing MergeMark 1.0 on a Mac

This checks the two things only a Mac can: that the app imports papers (PDFium
bundled in `Contents/Frameworks`), and that scanned pages are read by Apple's
Vision framework. Then the macOS job can go back into
`.github/workflows/release.yml`.

**Run on 10 Oct 2026, on the user's Mac: it passed.** The macOS job is back in
`release.yml`. Two things are left for the first build downloaded from a
release: the universal disk image (this test built for the Mac's own
processor), and the README's "Open Anyway" steps.

## 1. Get the code onto the Mac

The branch is not pushed yet, so carry it over as a git bundle. On the Windows
machine, in the repository:

```bash
git bundle create mergemark.bundle import-fixes-rounds-1-2
```

Copy `mergemark.bundle` to the Mac. To run the tests there too, also copy the
fixtures key, `~/.mergemark/fixtures.key` (on Windows
`C:\Users\<you>\.mergemark\fixtures.key`), to the same place on the Mac. It is
a secret: copy it directly (USB stick or AirDrop), not through a chat or email.

On the Mac:

```bash
git clone -b import-fixes-rounds-1-2 mergemark.bundle MergeMark
cd MergeMark
```

## 2. Tools

- Xcode command line tools: `xcode-select --install`
- Node 22 (nodejs.org, or `brew install node@22`)
- Rust: https://rustup.rs

## 3. Build

```bash
npm ci
bash scripts/fetch-pdfium-macos.sh
npm run tauri build -- --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

`fetch-pdfium-macos.sh` downloads the same PDFium build Windows ships, as a
universal library, to `src-tauri/libpdfium.dylib`. The `--config` part skips
the signed update files: those need the release's private key, which only the
release workflow has. The disk image lands in
`src-tauri/target/release/bundle/dmg/`.

## 4. Run the tests (needs the fixtures key)

```bash
node scripts/fixtures.mjs unpack
MERGEMARK_REQUIRE_FIXTURES=1 cargo test --manifest-path src-tauri/Cargo.toml
```

`unpack` restores the test papers into `papers/`. This is the suite's first run
on macOS. `scanned_pages_are_read_by_local_ocr_and_never_clean` is the first
run of the Vision backend: it reads the scanned Edexcel M3 Jun 2004 paper.
Note every test that fails, with its message.

## 5. Install and use the app

Open the disk image and drag MergeMark into Applications, then open it. A build
made on this Mac opens without the Gatekeeper prompt (only downloaded files are
quarantined); the "Open Anyway" steps in the README are checked later, on a
downloaded build.

Try (without the key, copy these two papers over from the Windows machine's
`papers/used/` instead of unpacking them):

1. Import a digital paper, for example `papers/used/physics '18.pdf`: cards
   appear, with their diagrams.
2. Import the scanned paper `papers/used/Maths_A-level_M3_Papers-Edexcel__June_2004_QP.pdf`:
   its text is read on the Mac, and every card is marked for review.
3. Open a card's source page.
4. Settings: the version reads 1.0.0, and "Check for updates" answers without
   an error window (there is no published 1.0 release to find yet).
5. Optional, with MacTeX installed: make a worksheet PDF.

## 6. Report back

For each step: worked, or what happened (the error text, or a screenshot). If
the app will not open at all, run it from a terminal to see why:

```bash
/Applications/MergeMark.app/Contents/MacOS/*
```
