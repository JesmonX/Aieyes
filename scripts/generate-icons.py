"""Render and package the code-native eye / usage gauge icon. Requires macOS Swift."""
import os
import pathlib
import struct
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
SIZES = (16, 32, 48, 64, 128, 256, 512, 1024)


def main():
    cache = ROOT / ".build/icon-module-cache"
    cache.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="aieyes-icons-") as directory:
        subprocess.run(
            ["swift", "-module-cache-path", str(cache), str(ROOT / "scripts/render-icon.swift"), directory],
            check=True, env={**os.environ, "CLANG_MODULE_CACHE_PATH": str(cache)},
        )
        template = (pathlib.Path(directory) / "template-64.png").read_bytes()
        rgba = (pathlib.Path(directory) / "brand.rgba").read_bytes()
        images = {s: (pathlib.Path(directory) / f"{s}.png").read_bytes() for s in SIZES}
        brands = {s: (pathlib.Path(directory) / f"brand-{s}.png").read_bytes() for s in SIZES}
    (ROOT / "apps/macos/Resources/Brand.png").write_bytes(brands[128])
    (ROOT / "apps/macos/Resources/BrandTemplate.png").write_bytes(template)
    (ROOT / "apps/desktop/web/brand.png").write_bytes(brands[128])
    (ROOT / "apps/desktop/src-tauri/icons/brand.rgba").write_bytes(rgba)
    icons = ROOT / "apps/desktop/src-tauri/icons"
    icons.mkdir(parents=True, exist_ok=True)
    (icons / "icon.png").write_bytes(images[1024])
    ico_sizes = (16, 32, 48, 64, 128, 256)
    offset = 6 + 16 * len(ico_sizes)
    entries = bytearray()
    for size in ico_sizes:
        data = images[size]
        entries.extend(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset))
        offset += len(data)
    (icons / "icon.ico").write_bytes(struct.pack("<HHH", 0, 1, len(ico_sizes)) + entries + b"".join(images[s] for s in ico_sizes))
    chunks = []
    for kind, size in ((b"icp4", 16), (b"icp5", 32), (b"icp6", 64), (b"ic07", 128), (b"ic08", 256), (b"ic09", 512), (b"ic10", 1024)):
        data = images[size]
        chunks.append(kind + struct.pack(">I", len(data) + 8) + data)
    payload = b"".join(chunks)
    icns = b"icns" + struct.pack(">I", len(payload) + 8) + payload
    (icons / "icon.icns").write_bytes(icns)
    (ROOT / "apps/macos/Resources/AppIcon.icns").write_bytes(icns)
    previews = ROOT / "docs/icon-design"
    previews.mkdir(parents=True, exist_ok=True)
    for size in (16, 32, 64, 256, 512):
        (previews / f"aieyes-{size}.png").write_bytes(images[size])
    for size in (16, 32, 64, 128, 256):
        (previews / f"brand-{size}.png").write_bytes(brands[size])
    print("Generated platform application icons, transparent brand marks and previews")


if __name__ == "__main__":
    main()
