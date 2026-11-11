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
parser.add_argument("--demo", action="store_true", help="render the states of the website demo only")
args = parser.parse_args()
languages = ["en", "zh-CN"] if args.language == "all" else [args.language]
ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / ".local/website"
# The website demo's windows: an audited main-window width at a height that fits a laptop screen, and the panel.
DEMO_SIZE = {"main": (1120, 760), "panel": (450, 720)}
DEMO_SWIFT = {"DEMO_MAIN_WIDTH": DEMO_SIZE["main"][0], "DEMO_MAIN_HEIGHT": DEMO_SIZE["main"][1],
              "DEMO_PANEL_WIDTH": DEMO_SIZE["panel"][0], "DEMO_PANEL_HEIGHT": DEMO_SIZE["panel"][1]}
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
    if source.name == "MainWindowView.swift":
        # cacheDisplay cannot draw the split view's floating glass sidebar. Lay out the same two columns in
        # an HStack and draw the sidebar with the app's own navigation buttons and group labels.
        for old, new in [("NavigationSplitView(columnVisibility: columnVisibility) {", "HStack(spacing: 0) {"),
                         (".navigationSplitViewColumnWidth(min: 190, ideal: 214, max: 260)", ""),
                         (".toolbar(removing: .sidebarToggle)", ""),
                         ("} detail: {", "Divider()"), ("}.navigationSplitViewStyle(.balanced)", "}"),
                         # The sidebar list's source-list material is not captured either; draw a plain list.
                         ("}.listStyle(.sidebar)", "}.listStyle(.plain).scrollContentBackground(.hidden)")]:
            assert text.count(old) == 1, old
            text = text.replace(old, new)
        # Rows of the plain list expose no accessibility frames; the capture dump records each label's own layout frame.
        row = "Label { Text(title).lineLimit(1) } icon: { Image(systemName: icon) }\n            .accessibilityLabel(title)"
        assert text.count(row) == 1
        text = text.replace(row, row + "\n            .background(GeometryReader { geometry in Color.clear.onChange(of: geometry.frame(in: .global), initial: true) { _, frame in websiteSidebarRows[title] = frame } })")
        text += "\n@MainActor var websiteSidebarRows: [String: CGRect] = [:]\n"
    if source.name == "AieyesApp.swift":
        text = text.replace("200_000_000", "1_200_000_000")
        # Keep invisible capture windows on a real screen so AppKit draws layers.
        offscreen = "window.setFrameOrigin(NSPoint(x: -20000, y: -20000))"
        assert text.count(offscreen) == 1
        text = text.replace(offscreen, """window.alphaValue = 0
        if let screen = NSScreen.main { window.setFrameOrigin(NSPoint(x: screen.frame.maxX - size.width, y: screen.frame.minY)) }""")
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
    let sidebar: [[String: Any]] = websiteSidebarRows.map { ["title": $0.key, "x": $0.value.minX, "y": $0.value.minY, "width": $0.value.width, "height": $0.value.height] }
    try JSONSerialization.data(withJSONObject: ["width": bounds.width, "height": bounds.height, "title": window.title, "sidebar": sidebar, "elements": rows], options: [.prettyPrinted, .sortedKeys]).write(to: url)
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
            if let demo = ProcessInfo.processInfo.environment["AIEYES_DEMO_STATE"] {
                // name|surface|page|settings tab|general page|agent|sessions expanded|Codex card expanded
                let parts = demo.split(separator: "|", omittingEmptySubsequences: false).map(String.init)
                guard parts.count == 8, ["main", "panel"].contains(parts[1]) else { throw ClientError.message("Invalid demo state") }
                let directory = root.appendingPathComponent("demo")
                try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
                let panel = parts[1] == "panel"
                model.detailPage = parts[2]; model.settingsTab = parts[3]; model.generalPage = parts[4]; model.settingsAgent = parts[5]; model.panelHeight = DEMO_PANEL_HEIGHT
                var preview = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)
                preview[NativePanelKey.scoped(.panelSessions, .detailSessions, compact: panel)] = parts[6] == "1"
                preview[NativePanelKey.expansion(.quota, id: "codex:codex", compact: panel)] = parts[7] == "1"
                preview["setup.scene.v1"] = "mixed"; preview["setup.expanded.v2"] = ""
                UserDefaults.standard.setVolatileDomain(preview, forName: UserDefaults.argumentDomain)
                NSApp.accessibilitySetValue(true, forAttribute: NSAccessibility.Attribute(rawValue: "AXEnhancedUserInterface"))
                let url = directory.appendingPathComponent(parts[0] + ".png")
                if panel { try await capture(RootView(model: model, page: parts[2]), size: NSSize(width: DEMO_PANEL_WIDTH, height: DEMO_PANEL_HEIGHT), dark: true, to: url) }
                else { try await capture(MainWindowView(model: model), size: NSSize(width: DEMO_MAIN_WIDTH, height: DEMO_MAIN_HEIGHT), dark: true, to: url) }
                model.stop(); NSApp.terminate(nil); return
            }
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
    for key, value in DEMO_SWIFT.items():
        text = text.replace(key, str(value))
    (SOURCE / source.name).write_text(text)
if "--prepare" in sys.argv:
    print(SOURCE); sys.exit(0)

# Website demo: one native capture per place the tour or the live Windows frontend can reach.
def demo_state(name, page, surface="main", tab="integrations", general="appearance", agent="codex", sessions=False, card=True):
    return dict(name=name, surface=surface, page=page, tab=tab, general=general, agent=agent, sessions=sessions, card=card)


DEMO_GENERAL = ["appearance", "desktop", "network", "refresh", "notifications", "advanced", "updates"]
DEMO_AGENTS = ["codex", "claude", "antigravity"]
general_state = lambda key: "settings" if key == "appearance" else "settings-" + key
agent_state = lambda key: "accounts" if key == "codex" else "accounts-" + key
DEMO_STATES = [
    demo_state("overview", "agent"),
    demo_state("overview-sessions", "agent", sessions=True),
    demo_state("overview-collapsed", "agent", card=False),
    demo_state("overview-sessions-collapsed", "agent", sessions=True, card=False),
    demo_state("analytics", "analysis"),
    demo_state("servers", "servers"),
    *[demo_state(agent_state(key), "settings", agent=key) for key in DEMO_AGENTS],
    demo_state("machines", "settings", tab="servers"),
    demo_state("prices", "settings", tab="prices"),
    demo_state("wakeups", "settings", tab="wakeups"),
    *[demo_state(general_state(key), "settings", tab="general", general=key) for key in DEMO_GENERAL],
    demo_state("setup", "setup"),
    demo_state("panel", "agent", surface="panel", sessions=True),
    demo_state("panel-quotas", "agent", surface="panel"),
    demo_state("panel-servers", "servers", surface="panel", sessions=True),
]
DEMO_INITIAL = {"main": "overview", "panel": "panel"}


def write_demo_manifest():
    manifest = {surface: dict(width=size[0], height=size[1], initial=DEMO_INITIAL[surface], states={}) for surface, size in DEMO_SIZE.items()}
    for state in DEMO_STATES:
        name, surface = state["name"], state["surface"]
        width, height = DEMO_SIZE[surface]
        # `view` is where the state sits in the application, in the native application's own terms.
        entry = dict(view={key: state[key] for key in ("page", "tab", "general", "agent", "sessions", "card")}, title={})
        for language in ("en", "zh-CN"):
            dump = OUT / language / "demo" / (name + ".json")
            if not dump.exists():
                raise SystemExit(f"Missing {dump}; render both languages once with --demo --language all.")
            data = json.loads(dump.read_text())
            if (round(data["width"]), round(data["height"])) != (width, height):
                raise SystemExit(f"{dump}: unexpected capture size")
            if not (ROOT / "apps/website/public/demo/macos" / language / (name + ".webp")).exists():
                raise SystemExit(f"Missing encoded image for {language} {name}")
            if surface == "main":
                title = data["title"]
                if not title.endswith(" — Aieyes"):
                    raise SystemExit(f"{dump}: window title was not set")
                entry["title"][language] = title[:-len(" — Aieyes")]
            else:
                # The panel has no title bar; its selected tab names it.
                tabs = [e for e in data["elements"] if e["role"] == "AXRadioButton" and e["frameAvailable"] and e["width"] > 0 and e["height"] > 0][:2]
                if len(tabs) != 2:
                    raise SystemExit(f"{dump}: panel tabs are missing")
                entry["title"][language] = tabs[0 if state["page"] == "agent" else 1]["label"]
        manifest[surface]["states"][name] = entry
    target = ROOT / "apps/website/demo/macos.json"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print("Wrote demo manifest:", target)

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
    if args.demo:
        shutil.rmtree(output / "demo", ignore_errors=True)
        def capture_demo(state):
            demo_env = dict(environment, AIEYES_LAYOUT_AUDIT="1", AIEYES_DEMO_STATE="|".join([state["name"], state["surface"], state["page"], state["tab"], state["general"], state["agent"], str(int(state["sessions"])), str(int(state["card"]))]),
                            AIEYES_DATA_DIR=str(WORK / "native-demo-data" / language / state["name"]))
            subprocess.run([str(binary), "--render", str(output)], env=demo_env, check=True, timeout=60)
            print("Captured demo state:", language, state["name"], flush=True)

        # One process per state, as for the layout audit: each owns its accessibility tree.
        with ThreadPoolExecutor(max_workers=2) as executor:
            for index in range(0, len(DEMO_STATES), 2):
                list(executor.map(capture_demo, DEMO_STATES[index:index + 2]))
        continue
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
if args.demo:
    # The eight gallery screenshots are left as they are; only the demo states are encoded.
    subprocess.run(["node", str(ROOT / "scripts/website/encode.cjs"), "--demo", "--language", args.language], check=True)
    write_demo_manifest()
    sys.exit(0)
subprocess.run(["node", str(ROOT / "scripts/website/encode.cjs"), "--language", args.language], check=True)
print("Rendered native macOS product previews:", OUT)
