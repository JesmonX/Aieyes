#!/usr/bin/env python3
"""Render the current native UI in an isolated bilingual preview source tree.

No production sources, saved preferences, accounts or app bundles are modified.
Run with --prepare to inspect the fixture source without launching the GUI.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys

parser = argparse.ArgumentParser()
parser.add_argument("--language", choices=["all", "en", "zh-CN"], default="all")
parser.add_argument("--prepare", action="store_true")
parser.add_argument("--capture", action="store_true")
parser.add_argument("--audit-layout", action="store_true")
args = parser.parse_args()
languages = ["en", "zh-CN"] if args.language == "all" else [args.language]
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
        capture_anchor = 'if let data = bitmap.representation(using: .png, properties: [:]) { try data.write(to: url) }'
        assert text.count(capture_anchor) == 1
        text = text.replace(capture_anchor, capture_anchor + '\n        try websiteLayoutSnapshot(view, to: url.deletingPathExtension().appendingPathExtension("json"))')
        text = text.replace('        defer { window.close() }', '''
        let auditing = ProcessInfo.processInfo.environment["AIEYES_LAYOUT_AUDIT"] == "1"
        if auditing { websiteAuditWindow = window }
        defer { if !auditing { window.close() } }
''')
        text += '''
// Each AX capture runs in its own preview process. Keep the window mounted until
// exit so SwiftUI never updates a retired virtual accessibility tree.
@MainActor private var websiteAuditWindow: NSWindow?
@MainActor private func websiteLayoutSnapshot(_ view: NSView, to url: URL) throws {
    guard ProcessInfo.processInfo.environment["AIEYES_LAYOUT_AUDIT"] == "1", let window = view.window else { return }
    view.layoutSubtreeIfNeeded()
    let bounds = window.convertToScreen(view.convert(view.bounds, to: nil))
    var seen = Set<ObjectIdentifier>(), rows: [[String: Any]] = []
    var truncated = false
    func visit(_ object: Any, path: String, depth: Int = 0) {
        guard depth <= 32, rows.count < 4096 else { truncated = true; return }
        // SwiftUI's virtual elements expose AX selectors without adopting the
        // full AppKit protocol. Dynamic dispatch also includes those controls.
        let element = object as AnyObject
        let identity = ObjectIdentifier(element)
        guard seen.insert(identity).inserted else { return }
        let rawFrame = element.accessibilityFrame?() ?? .null
        // SwiftUI uses CGRect.null for virtual/offscreen elements. Preserve
        // their identity, but never serialize its infinite coordinates.
        let hasFrame = !rawFrame.isNull && !rawFrame.isInfinite
            && [rawFrame.minX, rawFrame.minY, rawFrame.width, rawFrame.height].allSatisfy { $0.isFinite }
        let frame = hasFrame ? rawFrame : .zero
        let valueSelector = NSSelectorFromString("accessibilityValue")
        let value = (object as? NSObject).flatMap { object in
            object.responds(to: valueSelector) ? object.perform(valueSelector)?.takeUnretainedValue() as? String : nil
        } ?? ""
        rows.append(["path": path, "role": element.accessibilityRole?()?.rawValue ?? "",
                     "label": element.accessibilityLabel?() ?? "", "value": value,
                     "id": element.accessibilityIdentifier?() ?? "",
                     "frameAvailable": hasFrame,
                     "x": hasFrame ? frame.minX - bounds.minX : 0, "y": hasFrame ? bounds.maxY - frame.maxY : 0,
                     "width": frame.width, "height": frame.height])
        for (index, child) in (element.accessibilityChildren?() ?? []).enumerated() { visit(child, path: path + "." + String(index), depth: depth + 1) }
    }
    visit(view, path: "root")
    guard !truncated, rows.count > 20 else { throw ClientError.message("Incomplete preview accessibility tree") }
    try JSONSerialization.data(withJSONObject: ["width": bounds.width, "height": bounds.height, "elements": rows], options: [.prettyPrinted, .sortedKeys]).write(to: url)
}
'''
        anchor = 'try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)'
        assert text.count(anchor) == 1
        text = text.replace(anchor, anchor + '''
            let shots = [("overview", "agent", "integrations", "appearance"),
                         ("analytics", "analysis", "integrations", "appearance"),
                         ("servers", "servers", "integrations", "appearance")]
            let selection: PetSelection = try await model.engine.call("pets.current")
            model.importedPet = selection.pet
            if let target = ProcessInfo.processInfo.environment["AIEYES_CAPTURE_TARGET"] {
                let parts = target.split(separator: "-").map(String.init)
                guard parts.count == 3, let width = Int(parts[1]), ["light", "dark"].contains(parts[2]),
                      (parts[0] == "panel" ? [360, 450] : [760, 1120, 1440]).contains(width),
                      ["panel", "overview", "analytics", "servers"].contains(parts[0]) else {
                    throw ClientError.message("Invalid preview capture target")
                }
                let audit = root.appendingPathComponent("audit")
                try FileManager.default.createDirectory(at: audit, withIntermediateDirectories: true)
                let panel = parts[0] == "panel", dark = parts[2] == "dark"
                model.detailPage = parts[0] == "analytics" ? "analysis" : parts[0] == "servers" ? "servers" : "agent"
                model.settingsTab = "integrations"; model.settingsAgent = "codex"; model.panelHeight = 720
                var preview = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)
                preview[NativePanelKey.panelSessions.rawValue] = true
                preview[NativePanelKey.expansion(.quota, id: "codex:codex", compact: panel)] = true
                UserDefaults.standard.setVolatileDomain(preview, forName: UserDefaults.argumentDomain)
                NSApp.accessibilitySetValue(true, forAttribute: NSAccessibility.Attribute(rawValue: "AXEnhancedUserInterface"))
                let url = audit.appendingPathComponent(target + ".png")
                if panel {
                    try await capture(RootView(model: model), size: NSSize(width: width, height: 720), dark: dark, to: url)
                } else {
                    try await capture(MainWindowView(model: model), size: NSSize(width: width, height: 900), dark: dark, to: url)
                }
                model.stop(); NSApp.terminate(nil); return
            }
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
for language in languages:
    output = OUT / language
    output.mkdir(parents=True, exist_ok=True)
    fixture_path = WORK / f"fixture-{language}.json"
    environment = dict(os.environ, AIEYES_LANGUAGE=language, AIEYES_CORE_PATH=str(fixture), AIEYES_DATA_DIR=str(WORK / "native-data" / language), AIEYES_UI_SCENARIO="website", AIEYES_UI_STAMP="1792584000", AIEYES_PREVIEW_FIXTURE=str(fixture_path), AIEYES_LAYOUT_AUDIT="0")
    environment.pop("AIEYES_CAPTURE_TARGET", None)
    fixture_path.write_bytes(subprocess.check_output([sys.executable, str(fixture), "--fixture"], env=environment))
    subprocess.run([str(binary), "--render", str(output)], env=environment, check=True, timeout=180)
    if args.audit_layout:
        def capture_audit(target):
            audit_env = dict(environment, AIEYES_LAYOUT_AUDIT="1", AIEYES_CAPTURE_TARGET=target,
                             AIEYES_DATA_DIR=str(WORK / "native-audit-data" / language / target))
            subprocess.run([str(binary), "--render", str(output)], env=audit_env, check=True, timeout=45)
            print("Captured native layout:", language, target, flush=True)

        # Separate processes, fixture-only RPC, unique output/data paths. Bound
        # concurrency and stop after the current pair if either capture fails.
        with ThreadPoolExecutor(max_workers=2) as executor:
            for theme in ("light", "dark"):
                targets = [f"{feature}-{width}-{theme}" for width in (760, 1120, 1440) for feature in ("overview", "analytics", "servers")]
                targets += [f"panel-{width}-{theme}" for width in (360, 450)]
                for index in range(0, len(targets), 2):
                    list(executor.map(capture_audit, targets[index:index + 2]))
subprocess.run(["node", str(ROOT / "scripts/website/encode.cjs"), "--language", args.language], check=True)
print("Rendered native macOS product previews:", OUT)
