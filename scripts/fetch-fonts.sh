#!/bin/sh
# Download the UI fonts (Cinzel, Alegreya Sans; both SIL Open Font License)
# into ui/fonts so Beacon looks right offline. Run once before building.
set -eu
cd "$(dirname "$0")/../ui"
mkdir -p fonts
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
(cd "$tmp" && npm pack --silent @fontsource-variable/cinzel @fontsource/alegreya-sans >/dev/null)
for f in "$tmp"/*.tgz; do tar xzf "$f" -C "$tmp" && mv "$tmp/package" "$f.d"; done
cp "$tmp"/fontsource-variable-cinzel-*.d/files/cinzel-latin-wght-normal.woff2 fonts/
cp "$tmp"/fontsource-alegreya-sans-*.d/files/alegreya-sans-latin-400-normal.woff2 fonts/
cp "$tmp"/fontsource-alegreya-sans-*.d/files/alegreya-sans-latin-700-normal.woff2 fonts/
echo "Fonts saved to ui/fonts"
