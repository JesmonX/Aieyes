#!/bin/sh
# Build the bilingual Aieyes promo videos into dist/promo.
#   sh scripts/promo/build.sh            # reuse existing native renders
#   sh scripts/promo/build.sh --render   # re-render the native macOS UI first (needs a Retina screen)
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
WORK="$ROOT/.local/promo"
OUT="$ROOT/dist/promo"
mkdir -p "$WORK/bin" "$WORK/fonts" "$OUT"
cd "$ROOT"
if [ "${1:-}" = "--render" ] || [ ! -f "$WORK/native/zh-CN/overview-grid.png" ]; then
  python3 scripts/promo/render_native.py --language all
fi
for weight in Regular Medium; do
  font="$WORK/fonts/LXGWWenKai-$weight.ttf"
  [ -f "$font" ] || curl -fsSL -o "$font" "https://github.com/lxgw/LxgwWenKai/releases/download/v1.530/LXGWWenKai-$weight.ttf"
done
[ -f "$ROOT/.local/website/pet-sheet.png" ] || node scripts/website/encode.cjs --pet
swiftc -O scripts/promo/video.swift -o "$WORK/bin/video"
python3 scripts/promo/soundtrack.py "$WORK/soundtrack.wav"
afconvert -f m4af -d aac -b 192000 "$WORK/soundtrack.wav" "$WORK/soundtrack.m4a"
# brand: the website's Comic Sans MS + LXGW WenKai; pingfang: the macOS system typeface.
for font in brand pingfang; do
  suffix=$([ "$font" = brand ] && echo "" || echo "-$font")
  for lang in zh-CN en; do
    node scripts/promo/capture.cjs --lang "$lang" --font "$font" --out "$WORK/video-$lang$suffix.mp4"
    "$WORK/bin/video" mux --video "$WORK/video-$lang$suffix.mp4" --audio "$WORK/soundtrack.m4a" --output "$OUT/aieyes-promo-$lang$suffix.mp4"
  done
done
# Light, PingFang, Chinese: light-mode native renders on a light stage.
[ -f "$WORK/native/zh-CN-light/overview-grid.png" ] && [ "${1:-}" != "--render" ] || python3 scripts/promo/render_native.py --language zh-CN --theme light
node scripts/promo/capture.cjs --lang zh-CN --font pingfang --theme light --out "$WORK/video-zh-CN-pingfang-light.mp4"
"$WORK/bin/video" mux --video "$WORK/video-zh-CN-pingfang-light.mp4" --audio "$WORK/soundtrack.m4a" --output "$OUT/aieyes-promo-zh-CN-pingfang-light.mp4"
ls -la "$OUT"
