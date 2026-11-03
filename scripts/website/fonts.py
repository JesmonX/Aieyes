#!/usr/bin/env python3
"""Self-host OFL fonts, subset to site copy. Requires fonttools[woff] and brotli.

Comic Neue ships Regular and Bold so every platform renders the same 400/700
weights. LXGW WenKai is split in two faces of one family: a tiny face with the
glyphs English pages need (the language names and a few symbols Comic Neue
lacks) and the main Chinese face. Disjoint unicode-range values keep English
visitors from downloading the full Chinese subset. build.mjs turns
sources.json into the @font-face rules and preload hints.
"""
from pathlib import Path
import hashlib
import json
import urllib.request
from fontTools import subset
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parents[2]
CACHE = ROOT / ".local/website/downloads"
OUT = ROOT / "apps/website/public/fonts"
CACHE.mkdir(parents=True, exist_ok=True)
OUT.mkdir(parents=True, exist_ok=True)
COMIC = "https://raw.githubusercontent.com/google/fonts/main/ofl/comicneue/"
LXGW = "https://github.com/lxgw/LxgwWenKai/releases/download/v1.530/LXGWWenKai-Regular.ttf"
LICENSES = {"comic-neue": COMIC + "OFL.txt", "lxgw-wenkai": "https://raw.githubusercontent.com/lxgw/LxgwWenKai/main/OFL.txt"}
LICENSE_CACHE = {"comic-neue": "ComicNeue-LICENSE.txt", "lxgw-wenkai": "LXGWWenKai-LICENSE.txt"}
# Words English pages show in Chinese: the language switch and language story.
ENGLISH_PAGE_CJK = "简体中文。"


def fetch(file, address):
    if not file.exists():
        file.write_bytes(urllib.request.urlopen(address, timeout=60).read())
    return file


def is_cjk(code):
    return 0x2E80 <= code <= 0x9FFF or 0xF900 <= code <= 0xFAFF or 0xFE30 <= code <= 0xFE4F or 0xFF00 <= code <= 0xFFEF


def ranges(codes):
    runs, start, previous = [], None, None
    for code in sorted(codes):
        if start is None:
            start = previous = code
        elif code == previous + 1:
            previous = code
        else:
            runs.append((start, previous))
            start = previous = code
    if start is not None:
        runs.append((start, previous))
    return ",".join(f"U+{a:X}" if a == b else f"U+{a:X}-{b:X}" for a, b in runs)


copy = "".join(path.read_text() for path in [ROOT / "apps/website/content.mjs", ROOT / "scripts/website/build.mjs", ROOT / "scripts/website/og.cjs"])
characters = {ord(c) for c in copy} | set(range(32, 127)) | set(range(0x2000, 0x2070))
comic_regular = fetch(CACHE / "ComicNeue-Regular.ttf", COMIC + "ComicNeue-Regular.ttf")
comic_bold = fetch(CACHE / "ComicNeue-Bold.ttf", COMIC + "ComicNeue-Bold.ttf")
lxgw = fetch(CACHE / "LXGWWenKai-Regular.ttf", LXGW)
comic_cmap = set(TTFont(comic_regular).getBestCmap())
lxgw_cmap = set(TTFont(lxgw).getBestCmap())
# Everything English pages may fall back to: symbols Comic Neue lacks plus the language names.
# The small face claims these code points even when LXGW lacks one (such as ◈), so the
# main face never downloads for them; the browser then uses a system font.
lite_range = {c for c in characters if not is_cjk(c) and c > 127 and c not in comic_cmap} | {ord(c) for c in ENGLISH_PAGE_CJK}
lite = lite_range & lxgw_cmap
main = (characters & lxgw_cmap) - lite_range
complement = ranges(set(range(0, 0x110000)) - lite_range)
FACES = [
    # file stem, family, weight, source, glyphs, unicode-range (None = whole font)
    ("comic-neue-regular", "Comic Neue", 400, comic_regular, COMIC + "ComicNeue-Regular.ttf", characters, None),
    ("comic-neue-bold", "Comic Neue", 700, comic_bold, COMIC + "ComicNeue-Bold.ttf", characters, None),
    ("lxgw-wenkai-regular", "LXGW WenKai", 400, lxgw, LXGW, main, complement),
    ("lxgw-wenkai-lite", "LXGW WenKai", 400, lxgw, LXGW, lite, ranges(lite_range)),
]
for name, address in LICENSES.items():
    license_file = fetch(CACHE / LICENSE_CACHE[name], address)
    license_text = "\n".join(line.rstrip() for line in license_file.read_text(encoding="utf-8").splitlines()) + "\n"
    (OUT / (name + "-LICENSE.txt")).write_text(license_text, encoding="utf-8")
manifest = []
for stem, family, weight, source, url, glyphs, unicode_range in FACES:
    font = TTFont(source)
    options = subset.Options()
    options.layout_features = ["*"]
    options.name_IDs = [0, 1, 2, 3, 4, 5, 6, 13, 14]
    processor = subset.Subsetter(options=options)
    processor.populate(unicodes=glyphs.intersection(font.getBestCmap()))
    processor.subset(font)
    font.flavor = "woff2"
    target = OUT / (stem + ".woff2")
    font.save(target)
    entry = dict(file=target.name, family=family, weight=weight, source=url, license=LICENSES[stem.rsplit("-", 1)[0]], sha256=hashlib.sha256(source.read_bytes()).hexdigest(), bytes=target.stat().st_size)
    if unicode_range:
        entry["unicodeRange"] = unicode_range
    manifest.append(entry)
(OUT / "sources.json").write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n")
print(json.dumps([{k: v for k, v in entry.items() if k != "unicodeRange"} for entry in manifest], indent=2))
