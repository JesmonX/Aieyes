#!/usr/bin/env python3
"""Render the current native macOS UI with the promo fixture, at 2x, for the promo video.

Works like scripts/website/render.py: an isolated copy of apps/macos/Sources is
patched and compiled under .local/promo. No production sources, saved
preferences, accounts or app bundles are modified. Output PNGs land in
.local/promo/native/<language>/. Needs a graphics session; the capture window
reports a 2x backing scale on any screen and stays fully transparent.
"""
import argparse
import os
from pathlib import Path
import platform
import plistlib
import shutil
import struct
import subprocess
import sys

parser = argparse.ArgumentParser()
parser.add_argument("--language", choices=["all", "en", "zh-CN"], default="all")
parser.add_argument("--skip-build", action="store_true")
parser.add_argument("--theme", choices=["dark", "light"], default="dark")
parser.add_argument("--story", choices=["overview", "workday"], default="overview",
                    help="workday renders the Chinese day-in-the-life shots at five moments of one day")
args = parser.parse_args()
languages = ["en", "zh-CN"] if args.language == "all" else [args.language]
if args.story == "workday" and languages != ["zh-CN"]:
    parser.error("--story workday is Chinese only; pass --language zh-CN")
ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / ".local/promo"
SOURCE = WORK / "native-source"
APP = WORK / "AieyesPromo.app"
STAMP = "Date(timeIntervalSince1970: promoStamp)"
# Workday moments on 2026-10-21 (UTC+08:00): state → clock.
MOMENTS = {"morning": 1792550400, "switched": 1792550700, "afternoon": 1792570800, "evening": 1792584600, "night": 1792593000}

# Shots: name, Swift expression producing the view, width, height (0 = fitting height), opaque background.
SHOTS = r'''
            let dark = ProcessInfo.processInfo.environment["AIEYES_PROMO_THEME"] != "light"
            let selection: PetSelection = try await model.engine.call("pets.current")
            model.importedPet = selection.pet
            model.settingsAgent = "codex"; model.generalPage = "appearance"; model.panelHeight = 720
            let accounts = ["codex:codex", "codex:codex-team", "claude:claude", "antigravity:antigravity"]
            func prefs(_ values: [String: Any]) {
                var state = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)
                for (key, value) in values { state[key] = value }
                UserDefaults.standard.setVolatileDomain(state, forName: UserDefaults.argumentDomain)
            }
            func cards(_ expanded: Set<String>, compact: Bool = false) {
                prefs(Dictionary(uniqueKeysWithValues: accounts.map { (NativePanelKey.expansion(.quota, id: $0, compact: compact), expanded.contains($0)) }))
            }
            func shot<V: View>(_ name: String, _ view: V, _ width: CGFloat, _ height: CGFloat, opaque: Bool = true) async throws {
                var height = height
                if height == 0 {
                    let probe = NSHostingView(rootView: view.frame(width: width).environment(\.colorScheme, .dark))
                    height = ceil(probe.fittingSize.height)
                }
                try await capture(view, size: NSSize(width: width, height: height), dark: dark, to: root.appendingPathComponent(name + ".png"), opaqueBackground: opaque)
                print("shot", name, Int(width), Int(height)); fflush(stdout)
            }
            func page(_ name: String, _ detail: String, tab: String = "integrations", width: CGFloat = 1360, height: CGFloat = 900) async throws {
                model.detailPage = detail; model.settingsTab = tab
                try await shot(name, MainWindowView(model: model), width, height)
            }
            // Workday story: one moment of the day per launch.
            if let state = ProcessInfo.processInfo.environment["AIEYES_PROMO_STATE"] {
                // The panel, plus a taller copy for scrolling inside the popover.
                func panel(_ name: String, page: String = "agent", tall: CGFloat = 1600) async throws {
                    try await shot(name, RootView(model: model, page: page), 450, 720)
                    model.panelHeight = tall
                    try await shot(name + "-tall", RootView(model: model, page: page), 450, tall)
                    model.panelHeight = 720
                }
                switch state {
                case "morning":
                    prefs([NativePanelKey.panelSessions.rawValue: true])
                    cards(["codex:codex", "codex:codex-team"], compact: true)
                    try await panel("panel")
                    let sessions = model.sessions, unavailable = model.sessionsUnavailable
                    model.sessionsUnavailable = false
                    for phase in [SessionPhase.idle, .working, .thinking, .tool, .needsInput, .complete] {
                        model.sessions = phase == .idle ? [] : [LiveSession(id: "promo", source: "Codex", phase: phase, updatedAt: Date())]
                        try await shot("menu-" + phase.symbol.replacingOccurrences(of: ".", with: "-"), MenuActivityLabel(model: model), 180, 28, opaque: false)
                    }
                    model.sessions = sessions; model.sessionsUnavailable = unavailable
                    promoSettle = 4_000_000_000  // account checks and the switch preparation finish
                    try await shot("targets", AccountTargetsView(model: model, accountKey: "codex:codex-team"), 640, 470)
                    promoInlineConfirmation = true
                    try await shot("confirm", CodexAccountsView(model: model, targetAccountID: "codex-team", initialSourceID: "codex-source", intent: "switch"), 530, 400)
                case "switched":
                    promoSettle = 4_000_000_000
                    try await shot("targets", AccountTargetsView(model: model, accountKey: "codex:codex-team"), 640, 470)
                case "afternoon":
                    prefs([NativePanelKey.panelSessions.rawValue: true])
                    cards(["codex:codex-team"], compact: true)
                    try await panel("panel")
                    if let quota = model.dashboard.quotas.first(where: { $0.id == "codex:codex-team" }) {
                        try await shot("estimate", QuotaEstimateView(model: model, quota: quota), 530, 600)
                        promoEstimateScroll = 2400  // the same window with its scroll area unrolled
                        try await shot("estimate-tall", QuotaEstimateView(model: model, quota: quota), 530, 2520)
                        promoEstimateScroll = 480
                    }
                case "evening":
                    cards(["codex:codex-team"], compact: true)
                    try await shot("panel", RootView(model: model), 450, 720)
                    try await shot("servers", RootView(model: model, page: "servers"), 450, 720)
                    prefs([NativePanelKey.panelServerSelection.rawValue: "preview-host"])
                    try await panel("servers-gpu", page: "servers", tall: 1500)
                case "night":
                    cards(["codex:codex-team"], compact: true)
                    try await shot("panel", RootView(model: model), 450, 720)
                    cards([])
                    try await page("overview", "agent")
                    for variant in ["", "Draft", "Deployed"] {
                        promoWakeupVariant = variant
                        try await page(variant.isEmpty ? "wakeups" : "wakeups-" + variant.lowercased(), "settings", tab: "wakeups")
                    }
                    var task = WakeTask()
                    task.name = "本周赶工"; task.sourceId = "codex-team-source"; task.times = ["07:00", "12:00", "17:00"]; task.model = "gpt-5.1-codex-mini"; task.effort = "low"
                    // The answer of wakeups.probe for a Codex CLI on this Mac.
                    promoWakeCapabilities = WakeCapabilities(models: [WakeModel(id: "gpt-5.1-codex-mini", efforts: ["low", "medium", "high"]), WakeModel(id: "gpt-5.1-codex", efforts: ["low", "medium", "high", "xhigh"])],
                                                             efforts: ["low", "medium", "high", "xhigh", "max"], timezone: "CST +08:00",
                                                             message: "参考 1000 输入 + 100 输出 Token 的 API 等价成本推荐模型；部署后使用所选模型", recommendedModel: "gpt-5.1-codex-mini")
                    promoSettle = 4_000_000_000
                    try await shot("wake-editor", WakeEditor(model: model, task: task, deployed: false, save: { _ in }), 550, 660)
                default: break
                }
                model.stop(); NSApp.terminate(nil); return ()
            }
            prefs([NativePanelKey.detailSessions.rawValue: false, NativePanelKey.panelSessions.rawValue: false])
            cards([])
            try await page("overview-grid", "agent")
            prefs([NativePanelKey.detailSessions.rawValue: true])
            try await page("overview-sessions", "agent")
            prefs([NativePanelKey.detailSessions.rawValue: false])
            cards(["codex:codex"])
            try await page("overview-codex", "agent", height: 1000)
            cards(["codex:codex", "claude:claude"])
            try await page("overview-tall", "agent", height: 1500)
            try await page("analysis", "analysis", height: 900)
            try await page("analysis-tall", "analysis", height: 1700)
            try await page("servers", "servers", height: 900)
            try await page("servers-tall", "servers", height: 2300)
            try await page("wakeups", "settings", tab: "wakeups", height: 900)
            try await page("integrations", "settings", tab: "integrations", height: 900)
            try await page("machines", "settings", tab: "servers", height: 900)
            for host in model.settings.monitoredHosts {
                try await shot("server-" + host.id, ServerCard(host: host, result: model.hosts.first { $0.id == host.id }, compact: false), 620, 0)
            }
            if let nas = model.settings.hosts.first(where: { $0.id == "nas-host" }) {
                try await shot("host-editor", HostEditor(host: nas, onSave: { _, _ in }), 620, 980)
            }
            cards([], compact: true)
            prefs([NativePanelKey.expansion(.quota, id: "codex:codex", compact: true): true])
            try await shot("panel", RootView(model: model), 450, 720)
            prefs([NativePanelKey.panelSessions.rawValue: true])
            try await shot("panel-sessions", RootView(model: model), 450, 720)
            // Taller copy of the same panel, for the scrolling demo inside the popover.
            model.panelHeight = 1500
            try await shot("panel-sessions-tall", RootView(model: model), 450, 1500)
            model.panelHeight = 720
            prefs([NativePanelKey.panelSessions.rawValue: false])
            try await shot("panel-servers", RootView(model: model, page: "servers"), 450, 720)
            if let codex = model.dashboard.quotas.first(where: { $0.id == "codex:codex" }) {
                try await shot("estimate", QuotaEstimateView(model: model, quota: codex), 518, 620)
                try await shot("estimate-credits", QuotaEstimateView(model: model, quota: codex, credits: true), 518, 620)
            }
            try await shot("valuation", AgentValuationView(model: model, provider: "codex"), 660, 650)
            promoSettle = 4_000_000_000  // let the per-machine login check finish
            try await shot("account-switch", AccountTargetsView(model: model, accountKey: "codex:codex"), 640, 470)
            promoSettle = 1_200_000_000
            try await shot("pet-library", PetLibraryView(model: model).padding(20).environment(\.surfaceActive, true), 530, 530)
            let sessions = model.sessions, unavailable = model.sessionsUnavailable
            model.sessionsUnavailable = false
            for phase in [SessionPhase.idle, .working, .tool, .needsInput, .complete] {
                model.sessions = phase == .idle ? [] : [LiveSession(id: "promo", source: "Codex", phase: phase, updatedAt: Date())]
                try await shot("menu-" + phase.symbol.replacingOccurrences(of: ".", with: "-"), MenuActivityLabel(model: model), 180, 28, opaque: false)
            }
            model.sessions = sessions; model.sessionsUnavailable = unavailable
            model.stop(); NSApp.terminate(nil); return ()
'''

WAKEUPS = '''
@MainActor var promoWakeupVariant = ""
// The wake editor opens as if 读取可用模型 had already answered.
nonisolated(unsafe) var promoWakeCapabilities: WakeCapabilities?
@MainActor private func promoWakeups() -> WakeupsStore {
    let store = WakeupsStore()
    let path = ProcessInfo.processInfo.environment["AIEYES_PREVIEW_FIXTURE"]!
    let object = try! JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: path))) as! [String: Any]
    store.records = try! JSONDecoder().decode([WakeRecord].self, from: JSONSerialization.data(withJSONObject: object["wakeups" + promoWakeupVariant]!))
    store.statuses = try! JSONDecoder().decode([String: WakeStatus].self, from: JSONSerialization.data(withJSONObject: object["wakeupStatuses" + promoWakeupVariant]!))
    return store
}
'''


def patch(name, text):
    text = text.replace("Date()", STAMP).replace("Date.now", STAMP).replace("date: .now", "date: " + STAMP)
    if name == "PetWindow.swift":
        text = text.replace("let showing = PetPreviewVisibility.isVisible(geometry.frame(in: .global), viewport: viewport, enclosing: enclosingViewport)", "let showing = true")
    if name == "Views.swift":
        # Server status ticks with the real clock; keep it on the fixture's sample time.
        old = "let label = status(at: context.date)"
        assert text.count(old) == 1
        text = text.replace(old, "let label = status(at: " + STAMP + ")")
    if name == "SettingsView.swift":
        # Show the host's metric and device choices instead of the folded section.
        old = 'DisclosureGroup(L("监控指标与设备")) {'
        assert text.count(old) == 1
        text = text.replace(old, 'DisclosureGroup(L("监控指标与设备"), isExpanded: .constant(true)) {')
    if name == "WakeupsView.swift":
        for old, new in [("@StateObject private var store = WakeupsStore()", "@StateObject private var store = promoWakeups()"), (".task { await store.load() }", ".task {}"),
                         ("@State private var capabilities: WakeCapabilities?", "@State private var capabilities: WakeCapabilities? = promoWakeCapabilities")]:
            assert text.count(old) == 1, old
            text = text.replace(old, new)
        text += WAKEUPS
    if name == "MainWindowView.swift":
        # cacheDisplay cannot draw the split view's floating glass sidebar. Lay out the same two columns in
        # an HStack and draw the sidebar with the app's own navigation buttons and group labels.
        for old, new in [("NavigationSplitView {", "HStack(spacing: 0) {"),
                         (".navigationSplitViewColumnWidth(min:collapsed ? 62 : 190,ideal:collapsed ? 62 : 214,max:collapsed ? 62 : 260)", ""),
                         ("} detail: {", "Divider()"), (".navigationSplitViewStyle(.balanced).toolbar(removing:.sidebarToggle)", ""),
                         ("if collapsed {\n                    ScrollView { VStack(spacing:4) {\n                        ForEach(monitoring",
                          "if true {\n                    ScrollView { VStack(alignment: .leading, spacing:4) {\n                        groupLabel(L(\"监测\"))\n                        ForEach(monitoring"),
                         ("Divider()\n                        ForEach(configuration", "groupLabel(L(\"配置\"))\n                        ForEach(configuration")]:
            assert text.count(old) == 1, old
            text = text.replace(old, new)
    if name == "CodexAccountsView.swift":
        # Show the switch confirmation in place of its sheet; a capture cannot include sheet windows.
        old = "var body: some View {\n        let _ = interfaceLanguage;\n        Form {"
        assert text.count(old) == 1
        text = text.replace(old, "var body: some View {\n        if promoInlineConfirmation && switchSheet { switchConfirmation } else { promoForm }\n    }\n"
                                 "    @ViewBuilder private var promoForm: some View {\n        let _ = interfaceLanguage;\n        Form {")
        old = ".sheet(isPresented: $switchSheet,"
        assert text.count(old) == 1
        text = text.replace(old, ".sheet(isPresented: promoInlineConfirmation ? .constant(false) : $switchSheet,")
        text += "\n@MainActor var promoInlineConfirmation = false\n"
    if name == "QuotaTools.swift":
        # The workday video scrolls the estimate window; render its scroll area unrolled as well.
        old = "ScrollView { content }.frame(maxHeight: 480)"
        assert text.count(old) == 1
        text = text.replace(old, "ScrollView { content }.frame(maxHeight: promoEstimateScroll)")
        text += "\n@MainActor var promoEstimateScroll: CGFloat = 480\n"
    if name == "AieyesApp.swift":
        # Async page loads (estimates, targets, wakeups) settle before each capture.
        assert text.count("try await Task.sleep(nanoseconds: 200_000_000)") == 1
        text = text.replace("try await Task.sleep(nanoseconds: 200_000_000)", "try await Task.sleep(nanoseconds: promoSettle)")
        text += "\n@MainActor private var promoSettle: UInt64 = 1_200_000_000\n"
        text += 'let promoStamp = TimeInterval(ProcessInfo.processInfo.environment["AIEYES_PROMO_STAMP"] ?? "") ?? 1792584060\n'
        # Layer contents rasterize at the window's backing scale. The capture window reports 2x,
        # so any screen works; it stays invisible in the corner of the main screen.
        window = "let window = NSWindow(contentRect: view.frame, styleMask: [.borderless], backing: .buffered, defer: false)"
        assert text.count(window) == 1
        text = text.replace(window, window.replace("NSWindow(", "PromoRetinaWindow("))
        text += "\nfinal class PromoRetinaWindow: NSWindow { override var backingScaleFactor: CGFloat { 2 } }\n"
        offscreen = "window.setFrameOrigin(NSPoint(x: -20000, y: -20000))"
        assert text.count(offscreen) == 1
        text = text.replace(offscreen, """window.alphaValue = 0
        if let screen = NSScreen.main { window.setFrameOrigin(NSPoint(x: screen.frame.maxX - size.width, y: screen.frame.minY)) }""")
        anchor = "try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)"
        assert text.count(anchor) == 1
        start = text.index(anchor) + len(anchor)
        end = text.index("        } catch { fputs", start)
        text = text[:start] + "\n" + SHOTS + text[end:]
    return text


def png_size(path):
    with open(path, "rb") as f:
        head = f.read(24)
    return struct.unpack(">II", head[16:24])


SOURCE.mkdir(parents=True, exist_ok=True)
for stale in SOURCE.glob("*.swift"):
    stale.unlink()
for source in (ROOT / "apps/macos/Sources/Aieyes").glob("*.swift"):
    (SOURCE / source.name).write_text(patch(source.name, source.read_text()))

framework = ROOT / ".build/swift/debug"
if not (framework / "Sparkle.framework").exists():
    raise SystemExit("Build the existing macOS target first so its cached Sparkle framework is available.")
binary = APP / "Contents/MacOS/AieyesPromo"
binary.parent.mkdir(parents=True, exist_ok=True)
resources = APP / "Contents/Resources"
resources.mkdir(parents=True, exist_ok=True)
for source in [ROOT / "apps/macos/Resources/Brand.png", ROOT / "apps/macos/Resources/BrandTemplate.png", *(ROOT / "apps/desktop/web").glob("provider-*.png")]:
    shutil.copy2(source, resources / source.name)
with (APP / "Contents/Info.plist").open("wb") as f:
    plistlib.dump(dict(CFBundleIdentifier="app.aieyes.promo-preview", CFBundleExecutable="AieyesPromo", CFBundleName="Aieyes Promo", CFBundleShortVersionString="1.0", LSUIElement=True), f)
if not args.skip_build:
    subprocess.run(["swiftc", "-parse-as-library", "-swift-version", "5", "-target", platform.machine() + "-apple-macosx14.0",
                    "-module-cache-path", str(ROOT / ".build/swift-module-cache"), "-F", str(framework), "-framework", "Sparkle",
                    "-Xlinker", "-rpath", "-Xlinker", str(framework), "-o", str(binary), *map(str, sorted(SOURCE.glob("*.swift")))], check=True)

fixture = ROOT / "scripts/promo/fixture.py"
fixture.chmod(0o755)
jobs = [(language, "") for language in languages] if args.story == "overview" else [("zh-CN", state) for state in MOMENTS]
for language, state in jobs:
    output = WORK / "native" / (language + ("-light" if args.theme == "light" else "") + ("-workday" if state else "")) / state
    if output.exists():
        shutil.rmtree(output)
    output.mkdir(parents=True)
    fixture_path = WORK / f"fixture-{language}-{args.theme}{'-' + state if state else ''}.json"
    environment = dict(os.environ, AIEYES_LANGUAGE=language, AIEYES_CORE_PATH=str(fixture), AIEYES_DATA_DIR=str(WORK / "native-data" / language),
                       AIEYES_UI_SCENARIO="website", AIEYES_UI_STAMP="1792584000", AIEYES_PREVIEW_FIXTURE=str(fixture_path), AIEYES_PROMO_THEME=args.theme)
    for key in ("AIEYES_CAPTURE_TARGET", "AIEYES_LAYOUT_AUDIT", "AIEYES_UI_LAYOUT_ONLY", "AIEYES_UI_SETUP_ONLY", "AIEYES_UI_WORKSPACE_ONLY", "AIEYES_PROMO_STATE", "AIEYES_PROMO_STAMP"):
        environment.pop(key, None)
    if state:
        environment.update(AIEYES_PROMO_STATE=state, AIEYES_PROMO_STAMP=str(MOMENTS[state]))
    fixture_path.write_bytes(subprocess.check_output([sys.executable, str(fixture), "--fixture"], env=environment))
    log = subprocess.run([str(binary), "--render", str(output)], env=environment, check=True, timeout=600, capture_output=True, text=True).stdout
    shots = [line.split() for line in log.splitlines() if line.startswith("shot ")]
    for _, name, width, height in shots:
        if png_size(output / (name + ".png")) != (2 * int(width), 2 * int(height)):
            raise SystemExit(f"{name}.png was not rendered at 2x")
    print(f"Rendered {len(shots)} {' '.join(filter(None, [language, state]))} shots into {output}")
