#!/bin/sh
# Build the bilingual Aieyes promo videos and the Chinese workday story into dist/promo.
#   sh scripts/promo/build.sh            # reuse existing native renders
#   sh scripts/promo/build.sh --story workday  # only the Chinese workday video
#   sh scripts/promo/build.sh --render   # re-render the native macOS UI first (needs a graphics session)
set -eu
STORY=all
RENDER=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --story)
      [ "$#" -ge 2 ] || { echo "--story needs all, overview or workday" >&2; exit 1; }
      STORY=$2
      case "$STORY" in all|overview|workday) ;; *) echo "Unknown story: $STORY" >&2; exit 1 ;; esac
      shift 2
      ;;
    --render) RENDER=1; shift ;;
    --help|-h) echo "Usage: sh scripts/promo/build.sh [--story all|overview|workday] [--render]"; exit 0 ;;
    *) echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
WORK="$ROOT/.local/promo"
OUT="$ROOT/dist/promo"
mkdir -p "$WORK/bin" "$WORK/fonts" "$OUT"
cd "$ROOT"
[ -f "$ROOT/.local/website/pet-sheet.png" ] || node scripts/website/encode.cjs --pet
swiftc -O scripts/promo/video.swift -o "$WORK/bin/video"
if [ "$STORY" != workday ]; then
  if [ "$RENDER" = 1 ] || [ ! -f "$WORK/native/zh-CN/overview-grid.png" ] || [ ! -f "$WORK/native/en/overview-grid.png" ]; then
    python3 scripts/promo/render_native.py --language all
  fi
  for weight in Regular Medium; do
    font="$WORK/fonts/LXGWWenKai-$weight.ttf"
    [ -f "$font" ] || curl -fsSL -o "$font" "https://github.com/lxgw/LxgwWenKai/releases/download/v1.530/LXGWWenKai-$weight.ttf"
  done
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
  [ -f "$WORK/native/zh-CN-light/overview-grid.png" ] && [ "$RENDER" != 1 ] || python3 scripts/promo/render_native.py --language zh-CN --theme light
  node scripts/promo/capture.cjs --lang zh-CN --font pingfang --theme light --out "$WORK/video-zh-CN-pingfang-light.mp4"
  "$WORK/bin/video" mux --video "$WORK/video-zh-CN-pingfang-light.mp4" --audio "$WORK/soundtrack.m4a" --output "$OUT/aieyes-promo-zh-CN-pingfang-light.mp4"
fi
if [ "$STORY" != overview ]; then
  # Workday: one day of Codex work, scored from the composition's own cue sheet.
  [ -f "$WORK/native/zh-CN-light-workday/night/wake-editor.png" ] && [ "$RENDER" != 1 ] || python3 scripts/promo/render_native.py --language zh-CN --theme light --story workday
  node scripts/promo/capture.cjs --composition workday --lang zh-CN --font pingfang --theme light --cues "$WORK/cues-workday.json"
  python3 scripts/promo/soundtrack.py "$WORK/soundtrack-workday.wav" "$WORK/cues-workday.json"
  afconvert -f m4af -d aac -b 192000 "$WORK/soundtrack-workday.wav" "$WORK/soundtrack-workday.m4a"
  node scripts/promo/capture.cjs --composition workday --lang zh-CN --font pingfang --theme light --out "$WORK/video-workday-zh-CN-pingfang-light.mp4"
  "$WORK/bin/video" mux --video "$WORK/video-workday-zh-CN-pingfang-light.mp4" --audio "$WORK/soundtrack-workday.m4a" --output "$OUT/aieyes-promo-workday-zh-CN-pingfang-light.mp4"
fi
ls -la "$OUT"
