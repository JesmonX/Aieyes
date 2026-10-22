#!/usr/bin/env python3
"""Render the current native UI in an isolated English preview source tree.

No production sources, saved preferences, accounts or app bundles are modified.
Run with --prepare to inspect the fixture source without launching the GUI.
"""
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / ".local/website"
OUT = WORK / "native"
SOURCE = WORK / "native-source"
APP = WORK / "AieyesPreview.app"
SOURCE.mkdir(parents=True, exist_ok=True)
OUT.mkdir(parents=True, exist_ok=True)
fixture_json = subprocess.check_output([sys.executable, str(ROOT / "scripts/website/fixture.py"), "--fixture"])
(WORK / "fixture.json").write_bytes(fixture_json)
for source in (ROOT / "apps/macos/Sources/Aieyes").glob("*.swift"):
    text = source.read_text()
    text = text.replace("Date()", "Date(timeIntervalSince1970: 1792584060)").replace("Date.now", "Date(timeIntervalSince1970: 1792584060)").replace("date: .now", "date: Date(timeIntervalSince1970: 1792584060)")
    if source.name == "PetWindow.swift":
        text = text.replace("let showing = PetPreviewVisibility.isVisible(geometry.frame(in: .global), viewport: viewport, enclosing: enclosingViewport)", "let showing = true")
    if source.name == "WakeupsView.swift":
        text = text.replace("@StateObject private var store = WakeupsStore()", "@StateObject private var store = websiteWakeups()")
        text = text.replace(".task { await store.load() }", ".task {}")
        text += '''
@MainActor private func websiteWakeups() -> WakeupsStore {
    let store = WakeupsStore()
    let path = ProcessInfo.processInfo.environment["AIEYES_PREVIEW_FIXTURE"]!
    let object = try! JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: path))) as! [String: Any]
    let rows = try! JSONSerialization.data(withJSONObject: object["wakeups"]!)
    store.records = try! JSONDecoder().decode([WakeRecord].self, from: rows)
    for record in store.records {
        store.statuses[record.id] = WakeStatus(installed: true, enabled: true, timezone: "UTC+08:00", nextRunAt: 1792629000, history: [])
    }
    return store
}
'''
    if source.name == "AieyesApp.swift":
        text = text.replace("200_000_000", "1_200_000_000")
        anchor = 'try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)'
        assert text.count(anchor) == 1
        text = text.replace(anchor, anchor + '''
            let shots = [("overview", "agent", "integrations", "appearance"),
                         ("analytics", "analysis", "integrations", "appearance"),
                         ("servers", "servers", "integrations", "appearance")]
            let selection: PetSelection = try await model.engine.call("pets.current")
            model.importedPet = selection.pet
            for (feature, page, tab, general) in shots {
                model.detailPage = page; model.settingsTab = tab; model.settingsAgent = "codex"; model.generalPage = general
                var state = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)
                state[NativePanelKey.expansion(.quota, id: "codex:codex", compact: false)] = true
                UserDefaults.standard.setVolatileDomain(state, forName: UserDefaults.argumentDomain)
                try await capture(MainWindowView(model: model), size: NSSize(width: 1360, height: 1000), dark: true, to: root.appendingPathComponent("macos-\\(feature).png"))
            }
            model.detailPage = "agent"; model.panelHeight = 720
            var preview = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)
            preview[NativePanelKey.panelSessions.rawValue] = true
            preview[NativePanelKey.expansion(.quota, id: "codex:codex", compact: true)] = true
            UserDefaults.standard.setVolatileDomain(preview, forName: UserDefaults.argumentDomain)
            try await capture(RootView(model: model), size: NSSize(width: 450, height: 720), dark: true, to: root.appendingPathComponent("macos-panel.png"))
            model.stop(); NSApp.terminate(nil); return ()
''')
        start = text.index('            if ProcessInfo.processInfo.environment["AIEYES_UI_LAYOUT_ONLY"]')
        end = text.index('        } catch { fputs', start)
        text = text[:start] + text[end:]
    (SOURCE / source.name).write_text(text)
if "--prepare" in sys.argv:
    print(SOURCE); sys.exit(0)

framework = ROOT / ".build/swift/debug"
if not (framework / "Sparkle.framework").exists():
    raise SystemExit("Build the existing macOS target first so its cached Sparkle framework is available.")
binary = APP / "Contents/MacOS/AieyesPreview"
binary.parent.mkdir(parents=True, exist_ok=True)
resources = APP / "Contents/Resources"
resources.mkdir(parents=True, exist_ok=True)
for source in [ROOT / "apps/macos/Resources/Brand.png", ROOT / "apps/macos/Resources/BrandTemplate.png", *(ROOT / "apps/desktop/web").glob("provider-*.png")]:
    shutil.copy2(source, resources / source.name)
import plistlib
with (APP / "Contents/Info.plist").open("wb") as f:
    plistlib.dump(dict(CFBundleIdentifier="app.aieyes.marketing-preview", CFBundleExecutable="AieyesPreview", CFBundleName="Aieyes Preview", CFBundleShortVersionString="1.0", LSUIElement=True), f)
command = ["swiftc", "-parse-as-library", "-swift-version", "5", "-target", platform.machine()+"-apple-macosx14.0", "-module-cache-path", str(ROOT / ".build/swift-module-cache"), "-F", str(framework), "-framework", "Sparkle", "-Xlinker", "-rpath", "-Xlinker", str(framework), "-o", str(binary), *map(str, sorted(SOURCE.glob("*.swift")))]
if "--capture" not in sys.argv:
    subprocess.run(command, check=True)
fixture = ROOT / "scripts/website/fixture.py"
fixture.chmod(0o755)
environment = dict(os.environ, AIEYES_LANGUAGE="en", AIEYES_CORE_PATH=str(fixture), AIEYES_DATA_DIR=str(WORK / "native-data"), AIEYES_UI_SCENARIO="website", AIEYES_UI_STAMP="1792584000", AIEYES_PREVIEW_FIXTURE=str(WORK / "fixture.json"))
subprocess.run([str(binary), "--render", str(OUT)], env=environment, check=True, timeout=180)
subprocess.run(["node", str(ROOT / "scripts/website/encode.cjs")], check=True)
print("Rendered native macOS product previews:", OUT)
