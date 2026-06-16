#!/usr/bin/env bash
# Render the marketing screenshots.
#
# gen-assets produces intermediate SVGs (Berkeley Mono primary, JetBrains Mono
# embedded as fallback for the glyphs Berkeley lacks). rsvg-convert then
# rasterizes each to a 2x PNG via fontconfig font fallback, the same way a
# terminal substitutes a fallback font. The shipped artifact is a PNG (pixels),
# so no font file is committed to the repository.
#
# Requires: rsvg-convert (librsvg / librsvg2-bin) and both Berkeley Mono and
# JetBrains Mono available to fontconfig. Optional: pngquant for size.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PURPLE_BIN="${PURPLE_BIN:-$ROOT/target/release/purple}"
FONT_DIR="$ROOT/assets/fonts"
PNG_DIR="${PNG_DIR:-$ROOT/assets/png}"
HERO_OUT="${HERO_OUT:-$ROOT/assets/hero.svg}"
SCALE="${SCALE:-2}"

if [[ ! -x "$PURPLE_BIN" ]]; then
  echo "error: purple binary not found at $PURPLE_BIN (set PURPLE_BIN or run cargo build --release)" >&2
  exit 1
fi
if ! command -v rsvg-convert >/dev/null 2>&1; then
  echo "error: rsvg-convert not found (install librsvg / librsvg2-bin)" >&2
  exit 1
fi

SVG_DIR="$(mktemp -d)"
trap 'rm -rf "$SVG_DIR"' EXIT

# hero.svg is a committed artifact: the landing-page build inlines it via the
# __HERO_SVG__ placeholder, so it ships as a vector and never rasterizes here.
"$PURPLE_BIN" gen-assets "$SVG_DIR" --font-dir "$FONT_DIR" --hero-out "$HERO_OUT"

mkdir -p "$PNG_DIR"
for svg in "$SVG_DIR"/*.svg; do
  name="$(basename "$svg" .svg)"
  out="$PNG_DIR/$name.png"
  rsvg-convert -z "$SCALE" "$svg" -o "$out"
  if command -v pngquant >/dev/null 2>&1; then
    pngquant --force --skip-if-larger --quality=70-95 --output "$out" "$out" || true
  fi
  printf '  %s (%s KB)\n' "$name.png" "$(( $(wc -c < "$out") / 1024 ))"
done
echo "Wrote PNGs to $PNG_DIR"
