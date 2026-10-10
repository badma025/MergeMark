#!/usr/bin/env bash
# Puts PDFium for macOS at src-tauri/libpdfium.dylib, where the macOS build
# bundles it (tauri.macos.conf.json, into the app's Contents/Frameworks).
#   bash scripts/fetch-pdfium-macos.sh
# It is the same PDFium build as src-tauri/pdfium.dll (src-tauri/VERSION:
# chromium build 7961), from bblanchon/pdfium-binaries, as a universal
# (Apple Silicon and Intel) library. The library is not committed; the archive
# is checked against the SHA-256 GitHub records for that release asset.
set -euo pipefail
build=7961
sha256=432ba1831a4581cb0d52550fdad977d0ebf4f31188223b5ddd99b9b89d7124fe
url="https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F${build}/pdfium-mac-univ.tgz"
root="$(cd "$(dirname "$0")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
echo "downloading $url"
curl -fsSL "$url" -o "$tmp/pdfium.tgz"
echo "${sha256}  $tmp/pdfium.tgz" | shasum -a 256 -c - || { echo "pdfium-mac-univ.tgz is not the archive this script expects" >&2; exit 1; }
tar -xzf "$tmp/pdfium.tgz" -C "$tmp"
grep -q "BUILD=${build}" "$tmp/VERSION" || { echo "unexpected PDFium build: $(cat "$tmp/VERSION")" >&2; exit 1; }
cp "$tmp/lib/libpdfium.dylib" "$root/src-tauri/libpdfium.dylib"
shasum -a 256 "$root/src-tauri/libpdfium.dylib"
lipo -info "$root/src-tauri/libpdfium.dylib"
