import AppKit
import SwiftUI
@testable import Aieyes

@main struct VerifyDisplayLab {
    @MainActor static func main() async throws {
        _ = NSApplication.shared
        NSApp.setActivationPolicy(.accessory)
        let root = URL(fileURLWithPath: CommandLine.arguments.dropFirst().first ?? ".local/display-lab-verification")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let preferences = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)
        defer { UserDefaults.standard.setVolatileDomain(preferences, forName: UserDefaults.argumentDomain) }
        let defaultsDomain = Bundle.main.bundleIdentifier ?? ProcessInfo.processInfo.processName
        let savedDefaults = UserDefaults.standard.persistentDomain(forName: defaultsDomain)
        defer {
            if let savedDefaults { UserDefaults.standard.setPersistentDomain(savedDefaults, forName: defaultsDomain) }
            else { UserDefaults.standard.removePersistentDomain(forName: defaultsDomain) }
        }
        var overrides: [String: Any] = [DisplayPreferences.dockEnabled: false, DisplayPreferences.cardEnabled: false, DisplayPreferences.ribbonEnabled: false, DisplayPreferences.animations: false]
        UserDefaults.standard.setVolatileDomain(overrides, forName: UserDefaults.argumentDomain)

        let now = Date()
        var snapshot = DisplaySnapshot.sample(.working, now: now)
        precondition(snapshot.phase(at: now) == .working)
        precondition(snapshot.phase(at: now.addingTimeInterval(121)) == .unknown, "An abandoned snapshot must not claim ongoing work")
        snapshot.appRunning = false
        precondition(snapshot.phase(at: now) == .unknown)
        snapshot.appRunning = true
        let tomorrow = Calendar.current.date(byAdding: .day, value: 1, to: now)!
        precondition(snapshot.tokens(at: tomorrow) == nil, "Yesterday's usage must not appear under Today")
        precondition(DisplaySnapshot.compact(.nan) == "—" && DisplaySnapshot.compact(-1) == "—")
        precondition(DisplayQuota(provider: "Codex", title: "5h", remaining: 140).fraction == 1)
        precondition(DisplayQuota(provider: "Codex", title: "5h", remaining: -20).fraction == 0)
        let snapshotFile = root.appendingPathComponent("roundtrip.json")
        try DisplaySnapshotFile.write(snapshot, to: snapshotFile)
        precondition(DisplaySnapshotFile.read(from: snapshotFile) == snapshot)
        let permissions = try FileManager.default.attributesOfItem(atPath: snapshotFile.path)[.posixPermissions] as! NSNumber
        precondition(permissions.intValue == 0o600)
        let encoded = String(data: try Data(contentsOf: snapshotFile), encoding: .utf8)!
        precondition(!encoded.contains("accountId") && !encoded.contains("sourceId") && !encoded.contains("spritePath") && !encoded.contains("email"))
        snapshot.version = 999
        try DisplaySnapshotFile.write(snapshot, to: snapshotFile)
        precondition(!DisplaySnapshotFile.read(from: snapshotFile).appRunning, "An incompatible snapshot must fail closed")
        try Data("partial write".utf8).write(to: snapshotFile)
        precondition(!DisplaySnapshotFile.read(from: snapshotFile).appRunning)

        // A fixture deliberately disagrees with the filtered dashboard. The new
        // displays must consume dashboard.summary, never the active page's filters.
        let fixture = root.appendingPathComponent("core.py")
        let fixtureData = root.appendingPathComponent("dashboard.json")
        var dashboard = Dashboard(); dashboard.generatedAt = now.timeIntervalSince1970
        dashboard.summary.total = 12_345; dashboard.summary.cost = 1.23; dashboard.summary.pricedTokens = 12_345
        try JSONEncoder().encode(dashboard).write(to: fixtureData)
        try """
        #!/usr/bin/env python3
        import json,sys,pathlib
        data=json.loads(pathlib.Path(__file__).with_name('dashboard.json').read_text())
        for line in sys.stdin:
            request=json.loads(line); method=request['method']
            if method=='runtime.revisions': result={}
            elif method=='dashboard.summary': result=data
            elif method=='dashboard':
                result=json.loads(json.dumps(data)); result['summary']['total']=999999
            elif method=='dashboard.status': result={'quotas':[], 'sources':[]}
            else: result={}
            print(json.dumps({'id':request['id'],'result':result}),flush=True)
        """.write(to: fixture, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: fixture.path)
        setenv("AIEYES_CORE_PATH", fixture.path, 1); setenv("AIEYES_DATA_DIR", root.appendingPathComponent("data").path, 1)
        let model = AppModel(autostart: false)
        defer { model.displays.stop(); model.stop() }
        let loaded = await model.reload()
        precondition(loaded && model.dashboard.summary.total == 999999 && model.menuDashboard.summary.total == 12345)
        model.sessions = [LiveSession(id: "working", source: "local", phase: .working, updatedAt: now), LiveSession(id: "failed", source: "local", phase: .failed, updatedAt: now), LiveSession(id: "waiting", source: "local", phase: .needsInput, updatedAt: now)]
        let live = DisplayStore.liveSnapshot(model: model, now: now)
        precondition(live.phase == .needsInput && live.activeCount == 2 && live.waitingCount == 1 && live.failedCount == 1)
        precondition(live.totalTokens == 12345 && !live.isDemo)
        model.sessionsUnavailable = true
        precondition(DisplayStore.liveSnapshot(model: model).phase == .needsInput, "A partial source failure must not hide known attention states")
        let sessions = model.sessions, settings = model.settings
        model.displays.scenario = .lowQuota
        precondition(model.displays.snapshot.isDemo && model.displays.snapshot.quotas[0].remaining == 8)
        precondition(model.sessions == sessions && model.settings == settings && model.menuDashboard.summary.total == 12345, "Demo scenes must not mutate live data")
        model.displays.scenario = .live
        precondition(!model.displays.snapshot.isDemo && model.displays.snapshot.totalTokens == 12345)
        model.sessions = []
        precondition(DisplayStore.liveSnapshot(model: model).phase == .unknown)

        for blocked in 0..<4 {
            precondition(!DynamicDockController.shouldAnimate(enabled: true, motion: blocked != 0, reduced: blocked == 1, lowPower: blocked == 2, suspended: blocked == 3, style: .eyes, phase: .working))
        }
        precondition(!DynamicDockController.shouldAnimate(enabled: true, motion: true, reduced: false, lowPower: false, suspended: false, style: .eyes, phase: .idle))
        precondition(DynamicDockController.shouldAnimate(enabled: true, motion: true, reduced: false, lowPower: false, suspended: false, style: .eyes, phase: .working))
        var policies: [NSApplication.ActivationPolicy] = []
        let presence = DockPresence { policies.append($0) }
        let dock = DynamicDockController(model: model, presence: presence)
        defer { dock.stop() }
        model.displays.scenario = .needsInput
        overrides[DisplayPreferences.dockEnabled] = true
        UserDefaults.standard.setVolatileDomain(overrides, forName: UserDefaults.argumentDomain)
        NotificationCenter.default.post(name: UserDefaults.didChangeNotification, object: nil)
        try await Task.sleep(for: .milliseconds(100))
        precondition(NSApp.dockTile.contentView is DynamicDockView && NSApp.dockTile.badgeLabel == "1" && policies.last == .regular)
        precondition(!dock.animating, "Disabling animation must remove the Dock timer")
        let draws = dock.drawCount
        try await Task.sleep(for: .milliseconds(300))
        precondition(dock.drawCount == draws, "A static Dock must not keep redrawing")
        overrides[DisplayPreferences.animations] = true
        UserDefaults.standard.setVolatileDomain(overrides, forName: UserDefaults.argumentDomain)
        NotificationCenter.default.post(name: UserDefaults.didChangeNotification, object: nil)
        try await Task.sleep(for: .milliseconds(100))
        let animatedDraws = dock.drawCount
        try await Task.sleep(for: .milliseconds(400))
        if !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion && !ProcessInfo.processInfo.isLowPowerModeEnabled {
            precondition(dock.animating && dock.drawCount > animatedDraws, "An enabled animated Dock must actually advance frames")
        }
        overrides[DisplayPreferences.dockEnabled] = false
        UserDefaults.standard.setVolatileDomain(overrides, forName: UserDefaults.argumentDomain)
        NotificationCenter.default.post(name: UserDefaults.didChangeNotification, object: nil)
        try await Task.sleep(for: .milliseconds(100))
        precondition(NSApp.dockTile.contentView == nil && NSApp.dockTile.badgeLabel == nil && policies.last == .accessory)

        let left = NSRect(x: -1440, y: 0, width: 1440, height: 900), right = NSRect(x: 0, y: 0, width: 1440, height: 900)
        let offscreen = DesktopDisplaysController.clamped(NSRect(x: 5000, y: -300, width: 364, height: 170), screens: [left, right])
        precondition(right.contains(offscreen) || left.contains(offscreen))
        let second = DesktopDisplaysController.clamped(NSRect(x: -1200, y: 500, width: 364, height: 170), screens: [left, right])
        precondition(left.contains(second), "A valid secondary-monitor position must not jump to the primary display")

        // Exercise the real panel host: decorative SwiftUI descendants must not
        // swallow the click or drag, and locking must preserve the saved position.
        overrides[DisplayPreferences.cardEnabled] = true
        overrides[DisplayPreferences.ribbonEnabled] = true
        overrides[DisplayPreferences.locked] = false
        UserDefaults.standard.setVolatileDomain(overrides, forName: UserDefaults.argumentDomain)
        var opened = 0
        let desktop = DesktopDisplaysController(store: model.displays, open: { opened += 1 }, lab: {})
        guard let card = NSApp.windows.first(where: { $0.title == L("Aieyes 桌面卡片") }), let host = card.contentView,
              let ribbon = NSApp.windows.first(where: { $0.title == L("Aieyes 状态条") }) else { fatalError("Missing desktop displays") }
        precondition(card.isVisible && ribbon.isVisible && ribbon.frame.size == NSSize(width: 320, height: 54))
        let screen = NSScreen.main!.visibleFrame
        card.setFrameOrigin(NSPoint(x: screen.minX + 100, y: screen.minY + 100))
        host.layoutSubtreeIfNeeded()
        let center = NSPoint(x: host.bounds.midX, y: host.bounds.midY)
        precondition(host.hitTest(center) === host, "The whole card must receive pointer events")
        func pointer(_ type: NSEvent.EventType, at screenPoint: NSPoint) -> NSEvent {
            NSEvent.mouseEvent(with: type, location: card.convertPoint(fromScreen: screenPoint), modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                               windowNumber: card.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!
        }
        let initialOrigin = card.frame.origin
        let start = card.convertPoint(toScreen: center)
        host.mouseDown(with: pointer(.leftMouseDown, at: start))
        host.mouseUp(with: pointer(.leftMouseUp, at: start))
        precondition(opened == 1 && host.accessibilityPerformPress() && opened == 2, "Mouse and accessibility presses must open the overview")
        let target = NSPoint(x: start.x + 40, y: start.y + 25)
        host.mouseDown(with: pointer(.leftMouseDown, at: start))
        host.mouseDragged(with: pointer(.leftMouseDragged, at: target))
        host.mouseUp(with: pointer(.leftMouseUp, at: target))
        precondition(card.frame.origin == NSPoint(x: initialOrigin.x + 40, y: initialOrigin.y + 25) && opened == 2, "Dragging must move the card without opening a window")
        let savedPosition = UserDefaults.standard.dictionary(forKey: "display.card.position")!
        precondition(savedPosition["x"] as? Double == card.frame.minX && savedPosition["y"] as? Double == card.frame.minY)
        overrides[DisplayPreferences.locked] = true
        UserDefaults.standard.setVolatileDomain(overrides, forName: UserDefaults.argumentDomain)
        let lockedFrame = card.frame
        host.mouseDown(with: pointer(.leftMouseDown, at: target))
        host.mouseDragged(with: pointer(.leftMouseDragged, at: start))
        host.mouseUp(with: pointer(.leftMouseUp, at: start))
        precondition(card.frame == lockedFrame && opened == 2, "A locked drag must neither move the card nor open the overview")
        desktop.stop()
        overrides[DisplayPreferences.cardEnabled] = false
        overrides[DisplayPreferences.ribbonEnabled] = false
        UserDefaults.standard.setVolatileDomain(overrides, forName: UserDefaults.argumentDomain)

        for language in ["zh-CN", "en"] {
            for dark in [false, true] {
                for size in DisplayCardSize.allCases {
                    let value = DisplaySnapshot.sample(.needsInput, language: language)
                    try await capture(DisplayCard(snapshot: value, size: size).background(DisplayCardBackground(phase: value.phase)), size: size.dimensions, dark: dark, to: root.appendingPathComponent("widget-\(size.rawValue)-\(language)-\(dark ? "dark" : "light").png"))
                }
            }
        }
        for style in DynamicDockStyle.allCases {
            for phase in DisplayPhase.allCases {
                try await capture(DockArtworkPreview(snapshot: .sample(phase), style: style, elapsed: 1.0), size: CGSize(width: 128, height: 128), dark: false, to: root.appendingPathComponent("dock-\(style.rawValue)-\(phase.rawValue).png"), opaque: false)
            }
        }
        model.displays.scenario = .working
        try await capture(DisplayCard(snapshot: .sample(.needsInput), size: .large).background(DisplayCardBackground(phase: .needsInput)), size: CGSize(width: 320, height: 320), dark: false, to: root.appendingPathComponent("widget-large-system-size.png"))
        try await capture(DisplayLabView(model: model), size: CGSize(width: 870, height: 1100), dark: false, to: root.appendingPathComponent("display-lab-light.png"))
        try await capture(DisplayLabView(model: model), size: CGSize(width: 530, height: 850), dark: true, to: root.appendingPathComponent("display-lab-narrow-dark.png"))
        print("Display Lab checks passed: snapshot isolation/freshness, unfiltered usage, attention, Dock lifetime/animation/timer cleanup, screen placement, card click/drag/lock/persistence; rendered 39 previews to \(root.path)")
    }

    @MainActor static func capture<V: View>(_ content: V, size: CGSize, dark: Bool, to url: URL, opaque: Bool = true) async throws {
        let host = NSHostingView(rootView: content.frame(width: size.width, height: size.height).background(opaque ? Color(nsColor: .windowBackgroundColor) : Color.clear).environment(\.colorScheme, dark ? .dark : .light).environment(\.surfaceActive, false))
        let window = NSWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: .borderless, backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.backgroundColor = .clear; window.isOpaque = false
        window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        window.contentView = host; window.setFrameOrigin(NSPoint(x: -20000, y: -20000)); window.orderBack(nil)
        defer { window.close() }
        try await Task.sleep(for: .milliseconds(100))
        host.layoutSubtreeIfNeeded()
        guard let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { fatalError("Missing bitmap") }
        host.cacheDisplay(in: host.bounds, to: bitmap)
        try bitmap.representation(using: .png, properties: [:])!.write(to: url)
    }
}
