#!/usr/bin/env python3
"""Self-host OFL fonts, subset to site copy. Requires fonttools[woff] and brotli.

Comic Sans MS remains an installed-font preference and is never redistributed.
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
SOURCES = [
    ("ComicNeue", "comic-neue", "https://raw.githubusercontent.com/google/fonts/main/ofl/comicneue/ComicNeue-Regular.ttf", "https://raw.githubusercontent.com/google/fonts/main/ofl/comicneue/OFL.txt"),
    ("LXGWWenKai", "lxgw-wenkai", "https://github.com/lxgw/LxgwWenKai/releases/download/v1.530/LXGWWenKai-Regular.ttf", "https://raw.githubusercontent.com/lxgw/LxgwWenKai/main/OFL.txt"),
]
copy = "".join(path.read_text() for path in [ROOT / "apps/website/content.mjs", ROOT / "scripts/website/build.mjs"])
characters = {ord(c) for c in copy} | set(range(32, 127)) | set(range(0x2000, 0x2070))
manifest = []
for family, name, url, license_url in SOURCES:
    source = CACHE / (family + "-Regular.ttf")
    license_file = CACHE / (family + "-LICENSE.txt")
    for file, address in [(source, url), (license_file, license_url)]:
        if not file.exists():
            file.write_bytes(urllib.request.urlopen(address, timeout=60).read())
    font = TTFont(source)
    options = subset.Options()
    options.layout_features = ["*"]
    options.name_IDs = [0, 1, 2, 3, 4, 5, 6, 13, 14]
    processor = subset.Subsetter(options=options)
    processor.populate(unicodes=characters.intersection(font.getBestCmap()))
    processor.subset(font)
    font.flavor = "woff2"
    target = OUT / (name + "-regular.woff2")
    font.save(target)
    license_text = "\n".join(line.rstrip() for line in license_file.read_text(encoding="utf-8").splitlines()) + "\n"
    (OUT / (name + "-LICENSE.txt")).write_text(license_text, encoding="utf-8")
    manifest.append(dict(file=target.name, source=url, license=license_url, sha256=hashlib.sha256(source.read_bytes()).hexdigest(), bytes=target.stat().st_size))
(OUT / "sources.json").write_text(json.dumps(manifest, indent=2) + "\n")
print(json.dumps(manifest, indent=2))
