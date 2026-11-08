#!/usr/bin/env python3
"""Build the local Display Lab WidgetKit extension without requiring an Apple team.

The sandbox can only read the lab's aggregate snapshot directory. This is a
development packaging path, not the signing policy for an App Store release.
"""
import os
from pathlib import Path
import platform
import plistlib
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent


def build(app):
    source = ROOT / "apps/macos/WidgetExtension"
    shared = ROOT / "apps/macos/Sources/Aieyes"
    extension = app / "Contents/PlugIns/AieyesWidgets.appex"
    executable = extension / "Contents/MacOS/AieyesWidgets"
    executable.parent.mkdir(parents=True, exist_ok=True)
    snapshot = Path.home() / "Library/Application Support/Aieyes Display Lab/widget-snapshot.json"
    host_path = app / "Contents/Info.plist"
    host = plistlib.loads(host_path.read_bytes())
    host.update(CFBundleIdentifier="app.aieyes.displaylab", CFBundleName="Aieyes Lab", CFBundleDisplayName="Aieyes Lab", CFBundleExecutable="AieyesLab",
                AieyesDisplayLab=True, AieyesDevelopmentDataDirectory=str(ROOT / ".local/display-lab/app"),
                AieyesWidgetSnapshotPath=str(snapshot),
                CFBundleURLTypes=[{"CFBundleURLName": "Aieyes Display Lab", "CFBundleURLSchemes": ["aieyes-display-lab"]}])
    host.pop("SUPublicEDKey", None)
    host_path.write_bytes(plistlib.dumps(host))
    primary = app / "Contents/MacOS/Aieyes"
    if primary.exists():
        primary.replace(app / "Contents/MacOS/AieyesLab")
    info = plistlib.loads((source / "Info.plist").read_bytes())
    info.update(CFBundleShortVersionString=host["CFBundleShortVersionString"], CFBundleVersion=host["CFBundleVersion"], AieyesWidgetSnapshotPath=str(snapshot),
                CFBundleInfoDictionaryVersion="6.0", CFBundleSupportedPlatforms=["MacOSX"], DTPlatformName="macosx")
    (extension / "Contents/Info.plist").write_bytes(plistlib.dumps(info))
    sdk = subprocess.check_output(["xcrun", "--sdk", "macosx", "--show-sdk-path"], text=True).strip()
    command = ["xcrun", "swiftc", "-parse-as-library", "-application-extension", "-swift-version", "5", "-O",
               "-sdk", sdk, "-target", platform.machine() + "-apple-macos14.0", "-module-name", "AieyesWidgets",
               "-module-cache-path", str(ROOT / ".build/swift-module-cache"),
               "-Xlinker", "-e", "-Xlinker", "_NSExtensionMain",
               str(shared / "DisplaySnapshot.swift"), str(shared / "DisplayCard.swift"), str(shared / "LocalizationCatalog.swift"),
               str(source / "AieyesWidgets.swift"), "-o", str(executable)]
    subprocess.run(command, check=True)
    for language in ("zh-Hans", "en"):
        resources = extension / "Contents/Resources" / (language + ".lproj")
        resources.mkdir(parents=True, exist_ok=True)
        description = "Today’s usage, account quotas and session status at a glance." if language == "en" else "今日用量、账户额度与会话状态快照。"
        (resources / "Localizable.strings").write_text('"今日用量、账户额度与会话状态快照。" = "' + description + '";\n', encoding="utf-8")
        (app / "Contents/Resources" / (language + ".lproj") / "InfoPlist.strings").write_text('"CFBundleDisplayName" = "Aieyes Lab";\n', encoding="utf-8")
    print(extension)


if __name__ == "__main__":
    build(Path(sys.argv[1]).resolve())
