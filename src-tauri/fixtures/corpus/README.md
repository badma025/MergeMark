# Test papers, encrypted

The Rust tests read real exam papers from the repository's `papers/` folder.
That folder is not in git: the papers are the exam boards' copyright and this
repository is public. This folder holds an encrypted copy of exactly the
papers the tests read, so CI and a fresh clone can run the whole suite.

- `manifest.json` lists each paper by its corpus path, with its SHA-256.
- `used/<name>.pdf.enc` and `unused/<name>.pdf.enc` are the papers, each sealed with
  AES-256-GCM under the fixtures key and bound to its own path.
- The key is never committed. CI reads it from the secret
  `MERGEMARK_FIXTURES_KEY`; on a developer machine it is
  `~/.mergemark/fixtures.key` (or the same environment variable).

## Restoring the papers on a new machine

```bash
node scripts/fixtures.mjs unpack
```

This writes each listed paper into `papers/` (it never overwrites a paper
that differs from the listed one). Then run the tests with fixtures required:

```bash
MERGEMARK_REQUIRE_FIXTURES=1 cargo test --manifest-path src-tauri/Cargo.toml
```

Without `MERGEMARK_REQUIRE_FIXTURES=1` a test whose paper is missing prints
`[FIXTURE] … this test checked nothing` and passes; with it, the test fails.

## After adding a test on a paper not yet listed

`scripts/verify.sh` reports `read by a test but not listed` for it. Record the
papers the tests read and seal the new one:

```bash
MERGEMARK_FIXTURE_LOG=output/fixtures.log cargo test --manifest-path src-tauri/Cargo.toml
node scripts/fixtures.mjs record output/fixtures.log
node scripts/fixtures.mjs pack
```

Commit `manifest.json` and the new `.enc` file. A paper already sealed is not
sealed again, so unchanged papers never change in git.
